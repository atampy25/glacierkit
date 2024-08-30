use std::{
	fs,
	path::{Path, PathBuf}
};

use anyhow::{anyhow, bail, Context, Result};
use arc_swap::ArcSwap;
use dashmap::DashMap;
use fn_error_context::context;
use hashbrown::HashMap;
use hitman_commons::{game::GameVersion, hash_list::HashList, metadata::RuntimeID};
use hitman_formats::ores::parse_json_ores;
use indexmap::IndexMap;
use itertools::Itertools;
use quickentity_rs::{
	apply_patch,
	patch_structs::Patch,
	qn_structs::{CommentEntity, Entity}
};
use rayon::iter::{
	IndexedParallelIterator, IntoParallelIterator, IntoParallelRefIterator, ParallelExtend, ParallelIterator
};
use rpkg_rs::{
	misc::ini_file_system::IniFileSystem, resource::partition_manager::PartitionManager,
	resource::pdefs::PackageDefinitionSource
};
use serde_json::{from_slice, from_str, from_value, to_value, Value};
use tauri::{AppHandle, Manager};
use tryvial::try_fn;
use uuid::Uuid;
use velcro::vec;

use crate::ores_repo::RepositoryItem;
use crate::rpkg::extract_latest_resource;
use crate::{event_handling::resource_overview::initialise_resource_overview, get_loaded_game_version};
use crate::{
	finish_task, send_notification, send_request, start_task, Notification, NotificationKind, HASH_LIST_ENDPOINT,
	HASH_LIST_VERSION_ENDPOINT, TONYTOOLS_HASH_LIST_ENDPOINT, TONYTOOLS_HASH_LIST_VERSION_ENDPOINT
};
use crate::{intellisense::Intellisense, ores_repo::UnlockableItem};
use crate::{
	model::{
		AppSettings, AppState, ContentSearchRequest, EditorData, EditorState, EditorType, FileBrowserRequest,
		GameBrowserRequest, GlobalRequest, JsonPatchType, Request, TextFileType, ToolRequest
	},
	rpkg::extract_entity
};

