use anyhow::{Context, Result, anyhow, bail};
use dashmap::DashMap;
use ecow::EcoString;
use fn_error_context::context;
use hashbrown::{HashMap, HashSet};
use hitman_commons::{
	game::GameVersion,
	metadata::{ResourceType, RuntimeID},
	rpkg_tool::RpkgResourceMeta
};
use indexmap::IndexMap;
use itertools::Itertools;
use quickentity_rs::{
	entity::{Entity, EntityID, Ref, SubEntity},
	variant::Variant
};
use rand::{rng, seq::IndexedRandom};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use rpkg_rs::resource::partition_manager::PartitionManager;
use serde::{Deserialize, Serialize};
use serde_json::to_string;
use specta::Type;
use tonytools::hmlanguages;
use tryvial::{try_block, try_fn};
use velcro::vec;

use crate::{
	languages::get_language_map,
	model::EditorValidity,
	ores_repo::RepositoryItem,
	rpkg::{extract_entity, extract_latest_metadata, extract_latest_resource}
};

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReverseReference {
	pub from: EntityID,
	pub data: ReverseReferenceData
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum ReverseReferenceData {
	Parent,
	Property {
		#[specta(type = String)]
		property_name: EcoString
	},
	PlatformSpecificProperty {
		#[specta(type = String)]
		property_name: EcoString,

		#[specta(type = String)]
		platform: EcoString
	},
	Event {
		#[specta(type = String)]
		event: EcoString,

		#[specta(type = String)]
		trigger: EcoString
	},
	InputCopy {
		#[specta(type = String)]
		trigger: EcoString,

		#[specta(type = String)]
		propagate: EcoString
	},
	OutputCopy {
		#[specta(type = String)]
		event: EcoString,

		#[specta(type = String)]
		propagate: EcoString
	},
	PropertyAlias {
		#[specta(type = String)]
		aliased_name: EcoString,

		#[specta(type = String)]
		original_property: EcoString
	},
	ExposedEntity {
		#[specta(type = String)]
		exposed_name: EcoString
	},
	ExposedInterface {
		#[specta(type = String)]
		interface: EcoString
	},
	Subset {
		#[specta(type = String)]
		subset: EcoString
	}
}

pub fn visit_variant(variant: &Variant, handle: &mut impl FnMut(&Variant)) {
	handle(variant);

	match variant {
		Variant::PairStringVariant(_, variant) => visit_variant(variant, handle),

		Variant::Variant(variant) => visit_variant(variant, handle),

		Variant::Array(_, variants) => {
			for variant in variants {
				visit_variant(variant, handle);
			}
		}

		_ => {}
	}
}

pub fn visit_variant_mut(variant: &mut Variant, handle: &mut impl FnMut(&mut Variant)) {
	handle(variant);

	match variant {
		Variant::PairStringVariant(_, variant) => visit_variant_mut(variant, handle),

		Variant::Variant(variant) => visit_variant_mut(variant, handle),

		Variant::Array(_, variants) => {
			for variant in variants {
				visit_variant_mut(variant, handle);
			}
		}

		_ => {}
	}
}

#[try_fn]
#[context("Couldn't calculate reverse references")]
pub fn calculate_reverse_references(entity: &Entity) -> Result<HashMap<EntityID, Vec<ReverseReference>>> {
	let mut reverse_references: HashMap<EntityID, Vec<ReverseReference>> = HashMap::new();

	reverse_references.reserve(entity.entities.len());

	for entity_id in entity.entities.keys() {
		reverse_references.insert(entity_id.to_owned(), vec![]);
	}

	for (entity_id, entity) in entity.entities.iter() {
		if let Some(ent) = entity.parent.as_ref().and_then(Ref::as_local) {
			reverse_references.entry(ent).or_default().push(ReverseReference {
				from: entity_id.to_owned(),
				data: ReverseReferenceData::Parent
			});
		}

		for (property_name, property_data) in &entity.properties {
			visit_variant(&property_data.value, &mut |val| {
				if let Variant::Ref(val) = val
					&& let Some(ent) = val.as_ref().and_then(Ref::as_local)
				{
					reverse_references.entry(ent).or_default().push(ReverseReference {
						from: *entity_id,
						data: ReverseReferenceData::Property {
							property_name: property_name.to_owned()
						}
					});
				}
			});
		}

		for (platform, properties) in &entity.platform_specific_properties {
			for (property_name, property_data) in properties {
				visit_variant(&property_data.value, &mut |val| {
					if let Variant::Ref(val) = val
						&& let Some(ent) = val.as_ref().and_then(Ref::as_local)
					{
						reverse_references.entry(ent).or_default().push(ReverseReference {
							from: *entity_id,
							data: ReverseReferenceData::PlatformSpecificProperty {
								property_name: property_name.to_owned(),
								platform: platform.to_owned()
							}
						});
					}
				});
			}
		}

		for (event, triggers) in &entity.events {
			for (trigger, trigger_entities) in triggers {
				for reference in trigger_entities {
					let reference = &reference.entity_ref;

					if let Some(ent) = reference.as_local() {
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

		for (trigger, propagates) in &entity.input_copying {
			for (propagate, propagate_entities) in propagates {
				for reference in propagate_entities {
					reverse_references
						.entry(reference.entity_id)
						.or_default()
						.push(ReverseReference {
							from: entity_id.to_owned(),
							data: ReverseReferenceData::InputCopy {
								trigger: trigger.to_owned(),
								propagate: propagate.to_owned()
							}
						});
				}
			}
		}

		for (event, propagates) in &entity.output_copying {
			for (propagate, propagate_entities) in propagates {
				for reference in propagate_entities {
					reverse_references
						.entry(reference.entity_id)
						.or_default()
						.push(ReverseReference {
							from: entity_id.to_owned(),
							data: ReverseReferenceData::OutputCopy {
								event: event.to_owned(),
								propagate: propagate.to_owned()
							}
						});
				}
			}
		}

		for (aliased_name, aliases) in &entity.property_aliases {
			for alias_data in aliases {
				reverse_references
					.entry(alias_data.original_entity)
					.or_default()
					.push(ReverseReference {
						from: entity_id.to_owned(),
						data: ReverseReferenceData::PropertyAlias {
							aliased_name: aliased_name.to_owned(),
							original_property: alias_data.original_property.to_owned()
						}
					});
			}
		}

		for (exposed_name, exposed_entity) in &entity.exposed_entities {
			for reference in &exposed_entity.refers_to {
				if let Some(ent) = reference.as_local() {
					reverse_references.entry(ent).or_default().push(ReverseReference {
						from: entity_id.to_owned(),
						data: ReverseReferenceData::ExposedEntity {
							exposed_name: exposed_name.to_owned()
						}
					});
				}
			}
		}

		for (interface, referenced_entity) in &entity.exposed_interfaces {
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

		for (subset, member_of) in &entity.subsets {
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
	target: EntityID,
	reverse_references: &HashMap<EntityID, Vec<ReverseReference>>
) -> Result<Vec<EntityID>> {
	let child_ents = entity
		.entities
		.iter()
		.filter(|(_, x)| x.parent.as_ref().and_then(Ref::as_local).is_some_and(|x| x == target))
		.map(|(x, _)| x)
		.cloned()
		.collect_vec();

	let mut children = vec![target.to_owned()];

	for child in child_ents {
		children.extend(get_recursive_children(entity, child, reverse_references)?);
	}

	children
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CopiedEntityData {
	/// Which entity has been copied (and should be parented to the selection when pasting).
	pub root_entity: EntityID,

	pub data: IndexMap<EntityID, SubEntity>
}

pub fn random_entity_id() -> EntityID {
	let digits = [
		'0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f'
	];

	let mut id = String::from("cafe");

	for _ in 0..12 {
		id.push(*digits.choose(&mut rng()).expect("Slice is not empty"));
	}

	id.parse().unwrap()
}

/// Changes a Ref based on the given changelist (original entity ID -> new entity ID). Used for pasting.
pub fn alter_ref_according_to_changelist(reference: &Ref, changelist: &HashMap<EntityID, EntityID>) -> Ref {
	if let Some(local) = reference.as_local()
		&& let Some(changed) = changelist.get(&local)
	{
		reference.to_local(*changed)
	} else {
		reference.to_owned()
	}
}

#[try_fn]
#[context("Couldn't check whether local references refer to existing entities")]
pub fn check_local_references_exist(sub_entity: &SubEntity, entity: &Entity) -> Result<EditorValidity> {
	if let Some(ent) = sub_entity.parent.as_ref().and_then(Ref::as_local)
		&& !entity.entities.contains_key(&ent)
	{
		return Ok(EditorValidity::Invalid(format!("Invalid reference {}", ent)));
	}

	for property_data in sub_entity.properties.values() {
		let mut res = EditorValidity::Valid;
		visit_variant(&property_data.value, &mut |val| {
			if let Variant::Ref(val) = val
				&& let Some(ent) = val.as_ref().and_then(Ref::as_local)
				&& !entity.entities.contains_key(&ent)
			{
				res = EditorValidity::Invalid(format!("Invalid reference {}", ent));
			}
		});

		if matches!(res, EditorValidity::Invalid(_)) {
			return Ok(res);
		}
	}

	for properties in sub_entity.platform_specific_properties.values() {
		for property_data in properties.values() {
			let mut res = EditorValidity::Valid;
			visit_variant(&property_data.value, &mut |val| {
				if let Variant::Ref(val) = val
					&& let Some(ent) = val.as_ref().and_then(Ref::as_local)
					&& !entity.entities.contains_key(&ent)
				{
					res = EditorValidity::Invalid(format!("Invalid reference {}", ent));
				}
			});

			if matches!(res, EditorValidity::Invalid(_)) {
				return Ok(res);
			}
		}
	}

	for triggers in sub_entity.events.values() {
		for trigger_entities in triggers.values() {
			for reference in trigger_entities {
				let reference = &reference.entity_ref;

				if let Some(ent) = reference.as_local()
					&& !entity.entities.contains_key(&ent)
				{
					return Ok(EditorValidity::Invalid(format!("Invalid reference {}", ent)));
				}
			}
		}
	}

	for propagates in sub_entity
		.input_copying
		.values()
		.chain(sub_entity.output_copying.values())
	{
		for propagate_entities in propagates.values() {
			for reference in propagate_entities {
				if !entity.entities.contains_key(&reference.entity_id) {
					return Ok(EditorValidity::Invalid(format!(
						"Invalid reference {}",
						reference.entity_id
					)));
				}
			}
		}
	}

	for aliases in sub_entity.property_aliases.values() {
		for alias_data in aliases {
			if !entity.entities.contains_key(&alias_data.original_entity) {
				return Ok(EditorValidity::Invalid(format!(
					"Invalid reference {}",
					alias_data.original_entity
				)));
			}
		}
	}

	for exposed_entity in sub_entity.exposed_entities.values() {
		for reference in &exposed_entity.refers_to {
			if let Some(ent) = reference.as_local()
				&& !entity.entities.contains_key(&ent)
			{
				return Ok(EditorValidity::Invalid(format!("Invalid reference {}", ent)));
			}
		}
	}

	for referenced_entity in sub_entity.exposed_interfaces.values() {
		if !entity.entities.contains_key(referenced_entity) {
			return Ok(EditorValidity::Invalid(format!(
				"Invalid reference {}",
				referenced_entity
			)));
		}
	}

	for member_of in sub_entity.subsets.values() {
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
	cached_entities: &DashMap<RuntimeID, Entity>,
	game_version: GameVersion,
	entity: &Entity,
	reference: Option<&Ref>
) -> Option<(String, String)> {
	if let Some(ent) = reference.and_then(Ref::as_local) {
		Some((ent.to_string(), entity.entities.get(&ent)?.name.to_string()))
	} else if let Some(entity_ref) = reference
		&& let Some(external_scene) = entity_ref.external_scene
	{
		Some((entity_ref.entity_id.to_string(), {
			extract_entity(game_files, cached_entities, game_version, external_scene)
				.ok()?
				.entities
				.get(&entity_ref.entity_id)?
				.name
				.to_string()
		}))
	} else {
		None
	}
}

#[try_fn]
#[context("Couldn't get decoration for LINE {}", line)]
pub fn get_line_decoration(
	game_files: &PartitionManager,
	game_version: GameVersion,
	tonytools_hash_list: &tonytools::hashlist::HashList,
	line: RuntimeID
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
			if let Ok::<_, anyhow::Error>(x) = try_block! {
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
			.and_then(|x| x.as_str())
			.map(|x| x.to_owned())
	} else {
		locr.languages
			.get("en")
			.context("No en key in LOCR")?
			.get(&line_hash)
			.or_else(|| locr.languages.get("xx").and_then(|x| x.get(&line_hash)))
			.and_then(|x| x.as_str())
			.map(|x| x.to_owned())
	}
}

#[try_fn]
#[context("Couldn't get decorations for sub-entity {}", sub_entity.name)]
pub fn get_decorations(
	game_files: &PartitionManager,
	file_types: &HashMap<RuntimeID, ResourceType>,
	cached_entities: &DashMap<RuntimeID, Entity>,
	repository: &[RepositoryItem],
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
		entity,
		sub_entity.parent.as_ref()
	) {
		decorations.push(decoration);
	}

	// Hint decoration for unknown paths
	if sub_entity.factory.resource.get_path().is_none()
		&& let Some(entry) = sub_entity.factory.resource.get_info()
		&& let Some(hint) = entry.hint
	{
		decorations.push((sub_entity.factory.resource.to_string(), hint.into()));
	}

	if sub_entity.blueprint.get_path().is_none()
		&& let Some(entry) = sub_entity.blueprint.get_info()
		&& let Some(hint) = entry.hint
	{
		decorations.push((sub_entity.blueprint.to_string(), hint.into()));
	}

	for property_data in sub_entity.properties.values() {
		visit_variant(&property_data.value, &mut |val| match val {
			Variant::Ref(val) => {
				if let Some(decoration) =
					get_ref_decoration(game_files, cached_entities, game_version, entity, val.as_ref())
				{
					decorations.push(decoration);
				}
			}

			Variant::Resource(Some(reference)) => {
				let res = reference.resource;
				if file_types.get(&res).is_some_and(|x| x == "LINE") {
					if let Ok(Some(decoration)) =
						get_line_decoration(game_files, game_version, tonytools_hash_list, res)
					{
						decorations.push((res.to_string(), decoration));
					}
				} else if res.get_path().is_none()
					&& let Some(entry) = res.get_info()
					&& let Some(hint) = entry.hint
				{
					decorations.push((res.to_string(), hint.into()));
				}
			}

			Variant::Uuid(uuid) => {
				if let Some(repo_item) = repository.iter().find(|x| x.id == *uuid)
					&& let Some(name) = repo_item.data.get("Name").or(repo_item.data.get("CommonName"))
				{
					decorations.push((uuid.to_string(), name.as_str().unwrap_or("Non-string value").to_owned()));
				}
			}

			_ => {}
		});
	}

	for properties in sub_entity.platform_specific_properties.values() {
		for property_data in properties.values() {
			visit_variant(&property_data.value, &mut |val| match val {
				Variant::Ref(val) => {
					if let Some(decoration) =
						get_ref_decoration(game_files, cached_entities, game_version, entity, val.as_ref())
					{
						decorations.push(decoration);
					}
				}

				Variant::Resource(Some(reference)) => {
					let res = reference.resource;
					if file_types.get(&res).is_some_and(|x| x == "LINE") {
						if let Ok(Some(decoration)) =
							get_line_decoration(game_files, game_version, tonytools_hash_list, res)
						{
							decorations.push((res.to_string(), decoration));
						}
					} else if res.get_path().is_none()
						&& let Some(entry) = res.get_info()
						&& let Some(hint) = entry.hint
					{
						decorations.push((res.to_string(), hint.into()));
					}
				}

				Variant::Uuid(uuid) => {
					if let Some(repo_item) = repository.iter().find(|x| x.id == *uuid)
						&& let Some(name) = repo_item.data.get("Name").or(repo_item.data.get("CommonName"))
					{
						decorations.push((uuid.to_string(), name.as_str().unwrap_or("Non-string value").to_owned()));
					}
				}

				_ => {}
			});
		}
	}

	for triggers in sub_entity.events.values() {
		for trigger_entities in triggers.values() {
			for reference in trigger_entities {
				let reference = &reference.entity_ref;

				if let Some(decoration) =
					get_ref_decoration(game_files, cached_entities, game_version, entity, Some(reference))
				{
					decorations.push(decoration);
				}
			}
		}
	}

	for propagates in sub_entity
		.input_copying
		.values()
		.chain(sub_entity.output_copying.values())
	{
		for propagate_entities in propagates.values() {
			for reference in propagate_entities {
				let reference = reference.entity_id;

				if let Some(decoration) = get_ref_decoration(
					game_files,
					cached_entities,
					game_version,
					entity,
					Some(&Ref::local(reference))
				) {
					decorations.push(decoration);
				}
			}
		}
	}

	for aliases in sub_entity.property_aliases.values() {
		for alias_data in aliases {
			if let Some(decoration) = get_ref_decoration(
				game_files,
				cached_entities,
				game_version,
				entity,
				Some(&Ref::local(alias_data.original_entity))
			) {
				decorations.push(decoration);
			}
		}
	}

	for exposed_entity in sub_entity.exposed_entities.values() {
		for reference in &exposed_entity.refers_to {
			if let Some(decoration) =
				get_ref_decoration(game_files, cached_entities, game_version, entity, Some(reference))
			{
				decorations.push(decoration);
			}
		}
	}

	for referenced_entity in sub_entity.exposed_interfaces.values() {
		if let Some(decoration) = get_ref_decoration(
			game_files,
			cached_entities,
			game_version,
			entity,
			Some(&Ref::local(*referenced_entity))
		) {
			decorations.push(decoration);
		}
	}

	for member_of in sub_entity.subsets.values() {
		for parental_entity in member_of {
			if let Some(decoration) = get_ref_decoration(
				game_files,
				cached_entities,
				game_version,
				entity,
				Some(&Ref::local(parental_entity.to_owned()))
			) {
				decorations.push(decoration);
			}
		}
	}

	if sub_entity
		.factory
		.resource
		.get_info()
		.is_some_and(|entry| entry.resource_type == "MATT")
		&& let Some(mati) = extract_latest_metadata(game_files, sub_entity.factory.resource)?
			.core_info
			.references
			.into_iter()
			.find(|x| x.resource.get_info().is_some_and(|entry| entry.resource_type == "MATI"))
		&& let Some(mate) = extract_latest_metadata(game_files, mati.resource)?
			.core_info
			.references
			.into_iter()
			.find(|x| x.resource.get_info().is_some_and(|entry| entry.resource_type == "MATE"))
	{
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

/// Get the set of all entities which have reverse parent refs.
pub fn reverse_parent_refs_set(entity: &Entity) -> HashSet<EntityID> {
	let mut reverse_parent_refs = HashSet::new();

	for entity_data in entity.entities.values() {
		if let Some(parent) = entity_data.parent.as_ref().and_then(Ref::as_local) {
			reverse_parent_refs.insert(parent);
		}
	}

	reverse_parent_refs
}

/// New, modified, removed (ID, parent, name, factory, has reverse parent refs)
pub fn get_diff_info(
	original: &Entity,
	modified: &Entity
) -> (
	Vec<EntityID>,
	Vec<EntityID>,
	Vec<(EntityID, Option<Ref>, EcoString, RuntimeID, bool)>
) {
	let old_reverse_parent_refs = reverse_parent_refs_set(original);

	let removed = original
		.entities
		.par_iter()
		.filter(|&(id, _)| !modified.entities.contains_key(id))
		.map(|(id, orig)| {
			(
				id.to_owned(),
				orig.parent.to_owned(),
				orig.name.to_owned(),
				orig.factory.resource.to_owned(),
				old_reverse_parent_refs.contains(id)
			)
		})
		.collect();

	let mut diff = modified
		.entities
		.par_iter()
		.filter_map(|(id, modif)| {
			if let Some(orig) = original.entities.get(id) {
				if modif != orig { Some(("changed", id)) } else { None }
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
