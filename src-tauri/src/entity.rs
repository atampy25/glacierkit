use std::{collections::HashMap, path::PathBuf};

use anyhow::{Context, Result};
use fn_error_context::context;
use indexmap::IndexMap;
use itertools::Itertools;
use parking_lot::RwLock;
use quickentity_rs::qn_structs::{Entity, FullRef, Ref, RefMaybeConstantValue, RefWithConstantValue, SubEntity};
use rand::{seq::SliceRandom, thread_rng};
use rpkg_rs::runtime::resource::resource_package::ResourcePackage;
use serde::{Deserialize, Serialize};
use serde_json::from_value;
use specta::Type;
use tryvial::try_fn;
use velcro::vec;

use crate::{game_detection::GameVersion, model::EditorValidity, rpkg::extract_entity};

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReverseReference {
	pub from: String,
	pub data: ReverseReferenceData
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum ReverseReferenceData {
	Parent,
	Property {
		property_name: String
	},
	PlatformSpecificProperty {
		property_name: String,
		platform: String
	},
	Event {
		event: String,
		trigger: String
	},
	InputCopy {
		trigger: String,
		propagate: String
	},
	OutputCopy {
		event: String,
		propagate: String
	},
	PropertyAlias {
		aliased_name: String,
		original_property: String
	},
	ExposedEntity {
		exposed_name: String
	},
	ExposedInterface {
		interface: String
	},
	Subset {
		subset: String
	}
}

/// Get the local reference contained within a Ref, or None if it's an external or null reference.
pub fn get_local_reference(reference: &Ref) -> Option<String> {
	match reference {
		Ref::Short(Some(ref ent)) => Some(ent.to_owned()),

		Ref::Full(ref reference) => {
			if reference.external_scene.is_none() {
				Some(reference.entity_ref.to_owned())
			} else {
				None
			}
		}

		_ => None
	}
}

#[try_fn]
#[context("Couldn't calculate reverse references")]
pub fn calculate_reverse_references(entity: &Entity) -> Result<HashMap<String, Vec<ReverseReference>>> {
	let mut reverse_references: HashMap<String, Vec<ReverseReference>> = HashMap::new();

	for entity_id in entity.entities.keys() {
		reverse_references.insert(entity_id.to_owned(), vec![]);
	}

	for (entity_id, entity) in entity.entities.iter() {
		if let Some(ent) = get_local_reference(&entity.parent) {
			reverse_references.get_mut(&ent).unwrap().push(ReverseReference {
				from: entity_id.to_owned(),
				data: ReverseReferenceData::Parent
			});
		}

		for (property_name, property_data) in entity.properties.as_ref().unwrap_or(&Default::default()) {
			if property_data.property_type == "SEntityTemplateReference" {
				if let Some(ent) = get_local_reference(
					&from_value::<Ref>(property_data.value.to_owned()).context("Invalid reference")?
				) {
					reverse_references.get_mut(&ent).unwrap().push(ReverseReference {
						from: entity_id.to_owned(),
						data: ReverseReferenceData::Property {
							property_name: property_name.to_owned()
						}
					});
				}
			} else if property_data.property_type == "TArray<SEntityTemplateReference>" {
				for reference in
					from_value::<Vec<Ref>>(property_data.value.to_owned()).context("Invalid reference array")?
				{
					if let Some(ent) = get_local_reference(&reference) {
						reverse_references.get_mut(&ent).unwrap().push(ReverseReference {
							from: entity_id.to_owned(),
							data: ReverseReferenceData::Property {
								property_name: property_name.to_owned()
							}
						});
					}
				}
			}
		}

		for (platform, properties) in entity
			.platform_specific_properties
			.as_ref()
			.unwrap_or(&Default::default())
		{
			for (property_name, property_data) in properties {
				if property_data.property_type == "SEntityTemplateReference" {
					if let Some(ent) = get_local_reference(
						&from_value::<Ref>(property_data.value.to_owned()).context("Invalid reference")?
					) {
						reverse_references.get_mut(&ent).unwrap().push(ReverseReference {
							from: entity_id.to_owned(),
							data: ReverseReferenceData::PlatformSpecificProperty {
								property_name: property_name.to_owned(),
								platform: platform.to_owned()
							}
						});
					}
				} else if property_data.property_type == "TArray<SEntityTemplateReference>" {
					for reference in
						from_value::<Vec<Ref>>(property_data.value.to_owned()).context("Invalid reference array")?
					{
						if let Some(ent) = get_local_reference(&reference) {
							reverse_references.get_mut(&ent).unwrap().push(ReverseReference {
								from: entity_id.to_owned(),
								data: ReverseReferenceData::PlatformSpecificProperty {
									property_name: property_name.to_owned(),
									platform: platform.to_owned()
								}
							});
						}
					}
				}
			}
		}

		for (event, triggers) in entity.events.as_ref().unwrap_or(&Default::default()) {
			for (trigger, trigger_entities) in triggers {
				for reference in trigger_entities {
					let reference = match reference {
						RefMaybeConstantValue::Ref(x) => x,
						RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue { entity_ref, .. }) => {
							entity_ref
						}
					};

					if let Some(ent) = get_local_reference(reference) {
						reverse_references.get_mut(&ent).unwrap().push(ReverseReference {
							from: entity_id.to_owned(),
							data: ReverseReferenceData::Event {
								event: event.to_owned(),
								trigger: trigger.to_owned()
							}
						});
					}
				}
			}
		}

		for (trigger, propagates) in entity.input_copying.as_ref().unwrap_or(&Default::default()) {
			for (propagate, propagate_entities) in propagates {
				for reference in propagate_entities {
					let reference = match reference {
						RefMaybeConstantValue::Ref(x) => x,
						RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue { entity_ref, .. }) => {
							entity_ref
						}
					};

					if let Some(ent) = get_local_reference(reference) {
						reverse_references.get_mut(&ent).unwrap().push(ReverseReference {
							from: entity_id.to_owned(),
							data: ReverseReferenceData::InputCopy {
								trigger: trigger.to_owned(),
								propagate: propagate.to_owned()
							}
						});
					}
				}
			}
		}

		for (event, propagates) in entity.output_copying.as_ref().unwrap_or(&Default::default()) {
			for (propagate, propagate_entities) in propagates {
				for reference in propagate_entities {
					let reference = match reference {
						RefMaybeConstantValue::Ref(x) => x,
						RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue { entity_ref, .. }) => {
							entity_ref
						}
					};

					if let Some(ent) = get_local_reference(reference) {
						reverse_references.get_mut(&ent).unwrap().push(ReverseReference {
							from: entity_id.to_owned(),
							data: ReverseReferenceData::OutputCopy {
								event: event.to_owned(),
								propagate: propagate.to_owned()
							}
						});
					}
				}
			}
		}

		for (aliased_name, aliases) in entity.property_aliases.as_ref().unwrap_or(&Default::default()) {
			for alias_data in aliases {
				if let Some(ent) = get_local_reference(&alias_data.original_entity) {
					reverse_references.get_mut(&ent).unwrap().push(ReverseReference {
						from: entity_id.to_owned(),
						data: ReverseReferenceData::PropertyAlias {
							aliased_name: aliased_name.to_owned(),
							original_property: alias_data.original_property.to_owned()
						}
					});
				}
			}
		}

		for (exposed_name, exposed_entity) in entity.exposed_entities.as_ref().unwrap_or(&Default::default()) {
			for reference in &exposed_entity.refers_to {
				if let Some(ent) = get_local_reference(reference) {
					reverse_references.get_mut(&ent).unwrap().push(ReverseReference {
						from: entity_id.to_owned(),
						data: ReverseReferenceData::ExposedEntity {
							exposed_name: exposed_name.to_owned()
						}
					});
				}
			}
		}

		for (interface, referenced_entity) in entity.exposed_interfaces.as_ref().unwrap_or(&Default::default()) {
			reverse_references
				.get_mut(referenced_entity)
				.unwrap()
				.push(ReverseReference {
					from: entity_id.to_owned(),
					data: ReverseReferenceData::ExposedInterface {
						interface: interface.to_owned()
					}
				});
		}

		for (subset, member_of) in entity.subsets.as_ref().unwrap_or(&Default::default()) {
			for parental_entity in member_of {
				reverse_references
					.get_mut(parental_entity)
					.unwrap()
					.push(ReverseReference {
						from: entity_id.to_owned(),
						data: ReverseReferenceData::Subset {
							subset: subset.to_owned()
						}
					});
			}
		}
	}

	reverse_references
}

