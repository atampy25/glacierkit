use anyhow::{Context, Result, anyhow};
use fn_error_context::context;
use hitman_commons::metadata::RuntimeID;
use itertools::Itertools;
use log::debug;
use quickentity_rs::entity::{EntityID, SubEntity};
use serde_json::from_str;
use std::collections::HashMap;
use tauri::{AppHandle, Manager};
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	Notification, NotificationKind,
	entity::{
		check_local_references_exist, get_decorations, get_diff_info, is_valid_entity_blueprint,
		is_valid_entity_factory, reverse_parent_refs_set
	},
	event_handling::resource_overview::open_resource_overview,
	finish_task,
	general::open_in_editor,
	model::{
		AppState, EditorData, EditorRequest, EditorRequestData, EditorValidity, EntityEditorRequest, EntityMonacoEvent,
		EntityMonacoRequest, EntityTreeRequest, Request, TabRequest, TabRequestData
	},
	send_notification, send_request, start_task
};

pub static SAFE_TO_SYNC: [&str; 44] = [
	"SMatrix43",
	"float32",
	"bool",
	"SColorRGB",
	"ZString",
	"SVector3",
	"int32",
	"uint8",
	"SVector2",
	"uint32",
	"ZGuid",
	"ZCurve",
	"SColorRGBA",
	"ZGameTime",
	"TArray<ZGameTime>",
	"TArray<bool>",
	"TArray<SGaitTransitionEntry>",
	"TArray<SMapMarkerData>",
	"uint64",
	"TArray<int32>",
	"TArray<SConversationPart>",
	"SBodyPartDamageMultipliers",
	"TArray<SVector2>",
	"TArray<ZSharedSensorDef.SVisibilitySetting>",
	"TArray<ZString>",
	"TArray<STargetableBoneConfiguration>",
	"TArray<ZSecuritySystemCameraConfiguration.SHitmanVisibleEscalationRule>",
	"TArray<ZSecuritySystemCameraConfiguration.SDeadBodyVisibleEscalationRule>",
	"S25DProjectionSettings",
	"SVector4",
	"TArray<SClothVertex>",
	"TArray<SFontLibraryDefinition>",
	"TArray<SCamBone>",
	"TArray<SVector3>",
	"TArray<ZHUDOccluderTriggerEntity.SBoneTestSetup>",
	"uint16",
	"SWorldSpaceSettings",
	"SCCEffectSet",
	"TArray<AI.SFirePattern01>",
	"TArray<AI.SFirePattern02>",
	"SSCCuriousConfiguration",
	"TArray<SColorRGB>",
	"SEntityTemplateReference",
	"ZSpatialEntity.ERoomBehaviour"
];

#[static_init::dynamic]
pub static ENUMS: HashMap<&'static str, Vec<&'static str>> = hitman_bin1::game::h1::ENUMS
	.iter()
	.chain(hitman_bin1::game::h2::ENUMS.iter())
	.chain(hitman_bin1::game::h3::ENUMS.iter())
	.map(|(ty, shape)| {
		(
			*ty,
			match shape.ty {
				facet::Type::User(facet::UserType::Enum(enum_ty)) => {
					enum_ty.variants.iter().map(|variant| variant.name).collect()
				}

				_ => panic!("hitman-bin1 ENUMS member was not enum")
			}
		)
	})
	.unique()
	.collect();

#[try_fn]
#[context("Couldn't handle monaco event")]
pub async fn handle(app: &AppHandle, editor_id: Uuid, event: EntityMonacoEvent) -> Result<()> {
	let app_state = app.state::<AppState>();

	match event {
		EntityMonacoEvent::UpdateContent { entity_id, content } => {
			update_content(app, editor_id, entity_id, content).await?;
		}

		EntityMonacoEvent::FollowReference { reference } => {
			send_request(
				app,
				Request::Editor(EditorRequest {
					editor: editor_id,
					data: EditorRequestData::Entity(EntityEditorRequest::Tree(EntityTreeRequest::Select {
						id: Some(reference)
					}))
				})
			)?;
		}

		EntityMonacoEvent::OpenFactory { factory, .. } => {
			open_factory(app, factory).await?;
		}

		EntityMonacoEvent::SignalPin { entity_id, pin, output } => {
			let editor_state = app_state
				.editor_states
				.get(&editor_id)
				.await
				.context("No such editor")?;

			let entity = match editor_state.data {
				EditorData::QNEntity { ref entity, .. } => entity,
				EditorData::QNPatch { ref current, .. } => current,

				_ => {
					Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
					panic!();
				}
			};

			app_state
				.editor_connection
				.signal_pin(entity_id, &entity.blueprint.to_hash(), &pin, output)
				.await?;
		}

		EntityMonacoEvent::OpenResourceOverview { resource, .. } => {
			if let Some(game) = app_state.game.load().as_ref() {
				if game.resource_exists(resource) {
					open_resource_overview(app, resource).await?;
				} else {
					send_notification(
						app,
						Notification {
							kind: NotificationKind::Error,
							title: "Not a vanilla resource".into(),
							subtitle: "This factory doesn't exist in the base game files.".into()
						}
					)?;
				}
			} else {
				send_notification(
					app,
					Notification {
						kind: NotificationKind::Error,
						title: "No game selected".into(),
						subtitle: "You can't open game files without a copy of the game selected.".into()
					}
				)?;
			}
		}
	}
}

