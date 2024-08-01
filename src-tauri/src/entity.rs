use std::str::FromStr;

use anyhow::{anyhow, bail};
use anyhow::{Context, Result};
use dashmap::DashMap;
use fn_error_context::context;
use hashbrown::HashMap;
use hitman_commons::game::GameVersion;
use hitman_commons::hash_list::HashList;
use hitman_commons::metadata::{ResourceID, ResourceType};
use hitman_commons::rpkg_tool::RpkgResourceMeta;
use indexmap::IndexMap;
use itertools::Itertools;
use quickentity_rs::qn_structs::{Entity, FullRef, Ref, RefMaybeConstantValue, RefWithConstantValue, SubEntity};
use rand::{seq::SliceRandom, thread_rng};
use rayon::iter::IntoParallelRefIterator;
use rayon::iter::ParallelIterator;
use rpkg_rs::resource::partition_manager::PartitionManager;
use serde::{Deserialize, Serialize};
use serde_json::{from_value, to_string, Value};
use specta::Type;
use tonytools::hmlanguages;
use tryvial::try_fn;
use velcro::vec;

use crate::languages::get_language_map;
use crate::rpkg::extract_entity;
use crate::{
	model::EditorValidity,
	ores_repo::RepositoryItem,
	rpkg::{extract_latest_metadata, extract_latest_resource}
};

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

	reverse_references.reserve(entity.entities.len());

	for entity_id in entity.entities.keys() {
		reverse_references.insert(entity_id.to_owned(), vec![]);
	}

	for (entity_id, entity) in entity.entities.iter() {
		if let Some(ent) = get_local_reference(&entity.parent) {
			reverse_references.entry(ent).or_default().push(ReverseReference {
				from: entity_id.to_owned(),
				data: ReverseReferenceData::Parent
			});
		}

		for (property_name, property_data) in entity.properties.as_ref().unwrap_or(&Default::default()) {
			if property_data.property_type == "SEntityTemplateReference" {
				if let Some(ent) = get_local_reference(
					&from_value::<Ref>(property_data.value.to_owned()).context("Invalid reference")?
				) {
					reverse_references.entry(ent).or_default().push(ReverseReference {
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
						reverse_references.entry(ent).or_default().push(ReverseReference {
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
						reverse_references.entry(ent).or_default().push(ReverseReference {
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
							reverse_references.entry(ent).or_default().push(ReverseReference {
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
						reverse_references.entry(ent).or_default().push(ReverseReference {
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
						reverse_references.entry(ent).or_default().push(ReverseReference {
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
						reverse_references.entry(ent).or_default().push(ReverseReference {
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
					reverse_references.entry(ent).or_default().push(ReverseReference {
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
					reverse_references.entry(ent).or_default().push(ReverseReference {
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
				.entry(referenced_entity.to_owned())
				.or_default()
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
					.entry(parental_entity.to_owned())
					.or_default()
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

	for _ in 0..12 {
		id.push(*digits.choose(&mut thread_rng()).expect("Slice is not empty"));
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

pub fn get_ref_decoration(
	game_files: &PartitionManager,
	cached_entities: &DashMap<ResourceID, Entity>,
	game_version: GameVersion,
	hash_list: &HashList,
	entity: &Entity,
	reference: &Ref
) -> Option<(String, String)> {
	if let Some(ent) = get_local_reference(reference) {
		Some((ent.to_owned(), entity.entities.get(&ent)?.name.to_owned()))
	} else {
		match reference {
			Ref::Short(None) => None,

			Ref::Full(reference) => Some((reference.entity_ref.to_owned(), {
				extract_entity(
					game_files,
					cached_entities,
					game_version,
					hash_list,
					ResourceID::from_any(reference.external_scene.as_ref().expect("Not a local reference")).ok()?
				)
				.ok()?
				.entities
				.get(&reference.entity_ref)?
				.name
				.to_owned()
			})),

			_ => unreachable!()
		}
	}
}

#[try_fn]
#[context("Couldn't get decoration for LINE {}", line)]
pub fn get_line_decoration(
	game_files: &PartitionManager,
	game_version: GameVersion,
	tonytools_hash_list: &tonytools::hashlist::HashList,
	line: ResourceID
) -> Result<Option<String>> {
	let (res_meta, res_data) = extract_latest_resource(game_files, line)?;

	let (locr_meta, locr_data) = extract_latest_resource(
		game_files,
		res_meta
			.core_info
			.references
			.first()
			.context("No LOCR dependency on LINE")?
			.resource
	)?;

	let locr = {
		let mut iteration = 0;

		loop {
			if let Ok::<_, anyhow::Error>(x) = try {
				let langmap =
					get_language_map(game_version, iteration).context("No more alternate language maps available")?;

				let locr = hmlanguages::locr::LOCR::new(
					tonytools_hash_list.to_owned(),
					game_version.into(),
					langmap.1.to_owned(),
					langmap.0
				)
				.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

				locr.convert(
					&locr_data,
					to_string(&RpkgResourceMeta::from_resource_metadata(locr_meta.to_owned(), false))?
				)
				.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
			} {
				break x;
			} else {
				iteration += 1;

				if get_language_map(game_version, iteration).is_none() {
					bail!("No more alternate language maps available");
				}
			}
		}
	};

	let res_data: [u8; 5] = res_data.try_into().ok().context("Couldn't read LINE data as u32")?;

	let line_id = u32::from_le_bytes(res_data[0..4].try_into().unwrap());

	let line_hash = format!("{:0>8X}", line_id);

	let line_str = tonytools_hash_list.lines.get_by_left(&line_id).cloned();

	if let Some(line_str) = line_str {
		locr.languages
			.get("en")
			.context("No en key in LOCR")?
			.get(&line_str)
			.or_else(|| locr.languages.get("xx").and_then(|x| x.get(&line_str)))
			.and_then(|x| x.as_str().to_owned())
	} else {
		locr.languages
			.get("en")
			.context("No en key in LOCR")?
			.get(&line_hash)
			.or_else(|| locr.languages.get("xx").and_then(|x| x.get(&line_hash)))
			.and_then(|x| x.as_str().to_owned())
	}
}

#[try_fn]
#[context("Couldn't get decorations for sub-entity {}", sub_entity.name)]
pub fn get_decorations(
	game_files: &PartitionManager,
	cached_entities: &DashMap<ResourceID, Entity>,
	repository: &[RepositoryItem],
	hash_list: &HashList,
	game_version: GameVersion,
	tonytools_hash_list: &tonytools::hashlist::HashList,
	sub_entity: &SubEntity,
	entity: &Entity
) -> Result<Vec<(String, String)>> {
	let mut decorations = vec![];

	if let Some(decoration) = get_ref_decoration(
		game_files,
		cached_entities,
		game_version,
		hash_list,
		entity,
		&sub_entity.parent
	) {
		decorations.push(decoration);
	}

	// Hint decoration for unknown paths
	if sub_entity.factory.starts_with('0') {
		if let Some(entry) = hash_list.entries.get(&ResourceID::from_str(&sub_entity.factory)?) {
			if let Some(hint) = entry.hint.as_ref() {
				decorations.push((sub_entity.factory.to_owned(), hint.to_owned()));
			}
		}
	}

	if sub_entity.blueprint.starts_with('0') {
		if let Some(entry) = hash_list.entries.get(&ResourceID::from_str(&sub_entity.blueprint)?) {
			if let Some(hint) = entry.hint.as_ref() {
				decorations.push((sub_entity.blueprint.to_owned(), hint.to_owned()));
			}
		}
	}

	for property_data in sub_entity.properties.as_ref().unwrap_or(&Default::default()).values() {
		if property_data.property_type == "SEntityTemplateReference" {
			if let Some(decoration) = get_ref_decoration(
				game_files,
				cached_entities,
				game_version,
				hash_list,
				entity,
				&from_value::<Ref>(property_data.value.to_owned()).context("Invalid reference")?
			) {
				decorations.push(decoration);
			}
		} else if property_data.property_type == "TArray<SEntityTemplateReference>" {
			for reference in
				from_value::<Vec<Ref>>(property_data.value.to_owned()).context("Invalid reference array")?
			{
				if let Some(decoration) =
					get_ref_decoration(game_files, cached_entities, game_version, hash_list, entity, &reference)
				{
					decorations.push(decoration);
				}
			}
		} else if property_data.property_type == "ZGuid" {
			let repository_id = from_value::<String>(property_data.value.to_owned()).context("Invalid ZGuid")?;

			if let Some(repo_item) = repository.iter().find(|x| x.id.to_string() == repository_id) {
				if let Some(name) = repo_item.data.get("Name").or(repo_item.data.get("CommonName")) {
					decorations.push((
						repository_id.to_string(),
						name.as_str().context("Name or CommonName was not string")?.to_owned()
					));
				}
			}
		} else if property_data.property_type == "TArray<ZGuid>" {
			for repository_id in
				from_value::<Vec<String>>(property_data.value.to_owned()).context("Invalid ZGuid array")?
			{
				if let Some(repo_item) = repository.iter().find(|x| x.id.to_string() == repository_id) {
					if let Some(name) = repo_item.data.get("Name").or(repo_item.data.get("CommonName")) {
						decorations.push((
							repository_id.to_string(),
							name.as_str().context("Name or CommonName was not string")?.to_owned()
						));
					}
				}
			}
		} else if property_data.property_type == "ZRuntimeResourceID" {
			let res = if let Some(obj) = property_data.value.as_object() {
				obj.get("resource")
					.context("No resource property on object ZRuntimeResourceID")?
					.as_str()
					.context("Resource was not string")?
			} else {
				property_data.value.as_str().unwrap_or_default()
			};

			if let Some(entry) = hash_list.entries.get(&ResourceID::from_any(res)?)
				&& entry.resource_type == "LINE"
			{
				decorations.push((
					res.to_owned(),
					get_line_decoration(
						game_files,
						game_version,
						tonytools_hash_list,
						ResourceID::from_any(res)?
					)?
				));
			} else if res.starts_with('0') {
				if let Some(entry) = hash_list.entries.get(&ResourceID::from_str(res)?) {
					if let Some(hint) = entry.hint.as_ref() {
						decorations.push((res.to_owned(), hint.to_owned()));
					}
				}
			}
		} else if property_data.property_type == "TArray<ZRuntimeResourceID>" {
			for val in from_value::<Vec<Value>>(property_data.value.to_owned())
				.context("TArray<ZRuntimeResourceID> was not an array")?
			{
				let res = if let Some(obj) = val.as_object() {
					obj.get("resource")
						.context("No resource property on object ZRuntimeResourceID")?
						.as_str()
						.context("Resource was not string")?
				} else {
					val.as_str().unwrap_or_default()
				};

				if let Some(entry) = hash_list.entries.get(&ResourceID::from_any(res)?)
					&& entry.resource_type == "LINE"
				{
					decorations.push((
						res.to_owned(),
						get_line_decoration(
							game_files,
							game_version,
							tonytools_hash_list,
							ResourceID::from_any(res)?
						)?
					));
				} else if res.starts_with('0') {
					if let Some(entry) = hash_list.entries.get(&ResourceID::from_str(res)?) {
						if let Some(hint) = entry.hint.as_ref() {
							decorations.push((res.to_owned(), hint.to_owned()));
						}
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
				if let Some(decoration) = get_ref_decoration(
					game_files,
					cached_entities,
					game_version,
					hash_list,
					entity,
					&from_value::<Ref>(property_data.value.to_owned()).context("Invalid reference")?
				) {
					decorations.push(decoration);
				}
			} else if property_data.property_type == "TArray<SEntityTemplateReference>" {
				for reference in
					from_value::<Vec<Ref>>(property_data.value.to_owned()).context("Invalid reference array")?
				{
					if let Some(decoration) =
						get_ref_decoration(game_files, cached_entities, game_version, hash_list, entity, &reference)
					{
						decorations.push(decoration);
					}
				}
			} else if property_data.property_type == "ZGuid" {
				let repository_id = from_value::<String>(property_data.value.to_owned()).context("Invalid ZGuid")?;

				if let Some(repo_item) = repository.iter().find(|x| x.id.to_string() == repository_id) {
					if let Some(name) = repo_item.data.get("Name").or(repo_item.data.get("CommonName")) {
						decorations.push((
							repository_id.to_string(),
							name.as_str().context("Name or CommonName was not string")?.to_owned()
						));
					}
				}
			} else if property_data.property_type == "TArray<ZGuid>" {
				for repository_id in
					from_value::<Vec<String>>(property_data.value.to_owned()).context("Invalid ZGuid array")?
				{
					if let Some(repo_item) = repository.iter().find(|x| x.id.to_string() == repository_id) {
						if let Some(name) = repo_item.data.get("Name").or(repo_item.data.get("CommonName")) {
							decorations.push((
								repository_id.to_string(),
								name.as_str().context("Name or CommonName was not string")?.to_owned()
							));
						}
					}
				}
			} else if property_data.property_type == "ZRuntimeResourceID" {
				let res = if let Some(obj) = property_data.value.as_object() {
					obj.get("resource")
						.context("No resource property on object ZRuntimeResourceID")?
						.as_str()
						.context("Resource was not string")?
				} else {
					property_data.value.as_str().unwrap_or_default()
				};

				if let Some(entry) = hash_list.entries.get(&ResourceID::from_any(res)?)
					&& entry.resource_type == "LINE"
				{
					decorations.push((
						res.to_owned(),
						get_line_decoration(
							game_files,
							game_version,
							tonytools_hash_list,
							ResourceID::from_any(res)?
						)?
					));
				} else if res.starts_with('0') {
					if let Some(entry) = hash_list.entries.get(&ResourceID::from_str(res)?) {
						if let Some(hint) = entry.hint.as_ref() {
							decorations.push((res.to_owned(), hint.to_owned()));
						}
					}
				}
			} else if property_data.property_type == "TArray<ZRuntimeResourceID>" {
				for val in from_value::<Vec<Value>>(property_data.value.to_owned())
					.context("TArray<ZRuntimeResourceID> was not an array")?
				{
					let res = if let Some(obj) = val.as_object() {
						obj.get("resource")
							.context("No resource property on object ZRuntimeResourceID")?
							.as_str()
							.context("Resource was not string")?
					} else {
						val.as_str().unwrap_or_default()
					};

					if let Some(entry) = hash_list.entries.get(&ResourceID::from_any(res)?)
						&& entry.resource_type == "LINE"
					{
						decorations.push((
							res.to_owned(),
							get_line_decoration(
								game_files,
								game_version,
								tonytools_hash_list,
								ResourceID::from_any(res)?
							)?
						));
					} else if res.starts_with('0') {
						if let Some(entry) = hash_list.entries.get(&ResourceID::from_str(res)?) {
							if let Some(hint) = entry.hint.as_ref() {
								decorations.push((res.to_owned(), hint.to_owned()));
							}
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

				if let Some(decoration) =
					get_ref_decoration(game_files, cached_entities, game_version, hash_list, entity, reference)
				{
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

				if let Some(decoration) =
					get_ref_decoration(game_files, cached_entities, game_version, hash_list, entity, reference)
				{
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

				if let Some(decoration) =
					get_ref_decoration(game_files, cached_entities, game_version, hash_list, entity, reference)
				{
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
				game_files,
				cached_entities,
				game_version,
				hash_list,
				entity,
				&alias_data.original_entity
			) {
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
			if let Some(decoration) =
				get_ref_decoration(game_files, cached_entities, game_version, hash_list, entity, reference)
			{
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
			game_files,
			cached_entities,
			game_version,
			hash_list,
			entity,
			&Ref::Short(Some(referenced_entity.to_owned()))
		) {
			decorations.push(decoration);
		}
	}

	for member_of in sub_entity.subsets.as_ref().unwrap_or(&Default::default()).values() {
		for parental_entity in member_of {
			if let Some(decoration) = get_ref_decoration(
				game_files,
				cached_entities,
				game_version,
				hash_list,
				entity,
				&Ref::Short(Some(parental_entity.to_owned()))
			) {
				decorations.push(decoration);
			}
		}
	}

	if hash_list
		.entries
		.get(&ResourceID::from_any(&sub_entity.factory)?)
		.map(|entry| entry.resource_type == "MATT")
		.unwrap_or(false)
	{
		if let Some(mati) = extract_latest_metadata(game_files, ResourceID::from_any(&sub_entity.factory)?)?
			.core_info
			.references
			.into_iter()
			.find(|x| {
				hash_list
					.entries
					.get(&x.resource)
					.map(|entry| entry.resource_type == "MATI")
					.unwrap_or(false)
			}) {
			if let Some(mate) = extract_latest_metadata(game_files, mati.resource)?
				.core_info
				.references
				.into_iter()
				.find(|x| {
					hash_list
						.entries
						.get(&x.resource)
						.map(|entry| entry.resource_type == "MATE")
						.unwrap_or(false)
				}) {
				let mate_data = extract_latest_resource(game_files, mate.resource)?.1;

				let mut beginning = mate_data.len() - 1;
				while mate_data[beginning] == 0 || (mate_data[beginning] > 31 && mate_data[beginning] < 127) {
					beginning -= 1;
				}
				beginning += 1;

				decorations.extend(
					String::from_utf8(mate_data[beginning..mate_data.len() - 1].into())?
						.split('\x00')
						.filter(|x| !x.is_empty() && x.trim().as_bytes().iter().all(|x| *x > 31 && *x < 127))
						.map(|x| x.trim().to_owned())
						.tuples()
						.map(|(prop, friendly)| {
							(
								if prop.starts_with("map") {
									prop.chars().skip(3).collect()
								} else {
									prop
								},
								if friendly.starts_with("map") {
									friendly.chars().skip(3).collect()
								} else {
									friendly
								}
							)
						})
				);
			}
		}
	}

	decorations.into_iter().unique().collect()
}

pub fn is_valid_entity_factory(resource_type: ResourceType) -> bool {
	resource_type == "TEMP"
		|| resource_type == "CPPT"
		|| resource_type == "ASET"
		|| resource_type == "UICT"
		|| resource_type == "MATT"
		|| resource_type == "WSWT"
		|| resource_type == "ECPT"
		|| resource_type == "AIBX"
		|| resource_type == "WSGT"
}

pub fn is_valid_entity_blueprint(resource_type: ResourceType) -> bool {
	resource_type == "TBLU"
		|| resource_type == "CBLU"
		|| resource_type == "ASEB"
		|| resource_type == "UICB"
		|| resource_type == "MATB"
		|| resource_type == "WSWB"
		|| resource_type == "DSWB"
		|| resource_type == "ECPB"
		|| resource_type == "AIBB"
		|| resource_type == "WSGB"
}

/// New, modified, removed (ID, name, parent, factory, has reverse parent refs)
pub fn get_diff_info(
	original: &Entity,
	modified: &Entity
) -> (Vec<String>, Vec<String>, Vec<(String, String, Ref, String, bool)>) {
	let mut old_reverse_parent_refs: HashMap<String, Vec<String>> = HashMap::new();

	for (entity_id, entity_data) in original.entities.iter() {
		match entity_data.parent {
			Ref::Full(ref reference) if reference.external_scene.is_none() => {
				old_reverse_parent_refs
					.entry(reference.entity_ref.to_owned())
					.and_modify(|x| x.push(entity_id.to_owned()))
					.or_insert(vec![entity_id.to_owned()]);
			}

			Ref::Short(Some(ref reference)) => {
				old_reverse_parent_refs
					.entry(reference.to_owned())
					.and_modify(|x| x.push(entity_id.to_owned()))
					.or_insert(vec![entity_id.to_owned()]);
			}

			_ => {}
		}
	}

	let removed = original
		.entities
		.par_iter()
		.filter(|&(id, _)| !modified.entities.contains_key(id))
		.map(|(id, orig)| {
			(
				id.to_owned(),
				orig.name.to_owned(),
				orig.parent.to_owned(),
				orig.factory.to_owned(),
				old_reverse_parent_refs.contains_key(id)
			)
		})
		.collect();

	let mut diff = modified
		.entities
		.par_iter()
		.filter_map(|(id, modif)| {
			if let Some(orig) = original.entities.get(id) {
				if modif != orig {
					Some(("changed", id))
				} else {
					None
				}
			} else {
				Some(("new", id))
			}
		})
		.collect::<Vec<_>>()
		.into_iter()
		.into_group_map();

	(
		diff.remove("new")
			.map(|x| x.into_iter().cloned().collect())
			.unwrap_or_default(),
		diff.remove("changed")
			.map(|x| x.into_iter().cloned().collect())
			.unwrap_or_default(),
		removed
	)
}
