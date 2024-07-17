use anyhow::{anyhow, Context, Result};
use arc_swap::ArcSwap;
use fn_error_context::context;
use hashbrown::HashSet;
use log::debug;
use quickentity_rs::qn_structs::Ref;

use serde_json::from_str;

use tauri::{AppHandle, Manager};
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	editor_connection::PropertyValue,
	entity::{
		check_local_references_exist, get_decorations, get_diff_info, is_valid_entity_blueprint,
		is_valid_entity_factory
	},
	finish_task,
	general::open_in_editor,
	get_loaded_game_version,
	model::{
		AppSettings, AppState, EditorData, EditorRequest, EditorState, EditorType, EditorValidity, EntityEditorRequest,
		EntityMonacoEvent, EntityMonacoRequest, EntityTreeRequest, GlobalRequest, Request
	},
	rpkg::{extract_latest_overview_info, normalise_to_hash},
	send_notification, send_request, start_task, Notification, NotificationKind
};

pub const SAFE_TO_SYNC: [&str; 43] = [
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
	"SEntityTemplateReference"
];

#[try_fn]
#[context("Couldn't handle monaco event")]
pub async fn handle(app: &AppHandle, event: EntityMonacoEvent) -> Result<()> {
	let app_state = app.state::<AppState>();

	match event {
		EntityMonacoEvent::UpdateContent {
			editor_id,
			entity_id,
			content
		} => {
			update_content(app, editor_id, entity_id, content).await?;
		}

		EntityMonacoEvent::FollowReference { editor_id, reference } => {
			send_request(
				app,
				Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
					EntityTreeRequest::Select {
						editor_id,
						id: Some(reference)
					}
				)))
			)?;
		}

		EntityMonacoEvent::OpenFactory { factory, .. } => {
			open_factory(app, factory).await?;
		}

		EntityMonacoEvent::SignalPin {
			editor_id,
			entity_id,
			pin,
			output
		} => {
			let editor_state = app_state.editor_states.get(&editor_id).context("No such editor")?;

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
				.signal_pin(&entity_id, &entity.blueprint_hash, &pin, output)
				.await?;
		}

		EntityMonacoEvent::OpenResourceOverview { resource, .. } => {
			if let Some(resource_reverse_dependencies) = app_state.resource_reverse_dependencies.load().as_ref() {
				let resource = normalise_to_hash(resource);

				if resource_reverse_dependencies.contains_key(&resource) {
					let id = Uuid::new_v4();

					app_state.editor_states.insert(
						id.to_owned(),
						EditorState {
							file: None,
							data: EditorData::ResourceOverview {
								hash: resource.to_owned()
							}
						}
					);

					send_request(
						app,
						Request::Global(GlobalRequest::CreateTab {
							id,
							name: format!("Resource overview ({resource})"),
							editor_type: EditorType::ResourceOverview
						})
					)?;
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

#[try_fn]
#[context("Couldn't handle update content event")]
pub async fn update_content(app: &AppHandle, editor_id: Uuid, entity_id: String, content: String) -> Result<()> {
	let app_settings = app.state::<ArcSwap<AppSettings>>();
	let app_state = app.state::<AppState>();

	let mut editor_state = app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

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
					if let Some(hash_list) = app_state.hash_list.load().as_ref() {
						if let Some(entry) = hash_list.entries.get(&normalise_to_hash(sub_entity.factory.to_owned())) {
							if !is_valid_entity_factory(&entry.resource_type) {
								send_request(
									app,
									Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
										EntityMonacoRequest::UpdateValidity {
											editor_id,
											validity: EditorValidity::Invalid(
												"Invalid factory; unsupported resource type".into()
											)
										}
									)))
								)?;

								return Ok(());
							}
						}

						if let Some(entry) = hash_list
							.entries
							.get(&normalise_to_hash(sub_entity.blueprint.to_owned()))
						{
							if !is_valid_entity_blueprint(&entry.resource_type) {
								send_request(
									app,
									Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
										EntityMonacoRequest::UpdateValidity {
											editor_id,
											validity: EditorValidity::Invalid(
												"Invalid blueprint; unsupported resource type".into()
											)
										}
									)))
								)?;

								return Ok(());
							}
						}
					}

					entity.entities.insert(entity_id.to_owned(), sub_entity.to_owned());

					let mut reverse_parent_refs: HashSet<String> = HashSet::new();

					for entity_data in entity.entities.values() {
						match entity_data.parent {
							Ref::Full(ref reference) if reference.external_scene.is_none() => {
								reverse_parent_refs.insert(reference.entity_ref.to_owned());
							}

							Ref::Short(Some(ref reference)) => {
								reverse_parent_refs.insert(reference.to_owned());
							}

							_ => {}
						}
					}

					send_request(
						app,
						Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
							EntityTreeRequest::NewItems {
								editor_id,
								new_entities: vec![(
									entity_id.to_owned(),
									sub_entity.parent.to_owned(),
									sub_entity.name.to_owned(),
									sub_entity.factory.to_owned(),
									reverse_parent_refs.contains(&entity_id)
								)]
							}
						)))
					)?;

					send_request(
						app,
						Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
							EntityMonacoRequest::UpdateValidity {
								editor_id,
								validity: EditorValidity::Valid
							}
						)))
					)?;

					send_request(
						app,
						Request::Global(GlobalRequest::SetTabUnsaved {
							id: editor_id,
							unsaved: true
						})
					)?;

					if let Some(game_files) = app_state.game_files.load().as_ref()
						&& let Some(hash_list) = app_state.hash_list.load().as_ref()
						&& let Some(install) = app_settings.load().game_install.as_ref()
						&& let Some(repository) = app_state.repository.load().as_ref()
						&& let Some(tonytools_hash_list) = app_state.tonytools_hash_list.load().as_ref()
					{
						let task = start_task(app, "Updating decorations")?;

						let decorations = get_decorations(
							game_files,
							&app_state.cached_entities,
							repository,
							hash_list,
							get_loaded_game_version(app, install)?,
							tonytools_hash_list,
							entity.entities.get(&entity_id).context("No such entity")?,
							entity
						)?;

						send_request(
							app,
							Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
								EntityMonacoRequest::UpdateDecorationsAndMonacoInfo {
									editor_id: editor_id.to_owned(),
									entity_id: entity_id.to_owned(),
									local_ref_entity_ids: decorations
										.iter()
										.filter(|(x, _)| entity.entities.contains_key(x))
										.map(|(x, _)| x.to_owned())
										.collect(),
									decorations
								}
							)))
						)?;

						finish_task(app, task)?;
					}

					let task = start_task(app, "Syncing properties")?;

					if app_state.editor_connection.is_connected().await {
						let prev_props = previous.properties.unwrap_or_default();

						for (property, val) in sub_entity.properties.to_owned().unwrap_or_default() {
							let mut should_sync = false;

							if let Some(previous_val) = prev_props.get(&property)
								&& *previous_val != val
							{
								should_sync = true;
							} else if !prev_props.contains_key(&property) {
								should_sync = true;
							}

							if should_sync && SAFE_TO_SYNC.iter().any(|&x| val.property_type == x) {
								debug!("Syncing property {} for entity {}", property, entity_id);

								app_state
									.editor_connection
									.set_property(
										&entity_id,
										&entity.blueprint_hash,
										&property,
										PropertyValue {
											property_type: val.property_type,
											data: val.value
										}
									)
									.await?;
							}
						}

						// Set any removed properties back to their default values
						if let Some(intellisense) = app_state.intellisense.load().as_ref()
							&& let Some(game_files) = app_state.game_files.load().as_ref()
							&& let Some(hash_list) = app_state.hash_list.load().as_ref()
							&& let Some(install) = app_settings.load().game_install.as_ref()
						{
							for (property, val) in prev_props {
								if !sub_entity
									.properties
									.to_owned()
									.unwrap_or_default()
									.contains_key(&property) && SAFE_TO_SYNC.iter().any(|&x| val.property_type == x)
								{
									if let Some((_, ty, def_val, _)) = intellisense
										.get_properties(
											game_files,
											&app_state.cached_entities,
											hash_list,
											get_loaded_game_version(app, install)?,
											entity,
											&entity_id,
											false
										)?
										.into_iter()
										.find(|(name, _, _, _)| *name == property)
									{
										debug!(
											"Syncing removed property {} for entity {} with default value according \
											 to intellisense",
											property, entity_id
										);

										app_state
											.editor_connection
											.set_property(
												&entity_id,
												&entity.blueprint_hash,
												&property,
												PropertyValue {
													property_type: ty,
													data: def_val
												}
											)
											.await?;
									}
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
							Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
								EntityTreeRequest::SetDiffInfo {
									editor_id,
									diff_info: get_diff_info(base, current)
								}
							)))
						)?;
					}

					finish_task(app, task)?;
				} else {
					send_request(
						app,
						Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
							EntityMonacoRequest::UpdateValidity {
								editor_id,
								validity: EditorValidity::Valid
							}
						)))
					)?;
				}
			}

			Ok(EditorValidity::Invalid(reason)) => {
				send_request(
					app,
					Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
						EntityMonacoRequest::UpdateValidity {
							editor_id,
							validity: EditorValidity::Invalid(reason)
						}
					)))
				)?;
			}

			Err(err) => {
				send_request(
					app,
					Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
						EntityMonacoRequest::UpdateValidity {
							editor_id,
							validity: EditorValidity::Invalid(format!("Invalid entity: {}", err))
						}
					)))
				)?;
			}
		},

		Err(err) => {
			send_request(
				app,
				Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
					EntityMonacoRequest::UpdateValidity {
						editor_id,
						validity: EditorValidity::Invalid(format!("Invalid entity: {}", err))
					}
				)))
			)?;
		}
	}
}

#[try_fn]
#[context("Couldn't handle open factory event")]
pub async fn open_factory(app: &AppHandle, factory: String) -> Result<()> {
	let app_settings = app.state::<ArcSwap<AppSettings>>();
	let app_state = app.state::<AppState>();

	if let Some(install) = app_settings.load().game_install.as_ref()
		&& let Some(hash_list) = app_state.hash_list.load().as_ref()
		&& let Some(game_files) = app_state.game_files.load().as_deref()
	{
		let factory = normalise_to_hash(factory);

		if let Ok((filetype, _, _)) = extract_latest_overview_info(game_files, &factory) {
			if filetype == "TEMP" {
				open_in_editor(app, game_files, install, hash_list, &factory).await?;
			} else {
				let id = Uuid::new_v4();

				app_state.editor_states.insert(
					id.to_owned(),
					EditorState {
						file: None,
						data: EditorData::ResourceOverview {
							hash: factory.to_owned()
						}
					}
				);

				send_request(
					app,
					Request::Global(GlobalRequest::CreateTab {
						id,
						name: format!("Resource overview ({factory})"),
						editor_type: EditorType::ResourceOverview
					})
				)?;
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
