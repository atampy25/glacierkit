use std::fs;

use anyhow::{Context, Result, anyhow};
use arc_swap::ArcSwap;
use ecow::eco_format;
use fn_error_context::context;
use hitman_commons::{
	game::GameVersion,
	metadata::{ResourceReference, RuntimeID}
};
use hitman_formats::ores::parse_json_ores;
use indexmap::IndexMap;
use itertools::Itertools;
use quickentity_rs::{
	apply_patch, convert_to_game, convert_to_qn,
	entity::{CommentEntity, Entity, EntityID, SubEntity, SubType},
	generate_patch,
	patch::Patch
};
use rayon::iter::{ParallelBridge, ParallelIterator};
use rpkg_rs::resource::runtime_resource_id::RuntimeResourceID;
use serde_json::{Value, from_slice, from_str, from_value, to_string, to_value, to_vec};
use tauri::{AppHandle, Manager};
use tauri_plugin_aptabase::EventTracker;
use tryvial::try_fn;
use uuid::Uuid;
use velcro::vec;

use crate::{
	Notification, NotificationKind,
	bin1::{deserialize_modern_blueprint, deserialize_modern_factory},
	convert_json_patch_to_merge_patch,
	event_handling::{content_search::start_content_search, resource_overview::open_resource_overview},
	finish_task,
	general::{initialise_app, load_game_files, open_file, open_in_editor},
	model::{
		AppSettings, AppState, ContentSearchEvent, FileBrowserEvent, GameBrowserEntry, GameBrowserEvent,
		GameBrowserRequest, GlobalRequest, Hash, Request, SearchFilter, SettingsEvent, ToolEvent, ToolRequest
	},
	ores_repo::UnlockableItem,
	send_notification, send_request, start_task
};