#[try_fn]
#[context("Couldn't open file")]
pub async fn open_file(app: &AppHandle, path: impl AsRef<Path>) -> Result<()> {
	let app_state = app.state::<AppState>();
	let app_settings = app.state::<ArcSwap<AppSettings>>();

	let path = path.as_ref();

	let task = start_task(
		app,
		format!(
			"Opening {}",
			path.file_name().context("No file name")?.to_string_lossy()
		)
	)?;

	let existing = {
		app_state
			.editor_states
			.iter()
			.find(|x| x.file.as_ref().map(|x| x == path).unwrap_or(false))
			.map(|x| x.key().to_owned())
	};

	if let Some(existing) = existing {
		send_request(app, Request::Global(GlobalRequest::SelectTab(existing)))?;
	} else {
		let extension = path
			.file_name()
			.context("No file name")?
			.to_string_lossy()
			.split('.')
			.skip(1)
			.collect_vec()
			.join(".");

		match extension.as_ref() {
			"entity.json" => {
				let id = Uuid::new_v4();

				let mut entity: Entity =
					from_slice(&fs::read(path).context("Couldn't read file")?).context("Invalid entity")?;

				// Normalise comments to form used by GlacierKit (single comment for each entity)
				let mut comments: Vec<CommentEntity> = vec![];
				for comment in entity.comments {
					if let Some(x) = comments.iter_mut().find(|x| x.parent == comment.parent) {
						x.text = format!("{}\n\n{}", x.text, comment.text);
					} else {
						comments.push(CommentEntity {
							parent: comment.parent,
							name: "Notes".into(),
							text: comment.text
						});
					}
				}
				entity.comments = comments;

				app_state.editor_states.insert(
					id.to_owned(),
					EditorState {
						file: Some(path.to_owned()),
						data: EditorData::QNEntity {
							entity: Box::new(entity),
							settings: Default::default()
						}
					}
				);

				send_request(
					app,
					Request::Global(GlobalRequest::CreateTab {
						id,
						name: path.file_name().context("No file name")?.to_string_lossy().into(),
						editor_type: EditorType::QNEntity
					})
				)?;
			}

			"entity.patch.json" => {
				let id = Uuid::new_v4();

				if let Some(game_files) = app_state.game_files.load().as_ref()
					&& let Some(install) = app_settings.load().game_install.as_ref()
					&& let Some(hash_list) = app_state.hash_list.load().as_ref()
				{
					let patch: Patch =
						from_slice(&fs::read(path).context("Couldn't read file")?).context("Invalid entity")?;

					let mut entity = extract_entity(
						game_files,
						&app_state.cached_entities,
						get_loaded_game_version(app, install)?,
						hash_list,
						RuntimeID::from_any(&patch.factory_hash)?
					)?
					.to_owned();

					let base = entity.to_owned();

					apply_patch(&mut entity, patch, true).map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

					// Normalise comments to form used by GlacierKit (single comment for each entity)
					let mut comments: Vec<CommentEntity> = vec![];
					for comment in entity.comments {
						if let Some(x) = comments.iter_mut().find(|x| x.parent == comment.parent) {
							x.text = format!("{}\n\n{}", x.text, comment.text);
						} else {
							comments.push(CommentEntity {
								parent: comment.parent,
								name: "Notes".into(),
								text: comment.text
							});
						}
					}
					entity.comments = comments;

					app_state.editor_states.insert(
						id.to_owned(),
						EditorState {
							file: Some(path.to_owned()),
							data: EditorData::QNPatch {
								base: Box::new(base),
								current: Box::new(entity),
								settings: Default::default()
							}
						}
					);

					send_request(
						app,
						Request::Global(GlobalRequest::CreateTab {
							id,
							name: path.file_name().context("No file name")?.to_string_lossy().into(),
							editor_type: EditorType::QNPatch
						})
					)?;
				} else {
					send_request(
						app,
						Request::Tool(ToolRequest::FileBrowser(FileBrowserRequest::Select(None)))
					)?;

					send_notification(
						app,
						Notification {
							kind: NotificationKind::Error,
							title: "No game selected".into(),
							subtitle: "You can't open patch files without a copy of the game selected.".into()
						}
					)?;
				}
			}

			"json" | "JSON" => {
				let id = Uuid::new_v4();

				let file_type = if path.file_name().context("No file name")?.to_string_lossy() == "manifest.json" {
					TextFileType::ManifestJson
				} else {
					TextFileType::Json
				};

				app_state.editor_states.insert(
					id.to_owned(),
					EditorState {
						file: Some(path.to_owned()),
						data: EditorData::Text {
							content: fs::read_to_string(path)
								.context("Couldn't read file")?
								.replace("\r\n", "\n"),
							file_type: file_type.to_owned()
						}
					}
				);

				send_request(
					app,
					Request::Global(GlobalRequest::CreateTab {
						id,
						name: path.file_name().context("No file name")?.to_string_lossy().into(),
						editor_type: EditorType::Text { file_type }
					})
				)?;
			}

			"txt" => {
				let id = Uuid::new_v4();

				app_state.editor_states.insert(
					id.to_owned(),
					EditorState {
						file: Some(path.to_owned()),
						data: EditorData::Text {
							content: fs::read_to_string(path)
								.context("Couldn't read file")?
								.replace("\r\n", "\n"),
							file_type: TextFileType::PlainText
						}
					}
				);

				send_request(
					app,
					Request::Global(GlobalRequest::CreateTab {
						id,
						name: path.file_name().context("No file name")?.to_string_lossy().into(),
						editor_type: EditorType::Text {
							file_type: TextFileType::PlainText
						}
					})
				)?;
			}

			"md" => {
				let id = Uuid::new_v4();

				app_state.editor_states.insert(
					id.to_owned(),
					EditorState {
						file: Some(path.to_owned()),
						data: EditorData::Text {
							content: fs::read_to_string(path)
								.context("Couldn't read file")?
								.replace("\r\n", "\n"),
							file_type: TextFileType::Markdown
						}
					}
				);

				send_request(
					app,
					Request::Global(GlobalRequest::CreateTab {
						id,
						name: path.file_name().context("No file name")?.to_string_lossy().into(),
						editor_type: EditorType::Text {
							file_type: TextFileType::Markdown
						}
					})
				)?;
			}

			"repository.json" => {
				let id = Uuid::new_v4();

				if let Some(cached_repository) = app_state.repository.load().as_ref() {
					let mut repository = to_value(
						cached_repository
							.iter()
							.cloned()
							.map(|x| (x.id, x.data))
							.collect::<IndexMap<Uuid, IndexMap<String, Value>>>()
					)?;

					let base = to_value(cached_repository)?;

					let patch: Value =
						from_slice(&fs::read(path).context("Couldn't read file")?).context("Invalid JSON")?;

					json_patch::merge(&mut repository, &patch);

					let repository = from_value::<IndexMap<Uuid, IndexMap<String, Value>>>(repository)?
						.into_iter()
						.map(|(id, data)| RepositoryItem { id, data })
						.collect();

					app_state.editor_states.insert(
						id.to_owned(),
						EditorState {
							file: Some(path.to_owned()),
							data: EditorData::RepositoryPatch {
								base: from_value(base)?,
								current: repository,
								patch_type: JsonPatchType::MergePatch
							}
						}
					);

					send_request(
						app,
						Request::Global(GlobalRequest::CreateTab {
							id,
							name: path.file_name().context("No file name")?.to_string_lossy().into(),
							editor_type: EditorType::RepositoryPatch {
								patch_type: JsonPatchType::MergePatch
							}
						})
					)?;
				} else {
					send_request(
						app,
						Request::Tool(ToolRequest::FileBrowser(FileBrowserRequest::Select(None)))
					)?;

					send_notification(
						app,
						Notification {
							kind: NotificationKind::Error,
							title: "No game selected".into(),
							subtitle: "You can't open patch files without a copy of the game selected.".into()
						}
					)?;
				}
			}

			"unlockables.json" => {
				let id = Uuid::new_v4();

				if let Some(game_files) = app_state.game_files.load().as_ref() {
					let mut unlockables = to_value(
						from_str::<Vec<UnlockableItem>>(&parse_json_ores(
							&extract_latest_resource(game_files, "0057C2C3941115CA".parse()?)?.1
						)?)?
						.into_iter()
						.map(|x| {
							(
								x.data
									.get("Id")
									.expect("Unlockable did not have Id")
									.as_str()
									.expect("Id was not string")
									.to_owned(),
								{
									let mut y = IndexMap::new();
									y.insert("Guid".into(), to_value(x.id).unwrap());
									y.extend(x.data.into_iter().filter(|(key, _)| key != "Id"));
									y
								}
							)
						})
						.collect::<IndexMap<String, IndexMap<String, Value>>>()
					)?;

					let base = from_str::<Value>(&parse_json_ores(
						&extract_latest_resource(game_files, "0057C2C3941115CA".parse()?)?.1
					)?)?;

					let patch: Value =
						from_slice(&fs::read(path).context("Couldn't read file")?).context("Invalid JSON")?;

					json_patch::merge(&mut unlockables, &patch);

					let unlockables = from_value::<IndexMap<String, IndexMap<String, Value>>>(unlockables)?
						.into_iter()
						.map(|(id, data)| UnlockableItem {
							id: data
								.get("Guid")
								.expect("No Guid on unlockable item")
								.as_str()
								.expect("Guid was not string")
								.try_into()
								.expect("Guid was not valid UUID"),
							data: {
								let mut y = IndexMap::new();
								y.insert("Id".into(), Value::String(id));
								y.extend(data.into_iter().filter(|(key, _)| key != "Guid"));
								y
							}
						})
						.collect();

					app_state.editor_states.insert(
						id.to_owned(),
						EditorState {
							file: Some(path.to_owned()),
							data: EditorData::UnlockablesPatch {
								base: from_value(base)?,
								current: unlockables,
								patch_type: JsonPatchType::MergePatch
							}
						}
					);

					send_request(
						app,
						Request::Global(GlobalRequest::CreateTab {
							id,
							name: path.file_name().context("No file name")?.to_string_lossy().into(),
							editor_type: EditorType::UnlockablesPatch {
								patch_type: JsonPatchType::MergePatch
							}
						})
					)?;
				} else {
					send_request(
						app,
						Request::Tool(ToolRequest::FileBrowser(FileBrowserRequest::Select(None)))
					)?;

					send_notification(
						app,
						Notification {
							kind: NotificationKind::Error,
							title: "No game selected".into(),
							subtitle: "You can't open patch files without a copy of the game selected.".into()
						}
					)?;
				}
			}

			"JSON.patch.json" => {
				let id = Uuid::new_v4();

				let file: Value =
					from_slice(&fs::read(path).context("Couldn't read file")?).context("Invalid patch")?;

				match file
					.get("type")
					.unwrap_or(&Value::String("JSON".into()))
					.as_str()
					.context("Type key was not string")?
				{
					"REPO" => {
						if let Some(cached_repository) = app_state.repository.load().as_ref() {
							let mut repository = to_value(
								cached_repository
									.iter()
									.cloned()
									.map(|x| (x.id, x.data))
									.collect::<IndexMap<Uuid, IndexMap<String, Value>>>()
							)?;

							let base = to_value(cached_repository)?;

							let patch = from_slice::<Value>(&fs::read(path).context("Couldn't read file")?)
								.context("Invalid JSON")?;

							let patch = patch.get("patch").context("Patch had no patch key")?;

							json_patch::patch(
								&mut repository,
								&from_value::<Vec<json_patch::PatchOperation>>(patch.to_owned())
									.context("Invalid JSON patch")?
							)?;

							let repository = from_value::<IndexMap<Uuid, IndexMap<String, Value>>>(repository)?
								.into_iter()
								.map(|(id, data)| RepositoryItem { id, data })
								.collect();

							app_state.editor_states.insert(
								id.to_owned(),
								EditorState {
									file: Some(path.to_owned()),
									data: EditorData::RepositoryPatch {
										base: from_value(base)?,
										current: repository,
										patch_type: JsonPatchType::JsonPatch
									}
								}
							);

							send_request(
								app,
								Request::Global(GlobalRequest::CreateTab {
									id,
									name: path.file_name().context("No file name")?.to_string_lossy().into(),
									editor_type: EditorType::RepositoryPatch {
										patch_type: JsonPatchType::JsonPatch
									}
								})
							)?;
						} else {
							send_request(
								app,
								Request::Tool(ToolRequest::FileBrowser(FileBrowserRequest::Select(None)))
							)?;

							send_notification(
								app,
								Notification {
									kind: NotificationKind::Error,
									title: "No game selected".into(),
									subtitle: "You can't open patch files without a copy of the game selected.".into()
								}
							)?;
						}
					}

					"ORES"
						if file
							.get("file")
							.context("Patch had no file key")?
							.as_str()
							.context("Type key was not string")?
							== "0057C2C3941115CA" =>
					{
						let id = Uuid::new_v4();

						if let Some(game_files) = app_state.game_files.load().as_ref() {
							let mut unlockables = to_value(
								from_str::<Vec<UnlockableItem>>(&parse_json_ores(
									&extract_latest_resource(game_files, "0057C2C3941115CA".parse()?)?.1
								)?)?
								.into_iter()
								.map(|x| {
									(
										x.data
											.get("Id")
											.expect("Unlockable did not have Id")
											.as_str()
											.expect("Id was not string")
											.to_owned(),
										{
											let mut y = IndexMap::new();
											y.insert("Guid".into(), to_value(x.id).unwrap());
											y.extend(x.data.into_iter().filter(|(key, _)| key != "Id"));
											y
										}
									)
								})
								.collect::<IndexMap<String, IndexMap<String, Value>>>()
							)?;

							let base = from_str::<Value>(&parse_json_ores(
								&extract_latest_resource(game_files, "0057C2C3941115CA".parse()?)?.1
							)?)?;

							let patch = from_slice::<Value>(&fs::read(path).context("Couldn't read file")?)
								.context("Invalid JSON")?;

							let patch = patch.get("patch").context("Patch had no patch key")?;

							json_patch::patch(
								&mut unlockables,
								&from_value::<Vec<json_patch::PatchOperation>>(patch.to_owned())
									.context("Invalid JSON patch")?
							)?;

							let unlockables = from_value::<IndexMap<String, IndexMap<String, Value>>>(unlockables)?
								.into_iter()
								.map(|(id, data)| UnlockableItem {
									id: data
										.get("Guid")
										.expect("No Guid on unlockable item")
										.as_str()
										.expect("Guid was not string")
										.try_into()
										.expect("Guid was not valid UUID"),
									data: {
										let mut y = IndexMap::new();
										y.insert("Id".into(), Value::String(id));
										y.extend(data.into_iter().filter(|(key, _)| key != "Guid"));
										y
									}
								})
								.collect();

							app_state.editor_states.insert(
								id.to_owned(),
								EditorState {
									file: Some(path.to_owned()),
									data: EditorData::UnlockablesPatch {
										base: from_value(base)?,
										current: unlockables,
										patch_type: JsonPatchType::JsonPatch
									}
								}
							);

							send_request(
								app,
								Request::Global(GlobalRequest::CreateTab {
									id,
									name: path.file_name().context("No file name")?.to_string_lossy().into(),
									editor_type: EditorType::UnlockablesPatch {
										patch_type: JsonPatchType::JsonPatch
									}
								})
							)?;
						} else {
							send_request(
								app,
								Request::Tool(ToolRequest::FileBrowser(FileBrowserRequest::Select(None)))
							)?;

							send_notification(
								app,
								Notification {
									kind: NotificationKind::Error,
									title: "No game selected".into(),
									subtitle: "You can't open patch files without a copy of the game selected.".into()
								}
							)?;
						}
					}

					_ => {
						app_state.editor_states.insert(
							id.to_owned(),
							EditorState {
								file: Some(path.to_owned()),
								data: EditorData::Text {
									content: fs::read_to_string(path)
										.context("Couldn't read file")?
										.replace("\r\n", "\n"),
									file_type: TextFileType::Json
								}
							}
						);

						send_request(
							app,
							Request::Global(GlobalRequest::CreateTab {
								id,
								name: path.file_name().context("No file name")?.to_string_lossy().into(),
								editor_type: EditorType::Text {
									file_type: TextFileType::Json
								}
							})
						)?;
					}
				}
			}

			"dlge.json" | "locr.json" | "rtlv.json" | "clng.json" | "ditl.json" | "material.json" | "contract.json" => {
				let id = Uuid::new_v4();

				app_state.editor_states.insert(
					id.to_owned(),
					EditorState {
						file: Some(path.to_owned()),
						data: EditorData::Text {
							content: fs::read_to_string(path)
								.context("Couldn't read file")?
								.replace("\r\n", "\n"),
							file_type: TextFileType::Json
						}
					}
				);

				send_request(
					app,
					Request::Global(GlobalRequest::CreateTab {
						id,
						name: path.file_name().context("No file name")?.to_string_lossy().into(),
						editor_type: EditorType::Text {
							file_type: TextFileType::Json
						}
					})
				)?;
			}

			_ => {
				// Unsupported extension

				let id = Uuid::new_v4();

				app_state.editor_states.insert(
					id.to_owned(),
					EditorState {
						file: Some(path.to_owned()),
						data: EditorData::Nil
					}
				);

				send_request(
					app,
					Request::Global(GlobalRequest::CreateTab {
						id,
						name: path.file_name().context("No file name")?.to_string_lossy().into(),
						editor_type: EditorType::Nil
					})
				)?;
			}
		}
	}

	finish_task(app, task)?;
}

