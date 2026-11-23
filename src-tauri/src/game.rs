use std::{fs, path::Path, sync::Arc};

use anyhow::{Context, Result, anyhow, bail};
use dashmap::{DashMap, DashSet};
use glacier_commons::{
	game::{GamePlatform, GlacierGame, StorePlatform},
	game_detection::GameInstall,
	metadata::{ExtendedResourceMetadata, ResourceReference, ResourceType, RuntimeID},
	rid
};
use identity_hash::BuildIdentityHasher;
use itertools::Itertools;
use quickentity_rs::entity::Entity;
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use rpkg_rs::resource::{
	partition_manager::PartitionManager, resource_info::ResourceInfo, resource_partition::PatchId,
	runtime_resource_id::RuntimeResourceID
};
use simple_mod_framework_core::game::GameContext;
use tauri::{AppHandle, Manager};
use tryvial::try_fn;

use crate::{
	HashMap, Notification, NotificationKind, PapayaMap, finish_task,
	general::REPO_ID,
	intellisense::Intellisense,
	model::{AppState, ContentSearchRequest, Request, ResourceChangelogEntry, ResourceChangelogOperation, ToolRequest},
	ores_repo::RepositoryItem,
	send_notification, send_request, start_task
};

pub fn valid_game_path(path: impl AsRef<Path>) -> bool {
	let path = path.as_ref();
	path.join("HITMAN3.exe").is_file()
		|| path.join("HITMAN2.exe").is_file()
		|| path.join("HITMAN.exe").is_file()
		|| path.join("007FirstLight.exe").is_file()
}

#[try_fn]
pub fn custom_game_install(path: impl AsRef<Path>) -> Result<GameInstall> {
	let path = path.as_ref();

	let version = if path.join("HITMAN3.exe").is_file() {
		GlacierGame::H3
	} else if path.join("HITMAN2.exe").is_file() {
		GlacierGame::H2
	} else if path.join("HITMAN.exe").is_file() {
		GlacierGame::H1
	} else if path.join("007FirstLight.exe").is_file() {
		GlacierGame::FL
	} else {
		bail!("Unknown game version");
	};

	let platform = if path.join("steam_api64.dll").is_file() {
		StorePlatform::Steam
	} else if path.join("EOSSDK-Win64-Shipping.dll").is_file() {
		StorePlatform::Epic
	} else if path.join("../../..").join("MicrosoftGame.Config").is_file() {
		StorePlatform::Microsoft
	} else {
		StorePlatform::GOG
	};

	GameInstall {
		version,
		platform,
		path: path.to_owned()
	}
}

#[arc_trait::arc_trait]
#[dynosaur::dynosaur(pub DynGameFiles = dyn(box) GameFiles)]
pub trait GameFiles: Send + Sync {
	fn install(&self) -> &GameInstall;

	fn version(&self) -> GlacierGame;

	fn platform(&self) -> StorePlatform;

	fn resource_exists(&self, resource: RuntimeID) -> bool;

	fn resource_type(&self, resource: RuntimeID) -> Option<ResourceType>;

	fn resource_reverse_references(&self, resource: RuntimeID) -> Option<&Vec<RuntimeID>>;

	fn partition_manager(&self) -> &PartitionManager;

	fn smf_context(&self) -> Arc<GameContext>;

	fn all_resources(&self) -> impl Iterator<Item = RuntimeID> + Send + Sync;

	fn repository(&self) -> &Option<Vec<RepositoryItem>>;

	fn intellisense(&self) -> &Intellisense;

	fn to_rrid(&self, resource: RuntimeID) -> RuntimeResourceID {
		RuntimeResourceID::from(match self.version() {
			GlacierGame::FL => resource.as_u64() | ((GamePlatform::PC.tag().unwrap() as u64) << 56),
			_ => resource.as_u64()
		})
	}

	fn unlockables_id(&self) -> RuntimeID {
		match self.version() {
			GlacierGame::H1 | GlacierGame::H2 | GlacierGame::H3 => {
				rid!("[assembly:/_pro/online/default/offlineconfig/config.unlockables].pc_unlockables")
			}

			GlacierGame::FL => rid!("[assembly:/_knt/online/default/offlineconfig/config.unlockables].unlockables")
		}
	}