/// Given a sub-entity's ID, get a list of all recursive children of that sub-entity, including the target sub-entity itself.
#[try_fn]
#[context("Couldn't get recursive children of {}", target)]
pub fn get_recursive_children(
	entity: &Entity,
	target: &str,
	reverse_references: &HashMap<String, Vec<ReverseReference>>
) -> Result<Vec<String>> {
	let child_ents = entity
		.entities
		.iter()
		.filter(|(_, x)| get_local_reference(&x.parent).map(|x| x == target).unwrap_or(false))
		.map(|(x, _)| x)
		.cloned()
		.collect_vec();

	let mut children = vec![target.to_owned()];

	for child in child_ents {
		children.extend(get_recursive_children(entity, &child, reverse_references)?);
	}

	children
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CopiedEntityData {
	/// Which entity has been copied (and should be parented to the selection when pasting).
	pub root_entity: String,

	pub data: IndexMap<String, SubEntity>
}

pub fn random_entity_id() -> String {
	let digits = [
		'0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f'
	];

	let mut id = String::from("cafe");

	for _ in 0..14 {
		id.push(*digits.choose(&mut thread_rng()).unwrap());
	}

	id
}

/// Changes a Ref to refer to a given local entity, keeping the exposed entity the same if there was one.
pub fn change_reference_to_local(reference: &Ref, local: String) -> Ref {
	match reference {
		Ref::Short(_) => Ref::Short(Some(local)),

		Ref::Full(ref reference) => {
			if let Some(exposed) = reference.exposed_entity.as_ref() {
				Ref::Full(FullRef {
					entity_ref: local,
					exposed_entity: Some(exposed.to_owned()),
					external_scene: None
				})
			} else {
				Ref::Short(Some(local))
			}
		}
	}
}

/// Changes a Ref based on the given changelist (original entity ID -> new entity ID). Used for pasting.
pub fn alter_ref_according_to_changelist(reference: &Ref, changelist: &HashMap<String, String>) -> Ref {
	match reference {
		Ref::Short(None) => reference.to_owned(),

		Ref::Short(Some(local)) => {
			if let Some(changed) = changelist.get(local) {
				Ref::Short(Some(changed.to_owned()))
			} else {
				reference.to_owned()
			}
		}

		Ref::Full(ref full_ref) => {
			if let Some(changed) = changelist.get(&full_ref.entity_ref) {
				change_reference_to_local(reference, changed.to_owned())
			} else {
				reference.to_owned()
			}
		}
	}
}

#[try_fn]
#[context("Couldn't check whether local references refer to existing entities")]
pub fn check_local_references_exist(sub_entity: &SubEntity, entity: &Entity) -> Result<EditorValidity> {
	if let Some(ent) = get_local_reference(&sub_entity.parent) {
		if !entity.entities.contains_key(&ent) {
			return Ok(EditorValidity::Invalid(format!("Invalid reference {}", ent)));
		}
	}

	for property_data in sub_entity.properties.as_ref().unwrap_or(&Default::default()).values() {
		if property_data.property_type == "SEntityTemplateReference" {
			if let Some(ent) =
				get_local_reference(&from_value::<Ref>(property_data.value.to_owned()).context("Invalid reference")?)
			{
				if !entity.entities.contains_key(&ent) {
					return Ok(EditorValidity::Invalid(format!("Invalid reference {}", ent)));
				}
			}
		} else if property_data.property_type == "TArray<SEntityTemplateReference>" {
			for reference in
				from_value::<Vec<Ref>>(property_data.value.to_owned()).context("Invalid reference array")?
			{
				if let Some(ent) = get_local_reference(&reference) {
					if !entity.entities.contains_key(&ent) {
						return Ok(EditorValidity::Invalid(format!("Invalid reference {}", ent)));
					}
				}
			}
		}
	}

	for properties in sub_entity
		.platform_specific_properties
		.as_ref()
		.unwrap_or(&Default::default())
		.values()
	{
		for property_data in properties.values() {
			if property_data.property_type == "SEntityTemplateReference" {
				if let Some(ent) = get_local_reference(
					&from_value::<Ref>(property_data.value.to_owned()).context("Invalid reference")?
				) {
					if !entity.entities.contains_key(&ent) {
						return Ok(EditorValidity::Invalid(format!("Invalid reference {}", ent)));
					}
				}
			} else if property_data.property_type == "TArray<SEntityTemplateReference>" {
				for reference in
					from_value::<Vec<Ref>>(property_data.value.to_owned()).context("Invalid reference array")?
				{
					if let Some(ent) = get_local_reference(&reference) {
						if !entity.entities.contains_key(&ent) {
							return Ok(EditorValidity::Invalid(format!("Invalid reference {}", ent)));
						}
					}
				}
			}
		}
	}

	for triggers in sub_entity.events.as_ref().unwrap_or(&Default::default()).values() {
		for trigger_entities in triggers.values() {
			for reference in trigger_entities {
				let reference = match reference {
					RefMaybeConstantValue::Ref(x) => x,
					RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue { entity_ref, .. }) => entity_ref
				};

				if let Some(ent) = get_local_reference(reference) {
					if !entity.entities.contains_key(&ent) {
						return Ok(EditorValidity::Invalid(format!("Invalid reference {}", ent)));
					}
				}
			}
		}
	}

	for propagates in sub_entity
		.input_copying
		.as_ref()
		.unwrap_or(&Default::default())
		.values()
	{
		for propagate_entities in propagates.values() {
			for reference in propagate_entities {
				let reference = match reference {
					RefMaybeConstantValue::Ref(x) => x,
					RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue { entity_ref, .. }) => entity_ref
				};

				if let Some(ent) = get_local_reference(reference) {
					if !entity.entities.contains_key(&ent) {
						return Ok(EditorValidity::Invalid(format!("Invalid reference {}", ent)));
					}
				}
			}
		}
	}

	for propagates in sub_entity
		.output_copying
		.as_ref()
		.unwrap_or(&Default::default())
		.values()
	{
		for propagate_entities in propagates.values() {
			for reference in propagate_entities {
				let reference = match reference {
					RefMaybeConstantValue::Ref(x) => x,
					RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue { entity_ref, .. }) => entity_ref
				};

				if let Some(ent) = get_local_reference(reference) {
					if !entity.entities.contains_key(&ent) {
						return Ok(EditorValidity::Invalid(format!("Invalid reference {}", ent)));
					}
				}
			}
		}
	}

	for aliases in sub_entity
		.property_aliases
		.as_ref()
		.unwrap_or(&Default::default())
		.values()
	{
		for alias_data in aliases {
			if let Some(ent) = get_local_reference(&alias_data.original_entity) {
				if !entity.entities.contains_key(&ent) {
					return Ok(EditorValidity::Invalid(format!("Invalid reference {}", ent)));
				}
			}
		}
	}

	for exposed_entity in sub_entity
		.exposed_entities
		.as_ref()
		.unwrap_or(&Default::default())
		.values()
	{
		for reference in &exposed_entity.refers_to {
			if let Some(ent) = get_local_reference(reference) {
				if !entity.entities.contains_key(&ent) {
					return Ok(EditorValidity::Invalid(format!("Invalid reference {}", ent)));
				}
			}
		}
	}

	for referenced_entity in sub_entity
		.exposed_interfaces
		.as_ref()
		.unwrap_or(&Default::default())
		.values()
	{
		if !entity.entities.contains_key(referenced_entity) {
			return Ok(EditorValidity::Invalid(format!(
				"Invalid reference {}",
				referenced_entity
			)));
		}
	}

	for member_of in sub_entity.subsets.as_ref().unwrap_or(&Default::default()).values() {
		for parental_entity in member_of {
			if !entity.entities.contains_key(parental_entity) {
				return Ok(EditorValidity::Invalid(format!(
					"Invalid reference {}",
					parental_entity
				)));
			}
		}
	}

	EditorValidity::Valid
}

