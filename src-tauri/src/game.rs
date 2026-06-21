use std::{
	fs,
	path::Path,
	sync::{Arc, RwLock}
};

use anyhow::{Context, Result, anyhow, bail};
use arc_swap::ArcSwap;
use dashmap::{DashMap, DashSet};
use glacier_commons::{
	game::{GamePlatform, GlacierGame, StorePlatform},
	game_detection::GameInstall,
	metadata::{ExtendedResourceMetadata, ReferenceFlags, ResourceReference, ResourceType, RuntimeID}
};
use glacier_ini::IniFileSystem;
use identity_hash::BuildIdentityHasher;
use itertools::Itertools;
use quickentity_rs::entity::Entity;
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use rpkg_rs::resource::{
	partition_manager::PartitionManager,
	pdefs::{PackageDefinitionParser, PackageDefinitionSource, bond_parser::BondParser},
	resource_info::ResourceInfo,
	resource_partition::PatchId,
	runtime_resource_id::RuntimeResourceID
};
use tauri::{AppHandle, Manager};
use tryvial::try_fn;

use crate::{
	HashMap, PapayaMap, finish_task,
	general::REPO_ID,
	intellisense::Intellisense,
	model::{
		AppSettings, AppState, ContentSearchRequest, Request, ResourceChangelogEntry, ResourceChangelogOperation,
		ToolRequest
	},
	ores_repo::RepositoryItem,
	send_request, start_progress, start_task, task_progress
};

pub struct Game {
	install: GameInstall,
	game_files: PartitionManager,
	intellisense: Intellisense,

	resource_reverse_references: HashMap<RuntimeID, Vec<RuntimeID>, BuildIdentityHasher<u64>>,
	file_types: HashMap<RuntimeID, ResourceType, BuildIdentityHasher<u64>>,
	repository: Option<Vec<RepositoryItem>>,

	cached_entities: Arc<PapayaMap<RuntimeID, Arc<Entity>, BuildIdentityHasher<u64>>>
}

