use anyhow::{anyhow, bail, Context, Result};
use dashmap::{mapref::one::Ref, DashMap};
use fn_error_context::context;
use hitman_commons::{
	game::GameVersion,
	hash_list::HashList,
	metadata::{ExtendedResourceMetadata, ResourceType, RuntimeID},
	rpkg_tool::RpkgResourceMeta
};
use itertools::Itertools;
use quickentity_rs::{convert_to_qn, qn_structs::Entity};
use rpkg_rs::resource::{
	partition_manager::PartitionManager, resource_info::ResourceInfo, resource_package::ResourceReferenceFlags,
	resource_partition::PatchId, runtime_resource_id::RuntimeResourceID
};
use tryvial::try_fn;

use crate::{
	model::{ResourceChangelogEntry, ResourceChangelogOperation},
	resourcelib::{
		h2016_convert_binary_to_blueprint, h2016_convert_binary_to_factory, h2_convert_binary_to_blueprint,
		h2_convert_binary_to_factory, h3_convert_binary_to_blueprint, h3_convert_binary_to_factory
	}
};

/// Extract the latest copy of a resource.
#[context("Couldn't extract resource {}", resource)]
pub fn extract_latest_resource(
	game_files: &PartitionManager,
	resource: RuntimeID
) -> Result<(ExtendedResourceMetadata, Vec<u8>)> {
	let resource_id = RuntimeResourceID::from(resource);

	for partition in &game_files.partitions {
		if let Some((info, _)) = partition
			.latest_resources()
			.into_iter()
			.find(|(x, _)| *x.rrid() == resource_id)
		{
			return Ok((
				info.try_into()?,
				partition
					.read_resource(&resource_id)
					.context("Couldn't extract resource using rpkg-rs")?
			));
		}
	}

	bail!("Couldn't find the resource in any partition");
}

/// Get the metadata of the latest copy of a resource. Faster than fully extracting the resource.
#[context("Couldn't extract metadata for resource {}", resource)]
pub fn extract_latest_metadata(game_files: &PartitionManager, resource: RuntimeID) -> Result<ExtendedResourceMetadata> {
	let resource_id = RuntimeResourceID::from(resource);

	for partition in &game_files.partitions {
		if let Some((info, _)) = partition
			.latest_resources()
			.into_iter()
			.find(|(x, _)| *x.rrid() == resource_id)
		{
			return Ok(info.try_into()?);
		}
	}

	bail!("Couldn't find the resource in any partition");
}

/// Get miscellaneous information (filetype, chunk and patch, dependencies with hash and flag) for the latest copy of a resource.
#[context("Couldn't extract overview info for resource {}", resource)]
pub fn extract_latest_overview_info(
	game_files: &PartitionManager,
	resource: RuntimeID
) -> Result<(ResourceType, String, Vec<(RuntimeID, String)>)> {
	let resource_id = RuntimeResourceID::from(resource);

	for partition in &game_files.partitions {
		if let Some((info, patchlevel)) = partition
			.latest_resources()
			.into_iter()
			.find(|(x, _)| *x.rrid() == resource_id)
		{
			let package_name = match patchlevel {
				PatchId::Base => partition.partition_info().id().to_string(),
				PatchId::Patch(level) => format!("{}patch{}", partition.partition_info().id(), level)
			};

			return Ok((
				info.data_type().try_into()?,
				match partition.partition_info().name() {
					Some(name) => format!("{} ({})", name, package_name),
					None => package_name
				},
				info.references()
					.iter()
					.map(|(res_id, flag)| {
						Ok((
							(*res_id).try_into()?,
							format!(
								"{:02X}",
								match flag {
									ResourceReferenceFlags::Legacy(x) => x,
									ResourceReferenceFlags::Standard(x) => x
								}
							)
						))
					})
					.collect::<Result<_>>()?
			));
		}
	}

	bail!("Couldn't find the resource in any RPKG");
}