#[try_fn]
#[context("Couldn't handle tool event")]
pub async fn handle_tool_event(app: &AppHandle, event: ToolEvent) -> Result<()> {
	let app_settings = app.state::<ArcSwap<AppSettings>>();
	let app_state = app.state::<AppState>();

	match event {
		ToolEvent::FileBrowser(event) => match event {
			FileBrowserEvent::Select { path } => {
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
									factory: "[assembly:/something.entity].pc_entitytemplate".parse()?,
									blueprint: "[assembly:/something.entity].pc_entityblueprint".parse()?,
									root_entity: 0xfffffffffffffffe.into(),
									entities: velcro::map_iter! {
										EntityID::from(0xfffffffffffffffe): SubEntity {
											parent: None,
											name: "Scene".into(),
											factory: ResourceReference{resource:"[modules:/zspatialentity.class].pc_entitytype".parse().unwrap(),flags:Default::default()},
											blueprint: "[modules:/zspatialentity.class].pc_entityblueprint".parse().unwrap(),
											editor_only: Default::default(),
											properties: Default::default(),
											platform_specific_properties: Default::default(),
											events: Default::default(),
											input_copying: Default::default(),
											output_copying: Default::default(),
											property_aliases: Default::default(),
											exposed_entities: Default::default(),
											exposed_interfaces: Default::default(),
											subsets: Default::default()
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
									quickentity_version: 3.2,
									extra_factory_references: vec![],
									extra_blueprint_references: vec![],
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

			FileBrowserEvent::Delete { path } => {
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

				if let Some(game) = app_state.game.load().as_ref() {
					match extension.as_ref() {
						"entity.json" => {
							let mut entity: Entity = from_slice(&fs::read(&path).context("Couldn't read file")?)
								.context("Invalid entity")?;

							// Normalise comments to form used by GlacierKit (single comment for each entity)
							let mut comments: Vec<CommentEntity> = vec![];
							for comment in entity.comments {
								if let Some(x) = comments.iter_mut().find(|x| x.parent == comment.parent) {
									x.text = eco_format!("{}\n\n{}", x.text, comment.text);
								} else {
									comments.push(CommentEntity {
										parent: comment.parent,
										name: "Notes".into(),
										text: comment.text
									});
								}
							}
							entity.comments = vec![]; // we don't need them here, since they get erased by the conversion to RT anyway

							let (fac, fac_meta, blu, blu_meta) = convert_to_game(&entity, game.version())
								.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

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
							let patch: Patch = from_slice(&fs::read(&path).context("Couldn't read file")?)
								.context("Invalid entity")?;

							let mut entity = (*game.extract_entity(patch.factory)?).to_owned();

							let base = entity.to_owned();

							apply_patch(&mut entity, patch, |_| {})
								.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

							// Normalise comments to form used by GlacierKit (single comment for each entity)
							let mut comments: Vec<CommentEntity> = vec![];
							for comment in entity.comments {
								if let Some(x) = comments.iter_mut().find(|x| x.parent == comment.parent) {
									x.text = eco_format!("{}\n\n{}", x.text, comment.text);
								} else {
									comments.push(CommentEntity {
										parent: comment.parent,
										name: "Notes".into(),
										text: comment.text
									});
								}
							}
							entity.comments = vec![];

							let (fac, fac_meta, blu, blu_meta) = convert_to_game(&entity, game.version())
								.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

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
						}

						_ => {
							Err(anyhow!("Can't normalise non-QN files"))?;
							panic!();
						}
					}
				} else {
					send_notification(
						app,
						Notification {
							kind: NotificationKind::Error,
							title: "No game selected".into(),
							subtitle: "You can't normalise QuickEntity files without a copy of the game selected."
								.into()
						}
					)?;
				}

				finish_task(app, task)?;
			}

			FileBrowserEvent::ConvertEntityToPatch { path } => {
				if let Some(game) = app_state.game.load().as_ref() {
					let mut entity: Entity =
						from_slice(&fs::read(&path).context("Couldn't read file")?).context("Invalid entity")?;

					// Normalise comments to form used by GlacierKit (single comment for each entity)
					let mut comments: Vec<CommentEntity> = vec![];
					for comment in entity.comments {
						if let Some(x) = comments.iter_mut().find(|x| x.parent == comment.parent) {
							x.text = eco_format!("{}\n\n{}", x.text, comment.text);
						} else {
							comments.push(CommentEntity {
								parent: comment.parent,
								name: "Notes".into(),
								text: comment.text
							});
						}
					}
					entity.comments = comments;

					// `extract_entity` is not used here because the entity needs to be extracted in non-lossless mode to avoid meaningless `scale`-removing patch operations being added.
					let (temp_meta, temp_data) = game.extract_latest_resource(entity.factory)?;

					let factory = deserialize_modern_factory(game.version(), &temp_data)?;

					let blueprint_hash = temp_meta
						.core_info
						.references
						.get(factory.blueprint_index_in_resource_header as usize)
						.context("Blueprint referenced in factory does not exist in dependencies")?
						.resource;

					let (tblu_meta, tblu_data) = game.extract_latest_resource(blueprint_hash)?;

					let blueprint = deserialize_modern_blueprint(game.version(), &tblu_data)?;

					let base = convert_to_qn(&factory, &temp_meta.core_info, &blueprint, &tblu_meta.core_info, false)
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

				if let Some(game) = app_state.game.load().as_ref() {
					let mut entity = (*game.extract_entity(patch.factory)?).to_owned();

					apply_patch(&mut entity, patch, |_| {}).map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

					// Normalise comments to form used by GlacierKit (single comment for each entity)
					let mut comments: Vec<CommentEntity> = vec![];
					for comment in entity.comments {
						if let Some(x) = comments.iter_mut().find(|x| x.parent == comment.parent) {
							x.text = eco_format!("{}\n\n{}", x.text, comment.text);
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
					if let Some(game) = app_state.game.load().as_ref() {
						let mut current = to_value(
							game.repository()
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
				if let Some(game) = app_state.game.load().as_ref() {
					let mut current = to_value(
						game.repository()
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
					if let Some(game) = app_state.game.load().as_ref() {
						let mut current = to_value(
							from_str::<Vec<UnlockableItem>>(&parse_json_ores(
								&game
									.extract_latest_resource("0057C2C3941115CA".parse::<RuntimeID>()?)?
									.1
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
				if let Some(game) = app_state.game.load().as_ref() {
					let mut current = to_value(
						from_str::<Vec<UnlockableItem>>(&parse_json_ores(
							&game
								.extract_latest_resource("0057C2C3941115CA".parse::<RuntimeID>()?)?
								.1
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
			GameBrowserEvent::Select { resource } => {
				open_resource_overview(app, resource.0).await?;
			}

			GameBrowserEvent::Search { query, filter } => {
				let task = start_task(app, format!("Searching game files for {}", query))?;

				if let Some(game) = app_state.game.load().as_ref() {
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

					send_request(
						app,
						Request::Tool(ToolRequest::GameBrowser(GameBrowserRequest::NewTree {
							game_description: format!(
								"{} ({})",
								match game.version() {
									GameVersion::H1 => "HITMAN™",
									GameVersion::H2 => "HITMAN 2",
									GameVersion::H3 => "HITMAN 3"
								},
								game.platform()
							),
							entries: {
								if matches!(filter, SearchFilter::All) {
									game.all_resources()
										.par_bridge()
										.filter(|&id| {
											let mut s = if let Some(info) = id.get_info() {
												format!(
													"{}{}{}.{}",
													info.path.as_deref().unwrap_or(""),
													info.hint.as_deref().unwrap_or(""),
													id.to_hash(),
													info.resource_type
												)
											} else {
												format!("{}.{}", id.to_hash(), game.resource_type(id).unwrap())
											};
											s.make_ascii_lowercase();
											query_terms.iter().all(|&y| s.contains(y))
										})
										.map(|id| GameBrowserEntry {
											hash: Hash(id),
											path: id.get_info().and_then(|i| i.path),
											hint: id.get_info().and_then(|i| i.hint),
											filetype: game.resource_type(id).unwrap(),
											partition: {
												let rrid = RuntimeResourceID::from(id);

												let partition = game
													.partition_manager()
													.partitions
													.iter()
													.find(|x| x.contains(&rrid))
													.unwrap();

												(
													partition.partition_info().id.to_string(),
													partition
														.partition_info()
														.name
														.to_owned()
														.unwrap_or("<unnamed>".into())
												)
											}
										})
										.collect()
								} else {
									game.all_resources()
										.par_bridge()
										.filter(|&id| {
											let ty = game.resource_type(id).unwrap();
											filter_includes.iter().any(|&x| ty == x)
										})
										.filter(|&id| {
											let mut s = if let Some(info) = id.get_info() {
												format!(
													"{}{}{}.{}",
													info.path.as_deref().unwrap_or(""),
													info.hint.as_deref().unwrap_or(""),
													id.to_hash(),
													info.resource_type
												)
											} else {
												format!("{}.{}", id.to_hash(), game.resource_type(id).unwrap())
											};
											s.make_ascii_lowercase();
											query_terms.iter().all(|&y| s.contains(y))
										})
										.map(|id| GameBrowserEntry {
											hash: Hash(id),
											path: id.get_info().and_then(|i| i.path),
											hint: id.get_info().and_then(|i| i.hint),
											filetype: game.resource_type(id).unwrap(),
											partition: {
												let rrid = RuntimeResourceID::from(id);

												let partition = game
													.partition_manager()
													.partitions
													.iter()
													.find(|x| x.contains(&rrid))
													.unwrap();

												(
													partition.partition_info().id.to_string(),
													partition
														.partition_info()
														.name
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

				finish_task(app, task)?;
			}

			GameBrowserEvent::OpenInEditor { resource } => {
				if let Some(game) = app_state.game.load().as_ref() {
					open_in_editor(app, game, resource.0).await?;
				}
			}
		},

		ToolEvent::Settings(event) => match event {
			SettingsEvent::Initialise => {
				initialise_app(app).await?;
			}

			SettingsEvent::ChangeGameInstall { path } => {
				let mut settings = (*app_settings.load_full()).to_owned();

				if path != settings.game_install {
					settings.game_install = path;
					fs::write(
						app.path()
							.app_data_dir()
							.context("Couldn't get app data dir")?
							.join("settings.json"),
						to_vec(&settings)?
					)?;
					app_settings.store(settings.into());

					load_game_files(app).await?;
				}
			}

			SettingsEvent::ChangeExtractModdedFiles { value } => {
				let mut settings = (*app_settings.load_full()).to_owned();
				settings.extract_modded_files = value;
				fs::write(
					app.path()
						.app_data_dir()
						.context("Couldn't get app data dir")?
						.join("settings.json"),
					to_vec(&settings)?
				)?;
				app_settings.store(settings.into());
			}

			SettingsEvent::ChangeColourblind { value } => {
				let mut settings = (*app_settings.load_full()).to_owned();
				settings.colourblind_mode = value;
				fs::write(
					app.path()
						.app_data_dir()
						.context("Couldn't get app data dir")?
						.join("settings.json"),
					to_vec(&settings)?
				)?;
				app_settings.store(settings.into());
			}

			SettingsEvent::ChangeEditorConnection { value } => {
				let mut settings = (*app_settings.load_full()).to_owned();
				settings.editor_connection = value;

				if !value && app_state.editor_connection.is_connected().await {
					app_state.editor_connection.disconnect().await?;
				}

				fs::write(
					app.path()
						.app_data_dir()
						.context("Couldn't get app data dir")?
						.join("settings.json"),
					to_vec(&settings)?
				)?;
				app_settings.store(settings.into());
			}

			SettingsEvent::ChangeCustomPaths { value } => {
				if let Some(project) = app_state.project.load().as_ref() {
					app.track_event("Edit custom paths list manually", None).unwrap();

					let mut settings = (*project.settings.load_full()).to_owned();
					settings.custom_paths = value;
					fs::write(project.path.join("project.json"), to_vec(&settings)?)?;
					project.settings.store(settings.into());
				}
			}
		},

		ToolEvent::ContentSearch(event) => match event {
			ContentSearchEvent::Search {
				query,
				resource_types,
				use_qn_format,
				partitions_to_search
			} => {
				start_content_search(app, query, resource_types, use_qn_format, partitions_to_search).await?;
			}
		}
	}
}
