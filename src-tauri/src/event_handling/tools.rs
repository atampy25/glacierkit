use std::{fs, ops::Deref, time::Duration};

use anyhow::{anyhow, Context, Result};
use arc_swap::ArcSwap;
use fn_error_context::context;
use hitman_commons::{game::GameVersion, metadata::RuntimeID, rpkg_tool::RpkgResourceMeta};
use hitman_formats::ores::parse_json_ores;
use indexmap::IndexMap;
use itertools::Itertools;
use quickentity_rs::{
	apply_patch, convert_to_qn, convert_to_rt, generate_patch,
	patch_structs::Patch,
	qn_structs::{CommentEntity, Entity, Ref, SubEntity, SubType}
};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use rpkg_rs::resource::runtime_resource_id::RuntimeResourceID;
use serde_json::{from_slice, from_str, from_value, json, to_string, to_value, to_vec, Value};
use tauri::{async_runtime, AppHandle, Manager};
use tauri_plugin_aptabase::EventTracker;
use tokio::net::TcpStream;
use tryvial::try_fn;
use uuid::Uuid;
use velcro::vec;

use crate::ores_repo::UnlockableItem;
use crate::resourcelib::{
	h2016_convert_binary_to_blueprint, h2016_convert_binary_to_factory, h2_convert_binary_to_blueprint,
	h2_convert_binary_to_factory, h3_convert_binary_to_blueprint, h3_convert_binary_to_factory
};
use crate::rpkg::extract_latest_resource;
use crate::{
	convert_json_patch_to_merge_patch,
	model::{
		AppSettings, AppState, ContentSearchEvent, EditorData, EditorState, EditorType, FileBrowserEvent,
		GameBrowserEntry, GameBrowserEvent, GameBrowserRequest, GlobalRequest, Request, SearchFilter, SettingsEvent,
		SettingsRequest, ToolEvent, ToolRequest
	}
};
use crate::{event_handling::content_search::start_content_search, send_request};
use crate::{finish_task, start_task};
use crate::{general::open_in_editor, rpkg::extract_entity};
use crate::{
	general::{load_game_files, open_file},
	get_loaded_game_version
};
use crate::{send_notification, Notification, NotificationKind};

