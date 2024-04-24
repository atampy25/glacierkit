// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Specta creates non snake case functions
#![allow(non_snake_case)]
#![feature(try_blocks)]
#![feature(try_find)]
#![allow(clippy::type_complexity)]
#![feature(let_chains)]
#![feature(async_closure)]
#![feature(cursor_remaining)]
#![feature(option_get_or_insert_default)]

pub mod editor_connection;
pub mod entity;
pub mod event_handling;
pub mod game_detection;
pub mod hash_list;
pub mod intellisense;
pub mod material;
pub mod model;
pub mod ores;
pub mod repository;
pub mod resourcelib;
pub mod rpkg;
pub mod rpkg_tool;
pub mod show_in_folder;
pub mod wwev;

use std::{
	fs,
	future::Future,
	ops::{Deref, DerefMut},
	path::Path,
	sync::Arc,
	time::Duration
};

use anyhow::{anyhow, bail, Context, Error, Result};
use arboard::Clipboard;
use arc_swap::ArcSwap;
use dashmap::DashMap;
use editor_connection::EditorConnection;
use entity::{
	calculate_reverse_references, get_diff_info, get_local_reference, get_recursive_children, CopiedEntityData
};
use event_handling::{
	content_search::start_content_search,
	entity_metadata::handle_entity_metadata_event,
	entity_monaco::{handle_openfactory, handle_updatecontent},
	entity_overrides::handle_entity_overrides_event,
	entity_tree::{
		handle_delete, handle_gamebrowseradd, handle_helpmenu, handle_moveentitytocamera, handle_moveentitytoplayer,
		handle_paste, handle_restoretooriginal, handle_rotateentityascamera, handle_rotateentityasplayer,
		handle_select, handle_selectentityineditor
	},
	repository_patch::handle_repository_patch_event,
	resource_overview::{handle_resource_overview_event, initialise_resource_overview},
	unlockables_patch::handle_unlockables_patch_event
};
use fn_error_context::context;
use game_detection::{detect_installs, GameVersion};
use hash_list::HashList;
use hashbrown::{HashMap, HashSet};
use indexmap::IndexMap;
use intellisense::Intellisense;
use itertools::Itertools;
use measure_time::print_time;
use model::{
	AppSettings, AppState, ContentSearchEvent, ContentSearchRequest, ContentSearchResultsEvent,
	ContentSearchResultsRequest, EditorConnectionEvent, EditorData, EditorEvent, EditorRequest, EditorState,
	EditorType, EntityEditorEvent, EntityEditorRequest, EntityGeneralEvent, EntityGeneralRequest, EntityMetaPaneEvent,
	EntityMetadataRequest, EntityMonacoEvent, EntityMonacoRequest, EntityTreeEvent, EntityTreeRequest, Event,
	FileBrowserEvent, FileBrowserRequest, GameBrowserEntry, GameBrowserEvent, GameBrowserRequest, GlobalEvent,
	GlobalRequest, JsonPatchType, Project, ProjectSettings, Request, SearchFilter, SettingsEvent, SettingsRequest,
	TextEditorEvent, TextEditorRequest, TextFileType, ToolEvent, ToolRequest
};
use notify::Watcher;
use ores::{parse_json_ores, UnlockableItem};
use quickentity_rs::{
	apply_patch, convert_2016_blueprint_to_modern, convert_2016_factory_to_modern, convert_to_qn, convert_to_rt,
	generate_patch,
	patch_structs::Patch,
	qn_structs::{CommentEntity, Entity, Property, Ref, SubEntity, SubType}
};
use rayon::{
	iter::{IndexedParallelIterator, IntoParallelIterator, IntoParallelRefIterator, ParallelExtend, ParallelIterator},
	ThreadPoolBuilder
};
use repository::RepositoryItem;
use resourcelib::{
	h2016_convert_binary_to_blueprint, h2016_convert_binary_to_factory, h2_convert_binary_to_blueprint,
	h2_convert_binary_to_factory, h3_convert_binary_to_blueprint, h3_convert_binary_to_factory
};
use rfd::AsyncFileDialog;
use rpkg::{ensure_entity_in_cache, extract_latest_resource, normalise_to_hash};
use rpkg_rs::{
	misc::ini_file_system::IniFileSystem,
	runtime::resource::{package_defs::PackageDefinitionSource, partition_manager::PartitionManager}
};
use serde::{Deserialize, Serialize};
use serde_json::{from_slice, from_str, from_value, json, to_string, to_value, to_vec, Value};
use show_in_folder::show_in_folder;
use tauri::{api::process::Command, async_runtime, AppHandle, Manager};
use tauri_plugin_aptabase::{EventTracker, InitOptions};
use tryvial::try_fn;
use uuid::Uuid;
use velcro::vec;
use walkdir::WalkDir;

const HASH_LIST_VERSION_ENDPOINT: &str =
	"https://github.com/glacier-modding/Hitman-Hashes/releases/latest/download/version";

const HASH_LIST_ENDPOINT: &str =
	"https://github.com/glacier-modding/Hitman-Hashes/releases/latest/download/hash_list.sml";

pub trait RunCommandExt {
	/// Run the command, returning its stdout. If the command fails (status code non-zero), an error is returned with the stderr output.
	fn run(self) -> Result<String>;
}

impl RunCommandExt for Command {
	#[try_fn]
	#[context("Couldn't run command")]
	fn run(self) -> Result<String> {
		let output = self.output()?;

		if output.status.success() {
			output.stdout
		} else {
			bail!("Command failed: {}", output.stderr);
		}
	}
}

fn main() {
	let specta = {
		let specta_builder =
			tauri_specta::ts::builder().commands(tauri_specta::collect_commands![event, show_in_folder]);

		#[cfg(debug_assertions)]
		let specta_builder = if Path::new("../src/lib").is_dir() {
			specta_builder.path("../src/lib/bindings.ts")
		} else {
			specta_builder
		};

		#[cfg(debug_assertions)]
		if Path::new("../src/lib").is_dir() {
			specta::export::ts("../src/lib/bindings-types.ts").expect("Failed to export types");
		}

		specta_builder.into_plugin()
	};

	tauri::Builder::default()
		.plugin(
			tauri_plugin_aptabase::Builder::new("A-SH-1114087815")
				.with_options(InitOptions {
					host: Some("http://159.13.49.212".into()),
					flush_interval: None
				})
				.build()
		)
		.plugin(specta)
		.setup(|app| {
			app.track_event("App started", None);

			let app_data_path = app.path_resolver().app_data_dir().expect("Couldn't get data dir");

			let mut invalid = true;
			if let Ok(read) = fs::read(app_data_path.join("settings.json")) {
				if let Ok(settings) = from_slice::<AppSettings>(&read) {
					invalid = false;
					app.manage(ArcSwap::new(settings.into()));
				}
			}

			if invalid {
				let settings = AppSettings::default();
				fs::create_dir_all(&app_data_path).expect("Couldn't create app data dir");
				fs::write(
					app_data_path.join("settings.json"),
					to_vec(&settings).expect("Couldn't serialise default app settings")
				)
				.expect("Couldn't write default app settings");
				app.manage(ArcSwap::new(settings.into()));
			}

			if app_data_path.join("temp").exists() {
				fs::remove_dir_all(app_data_path.join("temp"))?;
			}

			app.manage(AppState {
				game_installs: detect_installs().expect("Couldn't detect game installs"),
				project: None.into(),
				hash_list: fs::read(app_data_path.join("hash_list.sml"))
					.ok()
					.and_then(|x| serde_smile::from_slice(&x).ok())
					.into(),
				fs_watcher: None.into(),
				editor_states: DashMap::new().into(),
				game_files: None.into(),
				resource_reverse_dependencies: None.into(),
				cached_entities: DashMap::new().into(),
				repository: None.into(),
				intellisense: None.into(),
				editor_connection: EditorConnection::new(app.handle())
			});

			Ok(())
		})
		.build(tauri::generate_context!())
		.expect("error while building tauri application")
		.run(|handler, event| {
			if let tauri::RunEvent::Exit = event {
				handler.track_event("App exited", None);
				handler.flush_events_blocking();
			}
		});
}

pub fn handle_event(app: &AppHandle, evt: Event) {
	event(app.clone(), evt);
}