	/// Extract the latest copy of a resource.
	fn extract_latest_resource(&self, resource: RuntimeID) -> Result<(ExtendedResourceMetadata, Vec<u8>)>;

	fn extract_latest_metadata(&self, resource: RuntimeID) -> Result<ExtendedResourceMetadata>;

	fn extract_latest_overview_info(
		&self,
		resource: RuntimeID
	) -> Result<(ResourceType, u32, String, Vec<ResourceReference>)>;

	fn extract_entity(&self, factory_id: RuntimeID) -> Result<Arc<Entity>>;

	/// Get the history of the file, a changelog of events within the partitions. Will return an empty vector if the resource is not found in any partition.
	fn extract_resource_changelog(&self, resource: RuntimeID) -> Vec<ResourceChangelogEntry>;
}

impl GameFiles for Arc<DynGameFiles<'static>> {
	delegate::delegate! {
		to (**self) {
			fn install(&self) -> &GameInstall;

			fn version(&self) -> GlacierGame;

			fn platform(&self) -> StorePlatform;

			fn resource_exists(&self, resource: RuntimeID) -> bool;

			fn resource_type(&self, resource: RuntimeID) -> Option<ResourceType>;

			fn resource_reverse_references(&self, resource: RuntimeID) -> Option<&Vec<RuntimeID>>;

			fn partition_manager(&self) -> &PartitionManager;

			fn smf_context(&self) -> Arc<GameContext>;

			fn all_resources(&self) -> impl Iterator<Item = RuntimeID>;

			fn repository(&self) -> &Option<Vec<RepositoryItem>>;

			fn intellisense(&self) -> &Intellisense;

			/// Extract the latest copy of a resource.
			fn extract_latest_resource(
				&self,
				resource: RuntimeID
			) -> Result<(ExtendedResourceMetadata, Vec<u8>)>;

			fn extract_latest_metadata(&self, resource: RuntimeID) -> Result<ExtendedResourceMetadata>;

			fn extract_latest_overview_info(
				&self,
				resource: RuntimeID
			) -> Result<(ResourceType, u32, String, Vec<ResourceReference>)>;

			fn extract_entity(&self, factory_id: RuntimeID) -> Result<Arc<Entity>>;

			/// Get the history of the file, a changelog of events within the partitions. Will return an empty vector if the resource is not found in any partition.
			fn extract_resource_changelog(&self, resource: RuntimeID) -> Vec<ResourceChangelogEntry>;
		}
	}
}

pub struct NotificationDiagnostics {
	app: AppHandle
}

impl NotificationDiagnostics {
	pub fn new(app: &AppHandle) -> Self {
		Self { app: app.clone() }
	}
}

impl simple_mod_framework_core::world::Diagnostics for NotificationDiagnostics {
	#[try_fn]
	fn emit_diagnostic(&self, diagnostic: simple_mod_framework::diagnostics::Diagnostic) -> Result<(), eyre::Report> {
		send_notification(
			&self.app,
			Notification {
				kind: NotificationKind::Warning,
				title: match diagnostic.target {
					simple_mod_framework::diagnostics::DiagnosticTarget::Operation {
						mod_id,
						source,
						script_identifier
					} => format!(
						"{mod_id} - {source}{}",
						script_identifier.map_or_else(|| "".into(), |x| format!(" ({x})"))
					),
					simple_mod_framework::diagnostics::DiagnosticTarget::Mod { mod_id } => format!("{mod_id}"),
					simple_mod_framework::diagnostics::DiagnosticTarget::None => "Warning".into()
				},
				subtitle: diagnostic.kind.to_string()
			}
		)
		.map_err(|e| eyre::eyre!("{e:?}"))?;
	}
}

pub struct VanillaGame {
	install: GameInstall,
	game_files: Arc<GameContext>,
	intellisense: Intellisense,

