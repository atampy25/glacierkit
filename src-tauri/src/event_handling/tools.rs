use std::fs;

use anyhow::{Context, Result, anyhow};
use ecow::{EcoString, eco_format};
use fn_error_context::context;
use glacier_commons::{
	game::GlacierGame,
	metadata::{ResourceReference, ResourceType, RuntimeID},
	resource_type, rid
};
use indexmap::IndexMap;
use itertools::Itertools;
use quickentity_rs::{
	apply_patch,
	entity::{CommentEntity, Entity, EntityID, SubEntity, SubType},
	generate_patch,
	patch::Patch
};
use rayon::iter::{IntoParallelRefMutIterator, ParallelBridge, ParallelIterator};
use serde_json::{Value, from_slice, from_str, from_value, to_string, to_value, to_vec};
use tauri::{AppHandle, Manager};
use tauri_plugin_aptabase::EventTracker;
use tryvial::try_fn;
use uuid::Uuid;
use velcro::vec;

use crate::{
	Notification, NotificationKind, convert_json_patch_to_merge_patch,
	event_handling::{content_search::start_content_search, resource_overview::open_resource_overview},
	finish_task,
	game::{custom_game_install, valid_game_path},
	general::{EMPTY_ID, REPO_ID, initialise_app, load_game_files, open_file, open_in_editor},
	model::{
		AppSettings, AppState, ContentSearchEvent, FileBrowserEvent, GameBrowserEntry, GameBrowserEvent,
		GameBrowserRequest, GlobalRequest, Hash, Request, SearchFilter, SearchSort, SettingsEvent, SettingsRequest,
		ToolEvent, ToolRequest
	},
	ores_repo::UnlockableItem,
	send_notification, send_request, start_task
};