#[try_fn]
#[context("Couldn't load game files")]
pub async fn load_game_files(app: &AppHandle) -> Result<()> {
	let app_state = app.state::<AppState>();
	let app_settings = app.state::<ArcSwap<AppSettings>>();

	app_state.game_files.store(None);
	app_state.resource_reverse_dependencies.store(None);
	app_state.intellisense.store(None);
	app_state.repository.store(None);
	app_state.cached_entities.clear();

	if let Some(path) = app_settings.load().game_install.as_ref() {
		let task = start_task(app, "Loading game files")?;

		let thumbs = IniFileSystem::from(path.join("thumbs.dat")).context("Couldn't load thumbs.dat")?;

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

		// Workaround for the Linux filesystem.
		// The relative_runtime_path will in most cases be "runtime", while the folder is actually called "Runtime"
		// Windows doesn't care about the mismatched casing, UNIX does :(
		let relative_runtime_path_uppercased = relative_runtime_path
			.char_indices()
			.map(|(idx, ch)| if idx == 0 { ch.to_ascii_uppercase() } else { ch })
			.collect::<String>();

		let runtime_path = [relative_runtime_path, &relative_runtime_path_uppercased]
			.iter()
			.flat_map(|folder| path.join(proj_path.replace('\\', "/")).join(folder).canonicalize())
			.find(|joined_path| joined_path.exists())
			.context("Couldn't find valid runtime folder")?;

		let mut partition_manager = PartitionManager::new(runtime_path.clone());

		let mut partitions = match get_loaded_game_version(app, path)? {
			GameVersion::H1 => PackageDefinitionSource::HM2016(fs::read(runtime_path.join("packagedefinition.txt"))?)
				.read()
				.context("Couldn't read packagedefinition")?,

			GameVersion::H2 => PackageDefinitionSource::HM2(fs::read(runtime_path.join("packagedefinition.txt"))?)
				.read()
				.context("Couldn't read packagedefinition")?,

			GameVersion::H3 => PackageDefinitionSource::HM3(fs::read(runtime_path.join("packagedefinition.txt"))?)
				.read()
				.context("Couldn't read packagedefinition")?
		};

		if !app_settings.load().extract_modded_files {
			for partition in &mut partitions {
				partition.set_max_patch_level(9);
			}
		}

		finish_task(app, task)?;

		let partition_names = partitions.iter().map(|x| x.id().to_string()).collect_vec();

		let mut last_index = 0;
		let mut last_progress = 0;
		let mut loading_task = start_task(app, format!("Loading {} (0%)", partition_names[last_index]))?;

		partition_manager
			.mount_partitions(PackageDefinitionSource::Custom(partitions), |cur_partition, state| {
				if cur_partition < partition_names.len() {
					if cur_partition != last_index {
						last_index = cur_partition;
						last_progress = 0;

						finish_task(app, loading_task).expect("Couldn't send data to frontend");
						loading_task = start_task(app, format!("Loading {} (0%)", partition_names[last_index]))
							.expect("Couldn't send data to frontend");
					}

					let progress = ((state.install_progress * 10.0).round() * 10.0) as u8;
					if progress != last_progress {
						last_progress = progress;

						finish_task(app, loading_task).expect("Couldn't send data to frontend");
						loading_task = start_task(
							app,
							format!("Loading {} ({}%)", partition_names[last_index], last_progress)
						)
						.expect("Couldn't send data to frontend");
					}
				}
			})
			.context("Couldn't mount partitions")?;

		finish_task(app, loading_task)?;
		let task = start_task(app, "Caching reverse references")?;

		let mut reverse_dependencies: DashMap<RuntimeID, Vec<RuntimeID>> = DashMap::new();

		// Ensure we only get the references from the lowest chunk version of each resource (matches the rest of GK's behaviour)
		let resources = partition_manager
			.partitions()
			.into_par_iter()
			.rev()
			.flat_map(|partition| {
				partition.latest_resources().into_par_iter().map(|(resource, _)| {
					(
						RuntimeID::try_from(*resource.rrid()).expect("Invalid ID in game files"),
						resource.references()
					)
				})
			})
			.collect::<HashMap<_, _>>();

		reverse_dependencies
			.try_reserve(resources.len())
			.map_err(|e| anyhow!("Reserve error: {e:?}"))?;

		reverse_dependencies.par_extend(resources.par_keys().map(|&x| (x, Default::default())));

		resources
			.into_par_iter()
			.flat_map(|(resource_id, resource_references)| {
				resource_references.par_iter().map(move |(reference_id, _)| {
					(
						(*reference_id).try_into().expect("Invalid ID in game files"),
						resource_id
					)
				})
			})
			.for_each(|(key, value)| {
				if let Some(mut x) = reverse_dependencies.get_mut(&key) {
					x.push(value);
				}
			});

		send_request(
			app,
			Request::Tool(ToolRequest::ContentSearch(ContentSearchRequest::SetPartitions(
				partition_manager
					.partitions()
					.into_iter()
					.map(|x| {
						(
							x.partition_info().name().as_deref().unwrap_or("<unnamed>").to_owned(),
							x.partition_info().id().to_string()
						)
					})
					.collect()
			)))
		)?;

		app_state.game_files.store(Some(partition_manager.into()));

		app_state.resource_reverse_dependencies.store(Some(
			reverse_dependencies
				.into_par_iter()
				.map(|(x, mut y)| {
					(x, {
						y.sort_unstable();
						y.into_iter().dedup().collect()
					})
				})
				.collect::<HashMap<_, _>>()
				.into()
		));

		finish_task(app, task)?;
	}

	let task = start_task(app, "Acquiring latest hash list")?;

	let current_version = app_state.hash_list.load().as_ref().map(|x| x.version).unwrap_or(0);

	if let Ok(data) = reqwest::get(HASH_LIST_VERSION_ENDPOINT).await {
		if let Ok(data) = data.text().await {
			let new_version = data
				.trim()
				.parse::<u32>()
				.context("Online hash list version wasn't a number")?;

			if current_version < new_version {
				if let Ok(data) = reqwest::get(HASH_LIST_ENDPOINT).await {
					if let Ok(data) = data.bytes().await {
						let hash_list = HashList::from_compressed(&data)?;

						fs::write(
							app.path_resolver()
								.app_data_dir()
								.context("Couldn't get app data dir")?
								.join("hash_list.sml"),
							serde_smile::to_vec(&hash_list)?
						)?;

						app_state.hash_list.store(Some(hash_list.into()));
					}
				}
			}
		}
	}

	let current_version = app_state
		.tonytools_hash_list
		.load()
		.as_ref()
		.map(|x| x.version)
		.unwrap_or(0);

	if let Ok(data) = reqwest::get(TONYTOOLS_HASH_LIST_VERSION_ENDPOINT).await {
		if let Ok(data) = data.text().await {
			let new_version = from_str::<Value>(&data)
				.context("Couldn't parse online version data as JSON")?
				.get("version")
				.context("No version key in online version data")?
				.as_u64()
				.context("Online hash list version wasn't a number")? as u32;

			if current_version < new_version {
				if let Ok(data) = reqwest::get(TONYTOOLS_HASH_LIST_ENDPOINT).await {
					if let Ok(data) = data.bytes().await {
						let tonytools_hash_list = tonytools::hashlist::HashList::load(&data)
							.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

						fs::write(
							app.path_resolver()
								.app_data_dir()
								.context("Couldn't get app data dir")?
								.join("tonytools_hash_list.hmla"),
							data
						)?;

						app_state.tonytools_hash_list.store(Some(tonytools_hash_list.into()));
					}
				}
			}
		}
	}

	send_request(
		app,
		Request::Tool(ToolRequest::GameBrowser(GameBrowserRequest::SetEnabled(
			app_settings.load().game_install.is_some() && app_state.hash_list.load().is_some()
		)))
	)?;

	send_request(
		app,
		Request::Tool(ToolRequest::ContentSearch(ContentSearchRequest::SetEnabled(
			app_settings.load().game_install.is_some() && app_state.hash_list.load().is_some()
		)))
	)?;

	finish_task(app, task)?;

	if let Some(hash_list) = app_state.hash_list.load().as_ref()
		&& let Some(resource_reverse_dependencies) = app_state.resource_reverse_dependencies.load().as_ref()
	{
		let task = start_task(app, "Setting up intellisense")?;

		app_state.intellisense.store(Some(
			Intellisense {
				cppt_properties: DashMap::new().into(),
				cppt_pins: from_slice(include_bytes!("../assets/pins.json")).unwrap(),
				uicb_prop_types: from_slice(include_bytes!("../assets/uicbPropTypes.json")).unwrap(),
				matt_properties: DashMap::new().into(),
				file_types: resource_reverse_dependencies
					.par_iter()
					.filter_map(|(x, _)| Some((x.to_owned(), hash_list.entries.get(x)?.resource_type.to_owned())))
					.collect()
			}
			.into()
		));

		finish_task(app, task)?
	};

	if let Some(game_files) = app_state.game_files.load().as_ref() {
		let task = start_task(app, "Caching repository")?;

		app_state.repository.store(Some(
			from_slice::<Vec<RepositoryItem>>(&extract_latest_resource(game_files, "00204D1AFD76AB13".parse()?)?.1)?
				.into()
		));

		finish_task(app, task)?;
	}

	if let Some(game_files) = app_state.game_files.load().as_ref()
		&& let Some(resource_reverse_dependencies) = app_state.resource_reverse_dependencies.load().as_ref()
		&& let Some(install) = app_settings.load().game_install.as_ref()
		&& let Some(hash_list) = app_state.hash_list.load().as_ref()
	{
		let task = start_task(app, "Refreshing editors")?;

		for editor in app_state.editor_states.iter_mut() {
			if let EditorData::ResourceOverview { hash } = editor.data {
				let task = start_task(app, format!("Refreshing resource overview for {}", hash))?;

				initialise_resource_overview(
					app,
					&app_state,
					editor.key().to_owned(),
					hash,
					game_files,
					get_loaded_game_version(app, install)?,
					resource_reverse_dependencies,
					hash_list
				)?;

				finish_task(app, task)?;
			}
		}

		finish_task(app, task)?;
	}
}

