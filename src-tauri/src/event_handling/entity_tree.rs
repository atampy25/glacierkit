use std::{
	collections::{HashMap, HashSet},
	io::{BufReader, Cursor},
	ops::Deref
};

use anyhow::{anyhow, Context, Result};
use arc_swap::ArcSwap;
use fn_error_context::context;
use itertools::Itertools;
use quickentity_rs::{
	apply_patch, convert_2016_factory_to_modern,
	patch_structs::{Patch, PatchOperation, SubEntityOperation},
	qn_structs::{FullRef, Ref, RefMaybeConstantValue, RefWithConstantValue, SubEntity}
};
use serde::Serialize;
use serde_json::{from_value, json, to_value, Value};
use syntect::{highlighting::ThemeSet, html::highlighted_html_for_string, parsing::SyntaxSet};
use tauri::{AppHandle, Manager};
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	entity::{
		alter_ref_according_to_changelist, calculate_reverse_references, change_reference_to_local, get_decorations,
		get_local_reference, get_recursive_children, is_valid_entity_factory, random_entity_id, CopiedEntityData,
		ReverseReferenceData
	},
	finish_task,
	game_detection::GameVersion,
	model::{
		AppSettings, AppState, EditorData, EditorRequest, EditorValidity, EntityEditorRequest, EntityMetaPaneRequest,
		EntityMonacoRequest, EntityTreeRequest, GlobalRequest, Request
	},
	resourcelib::{
		h2016_convert_binary_to_factory, h2016_convert_cppt, h2_convert_binary_to_factory, h2_convert_cppt,
		h3_convert_binary_to_factory, h3_convert_cppt
	},
	rpkg::{ensure_entity_in_cache, extract_latest_metadata, extract_latest_resource, normalise_to_hash},
	send_notification, send_request, start_task, Notification, NotificationKind
};