#[try_fn]
#[context("Couldn't handle tool event")]
pub async fn handle_tool_event(app: &AppHandle, event: ToolEvent) -> Result<()> {
	let app_settings = app.state::<AppSettings>();
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
									factory: EMPTY_ID,
									blueprint: EMPTY_ID,
									root_entity: 0xfffffffffffffffe.into(),
									entities: velcro::map_iter! {
										EntityID::from(0xfffffffffffffffe): SubEntity {
											parent: None,
											name: "Scene".into(),
											factory: ResourceReference {
												resource: if let Some(game) = app_state.game.load().as_ref() && game.version() == GlacierGame::FL {
													rid!("[modules:/zspatialentity.class].entitytype")
												} else {
													rid!("[modules:/zspatialentity.class].pc_entitytype")
												},
												flags: Default::default()
											},
											blueprint: if let Some(game) = app_state.game.load().as_ref() && game.version() == GlacierGame::FL {
												rid!("[modules:/zspatialentity.class].entityblueprint")
											} else {
												rid!("[modules:/zspatialentity.class].pc_entityblueprint")
											},
											..Default::default()
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

							let mut reconverted = match game.version() {
								GlacierGame::H1 => {
									let (fac, fac_meta, blu, blu_meta): (
										glacier_bin1::game::h1::STemplateEntity,
										_,
										_,
										_
									) = entity.to_game().map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;
									Entity::from_game(&fac, &fac_meta, &blu, &blu_meta, false)
										.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?
								}

								GlacierGame::H2 => {
									let (fac, fac_meta, blu, blu_meta): (
										glacier_bin1::game::h2::STemplateEntityFactory,
										_,
										_,
										_
									) = entity.to_game().map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;
									Entity::from_game(&fac, &fac_meta, &blu, &blu_meta, false)
										.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?
								}

								GlacierGame::H3 => {
									let (fac, fac_meta, blu, blu_meta): (
										glacier_bin1::game::h3::STemplateEntityFactory,
										_,
										_,
										_
									) = entity.to_game().map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;
									Entity::from_game(&fac, &fac_meta, &blu, &blu_meta, false)
										.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?
								}

								GlacierGame::FL => {
									let (fac, fac_meta, blu, blu_meta): (
										glacier_bin1::game::fl::STemplateEntityFactory,
										_,
										_,
										_
									) = entity.to_game().map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;
									Entity::from_game(&fac, &fac_meta, &blu, &blu_meta, false)
										.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?
								}
							};

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

							let mut reconverted = match game.version() {
								GlacierGame::H1 => {
									let (fac, fac_meta, blu, blu_meta): (
										glacier_bin1::game::h1::STemplateEntity,
										_,
										_,
										_
									) = entity.to_game().map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;
									Entity::from_game(&fac, &fac_meta, &blu, &blu_meta, false)
										.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?
								}

								GlacierGame::H2 => {
									let (fac, fac_meta, blu, blu_meta): (
										glacier_bin1::game::h2::STemplateEntityFactory,
										_,
										_,
										_
									) = entity.to_game().map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;
									Entity::from_game(&fac, &fac_meta, &blu, &blu_meta, false)
										.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?
								}

								GlacierGame::H3 => {
									let (fac, fac_meta, blu, blu_meta): (
										glacier_bin1::game::h3::STemplateEntityFactory,
										_,
										_,
										_
									) = entity.to_game().map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;
									Entity::from_game(&fac, &fac_meta, &blu, &blu_meta, false)
										.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?
								}

								GlacierGame::FL => {
									let (fac, fac_meta, blu, blu_meta): (
										glacier_bin1::game::fl::STemplateEntityFactory,
										_,
										_,
										_
									) = entity.to_game().map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;
									Entity::from_game(&fac, &fac_meta, &blu, &blu_meta, false)
										.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?
								}
							};

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

					let base = game.extract_entity(entity.factory)?;

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
				if let Some(game) = app_state.game.load().as_ref()
					&& let Some(repo) = game.repository()
				{
					if game.resource_type(
						from_slice::<Value>(&fs::read(&path).context("Couldn't read file")?)
							.context("Invalid JSON")?
							.get("id")
							.context("Patch had no id key")?
							.as_str()
							.context("id key was not string")?
							.parse::<RuntimeID>()
							.context("id key was invalid")?
					) == Some(resource_type!("REPO"))
					{
						let mut current = to_value(
							repo.iter()
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
								title: "Not a repository patch".into(),
								subtitle: "This patch is for a different type of file, so it can't be converted to a \
								           repository.json file."
									.into()
							}
						)?;
					}
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

			FileBrowserEvent::ConvertRepoPatchToJsonPatch { path } => {
				if let Some(game) = app_state.game.load().as_ref()
					&& let Some(repo) = game.repository()
				{
					let mut current = to_value(
						repo.iter()
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
							id: REPO_ID
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
				if let Some(game) = app_state.game.load().as_ref() {
					if from_slice::<Value>(&fs::read(&path).context("Couldn't read file")?)
						.context("Invalid JSON")?
						.get("file")
						.context("Patch had no file key")?
						.as_str()
						.context("File key was not string")?
						.parse::<RuntimeID>()
						.context("File key was invalid")?
						== game.unlockables_id()
					{
						let mut current = to_value(
							from_str::<Vec<UnlockableItem>>(&glacier_bin1::deserialize::<EcoString>(
								&game.extract_latest_resource(game.unlockables_id())?.1
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
								title: "Not an unlockables patch".into(),
								subtitle: "This patch is for a different type of file, so it can't be converted to a \
								           unlockables.json file."
									.into()
							}
						)?;
					}
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

			FileBrowserEvent::ConvertUnlockablesPatchToJsonPatch { path } => {
				if let Some(game) = app_state.game.load().as_ref() {
					let mut current = to_value(
						from_str::<Vec<UnlockableItem>>(&glacier_bin1::deserialize::<EcoString>(
							&game.extract_latest_resource(game.unlockables_id())?.1
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
							id: game.unlockables_id()
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

			GameBrowserEvent::Search { query, filter, sort } => {
				let task = start_task(app, format!("Searching game files for {}", query))?;

				if let Some(game) = app_state.game.load().as_ref() {
					let filter_includes: &[ResourceType] = match filter {
						SearchFilter::All => &[],
						SearchFilter::Templates => &[
							resource_type!("TEMP"),
							resource_type!("CPPT"),
							resource_type!("ASET"),
							resource_type!("UICT"),
							resource_type!("MATT"),
							resource_type!("WSWT"),
							resource_type!("ECPT"),
							resource_type!("AIBX"),
							resource_type!("WSGT")
						],
						SearchFilter::Classes => &[resource_type!("CPPT")],
						SearchFilter::Models => {
							&[resource_type!("PRIM"), resource_type!("BORG"), resource_type!("ALOC")]
						}
						SearchFilter::Textures => &[resource_type!("TEXT"), resource_type!("TEXD")],
						SearchFilter::Sound => &[
							resource_type!("WBNK"),
							resource_type!("WWFX"),
							resource_type!("WWEV"),
							resource_type!("WWES"),
							resource_type!("WWEM")
						]
					};

					let query_terms = query.split(' ').collect_vec();

					send_request(
						app,
						Request::Tool(ToolRequest::GameBrowser(GameBrowserRequest::NewTree {
							game_description: format!("{} ({})", game.version(), game.platform()),
							entries: {
								let mut entries: Vec<_> = if matches!(filter, SearchFilter::All) {
									game.all_resources()
										.par_bridge()
										.map(|id| (id, id.get_info(), game.resource_type(id).unwrap()))
										.filter(|(id, info, resource_type)| {
											let mut s = if let Some(info) = &info {
												format!(
													"{}{}{}.{}",
													info.path.as_deref().unwrap_or(""),
													info.hint.as_deref().unwrap_or(""),
													id.to_hash(),
													resource_type
												)
											} else {
												format!("{}.{}", id.to_hash(), resource_type)
											};
											s.make_ascii_lowercase();
											query_terms.iter().all(|&y| s.contains(y))
										})
										.map(|(id, info, resource_type)| {
											let (path, hint) = if let Some(info) = info {
												(info.path, info.hint)
											} else {
												(None, None)
											};

											let rrid = game.to_rrid(id);

											let partition = game
												.partition_manager()
												.partitions
												.iter()
												.find(|x| x.contains(&rrid))
												.unwrap();

											(
												partition,
												GameBrowserEntry {
													hash: Hash(id),
													path,
													hint,
													resource_type,
													partition: {
														(
															partition.partition_info().id.to_string(),
															partition
																.partition_info()
																.name
																.to_owned()
																.unwrap_or("<unnamed>".into())
														)
													},
													order: None
												}
											)
										})
										.collect()
								} else {
									game.all_resources()
										.par_bridge()
										.map(|id| (id, game.resource_type(id).unwrap()))
										.filter_map(|(id, resource_type)| {
											filter_includes.contains(&resource_type).then_some((
												id,
												id.get_info(),
												resource_type
											))
										})
										.filter(|(id, info, resource_type)| {
											let mut s = if let Some(info) = &info {
												format!(
													"{}{}{}.{}",
													info.path.as_deref().unwrap_or(""),
													info.hint.as_deref().unwrap_or(""),
													id.to_hash(),
													resource_type
												)
											} else {
												format!("{}.{}", id.to_hash(), resource_type)
											};
											s.make_ascii_lowercase();
											query_terms.iter().all(|&y| s.contains(y))
										})
										.map(|(id, info, resource_type)| {
											let (path, hint) = if let Some(info) = info {
												(info.path, info.hint)
											} else {
												(None, None)
											};

											let rrid = game.to_rrid(id);

											let partition = game
												.partition_manager()
												.partitions
												.iter()
												.find(|x| x.contains(&rrid))
												.unwrap();

											(
												partition,
												GameBrowserEntry {
													hash: Hash(id),
													path,
													hint,
													resource_type,
													partition: {
														(
															partition.partition_info().id.to_string(),
															partition
																.partition_info()
																.name
																.to_owned()
																.unwrap_or("<unnamed>".into())
														)
													},
													order: None
												}
											)
										})
										.collect()
								};

								if let Some((sort, descending)) = sort {
									match sort {
										SearchSort::Size => {
											entries.par_iter_mut().try_for_each(|(partition, entry)| {
												let size =
													partition.get_resource_info(&game.to_rrid(entry.hash.0))?.size();

												entry.order =
													Some(if descending { u32::MAX - size } else { size } as usize);

												anyhow::Ok(())
											})?;
										}
									}
								}

								entries.into_iter().map(|(_, entry)| entry).collect()
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
				if path != app_settings.settings().game_path {
					if let Some(path) = &path
						&& !valid_game_path(path)
					{
						send_notification(
							app,
							Notification {
								kind: NotificationKind::Error,
								title: "Invalid game path".into(),
								subtitle: "Make sure to select the Retail path containing the game executable.".into()
							}
						)?;

						return Ok(());
					}

					if let Some(path) = &path
						&& !app_state.game_installs.iter().any(|x| x.path == *path)
					{
						let mut custom_game_paths = app_settings.settings().custom_game_paths.to_owned();
						if !custom_game_paths.contains(path) {
							custom_game_paths.push(path.to_owned());
							app_settings.set_custom_game_paths(custom_game_paths)?;
						}
					}

					app_settings.set_game_path(path)?;

					send_request(
						app,
						Request::Tool(ToolRequest::Settings(SettingsRequest::Initialise {
							game_installs: app_state
								.game_installs
								.iter()
								.cloned()
								.map(Ok)
								.chain(
									app_settings
										.settings()
										.custom_game_paths
										.iter()
										.map(custom_game_install)
								)
								.collect::<Result<_>>()?,
							settings: (**app_settings.settings()).to_owned()
						}))
					)?;

					load_game_files(app).await?;
				}
			}

			SettingsEvent::RemoveCustomGamePath { path } => {
				let mut custom_game_paths = app_settings.settings().custom_game_paths.to_owned();
				custom_game_paths.retain(|x| *x != path);
				app_settings.set_custom_game_paths(custom_game_paths)?;

				send_request(
					app,
					Request::Tool(ToolRequest::Settings(SettingsRequest::Initialise {
						game_installs: app_state
							.game_installs
							.iter()
							.cloned()
							.map(Ok)
							.chain(
								app_settings
									.settings()
									.custom_game_paths
									.iter()
									.map(custom_game_install)
							)
							.collect::<Result<_>>()?,
						settings: (**app_settings.settings()).to_owned()
					}))
				)?;
			}

			SettingsEvent::ChangeExtractModdedFiles { value } => {
				app_settings.set_extract_modded_files(value)?;
			}

			SettingsEvent::ChangeColourblindMode { value } => {
				app_settings.set_colourblind_mode(value)?;
			}

			SettingsEvent::ChangeEditorConnection { value } => {
				app_settings.set_editor_connection(value)?;

				if !value && app_state.editor_connection.is_connected().await {
					app_state.editor_connection.disconnect().await?;
				}
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
				partitions_to_search
			} => {
				start_content_search(app, query, resource_types, partitions_to_search).await?;
			}
		}
	}
}