#[tauri::command]
#[specta::specta]
fn event(app: AppHandle, event: Event) {
	async_runtime::spawn(async move {
		let cloned_app = app.clone();

		if let Err(e) = async_runtime::spawn(async move {
			let app_settings = app.state::<ArcSwap<AppSettings>>();
			let app_state = app.state::<AppState>();

			if let Err::<_, Error>(e) = try {
				match event {
					Event::Tool(event) => match event {
						ToolEvent::FileBrowser(event) => match event {
							FileBrowserEvent::Select(path) => {
								if let Some(path) = path {
									let task = start_task(
										&app,
										format!(
											"Opening {}",
											path.file_name().context("No file name")?.to_string_lossy()
										)
									)?;

									let existing = {
										app_state
											.editor_states
											.iter()
											.find(|x| x.file.as_ref().map(|x| x == &path).unwrap_or(false))
											.map(|x| x.key().to_owned())
									};

									if let Some(existing) = existing {
										send_request(&app, Request::Global(GlobalRequest::SelectTab(existing)))?;
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
													from_slice(&fs::read(&path).context("Couldn't read file")?)
														.context("Invalid entity")?;

												// Normalise comments to form used by GlacierKit (single comment for each entity)
												let mut comments: Vec<CommentEntity> = vec![];
												for comment in entity.comments {
													if let Some(x) =
														comments.iter_mut().find(|x| x.parent == comment.parent)
													{
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
													&app,
													Request::Global(GlobalRequest::CreateTab {
														id,
														name: path
															.file_name()
															.context("No file name")?
															.to_string_lossy()
															.into(),
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
														from_slice(&fs::read(&path).context("Couldn't read file")?)
															.context("Invalid entity")?;

													ensure_entity_in_cache(
														game_files,
														&app_state.cached_entities,
														app_state
															.game_installs
															.iter()
															.try_find(|x| anyhow::Ok(x.path == *install))?
															.context("No such game install")?
															.version,
														hash_list,
														&normalise_to_hash(patch.factory_hash.to_owned())
													)?;

													let mut entity = app_state
														.cached_entities
														.get(&normalise_to_hash(patch.factory_hash.to_owned()))
														.unwrap()
														.to_owned();

													let base = entity.to_owned();

													apply_patch(&mut entity, patch, true)
														.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

													// Normalise comments to form used by GlacierKit (single comment for each entity)
													let mut comments: Vec<CommentEntity> = vec![];
													for comment in entity.comments {
														if let Some(x) =
															comments.iter_mut().find(|x| x.parent == comment.parent)
														{
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
														&app,
														Request::Global(GlobalRequest::CreateTab {
															id,
															name: path
																.file_name()
																.context("No file name")?
																.to_string_lossy()
																.into(),
															editor_type: EditorType::QNPatch
														})
													)?;
												} else {
													send_request(
														&app,
														Request::Tool(ToolRequest::FileBrowser(
															FileBrowserRequest::Select(None)
														))
													)?;

													send_notification(
														&app,
														Notification {
															kind: NotificationKind::Error,
															title: "No game selected".into(),
															subtitle: "You can't open patch files without a copy of \
															           the game selected."
																.into()
														}
													)?;
												}
											}

											"json" => {
												let id = Uuid::new_v4();

												let file_type =
													if path.file_name().context("No file name")?.to_string_lossy()
														== "manifest.json"
													{
														TextFileType::ManifestJson
													} else {
														TextFileType::Json
													};

												app_state.editor_states.insert(
													id.to_owned(),
													EditorState {
														file: Some(path.to_owned()),
														data: EditorData::Text {
															content: fs::read_to_string(&path)
																.context("Couldn't read file")?,
															file_type: file_type.to_owned()
														}
													}
												);

												send_request(
													&app,
													Request::Global(GlobalRequest::CreateTab {
														id,
														name: path
															.file_name()
															.context("No file name")?
															.to_string_lossy()
															.into(),
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
															content: fs::read_to_string(&path)
																.context("Couldn't read file")?,
															file_type: TextFileType::PlainText
														}
													}
												);

												send_request(
													&app,
													Request::Global(GlobalRequest::CreateTab {
														id,
														name: path
															.file_name()
															.context("No file name")?
															.to_string_lossy()
															.into(),
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
															content: fs::read_to_string(&path)
																.context("Couldn't read file")?,
															file_type: TextFileType::Markdown
														}
													}
												);

												send_request(
													&app,
													Request::Global(GlobalRequest::CreateTab {
														id,
														name: path
															.file_name()
															.context("No file name")?
															.to_string_lossy()
															.into(),
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
														from_slice(&fs::read(&path).context("Couldn't read file")?)
															.context("Invalid JSON")?;

													json_patch::merge(&mut repository, &patch);

													let repository = from_value::<
														IndexMap<Uuid, IndexMap<String, Value>>
													>(repository)?
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
														&app,
														Request::Global(GlobalRequest::CreateTab {
															id,
															name: path
																.file_name()
																.context("No file name")?
																.to_string_lossy()
																.into(),
															editor_type: EditorType::RepositoryPatch {
																patch_type: JsonPatchType::MergePatch
															}
														})
													)?;
												} else {
													send_request(
														&app,
														Request::Tool(ToolRequest::FileBrowser(
															FileBrowserRequest::Select(None)
														))
													)?;

													send_notification(
														&app,
														Notification {
															kind: NotificationKind::Error,
															title: "No game selected".into(),
															subtitle: "You can't open patch files without a copy of \
															           the game selected."
																.into()
														}
													)?;
												}
											}

											"unlockables.json" => {
												let id = Uuid::new_v4();

												if let Some(game_files) = app_state.game_files.load().as_ref()
													&& let Some(hash_list) = app_state.hash_list.load().as_ref()
												{
													let mut unlockables = to_value(
														from_value::<Vec<UnlockableItem>>(parse_json_ores(
															&extract_latest_resource(
																game_files,
																hash_list,
																"0057C2C3941115CA"
															)?
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
																	y.extend(
																		x.data
																			.into_iter()
																			.filter(|(key, _)| key != "Id")
																	);
																	y
																}
															)
														})
														.collect::<IndexMap<String, IndexMap<String, Value>>>()
													)?;

													let base = parse_json_ores(
														&extract_latest_resource(
															game_files,
															hash_list,
															"0057C2C3941115CA"
														)?
														.1
													)?;

													let patch: Value =
														from_slice(&fs::read(&path).context("Couldn't read file")?)
															.context("Invalid JSON")?;

													json_patch::merge(&mut unlockables, &patch);

													let unlockables = from_value::<
														IndexMap<String, IndexMap<String, Value>>
													>(unlockables)?
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
														&app,
														Request::Global(GlobalRequest::CreateTab {
															id,
															name: path
																.file_name()
																.context("No file name")?
																.to_string_lossy()
																.into(),
															editor_type: EditorType::UnlockablesPatch {
																patch_type: JsonPatchType::MergePatch
															}
														})
													)?;
												} else {
													send_request(
														&app,
														Request::Tool(ToolRequest::FileBrowser(
															FileBrowserRequest::Select(None)
														))
													)?;

													send_notification(
														&app,
														Notification {
															kind: NotificationKind::Error,
															title: "No game selected".into(),
															subtitle: "You can't open patch files without a copy of \
															           the game selected."
																.into()
														}
													)?;
												}
											}

											"JSON.patch.json" => {
												let id = Uuid::new_v4();

												let file: Value =
													from_slice(&fs::read(&path).context("Couldn't read file")?)
														.context("Invalid patch")?;

												match file
													.get("type")
													.unwrap_or(&Value::String("JSON".into()))
													.as_str()
													.context("Type key was not string")?
												{
													"REPO" => {
														if let Some(cached_repository) =
															app_state.repository.load().as_ref()
														{
															let mut repository = to_value(
																cached_repository
																	.iter()
																	.cloned()
																	.map(|x| (x.id, x.data))
																	.collect::<IndexMap<Uuid, IndexMap<String, Value>>>(
																	)
															)?;

															let base = to_value(cached_repository)?;

															let patch = from_slice::<Value>(
																&fs::read(&path).context("Couldn't read file")?
															)
															.context("Invalid JSON")?;

															let patch =
																patch.get("patch").context("Patch had no patch key")?;

															json_patch::patch(
																&mut repository,
																&from_value::<Vec<json_patch::PatchOperation>>(
																	patch.to_owned()
																)
																.context("Invalid JSON patch")?
															)?;

															let repository = from_value::<
																IndexMap<Uuid, IndexMap<String, Value>>
															>(repository)?
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
																&app,
																Request::Global(GlobalRequest::CreateTab {
																	id,
																	name: path
																		.file_name()
																		.context("No file name")?
																		.to_string_lossy()
																		.into(),
																	editor_type: EditorType::RepositoryPatch {
																		patch_type: JsonPatchType::JsonPatch
																	}
																})
															)?;
														} else {
															send_request(
																&app,
																Request::Tool(ToolRequest::FileBrowser(
																	FileBrowserRequest::Select(None)
																))
															)?;

															send_notification(
																&app,
																Notification {
																	kind: NotificationKind::Error,
																	title: "No game selected".into(),
																	subtitle: "You can't open patch files without a \
																	           copy of the game selected."
																		.into()
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

														if let Some(game_files) = app_state.game_files.load().as_ref()
															&& let Some(hash_list) = app_state.hash_list.load().as_ref()
														{
															let mut unlockables = to_value(
																from_value::<Vec<UnlockableItem>>(parse_json_ores(
																	&extract_latest_resource(
																		game_files,
																		hash_list,
																		"0057C2C3941115CA"
																	)?
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
																			y.insert(
																				"Guid".into(),
																				to_value(x.id).unwrap()
																			);
																			y.extend(
																				x.data
																					.into_iter()
																					.filter(|(key, _)| key != "Id")
																			);
																			y
																		}
																	)
																})
																.collect::<IndexMap<String, IndexMap<String, Value>>>()
															)?;

															let base = parse_json_ores(
																&extract_latest_resource(
																	game_files,
																	hash_list,
																	"0057C2C3941115CA"
																)?
																.1
															)?;

															let patch = from_slice::<Value>(
																&fs::read(&path).context("Couldn't read file")?
															)
															.context("Invalid JSON")?;

															let patch =
																patch.get("patch").context("Patch had no patch key")?;

															json_patch::patch(
																&mut unlockables,
																&from_value::<Vec<json_patch::PatchOperation>>(
																	patch.to_owned()
																)
																.context("Invalid JSON patch")?
															)?;

															let unlockables = from_value::<
																IndexMap<String, IndexMap<String, Value>>
															>(unlockables)?
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
																	y.extend(
																		data.into_iter()
																			.filter(|(key, _)| key != "Guid")
																	);
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
																&app,
																Request::Global(GlobalRequest::CreateTab {
																	id,
																	name: path
																		.file_name()
																		.context("No file name")?
																		.to_string_lossy()
																		.into(),
																	editor_type: EditorType::UnlockablesPatch {
																		patch_type: JsonPatchType::JsonPatch
																	}
																})
															)?;
														} else {
															send_request(
																&app,
																Request::Tool(ToolRequest::FileBrowser(
																	FileBrowserRequest::Select(None)
																))
															)?;

															send_notification(
																&app,
																Notification {
																	kind: NotificationKind::Error,
																	title: "No game selected".into(),
																	subtitle: "You can't open patch files without a \
																	           copy of the game selected."
																		.into()
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
																	content: fs::read_to_string(&path)
																		.context("Couldn't read file")?,
																	file_type: TextFileType::Json
																}
															}
														);

														send_request(
															&app,
															Request::Global(GlobalRequest::CreateTab {
																id,
																name: path
																	.file_name()
																	.context("No file name")?
																	.to_string_lossy()
																	.into(),
																editor_type: EditorType::Text {
																	file_type: TextFileType::Json
																}
															})
														)?;
													}
												}
											}

											"dlge.json" | "locr.json" | "rtlv.json" | "clng.json" | "ditl.json"
											| "material.json" | "contract.json" => {
												let id = Uuid::new_v4();

												app_state.editor_states.insert(
													id.to_owned(),
													EditorState {
														file: Some(path.to_owned()),
														data: EditorData::Text {
															content: fs::read_to_string(&path)
																.context("Couldn't read file")?,
															file_type: TextFileType::Json
														}
													}
												);

												send_request(
													&app,
													Request::Global(GlobalRequest::CreateTab {
														id,
														name: path
															.file_name()
															.context("No file name")?
															.to_string_lossy()
															.into(),
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
													&app,
													Request::Global(GlobalRequest::CreateTab {
														id,
														name: path
															.file_name()
															.context("No file name")?
															.to_string_lossy()
															.into(),
														editor_type: EditorType::Nil
													})
												)?;
											}
										}
									}

									finish_task(&app, task)?;
								}
							}

							FileBrowserEvent::Create { path, is_folder } => {
								let task = start_task(
									&app,
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

								finish_task(&app, task)?;
							}

							FileBrowserEvent::Delete(path) => {
								let task = start_task(
									&app,
									format!("Moving {} to bin", path.file_name().unwrap().to_string_lossy())
								)?;

								trash::delete(path)?;

								finish_task(&app, task)?;
							}

							FileBrowserEvent::Rename { old_path, new_path } => {
								let task = start_task(
									&app,
									format!(
										"Renaming {} to {}",
										old_path.file_name().unwrap().to_string_lossy(),
										new_path.file_name().unwrap().to_string_lossy()
									)
								)?;

								fs::rename(old_path, new_path)?;

								finish_task(&app, task)?;
							}

							FileBrowserEvent::NormaliseQNFile { path } => {
								let task = start_task(
									&app,
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
											from_slice(&fs::read(&path).context("Couldn't read file")?)
												.context("Invalid entity")?;

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

										let (fac, fac_meta, blu, blu_meta) = convert_to_rt(&entity)
											.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

										let mut reconverted = convert_to_qn(&fac, &fac_meta, &blu, &blu_meta, true)
											.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

										reconverted.comments = comments;

										fs::write(path, to_vec(&reconverted)?)?;

										send_notification(
											&app,
											Notification {
												kind: NotificationKind::Success,
												title: "File normalised".into(),
												subtitle: "The entity file has been re-saved in canonical format."
													.into()
											}
										)?;
									}

									"entity.patch.json" => {
										let patch: Patch = from_slice(&fs::read(&path).context("Couldn't read file")?)
											.context("Invalid entity")?;

										if let Some(game_files) = app_state.game_files.load().as_ref()
											&& let Some(install) = app_settings.load().game_install.as_ref()
											&& let Some(hash_list) = app_state.hash_list.load().as_ref()
										{
											ensure_entity_in_cache(
												game_files,
												&app_state.cached_entities,
												app_state
													.game_installs
													.iter()
													.try_find(|x| anyhow::Ok(x.path == *install))?
													.context("No such game install")?
													.version,
												hash_list,
												&normalise_to_hash(patch.factory_hash.to_owned())
											)?;

											let mut entity = app_state
												.cached_entities
												.get(&normalise_to_hash(patch.factory_hash.to_owned()))
												.unwrap()
												.to_owned();

											let base = entity.to_owned();

											apply_patch(&mut entity, patch, true)
												.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

											// Normalise comments to form used by GlacierKit (single comment for each entity)
											let mut comments: Vec<CommentEntity> = vec![];
											for comment in entity.comments {
												if let Some(x) =
													comments.iter_mut().find(|x| x.parent == comment.parent)
												{
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

											let (fac, fac_meta, blu, blu_meta) = convert_to_rt(&entity)
												.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

											let mut reconverted = convert_to_qn(&fac, &fac_meta, &blu, &blu_meta, true)
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
												&app,
												Notification {
													kind: NotificationKind::Success,
													title: "File normalised".into(),
													subtitle: "The patch file has been re-saved in canonical format."
														.into()
												}
											)?;
										} else {
											send_notification(
												&app,
												Notification {
													kind: NotificationKind::Error,
													title: "No game selected".into(),
													subtitle: "You can't normalise patch files without a copy of the \
													           game selected."
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

								finish_task(&app, task)?;
							}

							FileBrowserEvent::ConvertEntityToPatch { path } => {
								if let Some(game_files) = app_state.game_files.load().as_ref()
									&& let Some(install) = app_settings.load().game_install.as_ref()
									&& let Some(hash_list) = app_state.hash_list.load().as_ref()
								{
									let mut entity: Entity =
										from_slice(&fs::read(&path).context("Couldn't read file")?)
											.context("Invalid entity")?;

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

									let game_version = app_state
										.game_installs
										.iter()
										.try_find(|x| anyhow::Ok(x.path == *install))?
										.context("No such game install")?
										.version;

									// `ensure_entity_in_cache` is not used here because the entity needs to be extracted in non-lossless mode to avoid meaningless `scale`-removing patch operations being added.
									let (temp_meta, temp_data) = extract_latest_resource(
										game_files,
										hash_list,
										&normalise_to_hash(entity.factory_hash.to_owned())
									)?;

									let factory = match game_version {
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

									let (tblu_meta, tblu_data) =
										extract_latest_resource(game_files, hash_list, blueprint_hash)?;

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

									let base = convert_to_qn(&factory, &temp_meta, &blueprint, &tblu_meta, false)
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
										to_vec(
											&generate_patch(&base, &entity)
												.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?
										)?
									)?;

									fs::remove_file(&path)?;

									send_notification(
										&app,
										Notification {
											kind: NotificationKind::Success,
											title: "File converted to patch".into(),
											subtitle: "The entity.json file has been converted into a patch file."
												.into()
										}
									)?;
								} else {
									send_notification(
										&app,
										Notification {
											kind: NotificationKind::Error,
											title: "No game selected".into(),
											subtitle: "You can't convert between entity and patch without a copy of \
											           the game selected."
												.into()
										}
									)?;
								}
							}

							FileBrowserEvent::ConvertPatchToEntity { path } => {
								let patch: Patch = from_slice(&fs::read(&path).context("Couldn't read file")?)
									.context("Invalid entity")?;

								if let Some(game_files) = app_state.game_files.load().as_ref()
									&& let Some(install) = app_settings.load().game_install.as_ref()
									&& let Some(hash_list) = app_state.hash_list.load().as_ref()
								{
									ensure_entity_in_cache(
										game_files,
										&app_state.cached_entities,
										app_state
											.game_installs
											.iter()
											.try_find(|x| anyhow::Ok(x.path == *install))?
											.context("No such game install")?
											.version,
										hash_list,
										&normalise_to_hash(patch.factory_hash.to_owned())
									)?;

									let mut entity = app_state
										.cached_entities
										.get(&normalise_to_hash(patch.factory_hash.to_owned()))
										.unwrap()
										.to_owned();

									apply_patch(&mut entity, patch, true)
										.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

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
										&app,
										Notification {
											kind: NotificationKind::Success,
											title: "File converted to entity.json".into(),
											subtitle: "The patch file has been converted into an entity.json file."
												.into()
										}
									)?;
								} else {
									send_notification(
										&app,
										Notification {
											kind: NotificationKind::Error,
											title: "No game selected".into(),
											subtitle: "You can't convert between entity and patch without a copy of \
											           the game selected."
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
									.context("Type key was not string")? == "REPO"
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

										let mut merge_patch = json!({});

										for operation in patch.0 {
											match operation {
												json_patch::PatchOperation::Add(json_patch::AddOperation {
													path,
													value
												}) => {
													let mut view = &mut merge_patch;

													if path
														.chars()
														.skip(1)
														.collect::<String>()
														.split('/')
														.last()
														.unwrap()
														.parse::<usize>()
														.is_err()
													{
														for component in
															path.chars().skip(1).collect::<String>().split('/')
														{
															view = view
																.as_object_mut()
																.unwrap()
																.entry(component)
																.or_insert(json!({}));
														}

														*view = value;
													} else {
														// If the last component is a number we assume it's an array operation, so we replace the whole array with the correct data
														for component in path
															.chars()
															.skip(1)
															.collect::<String>()
															.split('/')
															.collect::<Vec<_>>()
															.into_iter()
															.rev()
															.skip(1)
															.rev()
														{
															view = view
																.as_object_mut()
																.unwrap()
																.entry(component)
																.or_insert(json!({}));
														}

														*view = current
															.pointer(
																&path
																	.chars()
																	.skip(1)
																	.collect::<String>()
																	.split('/')
																	.collect::<Vec<_>>()
																	.into_iter()
																	.rev()
																	.skip(1)
																	.rev()
																	.collect::<Vec<_>>()
																	.join("/")
															)
															.unwrap()
															.to_owned();
													}
												}

												json_patch::PatchOperation::Remove(json_patch::RemoveOperation {
													path
												}) => {
													let mut view = &mut merge_patch;

													if path
														.chars()
														.skip(1)
														.collect::<String>()
														.split('/')
														.last()
														.unwrap()
														.parse::<usize>()
														.is_err()
													{
														for component in
															path.chars().skip(1).collect::<String>().split('/')
														{
															view = view
																.as_object_mut()
																.unwrap()
																.entry(component)
																.or_insert(json!({}));
														}

														*view = Value::Null;
													} else {
														// If the last component is a number we assume it's an array operation, so we replace the whole array with the correct data
														for component in path
															.chars()
															.skip(1)
															.collect::<String>()
															.split('/')
															.collect::<Vec<_>>()
															.into_iter()
															.rev()
															.skip(1)
															.rev()
														{
															view = view
																.as_object_mut()
																.unwrap()
																.entry(component)
																.or_insert(json!({}));
														}

														*view = current
															.pointer(
																&path
																	.chars()
																	.skip(1)
																	.collect::<String>()
																	.split('/')
																	.collect::<Vec<_>>()
																	.into_iter()
																	.rev()
																	.skip(1)
																	.rev()
																	.collect::<Vec<_>>()
																	.join("/")
															)
															.unwrap()
															.to_owned();
													}
												}

												json_patch::PatchOperation::Replace(json_patch::ReplaceOperation {
													path,
													value
												}) => {
													let mut view = &mut merge_patch;

													if path
														.chars()
														.skip(1)
														.collect::<String>()
														.split('/')
														.last()
														.unwrap()
														.parse::<usize>()
														.is_err()
													{
														for component in
															path.chars().skip(1).collect::<String>().split('/')
														{
															view = view
																.as_object_mut()
																.unwrap()
																.entry(component)
																.or_insert(json!({}));
														}

														*view = value;
													} else {
														// If the last component is a number we assume it's an array operation, so we replace the whole array with the correct data
														for component in path
															.chars()
															.skip(1)
															.collect::<String>()
															.split('/')
															.collect::<Vec<_>>()
															.into_iter()
															.rev()
															.skip(1)
															.rev()
														{
															view = view
																.as_object_mut()
																.unwrap()
																.entry(component)
																.or_insert(json!({}));
														}

														*view = current
															.pointer(
																&path
																	.chars()
																	.skip(1)
																	.collect::<String>()
																	.split('/')
																	.collect::<Vec<_>>()
																	.into_iter()
																	.rev()
																	.skip(1)
																	.rev()
																	.collect::<Vec<_>>()
																	.join("/")
															)
															.unwrap()
															.to_owned();
													}
												}

												json_patch::PatchOperation::Move(_) => unreachable!(
													"Calculation of JSON patch does not emit Move operations"
												),

												json_patch::PatchOperation::Copy(_) => unreachable!(
													"Calculation of JSON patch does not emit Copy operations"
												),

												json_patch::PatchOperation::Test(_) => unreachable!(
													"Calculation of JSON patch does not emit Test operations"
												)
											}
										}

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
											to_vec(&merge_patch)?
										)?;

										fs::remove_file(&path)?;

										send_notification(
											&app,
											Notification {
												kind: NotificationKind::Success,
												title: "File converted to repository.json".into(),
												subtitle: "The patch file has been converted into a repository.json \
												           file."
													.into()
											}
										)?;
									} else {
										send_notification(
											&app,
											Notification {
												kind: NotificationKind::Error,
												title: "No game selected".into(),
												subtitle: "You can't convert between patch formats without a copy of \
												           the game selected."
													.into()
											}
										)?;
									}
								} else {
									send_notification(
										&app,
										Notification {
											kind: NotificationKind::Error,
											title: "Not a repository patch".into(),
											subtitle: "This patch is for a different type of file, so it can't be \
											           converted to a repository.json file."
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

									let patch: Value = from_slice(&fs::read(&path).context("Couldn't read file")?)
										.context("Invalid JSON")?;

									json_patch::merge(&mut current, &patch);

									send_request(
										&app,
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
										&app,
										Notification {
											kind: NotificationKind::Success,
											title: "File converted to JSON.patch.json".into(),
											subtitle: "The patch file has been converted into a JSON.patch.json file."
												.into()
										}
									)?;
								} else {
									send_notification(
										&app,
										Notification {
											kind: NotificationKind::Error,
											title: "No game selected".into(),
											subtitle: "You can't convert between patch formats without a copy of the \
											           game selected."
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
									.context("File key was not string")? == "0057C2C3941115CA"
								{
									if let Some(game_files) = app_state.game_files.load().as_ref()
										&& let Some(hash_list) = app_state.hash_list.load().as_ref()
									{
										let mut current = to_value(
											from_value::<Vec<UnlockableItem>>(parse_json_ores(
												&extract_latest_resource(game_files, hash_list, "0057C2C3941115CA")?.1
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

										let mut merge_patch = json!({});

										for operation in patch.0 {
											match operation {
												json_patch::PatchOperation::Add(json_patch::AddOperation {
													path,
													value
												}) => {
													let mut view = &mut merge_patch;

													if path
														.chars()
														.skip(1)
														.collect::<String>()
														.split('/')
														.last()
														.unwrap()
														.parse::<usize>()
														.is_err()
													{
														for component in
															path.chars().skip(1).collect::<String>().split('/')
														{
															view = view
																.as_object_mut()
																.unwrap()
																.entry(component)
																.or_insert(json!({}));
														}

														*view = value;
													} else {
														// If the last component is a number we assume it's an array operation, so we replace the whole array with the correct data
														for component in path
															.chars()
															.skip(1)
															.collect::<String>()
															.split('/')
															.collect::<Vec<_>>()
															.into_iter()
															.rev()
															.skip(1)
															.rev()
														{
															view = view
																.as_object_mut()
																.unwrap()
																.entry(component)
																.or_insert(json!({}));
														}

														*view = current
															.pointer(
																&path
																	.chars()
																	.skip(1)
																	.collect::<String>()
																	.split('/')
																	.collect::<Vec<_>>()
																	.into_iter()
																	.rev()
																	.skip(1)
																	.rev()
																	.collect::<Vec<_>>()
																	.join("/")
															)
															.unwrap()
															.to_owned();
													}
												}

												json_patch::PatchOperation::Remove(json_patch::RemoveOperation {
													path
												}) => {
													let mut view = &mut merge_patch;

													if path
														.chars()
														.skip(1)
														.collect::<String>()
														.split('/')
														.last()
														.unwrap()
														.parse::<usize>()
														.is_err()
													{
														for component in
															path.chars().skip(1).collect::<String>().split('/')
														{
															view = view
																.as_object_mut()
																.unwrap()
																.entry(component)
																.or_insert(json!({}));
														}

														*view = Value::Null;
													} else {
														// If the last component is a number we assume it's an array operation, so we replace the whole array with the correct data
														for component in path
															.chars()
															.skip(1)
															.collect::<String>()
															.split('/')
															.collect::<Vec<_>>()
															.into_iter()
															.rev()
															.skip(1)
															.rev()
														{
															view = view
																.as_object_mut()
																.unwrap()
																.entry(component)
																.or_insert(json!({}));
														}

														*view = current
															.pointer(
																&path
																	.chars()
																	.skip(1)
																	.collect::<String>()
																	.split('/')
																	.collect::<Vec<_>>()
																	.into_iter()
																	.rev()
																	.skip(1)
																	.rev()
																	.collect::<Vec<_>>()
																	.join("/")
															)
															.unwrap()
															.to_owned();
													}
												}

												json_patch::PatchOperation::Replace(json_patch::ReplaceOperation {
													path,
													value
												}) => {
													let mut view = &mut merge_patch;

													if path
														.chars()
														.skip(1)
														.collect::<String>()
														.split('/')
														.last()
														.unwrap()
														.parse::<usize>()
														.is_err()
													{
														for component in
															path.chars().skip(1).collect::<String>().split('/')
														{
															view = view
																.as_object_mut()
																.unwrap()
																.entry(component)
																.or_insert(json!({}));
														}

														*view = value;
													} else {
														// If the last component is a number we assume it's an array operation, so we replace the whole array with the correct data
														for component in path
															.chars()
															.skip(1)
															.collect::<String>()
															.split('/')
															.collect::<Vec<_>>()
															.into_iter()
															.rev()
															.skip(1)
															.rev()
														{
															view = view
																.as_object_mut()
																.unwrap()
																.entry(component)
																.or_insert(json!({}));
														}

														*view = current
															.pointer(
																&path
																	.chars()
																	.skip(1)
																	.collect::<String>()
																	.split('/')
																	.collect::<Vec<_>>()
																	.into_iter()
																	.rev()
																	.skip(1)
																	.rev()
																	.collect::<Vec<_>>()
																	.join("/")
															)
															.unwrap()
															.to_owned();
													}
												}

												json_patch::PatchOperation::Move(_) => unreachable!(
													"Calculation of JSON patch does not emit Move operations"
												),

												json_patch::PatchOperation::Copy(_) => unreachable!(
													"Calculation of JSON patch does not emit Copy operations"
												),

												json_patch::PatchOperation::Test(_) => unreachable!(
													"Calculation of JSON patch does not emit Test operations"
												)
											}
										}

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
											to_vec(&merge_patch)?
										)?;

										fs::remove_file(&path)?;

										send_notification(
											&app,
											Notification {
												kind: NotificationKind::Success,
												title: "File converted to unlockables.json".into(),
												subtitle: "The patch file has been converted into a unlockables.json \
												           file."
													.into()
											}
										)?;
									} else {
										send_notification(
											&app,
											Notification {
												kind: NotificationKind::Error,
												title: "No game selected".into(),
												subtitle: "You can't convert between patch formats without a copy of \
												           the game selected."
													.into()
											}
										)?;
									}
								} else {
									send_notification(
										&app,
										Notification {
											kind: NotificationKind::Error,
											title: "Not an unlockables patch".into(),
											subtitle: "This patch is for a different type of file, so it can't be \
											           converted to a unlockables.json file."
												.into()
										}
									)?;
								}
							}

							FileBrowserEvent::ConvertUnlockablesPatchToJsonPatch { path } => {
								if let Some(game_files) = app_state.game_files.load().as_ref()
									&& let Some(hash_list) = app_state.hash_list.load().as_ref()
								{
									let mut current = to_value(
										from_value::<Vec<UnlockableItem>>(parse_json_ores(
											&extract_latest_resource(game_files, hash_list, "0057C2C3941115CA")?.1
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

									let patch: Value = from_slice(&fs::read(&path).context("Couldn't read file")?)
										.context("Invalid JSON")?;

									json_patch::merge(&mut current, &patch);

									send_request(
										&app,
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
										&app,
										Notification {
											kind: NotificationKind::Success,
											title: "File converted to JSON.patch.json".into(),
											subtitle: "The patch file has been converted into a JSON.patch.json file."
												.into()
										}
									)?;
								} else {
									send_notification(
										&app,
										Notification {
											kind: NotificationKind::Error,
											title: "No game selected".into(),
											subtitle: "You can't convert between patch formats without a copy of the \
											           game selected."
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
									&app,
									Request::Global(GlobalRequest::CreateTab {
										id,
										name: format!("Resource overview ({hash})"),
										editor_type: EditorType::ResourceOverview
									})
								)?;
							}

							GameBrowserEvent::Search(query, filter) => {
								let task = start_task(&app, format!("Searching game files for {}", query))?;

								if let Some(install) = app_settings.load().game_install.as_ref()
									&& let Some(resource_reverse_dependencies) =
										app_state.resource_reverse_dependencies.load().as_ref()
								{
									let install = app_state
										.game_installs
										.iter()
										.find(|x| x.path == *install)
										.context("No such game install as specified in project.json")?;

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
											&app,
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
															.filter(|(hash, _)| {
																resource_reverse_dependencies.contains_key(*hash)
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
															.map(|(hash, entry)| GameBrowserEntry {
																hash: hash.to_owned(),
																path: entry.path.to_owned(),
																hint: entry.hint.to_owned(),
																filetype: entry.resource_type.to_owned()
															})
															.collect()
													} else {
														hash_list
															.entries
															.par_iter()
															.filter(|(hash, _)| {
																resource_reverse_dependencies.contains_key(*hash)
															})
															.filter(|(_, entry)| {
																filter_includes
																	.iter()
																	.any(|&x| x == entry.resource_type)
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
															.map(|(hash, entry)| GameBrowserEntry {
																hash: hash.to_owned(),
																path: entry.path.to_owned(),
																hint: entry.hint.to_owned(),
																filetype: entry.resource_type.to_owned()
															})
															.collect()
													}
												}
											}))
										)?;
									}
								}

								finish_task(&app, task)?;
							}

							GameBrowserEvent::OpenInEditor(hash) => {
								// Only available for entities, the repository and unlockables currently

								if let Some(game_files) = app_state.game_files.load().as_ref()
									&& let Some(install) = app_settings.load().game_install.as_ref()
									&& let Some(hash_list) = app_state.hash_list.load().as_ref()
								{
									match hash_list
										.entries
										.get(&hash)
										.context("Not in hash list")?
										.resource_type
										.as_ref()
									{
										"TEMP" => {
											let task = start_task(&app, format!("Loading entity {}", hash))?;

											let game_install_data = app_state
												.game_installs
												.iter()
												.try_find(|x| anyhow::Ok(x.path == *install))?
												.context("No such game install")?;

											ensure_entity_in_cache(
												game_files,
												&app_state.cached_entities,
												game_install_data.version,
												hash_list,
												&hash
											)?;

											let entity = app_state.cached_entities.get(&hash).unwrap().to_owned();

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
												&app,
												Request::Global(GlobalRequest::CreateTab {
													id,
													name: tab_name,
													editor_type: EditorType::QNPatch
												})
											)?;

											finish_task(&app, task)?;
										}

										"REPO" => {
											let task = start_task(&app, "Loading repository")?;

											let id = Uuid::new_v4();

											let repository: Vec<RepositoryItem> =
												if let Some(x) = app_state.repository.load().as_ref() {
													x.par_iter().cloned().collect()
												} else {
													from_slice(
														&extract_latest_resource(
															game_files,
															hash_list,
															"00204D1AFD76AB13"
														)?
														.1
													)?
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
												&app,
												Request::Global(GlobalRequest::CreateTab {
													id,
													name: "pro.repo".into(),
													editor_type: EditorType::RepositoryPatch {
														patch_type: JsonPatchType::MergePatch
													}
												})
											)?;

											finish_task(&app, task)?;
										}

										"ORES" if hash == "0057C2C3941115CA" => {
											let task = start_task(&app, "Loading unlockables")?;

											let id = Uuid::new_v4();

											let unlockables: Vec<UnlockableItem> = from_value(parse_json_ores(
												&extract_latest_resource(game_files, hash_list, "0057C2C3941115CA")?.1
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
												&app,
												Request::Global(GlobalRequest::CreateTab {
													id,
													name: "config.unlockables".into(),
													editor_type: EditorType::UnlockablesPatch {
														patch_type: JsonPatchType::MergePatch
													}
												})
											)?;

											finish_task(&app, task)?;
										}

										x => panic!("Opening {x} files in editor is not supported")
									}
								}
							}
						},

						ToolEvent::Settings(event) => match event {
							SettingsEvent::Initialise => {
								if let Ok(req) =
									reqwest::get("https://hitman-resources.netlify.app/glacierkit/dynamics.json").await
								{
									send_request(
										&app,
										Request::Global(GlobalRequest::InitialiseDynamics {
											dynamics: req
												.json()
												.await
												.context("Couldn't deserialise dynamics response")?,
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
										"selected_install": selected_install_info
									}))
								);

								send_request(
									&app,
									Request::Tool(ToolRequest::Settings(SettingsRequest::Initialise {
										game_installs: app_state.game_installs.to_owned(),
										settings: (*app_settings.load_full()).to_owned()
									}))
								)?;

								load_game_files(&app).await?;

								let app = app.clone();

								async_runtime::spawn(async move {
									let mut interval = tokio::time::interval(Duration::from_secs(10));

									loop {
										interval.tick().await;

										// Attempt to connect every 10 seconds; it doesn't matter if it fails or is already connected
										let _ = app.state::<AppState>().editor_connection.connect().await;
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

									load_game_files(&app).await?;
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

							SettingsEvent::ChangeCustomPaths(value) => {
								if let Some(project) = app_state.project.load().as_ref() {
									let mut settings = (*project.settings.load_full()).to_owned();
									settings.custom_paths = value;
									fs::write(project.path.join("project.json"), to_vec(&settings)?)?;
									project.settings.store(settings.into());
								}
							}
						},

						ToolEvent::ContentSearch(event) => match event {
							ContentSearchEvent::Search(query, filetypes, use_qn_format) => {
								start_content_search(&app, query, filetypes, use_qn_format)?;
							}
						}
					},

					Event::Editor(event) => match event {
						EditorEvent::Text(event) => match event {
							TextEditorEvent::Initialise { id } => {
								let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

								let EditorData::Text { content, file_type } = editor_state.data.to_owned() else {
									Err(anyhow!("Editor {} is not a text editor", id))?;
									panic!();
								};

								send_request(
									&app,
									Request::Editor(EditorRequest::Text(TextEditorRequest::ReplaceContent {
										id: id.to_owned(),
										content
									}))
								)?;

								send_request(
									&app,
									Request::Editor(EditorRequest::Text(TextEditorRequest::SetFileType {
										id: id.to_owned(),
										file_type
									}))
								)?;
							}

							TextEditorEvent::UpdateContent { id, content } => {
								let mut editor_state =
									app_state.editor_states.get_mut(&id).context("No such editor")?;

								let EditorData::Text {
									file_type,
									content: old_content
								} = editor_state.data.to_owned()
								else {
									Err(anyhow!("Editor {} is not a text editor", id))?;
									panic!();
								};

								if content != old_content {
									editor_state.data = EditorData::Text { content, file_type };

									send_request(
										&app,
										Request::Global(GlobalRequest::SetTabUnsaved { id, unsaved: true })
									)?;
								}
							}
						},

						EditorEvent::Entity(event) => match event {
							EntityEditorEvent::General(event) => match event {
								EntityGeneralEvent::SetShowReverseParentRefs {
									editor_id,
									show_reverse_parent_refs
								} => {
									let mut editor_state =
										app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

									let settings = match editor_state.data {
										EditorData::QNEntity { ref mut settings, .. } => settings,
										EditorData::QNPatch { ref mut settings, .. } => settings,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									settings.show_reverse_parent_refs = show_reverse_parent_refs;
								}

								EntityGeneralEvent::SetShowChangesFromOriginal {
									editor_id,
									show_changes_from_original
								} => {
									let mut editor_state =
										app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

									let settings = match editor_state.data {
										EditorData::QNEntity { ref mut settings, .. } => settings,
										EditorData::QNPatch { ref mut settings, .. } => settings,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									settings.show_changes_from_original = show_changes_from_original;

									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
											EntityTreeRequest::SetShowDiff {
												editor_id,
												show_diff: show_changes_from_original
											}
										)))
									)?;
								}
							},

							EntityEditorEvent::Tree(event) => match event {
								EntityTreeEvent::Initialise { editor_id } => {
									let editor_state =
										app_state.editor_states.get(&editor_id).context("No such editor")?;

									let entity = match editor_state.data {
										EditorData::QNEntity { ref entity, .. } => entity,
										EditorData::QNPatch { ref current, .. } => current,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									let mut entities = vec![];
									let mut reverse_parent_refs: HashMap<String, Vec<String>> = HashMap::new();

									for (entity_id, entity_data) in entity.entities.iter() {
										match entity_data.parent {
											Ref::Full(ref reference) if reference.external_scene.is_none() => {
												reverse_parent_refs
													.entry(reference.entity_ref.to_owned())
													.and_modify(|x| x.push(entity_id.to_owned()))
													.or_insert(vec![entity_id.to_owned()]);
											}

											Ref::Short(Some(ref reference)) => {
												reverse_parent_refs
													.entry(reference.to_owned())
													.and_modify(|x| x.push(entity_id.to_owned()))
													.or_insert(vec![entity_id.to_owned()]);
											}

											_ => {}
										}
									}

									for (entity_id, entity_data) in entity.entities.iter() {
										entities.push((
											entity_id.to_owned(),
											entity_data.parent.to_owned(),
											entity_data.name.to_owned(),
											entity_data.factory.to_owned(),
											reverse_parent_refs.contains_key(entity_id)
										));
									}

									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::General(
											EntityGeneralRequest::SetIsPatchEditor {
												editor_id: editor_id.to_owned(),
												is_patch_editor: matches!(
													editor_state.data,
													EditorData::QNPatch { .. }
												)
											}
										)))
									)?;

									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
											EntityTreeRequest::NewTree {
												editor_id: editor_id.to_owned(),
												entities
											}
										)))
									)?;

									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
											EntityTreeRequest::SetTemplates {
												editor_id: editor_id.to_owned(),
												templates: from_slice(include_bytes!("../assets/templates.json"))
													.unwrap()
											}
										)))
									)?;

									let editor_connected = app_state.editor_connection.is_connected().await;

									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
											EntityTreeRequest::SetEditorConnectionAvailable {
												editor_id: editor_id.to_owned(),
												editor_connection_available: editor_connected
											}
										)))
									)?;

									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
											EntityMonacoRequest::SetEditorConnected {
												editor_id: editor_id.to_owned(),
												connected: editor_connected
											}
										)))
									)?;

									if let EditorData::QNPatch {
										ref base, ref current, ..
									} = editor_state.data
									{
										send_request(
											&app,
											Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
												EntityTreeRequest::SetDiffInfo {
													editor_id,
													diff_info: get_diff_info(base, current)
												}
											)))
										)?;
									}
								}

								EntityTreeEvent::Select { editor_id, id } => {
									handle_select(&app, editor_id, id).await?;
								}

								EntityTreeEvent::Create { editor_id, id, content } => {
									let mut editor_state =
										app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

									let entity = match editor_state.data {
										EditorData::QNEntity { ref mut entity, .. } => entity,
										EditorData::QNPatch { ref mut current, .. } => current,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									entity.entities.insert(id, content);

									send_request(
										&app,
										Request::Global(GlobalRequest::SetTabUnsaved {
											id: editor_id.to_owned(),
											unsaved: true
										})
									)?;

									if let EditorData::QNPatch {
										ref base, ref current, ..
									} = editor_state.data
									{
										send_request(
											&app,
											Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
												EntityTreeRequest::SetDiffInfo {
													editor_id,
													diff_info: get_diff_info(base, current)
												}
											)))
										)?;
									}
								}

								EntityTreeEvent::Delete { editor_id, id } => {
									handle_delete(&app, editor_id, id).await?;
								}

								EntityTreeEvent::Rename {
									editor_id,
									id,
									new_name
								} => {
									let mut editor_state =
										app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

									let entity = match editor_state.data {
										EditorData::QNEntity { ref mut entity, .. } => entity,
										EditorData::QNPatch { ref mut current, .. } => current,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									entity.entities.get_mut(&id).context("No such entity")?.name = new_name;

									send_request(
										&app,
										Request::Global(GlobalRequest::SetTabUnsaved {
											id: editor_id,
											unsaved: true
										})
									)?;

									let mut buf = Vec::new();
									let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
									let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

									entity
										.entities
										.get(&id)
										.context("No such entity")?
										.serialize(&mut ser)?;

									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
											EntityMonacoRequest::ReplaceContentIfSameEntityID {
												editor_id: editor_id.to_owned(),
												entity_id: id.to_owned(),
												content: String::from_utf8(buf)?
											}
										)))
									)?;

									if let EditorData::QNPatch {
										ref base, ref current, ..
									} = editor_state.data
									{
										send_request(
											&app,
											Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
												EntityTreeRequest::SetDiffInfo {
													editor_id,
													diff_info: get_diff_info(base, current)
												}
											)))
										)?;
									}
								}

								EntityTreeEvent::Reparent {
									editor_id,
									id,
									new_parent
								} => {
									let mut editor_state =
										app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

									let entity = match editor_state.data {
										EditorData::QNEntity { ref mut entity, .. } => entity,
										EditorData::QNPatch { ref mut current, .. } => current,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									entity.entities.get_mut(&id).context("No such entity")?.parent = new_parent;

									send_request(
										&app,
										Request::Global(GlobalRequest::SetTabUnsaved {
											id: editor_id,
											unsaved: true
										})
									)?;

									let mut buf = Vec::new();
									let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
									let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

									entity
										.entities
										.get(&id)
										.context("No such entity")?
										.serialize(&mut ser)?;

									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
											EntityMonacoRequest::ReplaceContentIfSameEntityID {
												editor_id: editor_id.to_owned(),
												entity_id: id.to_owned(),
												content: String::from_utf8(buf)?
											}
										)))
									)?;

									if let EditorData::QNPatch {
										ref base, ref current, ..
									} = editor_state.data
									{
										send_request(
											&app,
											Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
												EntityTreeRequest::SetDiffInfo {
													editor_id,
													diff_info: get_diff_info(base, current)
												}
											)))
										)?;
									}
								}

								EntityTreeEvent::Copy { editor_id, id } => {
									let task = start_task(&app, format!("Copying entity {} and its children", id))?;

									let editor_state =
										app_state.editor_states.get(&editor_id).context("No such editor")?;

									let entity = match editor_state.data {
										EditorData::QNEntity { ref entity, .. } => entity,
										EditorData::QNPatch { ref current, .. } => current,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									let reverse_refs = calculate_reverse_references(entity)?;

									let entities_to_copy = get_recursive_children(entity, &id, &reverse_refs)?
										.into_iter()
										.collect::<HashSet<_>>();

									let data_to_copy = CopiedEntityData {
										root_entity: id.to_owned(),
										data: entity
											.entities
											.iter()
											.filter(|(x, _)| entities_to_copy.contains(*x))
											.map(|(x, y)| (x.to_owned(), y.to_owned()))
											.collect()
									};

									Clipboard::new()?.set_text(to_string(&data_to_copy)?)?;

									finish_task(&app, task)?;

									if let EditorData::QNPatch {
										ref base, ref current, ..
									} = editor_state.data
									{
										send_request(
											&app,
											Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
												EntityTreeRequest::SetDiffInfo {
													editor_id,
													diff_info: get_diff_info(base, current)
												}
											)))
										)?;
									}
								}

								EntityTreeEvent::Paste { editor_id, parent_id } => {
									handle_paste(
										&app,
										editor_id,
										parent_id,
										from_str::<CopiedEntityData>(&Clipboard::new()?.get_text()?)?
									)
									.await?;
								}

								EntityTreeEvent::Search { editor_id, query } => {
									let task = start_task(&app, format!("Searching for {}", query))?;

									let editor_state =
										app_state.editor_states.get(&editor_id).context("No such editor")?;

									let entity = match editor_state.data {
										EditorData::QNEntity { ref entity, .. } => entity,
										EditorData::QNPatch { ref current, .. } => current,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
											EntityTreeRequest::SearchResults {
												editor_id,
												results: entity
													.entities
													.par_iter()
													.filter(|(id, ent)| {
														let mut parent_names = vec![];

														// Get all parent names
														let mut parent_ent = *ent;

														while let Ref::Short(Some(ref x)) = &parent_ent.parent {
															if let Some(next) = entity.entities.get(x) {
																parent_names.push(next.name.to_owned());
																parent_ent = next;
															} else {
																break;
															}
														}

														let mut s = format!(
															"{}{}{}",
															parent_names.join("/"),
															id,
															to_string(ent).unwrap()
														);
														s.make_ascii_lowercase();
														query.split(' ').all(|q| s.contains(q))
													})
													.map(|(id, _)| id.to_owned())
													.collect()
											}
										)))
									)?;

									finish_task(&app, task)?;
								}

								EntityTreeEvent::ShowHelpMenu { editor_id, entity_id } => {
									handle_helpmenu(&app, editor_id, entity_id).await?;
								}

								EntityTreeEvent::UseTemplate {
									editor_id,
									parent_id,
									template
								} => {
									handle_paste(&app, editor_id, parent_id, template).await?;
								}

								EntityTreeEvent::AddGameBrowserItem {
									editor_id,
									parent_id,
									file
								} => {
									handle_gamebrowseradd(&app, editor_id, parent_id, file).await?;
								}

								EntityTreeEvent::SelectEntityInEditor { editor_id, entity_id } => {
									handle_selectentityineditor(&app, editor_id, entity_id).await?;
								}

								EntityTreeEvent::MoveEntityToPlayer { editor_id, entity_id } => {
									handle_moveentitytoplayer(&app, editor_id, entity_id).await?;
								}

								EntityTreeEvent::RotateEntityAsPlayer { editor_id, entity_id } => {
									handle_rotateentityasplayer(&app, editor_id, entity_id).await?;
								}

								EntityTreeEvent::MoveEntityToCamera { editor_id, entity_id } => {
									handle_moveentitytocamera(&app, editor_id, entity_id).await?;
								}

								EntityTreeEvent::RotateEntityAsCamera { editor_id, entity_id } => {
									handle_rotateentityascamera(&app, editor_id, entity_id).await?;
								}

								EntityTreeEvent::RestoreToOriginal { editor_id, entity_id } => {
									handle_restoretooriginal(&app, editor_id, entity_id).await?;
								}
							},

							EntityEditorEvent::Monaco(event) => match event {
								EntityMonacoEvent::UpdateContent {
									editor_id,
									entity_id,
									content
								} => {
									handle_updatecontent(&app, editor_id, entity_id, content).await?;
								}

								EntityMonacoEvent::FollowReference { editor_id, reference } => {
									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
											EntityTreeRequest::Select {
												editor_id,
												id: Some(reference)
											}
										)))
									)?;
								}

								EntityMonacoEvent::OpenFactory { factory, .. } => {
									handle_openfactory(&app, factory).await?;
								}

								EntityMonacoEvent::SignalPin {
									editor_id,
									entity_id,
									pin,
									output
								} => {
									let editor_state =
										app_state.editor_states.get(&editor_id).context("No such editor")?;

									let entity = match editor_state.data {
										EditorData::QNEntity { ref entity, .. } => entity,
										EditorData::QNPatch { ref current, .. } => current,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									app_state
										.editor_connection
										.signal_pin(&entity_id, &entity.blueprint_hash, &pin, output)
										.await?;
								}
							},

							EntityEditorEvent::MetaPane(event) => match event {
								EntityMetaPaneEvent::JumpToReference { editor_id, reference } => {
									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
											EntityTreeRequest::Select {
												editor_id,
												id: Some(reference)
											}
										)))
									)?;
								}

								EntityMetaPaneEvent::SetNotes {
									editor_id,
									entity_id,
									notes
								} => {
									let mut editor_state =
										app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

									let entity = match editor_state.data {
										EditorData::QNEntity { ref mut entity, .. } => entity,
										EditorData::QNPatch { ref mut current, .. } => current,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									// Remove comment referring to given entity
									entity.comments.retain(|x| {
										get_local_reference(&x.parent).map(|x| x != entity_id).unwrap_or(true)
									});

									// Add new comment
									entity.comments.push(CommentEntity {
										parent: Ref::Short(Some(entity_id)),
										name: "Notes".into(),
										text: notes
									});
								}
							},

							EntityEditorEvent::Metadata(event) => {
								handle_entity_metadata_event(&app, event).await?;
							}

							EntityEditorEvent::Overrides(event) => {
								handle_entity_overrides_event(&app, event).await?;
							}
						},

						EditorEvent::ResourceOverview(event) => {
							handle_resource_overview_event(&app, event).await?;
						}

						EditorEvent::RepositoryPatch(event) => {
							handle_repository_patch_event(&app, event).await?;
						}

						EditorEvent::UnlockablesPatch(event) => {
							handle_unlockables_patch_event(&app, event).await?;
						}

						EditorEvent::ContentSearchResults(event) => match event {
							ContentSearchResultsEvent::Initialise { id } => {
								let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

								let results = match editor_state.data {
									EditorData::ContentSearchResults { ref results, .. } => results,

									_ => {
										Err(anyhow!("Editor {} is not a content search results page", id))?;
										panic!();
									}
								};

								send_request(
									&app,
									Request::Editor(EditorRequest::ContentSearchResults(
										ContentSearchResultsRequest::Initialise {
											id,
											results: results.to_owned()
										}
									))
								)?;
							}

							ContentSearchResultsEvent::OpenResourceOverview { hash, .. } => {
								let id = Uuid::new_v4();

								app_state.editor_states.insert(
									id.to_owned(),
									EditorState {
										file: None,
										data: EditorData::ResourceOverview { hash: hash.to_owned() }
									}
								);

								send_request(
									&app,
									Request::Global(GlobalRequest::CreateTab {
										id,
										name: format!("Resource overview ({hash})"),
										editor_type: EditorType::ResourceOverview
									})
								)?;
							}
						}
					},

					Event::Global(event) => match event {
						GlobalEvent::SetSeenAnnouncements(seen_announcements) => {
							let mut settings = (*app_settings.load_full()).to_owned();
							settings.seen_announcements = seen_announcements;
							fs::write(
								app.path_resolver()
									.app_data_dir()
									.context("Couldn't get app data dir")?
									.join("settings.json"),
								to_vec(&settings).unwrap()
							)
							.unwrap();
							app_settings.store(settings.into());
						}

						GlobalEvent::LoadWorkspace(path) => {
							app.track_event("Workspace loaded", None);
							let task = start_task(&app, format!("Loading project {}", path.display()))?;

							let mut files = vec![];

							for entry in WalkDir::new(&path)
								.sort_by_file_name()
								.into_iter()
								.filter_map(|x| x.ok())
							{
								files.push((
									entry.path().into(),
									entry.metadata().context("Couldn't get file metadata")?.is_dir()
								));
							}

							let settings;
							if let Ok(read) = fs::read(path.join("project.json")) {
								if let Ok(read_settings) = from_slice::<ProjectSettings>(&read) {
									settings = read_settings;
								} else {
									settings = ProjectSettings::default();
									fs::write(path.join("project.json"), to_vec(&settings).unwrap()).unwrap();
								}
							} else {
								settings = ProjectSettings::default();
								fs::write(path.join("project.json"), to_vec(&settings).unwrap()).unwrap();
							}

							for editor in app.state::<AppState>().editor_states.iter() {
								if matches!(editor.data, EditorData::QNEntity { .. } | EditorData::QNPatch { .. }) {
									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Metadata(
											EntityMetadataRequest::UpdateCustomPaths {
												editor_id: editor.key().to_owned(),
												custom_paths: settings.custom_paths.to_owned()
											}
										)))
									)?;
								}
							}

							app_state.project.store(Some(
								Project {
									path: path.to_owned(),
									settings: Arc::new(settings.to_owned()).into()
								}
								.into()
							));

							send_request(
								&app,
								Request::Global(GlobalRequest::SetWindowTitle(
									path.file_name().unwrap().to_string_lossy().into()
								))
							)?;

							send_request(
								&app,
								Request::Tool(ToolRequest::Settings(SettingsRequest::ChangeProjectSettings(
									settings.to_owned()
								)))
							)?;

							send_request(
								&app,
								Request::Tool(ToolRequest::FileBrowser(FileBrowserRequest::NewTree {
									base_path: path.to_owned(),
									files
								}))
							)?;

							let notify_path = path.to_owned();
							let notify_app = app.to_owned();

							app_state.fs_watcher.store(Some(
								{
									let mut watcher = notify_debouncer_full::new_debouncer(
										Duration::from_secs(2),
										None,
										move |evts: notify_debouncer_full::DebounceEventResult| {
											if let Err::<_, Error>(e) = try {
												if let Ok(evts) = evts {
													for evt in evts {
														if evt.need_rescan() {
															// Refresh the whole tree

															let mut files = vec![];

															for entry in WalkDir::new(&notify_path)
																.sort_by_file_name()
																.into_iter()
																.filter_map(|x| x.ok())
															{
																files.push((
																	entry.path().into(),
																	entry
																		.metadata()
																		.context("Couldn't get file metadata")?
																		.is_dir()
																));
															}

															send_request(
																&notify_app,
																Request::Tool(ToolRequest::FileBrowser(
																	FileBrowserRequest::NewTree {
																		base_path: notify_path.to_owned(),
																		files
																	}
																))
															)?;

															return;
														}

														match evt.kind {
															notify::EventKind::Create(kind) => match kind {
																notify::event::CreateKind::File => {
																	send_request(
																		&notify_app,
																		Request::Tool(ToolRequest::FileBrowser(
																			FileBrowserRequest::Create {
																				path: evt
																					.paths
																					.first()
																					.context(
																						"Create event had no paths"
																					)?
																					.to_owned(),
																				is_folder: false
																			}
																		))
																	)?;
																}

																notify::event::CreateKind::Folder => {
																	send_request(
																		&notify_app,
																		Request::Tool(ToolRequest::FileBrowser(
																			FileBrowserRequest::Create {
																				path: evt
																					.paths
																					.first()
																					.context(
																						"Create event had no path"
																					)?
																					.to_owned(),
																				is_folder: true
																			}
																		))
																	)?;
																}

																notify::event::CreateKind::Any
																| notify::event::CreateKind::Other => {
																	if let Ok(metadata) = fs::metadata(
																		evt.paths
																			.first()
																			.context("Create event had no paths")?
																	) {
																		send_request(
																			&notify_app,
																			Request::Tool(ToolRequest::FileBrowser(
																				FileBrowserRequest::Create {
																					path: evt
																						.paths
																						.first()
																						.context(
																							"Create event had no paths"
																						)?
																						.to_owned(),
																					is_folder: metadata.is_dir()
																				}
																			))
																		)?;
																	}
																}
															},

															notify::EventKind::Modify(
																notify::event::ModifyKind::Name(
																	notify::event::RenameMode::Both
																)
															) => {
																send_request(
																	&notify_app,
																	Request::Tool(ToolRequest::FileBrowser(
																		FileBrowserRequest::Rename {
																			old_path: evt
																				.paths
																				.first()
																				.context(
																					"Rename-both event had no first \
																					 path"
																				)?
																				.to_owned(),
																			new_path: evt
																				.paths
																				.get(1)
																				.context(
																					"Rename-both event had no second \
																					 path"
																				)?
																				.to_owned()
																		}
																	))
																)?;
															}

															notify::EventKind::Modify(
																notify::event::ModifyKind::Name(
																	notify::event::RenameMode::From
																)
															) => {
																send_request(
																	&notify_app,
																	Request::Tool(ToolRequest::FileBrowser(
																		FileBrowserRequest::BeginRename {
																			old_path: evt
																				.paths
																				.first()
																				.context(
																					"Rename-from event had no path"
																				)?
																				.to_owned()
																		}
																	))
																)?;
															}

															notify::EventKind::Modify(
																notify::event::ModifyKind::Name(
																	notify::event::RenameMode::To
																)
															) => {
																send_request(
																	&notify_app,
																	Request::Tool(ToolRequest::FileBrowser(
																		FileBrowserRequest::FinishRename {
																			new_path: evt
																				.paths
																				.first()
																				.context("Rename-to event had no path")?
																				.to_owned()
																		}
																	))
																)?;
															}

															notify::EventKind::Remove(_) => {
																send_request(
																	&notify_app,
																	Request::Tool(ToolRequest::FileBrowser(
																		FileBrowserRequest::Delete(
																			evt.paths
																				.first()
																				.context("Remove event had no path")?
																				.to_owned()
																		)
																	))
																)?;
															}

															_ => {}
														}
													}
												}
											} {
												send_request(
													&notify_app,
													Request::Global(GlobalRequest::ErrorReport {
														error: format!("{:?}", e.context("Notifier error"))
													})
												)
												.expect("Couldn't send error report to frontend");
											}
										}
									)?;

									watcher.watcher().watch(&path, notify::RecursiveMode::Recursive)?;
									watcher.cache().add_root(&path, notify::RecursiveMode::Recursive);

									watcher
								}
								.into()
							));

							finish_task(&app, task)?;
						}

						GlobalEvent::SelectTab(tab) => {
							if let Some(tab) = tab {
								if let Some(file) = app_state
									.editor_states
									.get(&tab)
									.context("No such editor")?
									.file
									.as_ref()
								{
									send_request(
										&app,
										Request::Tool(ToolRequest::FileBrowser(FileBrowserRequest::Select(Some(
											file.to_owned()
										))))
									)?;
								}
							} else {
								send_request(
									&app,
									Request::Tool(ToolRequest::FileBrowser(FileBrowserRequest::Select(None)))
								)?;
							}
						}

						GlobalEvent::RemoveTab(tab) => {
							let (_, old) = app_state.editor_states.remove(&tab).context("No such editor")?;

							if old.file.is_some() {
								send_request(
									&app,
									Request::Tool(ToolRequest::FileBrowser(FileBrowserRequest::Select(None)))
								)?;
							}

							send_request(&app, Request::Global(GlobalRequest::RemoveTab(tab)))?;
						}

						GlobalEvent::SaveTab(tab) => {
							let mut editor = app_state.editor_states.get_mut(&tab).context("No such editor")?;

							let data_to_save = match &editor.data {
								EditorData::Nil => {
									Err(anyhow!("Editor is a nil editor"))?;
									panic!();
								}

								EditorData::ResourceOverview { .. } => {
									Err(anyhow!("Editor is a resource overview"))?;
									panic!();
								}

								EditorData::ContentSearchResults { .. } => {
									Err(anyhow!("Editor is a content search results page"))?;
									panic!();
								}

								EditorData::Text { content, file_type } => {
									app.track_event(
										"Editor saved",
										Some(json!({
											"file_type": file_type
										}))
									);

									content.as_bytes().to_owned()
								}

								EditorData::QNEntity { entity, settings } => {
									app.track_event(
										"Editor saved",
										Some(json!({
											"file_type": "QNEntity",
											"show_reverse_parent_refs": settings.show_reverse_parent_refs
										}))
									);

									serde_json::to_vec(&entity).context("Entity is invalid")?
								}

								EditorData::QNPatch {
									base,
									current,
									settings
								} => {
									app.track_event(
										"Editor saved",
										Some(json!({
											"file_type": "QNPatch",
											"show_reverse_parent_refs": settings.show_reverse_parent_refs
										}))
									);

									// Once a patch has been saved you can no longer modify the hashes without manually converting to entity.json
									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Metadata(
											EntityMetadataRequest::SetHashModificationAllowed {
												editor_id: tab.to_owned(),
												hash_modification_allowed: false
											}
										)))
									)?;

									serde_json::to_vec(
										&generate_patch(base, current)
											.map_err(|x| anyhow!(x))
											.context("Couldn't generate patch")?
									)
									.context("Entity is invalid")?
								}

								EditorData::RepositoryPatch {
									base,
									current,
									patch_type
								} => {
									app.track_event(
										"Editor saved",
										Some(json!({
											"file_type": "RepositoryPatch",
											"json_patch_type": patch_type
										}))
									);

									match patch_type {
										JsonPatchType::MergePatch => {
											let base = to_value(
												base.iter()
													.map(|x| (x.id.to_owned(), x.data.to_owned()))
													.collect::<HashMap<_, _>>()
											)?;

											let current = to_value(
												current
													.iter()
													.map(|x| (x.id.to_owned(), x.data.to_owned()))
													.collect::<HashMap<_, _>>()
											)?;

											let patch = json_patch::diff(&base, &current);

											let mut merge_patch = json!({});

											for operation in patch.0 {
												match operation {
													json_patch::PatchOperation::Add(json_patch::AddOperation {
														path,
														value
													}) => {
														let mut view = &mut merge_patch;

														if path
															.chars()
															.skip(1)
															.collect::<String>()
															.split('/')
															.last()
															.unwrap()
															.parse::<usize>()
															.is_err()
														{
															for component in
																path.chars().skip(1).collect::<String>().split('/')
															{
																view = view
																	.as_object_mut()
																	.unwrap()
																	.entry(component)
																	.or_insert(json!({}));
															}

															*view = value;
														} else {
															// If the last component is a number we assume it's an array operation, so we replace the whole array with the correct data
															for component in path
																.chars()
																.skip(1)
																.collect::<String>()
																.split('/')
																.collect::<Vec<_>>()
																.into_iter()
																.rev()
																.skip(1)
																.rev()
															{
																view = view
																	.as_object_mut()
																	.unwrap()
																	.entry(component)
																	.or_insert(json!({}));
															}

															*view = current
																.pointer(
																	&path
																		.chars()
																		.skip(1)
																		.collect::<String>()
																		.split('/')
																		.collect::<Vec<_>>()
																		.into_iter()
																		.rev()
																		.skip(1)
																		.rev()
																		.collect::<Vec<_>>()
																		.join("/")
																)
																.unwrap()
																.to_owned();
														}
													}

													json_patch::PatchOperation::Remove(
														json_patch::RemoveOperation { path }
													) => {
														let mut view = &mut merge_patch;

														if path
															.chars()
															.skip(1)
															.collect::<String>()
															.split('/')
															.last()
															.unwrap()
															.parse::<usize>()
															.is_err()
														{
															for component in
																path.chars().skip(1).collect::<String>().split('/')
															{
																view = view
																	.as_object_mut()
																	.unwrap()
																	.entry(component)
																	.or_insert(json!({}));
															}

															*view = Value::Null;
														} else {
															// If the last component is a number we assume it's an array operation, so we replace the whole array with the correct data
															for component in path
																.chars()
																.skip(1)
																.collect::<String>()
																.split('/')
																.collect::<Vec<_>>()
																.into_iter()
																.rev()
																.skip(1)
																.rev()
															{
																view = view
																	.as_object_mut()
																	.unwrap()
																	.entry(component)
																	.or_insert(json!({}));
															}

															*view = current
																.pointer(
																	&path
																		.chars()
																		.skip(1)
																		.collect::<String>()
																		.split('/')
																		.collect::<Vec<_>>()
																		.into_iter()
																		.rev()
																		.skip(1)
																		.rev()
																		.collect::<Vec<_>>()
																		.join("/")
																)
																.unwrap()
																.to_owned();
														}
													}

													json_patch::PatchOperation::Replace(
														json_patch::ReplaceOperation { path, value }
													) => {
														let mut view = &mut merge_patch;

														if path
															.chars()
															.skip(1)
															.collect::<String>()
															.split('/')
															.last()
															.unwrap()
															.parse::<usize>()
															.is_err()
														{
															for component in
																path.chars().skip(1).collect::<String>().split('/')
															{
																view = view
																	.as_object_mut()
																	.unwrap()
																	.entry(component)
																	.or_insert(json!({}));
															}

															*view = value;
														} else {
															// If the last component is a number we assume it's an array operation, so we replace the whole array with the correct data
															for component in path
																.chars()
																.skip(1)
																.collect::<String>()
																.split('/')
																.collect::<Vec<_>>()
																.into_iter()
																.rev()
																.skip(1)
																.rev()
															{
																view = view
																	.as_object_mut()
																	.unwrap()
																	.entry(component)
																	.or_insert(json!({}));
															}

															*view = current
																.pointer(
																	&path
																		.chars()
																		.skip(1)
																		.collect::<String>()
																		.split('/')
																		.collect::<Vec<_>>()
																		.into_iter()
																		.rev()
																		.skip(1)
																		.rev()
																		.collect::<Vec<_>>()
																		.join("/")
																)
																.unwrap()
																.to_owned();
														}
													}

													json_patch::PatchOperation::Move(_) => unreachable!(
														"Calculation of JSON patch does not emit Move operations"
													),

													json_patch::PatchOperation::Copy(_) => unreachable!(
														"Calculation of JSON patch does not emit Copy operations"
													),

													json_patch::PatchOperation::Test(_) => unreachable!(
														"Calculation of JSON patch does not emit Test operations"
													)
												}
											}

											serde_json::to_vec(&merge_patch)?
										}

										JsonPatchType::JsonPatch => {
											let base = to_value(
												base.iter()
													.map(|x| (x.id.to_owned(), x.to_owned()))
													.collect::<HashMap<_, _>>()
											)?;

											let current = to_value(
												current
													.iter()
													.map(|x| (x.id.to_owned(), x.to_owned()))
													.collect::<HashMap<_, _>>()
											)?;

											if let Some(file) = editor.file.as_ref() {
												send_request(
													&app,
													Request::Global(GlobalRequest::ComputeJSONPatchAndSave {
														base,
														current,
														save_path: file.to_owned(),
														file_and_type: ("00204D1AFD76AB13".into(), "REPO".into())
													})
												)?;

												send_request(
													&app,
													Request::Global(GlobalRequest::SetTabUnsaved {
														id: tab,
														unsaved: false
													})
												)?;
											} else {
												let mut dialog = AsyncFileDialog::new().set_title("Save file");

												if let Some(project) = app_state.project.load().as_ref() {
													dialog = dialog.set_directory(&project.path);
												}

												if let Some(save_handle) = dialog
													.add_filter("Repository JSON patch", &["JSON.patch.json"])
													.save_file()
													.await
												{
													editor.file = Some(save_handle.path().into());

													send_request(
														&app,
														Request::Global(GlobalRequest::ComputeJSONPatchAndSave {
															base,
															current,
															save_path: save_handle.path().to_owned(),
															file_and_type: ("00204D1AFD76AB13".into(), "REPO".into())
														})
													)?;

													send_request(
														&app,
														Request::Global(GlobalRequest::SetTabUnsaved {
															id: tab,
															unsaved: false
														})
													)?;
												}
											}

											return;
										}
									}
								}

								EditorData::UnlockablesPatch {
									base,
									current,
									patch_type
								} => {
									app.track_event(
										"Editor saved",
										Some(json!({
											"file_type": "UnlockablesPatch",
											"json_patch_type": patch_type
										}))
									);

									match patch_type {
										JsonPatchType::MergePatch => {
											let base = to_value(
												base.iter()
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
																y.extend(
																	x.data
																		.to_owned()
																		.into_iter()
																		.filter(|(key, _)| key != "Id")
																);
																y
															}
														)
													})
													.collect::<IndexMap<String, IndexMap<String, Value>>>()
											)?;

											let current = to_value(
												current
													.iter()
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
																y.extend(
																	x.data
																		.to_owned()
																		.into_iter()
																		.filter(|(key, _)| key != "Id")
																);
																y
															}
														)
													})
													.collect::<IndexMap<String, IndexMap<String, Value>>>()
											)?;

											let patch = json_patch::diff(&base, &current);

											let mut merge_patch = json!({});

											for operation in patch.0 {
												match operation {
													json_patch::PatchOperation::Add(json_patch::AddOperation {
														path,
														value
													}) => {
														let mut view = &mut merge_patch;

														if path
															.chars()
															.skip(1)
															.collect::<String>()
															.split('/')
															.last()
															.unwrap()
															.parse::<usize>()
															.is_err()
														{
															for component in
																path.chars().skip(1).collect::<String>().split('/')
															{
																view = view
																	.as_object_mut()
																	.unwrap()
																	.entry(component)
																	.or_insert(json!({}));
															}

															*view = value;
														} else {
															// If the last component is a number we assume it's an array operation, so we replace the whole array with the correct data
															for component in path
																.chars()
																.skip(1)
																.collect::<String>()
																.split('/')
																.collect::<Vec<_>>()
																.into_iter()
																.rev()
																.skip(1)
																.rev()
															{
																view = view
																	.as_object_mut()
																	.unwrap()
																	.entry(component)
																	.or_insert(json!({}));
															}

															*view = current
																.pointer(
																	&path
																		.chars()
																		.skip(1)
																		.collect::<String>()
																		.split('/')
																		.collect::<Vec<_>>()
																		.into_iter()
																		.rev()
																		.skip(1)
																		.rev()
																		.collect::<Vec<_>>()
																		.join("/")
																)
																.unwrap()
																.to_owned();
														}
													}

													json_patch::PatchOperation::Remove(
														json_patch::RemoveOperation { path }
													) => {
														let mut view = &mut merge_patch;

														if path
															.chars()
															.skip(1)
															.collect::<String>()
															.split('/')
															.last()
															.unwrap()
															.parse::<usize>()
															.is_err()
														{
															for component in
																path.chars().skip(1).collect::<String>().split('/')
															{
																view = view
																	.as_object_mut()
																	.unwrap()
																	.entry(component)
																	.or_insert(json!({}));
															}

															*view = Value::Null;
														} else {
															// If the last component is a number we assume it's an array operation, so we replace the whole array with the correct data
															for component in path
																.chars()
																.skip(1)
																.collect::<String>()
																.split('/')
																.collect::<Vec<_>>()
																.into_iter()
																.rev()
																.skip(1)
																.rev()
															{
																view = view
																	.as_object_mut()
																	.unwrap()
																	.entry(component)
																	.or_insert(json!({}));
															}

															*view = current
																.pointer(
																	&path
																		.chars()
																		.skip(1)
																		.collect::<String>()
																		.split('/')
																		.collect::<Vec<_>>()
																		.into_iter()
																		.rev()
																		.skip(1)
																		.rev()
																		.collect::<Vec<_>>()
																		.join("/")
																)
																.unwrap()
																.to_owned();
														}
													}

													json_patch::PatchOperation::Replace(
														json_patch::ReplaceOperation { path, value }
													) => {
														let mut view = &mut merge_patch;

														if path
															.chars()
															.skip(1)
															.collect::<String>()
															.split('/')
															.last()
															.unwrap()
															.parse::<usize>()
															.is_err()
														{
															for component in
																path.chars().skip(1).collect::<String>().split('/')
															{
																view = view
																	.as_object_mut()
																	.unwrap()
																	.entry(component)
																	.or_insert(json!({}));
															}

															*view = value;
														} else {
															// If the last component is a number we assume it's an array operation, so we replace the whole array with the correct data
															for component in path
																.chars()
																.skip(1)
																.collect::<String>()
																.split('/')
																.collect::<Vec<_>>()
																.into_iter()
																.rev()
																.skip(1)
																.rev()
															{
																view = view
																	.as_object_mut()
																	.unwrap()
																	.entry(component)
																	.or_insert(json!({}));
															}

															*view = current
																.pointer(
																	&path
																		.chars()
																		.skip(1)
																		.collect::<String>()
																		.split('/')
																		.collect::<Vec<_>>()
																		.into_iter()
																		.rev()
																		.skip(1)
																		.rev()
																		.collect::<Vec<_>>()
																		.join("/")
																)
																.unwrap()
																.to_owned();
														}
													}

													json_patch::PatchOperation::Move(_) => unreachable!(
														"Calculation of JSON patch does not emit Move operations"
													),

													json_patch::PatchOperation::Copy(_) => unreachable!(
														"Calculation of JSON patch does not emit Copy operations"
													),

													json_patch::PatchOperation::Test(_) => unreachable!(
														"Calculation of JSON patch does not emit Test operations"
													)
												}
											}

											serde_json::to_vec(&merge_patch)?
										}

										JsonPatchType::JsonPatch => {
											let base = to_value(
												base.iter()
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
																y.extend(
																	x.data
																		.to_owned()
																		.into_iter()
																		.filter(|(key, _)| key != "Id")
																);
																y
															}
														)
													})
													.collect::<IndexMap<String, IndexMap<String, Value>>>()
											)?;

											let current = to_value(
												current
													.iter()
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
																y.extend(
																	x.data
																		.to_owned()
																		.into_iter()
																		.filter(|(key, _)| key != "Id")
																);
																y
															}
														)
													})
													.collect::<IndexMap<String, IndexMap<String, Value>>>()
											)?;

											if let Some(file) = editor.file.as_ref() {
												send_request(
													&app,
													Request::Global(GlobalRequest::ComputeJSONPatchAndSave {
														base,
														current,
														save_path: file.to_owned(),
														file_and_type: ("0057C2C3941115CA".into(), "ORES".into())
													})
												)?;

												send_request(
													&app,
													Request::Global(GlobalRequest::SetTabUnsaved {
														id: tab,
														unsaved: false
													})
												)?;
											} else {
												let mut dialog = AsyncFileDialog::new().set_title("Save file");

												if let Some(project) = app_state.project.load().as_ref() {
													dialog = dialog.set_directory(&project.path);
												}

												if let Some(save_handle) = dialog
													.add_filter("Unlockables JSON patch", &["JSON.patch.json"])
													.save_file()
													.await
												{
													editor.file = Some(save_handle.path().into());

													send_request(
														&app,
														Request::Global(GlobalRequest::ComputeJSONPatchAndSave {
															base,
															current,
															save_path: save_handle.path().to_owned(),
															file_and_type: ("0057C2C3941115CA".into(), "ORES".into())
														})
													)?;

													send_request(
														&app,
														Request::Global(GlobalRequest::SetTabUnsaved {
															id: tab,
															unsaved: false
														})
													)?;
												}
											}

											return;
										}
									}
								}
							};

							if let Some(file) = editor.file.as_ref() {
								fs::write(file, data_to_save).context("Couldn't write file")?;

								send_request(
									&app,
									Request::Global(GlobalRequest::SetTabUnsaved {
										id: tab,
										unsaved: false
									})
								)?;
							} else {
								let mut dialog = AsyncFileDialog::new().set_title("Save file");

								if let Some(project) = app_state.project.load().as_ref() {
									dialog = dialog.set_directory(&project.path);
								}

								if let Some(save_handle) = dialog
									.add_filter(
										match &editor.data {
											EditorData::Nil => {
												Err(anyhow!("Editor is a nil editor"))?;
												panic!();
											}

											EditorData::ResourceOverview { .. } => {
												Err(anyhow!("Editor is a resource overview"))?;
												panic!();
											}

											EditorData::ContentSearchResults { .. } => {
												Err(anyhow!("Editor is a content search results page"))?;
												panic!();
											}

											EditorData::Text {
												file_type: TextFileType::PlainText,
												..
											} => "Text file",

											EditorData::Text {
												file_type: TextFileType::Markdown,
												..
											} => "Markdown file",

											EditorData::Text {
												file_type: TextFileType::Json | TextFileType::ManifestJson,
												..
											} => "JSON file",

											EditorData::QNEntity { .. } => "QuickEntity entity",

											EditorData::QNPatch { .. } => "QuickEntity patch",

											EditorData::RepositoryPatch { patch_type, .. } => match patch_type {
												JsonPatchType::MergePatch => "Repository merge patch",
												JsonPatchType::JsonPatch => "Repository JSON patch"
											},

											EditorData::UnlockablesPatch { patch_type, .. } => match patch_type {
												JsonPatchType::MergePatch => "Unlockables merge patch",
												JsonPatchType::JsonPatch => "Unlockables JSON patch"
											}
										},
										&[match &editor.data {
											EditorData::Nil => {
												Err(anyhow!("Editor is a nil editor"))?;
												panic!();
											}

											EditorData::ResourceOverview { .. } => {
												Err(anyhow!("Editor is a resource overview"))?;
												panic!();
											}

											EditorData::ContentSearchResults { .. } => {
												Err(anyhow!("Editor is a content search results page"))?;
												panic!();
											}

											EditorData::Text {
												file_type: TextFileType::PlainText,
												..
											} => "txt",

											EditorData::Text {
												file_type: TextFileType::Markdown,
												..
											} => "md",

											EditorData::Text {
												file_type: TextFileType::Json | TextFileType::ManifestJson,
												..
											} => "json",

											EditorData::QNEntity { .. } => "entity.json",

											EditorData::QNPatch { .. } => "entity.patch.json",

											EditorData::RepositoryPatch { patch_type, .. } => match patch_type {
												JsonPatchType::MergePatch => "repository.json",
												JsonPatchType::JsonPatch => "JSON.patch.json"
											},

											EditorData::UnlockablesPatch { patch_type, .. } => match patch_type {
												JsonPatchType::MergePatch => "unlockables.json",
												JsonPatchType::JsonPatch => "JSON.patch.json"
											}
										}]
									)
									.save_file()
									.await
								{
									editor.file = Some(save_handle.path().into());

									fs::write(save_handle.path(), data_to_save).context("Couldn't write file")?;

									send_request(
										&app,
										Request::Global(GlobalRequest::SetTabUnsaved {
											id: tab,
											unsaved: false
										})
									)?;
								}
							}
						}
					},

					Event::EditorConnection(event) => match event {
						EditorConnectionEvent::EntitySelected(id, tblu) => {
							for editor in app.state::<AppState>().editor_states.iter() {
								let entity = match editor.data {
									EditorData::QNEntity { ref entity, .. } => entity,
									EditorData::QNPatch { ref current, .. } => current,

									_ => continue
								};

								if entity.blueprint_hash == tblu {
									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
											EntityTreeRequest::Select {
												editor_id: editor.key().to_owned(),
												id: entity.entities.contains_key(&id).then_some(id.to_owned())
											}
										)))
									)?;
								}
							}
						}

						EditorConnectionEvent::EntityTransformUpdated(id, tblu, transform) => {
							let mut qn_editors = vec![];
							for editor in app_state.editor_states.iter() {
								if let EditorData::QNEntity { .. } | EditorData::QNPatch { .. } = editor.data {
									qn_editors.push(editor.key().to_owned());
								}
							}

							for editor_id in qn_editors {
								let mut editor_state = app_state.editor_states.get_mut(&editor_id).unwrap();
								let entity = match editor_state.data {
									EditorData::QNEntity { ref mut entity, .. } => entity,
									EditorData::QNPatch { ref mut current, .. } => current,

									_ => continue
								};

								if entity.blueprint_hash == tblu
									&& let Some(sub_entity) = entity.entities.get_mut(&id)
								{
									sub_entity.properties.get_or_insert_default().insert(
										"m_mTransform".into(),
										Property {
											property_type: "SMatrix43".into(),
											value: to_value(&transform)?,
											post_init: None
										}
									);

									send_request(
										&app,
										Request::Global(GlobalRequest::SetTabUnsaved {
											id: editor_id.to_owned(),
											unsaved: true
										})
									)?;

									let mut buf = Vec::new();
									let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
									let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

									entity
										.entities
										.get(&id)
										.context("No such entity")?
										.serialize(&mut ser)?;

									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
											EntityMonacoRequest::ReplaceContentIfSameEntityID {
												editor_id: editor_id.to_owned(),
												entity_id: id.to_owned(),
												content: String::from_utf8(buf)?
											}
										)))
									)?;

									if let EditorData::QNPatch {
										ref base, ref current, ..
									} = editor_state.data
									{
										send_request(
											&app,
											Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
												EntityTreeRequest::SetDiffInfo {
													editor_id,
													diff_info: get_diff_info(base, current)
												}
											)))
										)?;
									}
								}
							}
						}

						EditorConnectionEvent::EntityPropertyChanged(
							id,
							tblu,
							property_name,
							property_type,
							property_value
						) => {
							let mut qn_editors = vec![];
							for editor in app_state.editor_states.iter() {
								if let EditorData::QNEntity { .. } | EditorData::QNPatch { .. } = editor.data {
									qn_editors.push(editor.key().to_owned());
								}
							}

							for editor_id in qn_editors {
								let mut editor_state = app_state.editor_states.get_mut(&editor_id).unwrap();
								let entity = match editor_state.data {
									EditorData::QNEntity { ref mut entity, .. } => entity,
									EditorData::QNPatch { ref mut current, .. } => current,

									_ => continue
								};

								if entity.blueprint_hash == tblu && entity.entities.contains_key(&id) {
									let post_init = if let Some(intellisense) = app_state.intellisense.load().as_ref()
										&& let Some(game_files) = app_state.game_files.load().as_ref()
										&& let Some(hash_list) = app_state.hash_list.load().as_ref()
										&& let Some(install) = app_settings.load().game_install.as_ref()
									{
										let game_version = app_state
											.game_installs
											.iter()
											.try_find(|x| anyhow::Ok(x.path == *install))?
											.context("No such game install")?
											.version;

										if let Some((_, _, _, post_init)) = intellisense
											.get_properties(
												game_files,
												&app_state.cached_entities,
												hash_list,
												game_version,
												&entity,
												&id,
												true
											)?
											.into_iter()
											.find(|(name, _, _, _)| *name == property_name)
										{
											post_init.then_some(true)
										} else {
											None
										}
									} else {
										None
									};

									let Some(sub_entity) = entity.entities.get_mut(&id) else {
										unreachable!();
									};

									sub_entity.properties.get_or_insert_default().insert(
										property_name.to_owned(),
										Property {
											property_type: property_type.to_owned(),
											value: property_value.to_owned(),
											post_init
										}
									);

									send_request(
										&app,
										Request::Global(GlobalRequest::SetTabUnsaved {
											id: editor_id.to_owned(),
											unsaved: true
										})
									)?;

									let mut buf = Vec::new();
									let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
									let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

									entity
										.entities
										.get(&id)
										.context("No such entity")?
										.serialize(&mut ser)?;

									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
											EntityMonacoRequest::ReplaceContentIfSameEntityID {
												editor_id: editor_id.to_owned(),
												entity_id: id.to_owned(),
												content: String::from_utf8(buf)?
											}
										)))
									)?;

									if let EditorData::QNPatch {
										ref base, ref current, ..
									} = editor_state.data
									{
										send_request(
											&app,
											Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
												EntityTreeRequest::SetDiffInfo {
													editor_id,
													diff_info: get_diff_info(base, current)
												}
											)))
										)?;
									}
								}
							}
						}
					}
				}
			} {
				app.track_event("Error", Some(json!({ "error": format!("{:?}", e) })));
				send_request(
					&app,
					Request::Global(GlobalRequest::ErrorReport {
						error: format!("{:?}", e)
					})
				)
				.expect("Couldn't send error report to frontend");
			}
		})
		.await
		{
			let error = match e {
				tauri::Error::JoinError(x) if x.is_panic() => {
					let x = x.into_panic();
					let payload = x
						.downcast_ref::<String>()
						.map(String::as_str)
						.or_else(|| x.downcast_ref::<&str>().cloned())
						.unwrap_or("<non string panic payload>");

					format!("Thread panic: {}", payload)
				}

				_ => format!("{:?}", e)
			};

			cloned_app.track_event("Error", Some(json!({ "error": error.to_owned() })));
			send_request(&cloned_app, Request::Global(GlobalRequest::ErrorReport { error }))
				.expect("Couldn't send error report to frontend");
		}
	});
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

		let game_version = app_state
			.game_installs
			.iter()
			.find(|x| x.path == *path)
			.context("No such game install")?
			.version;

		let thumbs = IniFileSystem::from(path.join("thumbs.dat")).context("Couldn't load thumbs.dat")?;

		let thumbs = thumbs
			.get_root()
			.get_section("application")
			.context("Couldn't get application section")?;

		let (Some(proj_path), Some(relative_runtime_path)) =
			(thumbs.get_option("PROJECT_PATH"), thumbs.get_option("RUNTIME_PATH"))
		else {
			bail!("thumbs.dat was missing required properties");
		};

		let mut partition_manager = PartitionManager::new(path.join(proj_path).join(relative_runtime_path));

		let mut partitions = match game_version {
			GameVersion::H1 => PackageDefinitionSource::HM2016(fs::read(
				path.join(proj_path)
					.join(relative_runtime_path)
					.join("packagedefinition.txt")
			)?)
			.read()
			.context("Couldn't read packagedefinition")?,

			GameVersion::H2 => PackageDefinitionSource::HM2(fs::read(
				path.join(proj_path)
					.join(relative_runtime_path)
					.join("packagedefinition.txt")
			)?)
			.read()
			.context("Couldn't read packagedefinition")?,

			GameVersion::H3 => PackageDefinitionSource::HM3(fs::read(
				path.join(proj_path)
					.join(relative_runtime_path)
					.join("packagedefinition.txt")
			)?)
			.read()
			.context("Couldn't read packagedefinition")?
		};

		if !app_settings.load().extract_modded_files {
			for partition in &mut partitions {
				partition.patch_level = 9;
			}
		}

		finish_task(app, task)?;

		let partition_names = partitions.iter().map(|x| x.id.to_string()).collect_vec();

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

		let mut reverse_dependencies: DashMap<String, Vec<String>> = DashMap::new();

		// Ensure we only get the references from the lowest chunk version of each resource (matches the rest of GK's behaviour)
		let resources = partition_manager
			.get_all_partitions()
			.into_par_iter()
			.rev()
			.flat_map(|partition| {
				partition
					.get_latest_resources()
					.into_par_iter()
					.map(|(resource, _)| (resource.get_rrid(), resource.get_all_references()))
			})
			.collect::<HashMap<_, _>>();

		reverse_dependencies
			.try_reserve(resources.len())
			.map_err(|e| anyhow!("Reserve error: {e:?}"))?;

		reverse_dependencies.par_extend(
			resources
				.par_iter()
				.map(|(x, _)| (x.to_hex_string(), Default::default()))
		);

		resources
			.into_par_iter()
			.flat_map(|(resource_id, resource_references)| {
				let res_id_str = resource_id.to_hex_string();

				resource_references
					.par_iter()
					.map(move |(reference_id, _)| (reference_id.to_hex_string(), res_id_str.to_owned()))
			})
			.for_each(|(key, value)| {
				if let Some(mut x) = reverse_dependencies.get_mut(&key) {
					x.push(value);
				}
			});

		app_state.game_files.store(Some(partition_manager.into()));

		app_state.resource_reverse_dependencies.store(Some(
			reverse_dependencies
				.into_par_iter()
				.map(|(x, y)| (x, y.into_iter().dedup().collect()))
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
				.parse::<u16>()
				.context("Online hash list version wasn't a number")?;

			if current_version < new_version {
				if let Ok(data) = reqwest::get(HASH_LIST_ENDPOINT).await {
					if let Ok(data) = data.bytes().await {
						let hash_list = HashList::from_slice(&data)?;

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
				all_cppts: hash_list
					.entries
					.iter()
					.filter(|(_, entry)| entry.resource_type == "CPPT")
					.filter(|(hash, _)| resource_reverse_dependencies.contains_key(*hash))
					.map(|(hash, _)| hash.to_owned())
					.collect(),
				all_asets: hash_list
					.entries
					.iter()
					.filter(|(_, entry)| entry.resource_type == "ASET")
					.filter(|(hash, _)| resource_reverse_dependencies.contains_key(*hash))
					.map(|(hash, _)| hash.to_owned())
					.collect(),
				all_uicts: hash_list
					.entries
					.iter()
					.filter(|(_, entry)| entry.resource_type == "UICT")
					.filter(|(hash, _)| resource_reverse_dependencies.contains_key(*hash))
					.map(|(hash, _)| hash.to_owned())
					.collect(),
				all_matts: hash_list
					.entries
					.iter()
					.filter(|(_, entry)| entry.resource_type == "MATT")
					.filter(|(hash, _)| resource_reverse_dependencies.contains_key(*hash))
					.map(|(hash, _)| hash.to_owned())
					.collect(),
				all_wswts: hash_list
					.entries
					.iter()
					.filter(|(_, entry)| entry.resource_type == "WSWT")
					.filter(|(hash, _)| resource_reverse_dependencies.contains_key(*hash))
					.map(|(hash, _)| hash.to_owned())
					.collect(),
				all_ecpts: hash_list
					.entries
					.iter()
					.filter(|(_, entry)| entry.resource_type == "ECPT")
					.filter(|(hash, _)| resource_reverse_dependencies.contains_key(*hash))
					.map(|(hash, _)| hash.to_owned())
					.collect(),
				all_aibxs: hash_list
					.entries
					.iter()
					.filter(|(_, entry)| entry.resource_type == "AIBX")
					.filter(|(hash, _)| resource_reverse_dependencies.contains_key(*hash))
					.map(|(hash, _)| hash.to_owned())
					.collect(),
				all_wsgts: hash_list
					.entries
					.iter()
					.filter(|(_, entry)| entry.resource_type == "WSGT")
					.filter(|(hash, _)| resource_reverse_dependencies.contains_key(*hash))
					.map(|(hash, _)| hash.to_owned())
					.collect()
			}
			.into()
		));

		finish_task(app, task)?
	};

	if let Some(game_files) = app_state.game_files.load().as_ref()
		&& let Some(hash_list) = app_state.hash_list.load().as_ref()
	{
		let task = start_task(app, "Caching repository")?;

		app_state.repository.store(Some(
			from_slice::<Vec<RepositoryItem>>(&extract_latest_resource(game_files, hash_list, "00204D1AFD76AB13")?.1)?
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
			if let EditorData::ResourceOverview { ref hash } = editor.data {
				let task = start_task(app, format!("Refreshing resource overview for {}", hash))?;

				let game_version = app_state
					.game_installs
					.iter()
					.try_find(|x| anyhow::Ok(x.path == *install))?
					.context("No such game install")?
					.version;

				initialise_resource_overview(
					app,
					&app_state,
					editor.key().to_owned(),
					hash,
					game_files,
					game_version,
					resource_reverse_dependencies,
					install,
					hash_list
				)?;

				finish_task(app, task)?;
			}
		}

		finish_task(app, task)?;
	}
}

#[try_fn]
#[context("Couldn't send task start event for {:?} to frontend", name.as_ref())]
pub fn start_task(app: &AppHandle, name: impl AsRef<str>) -> Result<Uuid> {
	let task_id = Uuid::new_v4();
	app.emit_all("start-task", (&task_id, name.as_ref()))?;
	task_id
}

#[try_fn]
#[context("Couldn't send task finish event for {:?} to frontend", task)]
pub fn finish_task(app: &AppHandle, task: Uuid) -> Result<()> {
	app.emit_all("finish-task", &task)?;
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub enum NotificationKind {
	Error,
	Info,
	Success,
	Warning
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Notification {
	pub kind: NotificationKind,
	pub title: String,
	pub subtitle: String
}

#[try_fn]
#[context("Couldn't send notification {:?} to frontend", notification)]
pub fn send_notification(app: &AppHandle, notification: Notification) -> Result<()> {
	app.emit_all("send-notification", (Uuid::new_v4(), &notification))?;
}

#[try_fn]
#[context("Couldn't send request {:?} to frontend", request)]
pub fn send_request(app: &AppHandle, request: Request) -> Result<()> {
	app.emit_all("request", &request)?;
}
