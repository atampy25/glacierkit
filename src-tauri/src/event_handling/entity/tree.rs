use std::ops::Deref;

use anyhow::{anyhow, Context, Result};
use arboard::Clipboard;
use arc_swap::ArcSwap;
use fn_error_context::context;
use hashbrown::{HashMap, HashSet};
use hitman_commons::{game::GameVersion, metadata::ResourceID};
use hitman_formats::wwev::WwiseEvent;
use indexmap::IndexMap;
use itertools::Itertools;
use log::debug;
use quickentity_rs::{
	apply_patch,
	patch_structs::{Patch, PatchOperation, SubEntityOperation},
	qn_structs::{FullRef, Property, Ref, RefMaybeConstantValue, RefWithConstantValue, SubEntity}
};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde::Serialize;
use serde_json::{from_slice, from_str, from_value, json, to_string, to_value, Value};
use tauri::{AppHandle, Manager};
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	editor_connection::PropertyValue,
	entity::{
		alter_ref_according_to_changelist, calculate_reverse_references, change_reference_to_local,
		check_local_references_exist, get_decorations, get_diff_info, get_local_reference, get_recursive_children,
		is_valid_entity_factory, random_entity_id, CopiedEntityData, ReverseReferenceData
	},
	finish_task, get_loaded_game_version,
	model::{
		AppSettings, AppState, EditorData, EditorRequest, EditorValidity, EntityEditorRequest, EntityGeneralRequest,
		EntityMetaPaneRequest, EntityMonacoRequest, EntityTreeEvent, EntityTreeRequest, GlobalRequest, Request
	},
	resourcelib::{
		h2016_convert_binary_to_factory, h2016_convert_cppt, h2_convert_binary_to_factory, h2_convert_cppt,
		h3_convert_binary_to_factory, h3_convert_cppt
	},
	rpkg::{extract_entity, extract_latest_metadata, extract_latest_resource},
	send_notification, send_request, start_task, Notification, NotificationKind
};

use super::monaco::SAFE_TO_SYNC;

#[try_fn]
#[context("Couldn't handle tree event")]
pub async fn handle(app: &AppHandle, event: EntityTreeEvent) -> Result<()> {
	match event {
		EntityTreeEvent::Initialise { editor_id } => {
			initialise(app, editor_id).await?;
		}

		EntityTreeEvent::Select { editor_id, id } => {
			select(app, editor_id, id).await?;
		}

		EntityTreeEvent::Create { editor_id, id, content } => {
			create(app, editor_id, id, content).await?;
		}

		EntityTreeEvent::Delete { editor_id, id } => {
			delete(app, editor_id, id).await?;
		}

		EntityTreeEvent::Rename {
			editor_id,
			id,
			new_name
		} => {
			rename(app, editor_id, id, new_name).await?;
		}

		EntityTreeEvent::Reparent {
			editor_id,
			id,
			new_parent
		} => {
			reparent(app, editor_id, id, new_parent).await?;
		}

		EntityTreeEvent::Copy { editor_id, id } => {
			copy(app, editor_id, id).await?;
		}

		EntityTreeEvent::Paste { editor_id, parent_id } => {
			paste(
				app,
				editor_id,
				parent_id,
				from_str::<CopiedEntityData>(&Clipboard::new()?.get_text()?)?
			)
			.await?;
		}

		EntityTreeEvent::Search { editor_id, query } => {
			search(app, editor_id, query).await?;
		}

		EntityTreeEvent::ShowHelpMenu { editor_id, entity_id } => {
			help_menu(app, editor_id, entity_id).await?;
		}

		EntityTreeEvent::UseTemplate {
			editor_id,
			parent_id,
			template
		} => {
			paste(app, editor_id, parent_id, template).await?;
		}

		EntityTreeEvent::AddGameBrowserItem {
			editor_id,
			parent_id,
			file
		} => {
			add_game_browser_item(app, editor_id, parent_id, file).await?;
		}

		EntityTreeEvent::SelectEntityInEditor { editor_id, entity_id } => {
			select_entity_in_editor(app, editor_id, entity_id).await?;
		}

		EntityTreeEvent::MoveEntityToPlayer { editor_id, entity_id } => {
			move_entity_to_player(app, editor_id, entity_id).await?;
		}

		EntityTreeEvent::RotateEntityAsPlayer { editor_id, entity_id } => {
			rotate_entity_as_player(app, editor_id, entity_id).await?;
		}

		EntityTreeEvent::MoveEntityToCamera { editor_id, entity_id } => {
			move_entity_to_camera(app, editor_id, entity_id).await?;
		}

		EntityTreeEvent::RotateEntityAsCamera { editor_id, entity_id } => {
			rotate_entity_as_camera(app, editor_id, entity_id).await?;
		}

		EntityTreeEvent::RestoreToOriginal { editor_id, entity_id } => {
			restore_to_original(app, editor_id, entity_id).await?;
		}
	}
}

#[try_fn]
#[context("Couldn't handle initialise event")]
pub async fn initialise(app: &AppHandle, editor_id: Uuid) -> Result<()> {
	let app_state = app.state::<AppState>();

	let editor_state = app_state.editor_states.get(&editor_id).context("No such editor")?;

	let entity = match editor_state.data {
		EditorData::QNEntity { ref entity, .. } => entity,
		EditorData::QNPatch { ref current, .. } => current,

		_ => {
			Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
			panic!();
		}
	};

	let mut entities = vec![];
	let mut reverse_parent_refs: HashMap<String, Vec<String>> = HashMap::new();

	for (entity_id, entity_data) in entity.entities.iter() {
		match entity_data.parent {
			Ref::Full(ref reference) if reference.external_scene.is_none() => {
				reverse_parent_refs
					.entry(reference.entity_ref.to_owned())
					.and_modify(|x| x.push(entity_id.to_owned()))
					.or_insert(vec![entity_id.to_owned()]);
			}

			Ref::Short(Some(ref reference)) => {
				reverse_parent_refs
					.entry(reference.to_owned())
					.and_modify(|x| x.push(entity_id.to_owned()))
					.or_insert(vec![entity_id.to_owned()]);
			}

			_ => {}
		}
	}

	for (entity_id, entity_data) in entity.entities.iter() {
		entities.push((
			entity_id.to_owned(),
			entity_data.parent.to_owned(),
			entity_data.name.to_owned(),
			entity_data.factory.to_owned(),
			reverse_parent_refs.contains_key(entity_id)
		));
	}

	send_request(
		app,
		Request::Editor(EditorRequest::Entity(EntityEditorRequest::General(
			EntityGeneralRequest::SetIsPatchEditor {
				editor_id: editor_id.to_owned(),
				is_patch_editor: matches!(editor_state.data, EditorData::QNPatch { .. })
			}
		)))
	)?;

	send_request(
		app,
		Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
			EntityTreeRequest::NewTree {
				editor_id: editor_id.to_owned(),
				entities
			}
		)))
	)?;

	send_request(
		app,
		Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
			EntityTreeRequest::SetTemplates {
				editor_id: editor_id.to_owned(),
				templates: from_slice(include_bytes!("../../../assets/templates.json")).unwrap()
			}
		)))
	)?;

	let editor_connected = app_state.editor_connection.is_connected().await;

	send_request(
		app,
		Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
			EntityTreeRequest::SetEditorConnectionAvailable {
				editor_id: editor_id.to_owned(),
				editor_connection_available: editor_connected
			}
		)))
	)?;

	send_request(
		app,
		Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
			EntityMonacoRequest::SetEditorConnected {
				editor_id: editor_id.to_owned(),
				connected: editor_connected
			}
		)))
	)?;

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
}

#[try_fn]
#[context("Couldn't handle create event")]
pub async fn create(app: &AppHandle, editor_id: Uuid, id: String, content: SubEntity) -> Result<()> {
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

	entity.entities.insert(id, content);

	send_request(
		app,
		Request::Global(GlobalRequest::SetTabUnsaved {
			id: editor_id.to_owned(),
			unsaved: true
		})
	)?;

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
}

#[try_fn]
#[context("Couldn't handle rename event")]
pub async fn rename(app: &AppHandle, editor_id: Uuid, id: String, new_name: String) -> Result<()> {
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

	entity.entities.get_mut(&id).context("No such entity")?.name = new_name;

	send_request(
		app,
		Request::Global(GlobalRequest::SetTabUnsaved {
			id: editor_id,
			unsaved: true
		})
	)?;

	let mut buf = Vec::new();
	let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
	let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

	entity
		.entities
		.get(&id)
		.context("No such entity")?
		.serialize(&mut ser)?;

	send_request(
		app,
		Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
			EntityMonacoRequest::ReplaceContentIfSameEntityID {
				editor_id: editor_id.to_owned(),
				entity_id: id.to_owned(),
				content: String::from_utf8(buf)?
			}
		)))
	)?;

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
}

