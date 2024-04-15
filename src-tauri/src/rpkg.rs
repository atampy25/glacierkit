use std::collections::HashMap;

use anyhow::{anyhow, bail, Context, Result};
use fn_error_context::context;
use parking_lot::RwLock;
use quickentity_rs::{
	convert_2016_blueprint_to_modern, convert_2016_factory_to_modern, convert_to_qn,
	qn_structs::Entity,
	rpkg_structs::{ResourceDependency, ResourceMeta}
};
use rpkg_rs::runtime::resource::{
	partition_manager::PartitionManager, resource_partition::PatchId, runtime_resource_id::RuntimeResourceID
};
use tryvial::try_fn;

use crate::{
	game_detection::GameVersion,
	hash_list::HashList,
	resourcelib::{
		h2016_convert_binary_to_blueprint, h2016_convert_binary_to_factory, h2_convert_binary_to_blueprint,
		h2_convert_binary_to_factory, h3_convert_binary_to_blueprint, h3_convert_binary_to_factory
	}
};

/// Extract the latest copy of a resource by its hash or path.
#[context("Couldn't extract resource {}", resource)]
pub fn extract_latest_resource(
	game_files: &PartitionManager,
	hash_list: &HashList,
	resource: &str
) -> Result<(ResourceMeta, Vec<u8>)> {
	let resource = normalise_to_hash(resource.into());

	let resource_id = RuntimeResourceID::from_hex_string(&resource)?;

	for partition in game_files.get_all_partitions().into_iter().rev() {
		if let Some((info, _)) = partition
			.get_latest_resources()
			.into_iter()
			.find(|(x, _)| *x.get_rrid() == resource_id)
		{
			let rpkg_style_meta = ResourceMeta {
				hash_offset: info.get_data_offset(),
				hash_size: (info.get_compressed_size() | (if info.get_is_scrambled() { 0x80000000 } else { 0x0 }))
					as u32,
				hash_size_final: info.get_size(),
				hash_value: resource_id.to_hex_string(),
				hash_size_in_memory: info.get_system_memory_requirement(),
				hash_size_in_video_memory: info.get_video_memory_requirement(),
				hash_resource_type: info.get_type(),
				hash_reference_data: info
					.get_all_references()
					.iter()
					.map(|(hash, flag)| ResourceDependency {
						flag: format!("{:02X}", flag.to_owned().into_bytes()[0]),
						hash: hash_list
							.entries
							.get(&hash.to_hex_string())
							.map(|entry| {
								entry
									.path
									.as_ref()
									.map(|x| x.to_owned())
									.unwrap_or(hash.to_hex_string())
							})
							.unwrap_or(hash.to_hex_string())
					})
					.collect(),
				hash_reference_table_size: info.get_reference_chunk_size() as u32,
				hash_reference_table_dummy: info.get_states_chunk_size() as u32
			};

			return Ok((
				rpkg_style_meta,
				partition
					.get_resource(&resource_id)
					.context("Couldn't extract resource using rpkg-rs")?
			));
		}
	}

	bail!("Couldn't find the resource in any RPKG");
}

/// Get the metadata of the latest copy of a resource by its hash or path. Faster than fully extracting the resource.
#[context("Couldn't extract metadata for resource {}", resource)]
pub fn extract_latest_metadata(
	game_files: &PartitionManager,
	hash_list: &HashList,
	resource: &str
) -> Result<ResourceMeta> {
	let resource = normalise_to_hash(resource.into());

	let resource_id = RuntimeResourceID::from_hex_string(&resource)?;

	for partition in game_files.get_all_partitions().into_iter().rev() {
		if let Some((info, _)) = partition
			.get_latest_resources()
			.into_iter()
			.find(|(x, _)| *x.get_rrid() == resource_id)
		{
			let rpkg_style_meta = ResourceMeta {
				hash_offset: info.get_data_offset(),
				hash_size: (info.get_compressed_size() | (if info.get_is_scrambled() { 0x80000000 } else { 0x0 }))
					as u32,
				hash_size_final: info.get_size(),
				hash_value: resource_id.to_hex_string(),
				hash_size_in_memory: info.get_system_memory_requirement(),
				hash_size_in_video_memory: info.get_video_memory_requirement(),
				hash_resource_type: info.get_type(),
				hash_reference_data: info
					.get_all_references()
					.iter()
					.map(|(hash, flag)| ResourceDependency {
						flag: format!("{:02X}", flag.to_owned().into_bytes()[0]),
						hash: hash_list
							.entries
							.get(&hash.to_hex_string())
							.map(|entry| {
								entry
									.path
									.as_ref()
									.map(|x| x.to_owned())
									.unwrap_or(hash.to_hex_string())
							})
							.unwrap_or(hash.to_hex_string())
					})
					.collect(),
				hash_reference_table_size: info.get_reference_chunk_size() as u32,
				hash_reference_table_dummy: info.get_states_chunk_size() as u32
			};

			return Ok(rpkg_style_meta);
		}
	}

	bail!("Couldn't find the resource in any RPKG");
}