#[try_fn]
#[context("Couldn't get name of referenced entity {:?}", reference)]
fn get_ref_decoration(
	resource_packages: &IndexMap<PathBuf, ResourcePackage>,
	cached_entities: &RwLock<HashMap<String, Entity>>,
	game_version: GameVersion,
	hash_list_mapping: &HashMap<String, (String, Option<String>)>,
	entity: &Entity,
	reference: &Ref
) -> Result<Option<(String, String)>> {
	if let Some(ent) = get_local_reference(reference) {
		Some((
			ent.to_owned(),
			entity
				.entities
				.get(&ent)
				.context("Referenced local entity doesn't exist")?
				.name
				.to_owned()
		))
	} else {
		match reference {
			Ref::Short(None) => None,

			Ref::Full(reference) => Some((
				reference.entity_ref.to_owned(),
				extract_entity(
					resource_packages,
					cached_entities,
					game_version,
					hash_list_mapping,
					reference.external_scene.as_ref().unwrap()
				)?
				.entities
				.get(&reference.entity_ref)
				.context("Referenced entity doesn't exist in external scene")?
				.name
				.to_owned()
			)),

			_ => unreachable!()
		}
	}
}

#[try_fn]
#[context("Couldn't get decorations for sub-entity {}", sub_entity.name)]
pub fn get_decorations(
	resource_packages: &IndexMap<PathBuf, ResourcePackage>,
	cached_entities: &RwLock<HashMap<String, Entity>>,
	hash_list_mapping: &HashMap<String, (String, Option<String>)>,
	game_version: GameVersion,
	sub_entity: &SubEntity,
	entity: &Entity
) -> Result<Vec<(String, String)>> {
	let mut decorations = vec![];

	if let Some(decoration) = get_ref_decoration(
		resource_packages,
		cached_entities,
		game_version,
		hash_list_mapping,
		entity,
		&sub_entity.parent
	)? {
		decorations.push(decoration);
	}

	for property_data in sub_entity.properties.as_ref().unwrap_or(&Default::default()).values() {
		if property_data.property_type == "SEntityTemplateReference" {
			if let Some(decoration) = get_ref_decoration(
				resource_packages,
				cached_entities,
				game_version,
				hash_list_mapping,
				entity,
				&from_value::<Ref>(property_data.value.to_owned()).context("Invalid reference")?
			)? {
				decorations.push(decoration);
			}
		} else if property_data.property_type == "TArray<SEntityTemplateReference>" {
			for reference in
				from_value::<Vec<Ref>>(property_data.value.to_owned()).context("Invalid reference array")?
			{
				if let Some(decoration) = get_ref_decoration(
					resource_packages,
					cached_entities,
					game_version,
					hash_list_mapping,
					entity,
					&reference
				)? {
					decorations.push(decoration);
				}
			}
		}
	}

	for properties in sub_entity
		.platform_specific_properties
		.as_ref()
		.unwrap_or(&Default::default())
		.values()
	{
		for property_data in properties.values() {
			if property_data.property_type == "SEntityTemplateReference" {
				if let Some(decoration) = get_ref_decoration(
					resource_packages,
					cached_entities,
					game_version,
					hash_list_mapping,
					entity,
					&from_value::<Ref>(property_data.value.to_owned()).context("Invalid reference")?
				)? {
					decorations.push(decoration);
				}
			} else if property_data.property_type == "TArray<SEntityTemplateReference>" {
				for reference in
					from_value::<Vec<Ref>>(property_data.value.to_owned()).context("Invalid reference array")?
				{
					if let Some(decoration) = get_ref_decoration(
						resource_packages,
						cached_entities,
						game_version,
						hash_list_mapping,
						entity,
						&reference
					)? {
						decorations.push(decoration);
					}
				}
			}
		}
	}

	for triggers in sub_entity.events.as_ref().unwrap_or(&Default::default()).values() {
		for trigger_entities in triggers.values() {
			for reference in trigger_entities {
				let reference = match reference {
					RefMaybeConstantValue::Ref(x) => x,
					RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue { entity_ref, .. }) => entity_ref
				};

				if let Some(decoration) = get_ref_decoration(
					resource_packages,
					cached_entities,
					game_version,
					hash_list_mapping,
					entity,
					reference
				)? {
					decorations.push(decoration);
				}
			}
		}
	}

	for propagates in sub_entity
		.input_copying
		.as_ref()
		.unwrap_or(&Default::default())
		.values()
	{
		for propagate_entities in propagates.values() {
			for reference in propagate_entities {
				let reference = match reference {
					RefMaybeConstantValue::Ref(x) => x,
					RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue { entity_ref, .. }) => entity_ref
				};

				if let Some(decoration) = get_ref_decoration(
					resource_packages,
					cached_entities,
					game_version,
					hash_list_mapping,
					entity,
					reference
				)? {
					decorations.push(decoration);
				}
			}
		}
	}

	for propagates in sub_entity
		.output_copying
		.as_ref()
		.unwrap_or(&Default::default())
		.values()
	{
		for propagate_entities in propagates.values() {
			for reference in propagate_entities {
				let reference = match reference {
					RefMaybeConstantValue::Ref(x) => x,
					RefMaybeConstantValue::RefWithConstantValue(RefWithConstantValue { entity_ref, .. }) => entity_ref
				};

				if let Some(decoration) = get_ref_decoration(
					resource_packages,
					cached_entities,
					game_version,
					hash_list_mapping,
					entity,
					reference
				)? {
					decorations.push(decoration);
				}
			}
		}
	}

	for aliases in sub_entity
		.property_aliases
		.as_ref()
		.unwrap_or(&Default::default())
		.values()
	{
		for alias_data in aliases {
			if let Some(decoration) = get_ref_decoration(
				resource_packages,
				cached_entities,
				game_version,
				hash_list_mapping,
				entity,
				&alias_data.original_entity
			)? {
				decorations.push(decoration);
			}
		}
	}

	for exposed_entity in sub_entity
		.exposed_entities
		.as_ref()
		.unwrap_or(&Default::default())
		.values()
	{
		for reference in &exposed_entity.refers_to {
			if let Some(decoration) = get_ref_decoration(
				resource_packages,
				cached_entities,
				game_version,
				hash_list_mapping,
				entity,
				reference
			)? {
				decorations.push(decoration);
			}
		}
	}

	for referenced_entity in sub_entity
		.exposed_interfaces
		.as_ref()
		.unwrap_or(&Default::default())
		.values()
	{
		if let Some(decoration) = get_ref_decoration(
			resource_packages,
			cached_entities,
			game_version,
			hash_list_mapping,
			entity,
			&Ref::Short(Some(referenced_entity.to_owned()))
		)? {
			decorations.push(decoration);
		}
	}

	for member_of in sub_entity.subsets.as_ref().unwrap_or(&Default::default()).values() {
		for parental_entity in member_of {
			if let Some(decoration) = get_ref_decoration(
				resource_packages,
				cached_entities,
				game_version,
				hash_list_mapping,
				entity,
				&Ref::Short(Some(parental_entity.to_owned()))
			)? {
				decorations.push(decoration);
			}
		}
	}

	decorations.into_iter().unique().collect()
}