#[try_fn]
#[context("Couldn't handle select event")]
pub async fn select(app: &AppHandle, editor_id: Uuid, id: String) -> Result<()> {
	let app_settings = app.state::<ArcSwap<AppSettings>>();
	let app_state = app.state::<AppState>();

	let editor_state = app_state.editor_states.get(&editor_id).context("No such editor")?;

	let entity = match editor_state.data {
		EditorData::QNEntity { ref entity, .. } => entity,
		EditorData::QNPatch { ref current, .. } => current,

		_ => {
			Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
			panic!();
		}
	};

	let task = start_task(app, format!("Selecting {}", id))?;

	let mut buf = Vec::new();
	let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
	let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

	entity
		.entities
		.get(&id)
		.context("No such entity")?
		.serialize(&mut ser)?;

	send_request(
		app,
		Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
			EntityMonacoRequest::ReplaceContent {
				editor_id: editor_id.to_owned(),
				entity_id: id.to_owned(),
				content: String::from_utf8(buf)?
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
		Request::Editor(EditorRequest::Entity(EntityEditorRequest::MetaPane(
			EntityMetaPaneRequest::SetReverseRefs {
				editor_id: editor_id.to_owned(),
				entity_names: reverse_refs
					.iter()
					.filter(|x| settings.show_reverse_parent_refs || !matches!(x.data, ReverseReferenceData::Parent))
					.map(|x| (x.from.to_owned(), entity.entities.get(&x.from).unwrap().name.to_owned()))
					.collect(),
				reverse_refs: reverse_refs
					.into_iter()
					.filter(|x| settings.show_reverse_parent_refs || !matches!(x.data, ReverseReferenceData::Parent))
					.collect()
			}
		)))
	)?;

	send_request(
		app,
		Request::Editor(EditorRequest::Entity(EntityEditorRequest::MetaPane(
			EntityMetaPaneRequest::SetNotes {
				editor_id: editor_id.to_owned(),
				entity_id: id.to_owned(),
				notes: entity
					.comments
					.iter()
					.find(|x| matches!(x.parent, Ref::Short(Some(ref x)) if *x == id))
					.map(|x| x.text.deref())
					.unwrap_or("")
					.into()
			}
		)))
	)?;

	finish_task(app, task)?;

	if let Some(intellisense) = app_state.intellisense.load().as_ref()
		&& let Some(game_files) = app_state.game_files.load().as_ref()
		&& let Some(hash_list) = app_state.hash_list.load().as_ref()
		&& let Some(install) = app_settings.load().game_install.as_ref()
		&& let Some(repository) = app_state.repository.load().as_ref()
		&& let Some(tonytools_hash_list) = app_state.tonytools_hash_list.load().as_ref()
	{
		let game_version = get_loaded_game_version(app, install)?;

		let task = start_task(app, format!("Gathering intellisense data for {}", id))?;

		let (properties, pins) = rayon::join(
			|| {
				intellisense.get_properties(
					game_files,
					&app_state.cached_entities,
					hash_list,
					game_version,
					entity,
					&id,
					true
				)
			},
			|| {
				intellisense.get_pins(
					game_files,
					&app_state.cached_entities,
					hash_list,
					game_version,
					entity,
					&id,
					false
				)
			}
		);

		send_request(
			app,
			Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
				EntityMonacoRequest::UpdateIntellisense {
					editor_id: editor_id.to_owned(),
					entity_id: id.to_owned(),
					properties: properties?,
					pins: pins?
				}
			)))
		)?;

		finish_task(app, task)?;

		let task = start_task(app, format!("Computing decorations for {}", id))?;

		let decorations = get_decorations(
			game_files,
			&app_state.cached_entities,
			repository,
			hash_list,
			game_version,
			tonytools_hash_list,
			entity.entities.get(&id).context("No such entity")?,
			entity
		)?;

		send_request(
			app,
			Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
				EntityMonacoRequest::UpdateDecorationsAndMonacoInfo {
					editor_id: editor_id.to_owned(),
					entity_id: id.to_owned(),
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

	let task = start_task(app, format!("Selecting {} in editor", id))?;

	if app_state.editor_connection.is_connected().await {
		app_state
			.editor_connection
			.select_entity(&id, &entity.blueprint_hash)
			.await?;
	}

	finish_task(app, task)?;
}

#[try_fn]
#[context("Couldn't handle reparent event")]
pub async fn reparent(app: &AppHandle, editor_id: Uuid, id: String, new_parent: Ref) -> Result<()> {
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

	entity.entities.get_mut(&id).context("No such entity")?.parent = new_parent;

	send_request(
		app,
		Request::Global(GlobalRequest::SetTabUnsaved {
			id: editor_id,
			unsaved: true
		})
	)?;

	let mut buf = Vec::new();
	let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
	let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

	entity
		.entities
		.get(&id)
		.context("No such entity")?
		.serialize(&mut ser)?;

	send_request(
		app,
		Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
			EntityMonacoRequest::ReplaceContentIfSameEntityID {
				editor_id: editor_id.to_owned(),
				entity_id: id.to_owned(),
				content: String::from_utf8(buf)?
			}
		)))
	)?;

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
}

