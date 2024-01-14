use std::{
	collections::HashMap,
	fs::File,
	io::{Cursor, Read, Seek, SeekFrom},
	path::{Path, PathBuf}
};

use anyhow::{anyhow, bail, Context, Result};
use binrw::BinReaderExt;
use fn_error_context::context;
use indexmap::IndexMap;
use itertools::Itertools;
use lz4::block::decompress_to_buffer;
use memmap2::Mmap;
use quickentity_rs::{
	convert_2016_blueprint_to_modern, convert_2016_factory_to_modern, convert_to_qn,
	qn_structs::Entity,
	rpkg_structs::{ResourceDependency, ResourceMeta}
};
use rpkg_rs::{
	encryption::md5_engine::Md5Engine,
	misc::ini_file::IniFile,
	runtime::resource::{
		package_manager::PackageManager, resource_container::ResourceContainer, resource_package::ResourcePackage,
		runtime_resource_id::RuntimeResourceID
	}
};
use tryvial::try_fn;
use velcro::vec;

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
	resource_packages: &IndexMap<PathBuf, ResourcePackage>,
	hash_list: &HashList,
	resource: &str
) -> Result<(ResourceMeta, Vec<u8>)> {
	let resource = if resource.starts_with('0') {
		resource.to_owned()
	} else {
		format!("{:0>16X}", Md5Engine::compute(&resource.to_lowercase()))
	};

	let resource_id = RuntimeResourceID {
		id: u64::from_str_radix(&resource, 16)?
	};

	let hash_list_mapping = hash_list
		.entries
		.iter()
		.map(|x| {
			(
				x.hash.to_owned(),
				if x.path.is_empty() {
					None
				} else {
					Some(x.path.to_owned())
				}
			)
		})
		.collect::<HashMap<_, _>>();

	for (rpkg_path, rpkg) in resource_packages {
		if let Some((resource_header, offset_info)) = rpkg
			.resource_entries
			.iter()
			.enumerate()
			.find(|(_, entry)| entry.runtime_resource_id == resource_id)
			.map(|(index, entry)| (rpkg.resource_metadata.get(index).unwrap(), entry))
		{
			let final_size = offset_info.compressed_size_and_is_scrambled_flag & 0x3FFFFFFF;
			let is_lz4ed = final_size != resource_header.data_size;
			let is_scrambled = offset_info.compressed_size_and_is_scrambled_flag & 0x80000000 == 0x80000000;

			let mut package_file = File::open(rpkg_path)?;
			package_file.seek(SeekFrom::Start(offset_info.data_offset))?;

			let mut extracted = vec![0; final_size as usize];
			package_file.read_exact(&mut extracted)?;

			// Unscramble the data
			if is_scrambled {
				let xor_key = vec![0xdc, 0x45, 0xa6, 0x9c, 0xd3, 0x72, 0x4c, 0xab];

				extracted = extracted
					.iter()
					.enumerate()
					.map(|(index, byte)| byte ^ xor_key[index % xor_key.len()])
					.collect();
			}

			let rpkg_style_meta = ResourceMeta {
				hash_offset: offset_info.data_offset,
				hash_size: offset_info.compressed_size_and_is_scrambled_flag,
				hash_size_final: resource_header.data_size,
				hash_value: offset_info.runtime_resource_id.to_hex_string(),
				hash_size_in_memory: resource_header.system_memory_requirement,
				hash_size_in_video_memory: resource_header.video_memory_requirement,
				hash_resource_type: resource_header.m_type.iter().rev().map(|x| char::from(*x)).join(""),
				hash_reference_data: resource_header
					.m_references
					.as_ref()
					.map(|refs| {
						refs.reference_flags
							.iter()
							.zip(refs.reference_hash.iter())
							.map(|(flag, hash)| ResourceDependency {
								flag: format!("{:02X}", flag.to_owned().into_bytes()[0]),
								hash: hash_list_mapping
									.get(&hash.to_hex_string())
									.map(|x| x.as_ref().map(|x| x.to_owned()).unwrap_or(hash.to_hex_string()))
									.unwrap_or(hash.to_hex_string())
							})
							.collect()
					})
					.unwrap_or(vec![]),
				hash_reference_table_size: resource_header.references_chunk_size,
				hash_reference_table_dummy: resource_header.states_chunk_size
			};

			// Decompress the data
			if is_lz4ed {
				let mut decompressed = vec![0; resource_header.data_size as usize];

				let size = decompress_to_buffer(&extracted, Some(resource_header.data_size as i32), &mut decompressed)
					.context("Couldn't decompress data")?;

				if size == resource_header.data_size as usize {
					return Ok((rpkg_style_meta, decompressed));
				} else {
					bail!("Decompressed size didn't match defined size");
				}
			} else {
				return Ok((rpkg_style_meta, extracted));
			}
		}
	}

	bail!("Couldn't find the resource in any RPKG");
}

/// Extract an entity by its factory hash
#[try_fn]
#[context("Couldn't extract entity {}", factory_hash)]
pub fn extract_entity(
	resource_packages: &IndexMap<PathBuf, ResourcePackage>,
	game_version: GameVersion,
	hash_list: &HashList,
	factory_hash: &str
) -> Result<Entity> {
	let (temp_meta, temp_data) = extract_latest_resource(resource_packages, hash_list, factory_hash)?;

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

	let (tblu_meta, tblu_data) = extract_latest_resource(resource_packages, hash_list, blueprint_hash)?;

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

	convert_to_qn(&factory, &temp_meta, &blueprint, &tblu_meta, true)
		.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?
}
