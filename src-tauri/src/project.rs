use std::{collections::HashMap, fs, path::PathBuf, sync::Arc};

use anyhow::Result;
use arc_swap::ArcSwap;
use eyre::{OptionExt, WrapErr};
use futures::{StreamExt, stream::FuturesUnordered};
use indexmap::IndexMap;
use itertools::Itertools;
use parking_lot::RwLock;
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use relative_path::{PathExt, RelativePath, RelativePathBuf};
use serde::{Deserialize, Serialize};
use simple_mod_framework::{
	analysis::{analyse, get_effective_option_values},
	cache::Cache,
	state::{DeployContext, ResourceStates, State}
};
use simple_mod_framework_core::{
	game::{GameContext, NominalPartition, ResourceSpecifier},
	world::{Diagnostics, Mods, Output, Progress}
};
use specta::Type;
use tauri::{AppHandle, Manager};
use tryvial::try_fn;

use crate::{
	Notification, NotificationKind, finish_task,
	game::{GameFiles, NotificationDiagnostics},
	model::{AppSettings, Request, SettingsRequest, ToolRequest},
	send_notification, send_request, start_task
};

pub struct Project {
	pub path: PathBuf,
	pub settings: ArcSwap<ProjectSettings>,
	pub deploy: Deploy
}

#[derive(Type, Serialize, Deserialize, Clone, derive_more::Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSettings {
	#[debug(skip)]
	pub mod_options: HashMap<simple_mod_framework_types::ModOptionID, simple_mod_framework_types::ModOptionValue>
}

pub struct Deploy {
	sync: tokio::sync::RwLock<()>,
	pub result: RwLock<Option<SMFResult>>
}

impl Default for Deploy {
	fn default() -> Self {
		Self {
			sync: ().into(),
			result: None.into()
		}
	}
}

pub struct SMFResult {
	pub resource_states: HashMap<ResourceSpecifier, ResourceStates>
}

struct ProjectWorld {
	path: PathBuf,
	manifest: Arc<simple_mod_framework_types::Manifest>,

	diagnostics: NotificationDiagnostics
}

impl ProjectWorld {
	#[try_fn]
	pub fn new(app: &AppHandle, path: PathBuf) -> Result<Self, eyre::Report> {
		Self {
			manifest: Arc::new(
				serde_json::from_slice::<simple_mod_framework_types::Manifest>(
					&fs::read(path.join("manifest.json")).wrap_err("Couldn't read project manifest.json")?
				)
				.wrap_err("Project manifest is invalid")?
			),
			path,
			diagnostics: NotificationDiagnostics::new(app)
		}
	}
}

impl Diagnostics for ProjectWorld {
	fn emit_diagnostic(&self, diagnostic: simple_mod_framework::diagnostics::Diagnostic) -> Result<(), eyre::Report> {
		self.diagnostics.emit_diagnostic(diagnostic)
	}
}

impl Mods for ProjectWorld {
	#[try_fn]
	fn get_all_mods(&self) -> eyre::Result<Vec<simple_mod_framework_types::ModID>> {
		vec![self.manifest.id.to_owned()]
	}

	fn get_mod_manifest(
		&self,
		id: &simple_mod_framework_types::ModID
	) -> eyre::Result<Arc<simple_mod_framework_types::Manifest>> {
		if id == &self.manifest.id {
			Ok(self.manifest.to_owned())
		} else {
			Err(eyre::eyre!("No such mod: {}", id))
		}
	}

	fn get_mod_root(&self, id: &simple_mod_framework_types::ModID) -> Option<PathBuf> {
		if id == &self.manifest.id {
			Some(self.path.to_owned())
		} else {
			None
		}
	}

	#[try_fn]
	fn read_mod_file(&self, id: &simple_mod_framework_types::ModID, path: &RelativePath) -> eyre::Result<Vec<u8>> {
		let mod_path = self.get_mod_root(id).ok_or_eyre("No such mod")?;
		fs::read(path.to_logical_path(mod_path))?
	}