#[try_fn]
#[context("Couldn't handle delete event")]
pub async fn delete(app: &AppHandle, editor_id: Uuid, id: String) -> Result<()> {
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Deleting entity {}", id))?;

	let mut editor_state = app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

	let entity = match editor_state.data {
		EditorData::QNEntity { ref mut entity, .. } => entity,
		EditorData::QNPatch { ref mut current, .. } => current,

		_ => {
			Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
			panic!();
		}
	};

	let reverse_refs = calculate_reverse_references(entity)?;

	let entities_to_delete = get_recursive_children(entity, &id, &reverse_refs)?
		.into_iter()
		.collect::<HashSet<_>>();

	let mut patch = Patch {
		factory_hash: String::new(),
		blueprint_hash: String::new(),
		patch: vec![],
		patch_version: 6
	};

	let mut refs_deleted = 0;

	for entity_to_delete in &entities_to_delete {
		for reverse_ref in reverse_refs.get(entity_to_delete).context("No such entity")? {
			match &reverse_ref.data {
				ReverseReferenceData::Parent => {
					// The entity itself will be deleted later
				}

				ReverseReferenceData::Property { property_name } => {
					let entity_props = entity
						.entities
						.get_mut(&reverse_ref.from)
						.unwrap()
						.properties
						.as_mut()
						.unwrap();

					if entity_props.get(property_name).unwrap().property_type == "SEntityTemplateReference" {
						entity_props.shift_remove(property_name).unwrap();
					} else {
						entity_props
							.get_mut(property_name)
							.unwrap()
							.value
							.as_array_mut()
							.unwrap()
							.retain(|item| {
								if let Some(local_ref) = get_local_reference(
									&from_value::<Ref>(item.to_owned())
										.expect("Already done in reverse refs so no error here")
								) {
									local_ref != *entity_to_delete
								} else {
									true
								}
							});
					}
				}

				ReverseReferenceData::PlatformSpecificProperty {
					property_name,
					platform
				} => {
					let entity_props = entity
						.entities
						.get_mut(&reverse_ref.from)
						.unwrap()
						.platform_specific_properties
						.as_mut()
						.unwrap()
						.get_mut(platform)
						.unwrap();

					if entity_props.get(property_name).unwrap().property_type == "SEntityTemplateReference" {
						entity_props.shift_remove(property_name).unwrap();
					} else {
						entity_props
							.get_mut(property_name)
							.unwrap()
							.value
							.as_array_mut()
							.unwrap()
							.retain(|item| {
								if let Some(local_ref) = get_local_reference(
									&from_value::<Ref>(item.to_owned())
										.expect("Already done in reverse refs so no error here")
								) {
									local_ref != *entity_to_delete
								} else {
									true
								}
							});
					}
				}

				ReverseReferenceData::Event { event, trigger } => {
					patch.patch.push(PatchOperation::SubEntityOperation(
						reverse_ref.from.to_owned(),
						SubEntityOperation::RemoveEventConnection(
							event.to_owned(),
							trigger.to_owned(),
							entity
								.entities
								.get(&reverse_ref.from)
								.unwrap()
								.events
								.as_ref()
								.unwrap()
								.get(event)
								.unwrap()
								.get(trigger)
								.unwrap()
								.iter()
								.find(|x| {
									get_local_reference(match x {
										RefMaybeConstantValue::Ref(ref x) => x,
										RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue {
											ref entity_ref,
											..
										}) => entity_ref
									})
									.map(|x| x == *entity_to_delete)
									.unwrap_or(false)
								})
								.unwrap()
								.to_owned()
						)
					));
				}

				ReverseReferenceData::InputCopy { trigger, propagate } => {
					patch.patch.push(PatchOperation::SubEntityOperation(
						reverse_ref.from.to_owned(),
						SubEntityOperation::RemoveInputCopyConnection(
							trigger.to_owned(),
							propagate.to_owned(),
							entity
								.entities
								.get(&reverse_ref.from)
								.unwrap()
								.input_copying
								.as_ref()
								.unwrap()
								.get(trigger)
								.unwrap()
								.get(propagate)
								.unwrap()
								.iter()
								.find(|x| {
									get_local_reference(match x {
										RefMaybeConstantValue::Ref(ref x) => x,
										RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue {
											ref entity_ref,
											..
										}) => entity_ref
									})
									.map(|x| x == *entity_to_delete)
									.unwrap_or(false)
								})
								.unwrap()
								.to_owned()
						)
					));
				}

				ReverseReferenceData::OutputCopy { event, propagate } => {
					patch.patch.push(PatchOperation::SubEntityOperation(
						reverse_ref.from.to_owned(),
						SubEntityOperation::RemoveOutputCopyConnection(
							event.to_owned(),
							propagate.to_owned(),
							entity
								.entities
								.get(&reverse_ref.from)
								.unwrap()
								.output_copying
								.as_ref()
								.unwrap()
								.get(event)
								.unwrap()
								.get(propagate)
								.unwrap()
								.iter()
								.find(|x| {
									get_local_reference(match x {
										RefMaybeConstantValue::Ref(ref x) => x,
										RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue {
											ref entity_ref,
											..
										}) => entity_ref
									})
									.map(|x| x == *entity_to_delete)
									.unwrap_or(false)
								})
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
						.as_mut()
						.unwrap()
						.get_mut(aliased_name)
						.unwrap()
						.retain(|x| {
							get_local_reference(&x.original_entity)
								.map(|x| x != *entity_to_delete)
								.unwrap_or(false)
						});
				}

				ReverseReferenceData::ExposedEntity { exposed_name } => {
					entity
						.entities
						.get_mut(&reverse_ref.from)
						.unwrap()
						.exposed_entities
						.as_mut()
						.unwrap()
						.get_mut(exposed_name)
						.unwrap()
						.refers_to
						.retain(|x| get_local_reference(x).map(|x| x != *entity_to_delete).unwrap_or(false));

					if entity
						.entities
						.get_mut(&reverse_ref.from)
						.unwrap()
						.exposed_entities
						.as_mut()
						.unwrap()
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
							.as_mut()
							.unwrap()
							.shift_remove(exposed_name)
							.unwrap();
					}
				}

				ReverseReferenceData::ExposedInterface { interface } => {
					entity
						.entities
						.get_mut(&reverse_ref.from)
						.unwrap()
						.exposed_interfaces
						.as_mut()
						.unwrap()
						.shift_remove(interface)
						.unwrap();
				}

				ReverseReferenceData::Subset { subset } => {
					entity
						.entities
						.get_mut(&reverse_ref.from)
						.unwrap()
						.subsets
						.as_mut()
						.unwrap()
						.get_mut(subset)
						.unwrap()
						.retain(|x| x != entity_to_delete);
				}
			}

			refs_deleted += 1;
		}
	}

	apply_patch(entity, patch, false).map_err(|x| anyhow!(x))?;

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
		Request::Global(GlobalRequest::SetTabUnsaved {
			id: editor_id,
			unsaved: true
		})
	)?;

	send_request(
		app,
		Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
			EntityMonacoRequest::DeselectIfSelected {
				editor_id: editor_id.to_owned(),
				entity_ids: entities_to_delete.iter().cloned().collect()
			}
		)))
	)?;

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
}

