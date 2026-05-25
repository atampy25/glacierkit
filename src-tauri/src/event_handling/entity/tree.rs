use std::{
	collections::{HashMap, HashSet},
	ops::Deref
};

use anyhow::{Context, Result, anyhow};
use arboard::Clipboard;
use ecow::EcoString;
use fn_error_context::context;
use hitman_bin1::game::h3::{ZSpatialEntity_ERoomBehaviour, ZVariant};
use hitman_commons::{
	game::GameVersion,
	metadata::{ReferenceFlags, ReferenceType, ResourceReference, RuntimeID},
	rid
};
use hitman_formats::wwev::WwiseEvent;
use log::debug;
use ordermap::OrderMap;
use quickentity_rs::{
	apply_patch,
	entity::{EntityID, LocalPinConnection, PinConnection, Property, Ref, SubEntity},
	patch::{Patch, PatchOperation, SubEntityOperation},
	variant::{Transform, Variant, Vec3}
};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde::Serialize;
use serde_json::{from_slice, from_str, json, to_string};
use tauri::{AppHandle, Manager};
use tryvial::try_fn;
use uuid::Uuid;

use super::monaco::SAFE_TO_SYNC;
use crate::{
	Notification, NotificationKind,
	biome::to_string_clear,
	entity::{
		CopiedEntityData, ReverseReferenceData, alter_ref_according_to_changelist, calculate_reverse_references,
		check_local_references_exist, get_decorations, get_diff_info, get_recursive_children, is_valid_entity_factory,
		random_entity_id, reverse_parent_refs_set, visit_variant_mut
	},
	finish_task,
	general::EMPTY_ID,
	model::{
		AppState, EditorData, EditorRequest, EditorRequestData, EditorValidity, EntityEditorRequest,
		EntityGeneralRequest, EntityMetaPaneRequest, EntityMonacoRequest, EntityTreeEvent, EntityTreeRequest, Request,
		TabRequest, TabRequestData
	},
	send_notification, send_request, start_task
};

#[try_fn]
#[context("Couldn't handle tree event")]
pub async fn handle(app: &AppHandle, editor_id: Uuid, event: EntityTreeEvent) -> Result<()> {
	match event {
		EntityTreeEvent::Initialise => {
			initialise(app, editor_id).await?;
		}

		EntityTreeEvent::Select { id } => {
			select(app, editor_id, id).await?;
		}

		EntityTreeEvent::Create { id, content } => {
			create(app, editor_id, id, *content).await?;
		}

		EntityTreeEvent::Delete { id } => {
			delete(app, editor_id, id).await?;
		}

		EntityTreeEvent::Rename { id, new_name } => {
			rename(app, editor_id, id, new_name).await?;
		}

		EntityTreeEvent::Reparent { id, new_parent } => {
			reparent(app, editor_id, id, new_parent).await?;
		}

		EntityTreeEvent::Copy { id } => {
			copy(app, editor_id, id).await?;
		}

		EntityTreeEvent::Paste { parent_id } => {
			paste(
				app,
				editor_id,
				parent_id,
				from_str::<CopiedEntityData>(&Clipboard::new()?.get_text()?)?
			)
			.await?;
		}

		EntityTreeEvent::Search { query } => {
			search(app, editor_id, query).await?;
		}

		EntityTreeEvent::ShowHelpMenu { entity_id } => {
			help_menu(app, editor_id, entity_id).await?;
		}

		EntityTreeEvent::UseTemplate { parent_id, template } => {
			paste(app, editor_id, parent_id, template).await?;
		}

		EntityTreeEvent::AddGameBrowserItem { parent_id, file } => {
			add_game_browser_item(app, editor_id, parent_id, file.0).await?;
		}

		EntityTreeEvent::SelectEntityInEditor { entity_id } => {
			select_entity_in_editor(app, editor_id, entity_id).await?;
		}

		EntityTreeEvent::MoveEntityToPlayer { entity_id } => {
			move_entity_to_player(app, editor_id, entity_id).await?;
		}

		EntityTreeEvent::RotateEntityAsPlayer { entity_id } => {
			rotate_entity_as_player(app, editor_id, entity_id).await?;
		}

		EntityTreeEvent::MoveEntityToCamera { entity_id } => {
			move_entity_to_camera(app, editor_id, entity_id).await?;
		}

		EntityTreeEvent::RotateEntityAsCamera { entity_id } => {
			rotate_entity_as_camera(app, editor_id, entity_id).await?;
		}

		EntityTreeEvent::RestoreToOriginal { entity_id } => {
			restore_to_original(app, editor_id, entity_id).await?;
		}
	}
}

#[try_fn]
#[context("Couldn't handle initialise event")]
pub async fn initialise(app: &AppHandle, editor_id: Uuid) -> Result<()> {
	let app_state = app.state::<AppState>();

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

	let mut entities = vec![];
	let reverse_parent_refs = reverse_parent_refs_set(entity);

	for (entity_id, entity_data) in entity.entities.iter() {
		entities.push((
			entity_id.to_owned(),
			entity_data.parent.to_owned(),
			entity_data.name.to_owned(),
			entity_data.factory.resource.to_owned(),
			reverse_parent_refs.contains(entity_id)
		));
	}

	send_request(
		app,
		Request::Editor(EditorRequest {
			editor: editor_id.to_owned(),
			data: EditorRequestData::Entity(EntityEditorRequest::General(EntityGeneralRequest::SetIsPatchEditor {
				is_patch_editor: matches!(editor_state.data, EditorData::QNPatch { .. })
			}))
		})
	)?;

	send_request(
		app,
		Request::Editor(EditorRequest {
			editor: editor_id.to_owned(),
			data: EditorRequestData::Entity(EntityEditorRequest::Tree(EntityTreeRequest::NewTree { entities }))
		})
	)?;

	send_request(
		app,
		Request::Editor(EditorRequest {
			editor: editor_id.to_owned(),
			data: EditorRequestData::Entity(EntityEditorRequest::Tree(EntityTreeRequest::SetTemplates {
				templates: from_slice(include_bytes!("../../../assets/templates.json")).unwrap()
			}))
		})
	)?;

	let editor_connected = app_state.editor_connection.is_connected().await;

	send_request(
		app,
		Request::Editor(EditorRequest {
			editor: editor_id.to_owned(),
			data: EditorRequestData::Entity(EntityEditorRequest::Tree(
				EntityTreeRequest::SetEditorConnectionAvailable {
					editor_connection_available: editor_connected
				}
			))
		})
	)?;

	send_request(
		app,
		Request::Editor(EditorRequest {
			editor: editor_id.to_owned(),
			data: EditorRequestData::Entity(EntityEditorRequest::Monaco(EntityMonacoRequest::SetEditorConnected {
				connected: editor_connected
			}))
		})
	)?;

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
}

#[try_fn]
#[context("Couldn't handle create event")]
pub async fn create(app: &AppHandle, editor_id: Uuid, id: EntityID, content: SubEntity) -> Result<()> {
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

	entity.entities.insert(id, content);

	send_request(
		app,
		Request::Tab(TabRequest {
			tab: editor_id.to_owned(),
			data: TabRequestData::SetUnsaved { unsaved: true }
		})
	)?;

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
}

#[try_fn]
#[context("Couldn't handle rename event")]
pub async fn rename(app: &AppHandle, editor_id: Uuid, id: EntityID, new_name: String) -> Result<()> {
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

	entity.entities.get_mut(&id).context("No such entity")?.name = new_name.into();

	send_request(
		app,
		Request::Tab(TabRequest {
			tab: editor_id,
			data: TabRequestData::SetUnsaved { unsaved: true }
		})
	)?;

	send_request(
		app,
		Request::Editor(EditorRequest {
			editor: editor_id.to_owned(),
			data: EditorRequestData::Entity(EntityEditorRequest::Monaco(
				EntityMonacoRequest::ReplaceContentIfSameEntityID {
					entity_id: id.to_owned(),
					content: to_string_clear(entity.entities.get(&id).context("No such entity")?)?
				}
			))
		})
	)?;

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
}

