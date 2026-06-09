use std::{fs, path::Path, sync::atomic::Ordering, time::Duration};

use anyhow::{Context, Result, anyhow};
use arc_swap::ArcSwap;
use ecow::eco_format;
use fn_error_context::context;
use hitman_commons::{hash_list::HASH_LIST, metadata::RuntimeID, rid};
use hitman_formats::ores::parse_json_ores;
use indexmap::IndexMap;
use itertools::Itertools;
use lazy_regex::regex_captures;
use quickentity_rs::{
	apply_patch,
	entity::{CommentEntity, Entity},
	patch::Patch
};
use regex::Regex;
use serde_json::{Value, from_slice, from_str, from_value, to_value};
use tauri::{AppHandle, Manager, async_runtime};
use tauri_plugin_aptabase::EventTracker;
use tokio::net::TcpStream;
use tryvial::try_fn;
use uuid::Uuid;
use velcro::vec;

use crate::{
	HASH_LIST_ENDPOINT, HASH_LIST_VERSION_ENDPOINT, Notification, NotificationKind, TONYTOOLS_HASH_LIST_ENDPOINT,
	TONYTOOLS_HASH_LIST_VERSION_ENDPOINT,
	event_handling::{entity::monaco::ENUMS, resource_overview::initialise_resource_overview},
	finish_task,
	game::Game,
	model::{
		AppSettings, AppState, ContentSearchRequest, EditorData, EditorState, EditorType, FileBrowserRequest,
		GameBrowserRequest, GlobalRequest, JsonPatchType, Request, SettingsRequest, TabRequest, TabRequestData,
		TextFileType, ToolRequest
	},
	ores_repo::{RepositoryItem, UnlockableItem},
	send_notification, send_request, start_task
};

pub const EMPTY_ID: RuntimeID = rid!("0000000000000000");

pub const REPO_ID: RuntimeID = rid!("[assembly:/repository/pro.repo].pc_repo");

pub const UNLOCKABLES_ID: RuntimeID =
	rid!("[assembly:/_pro/online/default/offlineconfig/config.unlockables].pc_unlockables");

/// Get a filename from a path as it would appear in the game file browser.
pub fn get_name(path: &str) -> String {
	if path.starts_with("[") {
		if let Some((_, inner, params, filetype)) = regex_captures!(r"^\[(.*)\](?:\((.*)\))?(\..*)?$", path) {
			let params = (!params.is_empty()).then_some(params);
			let filetype = (!filetype.is_empty()).then_some(filetype);

			let inner_name = get_name(inner);
			let type_str = filetype.map(|ft| ft.replace(".pc_", ".")).unwrap_or_default();
			let name = format!(
				"[{}]{}{}",
				inner_name,
				params.map(|p| format!("({})", p)).unwrap_or_default(),
				type_str
			);

			if name.ends_with(&format!("]{}{}", type_str, type_str)) {
				if let Some((_, name)) = regex_captures!(r"^\[(.*)\]\..*$", &name) {
					return name.into();
				}
			} else if Regex::new(&format!(
				r"^\[.*{}\]\(.*\){}$",
				regex::escape(&type_str),
				regex::escape(&type_str)
			))
			.unwrap()
			.is_match(&name)
			{
				if let Some((_, name)) = regex_captures!(r"^\[(.*)\]\(.*\)..*$", &name) {
					return format!("[{}]({})", name, params.unwrap_or(""));
				}
			} else if [".wwisebank", ".gfx", ".wes"]
				.iter()
				.any(|a| name.ends_with(&format!("]{})", a)))
			{
				if let Some((_, name)) = regex_captures!(r"^\[(.*)\]\..*$", &name) {
					return name.into();
				}
			} else if [".class", ".aspect", ".brick", ".entity", ".entitytemplate"]
				.iter()
				.any(|ty| name.ends_with(&format!("{}].entitytype", ty)))
			{
				if let Some((_, name)) = regex_captures!(r"^\[(.*)\]\..*$", &name) {
					return name.into();
				}
			} else if [".class", ".aspect", ".brick", ".entity", ".entitytemplate"]
				.iter()
				.any(|ty| name.ends_with(&format!("{}].entityblueprint", ty)))
				&& let Some((_, name)) = regex_captures!(r"^\[(.*)\]\..*$", &name)
			{
				return format!("{} (blueprint)", name);
			}

			name
		} else {
			path.into()
		}
	} else {
		path.split('/').next_back().unwrap_or("").into()
	}
}