#[try_fn]
#[context("Couldn't handle copy event")]
pub async fn copy(app: &AppHandle, editor_id: Uuid, id: String) -> Result<()> {
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Copying entity {} and its children", id))?;

	let editor_state = app_state.editor_states.get(&editor_id).context("No such editor")?;

	let entity = match editor_state.data {
		EditorData::QNEntity { ref entity, .. } => entity,
		EditorData::QNPatch { ref current, .. } => current,

		_ => {
			Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
			panic!();
		}
	};

	let reverse_refs = calculate_reverse_references(entity)?;

	let entities_to_copy = get_recursive_children(entity, &id, &reverse_refs)?
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

	let mut editor_state = app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

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
			sub_entity.parent = change_reference_to_local(
				&sub_entity.parent,
				changed_entity_ids
					.get(&get_local_reference(&sub_entity.parent).unwrap())
					.unwrap()
					.to_owned()
			);
		}

		for property_data in sub_entity
			.properties
			.as_mut()
			.unwrap_or(&mut Default::default())
			.values_mut()
		{
			if property_data.property_type == "SEntityTemplateReference" {
				let entity_ref = alter_ref_according_to_changelist(
					&from_value::<Ref>(property_data.value.to_owned()).context("Invalid reference")?,
					&changed_entity_ids
				);

				property_data.value = to_value(&entity_ref)?;

				// If the ref is external, add the external scene
				if let Ref::Full(FullRef {
					external_scene: Some(ref scene),
					..
				}) = entity_ref
				{
					if !entity.external_scenes.contains(scene) {
						entity.external_scenes.push(scene.to_owned());
						added_external_scenes += 1;
					}
				}

				// If the ref is local but to a sub-entity that doesn't exist in the entity we're pasting into (and isn't an internal reference within the paste), set the property to null
				if get_local_reference(&entity_ref)
					.map(|x| !entity.entities.contains_key(&x) && !all_paste_contents.contains(&x))
					.unwrap_or(false)
				{
					property_data.value = Value::Null;
				}
			} else if property_data.property_type == "TArray<SEntityTemplateReference>" {
				property_data.value = to_value(
					from_value::<Vec<Ref>>(property_data.value.to_owned())
						.context("Invalid reference array")?
						.into_iter()
						.map(|entity_ref| {
							if let Ref::Full(FullRef {
								external_scene: Some(ref scene),
								..
							}) = entity_ref
							{
								if !entity.external_scenes.contains(scene) {
									entity.external_scenes.push(scene.to_owned());
									added_external_scenes += 1;
								}
							}

							alter_ref_according_to_changelist(&entity_ref, &changed_entity_ids)
						})
						.filter(|entity_ref| {
							!get_local_reference(entity_ref)
								.map(|x| !entity.entities.contains_key(&x) && !all_paste_contents.contains(&x))
								.unwrap_or(false)
						})
						.collect_vec()
				)?;
			}
		}

		for properties in sub_entity
			.platform_specific_properties
			.as_mut()
			.unwrap_or(&mut Default::default())
			.values_mut()
		{
			for property_data in properties.values_mut() {
				if property_data.property_type == "SEntityTemplateReference" {
					let entity_ref = alter_ref_according_to_changelist(
						&from_value::<Ref>(property_data.value.to_owned()).context("Invalid reference")?,
						&changed_entity_ids
					);

					property_data.value = to_value(&entity_ref)?;

					// If the ref is external, add the external scene
					if let Ref::Full(FullRef {
						external_scene: Some(ref scene),
						..
					}) = entity_ref
					{
						if !entity.external_scenes.contains(scene) {
							entity.external_scenes.push(scene.to_owned());
							added_external_scenes += 1;
						}
					}

					// If the ref is local but to a sub-entity that doesn't exist in the entity we're pasting into (and isn't an internal reference within the paste), set the property to null
					if get_local_reference(&entity_ref)
						.map(|x| !entity.entities.contains_key(&x) && !all_paste_contents.contains(&x))
						.unwrap_or(false)
					{
						property_data.value = Value::Null;
					}
				} else if property_data.property_type == "TArray<SEntityTemplateReference>" {
					property_data.value = to_value(
						from_value::<Vec<Ref>>(property_data.value.to_owned())
							.context("Invalid reference array")?
							.into_iter()
							.map(|entity_ref| {
								if let Ref::Full(FullRef {
									external_scene: Some(ref scene),
									..
								}) = entity_ref
								{
									if !entity.external_scenes.contains(scene) {
										entity.external_scenes.push(scene.to_owned());
										added_external_scenes += 1;
									}
								}

								alter_ref_according_to_changelist(&entity_ref, &changed_entity_ids)
							})
							.filter(|entity_ref| {
								!get_local_reference(entity_ref)
									.map(|x| !entity.entities.contains_key(&x) && !all_paste_contents.contains(&x))
									.unwrap_or(false)
							})
							.collect_vec()
					)?;
				}
			}
		}

		for values in sub_entity
			.events
			.as_mut()
			.unwrap_or(&mut Default::default())
			.values_mut()
		{
			for refs in values.values_mut() {
				for reference in refs.iter_mut() {
					let underlying_ref = match reference {
						RefMaybeConstantValue::Ref(x) => x,
						RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue { entity_ref, .. }) => {
							entity_ref
						}
					};

					if let Ref::Full(FullRef {
						external_scene: Some(ref scene),
						..
					}) = underlying_ref
					{
						if !entity.external_scenes.contains(scene) {
							entity.external_scenes.push(scene.to_owned());
							added_external_scenes += 1;
						}
					}

					*reference = match reference {
						RefMaybeConstantValue::Ref(x) => {
							RefMaybeConstantValue::Ref(alter_ref_according_to_changelist(x, &changed_entity_ids))
						}
						RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue { entity_ref, value }) => {
							RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue {
								entity_ref: alter_ref_according_to_changelist(entity_ref, &changed_entity_ids),
								value: value.to_owned()
							})
						}
					};
				}

				refs.retain(|reference| {
					let underlying_ref = match reference {
						RefMaybeConstantValue::Ref(x) => x,
						RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue { entity_ref, .. }) => {
							entity_ref
						}
					};

					!get_local_reference(underlying_ref)
						.map(|x| !entity.entities.contains_key(&x) && !all_paste_contents.contains(&x))
						.unwrap_or(false)
				});
			}
		}

		for values in sub_entity
			.input_copying
			.as_mut()
			.unwrap_or(&mut Default::default())
			.values_mut()
		{
			for refs in values.values_mut() {
				for reference in refs.iter_mut() {
					let underlying_ref = match reference {
						RefMaybeConstantValue::Ref(x) => x,
						RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue { entity_ref, .. }) => {
							entity_ref
						}
					};

					if let Ref::Full(FullRef {
						external_scene: Some(ref scene),
						..
					}) = underlying_ref
					{
						if !entity.external_scenes.contains(scene) {
							entity.external_scenes.push(scene.to_owned());
							added_external_scenes += 1;
						}
					}

					*reference = match reference {
						RefMaybeConstantValue::Ref(x) => {
							RefMaybeConstantValue::Ref(alter_ref_according_to_changelist(x, &changed_entity_ids))
						}
						RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue { entity_ref, value }) => {
							RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue {
								entity_ref: alter_ref_according_to_changelist(entity_ref, &changed_entity_ids),
								value: value.to_owned()
							})
						}
					};
				}

				refs.retain(|reference| {
					let underlying_ref = match reference {
						RefMaybeConstantValue::Ref(x) => x,
						RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue { entity_ref, .. }) => {
							entity_ref
						}
					};

					!get_local_reference(underlying_ref)
						.map(|x| !entity.entities.contains_key(&x) && !all_paste_contents.contains(&x))
						.unwrap_or(false)
				});
			}
		}

		for values in sub_entity
			.output_copying
			.as_mut()
			.unwrap_or(&mut Default::default())
			.values_mut()
		{
			for refs in values.values_mut() {
				for reference in refs.iter_mut() {
					let underlying_ref = match reference {
						RefMaybeConstantValue::Ref(x) => x,
						RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue { entity_ref, .. }) => {
							entity_ref
						}
					};

					if let Ref::Full(FullRef {
						external_scene: Some(ref scene),
						..
					}) = underlying_ref
					{
						if !entity.external_scenes.contains(scene) {
							entity.external_scenes.push(scene.to_owned());
							added_external_scenes += 1;
						}
					}

					*reference = match reference {
						RefMaybeConstantValue::Ref(x) => {
							RefMaybeConstantValue::Ref(alter_ref_according_to_changelist(x, &changed_entity_ids))
						}
						RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue { entity_ref, value }) => {
							RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue {
								entity_ref: alter_ref_according_to_changelist(entity_ref, &changed_entity_ids),
								value: value.to_owned()
							})
						}
					};
				}

				refs.retain(|reference| {
					let underlying_ref = match reference {
						RefMaybeConstantValue::Ref(x) => x,
						RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue { entity_ref, .. }) => {
							entity_ref
						}
					};

					!get_local_reference(underlying_ref)
						.map(|x| !entity.entities.contains_key(&x) && !all_paste_contents.contains(&x))
						.unwrap_or(false)
				});
			}
		}

		for aliases in sub_entity
			.property_aliases
			.as_mut()
			.unwrap_or(&mut Default::default())
			.values_mut()
		{
			for alias_data in aliases.iter_mut() {
				alias_data.original_entity =
					alter_ref_according_to_changelist(&alias_data.original_entity, &changed_entity_ids);

				if let Ref::Full(FullRef {
					external_scene: Some(ref scene),
					..
				}) = alias_data.original_entity
				{
					if !entity.external_scenes.contains(scene) {
						entity.external_scenes.push(scene.to_owned());
						added_external_scenes += 1;
					}
				}
			}

			aliases.retain(|alias_data| {
				!get_local_reference(&alias_data.original_entity)
					.map(|x| !entity.entities.contains_key(&x) && !all_paste_contents.contains(&x))
					.unwrap_or(false)
			});
		}

		for exposed_entity in sub_entity
			.exposed_entities
			.as_mut()
			.unwrap_or(&mut Default::default())
			.values_mut()
		{
			for reference in exposed_entity.refers_to.iter_mut() {
				*reference = alter_ref_according_to_changelist(reference, &changed_entity_ids);

				if let Ref::Full(FullRef {
					external_scene: Some(ref scene),
					..
				}) = reference
				{
					if !entity.external_scenes.contains(scene) {
						entity.external_scenes.push(scene.to_owned());
						added_external_scenes += 1;
					}
				}
			}

			exposed_entity.refers_to.retain(|x| {
				// Only retain those not meeting the criteria for deletion (local ref, not in entity we're pasting into or the paste itself)
				!get_local_reference(x)
					.map(|x| !entity.entities.contains_key(&x) && !all_paste_contents.contains(&x))
					.unwrap_or(false)
			});
		}

		for referenced_entity in sub_entity
			.exposed_interfaces
			.as_mut()
			.unwrap_or(&mut Default::default())
			.values_mut()
		{
			*referenced_entity = changed_entity_ids
				.get(referenced_entity)
				.unwrap_or(referenced_entity)
				.to_owned();
		}

		sub_entity
			.exposed_interfaces
			.as_mut()
			.unwrap_or(&mut Default::default())
			.retain(|_, x| entity.entities.contains_key(x) || all_paste_contents.contains(x));

		for member_of in sub_entity
			.subsets
			.as_mut()
			.unwrap_or(&mut Default::default())
			.values_mut()
		{
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
		.parent = change_reference_to_local(
		&paste_data
			.data
			.get_mut(changed_entity_ids.get(&paste_data.root_entity).unwrap())
			.unwrap()
			.parent,
		parent_id.to_owned()
	);

	entity.entities.extend(paste_data.data.to_owned());

	let mut new_entities = vec![];
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

	for (entity_id, entity_data) in paste_data.data {
		let x = reverse_parent_refs.contains(&entity_id);
		new_entities.push((entity_id, entity_data.parent, entity_data.name, entity_data.factory, x));
	}

	// Make sure the entity being pasted under is updated to be considered a folder (if it's a ZEntity)
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
			.to_owned(),
		true
	));

	send_request(
		app,
		Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
			EntityTreeRequest::NewItems {
				editor_id,
				new_entities
			}
		)))
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
		Request::Global(GlobalRequest::SetTabUnsaved {
			id: editor_id,
			unsaved: true
		})
	)?;

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
}

#[try_fn]
#[context("Couldn't handle search event")]
pub async fn search(app: &AppHandle, editor_id: Uuid, query: String) -> Result<()> {
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Searching for {}", query))?;

	let editor_state = app_state.editor_states.get(&editor_id).context("No such editor")?;

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
		Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
			EntityTreeRequest::SearchResults {
				editor_id,
				results: entity
					.entities
					.par_iter()
					.filter(|(id, ent)| {
						let mut s = format!("{}{}", id, to_string(ent).unwrap());
						s.make_ascii_lowercase();
						query.split(' ').all(|q| s.contains(q))
					})
					.map(|(id, _)| id.to_owned())
					.collect()
			}
		)))
	)?;

	finish_task(app, task)?;
}