#[try_fn]
#[context("Couldn't handle select event")]
pub async fn select(app: &AppHandle, editor_id: Uuid, id: EntityID) -> Result<()> {
	let app_state = app.state::<AppState>();

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

	let task = start_task(app, format!("Selecting {}", id))?;

	send_request(
		app,
		Request::Editor(EditorRequest {
			editor: editor_id.to_owned(),
			data: EditorRequestData::Entity(EntityEditorRequest::Monaco(EntityMonacoRequest::ReplaceContent {
				entity_id: id.to_owned(),
				content: to_string_clear(entity.entities.get(&id).context("No such entity")?)?
			}))
		})
	)?;

	send_request(
		app,
		Request::Editor(EditorRequest {
			editor: editor_id.to_owned(),
			data: EditorRequestData::Entity(EntityEditorRequest::Monaco(EntityMonacoRequest::UpdateValidity {
				validity: EditorValidity::Valid
			}))
		})
	)?;

	let reverse_refs = calculate_reverse_references(entity)?
		.remove(&id)
		.context("No such entity")?;

	let settings = match editor_state.data {
		EditorData::QNEntity { ref settings, .. } => settings,
		EditorData::QNPatch { ref settings, .. } => settings,

		_ => {
			Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
			panic!();
		}
	};

	send_request(
		app,
		Request::Editor(EditorRequest {
			editor: editor_id.to_owned(),
			data: EditorRequestData::Entity(EntityEditorRequest::MetaPane(EntityMetaPaneRequest::SetReverseRefs {
				entity_names: reverse_refs
					.iter()
					.filter(|x| settings.show_reverse_parent_refs || !matches!(x.data, ReverseReferenceData::Parent))
					.map(|x| (x.from.to_owned(), entity.entities.get(&x.from).unwrap().name.to_owned()))
					.collect(),
				reverse_refs: reverse_refs
					.into_iter()
					.filter(|x| settings.show_reverse_parent_refs || !matches!(x.data, ReverseReferenceData::Parent))
					.collect()
			}))
		})
	)?;

	send_request(
		app,
		Request::Editor(EditorRequest {
			editor: editor_id.to_owned(),
			data: EditorRequestData::Entity(EntityEditorRequest::MetaPane(EntityMetaPaneRequest::SetNotes {
				entity_id: id.to_owned(),
				notes: entity
					.comments
					.iter()
					.find(|x| x.parent == Some(id))
					.map(|x| x.text.deref())
					.unwrap_or("")
					.into()
			}))
		})
	)?;

	finish_task(app, task)?;

	if let Some(game) = app_state.game.load().as_ref()
		&& let Some(tonytools_hash_list) = app_state.tonytools_hash_list.load().as_ref()
	{
		let task = start_task(app, format!("Gathering intellisense data for {}", id))?;

		let (properties, pins) = rayon::join(
			|| game.intellisense().get_properties(game, entity, id, true),
			|| game.intellisense().get_pins(game, entity, id, false)
		);

		let (input_pins, output_pins) = pins?;

		send_request(
			app,
			Request::Editor(EditorRequest {
				editor: editor_id.to_owned(),
				data: EditorRequestData::Entity(EntityEditorRequest::Monaco(EntityMonacoRequest::UpdateIntellisense {
					entity_id: id.to_owned(),
					properties: properties?,
					input_pins,
					output_pins
				}))
			})
		)?;

		finish_task(app, task)?;

		let task = start_task(app, format!("Computing decorations for {}", id))?;

		let decorations = get_decorations(
			game,
			tonytools_hash_list,
			entity.entities.get(&id).context("No such entity")?,
			entity
		)?;

		send_request(
			app,
			Request::Editor(EditorRequest {
				editor: editor_id.to_owned(),
				data: EditorRequestData::Entity(EntityEditorRequest::Monaco(
					EntityMonacoRequest::UpdateDecorationsAndMonacoInfo {
						entity_id: id.to_owned(),
						local_ref_entity_ids: decorations
							.iter()
							.filter_map(|(x, _)| x.parse::<EntityID>().ok().filter(|x| entity.entities.contains_key(x)))
							.collect(),
						decorations
					}
				))
			})
		)?;

		finish_task(app, task)?;
	}

	let task = start_task(app, format!("Selecting {} in editor", id))?;

	if app_state.editor_connection.is_connected().await {
		app_state
			.editor_connection
			.select_entity(id, &entity.blueprint.to_hash())
			.await?;
	}

	finish_task(app, task)?;
}

#[try_fn]
#[context("Couldn't handle reparent event")]
pub async fn reparent(app: &AppHandle, editor_id: Uuid, id: EntityID, new_parent: Option<EntityID>) -> Result<()> {
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

	entity.entities.get_mut(&id).context("No such entity")?.parent = new_parent.map(Ref::local);

	send_request(
		app,
		Request::Tab(TabRequest {
			tab: editor_id,
			data: TabRequestData::SetUnsaved { unsaved: true }
		})
	)?;

	send_request(
		app,
		Request::Editor(EditorRequest {
			editor: editor_id.to_owned(),
			data: EditorRequestData::Entity(EntityEditorRequest::Monaco(
				EntityMonacoRequest::ReplaceContentIfSameEntityID {
					entity_id: id.to_owned(),
					content: to_string_clear(entity.entities.get(&id).context("No such entity")?)?
				}
			))
		})
	)?;

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
}