/// Only available for entities, the repository and unlockables currently
#[try_fn]
#[context("Couldn't open {hash} in editor")]
pub async fn open_in_editor(
	app: &AppHandle,
	game_files: &PartitionManager,
	install: &PathBuf,
	hash_list: &HashList,
	hash: RuntimeID
) -> Result<()> {
	let app_state = app.state::<AppState>();

	match hash_list
		.entries
		.get(&hash)
		.context("Not in hash list")?
		.resource_type
		.as_ref()
	{
		"TEMP" => {
			let task = start_task(app, format!("Loading entity {}", hash))?;

			let entity = extract_entity(
				game_files,
				&app_state.cached_entities,
				get_loaded_game_version(app, install)?,
				hash_list,
				hash
			)?
			.to_owned();

			let default_tab_name = format!(
				"{} ({})",
				entity
					.entities
					.get(&entity.root_entity)
					.context("Root entity doesn't exist")?
					.name,
				hash
			);

			let tab_name = if let Some(entry) = hash_list.entries.get(&hash) {
				if let Some(path) = entry.path.as_ref() {
					path.replace("].pc_entitytype", "")
						.replace("].pc_entitytemplate", "")
						.split('/')
						.last()
						.map(|x| x.to_owned())
						.unwrap_or(default_tab_name)
				} else if let Some(hint) = entry.hint.as_ref() {
					format!("{} ({})", hint, hash)
				} else {
					default_tab_name
				}
			} else {
				default_tab_name
			};

			let id = Uuid::new_v4();

			app_state.editor_states.insert(
				id.to_owned(),
				EditorState {
					file: None,
					data: EditorData::QNPatch {
						base: Box::new(entity.to_owned()),
						current: Box::new(entity),
						settings: Default::default()
					}
				}
			);

			send_request(
				app,
				Request::Global(GlobalRequest::CreateTab {
					id,
					name: tab_name,
					editor_type: EditorType::QNPatch
				})
			)?;

			finish_task(app, task)?;
		}

		"REPO" => {
			let task = start_task(app, "Loading repository")?;

			let id = Uuid::new_v4();

			let repository: Vec<RepositoryItem> = if let Some(x) = app_state.repository.load().as_ref() {
				x.par_iter().cloned().collect()
			} else {
				from_slice(&extract_latest_resource(game_files, "00204D1AFD76AB13".parse()?)?.1)?
			};

			app_state.editor_states.insert(
				id.to_owned(),
				EditorState {
					file: None,
					data: EditorData::RepositoryPatch {
						base: repository.to_owned(),
						current: repository,
						patch_type: JsonPatchType::MergePatch
					}
				}
			);

			send_request(
				app,
				Request::Global(GlobalRequest::CreateTab {
					id,
					name: "pro.repo".into(),
					editor_type: EditorType::RepositoryPatch {
						patch_type: JsonPatchType::MergePatch
					}
				})
			)?;

			finish_task(app, task)?;
		}

		"ORES" if hash == "0057C2C3941115CA".parse()? => {
			let task = start_task(app, "Loading unlockables")?;

			let id = Uuid::new_v4();

			let unlockables: Vec<UnlockableItem> = from_str(&parse_json_ores(
				&extract_latest_resource(game_files, "0057C2C3941115CA".parse()?)?.1
			)?)?;

			app_state.editor_states.insert(
				id.to_owned(),
				EditorState {
					file: None,
					data: EditorData::UnlockablesPatch {
						base: unlockables.to_owned(),
						current: unlockables,
						patch_type: JsonPatchType::MergePatch
					}
				}
			);

			send_request(
				app,
				Request::Global(GlobalRequest::CreateTab {
					id,
					name: "config.unlockables".into(),
					editor_type: EditorType::UnlockablesPatch {
						patch_type: JsonPatchType::MergePatch
					}
				})
			)?;

			finish_task(app, task)?;
		}

		x => panic!("Opening {x} files in editor is not supported")
	}
}