#[try_fn]
#[context("Couldn't handle help menu event")]
pub async fn help_menu(app: &AppHandle, editor_id: Uuid, entity_id: String) -> Result<()> {
	let app_settings = app.state::<ArcSwap<AppSettings>>();
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Showing help menu for {}", entity_id))?;

	let editor_state = app_state.editor_states.get(&editor_id).context("No such editor")?;

	let entity = match editor_state.data {
		EditorData::QNEntity { ref entity, .. } => entity,
		EditorData::QNPatch { ref current, .. } => current,

		_ => {
			Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
			panic!();
		}
	};

	let sub_entity = entity.entities.get(&entity_id).context("No such entity")?;

	if let Some(intellisense) = app_state.intellisense.load().as_ref()
		&& let Some(game_files) = app_state.game_files.load().as_ref()
		&& let Some(hash_list) = app_state.hash_list.load().as_ref()
		&& let Some(install) = app_settings.load().game_install.as_ref()
	{
		let game_version = get_loaded_game_version(app, install)?;

		let (properties, pins) = if hash_list
			.entries
			.get(&ResourceID::from_any(&sub_entity.factory)?)
			.map(|entry| entry.resource_type == "TEMP")
			.unwrap_or(false)
		{
			let underlying_entity = extract_entity(
				game_files,
				&app_state.cached_entities,
				game_version,
				hash_list,
				ResourceID::from_any(&sub_entity.factory)?
			)?;

			(
				intellisense.get_properties(
					game_files,
					&app_state.cached_entities,
					hash_list,
					game_version,
					&underlying_entity,
					&underlying_entity.root_entity,
					false
				)?,
				intellisense.get_pins(
					game_files,
					&app_state.cached_entities,
					hash_list,
					game_version,
					&underlying_entity,
					&underlying_entity.root_entity,
					false
				)?
			)
		} else {
			(
				intellisense.get_properties(
					game_files,
					&app_state.cached_entities,
					hash_list,
					game_version,
					entity,
					&entity_id,
					true
				)?,
				intellisense.get_pins(
					game_files,
					&app_state.cached_entities,
					hash_list,
					game_version,
					entity,
					&entity_id,
					true
				)?
			)
		};

		let properties_data_str = {
			let mut buf = Vec::new();
			let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
			let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

			properties
				.into_iter()
				.map(|(name, ty, default_val, post_init)| {
					(
						name,
						if post_init {
							json!({
								"type": ty,
								"value": default_val,
								"postInit": true
							})
						} else {
							json!({
								"type": ty,
								"value": default_val
							})
						}
					)
				})
				.collect::<HashMap<_, _>>()
				.serialize(&mut ser)?;

			String::from_utf8(buf)?
		};

		send_request(
			app,
			Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
				EntityTreeRequest::ShowHelpMenu {
					editor_id,
					factory: sub_entity.factory.to_owned(),
					input_pins: pins.0,
					output_pins: pins.1,
					default_properties_json: properties_data_str
				}
			)))
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
pub async fn add_game_browser_item(
	app: &AppHandle,
	editor_id: Uuid,
	parent_id: String,
	file: ResourceID
) -> Result<()> {
	let app_settings = app.state::<ArcSwap<AppSettings>>();
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Adding {}", file))?;

	let mut editor_state = app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

	let entity = match editor_state.data {
		EditorData::QNEntity { ref mut entity, .. } => entity,
		EditorData::QNPatch { ref mut current, .. } => current,

		_ => {
			Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
			panic!();
		}
	};

	if let Some(game_files) = app_state.game_files.load().as_ref()
		&& let Some(hash_list) = app_state.hash_list.load().as_ref()
		&& let Some(install) = app_settings.load().game_install.as_ref()
	{
		let game_version = get_loaded_game_version(app, install)?;

		if is_valid_entity_factory(
			hash_list
				.entries
				.get(&file)
				.context("File not in hash list")?
				.resource_type
		) {
			let entity_id = random_entity_id();

			let sub_entity = match hash_list
				.entries
				.get(&file)
				.context("File not in hash list")?
				.resource_type
				.as_ref()
			{
				"TEMP" => {
					let (temp_meta, temp_data) = extract_latest_resource(game_files, file)?;

					let factory = match game_version {
						GameVersion::H1 => h2016_convert_binary_to_factory(&temp_data)
							.context("Couldn't convert binary data to ResourceLib factory")?
							.into_modern(),

						GameVersion::H2 => h2_convert_binary_to_factory(&temp_data)
							.context("Couldn't convert binary data to ResourceLib factory")?,

						GameVersion::H3 => h3_convert_binary_to_factory(&temp_data)
							.context("Couldn't convert binary data to ResourceLib factory")?
					};

					let blueprint_hash = &temp_meta
						.core_info
						.references
						.get(factory.blueprint_index_in_resource_header as usize)
						.context("Blueprint referenced in factory does not exist in dependencies")?
						.resource;

					let factory_path = hash_list.to_path(&file);
					let blueprint_path = hash_list.to_path(blueprint_hash);

					SubEntity {
						parent: Ref::Short((parent_id != "#").then_some(parent_id)),
						name: factory_path
							.replace("].pc_entitytype", "")
							.replace("].pc_entitytemplate", "")
							.replace(".entitytemplate", "")
							.split('/')
							.last()
							.map(|x| x.to_owned())
							.unwrap_or(factory_path.to_owned()),
						factory: factory_path,
						factory_flag: None,
						blueprint: blueprint_path,
						editor_only: None,
						properties: None,
						platform_specific_properties: None,
						events: None,
						input_copying: None,
						output_copying: None,
						property_aliases: None,
						exposed_entities: None,
						exposed_interfaces: None,
						subsets: None
					}
				}

				"CPPT" => {
					let (cppt_meta, cppt_data) = extract_latest_resource(game_files, file)?;

					let factory =
						match game_version {
							GameVersion::H1 => h2016_convert_cppt(&cppt_data)
								.context("Couldn't convert binary data to ResourceLib format")?,

							GameVersion::H2 => h2_convert_cppt(&cppt_data)
								.context("Couldn't convert binary data to ResourceLib format")?,

							GameVersion::H3 => h3_convert_cppt(&cppt_data)
								.context("Couldn't convert binary data to ResourceLib format")?
						};

					let blueprint_hash = &cppt_meta
						.core_info
						.references
						.get(factory.blueprint_index_in_resource_header as usize)
						.context("Blueprint referenced in factory does not exist in dependencies")?
						.resource;

					let factory_path = hash_list.to_path(&file);
					let blueprint_path = hash_list.to_path(blueprint_hash);

					SubEntity {
						parent: Ref::Short((parent_id != "#").then_some(parent_id)),
						name: factory_path
							.replace("].pc_entitytype", "")
							.replace(".class", "")
							.split('/')
							.last()
							.map(|x| x.to_owned())
							.unwrap_or(factory_path.to_owned()),
						factory: factory_path,
						factory_flag: None,
						blueprint: blueprint_path,
						editor_only: None,
						properties: None,
						platform_specific_properties: None,
						events: None,
						input_copying: None,
						output_copying: None,
						property_aliases: None,
						exposed_entities: None,
						exposed_interfaces: None,
						subsets: None
					}
				}

				"ASET" => {
					let blueprint_hash = extract_latest_metadata(game_files, file)?
						.core_info
						.references
						.into_iter()
						.last()
						.context("ASET had no dependencies")?
						.resource;

					let factory_path = hash_list.to_path(&file);
					let blueprint_path = hash_list.to_path(&blueprint_hash);

					SubEntity {
						parent: Ref::Short((parent_id != "#").then_some(parent_id)),
						name: factory_path.to_owned(),
						factory: factory_path,
						factory_flag: None,
						blueprint: blueprint_path,
						editor_only: None,
						properties: None,
						platform_specific_properties: None,
						events: None,
						input_copying: None,
						output_copying: None,
						property_aliases: None,
						exposed_entities: None,
						exposed_interfaces: None,
						subsets: None
					}
				}

				"UICT" => {
					let blueprint_hash = extract_latest_metadata(game_files, file)?
						.core_info
						.references
						.into_iter()
						.last()
						.context("UICT had no dependencies")?
						.resource;

					let factory_path = hash_list.to_path(&file);
					let blueprint_path = hash_list.to_path(&blueprint_hash);

					SubEntity {
						parent: Ref::Short((parent_id != "#").then_some(parent_id)),
						name: factory_path
							.replace("].pc_entitytype", "")
							.replace("].pc_entitytemplate", "")
							.replace(".entitytemplate", "")
							.split('/')
							.last()
							.map(|x| x.to_owned())
							.unwrap_or(factory_path.to_owned()),
						factory: factory_path,
						factory_flag: None,
						blueprint: blueprint_path,
						editor_only: None,
						properties: None,
						platform_specific_properties: None,
						events: None,
						input_copying: None,
						output_copying: None,
						property_aliases: None,
						exposed_entities: None,
						exposed_interfaces: None,
						subsets: None
					}
				}

				"MATT" => {
					let blueprint_hash = extract_latest_metadata(game_files, file)?
						.core_info
						.references
						.into_iter()
						.try_find(|dep| {
							anyhow::Ok(
								extract_latest_metadata(game_files, dep.resource)?
									.core_info
									.resource_type == "MATB"
							)
						})?
						.context("No blueprint dependency found")?
						.resource;

					let factory_path = hash_list.to_path(&file);
					let blueprint_path = hash_list.to_path(&blueprint_hash);

					SubEntity {
						parent: Ref::Short((parent_id != "#").then_some(parent_id)),
						name: factory_path
							.replace("].pc_entitytype", "")
							.replace("].pc_entitytemplate", "")
							.replace(".entitytemplate", "")
							.split('/')
							.last()
							.map(|x| x.to_owned())
							.unwrap_or(factory_path.to_owned()),
						factory: factory_path,
						factory_flag: None,
						blueprint: blueprint_path,
						editor_only: None,
						properties: None,
						platform_specific_properties: None,
						events: None,
						input_copying: None,
						output_copying: None,
						property_aliases: None,
						exposed_entities: None,
						exposed_interfaces: None,
						subsets: None
					}
				}

				"WSWT" => {
					let blueprint_hash = extract_latest_metadata(game_files, file)?
						.core_info
						.references
						.into_iter()
						.try_find(|dep| {
							anyhow::Ok({
								let x = extract_latest_metadata(game_files, dep.resource)?
									.core_info
									.resource_type;

								x == "WSWB" || x == "DSWB"
							})
						})?
						.context("No blueprint dependency found")?
						.resource;

					let factory_path = hash_list.to_path(&file);
					let blueprint_path = hash_list.to_path(&blueprint_hash);

					SubEntity {
						parent: Ref::Short((parent_id != "#").then_some(parent_id)),
						name: factory_path
							.replace("].pc_entitytype", "")
							.replace("].pc_entitytemplate", "")
							.replace(".entitytemplate", "")
							.split('/')
							.last()
							.map(|x| x.to_owned())
							.unwrap_or(factory_path.to_owned()),
						factory: factory_path,
						factory_flag: None,
						blueprint: blueprint_path,
						editor_only: None,
						properties: None,
						platform_specific_properties: None,
						events: None,
						input_copying: None,
						output_copying: None,
						property_aliases: None,
						exposed_entities: None,
						exposed_interfaces: None,
						subsets: None
					}
				}

				"ECPT" => {
					let blueprint_hash = extract_latest_metadata(game_files, file)?
						.core_info
						.references
						.into_iter()
						.try_find(|dep| {
							anyhow::Ok(
								extract_latest_metadata(game_files, dep.resource)?
									.core_info
									.resource_type == "ECPB"
							)
						})?
						.context("No blueprint dependency found")?
						.resource;

					let factory_path = hash_list.to_path(&file);
					let blueprint_path = hash_list.to_path(&blueprint_hash);

					SubEntity {
						parent: Ref::Short((parent_id != "#").then_some(parent_id)),
						name: factory_path
							.replace("].pc_entitytype", "")
							.replace("].pc_entitytemplate", "")
							.replace(".entitytemplate", "")
							.split('/')
							.last()
							.map(|x| x.to_owned())
							.unwrap_or(factory_path.to_owned()),
						factory: factory_path,
						factory_flag: None,
						blueprint: blueprint_path,
						editor_only: None,
						properties: None,
						platform_specific_properties: None,
						events: None,
						input_copying: None,
						output_copying: None,
						property_aliases: None,
						exposed_entities: None,
						exposed_interfaces: None,
						subsets: None
					}
				}

				"AIBX" => {
					let blueprint_hash = extract_latest_metadata(game_files, file)?
						.core_info
						.references
						.into_iter()
						.try_find(|dep| {
							anyhow::Ok(
								extract_latest_metadata(game_files, dep.resource)?
									.core_info
									.resource_type == "AIBB"
							)
						})?
						.context("No blueprint dependency found")?
						.resource;

					let factory_path = hash_list.to_path(&file);
					let blueprint_path = hash_list.to_path(&blueprint_hash);

					SubEntity {
						parent: Ref::Short((parent_id != "#").then_some(parent_id)),
						name: factory_path
							.replace("].pc_entitytype", "")
							.replace("].pc_entitytemplate", "")
							.replace(".entitytemplate", "")
							.split('/')
							.last()
							.map(|x| x.to_owned())
							.unwrap_or(factory_path.to_owned()),
						factory: factory_path,
						factory_flag: None,
						blueprint: blueprint_path,
						editor_only: None,
						properties: None,
						platform_specific_properties: None,
						events: None,
						input_copying: None,
						output_copying: None,
						property_aliases: None,
						exposed_entities: None,
						exposed_interfaces: None,
						subsets: None
					}
				}

				"WSGT" => {
					let blueprint_hash = extract_latest_metadata(game_files, file)?
						.core_info
						.references
						.into_iter()
						.try_find(|dep| {
							anyhow::Ok(
								extract_latest_metadata(game_files, dep.resource)?
									.core_info
									.resource_type == "WSGB"
							)
						})?
						.context("No blueprint dependency found")?
						.resource;

					let factory_path = hash_list.to_path(&file);
					let blueprint_path = hash_list.to_path(&blueprint_hash);

					SubEntity {
						parent: Ref::Short((parent_id != "#").then_some(parent_id)),
						name: factory_path
							.replace("].pc_entitytype", "")
							.replace("].pc_entitytemplate", "")
							.replace(".entitytemplate", "")
							.split('/')
							.last()
							.map(|x| x.to_owned())
							.unwrap_or(factory_path.to_owned()),
						factory: factory_path,
						factory_flag: None,
						blueprint: blueprint_path,
						editor_only: None,
						properties: None,
						platform_specific_properties: None,
						events: None,
						input_copying: None,
						output_copying: None,
						property_aliases: None,
						exposed_entities: None,
						exposed_interfaces: None,
						subsets: None
					}
				}

				_ => unreachable!()
			};

			send_request(
				app,
				Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
					EntityTreeRequest::NewItems {
						editor_id: editor_id.to_owned(),
						new_entities: vec![(
							entity_id.to_owned(),
							sub_entity.parent.to_owned(),
							sub_entity.name.to_owned(),
							sub_entity.factory.to_owned(),
							false
						)]
					}
				)))
			)?;

			entity.entities.insert(entity_id, sub_entity);

			send_request(
				app,
				Request::Global(GlobalRequest::SetTabUnsaved {
					id: editor_id,
					unsaved: true
				})
			)?;
		} else if hash_list
			.entries
			.get(&file)
			.context("File not in hash list")?
			.resource_type
			== "WWEV"
		{
			let (_, wwev_data) = extract_latest_resource(game_files, file)?;

			let wwev = WwiseEvent::parse(&wwev_data)?;

			let entity_id = random_entity_id();

			let file_path = hash_list.to_path(&file);

			let sub_entity = SubEntity {
				parent: Ref::Short((parent_id != "#").then_some(parent_id)),
				name: wwev.name,
				factory: "[modules:/zaudioevententity.class].pc_entitytype".into(),
				factory_flag: None,
				blueprint: "[modules:/zaudioevententity.class].pc_entityblueprint".into(),
				editor_only: None,
				properties: Some({
					let mut properties = IndexMap::new();
					properties.insert(
						"m_pMainEvent".into(),
						Property {
							property_type: "ZRuntimeResourceID".into(),
							value: json!({
								"resource": file_path,
								"flag": "5F"
							}),
							post_init: None
						}
					);
					properties
				}),
				platform_specific_properties: None,
				events: None,
				input_copying: None,
				output_copying: None,
				property_aliases: None,
				exposed_entities: None,
				exposed_interfaces: None,
				subsets: None
			};

			send_request(
				app,
				Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
					EntityTreeRequest::NewItems {
						editor_id: editor_id.to_owned(),
						new_entities: vec![(
							entity_id.to_owned(),
							sub_entity.parent.to_owned(),
							sub_entity.name.to_owned(),
							sub_entity.factory.to_owned(),
							false
						)]
					}
				)))
			)?;

			entity.entities.insert(entity_id, sub_entity);

			send_request(
				app,
				Request::Global(GlobalRequest::SetTabUnsaved {
					id: editor_id,
					unsaved: true
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
			Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
				EntityTreeRequest::SetDiffInfo {
					editor_id,
					diff_info: get_diff_info(base, current)
				}
			)))
		)?;
	}
}