#[try_fn]
#[context("Couldn't handle delete event")]
pub async fn delete(app: &AppHandle, editor_id: Uuid, id: EntityID) -> Result<()> {
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Deleting entity {}", id))?;

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

	let reverse_refs = calculate_reverse_references(entity)?;

	let entities_to_delete = get_recursive_children(entity, id, &reverse_refs)?
		.into_iter()
		.collect::<HashSet<_>>();

	let mut patch = Patch {
		factory: EMPTY_ID,
		blueprint: EMPTY_ID,
		patch: vec![],
		patch_version: 7
	};

	let mut refs_deleted = 0;

	for entity_to_delete in &entities_to_delete {
		for reverse_ref in reverse_refs.get(entity_to_delete).context("No such entity")? {
			match &reverse_ref.data {
				ReverseReferenceData::Parent => {
					// The entity itself will be deleted later
				}

				ReverseReferenceData::Property { property_name } => {
					let entity_props = &mut entity.entities.get_mut(&reverse_ref.from).unwrap().properties;

					if let Variant::Array(_, vals) = &mut entity_props.get_mut(property_name).unwrap().value {
						vals.retain(|item| {
							if let Variant::Ref(item) = item
								&& let Some(local_ref) = item.as_ref().and_then(Ref::as_local)
							{
								local_ref != *entity_to_delete
							} else {
								true
							}
						});
					} else {
						entity_props.remove(property_name).unwrap();
					}
				}

				ReverseReferenceData::PlatformProperty {
					property_name,
					platform
				} => {
					let entity_props = entity
						.entities
						.get_mut(&reverse_ref.from)
						.unwrap()
						.platform_properties
						.get_mut(platform)
						.unwrap();

					if let Variant::Array(_, vals) = &mut entity_props.get_mut(property_name).unwrap().value {
						vals.retain(|item| {
							if let Variant::Ref(item) = item
								&& let Some(local_ref) = item.as_ref().and_then(Ref::as_local)
							{
								local_ref != *entity_to_delete
							} else {
								true
							}
						});
					} else {
						entity_props.remove(property_name).unwrap();
					}
				}

				ReverseReferenceData::Event { event, trigger } => {
					patch.patch.push(PatchOperation::PatchEntity(
						reverse_ref.from.to_owned(),
						SubEntityOperation::RemoveEventConnection(
							event.to_owned(),
							trigger.to_owned(),
							entity
								.entities
								.get(&reverse_ref.from)
								.unwrap()
								.events
								.get(event)
								.unwrap()
								.get(trigger)
								.unwrap()
								.iter()
								.find(|x| x.entity_ref.as_local().is_some_and(|x| x == *entity_to_delete))
								.unwrap()
								.to_owned()
						)
					));
				}

				ReverseReferenceData::InputForwarding { trigger, propagate } => {
					patch.patch.push(PatchOperation::PatchEntity(
						reverse_ref.from.to_owned(),
						SubEntityOperation::RemoveInputForwarding(
							trigger.to_owned(),
							propagate.to_owned(),
							entity
								.entities
								.get(&reverse_ref.from)
								.unwrap()
								.input_forwardings
								.get(trigger)
								.unwrap()
								.get(propagate)
								.unwrap()
								.iter()
								.find(|x| x.entity_id == *entity_to_delete)
								.unwrap()
								.to_owned()
						)
					));
				}

				ReverseReferenceData::OutputForwarding { event, propagate } => {
					patch.patch.push(PatchOperation::PatchEntity(
						reverse_ref.from.to_owned(),
						SubEntityOperation::RemoveOutputForwarding(
							event.to_owned(),
							propagate.to_owned(),
							entity
								.entities
								.get(&reverse_ref.from)
								.unwrap()
								.output_forwardings
								.get(event)
								.unwrap()
								.get(propagate)
								.unwrap()
								.iter()
								.find(|x| x.entity_id == *entity_to_delete)
								.unwrap()
								.to_owned()
						)
					));
				}

				ReverseReferenceData::PropertyAlias { aliased_name, .. } => {
					entity
						.entities
						.get_mut(&reverse_ref.from)
						.unwrap()
						.property_aliases
						.get_mut(aliased_name)
						.unwrap()
						.retain(|x| x.original_entity != *entity_to_delete);
				}

				ReverseReferenceData::ExposedEntity { exposed_name } => {
					entity
						.entities
						.get_mut(&reverse_ref.from)
						.unwrap()
						.exposed_entities
						.get_mut(exposed_name)
						.unwrap()
						.refers_to
						.retain(|x| x.as_local().is_some_and(|x| x != *entity_to_delete));

					if entity
						.entities
						.get_mut(&reverse_ref.from)
						.unwrap()
						.exposed_entities
						.get_mut(exposed_name)
						.unwrap()
						.refers_to
						.is_empty()
					{
						entity
							.entities
							.get_mut(&reverse_ref.from)
							.unwrap()
							.exposed_entities
							.remove(exposed_name)
							.unwrap();
					}
				}

				ReverseReferenceData::ExposedInterface { interface } => {
					entity
						.entities
						.get_mut(&reverse_ref.from)
						.unwrap()
						.exposed_interfaces
						.remove(interface)
						.unwrap();
				}

				ReverseReferenceData::Subset { subset } => {
					entity
						.entities
						.get_mut(&reverse_ref.from)
						.unwrap()
						.subsets
						.get_mut(subset)
						.unwrap()
						.retain(|x| x != entity_to_delete);
				}
			}

			refs_deleted += 1;
		}
	}

	apply_patch(entity, patch, |_| {}).map_err(|x| anyhow!(x))?;

	entity.entities.retain(|x, _| !entities_to_delete.contains(x));

	finish_task(app, task)?;

	send_notification(
		app,
		Notification {
			kind: NotificationKind::Info,
			title: format!(
				"Deleted {} entit{}",
				entities_to_delete.len(),
				if entities_to_delete.len() == 1 { "y" } else { "ies" }
			),
			subtitle: format!(
				"The entity, its children and {} reference{} have been deleted",
				refs_deleted,
				if refs_deleted == 1 { "" } else { "s" }
			)
		}
	)?;

	send_request(
		app,
		Request::Tab(TabRequest {
			tab: editor_id,
			data: TabRequestData::SetUnsaved { unsaved: true }
		})
	)?;

	send_request(
		app,
		Request::Editor(EditorRequest {
			editor: editor_id.to_owned(),
			data: EditorRequestData::Entity(EntityEditorRequest::Monaco(EntityMonacoRequest::DeselectIfSelected {
				entity_ids: entities_to_delete.iter().cloned().collect()
			}))
		})
	)?;

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
}

#[try_fn]
#[context("Couldn't handle copy event")]
pub async fn copy(app: &AppHandle, editor_id: Uuid, id: EntityID) -> Result<()> {
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Copying entity {} and its children", id))?;

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

	let reverse_refs = calculate_reverse_references(entity)?;

	let entities_to_copy = get_recursive_children(entity, id, &reverse_refs)?
		.into_iter()
		.collect::<HashSet<_>>();

	let data_to_copy = CopiedEntityData {
		root_entity: id.to_owned(),
		data: entity
			.entities
			.iter()
			.filter(|(x, _)| entities_to_copy.contains(*x))
			.map(|(x, y)| (x.to_owned(), y.to_owned()))
			.collect()
	};

	Clipboard::new()?.set_text(to_string(&data_to_copy)?)?;

	finish_task(app, task)?;
}