	resource_reverse_references: HashMap<RuntimeID, Vec<RuntimeID>, BuildIdentityHasher<u64>>,
	resource_types: HashMap<RuntimeID, ResourceType, BuildIdentityHasher<u64>>,
	repository: Option<Vec<RepositoryItem>>,

	cached_entities: PapayaMap<RuntimeID, Arc<Entity>, BuildIdentityHasher<u64>>
}

impl VanillaGame {
	#[try_fn]
	pub fn load(app: &AppHandle, path: &Path) -> Result<Self> {
		let install = app
			.state::<AppState>()
			.game_installs
			.iter()
			.find(|x| x.path == path)
			.cloned()
			.map_or_else(|| custom_game_install(path), Ok)?;

		let task = start_task(app, "Loading game files")?;
		let game_files = GameContext::from_game(
			simple_mod_framework_types::Game {
				path: install.path.to_owned(),
				version: install.version,
				platform: match install.platform {
					StorePlatform::Epic => simple_mod_framework_types::Platform::Epic,
					StorePlatform::Steam => simple_mod_framework_types::Platform::Steam,
					StorePlatform::Microsoft => simple_mod_framework_types::Platform::Microsoft,
					StorePlatform::GOG => simple_mod_framework_types::Platform::GOG
				},
				hash: format!(
					"{:x}",
					md5::compute(if path.join("Runtime").join("chunk0.rpkg").exists() {
						fs::read(path.join("..").join("MicrosoftGame.Config")).context("Couldn't read game config")?
					} else {
						fs::read(path.join(match install.version {
							GlacierGame::H1 => "HITMAN.exe",
							GlacierGame::H2 => "HITMAN2.exe",
							GlacierGame::H3 => "HITMAN3.exe",
							GlacierGame::FL => "007FirstLight.exe"
						}))
						.context("Couldn't read game EXE")?
					})
				)
			},
			&NotificationDiagnostics::new(app),
			false
		)
		.map_err(|e| anyhow!("{e:?}"))?;
		finish_task(app, task)?;

		let partition_manager = &game_files.game_files;

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

		let task = start_task(app, "Caching reverse references")?;

		let resource_types: HashMap<RuntimeID, ResourceType, BuildIdentityHasher<u64>> = partition_manager
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
			DashMap::with_capacity_and_hasher(resource_types.len(), BuildIdentityHasher::default());

		// Ensure we only get the references from the lowest chunk version of each resource (matches the rest of GK's behaviour)
		let seen_resources: DashSet<RuntimeID, BuildIdentityHasher<u64>> =
			DashSet::with_capacity_and_hasher(resource_types.len(), BuildIdentityHasher::default());

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
			game_files: game_files.into(),
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
			resource_types,
			repository,
			cached_entities: Default::default()
		}
	}
}

impl GameFiles for VanillaGame {
	fn install(&self) -> &GameInstall {
		&self.install
	}

	fn version(&self) -> GlacierGame {
		self.install.version
	}

	fn platform(&self) -> StorePlatform {
		self.install.platform
	}

	fn resource_exists(&self, resource: RuntimeID) -> bool {
		self.resource_types.contains_key(&resource)
	}

	fn resource_type(&self, resource: RuntimeID) -> Option<ResourceType> {
		self.resource_types.get(&resource).copied()
	}

	fn resource_reverse_references(&self, resource: RuntimeID) -> Option<&Vec<RuntimeID>> {
		self.resource_reverse_references.get(&resource)
	}

	fn partition_manager(&self) -> &PartitionManager {
		&self.game_files.game_files
	}

	fn smf_context(&self) -> Arc<GameContext> {
		self.game_files.clone()
	}

	fn all_resources(&self) -> impl Iterator<Item = RuntimeID> {
		self.resource_types.keys().copied()
	}

	fn repository(&self) -> &Option<Vec<RepositoryItem>> {
		&self.repository
	}

	fn intellisense(&self) -> &Intellisense {
		&self.intellisense
	}