#[try_fn]
#[context("Couldn't handle select entity in editor event")]
pub async fn select_entity_in_editor(app: &AppHandle, editor_id: Uuid, entity_id: String) -> Result<()> {
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Selecting {} in editor", entity_id))?;

	let mut editor_state = app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

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
		.select_entity(&entity_id, &entity.blueprint_hash)
		.await?;

	finish_task(app, task)?;
}

#[try_fn]
#[context("Couldn't handle move entity to player event")]
pub async fn move_entity_to_player(app: &AppHandle, editor_id: Uuid, entity_id: String) -> Result<()> {
	let app_settings = app.state::<ArcSwap<AppSettings>>();
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Moving {} to player position", entity_id))?;

	let mut editor_state = app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

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
		.as_mut()
		.unwrap()
		.shift_remove(&String::from("m_eidParent"))
		.is_some()
	{
		app_state
			.editor_connection
			.set_property(
				&entity_id,
				&entity.blueprint_hash,
				"m_eidParent",
				PropertyValue {
					property_type: "SEntityTemplateReference".into(),
					data: Value::Null
				}
			)
			.await?;
	}

	let property = entity
		.entities
		.get_mut(&entity_id)
		.context("No such entity")?
		.properties
		.get_or_insert_default()
		.entry("m_mTransform".into())
		.or_insert(Property {
			property_type: "SMatrix43".into(),
			value: json!({
				"rotation": {
					"x": 0,
					"y": 0,
					"z": 0
				},
				"position": {
					"x": 0,
					"y": 0,
					"z": 0
				}
			}),
			post_init: None
		});

	property.value.as_object_mut().unwrap().insert(
		"position".into(),
		json!({
			"x": player_transform.position.x,
			"y": player_transform.position.y,
			"z": player_transform.position.z
		})
	);

	app_state
		.editor_connection
		.set_property(
			&entity_id,
			&entity.blueprint_hash,
			"m_mTransform",
			PropertyValue {
				property_type: "SMatrix43".into(),
				data: property.value.to_owned()
			}
		)
		.await?;

	if let Some(intellisense) = app_state.intellisense.load().as_ref()
		&& let Some(game_files) = app_state.game_files.load().as_ref()
		&& let Some(hash_list) = app_state.hash_list.load().as_ref()
		&& let Some(install) = app_settings.load().game_install.as_ref()
		&& intellisense
			.get_properties(
				game_files,
				&app_state.cached_entities,
				hash_list,
				get_loaded_game_version(app, install)?,
				entity,
				&entity_id,
				true
			)?
			.into_iter()
			.any(|(name, _, _, _)| name == "m_eRoomBehaviour")
	{
		entity
			.entities
			.get_mut(&entity_id)
			.context("No such entity")?
			.properties
			.as_mut()
			.unwrap()
			.insert(
				String::from("m_eRoomBehaviour"),
				Property {
					property_type: "ZSpatialEntity.ERoomBehaviour".into(),
					value: Value::String("ROOM_DYNAMIC".into()),
					post_init: None
				}
			);

		app_state
			.editor_connection
			.set_property(
				&entity_id,
				&entity.blueprint_hash,
				"m_eRoomBehaviour",
				PropertyValue {
					property_type: "ZSpatialEntity.ERoomBehaviour".into(),
					data: Value::String("ROOM_DYNAMIC".into())
				}
			)
			.await?;
	}

	send_request(
		app,
		Request::Global(GlobalRequest::SetTabUnsaved {
			id: editor_id,
			unsaved: true
		})
	)?;

	let mut buf = Vec::new();
	let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
	let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

	entity
		.entities
		.get(&entity_id)
		.context("No such entity")?
		.serialize(&mut ser)?;

	send_request(
		app,
		Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
			EntityMonacoRequest::ReplaceContentIfSameEntityID {
				editor_id: editor_id.to_owned(),
				entity_id,
				content: String::from_utf8(buf)?
			}
		)))
	)?;

	finish_task(app, task)?;

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
}