	fn read_mod_content_folder(
		&self,
		id: &simple_mod_framework_types::ModID,
		content_folder: &RelativePath
	) -> eyre::Result<Vec<(simple_mod_framework_core::game::NominalPartition, Vec<RelativePathBuf>)>> {
		let mod_path = self.get_mod_root(id).ok_or_eyre("No such mod")?;

		let mut partition_folders = content_folder
			.to_logical_path(&mod_path)
			.read_dir()
			.wrap_err("Couldn't read content folder entries")?
			.collect::<Result<Vec<_>, _>>()?;

		partition_folders.sort_by_key(|x| x.file_name());

		partition_folders
			.into_par_iter()
			.map(|partition_folder| {
				if partition_folder.file_type()?.is_dir() {
					let partition_folder_name = partition_folder.file_name();
					let partition_folder_name = partition_folder_name.to_str().expect("Non UTF-8 file name");

					Ok((
						NominalPartition(partition_folder_name.into()),
						walkdir::WalkDir::new(partition_folder.path())
							.sort_by_file_name()
							.into_iter()
							.map(|entry| {
								let entry = entry.wrap_err("Error reading partition folder")?;

								if entry.file_type().is_file() && entry.path().extension().is_some() {
									Ok(Some(entry.path().relative_to(&mod_path)?))
								} else {
									Ok(None)
								}
							})
							.flatten_ok()
							.collect::<eyre::Result<Vec<_>>>()?
					))
				} else {
					eyre::bail!("Content folders should contain partition folders, not files");
				}
			})
			.collect()
	}

	#[try_fn]
	fn read_mod_blob_folder(
		&self,
		id: &simple_mod_framework_types::ModID,
		blob_folder: &RelativePath
	) -> eyre::Result<Vec<(String, RelativePathBuf)>> {
		let mod_path = self.get_mod_root(id).ok_or_eyre("No such mod")?;
		let blob_folder = blob_folder.to_logical_path(&mod_path);

		walkdir::WalkDir::new(&blob_folder)
			.sort_by_file_name()
			.into_iter()
			.map(|entry| {
				let entry = entry.wrap_err("Error reading blob folder")?;

				if entry.file_type().is_file() && !entry.file_name().to_string_lossy().ends_with(".ini") {
					Ok(Some((
						entry.path().relative_to(&blob_folder)?.to_string().to_lowercase(),
						entry.path().relative_to(&mod_path)?
					)))
				} else {
					Ok(None)
				}
			})
			.collect::<eyre::Result<Vec<_>>>()?
			.into_iter()
			.flatten()
			.collect()
	}
}

impl Output for ProjectWorld {
	fn emit_package_definition(&self, _: String) -> eyre::Result<()> {
		Ok(())
	}

	fn emit_thumbs(&self, _: String) -> eyre::Result<()> {
		Ok(())
	}

	fn emit_rpkg(
		&self,
		_: (
			rpkg_rs::resource::pdefs::PartitionId,
			rpkg_rs::resource::resource_partition::PatchId
		),
		_: &[u8]
	) -> eyre::Result<()> {
		Ok(())
	}

	fn emit_sdk_mod(&self, _: String, _: PathBuf) -> eyre::Result<()> {
		Ok(())
	}
}

impl Progress for ProjectWorld {
	fn start_progress(&self, name: &str, max: u64) -> eyre::Result<u64> {
		log::debug!("SMF progress: starting {name} ({max})");
		Ok(0)
	}

	fn start_spinner(&self, prefix: &str, target: Option<&str>, name: &str) -> eyre::Result<u64> {
		log::debug!("SMF progress: {prefix} {} {name}", target.unwrap_or(""));
		Ok(0)
	}

	fn advance_progress(&self, _: u64, _: u64) -> eyre::Result<()> {
		Ok(())
	}

	fn finish_progress(&self, _: u64) -> eyre::Result<()> {
		Ok(())
	}
}