#[try_fn]
#[context("Couldn't handle paste event")]
pub async fn paste(
	app: &AppHandle,
	editor_id: Uuid,
	parent_id: String,
	mut paste_data: CopiedEntityData
) -> Result<()> {
	let app_state = app.state::<AppState>();

	let task = start_task(
		app,
		format!(
			"Pasting entity {}",
			paste_data
				.data
				.get(&paste_data.root_entity)
				.context("No such root entity")?
				.name
		)
	)?;

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

	let mut changed_entity_ids = HashMap::new();
	let mut added_external_scenes = 0;

	// Randomise new entity IDs for all subentities contained in the paste data
	for id in paste_data.data.keys() {
		changed_entity_ids.insert(id.to_owned(), random_entity_id());
	}

	// The IDs of all entities in the paste, in both changed and original forms.
	let all_paste_contents = paste_data
		.data
		.keys()
		.cloned()
		.chain(changed_entity_ids.values().cloned())
		.collect::<HashSet<_>>();

	// Change all internal references so they match with the new randomised entity IDs, and also remove any local references that don't exist in the entity we're pasting into
	for (sub_entity_id, sub_entity) in paste_data.data.iter_mut() {
		if paste_data.root_entity != *sub_entity_id {
			// Parent refs are all internal to the paste since the paste is created based on parent hierarchy
			sub_entity.parent = sub_entity
				.parent
				.as_ref()
				.map(|x| x.to_local(changed_entity_ids.get(&x.entity_id).unwrap().to_owned()));
		}

		for property_data in sub_entity.properties.values_mut() {
			visit_variant_mut(&mut property_data.value, &mut |val| {
				if let Variant::Ref(val) = val {
					if let Some(entity_ref) = val {
						*entity_ref = alter_ref_according_to_changelist(entity_ref, &changed_entity_ids);

						// If the ref is external, add the external scene
						if let Some(scene) = &entity_ref.external_scene
							&& !entity.external_scenes.contains(scene)
						{
							entity.external_scenes.push(scene.to_owned());
							added_external_scenes += 1;
						}
					}

					// If the ref is local but to a sub-entity that doesn't exist in the entity we're pasting into (and isn't an internal reference within the paste), set the property to null
					if val
						.as_ref()
						.and_then(Ref::as_local)
						.is_some_and(|x| !entity.entities.contains_key(&x) && !all_paste_contents.contains(&x))
					{
						*val = None;
					}
				}
			});
		}

		for properties in sub_entity.platform_properties.values_mut() {
			for property_data in properties.values_mut() {
				visit_variant_mut(&mut property_data.value, &mut |val| {
					if let Variant::Ref(val) = val {
						if let Some(entity_ref) = val {
							*entity_ref = alter_ref_according_to_changelist(entity_ref, &changed_entity_ids);

							// If the ref is external, add the external scene
							if let Some(scene) = &entity_ref.external_scene
								&& !entity.external_scenes.contains(scene)
							{
								entity.external_scenes.push(scene.to_owned());
								added_external_scenes += 1;
							}
						}

						// If the ref is local but to a sub-entity that doesn't exist in the entity we're pasting into (and isn't an internal reference within the paste), set the property to null
						if val
							.as_ref()
							.and_then(Ref::as_local)
							.is_some_and(|x| !entity.entities.contains_key(&x) && !all_paste_contents.contains(&x))
						{
							*val = None;
						}
					}
				});
			}
		}

		for values in sub_entity.events.values_mut() {
			for refs in values.values_mut() {
				for reference in refs.iter_mut() {
					let underlying_ref = &reference.entity_ref;

					if let Some(scene) = &underlying_ref.external_scene
						&& !entity.external_scenes.contains(scene)
					{
						entity.external_scenes.push(scene.to_owned());
						added_external_scenes += 1;
					}

					*reference = PinConnection {
						entity_ref: alter_ref_according_to_changelist(&reference.entity_ref, &changed_entity_ids),
						value: reference.value.to_owned()
					};
				}

				refs.retain(|reference| {
					let underlying_ref = &reference.entity_ref;

					!underlying_ref
						.as_local()
						.is_some_and(|x| !entity.entities.contains_key(&x) && !all_paste_contents.contains(&x))
				});
			}
		}

		for values in sub_entity
			.input_forwardings
			.values_mut()
			.chain(sub_entity.output_forwardings.values_mut())
		{
			for refs in values.values_mut() {
				for reference in refs.iter_mut() {
					let underlying_ref = reference.entity_id;

					*reference = LocalPinConnection {
						entity_id: changed_entity_ids
							.get(&underlying_ref)
							.copied()
							.unwrap_or(underlying_ref),
						value: reference.value.to_owned()
					};
				}

				refs.retain(|reference| {
					entity.entities.contains_key(&reference.entity_id)
						|| all_paste_contents.contains(&reference.entity_id)
				});
			}
		}

		for aliases in sub_entity.property_aliases.values_mut() {
			for alias_data in aliases.iter_mut() {
				alias_data.original_entity = changed_entity_ids
					.get(&alias_data.original_entity)
					.copied()
					.unwrap_or(alias_data.original_entity);
			}

			aliases.retain(|alias_data| {
				entity.entities.contains_key(&alias_data.original_entity)
					|| all_paste_contents.contains(&alias_data.original_entity)
			});
		}

		for exposed_entity in sub_entity.exposed_entities.values_mut() {
			for reference in exposed_entity.refers_to.iter_mut() {
				*reference = alter_ref_according_to_changelist(reference, &changed_entity_ids);

				if let Some(scene) = &reference.external_scene
					&& !entity.external_scenes.contains(scene)
				{
					entity.external_scenes.push(scene.to_owned());
					added_external_scenes += 1;
				}
			}

			exposed_entity.refers_to.retain(|x| {
				// Only retain those not meeting the criteria for deletion (local ref, not in entity we're pasting into or the paste itself)
				!x.as_local()
					.is_some_and(|x| !entity.entities.contains_key(&x) && !all_paste_contents.contains(&x))
			});
		}

		for referenced_entity in sub_entity.exposed_interfaces.values_mut() {
			*referenced_entity = changed_entity_ids
				.get(referenced_entity)
				.unwrap_or(referenced_entity)
				.to_owned();
		}

		sub_entity
			.exposed_interfaces
			.retain(|_, x| entity.entities.contains_key(x) || all_paste_contents.contains(x));

		for member_of in sub_entity.subsets.values_mut() {
			for parental_entity in member_of.iter_mut() {
				*parental_entity = changed_entity_ids
					.get(parental_entity)
					.unwrap_or(parental_entity)
					.to_owned();
			}

			member_of.retain(|x| entity.entities.contains_key(x) || all_paste_contents.contains(x));
		}
	}

	// Change the actual entity IDs in the paste data
	paste_data.data = paste_data
		.data
		.into_iter()
		.map(|(x, y)| (changed_entity_ids.get(&x).unwrap().to_owned(), y))
		.collect();

	paste_data
		.data
		.get_mut(changed_entity_ids.get(&paste_data.root_entity).unwrap())
		.unwrap()
		.parent = if parent_id == "#" {
		None
	} else {
		let parent_id = parent_id.parse()?;

		paste_data
			.data
			.get_mut(changed_entity_ids.get(&paste_data.root_entity).unwrap())
			.unwrap()
			.parent
			.as_ref()
			.map(|x| x.to_local(parent_id))
			.or_else(|| Some(Ref::local(parent_id)))
	};

	entity.entities.extend(paste_data.data.to_owned());

	let mut new_entities = vec![];
	let reverse_parent_refs = reverse_parent_refs_set(entity);

	for (entity_id, entity_data) in paste_data.data {
		let x = reverse_parent_refs.contains(&entity_id);
		new_entities.push((
			entity_id,
			entity_data.parent,
			entity_data.name.to_owned(),
			entity_data.factory.resource,
			x
		));
	}

	// Make sure the entity being pasted under is updated to be considered a folder (if it's a ZEntity)
	if parent_id != "#" {
		let parent_id: EntityID = parent_id.parse()?;
		new_entities.push((
			parent_id.to_owned(),
			entity
				.entities
				.get(&parent_id)
				.context("No such entity")?
				.parent
				.to_owned(),
			entity
				.entities
				.get(&parent_id)
				.context("No such entity")?
				.name
				.to_owned(),
			entity
				.entities
				.get(&parent_id)
				.context("No such entity")?
				.factory
				.resource
				.to_owned(),
			true
		));
	}

	send_request(
		app,
		Request::Editor(EditorRequest {
			editor: editor_id,
			data: EditorRequestData::Entity(EntityEditorRequest::Tree(EntityTreeRequest::NewItems { new_entities }))
		})
	)?;

	finish_task(app, task)?;

	if added_external_scenes > 0 {
		send_notification(
			app,
			Notification {
				kind: NotificationKind::Info,
				title: "Added external scenes".into(),
				subtitle: format!(
					"{} external scene{} been added to the entity to ensure that pasted references work.",
					added_external_scenes,
					if added_external_scenes > 1 { "s have" } else { " has" }
				)
			}
		)?;
	}

	send_request(
		app,
		Request::Tab(TabRequest {
			tab: editor_id,
			data: TabRequestData::SetUnsaved { unsaved: true }
		})
	)?;

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
}

#[try_fn]
#[context("Couldn't handle search event")]
pub async fn search(app: &AppHandle, editor_id: Uuid, query: String) -> Result<()> {
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Searching for {}", query))?;

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

	send_request(
		app,
		Request::Editor(EditorRequest {
			editor: editor_id,
			data: EditorRequestData::Entity(EntityEditorRequest::Tree(EntityTreeRequest::SearchResults {
				results: entity
					.entities
					.par_iter()
					.filter_map(|(id, ent)| {
						let mut s = format!("{}{}", id, to_string(ent).unwrap());
						s.make_ascii_lowercase();
						query.split(' ').all(|q| s.contains(q)).then_some(*id)
					})
					.collect()
			}))
		})
	)?;

	finish_task(app, task)?;
}

#[try_fn]
#[context("Couldn't handle help menu event")]
pub async fn help_menu(app: &AppHandle, editor_id: Uuid, entity_id: EntityID) -> Result<()> {
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Showing help menu for {}", entity_id))?;

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

	let sub_entity = entity.entities.get(&entity_id).context("No such entity")?;

	if let Some(game) = app_state.game.load().as_ref() {
		let (properties, pins) = if game
			.resource_type(sub_entity.factory.resource)
			.is_some_and(|ty| ty == "TEMP")
		{
			let underlying_entity = game.extract_entity(sub_entity.factory.resource)?;

			(
				game.intellisense()
					.get_properties(game, &underlying_entity, underlying_entity.root_entity, false)?,
				game.intellisense()
					.get_pins(game, &underlying_entity, underlying_entity.root_entity, false)?
			)
		} else {
			(
				game.intellisense().get_properties(game, entity, entity_id, true)?,
				game.intellisense().get_pins(game, entity, entity_id, true)?
			)
		};

		let properties_data_str = {
			let mut buf = Vec::new();
			let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
			let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

			properties
				.into_iter()
				.map(|(name, default_val, post_init)| {
					(
						name,
						json!(Property {
							value: default_val,
							post_init
						})
					)
				})
				.collect::<HashMap<_, _>>()
				.serialize(&mut ser)?;

			String::from_utf8(buf)?
		};

		send_request(
			app,
			Request::Editor(EditorRequest {
				editor: editor_id,
				data: EditorRequestData::Entity(EntityEditorRequest::Tree(EntityTreeRequest::ShowHelpMenu {
					factory: sub_entity.factory.resource.to_owned(),
					input_pins: pins.0,
					output_pins: pins.1,
					default_properties_json: properties_data_str
				}))
			})
		)?;
	} else {
		send_notification(
			app,
			Notification {
				kind: NotificationKind::Error,
				title: "Help menu unavailable".into(),
				subtitle: "A copy of the game hasn't been selected, or the hash list is unavailable.".into()
			}
		)?;
	}

	finish_task(app, task)?;
}