#[try_fn]
#[context("Couldn't handle rotate entity as player event")]
pub async fn rotate_entity_as_player(app: &AppHandle, editor_id: Uuid, entity_id: String) -> Result<()> {
	let app_settings = app.state::<ArcSwap<AppSettings>>();
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Adjusting {} to player rotation", entity_id))?;

	let mut editor_state = app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

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
		.as_mut()
		.unwrap()
		.shift_remove(&String::from("m_eidParent"))
		.is_some()
	{
		app_state
			.editor_connection
			.set_property(
				&entity_id,
				&entity.blueprint_hash,
				"m_eidParent",
				PropertyValue {
					property_type: "SEntityTemplateReference".into(),
					data: Value::Null
				}
			)
			.await?;
	}

	let property = entity
		.entities
		.get_mut(&entity_id)
		.context("No such entity")?
		.properties
		.get_or_insert_default()
		.entry("m_mTransform".into())
		.or_insert(Property {
			property_type: "SMatrix43".into(),
			value: json!({
				"rotation": {
					"x": 0,
					"y": 0,
					"z": 0
				},
				"position": {
					"x": 0,
					"y": 0,
					"z": 0
				}
			}),
			post_init: None
		});

	property.value.as_object_mut().unwrap().insert(
		"rotation".into(),
		json!({
			"x": player_transform.rotation.x,
			"y": player_transform.rotation.y,
			"z": player_transform.rotation.z
		})
	);

	app_state
		.editor_connection
		.set_property(
			&entity_id,
			&entity.blueprint_hash,
			"m_mTransform",
			PropertyValue {
				property_type: "SMatrix43".into(),
				data: property.value.to_owned()
			}
		)
		.await?;

	if let Some(intellisense) = app_state.intellisense.load().as_ref()
		&& let Some(game_files) = app_state.game_files.load().as_ref()
		&& let Some(hash_list) = app_state.hash_list.load().as_ref()
		&& let Some(install) = app_settings.load().game_install.as_ref()
		&& intellisense
			.get_properties(
				game_files,
				&app_state.cached_entities,
				hash_list,
				get_loaded_game_version(app, install)?,
				entity,
				&entity_id,
				true
			)?
			.into_iter()
			.any(|(name, _, _, _)| name == "m_eRoomBehaviour")
	{
		entity
			.entities
			.get_mut(&entity_id)
			.context("No such entity")?
			.properties
			.as_mut()
			.unwrap()
			.insert(
				String::from("m_eRoomBehaviour"),
				Property {
					property_type: "ZSpatialEntity.ERoomBehaviour".into(),
					value: Value::String("ROOM_DYNAMIC".into()),
					post_init: None
				}
			);

		app_state
			.editor_connection
			.set_property(
				&entity_id,
				&entity.blueprint_hash,
				"m_eRoomBehaviour",
				PropertyValue {
					property_type: "ZSpatialEntity.ERoomBehaviour".into(),
					data: Value::String("ROOM_DYNAMIC".into())
				}
			)
			.await?;
	}

	send_request(
		app,
		Request::Global(GlobalRequest::SetTabUnsaved {
			id: editor_id,
			unsaved: true
		})
	)?;

	let mut buf = Vec::new();
	let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
	let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

	entity
		.entities
		.get(&entity_id)
		.context("No such entity")?
		.serialize(&mut ser)?;

	send_request(
		app,
		Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
			EntityMonacoRequest::ReplaceContentIfSameEntityID {
				editor_id: editor_id.to_owned(),
				entity_id,
				content: String::from_utf8(buf)?
			}
		)))
	)?;

	finish_task(app, task)?;

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
}