	fn unlockables_id(&self) -> RuntimeID {
		match self.version() {
			GlacierGame::H1 | GlacierGame::H2 | GlacierGame::H3 => {
				rid!("[assembly:/_pro/online/default/offlineconfig/config.unlockables].pc_unlockables")
			}

			GlacierGame::FL => rid!("[assembly:/_knt/online/default/offlineconfig/config.unlockables].unlockables")
		}
	}

	/// Extract the latest copy of a resource.
	fn extract_latest_resource(&self, resource: RuntimeID) -> Result<(ExtendedResourceMetadata, Vec<u8>)> {
		let rrid = self.to_rrid(resource);
		for partition in &self.partition_manager().partitions {
			if partition.contains(&rrid) {
				return Ok((
					partition
						.get_resource_info(&rrid)
						.with_context(|| format!("Couldn't extract metadata for {rrid}"))?
						.try_into()
						.with_context(|| format!("Metadata was invalid for {rrid}"))?,
					partition
						.read_resource(&rrid)
						.with_context(|| format!("Couldn't extract data of {rrid}"))?
				));
			}
		}

		bail!("Couldn't find {rrid} in any partition when extracting resource");
	}

	/// Get the metadata of the latest copy of a resource. Faster than fully extracting the resource.
	fn extract_latest_metadata(&self, resource: RuntimeID) -> Result<ExtendedResourceMetadata> {
		let rrid = self.to_rrid(resource);
		for partition in &self.partition_manager().partitions {
			if partition.contains(&rrid) {
				return partition
					.get_resource_info(&rrid)
					.with_context(|| format!("Couldn't extract metadata for {rrid}"))?
					.try_into()
					.with_context(|| format!("Metadata was invalid for {rrid}"));
			}
		}

		bail!("Couldn't find {rrid} in any partition when extracting metadata");
	}