#[try_fn]
#[context("Couldn't handle game browser add event")]
pub async fn add_game_browser_item(app: &AppHandle, editor_id: Uuid, parent_id: String, file: RuntimeID) -> Result<()> {
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Adding {}", file))?;

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

	if let Some(game) = app_state.game.load().as_ref() {
		let resource_type = game.resource_type(file).context("Nonexistent resource")?;
		if is_valid_entity_factory(resource_type) {
			let entity_id = random_entity_id();

			let sub_entity = match resource_type.as_ref() {
				"TEMP" => {
					let (temp_meta, temp_data) = game.extract_latest_resource(file)?;

					let blueprint_index_in_resource_header = match game.version() {
						GameVersion::H1 => {
							hitman_bin1::deserialize::<hitman_bin1::game::h1::STemplateEntity>(&temp_data)
								.context("Couldn't deserialise factory")?
								.blueprint_index_in_resource_header
						}

						GameVersion::H2 => {
							hitman_bin1::deserialize::<hitman_bin1::game::h2::STemplateEntityFactory>(&temp_data)
								.context("Couldn't deserialise factory")?
								.blueprint_index_in_resource_header
						}

						GameVersion::H3 => {
							hitman_bin1::deserialize::<hitman_bin1::game::h3::STemplateEntityFactory>(&temp_data)
								.context("Couldn't deserialise factory")?
								.blueprint_index_in_resource_header
						}
					};

					let blueprint = &temp_meta
						.core_info
						.references
						.get(blueprint_index_in_resource_header as usize)
						.context("Blueprint referenced in factory does not exist in dependencies")?
						.resource;

					SubEntity {
						parent: if parent_id != "#" {
							Some(Ref::local(parent_id.parse()?))
						} else {
							None
						},
						name: file
							.get_path()
							.and_then(|x| {
								x.replace("].pc_entitytype", "")
									.replace("].pc_entitytemplate", "")
									.replace(".entitytemplate", "")
									.split('/')
									.next_back()
									.map(|x| x.to_owned().into())
							})
							.unwrap_or_else(|| file.to_string().into()),
						factory: ResourceReference {
							resource: file,
							flags: Default::default()
						},
						blueprint: blueprint.to_owned(),
						editor_only: Default::default(),
						properties: Default::default(),
						platform_properties: Default::default(),
						events: Default::default(),
						input_forwardings: Default::default(),
						output_forwardings: Default::default(),
						property_aliases: Default::default(),
						exposed_entities: Default::default(),
						exposed_interfaces: Default::default(),
						subsets: Default::default()
					}
				}

				"CPPT" => {
					let (cppt_meta, cppt_data) = game.extract_latest_resource(file)?;

					let blueprint_index_in_resource_header = match game.version() {
						GameVersion::H1 => {
							hitman_bin1::deserialize::<hitman_bin1::game::h1::SCppEntity>(&cppt_data)
								.context("Couldn't deserialise CPPT")?
								.blueprint_index_in_resource_header
						}

						GameVersion::H2 => {
							hitman_bin1::deserialize::<hitman_bin1::game::h2::SCppEntity>(&cppt_data)
								.context("Couldn't deserialise CPPT")?
								.blueprint_index_in_resource_header
						}

						GameVersion::H3 => {
							hitman_bin1::deserialize::<hitman_bin1::game::h3::SCppEntity>(&cppt_data)
								.context("Couldn't deserialise CPPT")?
								.blueprint_index_in_resource_header
						}
					};

					let blueprint = &cppt_meta
						.core_info
						.references
						.get(blueprint_index_in_resource_header as usize)
						.context("Blueprint referenced in factory does not exist in dependencies")?
						.resource;

					SubEntity {
						parent: if parent_id != "#" {
							Some(Ref::local(parent_id.parse()?))
						} else {
							None
						},
						name: file
							.get_path()
							.and_then(|x| {
								x.replace("].pc_entitytype", "")
									.replace("].pc_entitytemplate", "")
									.replace(".entitytemplate", "")
									.split('/')
									.next_back()
									.map(|x| x.to_owned().into())
							})
							.unwrap_or_else(|| file.to_string().into()),
						factory: ResourceReference {
							resource: file,
							flags: Default::default()
						},
						blueprint: blueprint.to_owned(),
						editor_only: Default::default(),
						properties: Default::default(),
						platform_properties: Default::default(),
						events: Default::default(),
						input_forwardings: Default::default(),
						output_forwardings: Default::default(),
						property_aliases: Default::default(),
						exposed_entities: Default::default(),
						exposed_interfaces: Default::default(),
						subsets: Default::default()
					}
				}

				"ASET" => {
					let blueprint = game
						.extract_latest_metadata(file)?
						.core_info
						.references
						.into_iter()
						.next_back()
						.context("ASET had no dependencies")?
						.resource;

					SubEntity {
						parent: if parent_id != "#" {
							Some(Ref::local(parent_id.parse()?))
						} else {
							None
						},
						name: file
							.get_path()
							.and_then(|x| {
								x.replace("].pc_entitytype", "")
									.replace("].pc_entitytemplate", "")
									.replace(".entitytemplate", "")
									.split('/')
									.next_back()
									.map(|x| x.to_owned().into())
							})
							.unwrap_or_else(|| file.to_string().into()),
						factory: ResourceReference {
							resource: file,
							flags: Default::default()
						},
						blueprint,
						editor_only: Default::default(),
						properties: Default::default(),
						platform_properties: Default::default(),
						events: Default::default(),
						input_forwardings: Default::default(),
						output_forwardings: Default::default(),
						property_aliases: Default::default(),
						exposed_entities: Default::default(),
						exposed_interfaces: Default::default(),
						subsets: Default::default()
					}
				}

				"UICT" => {
					let blueprint = game
						.extract_latest_metadata(file)?
						.core_info
						.references
						.into_iter()
						.next_back()
						.context("UICT had no dependencies")?
						.resource;

					SubEntity {
						parent: if parent_id != "#" {
							Some(Ref::local(parent_id.parse()?))
						} else {
							None
						},
						name: file
							.get_path()
							.and_then(|x| {
								x.replace("].pc_entitytype", "")
									.replace("].pc_entitytemplate", "")
									.replace(".entitytemplate", "")
									.split('/')
									.next_back()
									.map(|x| x.to_owned().into())
							})
							.unwrap_or_else(|| file.to_string().into()),
						factory: ResourceReference {
							resource: file,
							flags: Default::default()
						},
						blueprint,
						editor_only: Default::default(),
						properties: Default::default(),
						platform_properties: Default::default(),
						events: Default::default(),
						input_forwardings: Default::default(),
						output_forwardings: Default::default(),
						property_aliases: Default::default(),
						exposed_entities: Default::default(),
						exposed_interfaces: Default::default(),
						subsets: Default::default()
					}
				}

				"MATT" => {
					let blueprint = game
						.extract_latest_metadata(file)?
						.core_info
						.references
						.into_iter()
						.try_find(|dep| {
							anyhow::Ok(game.extract_latest_metadata(dep.resource)?.core_info.resource_type == "MATB")
						})?
						.context("No blueprint dependency found")?
						.resource;

					SubEntity {
						parent: if parent_id != "#" {
							Some(Ref::local(parent_id.parse()?))
						} else {
							None
						},
						name: file
							.get_path()
							.and_then(|x| {
								x.replace("].pc_entitytype", "")
									.replace("].pc_entitytemplate", "")
									.replace(".entitytemplate", "")
									.split('/')
									.next_back()
									.map(|x| x.into())
							})
							.unwrap_or_else(|| file.to_string().into()),
						factory: ResourceReference {
							resource: file,
							flags: Default::default()
						},
						blueprint,
						editor_only: Default::default(),
						properties: Default::default(),
						platform_properties: Default::default(),
						events: Default::default(),
						input_forwardings: Default::default(),
						output_forwardings: Default::default(),
						property_aliases: Default::default(),
						exposed_entities: Default::default(),
						exposed_interfaces: Default::default(),
						subsets: Default::default()
					}
				}

				"WSWT" => {
					let blueprint = game
						.extract_latest_metadata(file)?
						.core_info
						.references
						.into_iter()
						.try_find(|dep| {
							anyhow::Ok({
								let x = game.extract_latest_metadata(dep.resource)?.core_info.resource_type;

								x == "WSWB" || x == "DSWB"
							})
						})?
						.context("No blueprint dependency found")?
						.resource;

					SubEntity {
						parent: if parent_id != "#" {
							Some(Ref::local(parent_id.parse()?))
						} else {
							None
						},
						name: file
							.get_path()
							.and_then(|x| {
								x.replace("].pc_entitytype", "")
									.replace("].pc_entitytemplate", "")
									.replace(".entitytemplate", "")
									.split('/')
									.next_back()
									.map(|x| x.into())
							})
							.unwrap_or_else(|| file.to_string().into()),
						factory: ResourceReference {
							resource: file,
							flags: Default::default()
						},
						blueprint,
						editor_only: Default::default(),
						properties: Default::default(),
						platform_properties: Default::default(),
						events: Default::default(),
						input_forwardings: Default::default(),
						output_forwardings: Default::default(),
						property_aliases: Default::default(),
						exposed_entities: Default::default(),
						exposed_interfaces: Default::default(),
						subsets: Default::default()
					}
				}

				"ECPT" => {
					let blueprint = game
						.extract_latest_metadata(file)?
						.core_info
						.references
						.into_iter()
						.try_find(|dep| {
							anyhow::Ok(game.extract_latest_metadata(dep.resource)?.core_info.resource_type == "ECPB")
						})?
						.context("No blueprint dependency found")?
						.resource;

					SubEntity {
						parent: if parent_id != "#" {
							Some(Ref::local(parent_id.parse()?))
						} else {
							None
						},
						name: file
							.get_path()
							.and_then(|x| {
								x.replace("].pc_entitytype", "")
									.replace("].pc_entitytemplate", "")
									.replace(".entitytemplate", "")
									.split('/')
									.next_back()
									.map(|x| x.into())
							})
							.unwrap_or_else(|| file.to_string().into()),
						factory: ResourceReference {
							resource: file,
							flags: Default::default()
						},
						blueprint,
						editor_only: Default::default(),
						properties: Default::default(),
						platform_properties: Default::default(),
						events: Default::default(),
						input_forwardings: Default::default(),
						output_forwardings: Default::default(),
						property_aliases: Default::default(),
						exposed_entities: Default::default(),
						exposed_interfaces: Default::default(),
						subsets: Default::default()
					}
				}

				"AIBX" => {
					let blueprint = game
						.extract_latest_metadata(file)?
						.core_info
						.references
						.into_iter()
						.try_find(|dep| {
							anyhow::Ok(game.extract_latest_metadata(dep.resource)?.core_info.resource_type == "AIBB")
						})?
						.context("No blueprint dependency found")?
						.resource;

					SubEntity {
						parent: if parent_id != "#" {
							Some(Ref::local(parent_id.parse()?))
						} else {
							None
						},
						name: file
							.get_path()
							.and_then(|x| {
								x.replace("].pc_entitytype", "")
									.replace("].pc_entitytemplate", "")
									.replace(".entitytemplate", "")
									.split('/')
									.next_back()
									.map(|x| x.into())
							})
							.unwrap_or_else(|| file.to_string().into()),
						factory: ResourceReference {
							resource: file,
							flags: Default::default()
						},
						blueprint,
						editor_only: Default::default(),
						properties: Default::default(),
						platform_properties: Default::default(),
						events: Default::default(),
						input_forwardings: Default::default(),
						output_forwardings: Default::default(),
						property_aliases: Default::default(),
						exposed_entities: Default::default(),
						exposed_interfaces: Default::default(),
						subsets: Default::default()
					}
				}

				"WSGT" => {
					let blueprint = game
						.extract_latest_metadata(file)?
						.core_info
						.references
						.into_iter()
						.try_find(|dep| {
							anyhow::Ok(game.extract_latest_metadata(dep.resource)?.core_info.resource_type == "WSGB")
						})?
						.context("No blueprint dependency found")?
						.resource;

					SubEntity {
						parent: if parent_id != "#" {
							Some(Ref::local(parent_id.parse()?))
						} else {
							None
						},
						name: file
							.get_path()
							.and_then(|x| {
								x.replace("].pc_entitytype", "")
									.replace("].pc_entitytemplate", "")
									.replace(".entitytemplate", "")
									.split('/')
									.next_back()
									.map(|x| x.into())
							})
							.unwrap_or_else(|| file.to_string().into()),
						factory: ResourceReference {
							resource: file,
							flags: Default::default()
						},
						blueprint,
						editor_only: Default::default(),
						properties: Default::default(),
						platform_properties: Default::default(),
						events: Default::default(),
						input_forwardings: Default::default(),
						output_forwardings: Default::default(),
						property_aliases: Default::default(),
						exposed_entities: Default::default(),
						exposed_interfaces: Default::default(),
						subsets: Default::default()
					}
				}

				_ => unreachable!()
			};

			send_request(
				app,
				Request::Editor(EditorRequest {
					editor: editor_id.to_owned(),
					data: EditorRequestData::Entity(EntityEditorRequest::Tree(EntityTreeRequest::NewItems {
						new_entities: vec![(
							entity_id.to_owned(),
							sub_entity.parent.to_owned(),
							sub_entity.name.to_owned(),
							sub_entity.factory.resource.to_owned(),
							false
						)]
					}))
				})
			)?;

			entity.entities.insert(entity_id, sub_entity);

			send_request(
				app,
				Request::Tab(TabRequest {
					tab: editor_id,
					data: TabRequestData::SetUnsaved { unsaved: true }
				})
			)?;
		} else if resource_type == "WWEV" {
			let (wwev_meta, wwev_data) = game.extract_latest_resource(file)?;

			let wwev = WwiseEvent::parse(&wwev_data, &wwev_meta.core_info)?;

			let entity_id = random_entity_id();

			let sub_entity = SubEntity {
				parent: if parent_id != "#" {
					Some(Ref::local(parent_id.parse()?))
				} else {
					None
				},
				name: wwev.name.into(),
				factory: ResourceReference {
					resource: rid!("[modules:/zaudioevententity.class].pc_entitytype"),
					flags: Default::default()
				},
				blueprint: rid!("[modules:/zaudioevententity.class].pc_entityblueprint"),
				editor_only: Default::default(),
				properties: {
					let mut properties = OrderMap::new();
					properties.insert(
						"m_pMainEvent".into(),
						Property {
							value: Variant::Resource(Some(ResourceReference {
								resource: file,
								flags: ReferenceFlags {
									reference_type: ReferenceType::Normal,
									..Default::default()
								}
							})),
							post_init: false
						}
					);
					properties
				},
				platform_properties: Default::default(),
				events: Default::default(),
				input_forwardings: Default::default(),
				output_forwardings: Default::default(),
				property_aliases: Default::default(),
				exposed_entities: Default::default(),
				exposed_interfaces: Default::default(),
				subsets: Default::default()
			};

			send_request(
				app,
				Request::Editor(EditorRequest {
					editor: editor_id.to_owned(),
					data: EditorRequestData::Entity(EntityEditorRequest::Tree(EntityTreeRequest::NewItems {
						new_entities: vec![(
							entity_id.to_owned(),
							sub_entity.parent.to_owned(),
							sub_entity.name.to_owned(),
							sub_entity.factory.resource.to_owned(),
							false
						)]
					}))
				})
			)?;

			entity.entities.insert(entity_id, sub_entity);

			send_request(
				app,
				Request::Tab(TabRequest {
					tab: editor_id,
					data: TabRequestData::SetUnsaved { unsaved: true }
				})
			)?;
		} else {
			send_notification(
				app,
				Notification {
					kind: NotificationKind::Error,
					title: "Not a valid template".into(),
					subtitle: "Only entity templates can be dragged into the entity tree.".into()
				}
			)?;
		}
	} else {
		send_notification(
			app,
			Notification {
				kind: NotificationKind::Error,
				title: "Game data unavailable".into(),
				subtitle: "A copy of the game hasn't been selected, or the hash list is unavailable.".into()
			}
		)?;
	}

	finish_task(app, task)?;

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
}