#[try_fn]
#[context("Couldn't handle tool event")]
pub async fn handle_tool_event(app: &AppHandle, event: ToolEvent) -> Result<()> {
	let app_settings = app.state::<ArcSwap<AppSettings>>();
	let app_state = app.state::<AppState>();

	match event {
		ToolEvent::FileBrowser(event) => match event {
			FileBrowserEvent::Select(path) => {
				if let Some(path) = path {
					open_file(app, path).await?;
				}
			}

			FileBrowserEvent::Create { path, is_folder } => {
				let task = start_task(
					app,
					format!(
						"Creating {} {}",
						if is_folder { "folder" } else { "file" },
						path.file_name().unwrap().to_string_lossy()
					)
				)?;

				if is_folder {
					fs::create_dir(path)?;
				} else if path.extension().is_some() {
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
							fs::write(
								path,
								to_string(&Entity {
									factory_hash: String::new(),
									blueprint_hash: String::new(),
									root_entity: "fffffffffffffffe".into(),
									entities: velcro::map_iter! {
										"fffffffffffffffe": SubEntity {
											parent: Ref::Short(None),
											name: "Scene".into(),
											factory: "[modules:/zspatialentity.class].pc_entitytype".into(),
											blueprint: "[modules:/zspatialentity.class].pc_entityblueprint".into(),
											factory_flag: None,
											editor_only: None,
											properties: None,
											platform_specific_properties: None,
											events: None,
											input_copying: None,
											output_copying: None,
											property_aliases: None,
											exposed_entities: None,
											exposed_interfaces: None,
											subsets: None
										}
									}
									.map(|(x, y)| (x.to_owned(), y))
									.collect(),
									property_overrides: vec![],
									override_deletes: vec![],
									pin_connection_overrides: vec![],
									pin_connection_override_deletes: vec![],
									external_scenes: vec![],
									sub_type: SubType::Scene,
									quick_entity_version: 3.1,
									extra_factory_dependencies: vec![],
									extra_blueprint_dependencies: vec![],
									comments: vec![]
								})?
							)?;
						}

						"repository.json" => {
							fs::write(path, "{}")?;
						}

						_ => {
							fs::write(path, "")?;
						}
					}
				} else {
					fs::write(path, "")?;
				}

				finish_task(app, task)?;
			}

			FileBrowserEvent::Delete(path) => {
				let task = start_task(
					app,
					format!("Moving {} to bin", path.file_name().unwrap().to_string_lossy())
				)?;

				trash::delete(path)?;

				finish_task(app, task)?;
			}

			FileBrowserEvent::Rename { old_path, new_path } => {
				let task = start_task(
					app,
					format!(
						"Renaming {} to {}",
						old_path.file_name().unwrap().to_string_lossy(),
						new_path.file_name().unwrap().to_string_lossy()
					)
				)?;

				fs::rename(old_path, new_path)?;

				finish_task(app, task)?;
			}

			FileBrowserEvent::NormaliseQNFile { path } => {
				let task = start_task(
					app,
					format!("Normalising {}", path.file_name().unwrap().to_string_lossy())
				)?;

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
						let mut entity: Entity =
							from_slice(&fs::read(&path).context("Couldn't read file")?).context("Invalid entity")?;

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
						entity.comments = vec![]; // we don't need them here, since they get erased by the conversion to RT anyway

						let (fac, fac_meta, blu, blu_meta) =
							convert_to_rt(&entity).map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

						let mut reconverted = convert_to_qn(&fac, &fac_meta, &blu, &blu_meta, false)
							.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

						reconverted.comments = comments;

						fs::write(path, to_vec(&reconverted)?)?;

						send_notification(
							app,
							Notification {
								kind: NotificationKind::Success,
								title: "File normalised".into(),
								subtitle: "The entity file has been re-saved in canonical format.".into()
							}
						)?;
					}

					"entity.patch.json" => {
						let patch: Patch =
							from_slice(&fs::read(&path).context("Couldn't read file")?).context("Invalid entity")?;

						if let Some(game_files) = app_state.game_files.load().as_ref()
							&& let Some(install) = app_settings.load().game_install.as_ref()
							&& let Some(hash_list) = app_state.hash_list.load().as_ref()
						{
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
							entity.comments = vec![];

							let (fac, fac_meta, blu, blu_meta) =
								convert_to_rt(&entity).map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

							let mut reconverted = convert_to_qn(&fac, &fac_meta, &blu, &blu_meta, false)
								.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

							reconverted.comments = comments;

							fs::write(
								path,
								to_vec(
									&generate_patch(&base, &reconverted)
										.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?
								)?
							)?;

							send_notification(
								app,
								Notification {
									kind: NotificationKind::Success,
									title: "File normalised".into(),
									subtitle: "The patch file has been re-saved in canonical format.".into()
								}
							)?;
						} else {
							send_notification(
								app,
								Notification {
									kind: NotificationKind::Error,
									title: "No game selected".into(),
									subtitle: "You can't normalise patch files without a copy of the game selected."
										.into()
								}
							)?;
						}
					}

					_ => {
						Err(anyhow!("Can't normalise non-QN files"))?;
						panic!();
					}
				}

				finish_task(app, task)?;
			}

			FileBrowserEvent::ConvertEntityToPatch { path } => {
				if let Some(game_files) = app_state.game_files.load().as_ref()
					&& let Some(install) = app_settings.load().game_install.as_ref()
					&& let Some(hash_list) = app_state.hash_list.load().as_ref()
				{
					let mut entity: Entity =
						from_slice(&fs::read(&path).context("Couldn't read file")?).context("Invalid entity")?;

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

					let game_version = get_loaded_game_version(app, install)?;

					// `extract_entity` is not used here because the entity needs to be extracted in non-lossless mode to avoid meaningless `scale`-removing patch operations being added.
					let (temp_meta, temp_data) =
						extract_latest_resource(game_files, RuntimeID::from_any(&entity.factory_hash)?)?;

					let factory = match game_version {
						GameVersion::H1 => h2016_convert_binary_to_factory(&temp_data)
							.context("Couldn't convert binary data to ResourceLib factory")?
							.into_modern(),

						GameVersion::H2 => h2_convert_binary_to_factory(&temp_data)
							.context("Couldn't convert binary data to ResourceLib factory")?,

						GameVersion::H3 => h3_convert_binary_to_factory(&temp_data)
							.context("Couldn't convert binary data to ResourceLib factory")?
					};

					let blueprint_hash = temp_meta
						.core_info
						.references
						.get(factory.blueprint_index_in_resource_header as usize)
						.context("Blueprint referenced in factory does not exist in dependencies")?
						.resource;

					let (tblu_meta, tblu_data) = extract_latest_resource(game_files, blueprint_hash)?;

					let blueprint = match game_version {
						GameVersion::H1 => h2016_convert_binary_to_blueprint(&tblu_data)
							.context("Couldn't convert binary data to ResourceLib blueprint")?
							.into_modern(),

						GameVersion::H2 => h2_convert_binary_to_blueprint(&tblu_data)
							.context("Couldn't convert binary data to ResourceLib blueprint")?,

						GameVersion::H3 => h3_convert_binary_to_blueprint(&tblu_data)
							.context("Couldn't convert binary data to ResourceLib blueprint")?
					};

					let base = convert_to_qn(
						&factory,
						&RpkgResourceMeta::from_resource_metadata(temp_meta, false)
							.with_hash_list(&hash_list.entries)?,
						&blueprint,
						&RpkgResourceMeta::from_resource_metadata(tblu_meta, false)
							.with_hash_list(&hash_list.entries)?,
						false
					)
					.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

					fs::write(
						{
							let mut x = path.to_owned();
							x.pop();
							x.push(
								path.file_name()
									.context("No file name")?
									.to_string_lossy()
									.replace(".entity.json", ".entity.patch.json")
							);
							x
						},
						to_vec(&generate_patch(&base, &entity).map_err(|x| anyhow!("QuickEntity error: {:?}", x))?)?
					)?;

					fs::remove_file(&path)?;

					send_notification(
						app,
						Notification {
							kind: NotificationKind::Success,
							title: "File converted to patch".into(),
							subtitle: "The entity.json file has been converted into a patch file.".into()
						}
					)?;
				} else {
					send_notification(
						app,
						Notification {
							kind: NotificationKind::Error,
							title: "No game selected".into(),
							subtitle: "You can't convert between entity and patch without a copy of the game selected."
								.into()
						}
					)?;
				}
			}

			FileBrowserEvent::ConvertPatchToEntity { path } => {
				let patch: Patch =
					from_slice(&fs::read(&path).context("Couldn't read file")?).context("Invalid entity")?;

				if let Some(game_files) = app_state.game_files.load().as_ref()
					&& let Some(install) = app_settings.load().game_install.as_ref()
					&& let Some(hash_list) = app_state.hash_list.load().as_ref()
				{
					let mut entity = extract_entity(
						game_files,
						&app_state.cached_entities,
						get_loaded_game_version(app, install)?,
						hash_list,
						RuntimeID::from_any(&patch.factory_hash)?
					)?
					.to_owned();

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

					fs::write(
						{
							let mut x = path.to_owned();
							x.pop();
							x.push(
								path.file_name()
									.context("No file name")?
									.to_string_lossy()
									.replace(".entity.patch.json", ".entity.json")
							);
							x
						},
						to_vec(&entity)?
					)?;

					fs::remove_file(&path)?;

					send_notification(
						app,
						Notification {
							kind: NotificationKind::Success,
							title: "File converted to entity.json".into(),
							subtitle: "The patch file has been converted into an entity.json file.".into()
						}
					)?;
				} else {
					send_notification(
						app,
						Notification {
							kind: NotificationKind::Error,
							title: "No game selected".into(),
							subtitle: "You can't convert between entity and patch without a copy of the game selected."
								.into()
						}
					)?;
				}
			}

			FileBrowserEvent::ConvertRepoPatchToMergePatch { path } => {
				if from_slice::<Value>(&fs::read(&path).context("Couldn't read file")?)
					.context("Invalid JSON")?
					.get("type")
					.unwrap_or(&Value::String("JSON".into()))
					.as_str()
					.context("Type key was not string")?
					== "REPO"
				{
					if let Some(cached_repository) = app_state.repository.load().as_ref() {
						let mut current = to_value(
							cached_repository
								.iter()
								.cloned()
								.map(|x| (x.id, x.data))
								.collect::<IndexMap<Uuid, IndexMap<String, Value>>>()
						)?;

						let base = current.to_owned();

						let patch: Vec<json_patch::PatchOperation> = from_value(
							from_slice::<Value>(&fs::read(&path).context("Couldn't read file")?)
								.context("Invalid JSON")?
								.get("patch")
								.context("No patch key")?
								.to_owned()
						)
						.context("Invalid JSON patch")?;

						json_patch::patch(&mut current, &patch)?;

						let patch = json_patch::diff(&base, &current);

						fs::write(
							{
								let mut x = path.to_owned();
								x.pop();
								x.push(
									path.file_name()
										.context("No file name")?
										.to_string_lossy()
										.replace(".JSON.patch.json", ".repository.json")
								);
								x
							},
							to_vec(&convert_json_patch_to_merge_patch(&current, &patch)?)?
						)?;

						fs::remove_file(&path)?;

						send_notification(
							app,
							Notification {
								kind: NotificationKind::Success,
								title: "File converted to repository.json".into(),
								subtitle: "The patch file has been converted into a repository.json file.".into()
							}
						)?;
					} else {
						send_notification(
							app,
							Notification {
								kind: NotificationKind::Error,
								title: "No game selected".into(),
								subtitle: "You can't convert between patch formats without a copy of the game \
								           selected."
									.into()
							}
						)?;
					}
				} else {
					send_notification(
						app,
						Notification {
							kind: NotificationKind::Error,
							title: "Not a repository patch".into(),
							subtitle: "This patch is for a different type of file, so it can't be converted to a \
							           repository.json file."
								.into()
						}
					)?;
				}
			}

			FileBrowserEvent::ConvertRepoPatchToJsonPatch { path } => {
				if let Some(cached_repository) = app_state.repository.load().as_ref() {
					let mut current = to_value(
						cached_repository
							.iter()
							.cloned()
							.map(|x| (x.id, x.data))
							.collect::<IndexMap<Uuid, IndexMap<String, Value>>>()
					)?;

					let base = current.to_owned();

					let patch: Value =
						from_slice(&fs::read(&path).context("Couldn't read file")?).context("Invalid JSON")?;

					json_patch::merge(&mut current, &patch);

					send_request(
						app,
						Request::Global(GlobalRequest::ComputeJSONPatchAndSave {
							base,
							current,
							save_path: {
								let mut x = path.to_owned();
								x.pop();
								x.push(
									path.file_name()
										.context("No file name")?
										.to_string_lossy()
										.replace(".repository.json", ".JSON.patch.json")
								);
								x
							},
							file_and_type: ("00204D1AFD76AB13".into(), "REPO".into())
						})
					)?;

					fs::remove_file(&path)?;

					send_notification(
						app,
						Notification {
							kind: NotificationKind::Success,
							title: "File converted to JSON.patch.json".into(),
							subtitle: "The patch file has been converted into a JSON.patch.json file.".into()
						}
					)?;
				} else {
					send_notification(
						app,
						Notification {
							kind: NotificationKind::Error,
							title: "No game selected".into(),
							subtitle: "You can't convert between patch formats without a copy of the game selected."
								.into()
						}
					)?;
				}
			}

			FileBrowserEvent::ConvertUnlockablesPatchToMergePatch { path } => {
				if from_slice::<Value>(&fs::read(&path).context("Couldn't read file")?)
					.context("Invalid JSON")?
					.get("file")
					.context("Patch had no file key")?
					.as_str()
					.context("File key was not string")?
					== "0057C2C3941115CA"
				{
					if let Some(game_files) = app_state.game_files.load().as_ref() {
						let mut current = to_value(
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

						let base = current.to_owned();

						let patch: Vec<json_patch::PatchOperation> = from_value(
							from_slice::<Value>(&fs::read(&path).context("Couldn't read file")?)
								.context("Invalid JSON")?
								.get("patch")
								.context("No patch key")?
								.to_owned()
						)
						.context("Invalid JSON patch")?;

						json_patch::patch(&mut current, &patch)?;

						let patch = json_patch::diff(&base, &current);

						fs::write(
							{
								let mut x = path.to_owned();
								x.pop();
								x.push(
									path.file_name()
										.context("No file name")?
										.to_string_lossy()
										.replace(".JSON.patch.json", ".unlockables.json")
								);
								x
							},
							to_vec(&convert_json_patch_to_merge_patch(&current, &patch)?)?
						)?;

						fs::remove_file(&path)?;

						send_notification(
							app,
							Notification {
								kind: NotificationKind::Success,
								title: "File converted to unlockables.json".into(),
								subtitle: "The patch file has been converted into a unlockables.json file.".into()
							}
						)?;
					} else {
						send_notification(
							app,
							Notification {
								kind: NotificationKind::Error,
								title: "No game selected".into(),
								subtitle: "You can't convert between patch formats without a copy of the game \
								           selected."
									.into()
							}
						)?;
					}
				} else {
					send_notification(
						app,
						Notification {
							kind: NotificationKind::Error,
							title: "Not an unlockables patch".into(),
							subtitle: "This patch is for a different type of file, so it can't be converted to a \
							           unlockables.json file."
								.into()
						}
					)?;
				}
			}

			FileBrowserEvent::ConvertUnlockablesPatchToJsonPatch { path } => {
				if let Some(game_files) = app_state.game_files.load().as_ref() {
					let mut current = to_value(
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

					let base = current.to_owned();

					let patch: Value =
						from_slice(&fs::read(&path).context("Couldn't read file")?).context("Invalid JSON")?;

					json_patch::merge(&mut current, &patch);

					send_request(
						app,
						Request::Global(GlobalRequest::ComputeJSONPatchAndSave {
							base,
							current,
							save_path: {
								let mut x = path.to_owned();
								x.pop();
								x.push(
									path.file_name()
										.context("No file name")?
										.to_string_lossy()
										.replace(".unlockables.json", ".JSON.patch.json")
								);
								x
							},
							file_and_type: ("0057C2C3941115CA".into(), "ORES".into())
						})
					)?;

					fs::remove_file(&path)?;

					send_notification(
						app,
						Notification {
							kind: NotificationKind::Success,
							title: "File converted to JSON.patch.json".into(),
							subtitle: "The patch file has been converted into a JSON.patch.json file.".into()
						}
					)?;
				} else {
					send_notification(
						app,
						Notification {
							kind: NotificationKind::Error,
							title: "No game selected".into(),
							subtitle: "You can't convert between patch formats without a copy of the game selected."
								.into()
						}
					)?;
				}
			}
		},

		ToolEvent::GameBrowser(event) => match event {
			GameBrowserEvent::Select(hash) => {
				let id = Uuid::new_v4();

				app_state.editor_states.insert(
					id.to_owned(),
					EditorState {
						file: None,
						data: EditorData::ResourceOverview { hash: hash.to_owned() }
					}
				);

				send_request(
					app,
					Request::Global(GlobalRequest::CreateTab {
						id,
						name: format!("Resource overview ({hash})"),
						editor_type: EditorType::ResourceOverview
					})
				)?;
			}

			GameBrowserEvent::Search(query, filter) => {
				let task = start_task(app, format!("Searching game files for {}", query))?;

				if let Some(install) = app_settings.load().game_install.as_ref()
					&& let Some(game_files) = app_state.game_files.load().as_ref()
					&& let Some(resource_reverse_dependencies) = app_state.resource_reverse_dependencies.load().as_ref()
				{
					let install = app_state
						.game_installs
						.iter()
						.find(|x| x.path == *install)
						.context("No such game install")?;

					let filter_includes: &[&str] = match filter {
						SearchFilter::All => &[],
						SearchFilter::Templates => {
							&["TEMP", "CPPT", "ASET", "UICT", "MATT", "WSWT", "ECPT", "AIBX", "WSGT"]
						}
						SearchFilter::Classes => &["CPPT"],
						SearchFilter::Models => &["PRIM", "BORG", "ALOC"],
						SearchFilter::Textures => &["TEXT", "TEXD"],
						SearchFilter::Sound => &["WBNK", "WWFX", "WWEV", "WWES", "WWEM"]
					};

					let query_terms = query.split(' ').collect_vec();

					if let Some(hash_list) = app_state.hash_list.load().deref() {
						send_request(
							app,
							Request::Tool(ToolRequest::GameBrowser(GameBrowserRequest::NewTree {
								game_description: format!(
									"{} ({})",
									match install.version {
										GameVersion::H1 => "HITMAN™",
										GameVersion::H2 => "HITMAN 2",
										GameVersion::H3 => "HITMAN 3"
									},
									install.platform
								),
								entries: {
									if matches!(filter, SearchFilter::All) {
										hash_list
											.entries
											.par_iter()
											.filter(|(hash, _)| resource_reverse_dependencies.contains_key(*hash))
											.filter(|(hash, entry)| {
												query_terms.iter().all(|&y| {
													let mut s = format!(
														"{}{}{}.{}",
														entry.path.as_deref().unwrap_or(""),
														entry.hint.as_deref().unwrap_or(""),
														hash,
														entry.resource_type
													);

													s.make_ascii_lowercase();

													s.contains(y)
												})
											})
											.map(|(&hash, entry)| GameBrowserEntry {
												hash,
												path: entry.path.to_owned(),
												hint: entry.hint.to_owned(),
												filetype: entry.resource_type,
												partition: {
													let rrid = RuntimeResourceID::from(hash);

													let partition = game_files
														.partitions()
														.into_iter()
														.find(|x| x.contains(&rrid))
														.unwrap();

													(
														partition.partition_info().id().to_string(),
														partition
															.partition_info()
															.name()
															.to_owned()
															.unwrap_or("<unnamed>".into())
													)
												}
											})
											.collect()
									} else {
										hash_list
											.entries
											.par_iter()
											.filter(|(hash, _)| resource_reverse_dependencies.contains_key(*hash))
											.filter(|(_, entry)| {
												filter_includes.iter().any(|&x| entry.resource_type == x)
											})
											.filter(|(hash, entry)| {
												query_terms.iter().all(|&y| {
													let mut s = format!(
														"{}{}{}.{}",
														entry.path.as_deref().unwrap_or(""),
														entry.hint.as_deref().unwrap_or(""),
														hash,
														entry.resource_type
													);

													s.make_ascii_lowercase();

													s.contains(y)
												})
											})
											.map(|(&hash, entry)| GameBrowserEntry {
												hash,
												path: entry.path.to_owned(),
												hint: entry.hint.to_owned(),
												filetype: entry.resource_type,
												partition: {
													let rrid = RuntimeResourceID::from(hash);

													let partition = game_files
														.partitions()
														.into_iter()
														.find(|x| x.contains(&rrid))
														.unwrap();

													(
														partition.partition_info().id().to_string(),
														partition
															.partition_info()
															.name()
															.to_owned()
															.unwrap_or("<unnamed>".into())
													)
												}
											})
											.collect()
									}
								}
							}))
						)?;
					}
				}

				finish_task(app, task)?;
			}

			GameBrowserEvent::OpenInEditor(hash) => {
				if let Some(game_files) = app_state.game_files.load().as_ref()
					&& let Some(install) = app_settings.load().game_install.as_ref()
					&& let Some(hash_list) = app_state.hash_list.load().as_ref()
				{
					open_in_editor(app, game_files, install, hash_list, hash).await?;
				}
			}
		},

		ToolEvent::Settings(event) => match event {
			SettingsEvent::Initialise => {
				if let Ok(req) = reqwest::get("https://hitman-resources.netlify.app/glacierkit/dynamics.json").await {
					send_request(
						app,
						Request::Global(GlobalRequest::InitialiseDynamics {
							dynamics: req.json().await.context("Couldn't deserialise dynamics response")?,
							seen_announcements: app_settings.load().seen_announcements.to_owned()
						})
					)?;
				}

				let selected_install_info = app_settings
					.load()
					.game_install
					.as_ref()
					.map(|x| {
						let install = app_state
							.game_installs
							.iter()
							.find(|y| y.path == *x)
							.expect("No such game install");
						format!("{:?} {}", install.version, install.platform)
					})
					.unwrap_or("None".into());

				app.track_event(
					"App initialised",
					Some(json!({
						"game_installs": app_state.game_installs.len(),
						"extract_modded_files": app_settings.load().extract_modded_files,
						"colourblind_mode": app_settings.load().colourblind_mode,
						"editor_connection": app_settings.load().editor_connection,
						"selected_install": selected_install_info
					}))
				);

				send_request(
					app,
					Request::Tool(ToolRequest::Settings(SettingsRequest::Initialise {
						game_installs: app_state.game_installs.to_owned(),
						settings: (*app_settings.load_full()).to_owned()
					}))
				)?;

				if app
					.path_resolver()
					.app_log_dir()
					.context("Couldn't get log dir")?
					.join("..")
					.join("last_panic.txt")
					.exists()
				{
					send_request(app, Request::Global(GlobalRequest::RequestLastPanicUpload))?;
				}

				load_game_files(app).await?;

				let app = app.clone();

				async_runtime::spawn(async move {
					let mut interval = tokio::time::interval(Duration::from_secs(10));

					loop {
						interval.tick().await;

						// Attempt to connect every 10 seconds
						if app.state::<ArcSwap<AppSettings>>().load().editor_connection
							&& !app.state::<AppState>().editor_connection.is_connected().await
							&& TcpStream::connect("localhost:46735").await.is_ok()
						{
							let _ = app.state::<AppState>().editor_connection.connect().await;
						}
					}
				});
			}

			SettingsEvent::ChangeGameInstall(path) => {
				let mut settings = (*app_settings.load_full()).to_owned();

				if path != settings.game_install {
					settings.game_install = path;
					fs::write(
						app.path_resolver()
							.app_data_dir()
							.context("Couldn't get app data dir")?
							.join("settings.json"),
						to_vec(&settings)?
					)?;
					app_settings.store(settings.into());

					load_game_files(app).await?;
				}
			}

			SettingsEvent::ChangeExtractModdedFiles(value) => {
				let mut settings = (*app_settings.load_full()).to_owned();
				settings.extract_modded_files = value;
				fs::write(
					app.path_resolver()
						.app_data_dir()
						.context("Couldn't get app data dir")?
						.join("settings.json"),
					to_vec(&settings)?
				)?;
				app_settings.store(settings.into());
			}

			SettingsEvent::ChangeColourblind(value) => {
				let mut settings = (*app_settings.load_full()).to_owned();
				settings.colourblind_mode = value;
				fs::write(
					app.path_resolver()
						.app_data_dir()
						.context("Couldn't get app data dir")?
						.join("settings.json"),
					to_vec(&settings)?
				)?;
				app_settings.store(settings.into());
			}

			SettingsEvent::ChangeEditorConnection(value) => {
				let mut settings = (*app_settings.load_full()).to_owned();
				settings.editor_connection = value;

				if !value && app_state.editor_connection.is_connected().await {
					app_state.editor_connection.disconnect().await?;
				}

				fs::write(
					app.path_resolver()
						.app_data_dir()
						.context("Couldn't get app data dir")?
						.join("settings.json"),
					to_vec(&settings)?
				)?;
				app_settings.store(settings.into());
			}

			SettingsEvent::ChangeCustomPaths(value) => {
				if let Some(project) = app_state.project.load().as_ref() {
					app.track_event("Edit custom paths list manually", None);

					let mut settings = (*project.settings.load_full()).to_owned();
					settings.custom_paths = value;
					fs::write(project.path.join("project.json"), to_vec(&settings)?)?;
					project.settings.store(settings.into());
				}
			}
		},

		ToolEvent::ContentSearch(event) => match event {
			ContentSearchEvent::Search(query, filetypes, use_qn_format, partitions_to_search) => {
				start_content_search(app, query, filetypes, use_qn_format, partitions_to_search)?;
			}
		}
	}
}
