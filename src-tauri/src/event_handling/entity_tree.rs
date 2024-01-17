use std::collections::{HashMap, HashSet};

use anyhow::{anyhow, Context, Result};
use arboard::Clipboard;
use fn_error_context::context;
use itertools::Itertools;
use quickentity_rs::{
	apply_patch,
	patch_structs::{Patch, PatchOperation, SubEntityOperation},
	qn_structs::{FullRef, Ref, RefMaybeConstantValue, RefWithConstantValue}
};
use serde_json::{from_str, from_value, to_value, Value};
use tauri::{AppHandle, Manager};
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	entity::{
		alter_ref_according_to_changelist, calculate_reverse_references, change_reference_to_local,
		get_local_reference, get_recursive_children, random_entity_id, CopiedEntityData, ReverseReferenceData
	},
	finish_task,
	model::{AppState, EditorData, EditorRequest, EntityEditorRequest, EntityTreeRequest, GlobalRequest, Request},
	send_notification, send_request, start_task, Notification, NotificationKind
};

#[try_fn]
#[context("Couldn't handle paste event")]
pub async fn handle_delete(app: &AppHandle, editor_id: Uuid, id: String) -> Result<()> {
	let app_state = app.state::<AppState>();

	let task = start_task(app, format!("Deleting entity {}", id))?;

	let mut editor_state = app_state.editor_states.write().await;
	let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

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
								.input_copying
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
			title: format!("Deleted {} entities", entities_to_delete.len()),
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
}

#[try_fn]
#[context("Couldn't handle paste event")]
pub async fn handle_paste(app: &AppHandle, editor_id: Uuid, parent_id: String) -> Result<()> {
	let app_state = app.state::<AppState>();

	let mut paste_data = from_str::<CopiedEntityData>(&Clipboard::new()?.get_text()?)?;

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

	let mut editor_state = app_state.editor_states.write().await;
	let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

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
					"{} external scene{} been added to the entity to ensure that copied references work.",
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