impl Game {
	#[try_fn]
	pub fn load(app: &AppHandle, path: &Path) -> Result<Self> {
		let task = start_task(app, "Loading game files")?;

		let install = app
			.state::<AppState>()
			.game_installs
			.iter()
			.find(|x| x.path == path)
			.context("No such game install")?
			.to_owned();

		let (proj_path, relative_runtime_path) = if install.version == GlacierGame::FL {
			(r"..\".into(), "runtime".into())
		} else {
			let thumbs = IniFileSystem::from_path(path.join("thumbs.dat")).context("Couldn't load thumbs.dat")?;

			let thumbs = thumbs
				.root()
				.sections()
				.get("application")
				.context("Couldn't get application section")?;

			let (Some(proj_path), Some(relative_runtime_path)) = (
				thumbs.options().get("PROJECT_PATH"),
				thumbs.options().get("RUNTIME_PATH")
			) else {
				bail!("thumbs.dat was missing required properties");
			};

			(proj_path.to_owned(), relative_runtime_path.to_owned())
		};

		// Workaround for the Linux filesystem.
		// The relative_runtime_path will in most cases be "runtime", while the folder is actually called "Runtime"
		// Windows doesn't care about the mismatched casing, UNIX does :(
		let relative_runtime_path_uppercased = relative_runtime_path
			.char_indices()
			.map(|(idx, ch)| if idx == 0 { ch.to_ascii_uppercase() } else { ch })
			.collect::<String>();

		let runtime_path = [relative_runtime_path, relative_runtime_path_uppercased]
			.iter()
			.flat_map(|folder| path.join(proj_path.replace('\\', "/")).join(folder).canonicalize())
			.find(|joined_path| joined_path.exists())
			.context("Couldn't find valid runtime folder")?;

		let mut partitions = match install.version {
			GlacierGame::H1 => PackageDefinitionSource::HM2016(fs::read(runtime_path.join("packagedefinition.txt"))?)
				.read()
				.context("Couldn't read packagedefinition")?,

			GlacierGame::H2 => PackageDefinitionSource::HM2(fs::read(runtime_path.join("packagedefinition.txt"))?)
				.read()
				.context("Couldn't read packagedefinition")?,

			GlacierGame::H3 => PackageDefinitionSource::HM3(fs::read(runtime_path.join("packagedefinition.txt"))?)
				.read()
				.context("Couldn't read packagedefinition")?,

			GlacierGame::FL => BondParser::parse(&fs::read(runtime_path.join("packagedefinition.txt"))?)
				.context("Couldn't read packagedefinition")?
		};

		if !app.state::<ArcSwap<AppSettings>>().load().extract_modded_files {
			for partition in &mut partitions {
				partition.set_max_patch_level(9);
			}
		}

		finish_task(app, task)?;

		let partition_names = partitions.iter().map(|x| x.id.to_string()).collect_vec();

		let last_index = RwLock::new(0);
		let task = RwLock::new(start_progress(app, format!("Loading {}", partition_names[0]))?);

		let mut partition_manager = PartitionManager::new(
			runtime_path.clone(),
			install.version.into(),
			&PackageDefinitionSource::Custom(partitions)
		)
		.context("Couldn't create partition manager")?;

		partition_manager
			.mount_partitions(|cur_partition, state| {
				if cur_partition < partition_names.len() {
					if cur_partition != *last_index.read().unwrap() {
						*last_index.write().unwrap() = cur_partition;

						finish_task(app, *task.read().unwrap()).expect("Couldn't send data to frontend");
						*task.write().unwrap() =
							start_progress(app, format!("Loading {}", partition_names[cur_partition]))
								.expect("Couldn't send data to frontend");
					}

					task_progress(app, *task.read().unwrap(), state.install_progress)
						.expect("Couldn't send data to frontend");
				}
			})
			.context("Couldn't mount partitions")?;

		send_request(
			app,
			Request::Tool(ToolRequest::ContentSearch(ContentSearchRequest::SetPartitions {
				partitions: partition_manager
					.partitions
					.iter()
					.map(|x| {
						(
							x.partition_info().name.as_deref().unwrap_or("<unnamed>").to_owned(),
							x.partition_info().id.to_string()
						)
					})
					.collect()
			}))
		)?;

		finish_task(app, *task.read().unwrap())?;
		let task = start_task(app, "Caching reverse references")?;

		let file_types: HashMap<RuntimeID, ResourceType, BuildIdentityHasher<u64>> = partition_manager
			.partitions
			.par_iter()
			.rev()
			.flat_map(|partition| {
				partition.latest_resources().into_par_iter().map(|(resource, _)| {
					(
						RuntimeID::try_from(*resource.rrid()).expect("Invalid ID in game files"),
						resource
							.data_type()
							.try_into()
							.expect("Invalid resource type in game files")
					)
				})
			})
			.collect();

		let resource_reverse_references: DashMap<RuntimeID, Vec<RuntimeID>, BuildIdentityHasher<u64>> =
			DashMap::with_capacity_and_hasher(file_types.len(), BuildIdentityHasher::default());

		// Ensure we only get the references from the lowest chunk version of each resource (matches the rest of GK's behaviour)
		let seen_resources: DashSet<RuntimeID, BuildIdentityHasher<u64>> =
			DashSet::with_capacity_and_hasher(file_types.len(), BuildIdentityHasher::default());

		partition_manager
			.partitions
			.par_iter()
			.flat_map(|partition| {
				partition.latest_resources().into_par_iter().map(|(resource, _)| {
					(
						RuntimeID::try_from(*resource.rrid()).expect("Invalid ID in game files"),
						resource.references()
					)
				})
			})
			.for_each(|(resource_id, resource_references)| {
				if seen_resources.insert(resource_id) {
					for (reference_id, _) in resource_references {
						resource_reverse_references
							.entry(RuntimeID::try_from(*reference_id).expect("Invalid ID in game files"))
							.or_default()
							.push(resource_id);
					}
				}
			});

		let resource_reverse_references = resource_reverse_references.into_par_iter().collect();

		finish_task(app, task)?;

		let task = start_task(app, "Caching repository")?;

		let repository = partition_manager
			.read_resource_from(partition_manager.root_partition()?, REPO_ID.as_u64().into())
			.ok()
			.map(|x| serde_json::from_slice(&x))
			.transpose()?;

		finish_task(app, task)?;

		Self {
			install,
			game_files: partition_manager,
			intellisense: Intellisense::new(
				fs::read(
					dirs::data_local_dir()
						.context("No local data dir")?
						.join("glacier-commons")
						.join("pins.json")
				)
				.as_deref()
				.unwrap_or(b"[]")
			),
			resource_reverse_references,
			file_types,
			repository,
			cached_entities: Arc::new(Default::default())
		}
	}

	pub fn version(&self) -> GlacierGame {
		self.install.version
	}

	pub fn platform(&self) -> StorePlatform {
		self.install.platform
	}

	pub fn resource_exists(&self, resource: impl Into<RuntimeID>) -> bool {
		self.file_types.contains_key(&resource.into())
	}

	pub fn resource_type(&self, resource: impl Into<RuntimeID>) -> Option<ResourceType> {
		self.file_types.get(&resource.into()).copied()
	}

	pub fn resource_reverse_references(&self, resource: impl Into<RuntimeID>) -> Option<&Vec<RuntimeID>> {
		self.resource_reverse_references.get(&resource.into())
	}

	pub fn partition_manager(&self) -> &PartitionManager {
		&self.game_files
	}

	pub fn all_resources(&self) -> impl Iterator<Item = RuntimeID> {
		self.file_types.keys().copied()
	}

	pub fn repository(&self) -> &Option<Vec<RepositoryItem>> {
		&self.repository
	}

	pub fn intellisense(&self) -> &Intellisense {
		&self.intellisense
	}

	pub fn to_rrid(&self, resource: impl Into<RuntimeID>) -> RuntimeResourceID {
		RuntimeResourceID::from(match self.version() {
			GlacierGame::FL => resource.into().as_u64() | ((GamePlatform::PC.tag().unwrap() as u64) << 56),
			_ => resource.into().as_u64()
		})
	}

	/// Extract the latest copy of a resource.
	pub fn extract_latest_resource(
		&self,
		resource: impl Into<RuntimeID>
	) -> Result<(ExtendedResourceMetadata, Vec<u8>)> {
		let rrid = self.to_rrid(resource);
		for partition in &self.game_files.partitions {
			if partition.contains(&rrid)
				&& let Some((info, _)) = partition
					.latest_resources()
					.into_iter()
					.find(|(x, _)| *x.rrid() == rrid)
			{
				return Ok((
					info.try_into()
						.with_context(|| format!("Couldn't extract resource {rrid}"))?,
					partition
						.read_resource(&rrid)
						.with_context(|| format!("Couldn't extract {rrid} using rpkg-rs"))?
				));
			}
		}

		bail!("Couldn't find {rrid} in any partition when extracting resource");
	}

	/// Get the metadata of the latest copy of a resource. Faster than fully extracting the resource.
	pub fn extract_latest_metadata(&self, resource: impl Into<RuntimeID>) -> Result<ExtendedResourceMetadata> {
		let resource_id = self.to_rrid(resource);

		for partition in &self.game_files.partitions {
			if partition.contains(&resource_id)
				&& let Some((info, _)) = partition
					.latest_resources()
					.into_iter()
					.find(|(x, _)| *x.rrid() == resource_id)
			{
				return info
					.try_into()
					.with_context(|| format!("Couldn't extract metadata for resource {resource_id}"));
			}
		}

		bail!("Couldn't find {resource_id} in any partition when extracting metadata");
	}

	/// Get miscellaneous information (filetype, chunk and patch, dependencies with hash and flag) for the latest copy of a resource.
	pub fn extract_latest_overview_info(
		&self,
		resource: impl Into<RuntimeID>
	) -> Result<(ResourceType, String, Vec<ResourceReference>)> {
		let resource_id = self.to_rrid(resource);

		for partition in &self.game_files.partitions {
			if partition.contains(&resource_id)
				&& let Some((info, patchlevel)) = partition
					.latest_resources()
					.into_iter()
					.find(|(x, _)| *x.rrid() == resource_id)
			{
				let package_name = match patchlevel {
					PatchId::Base => partition.partition_info().id.to_string(),
					PatchId::Patch(level) => format!("{}patch{}", partition.partition_info().id, level)
				};

				return Ok((
					info.data_type()
						.try_into()
						.with_context(|| format!("Couldn't extract overview info for resource {resource_id}"))?,
					match &partition.partition_info().name {
						Some(name) => format!("{} ({})", name, package_name),
						None => package_name
					},
					info.references()
						.iter()
						.map(|(res_id, flag)| {
							Ok(ResourceReference {
								resource: res_id.try_into()?,
								flags: ReferenceFlags::from_any(flag.as_byte())
							})
						})
						.collect::<Result<_>>()
						.with_context(|| format!("Couldn't extract overview info for resource {resource_id}"))?
				));
			}
		}

		bail!("Couldn't find {resource_id} in any RPKG when extracting overview info");
	}

	/// Extract an entity by its factory and put it in the cache. Returns early if the entity is already cached.
	pub fn extract_entity(&self, factory_id: impl Into<RuntimeID>) -> Result<Arc<Entity>> {
		let runtime_id = factory_id.into();

		{
			if let Some(x) = self.cached_entities.pin().get(&runtime_id) {
				return Ok(x.clone());
			}
		}

		let (temp_meta, temp_data) = self
			.extract_latest_resource(runtime_id)
			.context("Couldn't extract TEMP")?;

		if temp_meta.core_info.resource_type != "TEMP" {
			bail!("Given factory was not a TEMP");
		}

		macro_rules! impl_game {
			($ty:ty) => {{
				let factory = glacier_bin1::deserialize::<$ty>(&temp_data)?;

				let blueprint_hash = temp_meta
					.core_info
					.references
					.get(factory.blueprint_index_in_resource_header as usize)
					.context("Blueprint referenced in factory does not exist in dependencies")?
					.resource;

				let (tblu_meta, tblu_data) = self
					.extract_latest_resource(blueprint_hash)
					.context("Couldn't extract TBLU")?;

				let blueprint = glacier_bin1::deserialize(&tblu_data)?;

				Entity::from_game(
					&factory,
					&temp_meta.core_info,
					&blueprint,
					&tblu_meta.core_info,
					false
				)
				.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?
			}};
		}

		let entity = match self.install.version {
			GlacierGame::H1 => impl_game!(glacier_bin1::game::h1::STemplateEntity),
			GlacierGame::H2 => impl_game!(glacier_bin1::game::h2::STemplateEntityFactory),
			GlacierGame::H3 => impl_game!(glacier_bin1::game::h3::STemplateEntityFactory),
			GlacierGame::FL => impl_game!(glacier_bin1::game::fl::STemplateEntityFactory)
		};

		let result: Arc<Entity> = entity.into();
		self.cached_entities.pin().insert(runtime_id, result.clone());

		Ok(result)
	}

	/// Get the history of the file, a changelog of events within the partitions. Will return an empty vector if the resource is not found in any partition.
	pub fn extract_resource_changelog(&self, resource: impl Into<RuntimeID>) -> Vec<ResourceChangelogEntry> {
		let resource_id = self.to_rrid(resource);

		let mut events = vec![];

		for partition in &self.game_files.partitions {
			let mut last_occurence: Option<&ResourceInfo> = None;

			let changes = partition.resource_patch_indices(&resource_id);
			let deletions = partition.resource_removal_indices(&resource_id);

			let occurrences = changes
				.clone()
				.into_iter()
				.chain(deletions.clone())
				.collect::<Vec<PatchId>>();

			for occurence in occurrences.iter().sorted() {
				let partition_name = match &partition.partition_info().name {
					Some(name) => format!("{} ({})", name, partition.partition_info().id),
					None => partition.partition_info().id.to_string()
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

		events
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
			.collect::<Vec<_>>()
	}
}