pub fn sub_entity_rough_eq(entity1: &SubEntity, entity2: &SubEntity) -> bool {
	entity1.parent == entity2.parent
		&& entity1.name == entity2.name
		&& entity1.factory == entity2.factory
		&& entity1.blueprint == entity2.blueprint
		&& entity1.editor_only == entity2.editor_only
		&& entity1.properties.len() == entity2.properties.len()
		&& entity1
			.properties
			.iter()
			.zip(entity2.properties.iter())
			.all(|(a, b)| a.0 == b.0 && a.1.post_init == b.1.post_init && a.1.value.rough_eq(&b.1.value))
		&& entity1.platform_specific_properties.len() == entity2.platform_specific_properties.len()
		&& entity1
			.platform_specific_properties
			.iter()
			.zip(entity2.platform_specific_properties.iter())
			.all(|(a, b)| {
				a.0 == b.0
					&& a.1.len() == b.1.len()
					&& a.1
						.iter()
						.zip(b.1.iter())
						.all(|(a, b)| a.0 == b.0 && a.1.post_init == b.1.post_init && a.1.value.rough_eq(&b.1.value))
			}) && entity1.events.len() == entity2.events.len()
		&& entity1.events.iter().zip(entity2.events.iter()).all(|(a, b)| {
			a.0 == b.0
				&& a.1.len() == b.1.len()
				&& a.1.iter().zip(b.1.iter()).all(|(a, b)| {
					a.0 == b.0
						&& a.1.len() == b.1.len()
						&& a.1.iter().zip(b.1.iter()).all(|(a, b)| {
							a.entity_ref == b.entity_ref
								&& ((a.value.is_none() && b.value.is_none())
									|| a.value
										.as_ref()
										.zip(b.value.as_ref())
										.is_some_and(|(a, b)| a.rough_eq(b)))
						})
				})
		}) && entity1.input_copying.len() == entity2.input_copying.len()
		&& entity1
			.input_copying
			.iter()
			.zip(entity2.input_copying.iter())
			.all(|(a, b)| {
				a.0 == b.0
					&& a.1.len() == b.1.len()
					&& a.1.iter().zip(b.1.iter()).all(|(a, b)| {
						a.0 == b.0
							&& a.1.len() == b.1.len()
							&& a.1.iter().zip(b.1.iter()).all(|(a, b)| {
								a.entity_id == b.entity_id
									&& ((a.value.is_none() && b.value.is_none())
										|| a.value
											.as_ref()
											.zip(b.value.as_ref())
											.is_some_and(|(a, b)| a.rough_eq(b)))
							})
					})
			}) && entity1.output_copying.len() == entity2.output_copying.len()
		&& entity1
			.output_copying
			.iter()
			.zip(entity2.output_copying.iter())
			.all(|(a, b)| {
				a.0 == b.0
					&& a.1.len() == b.1.len()
					&& a.1.iter().zip(b.1.iter()).all(|(a, b)| {
						a.0 == b.0
							&& a.1.len() == b.1.len()
							&& a.1.iter().zip(b.1.iter()).all(|(a, b)| {
								a.entity_id == b.entity_id
									&& ((a.value.is_none() && b.value.is_none())
										|| a.value
											.as_ref()
											.zip(b.value.as_ref())
											.is_some_and(|(a, b)| a.rough_eq(b)))
							})
					})
			}) && entity1.property_aliases == entity2.property_aliases
		&& entity1.exposed_entities == entity2.exposed_entities
		&& entity1.exposed_interfaces == entity2.exposed_interfaces
		&& entity1.subsets == entity2.subsets
}