#[try_fn]
#[context("Couldn't handle select entity in editor event")]
pub async fn select_entity_in_editor(app: &AppHandle, editor_id: Uuid, entity_id: EntityID) -> Result<()> {
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Selecting {} in editor", entity_id))?;

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

	app_state
		.editor_connection
		.select_entity(entity_id, &entity.blueprint.to_hash())
		.await?;

	finish_task(app, task)?;
}

#[try_fn]
#[context("Couldn't handle move entity to player event")]
pub async fn move_entity_to_player(app: &AppHandle, editor_id: Uuid, entity_id: EntityID) -> Result<()> {
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Moving {} to player position", entity_id))?;

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

	let player_transform = app_state.editor_connection.get_player_transform().await?;

	if entity
		.entities
		.get_mut(&entity_id)
		.context("No such entity")?
		.properties
		.remove("m_eidParent")
		.is_some()
	{
		app_state
			.editor_connection
			.set_property(
				entity_id,
				&entity.blueprint.to_hash(),
				"m_eidParent",
				Variant::Ref(None)
			)
			.await?;
	}

	let property = entity
		.entities
		.get_mut(&entity_id)
		.context("No such entity")?
		.properties
		.entry("m_mTransform".into())
		.or_insert(Property {
			value: Variant::Transform(Transform {
				rotation: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
				position: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
				scale: None
			}),
			post_init: false
		});

	let Variant::Transform(transform) = &mut property.value else {
		Err(anyhow!("Entity {entity_id}'s transform property is of the wrong type"))?;
		panic!();
	};

	transform.position.x = player_transform.position.x;
	transform.position.y = player_transform.position.y;
	transform.position.z = player_transform.position.z;

	app_state
		.editor_connection
		.set_property(
			entity_id,
			&entity.blueprint.to_hash(),
			"m_mTransform",
			property.value.to_owned()
		)
		.await?;

	if let Some(game) = app_state.game.load().as_ref()
		&& game
			.intellisense()
			.get_properties(game, entity, entity_id, true)?
			.into_iter()
			.any(|(name, _, _)| name == "m_eRoomBehaviour")
	{
		entity
			.entities
			.get_mut(&entity_id)
			.context("No such entity")?
			.properties
			.insert(
				EcoString::from("m_eRoomBehaviour"),
				Property {
					value: Variant::from_raw(&ZVariant::new(ZSpatialEntity_ERoomBehaviour::ROOM_DYNAMIC)),
					post_init: false
				}
			);

		app_state
			.editor_connection
			.set_property(
				entity_id,
				&entity.blueprint.to_hash(),
				"m_eRoomBehaviour",
				Variant::from_raw(&ZVariant::new(ZSpatialEntity_ERoomBehaviour::ROOM_DYNAMIC))
			)
			.await?;
	}

	send_request(
		app,
		Request::Tab(TabRequest {
			tab: editor_id,
			data: TabRequestData::SetUnsaved { unsaved: true }
		})
	)?;

	send_request(
		app,
		Request::Editor(EditorRequest {
			editor: editor_id.to_owned(),
			data: EditorRequestData::Entity(EntityEditorRequest::Monaco(
				EntityMonacoRequest::ReplaceContentIfSameEntityID {
					entity_id,
					content: to_string_clear(entity.entities.get(&entity_id).context("No such entity")?)?
				}
			))
		})
	)?;

	finish_task(app, task)?;

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
}