/// Extract an entity by its factory and put it in the cache. Returns early if the entity is already cached.
#[try_fn]
#[context("Couldn't extract and cache entity {}", factory_id)]
pub fn extract_entity<'a>(
	resource_packages: &PartitionManager,
	cached_entities: &'a DashMap<RuntimeID, Entity>,
	game_version: GameVersion,
	hash_list: &HashList,
	factory_id: RuntimeID
) -> Result<Ref<'a, RuntimeID, Entity>> {
	{
		if let Some(x) = cached_entities.get(&factory_id) {
			return Ok(x);
		}
	}

	let (temp_meta, temp_data) =
		extract_latest_resource(resource_packages, factory_id).context("Couldn't extract TEMP")?;

	if temp_meta.core_info.resource_type != "TEMP" {
		bail!("Given factory was not a TEMP");
	}

	let factory =
		match game_version {
			GameVersion::H1 => h2016_convert_binary_to_factory(&temp_data)
				.context("Couldn't convert binary data to ResourceLib factory")?
				.into_modern(),

			GameVersion::H2 => h2_convert_binary_to_factory(&temp_data)
				.context("Couldn't convert binary data to ResourceLib factory")?,

			GameVersion::H3 => h3_convert_binary_to_factory(&temp_data)
				.context("Couldn't convert binary data to ResourceLib factory")?
		};

	let blueprint_id = temp_meta
		.core_info
		.references
		.get(factory.blueprint_index_in_resource_header as usize)
		.context("Blueprint referenced in factory does not exist in dependencies")?
		.resource;

	let (tblu_meta, tblu_data) =
		extract_latest_resource(resource_packages, blueprint_id).context("Couldn't extract TBLU")?;

	let blueprint = match game_version {
		GameVersion::H1 => h2016_convert_binary_to_blueprint(&tblu_data)
			.context("Couldn't convert binary data to ResourceLib blueprint")?
			.into_modern(),

		GameVersion::H2 => h2_convert_binary_to_blueprint(&tblu_data)
			.context("Couldn't convert binary data to ResourceLib blueprint")?,

		GameVersion::H3 => h3_convert_binary_to_blueprint(&tblu_data)
			.context("Couldn't convert binary data to ResourceLib blueprint")?
	};

	let entity = convert_to_qn(
		&factory,
		&RpkgResourceMeta::from_resource_metadata(temp_meta, false).with_hash_list(&hash_list.entries)?,
		&blueprint,
		&RpkgResourceMeta::from_resource_metadata(tblu_meta, false).with_hash_list(&hash_list.entries)?,
		false
	)
	.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

	cached_entities.insert(factory_id, entity.to_owned());

	cached_entities.get(&factory_id).expect("We just added it")
}

/// Get the history of the file, a changelog of events within the partitions.
#[context("Couldn't extract changelog for resource {}", resource)]
pub fn extract_resource_changelog(
	game_files: &PartitionManager,
	resource: RuntimeID
) -> Result<Vec<ResourceChangelogEntry>> {
	let resource_id = RuntimeResourceID::from(resource);

	let mut events = vec![];

	for partition in &game_files.partitions {
		let mut last_occurence: Option<&ResourceInfo> = None;

		let changes = partition.resource_patch_indices(&resource_id);
		let deletions = partition.resource_removal_indices(&resource_id);

		let occurrences = changes
			.clone()
			.into_iter()
			.chain(deletions.clone().into_iter())
			.collect::<Vec<PatchId>>();

		for occurence in occurrences.iter().sorted() {
			let partition_name = match partition.partition_info().name() {
				Some(name) => format!("{} ({})", name, partition.partition_info().id()),
				None => partition.partition_info().id().to_string()
			};

			let op_desc = match occurence {
				x if deletions.contains(x) => Some((
					ResourceChangelogOperation::Delete,
					"Removed resource from partition".into()
				)),

				x if changes.contains(x) => match partition.resource_info_from(&resource_id, *x) {
					Ok(info) => {
						let op_desc = match last_occurence {
							Some(last_info) => match info.size() as isize - last_info.size() as isize {
								0 => (ResourceChangelogOperation::Edit, "Updated resource".into()),
								size_diff => (
									ResourceChangelogOperation::Edit,
									format!("Updated resource: {:>+0.2} kB", size_diff as f32 / 1024.0)
								)
							},
							None => (ResourceChangelogOperation::Init, "Added resource to partition".into())
						};
						last_occurence = Some(info);
						Some(op_desc)
					}
					Err(_) => None
				},

				_ => None
			};

			if let Some((operation, description)) = op_desc {
				events.push((operation, partition_name, *occurence, description));
			}
		}
	}

	Ok(events
		.into_iter()
		.sorted_by(|(op1, _, patch1, _), (op2, _, patch2, _)| patch1.cmp(patch2).then(op1.cmp(op2)))
		.map(|(operation, partition, patch, description)| ResourceChangelogEntry {
			operation,
			partition,
			patch: match patch {
				PatchId::Base => "Base".into(),
				PatchId::Patch(n) => {
					format!("Patch {}", n)
				}
			},
			description
		})
		.collect::<Vec<_>>())
}