#[try_fn]
#[context("Couldn't handle move entity to camera event")]
pub async fn move_entity_to_camera(app: &AppHandle, editor_id: Uuid, entity_id: String) -> Result<()> {
	let app_settings = app.state::<ArcSwap<AppSettings>>();
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Moving {} to camera position", entity_id))?;

	let mut editor_state = app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

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
		.as_mut()
		.unwrap()
		.shift_remove(&String::from("m_eidParent"))
		.is_some()
	{
		app_state
			.editor_connection
			.set_property(
				&entity_id,
				&entity.blueprint_hash,
				"m_eidParent",
				PropertyValue {
					property_type: "SEntityTemplateReference".into(),
					data: Value::Null
				}
			)
			.await?;
	}

	let property = entity
		.entities
		.get_mut(&entity_id)
		.context("No such entity")?
		.properties
		.get_or_insert_default()
		.entry("m_mTransform".into())
		.or_insert(Property {
			property_type: "SMatrix43".into(),
			value: json!({
				"rotation": {
					"x": 0,
					"y": 0,
					"z": 0
				},
				"position": {
					"x": 0,
					"y": 0,
					"z": 0
				}
			}),
			post_init: None
		});

	property.value.as_object_mut().unwrap().insert(
		"position".into(),
		json!({
			"x": camera_transform.position.x,
			"y": camera_transform.position.y,
			"z": camera_transform.position.z
		})
	);

	app_state
		.editor_connection
		.set_property(
			&entity_id,
			&entity.blueprint_hash,
			"m_mTransform",
			PropertyValue {
				property_type: "SMatrix43".into(),
				data: property.value.to_owned()
			}
		)
		.await?;

	if let Some(intellisense) = app_state.intellisense.load().as_ref()
		&& let Some(game_files) = app_state.game_files.load().as_ref()
		&& let Some(hash_list) = app_state.hash_list.load().as_ref()
		&& let Some(install) = app_settings.load().game_install.as_ref()
		&& intellisense
			.get_properties(
				game_files,
				&app_state.cached_entities,
				hash_list,
				get_loaded_game_version(app, install)?,
				entity,
				&entity_id,
				true
			)?
			.into_iter()
			.any(|(name, _, _, _)| name == "m_eRoomBehaviour")
	{
		entity
			.entities
			.get_mut(&entity_id)
			.context("No such entity")?
			.properties
			.as_mut()
			.unwrap()
			.insert(
				String::from("m_eRoomBehaviour"),
				Property {
					property_type: "ZSpatialEntity.ERoomBehaviour".into(),
					value: Value::String("ROOM_DYNAMIC".into()),
					post_init: None
				}
			);

		app_state
			.editor_connection
			.set_property(
				&entity_id,
				&entity.blueprint_hash,
				"m_eRoomBehaviour",
				PropertyValue {
					property_type: "ZSpatialEntity.ERoomBehaviour".into(),
					data: Value::String("ROOM_DYNAMIC".into())
				}
			)
			.await?;
	}

	send_request(
		app,
		Request::Global(GlobalRequest::SetTabUnsaved {
			id: editor_id,
			unsaved: true
		})
	)?;

	let mut buf = Vec::new();
	let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
	let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

	entity
		.entities
		.get(&entity_id)
		.context("No such entity")?
		.serialize(&mut ser)?;

	send_request(
		app,
		Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
			EntityMonacoRequest::ReplaceContentIfSameEntityID {
				editor_id: editor_id.to_owned(),
				entity_id,
				content: String::from_utf8(buf)?
			}
		)))
	)?;

	finish_task(app, task)?;

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
}

#[try_fn]
#[context("Couldn't handle rotate entity as camera event")]
pub async fn rotate_entity_as_camera(app: &AppHandle, editor_id: Uuid, entity_id: String) -> Result<()> {
	let app_settings = app.state::<ArcSwap<AppSettings>>();
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Adjusting {} to camera rotation", entity_id))?;

	let mut editor_state = app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

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
		.as_mut()
		.unwrap()
		.shift_remove(&String::from("m_eidParent"))
		.is_some()
	{
		app_state
			.editor_connection
			.set_property(
				&entity_id,
				&entity.blueprint_hash,
				"m_eidParent",
				PropertyValue {
					property_type: "SEntityTemplateReference".into(),
					data: Value::Null
				}
			)
			.await?;
	}

	let property = entity
		.entities
		.get_mut(&entity_id)
		.context("No such entity")?
		.properties
		.get_or_insert_default()
		.entry("m_mTransform".into())
		.or_insert(Property {
			property_type: "SMatrix43".into(),
			value: json!({
				"rotation": {
					"x": 0,
					"y": 0,
					"z": 0
				},
				"position": {
					"x": 0,
					"y": 0,
					"z": 0
				}
			}),
			post_init: None
		});

	property.value.as_object_mut().unwrap().insert(
		"rotation".into(),
		json!({
			"x": camera_transform.rotation.x,
			"y": camera_transform.rotation.y,
			"z": camera_transform.rotation.z
		})
	);

	app_state
		.editor_connection
		.set_property(
			&entity_id,
			&entity.blueprint_hash,
			"m_mTransform",
			PropertyValue {
				property_type: "SMatrix43".into(),
				data: property.value.to_owned()
			}
		)
		.await?;

	if let Some(intellisense) = app_state.intellisense.load().as_ref()
		&& let Some(game_files) = app_state.game_files.load().as_ref()
		&& let Some(hash_list) = app_state.hash_list.load().as_ref()
		&& let Some(install) = app_settings.load().game_install.as_ref()
		&& intellisense
			.get_properties(
				game_files,
				&app_state.cached_entities,
				hash_list,
				get_loaded_game_version(app, install)?,
				entity,
				&entity_id,
				true
			)?
			.into_iter()
			.any(|(name, _, _, _)| name == "m_eRoomBehaviour")
	{
		entity
			.entities
			.get_mut(&entity_id)
			.context("No such entity")?
			.properties
			.as_mut()
			.unwrap()
			.insert(
				String::from("m_eRoomBehaviour"),
				Property {
					property_type: "ZSpatialEntity.ERoomBehaviour".into(),
					value: Value::String("ROOM_DYNAMIC".into()),
					post_init: None
				}
			);

		app_state
			.editor_connection
			.set_property(
				&entity_id,
				&entity.blueprint_hash,
				"m_eRoomBehaviour",
				PropertyValue {
					property_type: "ZSpatialEntity.ERoomBehaviour".into(),
					data: Value::String("ROOM_DYNAMIC".into())
				}
			)
			.await?;
	}

	send_request(
		app,
		Request::Global(GlobalRequest::SetTabUnsaved {
			id: editor_id,
			unsaved: true
		})
	)?;

	let mut buf = Vec::new();
	let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
	let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

	entity
		.entities
		.get(&entity_id)
		.context("No such entity")?
		.serialize(&mut ser)?;

	send_request(
		app,
		Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
			EntityMonacoRequest::ReplaceContentIfSameEntityID {
				editor_id: editor_id.to_owned(),
				entity_id,
				content: String::from_utf8(buf)?
			}
		)))
	)?;

	finish_task(app, task)?;

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
}

#[try_fn]
#[context("Couldn't handle restore to original event")]
pub async fn restore_to_original(app: &AppHandle, editor_id: Uuid, entity_id: String) -> Result<()> {
	let app_settings = app.state::<ArcSwap<AppSettings>>();
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Reverting {} to original state", entity_id))?;

	let mut editor_state = app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

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

		let mut reverse_parent_refs: HashSet<String> = HashSet::new();

		for entity_data in current.entities.values() {
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

		let mut buf = Vec::new();
		let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
		let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

		sub_entity.serialize(&mut ser)?;

		send_request(
			app,
			Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
				EntityMonacoRequest::ReplaceContentIfSameEntityID {
					editor_id: editor_id.to_owned(),
					entity_id: entity_id.to_owned(),
					content: String::from_utf8(buf)?
				}
			)))
		)?;

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
					app_state
						.editor_connection
						.set_property(
							&entity_id,
							&current.blueprint_hash,
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
						.contains_key(&property)
						&& SAFE_TO_SYNC.iter().any(|&x| val.property_type == x)
					{
						if let Some((_, ty, def_val, _)) = intellisense
							.get_properties(
								game_files,
								&app_state.cached_entities,
								hash_list,
								get_loaded_game_version(app, install)?,
								current,
								&entity_id,
								false
							)?
							.into_iter()
							.find(|(name, _, _, _)| *name == property)
						{
							debug!(
								"Syncing removed property {} for entity {} with default value according to \
								 intellisense",
								property, entity_id
							);

							app_state
								.editor_connection
								.set_property(
									&entity_id,
									&current.blueprint_hash,
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
	} else {
		current.entities.insert(
			entity_id.to_owned(),
			base.entities
				.get(&entity_id)
				.context("Entity didn't exist in base")?
				.to_owned()
		);

		let sub_entity = current.entities.get(&entity_id).context("No such entity")?.to_owned();

		let mut reverse_parent_refs: HashSet<String> = HashSet::new();

		for entity_data in current.entities.values() {
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
	}

	send_request(
		app,
		Request::Global(GlobalRequest::SetTabUnsaved {
			id: editor_id,
			unsaved: true
		})
	)?;

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
}