	/// Get miscellaneous information (resource type, file size, chunk and patch, dependencies with hash and flag) for the latest copy of a resource.
	fn extract_latest_overview_info(
		&self,
		resource: RuntimeID
	) -> Result<(ResourceType, u32, String, Vec<ResourceReference>)> {
		let resource_id = self.to_rrid(resource);

		for partition in &self.partition_manager().partitions {
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
					info.size(),
					match &partition.partition_info().name {
						Some(name) => format!("{} ({})", name, package_name),
						None => package_name
					},
					info.references()
						.iter()
						.map(|(res_id, flags)| {
							Ok(ResourceReference {
								resource: res_id.try_into()?,
								flags: flags.into()
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
	fn extract_entity(&self, factory_id: RuntimeID) -> Result<Arc<Entity>> {
		{
			if let Some(x) = self.cached_entities.pin().get(&factory_id) {
				return Ok(x.clone());
			}
		}

		let (temp_meta, temp_data) = self
			.extract_latest_resource(factory_id)
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
		self.cached_entities.pin().insert(factory_id, result.clone());

		Ok(result)
	}

	/// Get the history of the file, a changelog of events within the partitions. Will return an empty vector if the resource is not found in any partition.
	fn extract_resource_changelog(&self, resource: RuntimeID) -> Vec<ResourceChangelogEntry> {
		let resource_id = self.to_rrid(resource);

		let mut events = vec![];

		for partition in &self.partition_manager().partitions {
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

pub struct EntitiesOverlay {
	pub game: Arc<DynGameFiles<'static>>,
	pub entities: HashMap<RuntimeID, Arc<Entity>>
}

impl GameFiles for EntitiesOverlay {
	delegate::delegate! {
		to self.game {
			fn install(&self) -> &GameInstall;

			fn version(&self) -> GlacierGame;

			fn platform(&self) -> StorePlatform;

			fn resource_exists(&self, resource: RuntimeID) -> bool;

			fn resource_type(&self, resource: RuntimeID) -> Option<ResourceType>;

			fn resource_reverse_references(&self, resource: RuntimeID) -> Option<&Vec<RuntimeID>>;

			fn partition_manager(&self) -> &PartitionManager;

			fn smf_context(&self) -> Arc<GameContext>;

			fn all_resources(&self) -> impl Iterator<Item = RuntimeID>;

			fn repository(&self) -> &Option<Vec<RepositoryItem>>;

			fn intellisense(&self) -> &Intellisense;

			/// Extract the latest copy of a resource.
			fn extract_latest_resource(
				&self,
				resource: RuntimeID
			) -> Result<(ExtendedResourceMetadata, Vec<u8>)>;

			fn extract_latest_metadata(&self, resource: RuntimeID) -> Result<ExtendedResourceMetadata>;

			fn extract_latest_overview_info(
				&self,
				resource: RuntimeID
			) -> Result<(ResourceType, u32, String, Vec<ResourceReference>)>;

			/// Get the history of the file, a changelog of events within the partitions. Will return an empty vector if the resource is not found in any partition.
			fn extract_resource_changelog(&self, resource: RuntimeID) -> Vec<ResourceChangelogEntry>;
		}
	}

	#[try_fn]
	fn extract_entity(&self, factory_id: RuntimeID) -> Result<Arc<Entity>> {
		if let Some(entity) = self.entities.get(&factory_id) {
			entity.clone()
		} else {
			self.game.extract_entity(factory_id)?
		}
	}
}

pub struct ProjectOverlay {
	pub game: Arc<DynGameFiles<'static>>,
	pub deploy: Arc<crate::project::SMFResult>
}

impl ProjectOverlay {
	fn deployed_state(&self, id: RuntimeID) -> Option<&simple_mod_framework::state::ResourceState> {
		let specifier = self.deploy.resource_specifiers.get(&id)?;
		self.deploy
			.resource_states
			.get(specifier)?
			.back()
			.map(|(_, state)| state)
	}

	fn deployed_meta(&self, id: RuntimeID) -> Option<ExtendedResourceMetadata> {
		let state = self.deployed_state(id)?;
		Some(ExtendedResourceMetadata {
			core_info: state.metadata.to_owned(),
			system_memory_requirement:
				glacier_commons::metadata::ResourceMetadata::calculate_system_memory_requirement(
					state.metadata.resource_type,
					&state.data
				)
				.unwrap_or(u32::MAX),
			video_memory_requirement: glacier_commons::metadata::ResourceMetadata::calculate_video_memory_requirement(
				state.metadata.resource_type,
				&state.data,
				self.version()
			)
			.unwrap_or(u32::MAX)
		})
	}
}

impl GameFiles for ProjectOverlay {
	delegate::delegate! {
		to self.game {
			fn install(&self) -> &GameInstall;

			fn version(&self) -> GlacierGame;

			fn platform(&self) -> StorePlatform;

			fn partition_manager(&self) -> &PartitionManager;

			fn smf_context(&self) -> Arc<GameContext>;

			fn intellisense(&self) -> &Intellisense;

			fn extract_resource_changelog(&self, resource: RuntimeID) -> Vec<ResourceChangelogEntry>;
		}
	}

	fn resource_exists(&self, resource: RuntimeID) -> bool {
		self.deploy.resource_specifiers.contains_key(&resource) || self.game.resource_exists(resource)
	}

	fn resource_type(&self, resource: RuntimeID) -> Option<ResourceType> {
		self.deployed_state(resource)
			.map(|x| x.metadata.resource_type)
			.or_else(|| self.game.resource_type(resource))
	}

	fn all_resources(&self) -> impl Iterator<Item = RuntimeID> + Send + Sync {
		let deployed = self.deploy.resource_specifiers.keys().copied().collect::<Vec<_>>();
		self.game.all_resources().chain(deployed).unique()
	}

	fn resource_reverse_references(&self, resource: RuntimeID) -> Option<&Vec<RuntimeID>> {
		self.deploy
			.reverse_references
			.get_or_init(|| {
				let mut merged: HashMap<RuntimeID, Vec<RuntimeID>, BuildIdentityHasher<u64>> = HashMap::default();

				for res in self.game.all_resources() {
					if let Some(refs) = self.game.resource_reverse_references(res) {
						merged.insert(res, refs.to_owned());
					}
				}

				for (id, specifier) in &self.deploy.resource_specifiers {
					let Some((_, state)) = self.deploy.resource_states.get(specifier).and_then(|x| x.back()) else {
						continue;
					};

					let old_refs = self
						.game
						.extract_latest_metadata(*id)
						.ok()
						.map(|m| {
							m.core_info
								.references
								.into_iter()
								.map(|r| r.resource)
								.collect::<Vec<_>>()
						})
						.unwrap_or_default();
					let new_refs = state.metadata.references.iter().map(|r| r.resource).collect::<Vec<_>>();

					for old_ref in &old_refs {
						if !new_refs.contains(old_ref)
							&& let Some(list) = merged.get_mut(old_ref)
						{
							list.retain(|x| x != id);
						}
					}

					for new_ref in &new_refs {
						if !old_refs.contains(new_ref) {
							let list = merged.entry(*new_ref).or_default();
							if !list.contains(id) {
								list.push(*id);
							}
						}
					}
				}

				merged.retain(|_, v| !v.is_empty());
				merged
			})
			.get(&resource)
	}

	fn repository(&self) -> &Option<Vec<RepositoryItem>> {
		self.deploy.repository.get_or_init(|| {
			if let Some(state) = self.deployed_state(crate::general::REPO_ID)
				&& let Ok(repo) = serde_json::from_slice(&state.data)
			{
				return Some(repo);
			}

			self.game.repository().to_owned()
		})
	}

	fn extract_latest_resource(&self, resource: RuntimeID) -> Result<(ExtendedResourceMetadata, Vec<u8>)> {
		if let Some(state) = self.deployed_state(resource) {
			let meta = self.deployed_meta(resource).context("Couldn't compute metadata")?;
			Ok((meta, state.data.to_owned()))
		} else {
			self.game.extract_latest_resource(resource)
		}
	}

	fn extract_latest_metadata(&self, resource: RuntimeID) -> Result<ExtendedResourceMetadata> {
		if let Some(meta) = self.deployed_meta(resource) {
			Ok(meta)
		} else {
			self.game.extract_latest_metadata(resource)
		}
	}

	fn extract_latest_overview_info(
		&self,
		resource: RuntimeID
	) -> Result<(ResourceType, u32, String, Vec<ResourceReference>)> {
		if let Some(state) = self.deployed_state(resource) {
			Ok((
				state.metadata.resource_type,
				state.data.len() as u32,
				self.deploy
					.resource_specifiers
					.get(&resource)
					.unwrap()
					.partition
					.0
					.to_owned()
					.into(),
				state.metadata.references.to_owned()
			))
		} else {
			self.game.extract_latest_overview_info(resource)
		}
	}

	#[try_fn]
	fn extract_entity(&self, factory_id: RuntimeID) -> Result<Arc<Entity>> {
		{
			if let Some(entity) = self.deploy.entity_cache.pin().get(&factory_id) {
				return Ok(entity.to_owned());
			}
		}

		if let Some(state) = self.deployed_state(factory_id) {
			if state.metadata.resource_type != "TEMP" {
				bail!("Given factory was not a TEMP");
			}

			let temp_meta = self
				.deployed_meta(factory_id)
				.context("Couldn't compute TEMP metadata")?;

			macro_rules! impl_game {
				($ty:ty) => {{
					let factory = glacier_bin1::deserialize::<$ty>(&state.data)?;

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

			let entity: Arc<Entity> = match self.version() {
				GlacierGame::H1 => impl_game!(glacier_bin1::game::h1::STemplateEntity),
				GlacierGame::H2 => impl_game!(glacier_bin1::game::h2::STemplateEntityFactory),
				GlacierGame::H3 => impl_game!(glacier_bin1::game::h3::STemplateEntityFactory),
				GlacierGame::FL => impl_game!(glacier_bin1::game::fl::STemplateEntityFactory)
			}
			.into();

			self.deploy.entity_cache.pin().insert(factory_id, entity.clone());
			entity
		} else {
			self.game.extract_entity(factory_id)?
		}
	}
}