/// Get miscellaneous information (filetype, chunk and patch, dependencies with hash and flag) for the latest copy of a resource by its hash.
#[context("Couldn't extract overview info for resource {}", hash)]
pub fn extract_latest_overview_info(
	game_files: &PartitionManager,
	hash: &str
) -> Result<(String, String, Vec<(String, String)>)> {
	let resource_id = RuntimeResourceID::from_hex_string(hash)?;

	for partition in game_files.get_all_partitions().into_iter().rev() {
		if let Some((info, patchlevel)) = partition
			.get_latest_resources()
			.into_iter()
			.find(|(x, _)| *x.get_rrid() == resource_id)
		{
			return Ok((
				info.get_type(),
				match patchlevel {
					PatchId::Base => partition.get_partition_info().id.to_string(),
					PatchId::Patch(level) => format!("{}patch{}", partition.get_partition_info().id, level)
				},
				info.get_all_references()
					.iter()
					.map(|(res_id, flag)| {
						(
							res_id.to_hex_string(),
							format!("{:02X}", flag.to_owned().into_bytes()[0])
						)
					})
					.collect()
			));
		}
	}

	bail!("Couldn't find the resource in any RPKG");
}

/// Extract an entity by its factory's hash (you must normalise paths yourself) and put it in the cache. Returns early if the entity is already cached.
#[try_fn]
#[context("Couldn't ensure caching of entity {}", factory_hash)]
pub fn ensure_entity_in_cache(
	resource_packages: &PartitionManager,
	cached_entities: &RwLock<HashMap<String, Entity>>,
	game_version: GameVersion,
	hash_list: &HashList,
	factory_hash: &str
) -> Result<()> {
	{
		if cached_entities.read().contains_key(factory_hash) {
			return Ok(());
		}
	}

	let (temp_meta, temp_data) =
		extract_latest_resource(resource_packages, hash_list, factory_hash).context("Couldn't extract TEMP")?;

	if temp_meta.hash_resource_type != "TEMP" {
		bail!("Given factory was not a TEMP");
	}

	let factory =
		match game_version {
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

	let (tblu_meta, tblu_data) =
		extract_latest_resource(resource_packages, hash_list, blueprint_hash).context("Couldn't extract TBLU")?;

	let blueprint = match game_version {
		GameVersion::H1 => convert_2016_blueprint_to_modern(
			&h2016_convert_binary_to_blueprint(&tblu_data)
				.context("Couldn't convert binary data to ResourceLib blueprint")?
		),

		GameVersion::H2 => h2_convert_binary_to_blueprint(&tblu_data)
			.context("Couldn't convert binary data to ResourceLib blueprint")?,

		GameVersion::H3 => h3_convert_binary_to_blueprint(&tblu_data)
			.context("Couldn't convert binary data to ResourceLib blueprint")?
	};

	let entity = convert_to_qn(&factory, &temp_meta, &blueprint, &tblu_meta, true)
		.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

	cached_entities
		.write()
		.insert(factory_hash.to_owned(), entity.to_owned());
}

pub fn normalise_to_hash(hash_or_path: String) -> String {
	if hash_or_path.starts_with('0') {
		hash_or_path
	} else {
		format!("{:0>16X}", {
			let digest = md5::compute(hash_or_path.to_lowercase());
			let mut hash = 0u64;
			for i in 1..8 {
				hash |= u64::from(digest[i]) << (8 * (7 - i));
			}
			hash
		})
	}
}