#[try_fn]
#[context("Couldn't handle select event")]
pub async fn handle_select(app: &AppHandle, editor_id: Uuid, id: String) -> Result<()> {
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
	{
		let game_version = app_state
			.game_installs
			.iter()
			.try_find(|x| anyhow::Ok(x.path == *install))?
			.context("No such game install")?
			.version;

		let task = start_task(app, format!("Gathering intellisense data for {}", id))?;

		send_request(
			app,
			Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
				EntityMonacoRequest::UpdateIntellisense {
					editor_id: editor_id.to_owned(),
					entity_id: id.to_owned(),
					properties: intellisense.get_properties(
						game_files,
						&app_state.cached_entities,
						hash_list,
						game_version,
						entity,
						&id,
						true
					)?,
					pins: intellisense.get_pins(
						game_files,
						&app_state.cached_entities,
						hash_list,
						game_version,
						entity,
						&id,
						false
					)?
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
}

#[try_fn]
#[context("Couldn't handle delete event")]
pub async fn handle_delete(app: &AppHandle, editor_id: Uuid, id: String) -> Result<()> {
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
}

#[try_fn]
#[context("Couldn't handle paste event")]
pub async fn handle_paste(
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
								entity.external_scenes.push(scene.to_owned());
								added_external_scenes += 1;
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
									entity.external_scenes.push(scene.to_owned());
									added_external_scenes += 1;
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
}

#[try_fn]
#[context("Couldn't handle help menu event")]
pub async fn handle_helpmenu(app: &AppHandle, editor_id: Uuid, entity_id: String) -> Result<()> {
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
		let game_version = app_state
			.game_installs
			.iter()
			.try_find(|x| anyhow::Ok(x.path == *install))?
			.context("No such game install")?
			.version;

		let (properties, pins) = if hash_list
			.entries
			.get(&sub_entity.factory)
			.map(|entry| entry.resource_type == "TEMP")
			.unwrap_or(false)
		{
			ensure_entity_in_cache(
				game_files,
				&app_state.cached_entities,
				game_version,
				hash_list,
				&normalise_to_hash(sub_entity.factory.to_owned())
			)?;

			let underlying_entity = app_state
				.cached_entities
				.get(&normalise_to_hash(sub_entity.factory.to_owned()))
				.unwrap();

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

		let ss = SyntaxSet::load_defaults_newlines();

		send_request(
			app,
			Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
				EntityTreeRequest::ShowHelpMenu {
					editor_id,
					factory: sub_entity.factory.to_owned(),
					input_pins: pins.0,
					output_pins: pins.1,
					default_properties_html: highlighted_html_for_string(
						&properties_data_str,
						&ss,
						ss.find_syntax_by_extension("json").unwrap(),
						&ThemeSet::load_from_reader(&mut BufReader::new(Cursor::new(include_bytes!(
							"../../assets/vs-dark.tmTheme"
						))))?
					)?
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
pub async fn handle_gamebrowseradd(app: &AppHandle, editor_id: Uuid, parent_id: String, file: String) -> Result<()> {
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
		let game_version = app_state
			.game_installs
			.iter()
			.try_find(|x| anyhow::Ok(x.path == *install))?
			.context("No such game install")?
			.version;

		if is_valid_entity_factory(
			&hash_list
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
				.as_str()
			{
				"TEMP" => {
					let (temp_meta, temp_data) = extract_latest_resource(game_files, hash_list, &file)?;

					let factory = match game_version {
						GameVersion::H1 => convert_2016_factory_to_modern(
							&h2016_convert_binary_to_factory(&temp_data)
								.context("Couldn't convert binary data to ResourceLib factory")?
						),

						GameVersion::H2 => h2_convert_binary_to_factory(&temp_data)
							.context("Couldn't convert binary data to ResourceLib factory")?,

						GameVersion::H3 => h3_convert_binary_to_factory(&temp_data)
							.context("Couldn't convert binary data to ResourceLib factory")?
					};

					let blueprint_hash = &temp_meta
						.hash_reference_data
						.get(factory.blueprint_index_in_resource_header as usize)
						.context("Blueprint referenced in factory does not exist in dependencies")?
						.hash;

					let factory_path = hash_list
						.entries
						.get(&file)
						.and_then(|x| x.path.to_owned())
						.unwrap_or(file);

					let blueprint_path = hash_list
						.entries
						.get(blueprint_hash)
						.and_then(|x| x.path.to_owned())
						.unwrap_or(blueprint_hash.to_owned());

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
					let (cppt_meta, cppt_data) = extract_latest_resource(game_files, hash_list, &file)?;

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
						.hash_reference_data
						.get(factory.blueprint_index_in_resource_header as usize)
						.context("Blueprint referenced in factory does not exist in dependencies")?
						.hash;

					let factory_path = hash_list
						.entries
						.get(&file)
						.and_then(|x| x.path.to_owned())
						.unwrap_or(file);

					let blueprint_path = hash_list
						.entries
						.get(blueprint_hash)
						.and_then(|x| x.path.to_owned())
						.unwrap_or(blueprint_hash.to_owned());

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
					let blueprint_hash = extract_latest_metadata(game_files, hash_list, &file)?
						.hash_reference_data
						.into_iter()
						.last()
						.context("ASET had no dependencies")?
						.hash;

					let factory_path = hash_list
						.entries
						.get(&file)
						.and_then(|x| x.path.to_owned())
						.unwrap_or(file);

					let blueprint_path = hash_list
						.entries
						.get(&blueprint_hash)
						.and_then(|x| x.path.to_owned())
						.unwrap_or(blueprint_hash.to_owned());

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
					let blueprint_hash = extract_latest_metadata(game_files, hash_list, &file)?
						.hash_reference_data
						.into_iter()
						.last()
						.context("UICT had no dependencies")?
						.hash;

					let factory_path = hash_list
						.entries
						.get(&file)
						.and_then(|x| x.path.to_owned())
						.unwrap_or(file);

					let blueprint_path = hash_list
						.entries
						.get(&blueprint_hash)
						.and_then(|x| x.path.to_owned())
						.unwrap_or(blueprint_hash.to_owned());

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
					let blueprint_hash = {
						let mut blueprint_hash = String::new();

						for dep in extract_latest_metadata(game_files, hash_list, &file)?
							.hash_reference_data
							.into_iter()
						{
							if extract_latest_metadata(game_files, hash_list, &dep.hash)?.hash_resource_type == "MATB" {
								blueprint_hash = dep.hash.to_owned();
								break;
							}
						}

						if blueprint_hash.is_empty() {
							Err(anyhow!("MATT had no MATB dependency"))?;
						}

						blueprint_hash
					};

					let factory_path = hash_list
						.entries
						.get(&file)
						.and_then(|x| x.path.to_owned())
						.unwrap_or(file);

					let blueprint_path = hash_list
						.entries
						.get(&blueprint_hash)
						.and_then(|x| x.path.to_owned())
						.unwrap_or(blueprint_hash.to_owned());

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
					let blueprint_hash = {
						let mut blueprint_hash = String::new();

						for dep in extract_latest_metadata(game_files, hash_list, &file)?
							.hash_reference_data
							.into_iter()
						{
							let metadata = extract_latest_metadata(game_files, hash_list, &dep.hash)?;

							if metadata.hash_resource_type == "WSWB" || metadata.hash_resource_type == "DSWB" {
								blueprint_hash = dep.hash.to_owned();
								break;
							}
						}

						if blueprint_hash.is_empty() {
							Err(anyhow!("WSWT had no WSWB/DSWB dependency"))?;
						}

						blueprint_hash
					};

					let factory_path = hash_list
						.entries
						.get(&file)
						.and_then(|x| x.path.to_owned())
						.unwrap_or(file);

					let blueprint_path = hash_list
						.entries
						.get(&blueprint_hash)
						.and_then(|x| x.path.to_owned())
						.unwrap_or(blueprint_hash.to_owned());

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
					let blueprint_hash = {
						let mut blueprint_hash = String::new();

						for dep in extract_latest_metadata(game_files, hash_list, &file)?
							.hash_reference_data
							.into_iter()
						{
							if extract_latest_metadata(game_files, hash_list, &dep.hash)?.hash_resource_type == "ECPB" {
								blueprint_hash = dep.hash.to_owned();
								break;
							}
						}

						if blueprint_hash.is_empty() {
							Err(anyhow!("ECPT had no ECPB dependency"))?;
						}

						blueprint_hash
					};

					let factory_path = hash_list
						.entries
						.get(&file)
						.and_then(|x| x.path.to_owned())
						.unwrap_or(file);

					let blueprint_path = hash_list
						.entries
						.get(&blueprint_hash)
						.and_then(|x| x.path.to_owned())
						.unwrap_or(blueprint_hash.to_owned());

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
					let blueprint_hash = {
						let mut blueprint_hash = String::new();

						for dep in extract_latest_metadata(game_files, hash_list, &file)?
							.hash_reference_data
							.into_iter()
						{
							if extract_latest_metadata(game_files, hash_list, &dep.hash)?.hash_resource_type == "AIBB" {
								blueprint_hash = dep.hash.to_owned();
								break;
							}
						}

						if blueprint_hash.is_empty() {
							Err(anyhow!("AIBX had no AIBB dependency"))?;
						}

						blueprint_hash
					};

					let factory_path = hash_list
						.entries
						.get(&file)
						.and_then(|x| x.path.to_owned())
						.unwrap_or(file);

					let blueprint_path = hash_list
						.entries
						.get(&blueprint_hash)
						.and_then(|x| x.path.to_owned())
						.unwrap_or(blueprint_hash.to_owned());

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
					let blueprint_hash = {
						let mut blueprint_hash = String::new();

						for dep in extract_latest_metadata(game_files, hash_list, &file)?
							.hash_reference_data
							.into_iter()
						{
							if extract_latest_metadata(game_files, hash_list, &dep.hash)?.hash_resource_type == "WSGB" {
								blueprint_hash = dep.hash.to_owned();
								break;
							}
						}

						if blueprint_hash.is_empty() {
							Err(anyhow!("WSGT had no WSGB dependency"))?;
						}

						blueprint_hash
					};

					let factory_path = hash_list
						.entries
						.get(&file)
						.and_then(|x| x.path.to_owned())
						.unwrap_or(file);

					let blueprint_path = hash_list
						.entries
						.get(&blueprint_hash)
						.and_then(|x| x.path.to_owned())
						.unwrap_or(blueprint_hash.to_owned());

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
}