#[try_fn]
#[context("Couldn't open file")]
pub async fn open_file(app: &AppHandle, path: impl AsRef<Path>) -> Result<()> {
	let app_state = app.state::<AppState>();

	let path = path.as_ref();

	let task = start_task(
		app,
		format!(
			"Opening {}",
			path.file_name().context("No file name")?.to_string_lossy()
		)
	)?;

	let mut existing = None;
	for id in app_state.editor_states.keys().await {
		if let Some(editor) = app_state.editor_states.get(&id).await
			&& let Some(file) = editor.file.as_ref()
			&& file == path
		{
			existing = Some(id);
		}
	}

	if let Some(existing) = existing {
		send_request(
			app,
			Request::Tab(TabRequest {
				tab: existing,
				data: TabRequestData::Select
			})
		)?;
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

				app_state
					.editor_states
					.insert(
						id.to_owned(),
						EditorState {
							file: Some(path.to_owned()),
							data: EditorData::QNEntity {
								entity: entity.into(),
								settings: Default::default()
							},
							..Default::default()
						}
					)
					.await;

				send_request(
					app,
					Request::Tab(TabRequest {
						tab: id,
						data: TabRequestData::Create {
							name: path.file_name().context("No file name")?.to_string_lossy().into(),
							editor_type: EditorType::QNEntity
						}
					})
				)?;
			}

			"entity.patch.json" => {
				let id = Uuid::new_v4();

				if let Some(game) = app_state.game.load().as_ref() {
					let patch: Patch =
						from_slice(&fs::read(path).context("Couldn't read file")?).context("Invalid entity")?;

					let base = game.extract_entity(patch.factory)?;
					let mut entity = (*base).to_owned();

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

					app_state
						.editor_states
						.insert(
							id.to_owned(),
							EditorState {
								file: Some(path.to_owned()),
								data: EditorData::QNPatch {
									base,
									current: entity.into(),
									settings: Default::default()
								},
								..Default::default()
							}
						)
						.await;

					send_request(
						app,
						Request::Tab(TabRequest {
							tab: id,
							data: TabRequestData::Create {
								name: path.file_name().context("No file name")?.to_string_lossy().into(),
								editor_type: EditorType::QNPatch
							}
						})
					)?;
				} else {
					send_request(
						app,
						Request::Tool(ToolRequest::FileBrowser(FileBrowserRequest::Select { path: None }))
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

				app_state
					.editor_states
					.insert(
						id.to_owned(),
						EditorState {
							file: Some(path.to_owned()),
							data: EditorData::Text {
								content: fs::read_to_string(path)
									.context("Couldn't read file")?
									.replace("\r\n", "\n"),
								file_type: file_type.to_owned()
							},
							..Default::default()
						}
					)
					.await;

				send_request(
					app,
					Request::Tab(TabRequest {
						tab: id,
						data: TabRequestData::Create {
							name: path.file_name().context("No file name")?.to_string_lossy().into(),
							editor_type: EditorType::Text { file_type }
						}
					})
				)?;
			}

			"txt" => {
				let id = Uuid::new_v4();

				app_state
					.editor_states
					.insert(
						id.to_owned(),
						EditorState {
							file: Some(path.to_owned()),
							data: EditorData::Text {
								content: fs::read_to_string(path)
									.context("Couldn't read file")?
									.replace("\r\n", "\n"),
								file_type: TextFileType::PlainText
							},
							..Default::default()
						}
					)
					.await;

				send_request(
					app,
					Request::Tab(TabRequest {
						tab: id,
						data: TabRequestData::Create {
							name: path.file_name().context("No file name")?.to_string_lossy().into(),
							editor_type: EditorType::Text {
								file_type: TextFileType::PlainText
							}
						}
					})
				)?;
			}

			"md" => {
				let id = Uuid::new_v4();

				app_state
					.editor_states
					.insert(
						id.to_owned(),
						EditorState {
							file: Some(path.to_owned()),
							data: EditorData::Text {
								content: fs::read_to_string(path)
									.context("Couldn't read file")?
									.replace("\r\n", "\n"),
								file_type: TextFileType::Markdown
							},
							..Default::default()
						}
					)
					.await;

				send_request(
					app,
					Request::Tab(TabRequest {
						tab: id,
						data: TabRequestData::Create {
							name: path.file_name().context("No file name")?.to_string_lossy().into(),
							editor_type: EditorType::Text {
								file_type: TextFileType::Markdown
							}
						}
					})
				)?;
			}

			"repository.json" => {
				let id = Uuid::new_v4();

				if let Some(game) = app_state.game.load().as_ref() {
					let mut repository = to_value(
						game.repository()
							.iter()
							.cloned()
							.map(|x| (x.id, x.data))
							.collect::<IndexMap<Uuid, IndexMap<String, Value>>>()
					)?;

					let base = to_value(game.repository())?;

					let patch: Value =
						from_slice(&fs::read(path).context("Couldn't read file")?).context("Invalid JSON")?;

					json_patch::merge(&mut repository, &patch);

					let repository = from_value::<IndexMap<Uuid, IndexMap<String, Value>>>(repository)?
						.into_iter()
						.map(|(id, data)| RepositoryItem { id, data })
						.collect();

					app_state
						.editor_states
						.insert(
							id.to_owned(),
							EditorState {
								file: Some(path.to_owned()),
								data: EditorData::RepositoryPatch {
									base: from_value(base)?,
									current: repository,
									patch_type: JsonPatchType::MergePatch
								},
								..Default::default()
							}
						)
						.await;

					send_request(
						app,
						Request::Tab(TabRequest {
							tab: id,
							data: TabRequestData::Create {
								name: path.file_name().context("No file name")?.to_string_lossy().into(),
								editor_type: EditorType::RepositoryPatch {
									patch_type: JsonPatchType::MergePatch
								}
							}
						})
					)?;
				} else {
					send_request(
						app,
						Request::Tool(ToolRequest::FileBrowser(FileBrowserRequest::Select { path: None }))
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

				if let Some(game) = app_state.game.load().as_ref() {
					let mut unlockables = to_value(
						from_str::<Vec<UnlockableItem>>(&parse_json_ores(
							&game.extract_latest_resource(UNLOCKABLES_ID)?.1
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

					let base = from_str::<Value>(&parse_json_ores(&game.extract_latest_resource(UNLOCKABLES_ID)?.1)?)?;

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

					app_state
						.editor_states
						.insert(
							id.to_owned(),
							EditorState {
								file: Some(path.to_owned()),
								data: EditorData::UnlockablesPatch {
									base: from_value(base)?,
									current: unlockables,
									patch_type: JsonPatchType::MergePatch
								},
								..Default::default()
							}
						)
						.await;

					send_request(
						app,
						Request::Tab(TabRequest {
							tab: id,
							data: TabRequestData::Create {
								name: path.file_name().context("No file name")?.to_string_lossy().into(),
								editor_type: EditorType::UnlockablesPatch {
									patch_type: JsonPatchType::MergePatch
								}
							}
						})
					)?;
				} else {
					send_request(
						app,
						Request::Tool(ToolRequest::FileBrowser(FileBrowserRequest::Select { path: None }))
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
						if let Some(game) = app_state.game.load().as_ref() {
							let mut repository = to_value(
								game.repository()
									.iter()
									.cloned()
									.map(|x| (x.id, x.data))
									.collect::<IndexMap<Uuid, IndexMap<String, Value>>>()
							)?;

							let base = to_value(game.repository())?;

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

							app_state
								.editor_states
								.insert(
									id.to_owned(),
									EditorState {
										file: Some(path.to_owned()),
										data: EditorData::RepositoryPatch {
											base: from_value(base)?,
											current: repository,
											patch_type: JsonPatchType::JsonPatch
										},
										..Default::default()
									}
								)
								.await;

							send_request(
								app,
								Request::Tab(TabRequest {
									tab: id,
									data: TabRequestData::Create {
										name: path.file_name().context("No file name")?.to_string_lossy().into(),
										editor_type: EditorType::RepositoryPatch {
											patch_type: JsonPatchType::JsonPatch
										}
									}
								})
							)?;
						} else {
							send_request(
								app,
								Request::Tool(ToolRequest::FileBrowser(FileBrowserRequest::Select { path: None }))
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
							.context("File key was not string")?
							.parse::<RuntimeID>()
							.context("File key was invalid")?
							== UNLOCKABLES_ID =>
					{
						let id = Uuid::new_v4();

						if let Some(game) = app_state.game.load().as_ref() {
							let mut unlockables = to_value(
								from_str::<Vec<UnlockableItem>>(&parse_json_ores(
									&game.extract_latest_resource(UNLOCKABLES_ID)?.1
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

							let base =
								from_str::<Value>(&parse_json_ores(&game.extract_latest_resource(UNLOCKABLES_ID)?.1)?)?;

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

							app_state
								.editor_states
								.insert(
									id.to_owned(),
									EditorState {
										file: Some(path.to_owned()),
										data: EditorData::UnlockablesPatch {
											base: from_value(base)?,
											current: unlockables,
											patch_type: JsonPatchType::JsonPatch
										},
										..Default::default()
									}
								)
								.await;

							send_request(
								app,
								Request::Tab(TabRequest {
									tab: id,
									data: TabRequestData::Create {
										name: path.file_name().context("No file name")?.to_string_lossy().into(),
										editor_type: EditorType::UnlockablesPatch {
											patch_type: JsonPatchType::JsonPatch
										}
									}
								})
							)?;
						} else {
							send_request(
								app,
								Request::Tool(ToolRequest::FileBrowser(FileBrowserRequest::Select { path: None }))
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
						app_state
							.editor_states
							.insert(
								id.to_owned(),
								EditorState {
									file: Some(path.to_owned()),
									data: EditorData::Text {
										content: fs::read_to_string(path)
											.context("Couldn't read file")?
											.replace("\r\n", "\n"),
										file_type: TextFileType::Json
									},
									..Default::default()
								}
							)
							.await;

						send_request(
							app,
							Request::Tab(TabRequest {
								tab: id,
								data: TabRequestData::Create {
									name: path.file_name().context("No file name")?.to_string_lossy().into(),
									editor_type: EditorType::Text {
										file_type: TextFileType::Json
									}
								}
							})
						)?;
					}
				}
			}

			"dlge.json" | "locr.json" | "rtlv.json" | "clng.json" | "ditl.json" | "material.json" | "contract.json" => {
				let id = Uuid::new_v4();

				app_state
					.editor_states
					.insert(
						id.to_owned(),
						EditorState {
							file: Some(path.to_owned()),
							data: EditorData::Text {
								content: fs::read_to_string(path)
									.context("Couldn't read file")?
									.replace("\r\n", "\n"),
								file_type: TextFileType::Json
							},
							..Default::default()
						}
					)
					.await;

				send_request(
					app,
					Request::Tab(TabRequest {
						tab: id,
						data: TabRequestData::Create {
							name: path.file_name().context("No file name")?.to_string_lossy().into(),
							editor_type: EditorType::Text {
								file_type: TextFileType::Json
							}
						}
					})
				)?;
			}

			_ => {
				// Unsupported extension

				let id = Uuid::new_v4();

				app_state
					.editor_states
					.insert(
						id.to_owned(),
						EditorState {
							file: Some(path.to_owned()),
							data: EditorData::Nil,
							..Default::default()
						}
					)
					.await;

				send_request(
					app,
					Request::Tab(TabRequest {
						tab: id,
						data: TabRequestData::Create {
							name: path.file_name().context("No file name")?.to_string_lossy().into(),
							editor_type: EditorType::Nil
						}
					})
				)?;
			}
		}
	}

	finish_task(app, task)?;
}

#[try_fn]
#[context("Couldn't initialise app")]
pub async fn initialise_app(app: &AppHandle) -> Result<()> {
	let app_settings = app.state::<ArcSwap<AppSettings>>();
	let app_state = app.state::<AppState>();

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
		Some(serde_json::json!({
			"game_installs": app_state.game_installs.len(),
			"extract_modded_files": app_settings.load().extract_modded_files,
			"colourblind_mode": app_settings.load().colourblind_mode,
			"editor_connection": app_settings.load().editor_connection,
			"selected_install": selected_install_info
		}))
	)
	.unwrap();

	send_request(
		app,
		Request::Tool(ToolRequest::Settings(SettingsRequest::Initialise {
			game_installs: app_state.game_installs.to_owned(),
			settings: (*app_settings.load_full()).to_owned()
		}))
	)?;

	let res = tokio::spawn({
		let app = app.clone();
		async move {
			let task = start_task(&app, "Acquiring latest hash list")?;

			let app_data_dir = app.path().app_data_dir().context("Couldn't get data dir")?;

			let _ = fs::read(app_data_dir.join("hash_list.sml"))
				.ok()
				.and_then(|x| HASH_LIST.load_compressed(&x).ok());

			let current_version = HASH_LIST.version.load(Ordering::SeqCst);

			if let Ok(data) = reqwest::get(HASH_LIST_VERSION_ENDPOINT).await
				&& let Ok(data) = data.text().await
			{
				let new_version = data
					.trim()
					.parse::<u32>()
					.context("Online hash list version wasn't a number")?;

				if current_version < new_version
					&& let Ok(data) = reqwest::get(HASH_LIST_ENDPOINT).await
					&& let Ok(data) = data.bytes().await
				{
					HASH_LIST.load_compressed(&data)?;

					fs::write(app_data_dir.join("hash_list.sml"), data)?;
				}
			}

			let app_state = app.state::<AppState>();
			let current_version = app_state
				.tonytools_hash_list
				.load()
				.as_ref()
				.map(|x| x.version)
				.unwrap_or(0);

			if let Ok(data) = reqwest::get(TONYTOOLS_HASH_LIST_VERSION_ENDPOINT).await
				&& let Ok(data) = data.text().await
			{
				let new_version = from_str::<Value>(&data)
					.context("Couldn't parse online version data as JSON")?
					.get("version")
					.context("No version key in online version data")?
					.as_u64()
					.context("Online hash list version wasn't a number")? as u32;

				if current_version < new_version
					&& let Ok(data) = reqwest::get(TONYTOOLS_HASH_LIST_ENDPOINT).await
					&& let Ok(data) = data.bytes().await
				{
					let tonytools_hash_list =
						tonytools::hashlist::HashList::load(&data).map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

					fs::write(app_data_dir.join("tonytools_hash_list.hmla"), data)?;

					app_state.tonytools_hash_list.store(Some(tonytools_hash_list.into()));
				}
			}

			finish_task(&app, task)?;

			anyhow::Ok(())
		}
	});

	load_game_files(app).await?;
	res.await??;

	send_request(
		app,
		Request::Global(GlobalRequest::SetEnums {
			enums: ENUMS
				.iter()
				.map(|(&x, y)| (x.to_owned(), y.iter().map(|&z| z.to_owned()).collect()))
				.collect()
		})
	)?;

	if app
		.path()
		.app_log_dir()
		.context("Couldn't get log dir")?
		.join("..")
		.join("last_panic.txt")
		.exists()
	{
		send_request(app, Request::Global(GlobalRequest::RequestLastPanicUpload))?;
	}

	if let Ok(req) = reqwest::get("https://hitman-resources.netlify.app/glacierkit/dynamics.json").await {
		send_request(
			app,
			Request::Global(GlobalRequest::InitialiseDynamics {
				dynamics: req.json().await.context("Couldn't deserialise dynamics response")?,
				seen_announcements: app_settings.load().seen_announcements.to_owned()
			})
		)?;
	}

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

#[try_fn]
#[context("Couldn't load game files")]
pub async fn load_game_files(app: &AppHandle) -> Result<()> {
	let app_state = app.state::<AppState>();
	let app_settings = app.state::<ArcSwap<AppSettings>>();

	if let Some(path) = app_settings.load().game_install.as_ref() {
		app_state.game.store(Some(Game::load(app, path)?.into()));
	} else {
		app_state.game.store(None);
	}

	send_request(
		app,
		Request::Tool(ToolRequest::GameBrowser(GameBrowserRequest::SetEnabled {
			enabled: app_state.game.load().is_some()
		}))
	)?;

	send_request(
		app,
		Request::Tool(ToolRequest::ContentSearch(ContentSearchRequest::SetEnabled {
			enabled: app_state.game.load().is_some()
		}))
	)?;

	if let Some(game) = app_state.game.load().as_ref() {
		let task = start_task(app, "Refreshing editors")?;

		for editor in app_state.editor_states.keys().await {
			if let Some(editor) = app_state.editor_states.get(&editor).await
				&& let EditorData::ResourceOverview { hash } = editor.data
			{
				let task = start_task(app, format!("Refreshing resource overview for {}", hash))?;

				initialise_resource_overview(app, &app_state, editor.key().to_owned(), hash, game).await?;

				finish_task(app, task)?;
			}
		}

		finish_task(app, task)?;
	}
}

/// Only available for entities, the repository and unlockables currently
#[try_fn]
#[context("Couldn't open {hash} in editor")]
pub async fn open_in_editor(app: &AppHandle, game: &Game, hash: RuntimeID) -> Result<()> {
	let app_state = app.state::<AppState>();

	match game.resource_type(hash).context("Nonexistent resource")?.as_ref() {
		"TEMP" => {
			let task = start_task(app, format!("Loading entity {}", hash))?;

			let entity = game.extract_entity(hash)?.to_owned();

			let default_tab_name = format!(
				"{} ({})",
				entity
					.entities
					.get(&entity.root_entity)
					.context("Root entity doesn't exist")?
					.name,
				hash.to_hash()
			);

			let tab_name = if let Some(path) = hash.get_path() {
				get_name(&path)
			} else if let Some(entry) = hash.get_info()
				&& let Some(hint) = entry.hint
			{
				format!("{} ({})", hint, hash.to_hash())
			} else {
				default_tab_name
			};

			let id = Uuid::new_v4();

			app_state
				.editor_states
				.insert(
					id.to_owned(),
					EditorState {
						file: None,
						data: EditorData::QNPatch {
							current: Box::new((*entity).to_owned()),
							base: entity,
							settings: Default::default()
						},
						..Default::default()
					}
				)
				.await;

			send_request(
				app,
				Request::Tab(TabRequest {
					tab: id,
					data: TabRequestData::Create {
						name: tab_name,
						editor_type: EditorType::QNPatch
					}
				})
			)?;

			finish_task(app, task)?;
		}

		"REPO" => {
			let task = start_task(app, "Loading repository")?;

			let id = Uuid::new_v4();

			let repository: Vec<RepositoryItem> = game.repository().to_owned();

			app_state
				.editor_states
				.insert(
					id.to_owned(),
					EditorState {
						file: None,
						data: EditorData::RepositoryPatch {
							base: repository.to_owned(),
							current: repository,
							patch_type: JsonPatchType::MergePatch
						},
						..Default::default()
					}
				)
				.await;

			send_request(
				app,
				Request::Tab(TabRequest {
					tab: id,
					data: TabRequestData::Create {
						name: "pro.repo".into(),
						editor_type: EditorType::RepositoryPatch {
							patch_type: JsonPatchType::MergePatch
						}
					}
				})
			)?;

			finish_task(app, task)?;
		}

		"ORES" if hash == UNLOCKABLES_ID => {
			let task = start_task(app, "Loading unlockables")?;

			let id = Uuid::new_v4();

			let unlockables: Vec<UnlockableItem> =
				from_str(&parse_json_ores(&game.extract_latest_resource(UNLOCKABLES_ID)?.1)?)?;

			app_state
				.editor_states
				.insert(
					id.to_owned(),
					EditorState {
						file: None,
						data: EditorData::UnlockablesPatch {
							base: unlockables.to_owned(),
							current: unlockables,
							patch_type: JsonPatchType::MergePatch
						},
						..Default::default()
					}
				)
				.await;

			send_request(
				app,
				Request::Tab(TabRequest {
					tab: id,
					data: TabRequestData::Create {
						name: "config.unlockables".into(),
						editor_type: EditorType::UnlockablesPatch {
							patch_type: JsonPatchType::MergePatch
						}
					}
				})
			)?;

			finish_task(app, task)?;
		}

		x => panic!("Opening {x} files in editor is not supported")
	}
}