#[try_fn]
#[context("Couldn't handle rotate entity as player event")]
pub async fn rotate_entity_as_player(app: &AppHandle, editor_id: Uuid, entity_id: EntityID) -> Result<()> {
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Adjusting {} to player rotation", entity_id))?;

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

	let player_transform = app_state.editor_connection.get_player_transform().await?;

	if entity
		.entities
		.get_mut(&entity_id)
		.context("No such entity")?
		.properties
		.remove("m_eidParent")
		.is_some()
	{
		app_state
			.editor_connection
			.set_property(
				entity_id,
				&entity.blueprint.to_hash(),
				"m_eidParent",
				Variant::Ref(None)
			)
			.await?;
	}

	let property = entity
		.entities
		.get_mut(&entity_id)
		.context("No such entity")?
		.properties
		.entry("m_mTransform".into())
		.or_insert(Property {
			value: Variant::Transform(Transform {
				rotation: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
				position: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
				scale: None
			}),
			post_init: false
		});

	let Variant::Transform(transform) = &mut property.value else {
		Err(anyhow!("Entity {entity_id}'s transform property is of the wrong type"))?;
		panic!();
	};

	transform.rotation.x = player_transform.rotation.x;
	transform.rotation.y = player_transform.rotation.y;
	transform.rotation.z = player_transform.rotation.z;

	app_state
		.editor_connection
		.set_property(
			entity_id,
			&entity.blueprint.to_hash(),
			"m_mTransform",
			property.value.to_owned()
		)
		.await?;

	if let Some(game) = app_state.game.load().as_ref()
		&& game
			.intellisense()
			.get_properties(game, entity, entity_id, true)?
			.into_iter()
			.any(|(name, _, _)| name == "m_eRoomBehaviour")
	{
		entity
			.entities
			.get_mut(&entity_id)
			.context("No such entity")?
			.properties
			.insert(
				EcoString::from("m_eRoomBehaviour"),
				Property {
					value: Variant::from_raw(&ZVariant::new(ZSpatialEntity_ERoomBehaviour::ROOM_DYNAMIC)),
					post_init: false
				}
			);

		app_state
			.editor_connection
			.set_property(
				entity_id,
				&entity.blueprint.to_hash(),
				"m_eRoomBehaviour",
				Variant::from_raw(&ZVariant::new(ZSpatialEntity_ERoomBehaviour::ROOM_DYNAMIC))
			)
			.await?;
	}

	send_request(
		app,
		Request::Tab(TabRequest {
			tab: editor_id,
			data: TabRequestData::SetUnsaved { unsaved: true }
		})
	)?;

	send_request(
		app,
		Request::Editor(EditorRequest {
			editor: editor_id.to_owned(),
			data: EditorRequestData::Entity(EntityEditorRequest::Monaco(
				EntityMonacoRequest::ReplaceContentIfSameEntityID {
					entity_id,
					content: to_string_clear(entity.entities.get(&entity_id).context("No such entity")?)?
				}
			))
		})
	)?;

	finish_task(app, task)?;

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
}