#[try_fn]
#[context("Couldn't handle update content event")]
pub async fn update_content(app: &AppHandle, editor_id: Uuid, entity_id: EntityID, content: String) -> Result<()> {
	let app_state = app.state::<AppState>();

	let mut editor_state = app_state
		.editor_states
		.get_mut(&editor_id)
		.await
		.context("No such editor")?;

	let entity = match editor_state.data {
		EditorData::QNEntity { ref mut entity, .. } => entity,
		EditorData::QNPatch { ref mut current, .. } => current,

		_ => {
			Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
			panic!();
		}
	};

	match from_str(&content) {
		Ok(sub_entity) => match check_local_references_exist(&sub_entity, entity) {
			Ok(EditorValidity::Valid) => {
				let previous = entity
					.entities
					.get(&entity_id)
					.context("No such sub-entity")?
					.to_owned();

				if sub_entity != previous {
					if let Some(game) = app_state.game.load().as_ref()
						&& let Some(ty) = game.resource_type(sub_entity.factory.resource)
						&& !is_valid_entity_factory(ty)
					{
						send_request(
							app,
							Request::Editor(EditorRequest {
								editor: editor_id,
								data: EditorRequestData::Entity(EntityEditorRequest::Monaco(
									EntityMonacoRequest::UpdateValidity {
										validity: EditorValidity::Invalid(
											"Invalid factory; unsupported resource type".into()
										)
									}
								))
							})
						)?;

						return Ok(());
					}

					if let Some(game) = app_state.game.load().as_ref()
						&& let Some(ty) = game.resource_type(sub_entity.blueprint)
						&& !is_valid_entity_blueprint(ty)
					{
						send_request(
							app,
							Request::Editor(EditorRequest {
								editor: editor_id,
								data: EditorRequestData::Entity(EntityEditorRequest::Monaco(
									EntityMonacoRequest::UpdateValidity {
										validity: EditorValidity::Invalid(
											"Invalid blueprint; unsupported resource type".into()
										)
									}
								))
							})
						)?;

						return Ok(());
					}

					entity.entities.insert(entity_id.to_owned(), sub_entity.to_owned());

					let reverse_parent_refs = reverse_parent_refs_set(entity);

					send_request(
						app,
						Request::Editor(EditorRequest {
							editor: editor_id,
							data: EditorRequestData::Entity(EntityEditorRequest::Tree(EntityTreeRequest::NewItems {
								new_entities: vec![(
									entity_id.to_owned(),
									sub_entity.parent.to_owned(),
									sub_entity.name.to_owned(),
									sub_entity.factory.resource.to_owned(),
									reverse_parent_refs.contains(&entity_id)
								)]
							}))
						})
					)?;

					send_request(
						app,
						Request::Editor(EditorRequest {
							editor: editor_id,
							data: EditorRequestData::Entity(EntityEditorRequest::Monaco(
								EntityMonacoRequest::UpdateValidity {
									validity: EditorValidity::Valid
								}
							))
						})
					)?;

					if !sub_entity_rough_eq(&sub_entity, &previous) {
						send_request(
							app,
							Request::Tab(TabRequest {
								tab: editor_id,
								data: TabRequestData::SetUnsaved { unsaved: true }
							})
						)?;
					}

					if let Some(game) = app_state.game.load().as_ref()
						&& let Some(tonytools_hash_list) = app_state.tonytools_hash_list.load().as_ref()
					{
						let task = start_task(app, "Updating decorations")?;

						let decorations = get_decorations(
							game,
							tonytools_hash_list,
							entity.entities.get(&entity_id).context("No such entity")?,
							entity
						)?;

						send_request(
							app,
							Request::Editor(EditorRequest {
								editor: editor_id.to_owned(),
								data: EditorRequestData::Entity(EntityEditorRequest::Monaco(
									EntityMonacoRequest::UpdateDecorationsAndMonacoInfo {
										entity_id: entity_id.to_owned(),
										local_ref_entity_ids: decorations
											.iter()
											.filter_map(|(x, _)| {
												x.parse::<EntityID>().ok().filter(|x| entity.entities.contains_key(x))
											})
											.collect(),
										decorations
									}
								))
							})
						)?;

						finish_task(app, task)?;
					}

					let task = start_task(app, "Syncing properties")?;

					if app_state.editor_connection.is_connected().await {
						let prev_props = previous.properties;

						for (property, val) in &sub_entity.properties {
							let mut should_sync = false;

							if let Some(previous_val) = prev_props.get(property)
								&& previous_val != val
							{
								should_sync = true;
							} else if !prev_props.contains_key(property) {
								should_sync = true;
							}

							if should_sync && SAFE_TO_SYNC.iter().any(|&x| val.value.variant_type() == x) {
								debug!("Syncing property {} for entity {}", property, entity_id);

								app_state
									.editor_connection
									.set_property(
										entity_id,
										&entity.blueprint.to_hash(),
										property,
										val.value.to_owned()
									)
									.await?;
							}
						}

						// Set any removed properties back to their default values
						if let Some(game) = app_state.game.load().as_ref() {
							for (property, val) in prev_props {
								if !sub_entity.properties.contains_key(&property)
									&& SAFE_TO_SYNC.iter().any(|&x| val.value.variant_type() == x)
									&& let Some((_, def_val, _)) = game
										.intellisense()
										.get_properties(game, entity, entity_id, false)?
										.into_iter()
										.find(|(name, _, _)| *name == property)
								{
									debug!(
										"Syncing removed property {} for entity {} with default value according to \
										 intellisense",
										property, entity_id
									);

									app_state
										.editor_connection
										.set_property(entity_id, &entity.blueprint.to_hash(), &property, def_val)
										.await?;
								}
							}
						}
					}

					finish_task(app, task)?;

					let task = start_task(app, "Updating change information")?;

					if let EditorData::QNPatch {
						ref base, ref current, ..
					} = editor_state.data
					{
						send_request(
							app,
							Request::Editor(EditorRequest {
								editor: editor_id,
								data: EditorRequestData::Entity(EntityEditorRequest::Tree({
									let (new, modified, removed) = get_diff_info(base, current);
									EntityTreeRequest::SetDiffInfo { new, modified, removed }
								}))
							})
						)?;
					}

					finish_task(app, task)?;
				} else {
					send_request(
						app,
						Request::Editor(EditorRequest {
							editor: editor_id,
							data: EditorRequestData::Entity(EntityEditorRequest::Monaco(
								EntityMonacoRequest::UpdateValidity {
									validity: EditorValidity::Valid
								}
							))
						})
					)?;
				}
			}

			Ok(EditorValidity::Invalid(reason)) => {
				send_request(
					app,
					Request::Editor(EditorRequest {
						editor: editor_id,
						data: EditorRequestData::Entity(EntityEditorRequest::Monaco(
							EntityMonacoRequest::UpdateValidity {
								validity: EditorValidity::Invalid(reason)
							}
						))
					})
				)?;
			}

			Err(err) => {
				send_request(
					app,
					Request::Editor(EditorRequest {
						editor: editor_id,
						data: EditorRequestData::Entity(EntityEditorRequest::Monaco(
							EntityMonacoRequest::UpdateValidity {
								validity: EditorValidity::Invalid(format!("Invalid entity: {}", err))
							}
						))
					})
				)?;
			}
		},

		Err(err) => {
			send_request(
				app,
				Request::Editor(EditorRequest {
					editor: editor_id,
					data: EditorRequestData::Entity(EntityEditorRequest::Monaco(EntityMonacoRequest::UpdateValidity {
						validity: EditorValidity::Invalid(format!("Invalid entity: {}", err))
					}))
				})
			)?;
		}
	}
}

#[try_fn]
#[context("Couldn't handle open factory event")]
pub async fn open_factory(app: &AppHandle, factory: RuntimeID) -> Result<()> {
	let app_state = app.state::<AppState>();

	if let Some(game) = app_state.game.load().as_deref() {
		if let Some(ty) = game.resource_type(factory) {
			if ty == "TEMP" {
				open_in_editor(app, game, factory).await?;
			} else {
				open_resource_overview(app, factory).await?;
			}
		} else {
			send_notification(
				app,
				Notification {
					kind: NotificationKind::Error,
					title: "Not a vanilla resource".into(),
					subtitle: "This factory doesn't exist in the base game files.".into()
				}
			)?;
		}
	} else {
		send_notification(
			app,
			Notification {
				kind: NotificationKind::Error,
				title: "No game selected".into(),
				subtitle: "You can't open game files without a copy of the game selected.".into()
			}
		)?;
	}
}