impl Project {
	#[try_fn]
	pub async fn deploy(
		&self,
		app: &AppHandle,
		localisation_hash_list: Arc<tonytools::hashlist::HashList>,
		game: Arc<GameContext>
	) -> Result<(), eyre::Report> {
		let _guard = self.deploy.sync.write().await;
		let world = Arc::new(ProjectWorld::new(app, self.path.to_owned())?);

		let mod_id = world.get_all_mods()?.remove(0);

		let config = Arc::new(simple_mod_framework_types::Config {
			deploy_order: world.get_all_mods()?,
			mod_options: [(
				mod_id.to_owned(),
				self.settings.load().mod_options.to_owned().into_iter().collect()
			)]
			.into_iter()
			.collect(),
			..Default::default()
		});

		let enabled_mod_versions: Arc<HashMap<simple_mod_framework_types::ModID, _, _>> = config
			.deploy_order
			.iter()
			.map(|mod_id| Ok((mod_id.to_owned(), world.get_mod_manifest(mod_id)?.version.0.to_owned())))
			.collect::<Result<HashMap<_, _, _>, eyre::Report>>()?
			.into();

		let mut effective_options = IndexMap::default();
		for option in &world.get_mod_manifest(&mod_id)?.options {
			get_effective_option_values(
				&world,
				(&*game).into(),
				&config,
				&enabled_mod_versions,
				config.mod_options.get(&mod_id).unwrap(),
				&mut effective_options,
				option
			)?;
		}

		self.settings.store(
			ProjectSettings {
				mod_options: effective_options.into_iter().collect()
			}
			.into()
		);

		send_request(
			&app,
			Request::Tool(ToolRequest::Settings(SettingsRequest::ChangeProjectSettings {
				settings: (**self.settings.load()).to_owned()
			}))
		)
		.map_err(|e| eyre::eyre!("{e:?}"))?;

		let cache = Cache::new(app.state::<AppSettings>().data_dir().join("smf-cache"), &game.hash)?;
		let (mut deploy_graph, enums, peacock_plugins, sdk_mods, resources_to_port) =
			analyse(&config, &world, &game, &cache)?;

		cache.gc(&deploy_graph);

		let cache = Arc::new(cache);

		let mut state = State {
			deployment: DeployContext::new(&config, &game)?,
			localisation_hash_list,
			game,
			config,
			world
		};

		state
			.prepare(
				&deploy_graph,
				enums,
				peacock_plugins,
				sdk_mods,
				resources_to_port,
				&cache
			)
			.await?;

		let state = Arc::new(state);

		// Evaluate graph
		let mut active_tasks = FuturesUnordered::new();

		let concurrency = Arc::new(tokio::sync::Semaphore::new(std::thread::available_parallelism()?.get()));

		loop {
			while let Some(ready_id) = deploy_graph.ready_mut().pop_front() {
				let ready_node = deploy_graph.take(&ready_id).unwrap();

				let cache = cache.clone();
				let state = state.clone();
				let concurrency = concurrency.clone();

				active_tasks.push(tokio::spawn(async move {
					let _permit = concurrency.acquire().await?;

					let attribution = ready_node.attribution.to_owned();
					let warn_on_identical = ready_node.operation.warn_on_identical();

					let mutations = ready_node.evaluate(&cache, &state, &ready_id).await.wrap_err_with(|| {
						format!(
							"Couldn't apply graph operation {} - {}",
							attribution.mod_id, attribution.source
						)
					})?;

					for mutation in mutations {
						state.apply(&attribution, mutation, warn_on_identical).await?;
					}

					eyre::Ok(ready_id)
				}));
			}

			if active_tasks.is_empty() && deploy_graph.nodes().is_empty() {
				break;
			}

			deploy_graph.complete(&active_tasks.next().await.unwrap()??);
		}

		let cache = Arc::into_inner(cache).unwrap();

		cache.persist()?;

		let mut resource_states = HashMap::default();

		for (id, states) in &state.deployment.resource_states {
			let mut states = std::mem::take(&mut *states.lock().await);
			states.shrink_to_fit();
			resource_states.insert(id.to_owned(), states);
		}

		log::debug!("SMF deployment complete with {} resources", resource_states.len());

		self.deploy.result.write().replace(SMFResult { resource_states });
	}

	pub fn background_deploy(self: &Arc<Self>, app: &AppHandle) {
		if app.state::<AppSettings>().settings().smf_integration && self.path.join("manifest.json").exists() {
			let app_state = app.state::<crate::model::AppState>();
			if let Some(localisation_hash_list) = app_state.tonytools_hash_list.load_full()
				&& let Some(game) = app_state.game.load().as_deref()
			{
				tauri::async_runtime::spawn({
					let project = self.clone();
					let app = app.clone();
					let game = game.smf_context();
					async move {
						let task = start_task(&app, "Deploying mod").unwrap();
						if let Err(e) = project.deploy(&app, localisation_hash_list, game).await {
							send_notification(
								&app,
								Notification {
									kind: NotificationKind::Error,
									title: "Mod failed to deploy".into(),
									subtitle: format!("{e:?}")
								}
							)
							.expect("Couldn't send data to frontend");
						}
						let _ = finish_task(&app, task);
					}
				});
			}
		}
	}
}