#[try_fn]
#[context("Couldn't handle move entity to camera event")]
pub async fn move_entity_to_camera(app: &AppHandle, editor_id: Uuid, entity_id: EntityID) -> Result<()> {
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Moving {} to camera position", entity_id))?;

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

	let camera_transform = app_state.editor_connection.get_camera_transform().await?;

	if entity
		.entities
		.get_mut(&entity_id)
		.context("No such entity")?
		.properties
		.remove("m_eidParent")
		.is_some()
	{
		app_state
			.editor_connection
			.set_property(
				entity_id,
				&entity.blueprint.to_hash(),
				"m_eidParent",
				Variant::Ref(None)
			)
			.await?;
	}

	let property = entity
		.entities
		.get_mut(&entity_id)
		.context("No such entity")?
		.properties
		.entry("m_mTransform".into())
		.or_insert(Property {
			value: Variant::Transform(Transform {
				rotation: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
				position: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
				scale: None
			}),
			post_init: false
		});

	let Variant::Transform(transform) = &mut property.value else {
		Err(anyhow!("Entity {entity_id}'s transform property is of the wrong type"))?;
		panic!();
	};

	transform.position.x = camera_transform.position.x;
	transform.position.y = camera_transform.position.y;
	transform.position.z = camera_transform.position.z;

	app_state
		.editor_connection
		.set_property(
			entity_id,
			&entity.blueprint.to_hash(),
			"m_mTransform",
			property.value.to_owned()
		)
		.await?;

	if let Some(game) = app_state.game.load().as_ref()
		&& game
			.intellisense()
			.get_properties(game, entity, entity_id, true)?
			.into_iter()
			.any(|(name, _, _)| name == "m_eRoomBehaviour")
	{
		entity
			.entities
			.get_mut(&entity_id)
			.context("No such entity")?
			.properties
			.insert(
				EcoString::from("m_eRoomBehaviour"),
				Property {
					value: Variant::from_raw(&ZVariant::new(ZSpatialEntity_ERoomBehaviour::ROOM_DYNAMIC)),
					post_init: false
				}
			);

		app_state
			.editor_connection
			.set_property(
				entity_id,
				&entity.blueprint.to_hash(),
				"m_eRoomBehaviour",
				Variant::from_raw(&ZVariant::new(ZSpatialEntity_ERoomBehaviour::ROOM_DYNAMIC))
			)
			.await?;
	}

	send_request(
		app,
		Request::Tab(TabRequest {
			tab: editor_id,
			data: TabRequestData::SetUnsaved { unsaved: true }
		})
	)?;

	send_request(
		app,
		Request::Editor(EditorRequest {
			editor: editor_id.to_owned(),
			data: EditorRequestData::Entity(EntityEditorRequest::Monaco(
				EntityMonacoRequest::ReplaceContentIfSameEntityID {
					entity_id,
					content: to_string_clear(entity.entities.get(&entity_id).context("No such entity")?)?
				}
			))
		})
	)?;

	finish_task(app, task)?;

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
}

#[try_fn]
#[context("Couldn't handle rotate entity as camera event")]
pub async fn rotate_entity_as_camera(app: &AppHandle, editor_id: Uuid, entity_id: EntityID) -> Result<()> {
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Adjusting {} to camera rotation", entity_id))?;

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

	let camera_transform = app_state.editor_connection.get_camera_transform().await?;

	let property = entity
		.entities
		.get_mut(&entity_id)
		.context("No such entity")?
		.properties
		.entry("m_mTransform".into())
		.or_insert(Property {
			value: Variant::Transform(Transform {
				rotation: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
				position: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
				scale: None
			}),
			post_init: false
		});

	let Variant::Transform(transform) = &mut property.value else {
		Err(anyhow!("Entity {entity_id}'s transform property is of the wrong type"))?;
		panic!();
	};

	transform.rotation.x = camera_transform.rotation.x;
	transform.rotation.y = camera_transform.rotation.y;
	transform.rotation.z = camera_transform.rotation.z;

	app_state
		.editor_connection
		.set_property(
			entity_id,
			&entity.blueprint.to_hash(),
			"m_mTransform",
			property.value.to_owned()
		)
		.await?;

	if let Some(game) = app_state.game.load().as_ref()
		&& game
			.intellisense()
			.get_properties(game, entity, entity_id, true)?
			.into_iter()
			.any(|(name, _, _)| name == "m_eRoomBehaviour")
	{
		entity
			.entities
			.get_mut(&entity_id)
			.context("No such entity")?
			.properties
			.insert(
				EcoString::from("m_eRoomBehaviour"),
				Property {
					value: Variant::from_raw(&ZVariant::new(ZSpatialEntity_ERoomBehaviour::ROOM_DYNAMIC)),
					post_init: false
				}
			);

		app_state
			.editor_connection
			.set_property(
				entity_id,
				&entity.blueprint.to_hash(),
				"m_eRoomBehaviour",
				Variant::from_raw(&ZVariant::new(ZSpatialEntity_ERoomBehaviour::ROOM_DYNAMIC))
			)
			.await?;
	}

	send_request(
		app,
		Request::Tab(TabRequest {
			tab: editor_id,
			data: TabRequestData::SetUnsaved { unsaved: true }
		})
	)?;

	send_request(
		app,
		Request::Editor(EditorRequest {
			editor: editor_id.to_owned(),
			data: EditorRequestData::Entity(EntityEditorRequest::Monaco(
				EntityMonacoRequest::ReplaceContentIfSameEntityID {
					entity_id,
					content: to_string_clear(entity.entities.get(&entity_id).context("No such entity")?)?
				}
			))
		})
	)?;

	finish_task(app, task)?;

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
}

#[try_fn]
#[context("Couldn't handle restore to original event")]
pub async fn restore_to_original(app: &AppHandle, editor_id: Uuid, entity_id: EntityID) -> Result<()> {
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Reverting {} to original state", entity_id))?;

	let mut editor_state = app_state
		.editor_states
		.get_mut(&editor_id)
		.await
		.context("No such editor")?;

	let EditorData::QNPatch {
		ref base,
		ref mut current,
		..
	} = editor_state.data
	else {
		Err(anyhow!("Editor {} is not a QN patch editor", editor_id))?;
		panic!();
	};

	if let EditorValidity::Invalid(err) = check_local_references_exist(
		base.entities.get(&entity_id).context("Entity didn't exist in base")?,
		current
	)? {
		send_notification(
			app,
			Notification {
				kind: NotificationKind::Error,
				title: "Entity would be invalid".into(),
				subtitle: err
			}
		)?;

		finish_task(app, task)?;
		return Ok(());
	}

	if let Some(previous) = current.entities.get(&entity_id).cloned() {
		current.entities.insert(
			entity_id.to_owned(),
			base.entities
				.get(&entity_id)
				.context("Entity didn't exist in base")?
				.to_owned()
		);

		let sub_entity = current.entities.get(&entity_id).context("No such entity")?.to_owned();

		let reverse_parent_refs = reverse_parent_refs_set(current);

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
				editor: editor_id.to_owned(),
				data: EditorRequestData::Entity(EntityEditorRequest::Monaco(
					EntityMonacoRequest::ReplaceContentIfSameEntityID {
						entity_id,
						content: to_string_clear(&sub_entity)?
					}
				))
			})
		)?;

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
					app_state
						.editor_connection
						.set_property(entity_id, &current.blueprint.to_hash(), property, val.value.to_owned())
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
							.get_properties(game, current, entity_id, false)?
							.into_iter()
							.find(|(name, _, _)| *name == property)
					{
						debug!(
							"Syncing removed property {} for entity {} with default value according to intellisense",
							property, entity_id
						);

						app_state
							.editor_connection
							.set_property(entity_id, &current.blueprint.to_hash(), &property, def_val)
							.await?;
					}
				}
			}
		}
	} else {
		current.entities.insert(
			entity_id.to_owned(),
			base.entities
				.get(&entity_id)
				.context("Entity didn't exist in base")?
				.to_owned()
		);

		let sub_entity = current.entities.get(&entity_id).context("No such entity")?.to_owned();

		let reverse_parent_refs = reverse_parent_refs_set(current);

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
	}

	send_request(
		app,
		Request::Tab(TabRequest {
			tab: editor_id,
			data: TabRequestData::SetUnsaved { unsaved: true }
		})
	)?;

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
}
