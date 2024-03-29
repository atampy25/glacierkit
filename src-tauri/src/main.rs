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

pub mod entity;
pub mod event_handling;
pub mod game_detection;
pub mod hash_list;
pub mod intellisense;
pub mod material;
pub mod model;
pub mod resourcelib;
pub mod rpkg;
pub mod rpkg_tool;
pub mod show_in_folder;

use std::{
	collections::{HashMap, HashSet},
	fs::{self, File},
	future::Future,
	io::{BufReader, Cursor},
	ops::Deref,
	path::{Path, PathBuf},
	sync::Arc
};

use anyhow::{anyhow, Context, Error, Result};
use arboard::Clipboard;
use arc_swap::ArcSwap;
use binrw::BinReaderExt;
use entity::{
	calculate_reverse_references, check_local_references_exist, get_decorations, get_local_reference,
	get_recursive_children, is_valid_entity_blueprint, is_valid_entity_factory, random_entity_id, CopiedEntityData,
	ReverseReferenceData
};
use event_handling::{
	entity_overrides::send_overrides_decorations,
	entity_tree::{handle_delete, handle_paste}
};
use fn_error_context::context;
use game_detection::{detect_installs, GameVersion};
use hash_list::HashList;
use indexmap::IndexMap;
use intellisense::Intellisense;
use itertools::Itertools;
use memmap2::Mmap;
use model::{
	AppSettings, AppState, EditorData, EditorEvent, EditorRequest, EditorState, EditorType, EditorValidity,
	EntityEditorEvent, EntityEditorRequest, EntityGeneralEvent, EntityMetaPaneEvent, EntityMetaPaneRequest,
	EntityMetadataEvent, EntityMetadataRequest, EntityMonacoEvent, EntityMonacoRequest, EntityOverridesEvent,
	EntityOverridesRequest, EntityTreeEvent, EntityTreeRequest, Event, FileBrowserEvent, FileBrowserRequest,
	GameBrowserEntry, GameBrowserEvent, GameBrowserRequest, GlobalEvent, GlobalRequest, Project, ProjectSettings,
	Request, ResourceOverviewData, ResourceOverviewEvent, ResourceOverviewRequest, SettingsEvent, SettingsRequest,
	TextEditorEvent, TextEditorRequest, TextFileType, ToolEvent, ToolRequest
};
use notify::Watcher;
use quickentity_rs::{
	apply_patch, convert_2016_blueprint_to_modern, convert_2016_factory_to_modern, convert_to_qn, convert_to_rt,
	generate_patch,
	patch_structs::Patch,
	qn_structs::{CommentEntity, Entity, Ref, SubEntity, SubType}
};
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, ParallelIterator};
use resourcelib::{
	h2016_convert_binary_to_blueprint, h2016_convert_binary_to_factory, h2_convert_binary_to_blueprint,
	h2_convert_binary_to_factory, h3_convert_binary_to_blueprint, h3_convert_binary_to_factory
};
use rfd::AsyncFileDialog;
use rpkg::{ensure_entity_in_cache, extract_latest_overview_info, extract_latest_resource, normalise_to_hash};
use rpkg_rs::{
	misc::ini_file_system::IniFileSystem,
	runtime::resource::{
		package_manager::PackageManager, resource_container::ResourceContainer, resource_package::ResourcePackage
	}
};
use rpkg_tool::generate_rpkg_meta;
use serde::{Deserialize, Serialize};
use serde_json::{from_slice, from_str, json, to_string, to_vec};
use show_in_folder::show_in_folder;
use syntect::{highlighting::ThemeSet, html::highlighted_html_for_string, parsing::SyntaxSet};
use tauri::{async_runtime, AppHandle, Manager, State};
use tauri_plugin_aptabase::{EventTracker, InitOptions};
use tokio::sync::RwLock;
use tryvial::try_fn;
use uuid::Uuid;
use velcro::vec;
use walkdir::WalkDir;

const HASH_LIST_VERSION_ENDPOINT: &str =
	"https://github.com/glacier-modding/Hitman-Hashes/releases/latest/download/version";

const HASH_LIST_ENDPOINT: &str =
	"https://github.com/glacier-modding/Hitman-Hashes/releases/latest/download/hash_list.sml";

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

			app.manage(AppState {
				game_installs: detect_installs().expect("Couldn't detect game installs"),
				project: None.into(),
				hash_list: fs::read(app_data_path.join("hash_list.sml"))
					.ok()
					.and_then(|x| serde_smile::from_slice(&x).ok())
					.into(),
				fs_watcher: None.into(),
				editor_states: RwLock::new(HashMap::new()).into(),
				resource_packages: None.into(),
				resource_reverse_dependencies: None.into(),
				cached_entities: parking_lot::RwLock::new(HashMap::new()).into(),
				intellisense: None.into()
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

fn spawn_fallible<F>(app: &AppHandle, task: F)
where
	F: Future<Output = Result<()>> + Send + 'static
{
	let cloned_app1 = app.clone();
	let cloned_app2 = app.clone();

	async_runtime::spawn(async move {
		if let Err(e) = async_runtime::spawn(async move {
			if let Err::<_, Error>(e) = task.await {
				send_request(
					&cloned_app1,
					Request::Global(GlobalRequest::ErrorReport {
						error: format!("{:?}", e)
					})
				)
				.expect("Couldn't send error report to frontend");
			}
		})
		.await
		{
			send_request(
				&cloned_app2,
				Request::Global(GlobalRequest::ErrorReport {
					error: match e {
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
					}
				})
			)
			.expect("Couldn't send error report to frontend");
		}
	});
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
										let guard = app_state.editor_states.read().await;

										guard
											.iter()
											.find(|(_, x)| x.file.as_ref().map(|x| x == &path).unwrap_or(false))
											.map(|(x, _)| x.to_owned())
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

												// Normalise comments to form used by Deeznuts (single comment for each entity)
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

												app_state.editor_states.write().await.insert(
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

												let patch: Patch =
													from_slice(&fs::read(&path).context("Couldn't read file")?)
														.context("Invalid entity")?;

												if let Some(resource_packages) =
													app_state.resource_packages.load().as_ref() && let Some(install) =
													app_settings.load().game_install.as_ref() && let Some(hash_list) =
													app_state.hash_list.load().as_ref()
												{
													ensure_entity_in_cache(
														resource_packages,
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
														.read()
														.get(&normalise_to_hash(patch.factory_hash.to_owned()))
														.unwrap()
														.to_owned();

													let base = entity.to_owned();

													apply_patch(&mut entity, patch, true)
														.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

													// Normalise comments to form used by Deeznuts (single comment for each entity)
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

													app_state.editor_states.write().await.insert(
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

												app_state.editor_states.write().await.insert(
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

												app_state.editor_states.write().await.insert(
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

												app_state.editor_states.write().await.insert(
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

											_ => {
												// Unsupported extension

												let id = Uuid::new_v4();

												app_state.editor_states.write().await.insert(
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

										// Normalise comments to form used by Deeznuts (single comment for each entity)
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

										if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
											&& let Some(install) = app_settings.load().game_install.as_ref()
											&& let Some(hash_list) = app_state.hash_list.load().as_ref()
										{
											ensure_entity_in_cache(
												resource_packages,
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
												.read()
												.get(&normalise_to_hash(patch.factory_hash.to_owned()))
												.unwrap()
												.to_owned();

											let base = entity.to_owned();

											apply_patch(&mut entity, patch, true)
												.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

											// Normalise comments to form used by Deeznuts (single comment for each entity)
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
								if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
									&& let Some(install) = app_settings.load().game_install.as_ref()
									&& let Some(hash_list) = app_state.hash_list.load().as_ref()
								{
									let mut entity: Entity =
										from_slice(&fs::read(&path).context("Couldn't read file")?)
											.context("Invalid entity")?;

									// Normalise comments to form used by Deeznuts (single comment for each entity)
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
										resource_packages,
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
										extract_latest_resource(resource_packages, hash_list, blueprint_hash)?;

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

								if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
									&& let Some(install) = app_settings.load().game_install.as_ref()
									&& let Some(hash_list) = app_state.hash_list.load().as_ref()
								{
									ensure_entity_in_cache(
										resource_packages,
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
										.read()
										.get(&normalise_to_hash(patch.factory_hash.to_owned()))
										.unwrap()
										.to_owned();

									apply_patch(&mut entity, patch, true)
										.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

									// Normalise comments to form used by Deeznuts (single comment for each entity)
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
						},

						ToolEvent::GameBrowser(event) => match event {
							GameBrowserEvent::Select(hash) => {
								let id = Uuid::new_v4();

								app_state.editor_states.write().await.insert(
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

							GameBrowserEvent::Search(query) => {
								let task = start_task(&app, format!("Searching game files for {}", query))?;

								if let Some(install) = app_settings.load().game_install.as_ref() {
									let install = app_state
										.game_installs
										.iter()
										.find(|x| x.path == *install)
										.context("No such game install as specified in project.json")?;

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
												entries: hash_list
													.entries
													.iter()
													.filter(|(_, entry)| entry.games.contains(install.version))
													.filter(|(_, entry)| {
														![
															"TBLU", "CBLU", "ASEB", "UICB", "MATB", "WSWB", "DSWB",
															"ECPB", "WSGB"
														]
														.iter()
														.any(|x| *x == entry.resource_type)
													})
													.filter(|(hash, entry)| {
														query.split(' ').all(|y| {
															entry
																.path
																.as_deref()
																.unwrap_or("")
																.to_lowercase()
																.contains(y) || entry
																.hint
																.as_deref()
																.unwrap_or("")
																.to_lowercase()
																.contains(y) || hash.to_lowercase().contains(y)
														})
													})
													.map(|(hash, entry)| GameBrowserEntry {
														hash: hash.to_owned(),
														path: entry.path.to_owned(),
														hint: entry.hint.to_owned(),
														filetype: entry.resource_type.to_owned()
													})
													.collect()
											}))
										)?;
									}
								}

								finish_task(&app, task)?;
							}

							GameBrowserEvent::OpenInEditor(hash) => {
								// Only available for entities currently

								if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
									&& let Some(install) = app_settings.load().game_install.as_ref()
									&& let Some(hash_list) = app_state.hash_list.load().as_ref()
								{
									let task = start_task(&app, format!("Loading entity {}", hash))?;

									let game_install_data = app_state
										.game_installs
										.iter()
										.try_find(|x| anyhow::Ok(x.path == *install))?
										.context("No such game install")?;

									ensure_entity_in_cache(
										resource_packages,
										&app_state.cached_entities,
										game_install_data.version,
										hash_list,
										&hash
									)?;

									let entity = app_state.cached_entities.read().get(&hash).unwrap().to_owned();

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

									app_state.editor_states.write().await.insert(
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
							}
						},

						ToolEvent::Settings(event) => match event {
							SettingsEvent::Initialise => {
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

								if let Some(path) = app_settings.load().game_install.as_ref() {
									let task = start_task(&app, "Loading game files")?;

									let game_version = app_state
										.game_installs
										.iter()
										.find(|x| x.path == *path)
										.context("No such game install")?
										.version;

									let mut thumbs = IniFileSystem::new();
									thumbs.load(&path.join("thumbs.dat"))?;

									let mut resource_packages: IndexMap<PathBuf, ResourcePackage> = IndexMap::new();

									if let (Ok(proj_path), Ok(relative_runtime_path)) = (
										thumbs.get_value("application", "PROJECT_PATH"),
										thumbs.get_value("application", "RUNTIME_PATH")
									) {
										let mut package_manager =
											PackageManager::new(&path.join(proj_path).join(relative_runtime_path));

										let mut resource_container = ResourceContainer::default();

										package_manager.initialize(&mut resource_container).map_err(|x| {
											anyhow!("RPKG error in initialising resource container: {:?}", x)
										})?;

										for partition in package_manager.partition_infos {
											resource_packages.extend(
												vec![
													0,
													..ResourceContainer::get_patch_indices(
														&package_manager.runtime_dir,
														partition.index
													)
													.map_err(|x| {
														anyhow!("RPKG error in getting patch indices: {:?}", x)
													})?
													.iter()
													.filter(|&&x| x <= 9 || app_settings.load().extract_modded_files),
												]
												.into_par_iter()
												.rev()
												.map(|patch| {
													anyhow::Ok({
														let rpkg_path =
															Path::new(&package_manager.runtime_dir).join(format!(
																"{}{}{}.rpkg",
																match game_version {
																	GameVersion::H1 | GameVersion::H2 =>
																		if partition.index > 0 {
																			"dlc"
																		} else {
																			"chunk"
																		},

																	GameVersion::H3 => "chunk"
																},
																match game_version {
																	GameVersion::H1 | GameVersion::H2 =>
																		if partition.index > 0 {
																			partition.index - 1 // H1/H2 go chunk0, dlc0, dlc1
																		} else {
																			partition.index
																		},
																	GameVersion::H3 => partition.index
																},
																if patch > 0 {
																	format!("patch{}", patch)
																} else {
																	"".into()
																}
															));

														(rpkg_path.to_owned(), {
															let package_file = File::open(&rpkg_path)?;

															let mmap = unsafe { Mmap::map(&package_file)? };
															let mut reader = Cursor::new(&mmap[..]);

															reader
																.read_ne_args((patch > 0,))
																.context("Couldn't parse RPKG file")?
														})
													})
												})
												.collect::<Result<Vec<_>>>()?
											);
										}
									} else {
										Err(anyhow!("thumbs.dat was missing required properties"))?;
										panic!();
									}

									finish_task(&app, task)?;

									let task = start_task(&app, "Caching reverse references")?;

									let mut reverse_dependencies: HashMap<String, HashSet<String>> = HashMap::new();

									for rpkg in resource_packages.values() {
										for (resource_header, offset_info) in
											rpkg.resource_entries.iter().enumerate().map(|(index, entry)| {
												(rpkg.resource_metadata.get(index).unwrap(), entry)
											}) {
											for reference in resource_header
												.m_references
												.as_ref()
												.map(|x| &x.reference_hash)
												.unwrap_or(&Vec::new())
											{
												reverse_dependencies
													.entry(reference.to_hex_string())
													.or_default()
													.insert(offset_info.runtime_resource_id.to_hex_string());
											}
										}
									}

									app_state.resource_packages.store(Some(resource_packages.into()));

									app_state.resource_reverse_dependencies.store(Some(
										reverse_dependencies
											.into_iter()
											.map(|(x, y)| (x, y.into_iter().collect()))
											.collect::<HashMap<_, _>>()
											.into()
									));

									finish_task(&app, task)?;
								}

								let task = start_task(&app, "Acquiring latest hash list")?;

								let current_version =
									app_state.hash_list.load().as_ref().map(|x| x.version).unwrap_or(0);

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
														serde_smile::to_vec(&hash_list).unwrap()
													)
													.unwrap();

													app_state.hash_list.store(Some(hash_list.into()));
												}
											}
										}
									}
								}

								if let Some(install) = app_settings.load().game_install.as_ref() {
									if let Some(hash_list) = app_state.hash_list.load().as_ref() {
										let game_version = app_state
											.game_installs
											.iter()
											.try_find(|x| anyhow::Ok(x.path == *install))?
											.context("No such game install")?
											.version;

										app_state.intellisense.store(Some(
											Intellisense {
												cppt_properties: parking_lot::RwLock::new(HashMap::new()).into(),
												cppt_pins: from_slice(include_bytes!("../assets/pins.json")).unwrap(),
												uicb_prop_types: from_slice(include_bytes!(
													"../assets/uicbPropTypes.json"
												))
												.unwrap(),
												matt_properties: parking_lot::RwLock::new(HashMap::new()).into(),
												all_cppts: hash_list
													.entries
													.iter()
													.filter(|(_, entry)| entry.games.contains(game_version))
													.filter(|(_, entry)| entry.resource_type == "CPPT")
													.map(|(hash, _)| hash.to_owned())
													.collect(),
												all_asets: hash_list
													.entries
													.iter()
													.filter(|(_, entry)| entry.games.contains(game_version))
													.filter(|(_, entry)| entry.resource_type == "ASET")
													.map(|(hash, _)| hash.to_owned())
													.collect(),
												all_uicts: hash_list
													.entries
													.iter()
													.filter(|(_, entry)| entry.games.contains(game_version))
													.filter(|(_, entry)| entry.resource_type == "UICT")
													.map(|(hash, _)| hash.to_owned())
													.collect(),
												all_matts: hash_list
													.entries
													.iter()
													.filter(|(_, entry)| entry.games.contains(game_version))
													.filter(|(_, entry)| entry.resource_type == "MATT")
													.map(|(hash, _)| hash.to_owned())
													.collect(),
												all_wswts: hash_list
													.entries
													.iter()
													.filter(|(_, entry)| entry.games.contains(game_version))
													.filter(|(_, entry)| entry.resource_type == "WSWT")
													.map(|(hash, _)| hash.to_owned())
													.collect(),
												all_ecpts: hash_list
													.entries
													.iter()
													.filter(|(_, entry)| entry.games.contains(game_version))
													.filter(|(_, entry)| entry.resource_type == "ECPT")
													.map(|(hash, _)| hash.to_owned())
													.collect(),
												all_aibxs: hash_list
													.entries
													.iter()
													.filter(|(_, entry)| entry.games.contains(game_version))
													.filter(|(_, entry)| entry.resource_type == "AIBX")
													.map(|(hash, _)| hash.to_owned())
													.collect(),
												all_wsgts: hash_list
													.entries
													.iter()
													.filter(|(_, entry)| entry.games.contains(game_version))
													.filter(|(_, entry)| entry.resource_type == "WSGT")
													.map(|(hash, _)| hash.to_owned())
													.collect()
											}
											.into()
										));
									}
								}

								send_request(
									&app,
									Request::Tool(ToolRequest::GameBrowser(GameBrowserRequest::SetEnabled(
										app_settings.load().game_install.is_some()
											&& app_state.hash_list.load().is_some()
									)))
								)?;

								finish_task(&app, task)?;
							}

							SettingsEvent::ChangeGameInstall(path) => {
								let task = start_task(&app, "Loading game files")?;

								if let Some(path) = path.as_ref()
									&& app_settings.load().game_install.as_ref() != Some(path)
								{
									let task = start_task(&app, "Loading game files")?;

									let game_version = app_state
										.game_installs
										.iter()
										.try_find(|x| anyhow::Ok(x.path == *path))?
										.context("No such game install")?
										.version;

									let mut thumbs = IniFileSystem::new();
									thumbs.load(&path.join("thumbs.dat"))?;

									let mut resource_packages: IndexMap<PathBuf, ResourcePackage> = IndexMap::new();

									if let (Ok(proj_path), Ok(relative_runtime_path)) = (
										thumbs.get_value("application", "PROJECT_PATH"),
										thumbs.get_value("application", "RUNTIME_PATH")
									) {
										let mut package_manager =
											PackageManager::new(&path.join(proj_path).join(relative_runtime_path));

										let mut resource_container = ResourceContainer::default();

										package_manager.initialize(&mut resource_container).map_err(|x| {
											anyhow!("RPKG error in initialising resource container: {:?}", x)
										})?;

										for partition in package_manager.partition_infos {
											resource_packages.extend(
												vec![
													0,
													..ResourceContainer::get_patch_indices(
														&package_manager.runtime_dir,
														partition.index
													)
													.map_err(|x| {
														anyhow!("RPKG error in getting patch indices: {:?}", x)
													})?
													.iter()
													.filter(|&&x| x <= 9 || app_settings.load().extract_modded_files),
												]
												.into_par_iter()
												.rev()
												.map(|patch| {
													anyhow::Ok({
														let rpkg_path =
															Path::new(&package_manager.runtime_dir).join(format!(
																"{}{}{}.rpkg",
																match game_version {
																	GameVersion::H1 | GameVersion::H2 =>
																		if partition.index > 0 {
																			"dlc"
																		} else {
																			"chunk"
																		},

																	GameVersion::H3 => "chunk"
																},
																match game_version {
																	GameVersion::H1 | GameVersion::H2 =>
																		if partition.index > 0 {
																			partition.index - 1 // H1/H2 go chunk0, dlc0, dlc1
																		} else {
																			partition.index
																		},
																	GameVersion::H3 => partition.index
																},
																if patch > 0 {
																	format!("patch{}", patch)
																} else {
																	"".into()
																}
															));

														(rpkg_path.to_owned(), {
															let package_file = File::open(&rpkg_path)?;

															let mmap = unsafe { Mmap::map(&package_file)? };
															let mut reader = Cursor::new(&mmap[..]);

															reader
																.read_ne_args((patch > 0,))
																.context("Couldn't parse RPKG file")?
														})
													})
												})
												.collect::<Result<Vec<_>>>()?
											);
										}
									} else {
										Err(anyhow!("thumbs.dat was missing required properties"))?;
										panic!();
									}

									finish_task(&app, task)?;

									let task = start_task(&app, "Caching reverse references")?;

									let mut reverse_dependencies: HashMap<String, HashSet<String>> = HashMap::new();

									for rpkg in resource_packages.values() {
										for (resource_header, offset_info) in
											rpkg.resource_entries.iter().enumerate().map(|(index, entry)| {
												(rpkg.resource_metadata.get(index).unwrap(), entry)
											}) {
											for reference in resource_header
												.m_references
												.as_ref()
												.map(|x| &x.reference_hash)
												.unwrap_or(&Vec::new())
											{
												reverse_dependencies
													.entry(reference.to_hex_string())
													.or_default()
													.insert(offset_info.runtime_resource_id.to_hex_string());
											}
										}
									}

									app_state.resource_packages.store(Some(resource_packages.into()));

									app_state.resource_reverse_dependencies.store(Some(
										reverse_dependencies
											.into_iter()
											.map(|(x, y)| (x, y.into_iter().collect()))
											.collect::<HashMap<_, _>>()
											.into()
									));

									finish_task(&app, task)?;

									if let Some(hash_list) = app_state.hash_list.load().as_ref() {
										app_state.intellisense.store(Some(
											Intellisense {
												cppt_properties: parking_lot::RwLock::new(HashMap::new()).into(),
												cppt_pins: from_slice(include_bytes!("../assets/pins.json")).unwrap(),
												uicb_prop_types: from_slice(include_bytes!(
													"../assets/uicbPropTypes.json"
												))
												.unwrap(),
												matt_properties: parking_lot::RwLock::new(HashMap::new()).into(),
												all_cppts: hash_list
													.entries
													.iter()
													.filter(|(_, entry)| entry.games.contains(game_version))
													.filter(|(_, entry)| entry.resource_type == "CPPT")
													.map(|(hash, _)| hash.to_owned())
													.collect(),
												all_asets: hash_list
													.entries
													.iter()
													.filter(|(_, entry)| entry.games.contains(game_version))
													.filter(|(_, entry)| entry.resource_type == "ASET")
													.map(|(hash, _)| hash.to_owned())
													.collect(),
												all_uicts: hash_list
													.entries
													.iter()
													.filter(|(_, entry)| entry.games.contains(game_version))
													.filter(|(_, entry)| entry.resource_type == "UICT")
													.map(|(hash, _)| hash.to_owned())
													.collect(),
												all_matts: hash_list
													.entries
													.iter()
													.filter(|(_, entry)| entry.games.contains(game_version))
													.filter(|(_, entry)| entry.resource_type == "MATT")
													.map(|(hash, _)| hash.to_owned())
													.collect(),
												all_wswts: hash_list
													.entries
													.iter()
													.filter(|(_, entry)| entry.games.contains(game_version))
													.filter(|(_, entry)| entry.resource_type == "WSWT")
													.map(|(hash, _)| hash.to_owned())
													.collect(),
												all_ecpts: hash_list
													.entries
													.iter()
													.filter(|(_, entry)| entry.games.contains(game_version))
													.filter(|(_, entry)| entry.resource_type == "ECPT")
													.map(|(hash, _)| hash.to_owned())
													.collect(),
												all_aibxs: hash_list
													.entries
													.iter()
													.filter(|(_, entry)| entry.games.contains(game_version))
													.filter(|(_, entry)| entry.resource_type == "AIBX")
													.map(|(hash, _)| hash.to_owned())
													.collect(),
												all_wsgts: hash_list
													.entries
													.iter()
													.filter(|(_, entry)| entry.games.contains(game_version))
													.filter(|(_, entry)| entry.resource_type == "WSGT")
													.map(|(hash, _)| hash.to_owned())
													.collect()
											}
											.into()
										));
									}
								} else {
									app_state.resource_packages.store(None);
									app_state.intellisense.store(None);
								}

								let mut settings = (*app_settings.load_full()).to_owned();
								settings.game_install = path;
								fs::write(
									app.path_resolver()
										.app_data_dir()
										.context("Couldn't get app data dir")?
										.join("settings.json"),
									to_vec(&settings).unwrap()
								)
								.unwrap();
								app_settings.store(settings.into());

								send_request(
									&app,
									Request::Tool(ToolRequest::GameBrowser(GameBrowserRequest::SetEnabled(
										app_settings.load().game_install.is_some()
											&& app_state.hash_list.load().is_some()
									)))
								)?;

								finish_task(&app, task)?;
							}

							SettingsEvent::ChangeExtractModdedFiles(value) => {
								let mut settings = (*app_settings.load_full()).to_owned();
								settings.extract_modded_files = value;
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
						}
					},

					Event::Editor(event) => match event {
						EditorEvent::Text(event) => match event {
							TextEditorEvent::Initialise { id } => {
								let editor_state = app_state.editor_states.read().await;
								let editor_state = editor_state.get(&id).context("No such editor")?;

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
								let mut editor_state = app_state.editor_states.write().await;
								let editor_state = editor_state.get_mut(&id).context("No such editor")?;

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
									let mut editor_state = app_state.editor_states.write().await;
									let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

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
							},

							EntityEditorEvent::Tree(event) => match event {
								EntityTreeEvent::Initialise { editor_id } => {
									let editor_state = app_state.editor_states.read().await;
									let editor_state = editor_state.get(&editor_id).context("No such editor")?;

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
												editor_id,
												templates: from_slice(include_bytes!("../assets/templates.json"))
													.unwrap()
											}
										)))
									)?;
								}

								EntityTreeEvent::Select { editor_id, id } => {
									let editor_state = app_state.editor_states.read().await;
									let editor_state = editor_state.get(&editor_id).context("No such editor")?;

									let entity = match editor_state.data {
										EditorData::QNEntity { ref entity, .. } => entity,
										EditorData::QNPatch { ref current, .. } => current,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									let task = start_task(&app, format!("Selecting {}", id))?;

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
											EntityMonacoRequest::ReplaceContent {
												editor_id: editor_id.to_owned(),
												entity_id: id.to_owned(),
												content: String::from_utf8(buf)?
											}
										)))
									)?;

									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
											EntityMonacoRequest::UpdateValidity {
												editor_id,
												validity: EditorValidity::Valid
											}
										)))
									)?;

									let reverse_refs = calculate_reverse_references(entity)?
										.remove(&id)
										.context("No such entity")?;

									let settings = match editor_state.data {
										EditorData::QNEntity { ref settings, .. } => settings,
										EditorData::QNPatch { ref settings, .. } => settings,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::MetaPane(
											EntityMetaPaneRequest::SetReverseRefs {
												editor_id: editor_id.to_owned(),
												entity_names: reverse_refs
													.iter()
													.filter(|x| {
														settings.show_reverse_parent_refs
															|| !matches!(x.data, ReverseReferenceData::Parent)
													})
													.map(|x| {
														(
															x.from.to_owned(),
															entity.entities.get(&x.from).unwrap().name.to_owned()
														)
													})
													.collect(),
												reverse_refs: reverse_refs
													.into_iter()
													.filter(|x| {
														settings.show_reverse_parent_refs
															|| !matches!(x.data, ReverseReferenceData::Parent)
													})
													.collect()
											}
										)))
									)?;

									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::MetaPane(
											EntityMetaPaneRequest::SetNotes {
												editor_id: editor_id.to_owned(),
												entity_id: id.to_owned(),
												notes: entity
													.comments
													.iter()
													.find(|x| matches!(x.parent, Ref::Short(Some(ref x)) if *x == id))
													.map(|x| x.text.deref())
													.unwrap_or("")
													.into()
											}
										)))
									)?;

									finish_task(&app, task)?;

									if let Some(intellisense) = app_state.intellisense.load().as_ref()
										&& let Some(resource_packages) = app_state.resource_packages.load().as_ref()
										&& let Some(hash_list) = app_state.hash_list.load().as_ref()
										&& let Some(install) = app_settings.load().game_install.as_ref()
									{
										let game_version = app_state
											.game_installs
											.iter()
											.try_find(|x| anyhow::Ok(x.path == *install))?
											.context("No such game install")?
											.version;

										let task = start_task(&app, format!("Gathering intellisense data for {}", id))?;

										send_request(
											&app,
											Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
												EntityMonacoRequest::UpdateIntellisense {
													editor_id: editor_id.to_owned(),
													entity_id: id.to_owned(),
													properties: intellisense.get_properties(
														resource_packages,
														&app_state.cached_entities,
														hash_list,
														game_version,
														entity,
														&id,
														true
													)?,
													pins: intellisense.get_pins(
														resource_packages,
														&app_state.cached_entities,
														hash_list,
														game_version,
														entity,
														&id,
														false
													)?
												}
											)))
										)?;

										finish_task(&app, task)?;

										let task = start_task(&app, format!("Computing decorations for {}", id))?;

										let decorations = get_decorations(
											resource_packages,
											&app_state.cached_entities,
											hash_list,
											game_version,
											entity.entities.get(&id).context("No such entity")?,
											entity
										)?;

										send_request(
											&app,
											Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
												EntityMonacoRequest::UpdateDecorationsAndMonacoInfo {
													editor_id: editor_id.to_owned(),
													entity_id: id.to_owned(),
													local_ref_entity_ids: decorations
														.iter()
														.filter(|(x, _)| entity.entities.contains_key(x))
														.map(|(x, _)| x.to_owned())
														.collect(),
													decorations
												}
											)))
										)?;

										finish_task(&app, task)?;
									}
								}

								EntityTreeEvent::Create { editor_id, id, content } => {
									let mut editor_state = app_state.editor_states.write().await;
									let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

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
											id: editor_id,
											unsaved: true
										})
									)?;
								}

								EntityTreeEvent::Delete { editor_id, id } => {
									handle_delete(&app, editor_id, id).await?;
								}

								EntityTreeEvent::Rename {
									editor_id,
									id,
									new_name
								} => {
									let mut editor_state = app_state.editor_states.write().await;
									let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

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
								}

								EntityTreeEvent::Reparent {
									editor_id,
									id,
									new_parent
								} => {
									let mut editor_state = app_state.editor_states.write().await;
									let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

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
								}

								EntityTreeEvent::Copy { editor_id, id } => {
									let task = start_task(&app, format!("Copying entity {} and its children", id))?;

									let editor_state = app_state.editor_states.read().await;
									let editor_state = editor_state.get(&editor_id).context("No such editor")?;

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

									let editor_state = app_state.editor_states.read().await;
									let editor_state = editor_state.get(&editor_id).context("No such editor")?;

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
													.iter()
													.filter(|(id, ent)| {
														format!("{}{}", id, to_string(ent).unwrap().to_lowercase())
															.contains(&query)
													})
													.map(|(id, _)| id.to_owned())
													.collect()
											}
										)))
									)?;

									finish_task(&app, task)?;
								}

								EntityTreeEvent::ShowHelpMenu { editor_id, entity_id } => {
									let task = start_task(&app, format!("Showing help menu for {}", entity_id))?;

									let editor_state = app_state.editor_states.read().await;
									let editor_state = editor_state.get(&editor_id).context("No such editor")?;

									let entity = match editor_state.data {
										EditorData::QNEntity { ref entity, .. } => entity,
										EditorData::QNPatch { ref current, .. } => current,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									let sub_entity = entity.entities.get(&entity_id).context("No such entity")?;

									if let Some(intellisense) = app_state.intellisense.load().as_ref()
										&& let Some(resource_packages) = app_state.resource_packages.load().as_ref()
										&& let Some(hash_list) = app_state.hash_list.load().as_ref()
										&& let Some(install) = app_settings.load().game_install.as_ref()
									{
										let game_version = app_state
											.game_installs
											.iter()
											.try_find(|x| anyhow::Ok(x.path == *install))?
											.context("No such game install")?
											.version;

										let (properties, pins) = if hash_list
											.entries
											.get(&sub_entity.factory)
											.map(|entry| entry.resource_type == "TEMP")
											.unwrap_or(false)
										{
											ensure_entity_in_cache(
												resource_packages,
												&app_state.cached_entities,
												game_version,
												hash_list,
												&normalise_to_hash(sub_entity.factory.to_owned())
											)?;

											let underlying_entity = app_state.cached_entities.read();
											let underlying_entity = underlying_entity
												.get(&normalise_to_hash(sub_entity.factory.to_owned()))
												.unwrap();

											(
												intellisense.get_properties(
													resource_packages,
													&app_state.cached_entities,
													hash_list,
													game_version,
													underlying_entity,
													&underlying_entity.root_entity,
													false
												)?,
												intellisense.get_pins(
													resource_packages,
													&app_state.cached_entities,
													hash_list,
													game_version,
													underlying_entity,
													&underlying_entity.root_entity,
													false
												)?
											)
										} else {
											(
												intellisense.get_properties(
													resource_packages,
													&app_state.cached_entities,
													hash_list,
													game_version,
													entity,
													&entity_id,
													true
												)?,
												intellisense.get_pins(
													resource_packages,
													&app_state.cached_entities,
													hash_list,
													game_version,
													entity,
													&entity_id,
													true
												)?
											)
										};

										let properties_data_str = {
											let mut buf = Vec::new();
											let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
											let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

											properties
												.into_iter()
												.map(|(name, ty, default_val, post_init)| {
													(
														name,
														if post_init {
															json!({
																"type": ty,
																"value": default_val,
																"postInit": true
															})
														} else {
															json!({
																"type": ty,
																"value": default_val
															})
														}
													)
												})
												.collect::<HashMap<_, _>>()
												.serialize(&mut ser)?;

											String::from_utf8(buf)?
										};

										let ss = SyntaxSet::load_defaults_newlines();

										send_request(
											&app,
											Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
												EntityTreeRequest::ShowHelpMenu {
													editor_id,
													factory: sub_entity.factory.to_owned(),
													input_pins: pins.0,
													output_pins: pins.1,
													default_properties_html: highlighted_html_for_string(
														&properties_data_str,
														&ss,
														ss.find_syntax_by_extension("json").unwrap(),
														&ThemeSet::load_from_reader(&mut BufReader::new(Cursor::new(
															include_bytes!("../assets/vs-dark.tmTheme")
														)))?
													)?
												}
											)))
										)?;
									} else {
										send_notification(
											&app,
											Notification {
												kind: NotificationKind::Error,
												title: "Help menu unavailable".into(),
												subtitle: "A copy of the game hasn't been selected, or the hash list \
												           is unavailable."
													.into()
											}
										)?;
									}

									finish_task(&app, task)?;
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
									let task = start_task(&app, format!("Adding {}", file))?;

									let mut editor_state = app_state.editor_states.write().await;
									let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

									let entity = match editor_state.data {
										EditorData::QNEntity { ref mut entity, .. } => entity,
										EditorData::QNPatch { ref mut current, .. } => current,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
										&& let Some(hash_list) = app_state.hash_list.load().as_ref()
										&& let Some(install) = app_settings.load().game_install.as_ref()
									{
										let game_version = app_state
											.game_installs
											.iter()
											.try_find(|x| anyhow::Ok(x.path == *install))?
											.context("No such game install")?
											.version;

										if is_valid_entity_factory(
											&hash_list
												.entries
												.get(&file)
												.context("File not in hash list")?
												.resource_type
										) {
											let (temp_meta, temp_data) =
												extract_latest_resource(resource_packages, hash_list, &file)?;

											let factory = match game_version {
												GameVersion::H1 => convert_2016_factory_to_modern(
													&h2016_convert_binary_to_factory(&temp_data).context(
														"Couldn't convert binary data to ResourceLib factory"
													)?
												),

												GameVersion::H2 => h2_convert_binary_to_factory(&temp_data)
													.context("Couldn't convert binary data to ResourceLib factory")?,

												GameVersion::H3 => h3_convert_binary_to_factory(&temp_data)
													.context("Couldn't convert binary data to ResourceLib factory")?
											};

											let blueprint_hash = &temp_meta
												.hash_reference_data
												.get(factory.blueprint_index_in_resource_header as usize)
												.context(
													"Blueprint referenced in factory does not exist in dependencies"
												)?
												.hash;

											let factory_path = hash_list
												.entries
												.get(&file)
												.and_then(|x| x.path.to_owned())
												.unwrap_or(file);

											let blueprint_path = hash_list
												.entries
												.get(blueprint_hash)
												.and_then(|x| x.path.to_owned())
												.unwrap_or(blueprint_hash.to_owned());

											let entity_id = random_entity_id();

											let sub_entity = SubEntity {
												parent: Ref::Short((parent_id != "#").then_some(parent_id)),
												name: factory_path
													.replace("].pc_entitytype", "")
													.replace("].pc_entitytemplate", "")
													.replace(".entitytemplate", "")
													.split('/')
													.last()
													.map(|x| x.to_owned())
													.unwrap_or(factory_path.to_owned()),
												factory: factory_path,
												factory_flag: None,
												blueprint: blueprint_path,
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
											};

											send_request(
												&app,
												Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
													EntityTreeRequest::NewItems {
														editor_id: editor_id.to_owned(),
														new_entities: vec![(
															entity_id.to_owned(),
															sub_entity.parent.to_owned(),
															sub_entity.name.to_owned(),
															sub_entity.factory.to_owned(),
															false
														)]
													}
												)))
											)?;

											entity.entities.insert(entity_id, sub_entity);

											send_request(
												&app,
												Request::Global(GlobalRequest::SetTabUnsaved {
													id: editor_id,
													unsaved: true
												})
											)?;
										} else {
											send_notification(
												&app,
												Notification {
													kind: NotificationKind::Error,
													title: "Not a valid template".into(),
													subtitle: "Only entity templates can be dragged into the entity \
													           tree."
														.into()
												}
											)?;
										}
									} else {
										send_notification(
											&app,
											Notification {
												kind: NotificationKind::Error,
												title: "Game data unavailable".into(),
												subtitle: "A copy of the game hasn't been selected, or the hash list \
												           is unavailable."
													.into()
											}
										)?;
									}

									finish_task(&app, task)?;
								}
							},

							EntityEditorEvent::Monaco(event) => match event {
								EntityMonacoEvent::UpdateContent {
									editor_id,
									entity_id,
									content
								} => {
									let mut editor_state = app_state.editor_states.write().await;
									let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

									let entity = match editor_state.data {
										EditorData::QNEntity { ref mut entity, .. } => entity,
										EditorData::QNPatch { ref mut current, .. } => current,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									match from_str(&content) {
										Ok(sub_entity) => match check_local_references_exist(&sub_entity, entity) {
											Ok(EditorValidity::Valid) => {
												if sub_entity
													!= *entity.entities.get(&entity_id).context("No such sub-entity")?
												{
													if let Some(hash_list) = app_state.hash_list.load().as_ref() {
														if let Some(entry) = hash_list
															.entries
															.get(&normalise_to_hash(sub_entity.factory.to_owned()))
														{
															if !is_valid_entity_factory(&entry.resource_type) {
																send_request(
																	&app,
																	Request::Editor(EditorRequest::Entity(
																		EntityEditorRequest::Monaco(
																			EntityMonacoRequest::UpdateValidity {
																				editor_id,
																				validity: EditorValidity::Invalid(
																					"Invalid factory; unsupported \
																					 resource type"
																						.into()
																				)
																			}
																		)
																	))
																)?;

																return;
															}
														}

														if let Some(entry) = hash_list
															.entries
															.get(&normalise_to_hash(sub_entity.blueprint.to_owned()))
														{
															if !is_valid_entity_blueprint(&entry.resource_type) {
																send_request(
																	&app,
																	Request::Editor(EditorRequest::Entity(
																		EntityEditorRequest::Monaco(
																			EntityMonacoRequest::UpdateValidity {
																				editor_id,
																				validity: EditorValidity::Invalid(
																					"Invalid blueprint; unsupported \
																					 resource type"
																						.into()
																				)
																			}
																		)
																	))
																)?;

																return;
															}
														}
													}
													let mut reverse_parent_refs: HashSet<String> = HashSet::new();

													for entity_data in entity.entities.values() {
														match entity_data.parent {
															Ref::Full(ref reference)
																if reference.external_scene.is_none() =>
															{
																reverse_parent_refs
																	.insert(reference.entity_ref.to_owned());
															}

															Ref::Short(Some(ref reference)) => {
																reverse_parent_refs.insert(reference.to_owned());
															}

															_ => {}
														}
													}

													send_request(
														&app,
														Request::Editor(EditorRequest::Entity(
															EntityEditorRequest::Tree(EntityTreeRequest::NewItems {
																editor_id,
																new_entities: vec![(
																	entity_id.to_owned(),
																	sub_entity.parent.to_owned(),
																	sub_entity.name.to_owned(),
																	sub_entity.factory.to_owned(),
																	reverse_parent_refs.contains(&entity_id)
																)]
															})
														))
													)?;

													entity.entities.insert(entity_id.to_owned(), sub_entity);

													send_request(
														&app,
														Request::Editor(EditorRequest::Entity(
															EntityEditorRequest::Monaco(
																EntityMonacoRequest::UpdateValidity {
																	editor_id,
																	validity: EditorValidity::Valid
																}
															)
														))
													)?;

													send_request(
														&app,
														Request::Global(GlobalRequest::SetTabUnsaved {
															id: editor_id,
															unsaved: true
														})
													)?;

													if let Some(resource_packages) =
														app_state.resource_packages.load().as_ref() && let Some(
														hash_list
													) =
														app_state.hash_list.load().as_ref() && let Some(install) =
														app_settings.load().game_install.as_ref()
													{
														let game_version = app_state
															.game_installs
															.iter()
															.try_find(|x| anyhow::Ok(x.path == *install))?
															.context("No such game install")?
															.version;

														let task = start_task(&app, "Updating decorations")?;

														let decorations = get_decorations(
															resource_packages,
															&app_state.cached_entities,
															hash_list,
															game_version,
															entity
																.entities
																.get(&entity_id)
																.context("No such entity")?,
															entity
														)?;

														send_request(
														&app,
														Request::Editor(EditorRequest::Entity(
															EntityEditorRequest::Monaco(
																EntityMonacoRequest::UpdateDecorationsAndMonacoInfo {
																	editor_id: editor_id.to_owned(),
																	entity_id: entity_id.to_owned(),
																	local_ref_entity_ids: decorations
																		.iter()
																		.filter(|(x, _)| {
																			entity.entities.contains_key(x)
																		})
																		.map(|(x, _)| x.to_owned())
																		.collect(),
																	decorations
																}
															)
														))
													)?;

														finish_task(&app, task)?;
													}
												} else {
													send_request(
														&app,
														Request::Editor(EditorRequest::Entity(
															EntityEditorRequest::Monaco(
																EntityMonacoRequest::UpdateValidity {
																	editor_id,
																	validity: EditorValidity::Valid
																}
															)
														))
													)?;
												}
											}

											Ok(EditorValidity::Invalid(reason)) => {
												send_request(
													&app,
													Request::Editor(EditorRequest::Entity(
														EntityEditorRequest::Monaco(
															EntityMonacoRequest::UpdateValidity {
																editor_id,
																validity: EditorValidity::Invalid(reason)
															}
														)
													))
												)?;
											}

											Err(err) => {
												send_request(
													&app,
													Request::Editor(EditorRequest::Entity(
														EntityEditorRequest::Monaco(
															EntityMonacoRequest::UpdateValidity {
																editor_id,
																validity: EditorValidity::Invalid(format!(
																	"Invalid entity: {}",
																	err
																))
															}
														)
													))
												)?;
											}
										},

										Err(err) => {
											send_request(
												&app,
												Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
													EntityMonacoRequest::UpdateValidity {
														editor_id,
														validity: EditorValidity::Invalid(format!(
															"Invalid entity: {}",
															err
														))
													}
												)))
											)?;
										}
									}
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
									if let Some(install) = app_settings.load().game_install.as_ref()
										&& let Some(hash_list) = app_state.hash_list.load().as_ref()
										&& let Some(resource_packages) = app_state.resource_packages.load().as_deref()
									{
										let factory = normalise_to_hash(factory);

										if let Ok((filetype, _, _)) =
											extract_latest_overview_info(resource_packages, &factory)
										{
											if filetype == "TEMP" {
												let task = start_task(&app, format!("Loading entity {}", factory))?;

												let game_install_data = app_state
													.game_installs
													.iter()
													.try_find(|x| anyhow::Ok(x.path == *install))?
													.context("No such game install")?;

												ensure_entity_in_cache(
													resource_packages,
													&app_state.cached_entities,
													game_install_data.version,
													hash_list,
													&factory
												)?;

												let entity =
													app_state.cached_entities.read().get(&factory).unwrap().to_owned();

												let default_tab_name = format!(
													"{} ({})",
													entity
														.entities
														.get(&entity.root_entity)
														.context("Root entity doesn't exist")?
														.name,
													factory
												);

												let tab_name = if let Some(entry) = hash_list.entries.get(&factory) {
													if let Some(path) = entry.path.as_ref() {
														path.replace("].pc_entitytype", "")
															.replace("].pc_entitytemplate", "")
															.split('/')
															.last()
															.map(|x| x.to_owned())
															.unwrap_or(default_tab_name)
													} else if let Some(hint) = entry.hint.as_ref() {
														format!("{} ({})", hint, factory)
													} else {
														default_tab_name
													}
												} else {
													default_tab_name
												};

												let id = Uuid::new_v4();

												app_state.editor_states.write().await.insert(
													id.to_owned(),
													EditorState {
														file: None,
														data: EditorData::QNPatch {
															base: Box::new(entity.to_owned()),
															current: Box::new(entity.to_owned()),
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
											} else {
												let id = Uuid::new_v4();

												app_state.editor_states.write().await.insert(
													id.to_owned(),
													EditorState {
														file: None,
														data: EditorData::ResourceOverview {
															hash: factory.to_owned()
														}
													}
												);

												send_request(
													&app,
													Request::Global(GlobalRequest::CreateTab {
														id,
														name: format!("Resource overview ({factory})"),
														editor_type: EditorType::ResourceOverview
													})
												)?;
											}
										} else {
											send_notification(
												&app,
												Notification {
													kind: NotificationKind::Error,
													title: "Not a vanilla resource".into(),
													subtitle: "This factory doesn't exist in the base game files."
														.into()
												}
											)?;
										}
									} else {
										send_notification(
											&app,
											Notification {
												kind: NotificationKind::Error,
												title: "No game selected".into(),
												subtitle: "You can't open game files without a copy of the game \
												           selected."
													.into()
											}
										)?;
									}
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
									let mut editor_state = app_state.editor_states.write().await;
									let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

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

							EntityEditorEvent::Metadata(event) => match event {
								EntityMetadataEvent::Initialise { editor_id } => {
									let editor_state = app_state.editor_states.read().await;
									let editor_state = editor_state.get(&editor_id).context("No such editor")?;

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
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Metadata(
											EntityMetadataRequest::Initialise {
												editor_id: editor_id.to_owned(),
												factory_hash: entity.factory_hash.to_owned(),
												blueprint_hash: entity.blueprint_hash.to_owned(),
												root_entity: entity.root_entity.to_owned(),
												sub_type: entity.sub_type.to_owned(),
												external_scenes: entity.external_scenes.to_owned()
											}
										)))
									)?;

									// allow user to modify hash if there is no defined file we're writing to; will automatically convert editor state into entity editor rather than patch editor
									// also allow user to modify hash if it's already an entity
									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Metadata(
											EntityMetadataRequest::SetHashModificationAllowed {
												editor_id,
												hash_modification_allowed: matches!(
													editor_state.data,
													EditorData::QNEntity { .. }
												) || editor_state.file.is_none()
											}
										)))
									)?;
								}

								EntityMetadataEvent::SetFactoryHash {
									editor_id,
									mut factory_hash
								} => {
									let mut editor_state = app_state.editor_states.write().await;

									let mut is_patch_editor = false;

									if factory_hash != normalise_to_hash(factory_hash.to_owned()) {
										factory_hash = normalise_to_hash(factory_hash);

										send_request(
											&app,
											Request::Editor(EditorRequest::Entity(EntityEditorRequest::Metadata(
												EntityMetadataRequest::SetFactoryHash {
													editor_id: editor_id.to_owned(),
													factory_hash: factory_hash.to_owned()
												}
											)))
										)?;
									}

									{
										let editor_state =
											editor_state.get_mut(&editor_id).context("No such editor")?;

										let entity = match editor_state.data {
											EditorData::QNEntity { ref mut entity, .. } => entity,

											EditorData::QNPatch { ref mut current, .. } => {
												is_patch_editor = true;
												current
											}

											_ => {
												Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
												panic!();
											}
										};

										entity.factory_hash = factory_hash;
									}

									// If it was a patch editor, we should convert it into an entity editor since now we're working on a new entity
									if is_patch_editor {
										let state = editor_state.remove(&editor_id).context("No such editor")?;

										let EditorState {
											data: EditorData::QNPatch { settings, current, .. },
											file: None
										} = state
										else {
											unreachable!();
										};

										editor_state.insert(
											editor_id.to_owned(),
											EditorState {
												data: EditorData::QNEntity {
													settings,
													entity: current
												},
												file: None
											}
										);
									}

									send_request(
										&app,
										Request::Global(GlobalRequest::SetTabUnsaved {
											id: editor_id,
											unsaved: true
										})
									)?;
								}

								EntityMetadataEvent::SetBlueprintHash {
									editor_id,
									mut blueprint_hash
								} => {
									let mut editor_state = app_state.editor_states.write().await;

									let mut is_patch_editor = false;

									if blueprint_hash != normalise_to_hash(blueprint_hash.to_owned()) {
										blueprint_hash = normalise_to_hash(blueprint_hash);

										send_request(
											&app,
											Request::Editor(EditorRequest::Entity(EntityEditorRequest::Metadata(
												EntityMetadataRequest::SetBlueprintHash {
													editor_id: editor_id.to_owned(),
													blueprint_hash: blueprint_hash.to_owned()
												}
											)))
										)?;
									}

									{
										let editor_state =
											editor_state.get_mut(&editor_id).context("No such editor")?;

										let entity = match editor_state.data {
											EditorData::QNEntity { ref mut entity, .. } => entity,

											EditorData::QNPatch { ref mut current, .. } => {
												is_patch_editor = true;
												current
											}

											_ => {
												Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
												panic!();
											}
										};

										entity.blueprint_hash = blueprint_hash;
									}

									// If it was a patch editor, we should convert it into an entity editor since now we're working on a new entity
									if is_patch_editor {
										let state = editor_state.remove(&editor_id).context("No such editor")?;

										let EditorState {
											data: EditorData::QNPatch { settings, current, .. },
											file: None
										} = state
										else {
											unreachable!();
										};

										editor_state.insert(
											editor_id.to_owned(),
											EditorState {
												data: EditorData::QNEntity {
													settings,
													entity: current
												},
												file: None
											}
										);
									}

									send_request(
										&app,
										Request::Global(GlobalRequest::SetTabUnsaved {
											id: editor_id,
											unsaved: true
										})
									)?;
								}

								EntityMetadataEvent::SetRootEntity { editor_id, root_entity } => {
									let mut editor_state = app_state.editor_states.write().await;
									let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

									let entity = match editor_state.data {
										EditorData::QNEntity { ref mut entity, .. } => entity,
										EditorData::QNPatch { ref mut current, .. } => current,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									entity.root_entity = root_entity;

									send_request(
										&app,
										Request::Global(GlobalRequest::SetTabUnsaved {
											id: editor_id,
											unsaved: true
										})
									)?;
								}

								EntityMetadataEvent::SetSubType { editor_id, sub_type } => {
									let mut editor_state = app_state.editor_states.write().await;
									let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

									let entity = match editor_state.data {
										EditorData::QNEntity { ref mut entity, .. } => entity,
										EditorData::QNPatch { ref mut current, .. } => current,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									entity.sub_type = sub_type;

									send_request(
										&app,
										Request::Global(GlobalRequest::SetTabUnsaved {
											id: editor_id,
											unsaved: true
										})
									)?;
								}

								EntityMetadataEvent::SetExternalScenes {
									editor_id,
									external_scenes
								} => {
									let mut editor_state = app_state.editor_states.write().await;
									let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

									let entity = match editor_state.data {
										EditorData::QNEntity { ref mut entity, .. } => entity,
										EditorData::QNPatch { ref mut current, .. } => current,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									entity.external_scenes = external_scenes;

									send_request(
										&app,
										Request::Global(GlobalRequest::SetTabUnsaved {
											id: editor_id,
											unsaved: true
										})
									)?;
								}
							},

							EntityEditorEvent::Overrides(event) => match event {
								EntityOverridesEvent::Initialise { editor_id } => {
									let editor_state = app_state.editor_states.read().await;
									let editor_state = editor_state.get(&editor_id).context("No such editor")?;

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
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Overrides(
											EntityOverridesRequest::Initialise {
												editor_id,
												property_overrides: {
													let mut buf = Vec::new();
													let formatter =
														serde_json::ser::PrettyFormatter::with_indent(b"\t");
													let mut ser =
														serde_json::Serializer::with_formatter(&mut buf, formatter);

													entity.property_overrides.serialize(&mut ser)?;

													String::from_utf8(buf)?
												},
												override_deletes: {
													let mut buf = Vec::new();
													let formatter =
														serde_json::ser::PrettyFormatter::with_indent(b"\t");
													let mut ser =
														serde_json::Serializer::with_formatter(&mut buf, formatter);

													entity.override_deletes.serialize(&mut ser)?;

													String::from_utf8(buf)?
												},
												pin_connection_overrides: {
													let mut buf = Vec::new();
													let formatter =
														serde_json::ser::PrettyFormatter::with_indent(b"\t");
													let mut ser =
														serde_json::Serializer::with_formatter(&mut buf, formatter);

													entity.pin_connection_overrides.serialize(&mut ser)?;

													String::from_utf8(buf)?
												},
												pin_connection_override_deletes: {
													let mut buf = Vec::new();
													let formatter =
														serde_json::ser::PrettyFormatter::with_indent(b"\t");
													let mut ser =
														serde_json::Serializer::with_formatter(&mut buf, formatter);

													entity.pin_connection_override_deletes.serialize(&mut ser)?;

													String::from_utf8(buf)?
												}
											}
										)))
									)?;

									send_overrides_decorations(&app, editor_id, entity)?;
								}

								EntityOverridesEvent::UpdatePropertyOverrides { editor_id, content } => {
									let mut editor_state = app_state.editor_states.write().await;
									let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

									let entity = match editor_state.data {
										EditorData::QNEntity { ref mut entity, .. } => entity,
										EditorData::QNPatch { ref mut current, .. } => current,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									if let Ok(deserialised) = from_str(&content) {
										if entity.property_overrides != deserialised {
											entity.property_overrides = deserialised;

											send_overrides_decorations(&app, editor_id.to_owned(), entity)?;

											send_request(
												&app,
												Request::Global(GlobalRequest::SetTabUnsaved {
													id: editor_id,
													unsaved: true
												})
											)?;
										}
									}
								}

								EntityOverridesEvent::UpdateOverrideDeletes { editor_id, content } => {
									let mut editor_state = app_state.editor_states.write().await;
									let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

									let entity = match editor_state.data {
										EditorData::QNEntity { ref mut entity, .. } => entity,
										EditorData::QNPatch { ref mut current, .. } => current,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									if let Ok(deserialised) = from_str(&content) {
										if entity.override_deletes != deserialised {
											entity.override_deletes = deserialised;

											send_overrides_decorations(&app, editor_id.to_owned(), entity)?;

											send_request(
												&app,
												Request::Global(GlobalRequest::SetTabUnsaved {
													id: editor_id,
													unsaved: true
												})
											)?;
										}
									}
								}

								EntityOverridesEvent::UpdatePinConnectionOverrides { editor_id, content } => {
									let mut editor_state = app_state.editor_states.write().await;
									let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

									let entity = match editor_state.data {
										EditorData::QNEntity { ref mut entity, .. } => entity,
										EditorData::QNPatch { ref mut current, .. } => current,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									if let Ok(deserialised) = from_str(&content) {
										if entity.pin_connection_overrides != deserialised {
											entity.pin_connection_overrides = deserialised;

											send_overrides_decorations(&app, editor_id.to_owned(), entity)?;

											send_request(
												&app,
												Request::Global(GlobalRequest::SetTabUnsaved {
													id: editor_id,
													unsaved: true
												})
											)?;
										}
									}
								}

								EntityOverridesEvent::UpdatePinConnectionOverrideDeletes { editor_id, content } => {
									let mut editor_state = app_state.editor_states.write().await;
									let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

									let entity = match editor_state.data {
										EditorData::QNEntity { ref mut entity, .. } => entity,
										EditorData::QNPatch { ref mut current, .. } => current,

										_ => {
											Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
											panic!();
										}
									};

									if let Ok(deserialised) = from_str(&content) {
										if entity.pin_connection_override_deletes != deserialised {
											entity.pin_connection_override_deletes = deserialised;

											send_overrides_decorations(&app, editor_id.to_owned(), entity)?;

											send_request(
												&app,
												Request::Global(GlobalRequest::SetTabUnsaved {
													id: editor_id,
													unsaved: true
												})
											)?;
										}
									}
								}
							}
						},

						EditorEvent::ResourceOverview(event) => match event {
							ResourceOverviewEvent::Initialise { id } => {
								let editor_state = app_state.editor_states.read().await;
								let editor_state = editor_state.get(&id).context("No such editor")?;

								let hash = match editor_state.data {
									EditorData::ResourceOverview { ref hash, .. } => hash,

									_ => {
										Err(anyhow!("Editor {} is not a resource overview", id))?;
										panic!();
									}
								};

								let task = start_task(&app, format!("Loading resource overview for {}", hash))?;

								if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
									&& let Some(resource_reverse_dependencies) =
										app_state.resource_reverse_dependencies.load().as_ref()
									&& let Some(install) = app_settings.load().game_install.as_ref()
									&& let Some(hash_list) = app_state.hash_list.load().as_ref()
								{
									initialise_resource_overview(
										&app,
										&app_state,
										id,
										hash,
										resource_packages,
										resource_reverse_dependencies,
										install,
										hash_list
									)?;
								}

								finish_task(&app, task)?;
							}

							ResourceOverviewEvent::FollowDependency { id, new_hash } => {
								let mut editor_state = app_state.editor_states.write().await;
								let editor_state = editor_state.get_mut(&id).context("No such editor")?;

								let hash = match editor_state.data {
									EditorData::ResourceOverview { ref mut hash, .. } => hash,

									_ => {
										Err(anyhow!("Editor {} is not a resource overview", id))?;
										panic!();
									}
								};

								*hash = new_hash.to_owned();

								let task = start_task(&app, format!("Loading resource overview for {}", hash))?;

								if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
									&& let Some(resource_reverse_dependencies) =
										app_state.resource_reverse_dependencies.load().as_ref()
									&& let Some(install) = app_settings.load().game_install.as_ref()
									&& let Some(hash_list) = app_state.hash_list.load().as_ref()
								{
									initialise_resource_overview(
										&app,
										&app_state,
										id,
										hash,
										resource_packages,
										resource_reverse_dependencies,
										install,
										hash_list
									)?;

									send_request(
										&app,
										Request::Global(GlobalRequest::RenameTab {
											id,
											new_name: format!("Resource overview ({new_hash})")
										})
									)?;
								}

								finish_task(&app, task)?;
							}

							ResourceOverviewEvent::FollowDependencyInNewTab { hash, .. } => {
								let id = Uuid::new_v4();

								app_state.editor_states.write().await.insert(
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

							ResourceOverviewEvent::OpenInEditor { id } => {
								let hash = {
									let editor_state = app_state.editor_states.read().await;
									let editor_state = editor_state.get(&id).context("No such editor")?;
									match editor_state.data {
										EditorData::ResourceOverview { ref hash, .. } => hash,

										_ => {
											Err(anyhow!("Editor {} is not a resource overview", id))?;
											panic!();
										}
									}
									.to_owned()
								};

								// Only available for entities currently

								if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
									&& let Some(install) = app_settings.load().game_install.as_ref()
									&& let Some(hash_list) = app_state.hash_list.load().as_ref()
								{
									let task = start_task(&app, format!("Loading entity {}", hash))?;

									let game_install_data = app_state
										.game_installs
										.iter()
										.try_find(|x| anyhow::Ok(x.path == *install))?
										.context("No such game install")?;

									ensure_entity_in_cache(
										resource_packages,
										&app_state.cached_entities,
										game_install_data.version,
										hash_list,
										&hash
									)?;

									let entity = app_state.cached_entities.read().get(&hash).unwrap().to_owned();

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

									app_state.editor_states.write().await.insert(
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
							}

							ResourceOverviewEvent::ExtractAsFile { id } => {
								let editor_state = app_state.editor_states.read().await;
								let editor_state = editor_state.get(&id).context("No such editor")?;

								let hash = match editor_state.data {
									EditorData::ResourceOverview { ref hash, .. } => hash,

									_ => {
										Err(anyhow!("Editor {} is not a resource overview", id))?;
										panic!();
									}
								};

								if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
									&& let Some(hash_list) = app_state.hash_list.load().as_ref()
								{
									let (metadata, data) = extract_latest_resource(resource_packages, hash_list, hash)?;
									let metadata_file = generate_rpkg_meta(&metadata)?;

									let file_type = hash_list
										.entries
										.get(hash)
										.expect("Can only open files from the hash list")
										.resource_type
										.to_owned();

									let mut dialog = AsyncFileDialog::new().set_title("Extract file");

									if let Some(project) = app_state.project.load().as_ref() {
										dialog = dialog.set_directory(&project.path);
									}

									if let Some(save_handle) = dialog
										.add_filter(&format!("{} file", file_type), &[&file_type])
										.save_file()
										.await
									{
										fs::write(save_handle.path(), data)?;

										fs::write(
											save_handle.path().parent().unwrap().join(format!(
												"{}.meta",
												save_handle.path().file_name().unwrap().to_string_lossy()
											)),
											metadata_file
										)?;
									}
								}
							}

							ResourceOverviewEvent::ExtractAsQN { id } => {
								let editor_state = app_state.editor_states.read().await;
								let editor_state = editor_state.get(&id).context("No such editor")?;

								let hash = match editor_state.data {
									EditorData::ResourceOverview { ref hash, .. } => hash,

									_ => {
										Err(anyhow!("Editor {} is not a resource overview", id))?;
										panic!();
									}
								};

								if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
									&& let Some(install) = app_settings.load().game_install.as_ref()
									&& let Some(hash_list) = app_state.hash_list.load().as_ref()
								{
									ensure_entity_in_cache(
										resource_packages,
										&app_state.cached_entities,
										app_state
											.game_installs
											.iter()
											.try_find(|x| anyhow::Ok(x.path == *install))?
											.context("No such game install")?
											.version,
										hash_list,
										hash
									)?;

									let entity_json = {
										let entity = app_state.cached_entities.read();
										let entity = entity.get(hash).unwrap();
										to_vec(entity)?
									};

									let mut dialog = AsyncFileDialog::new().set_title("Extract entity");

									if let Some(project) = app_state.project.load().as_ref() {
										dialog = dialog.set_directory(&project.path);
									}

									if let Some(save_handle) = dialog
										.add_filter("QuickEntity entity", &["entity.json"])
										.save_file()
										.await
									{
										fs::write(save_handle.path(), entity_json)?;
									}
								}
							}

							ResourceOverviewEvent::ExtractTEMPAsRT { id } => {
								let editor_state = app_state.editor_states.read().await;
								let editor_state = editor_state.get(&id).context("No such editor")?;

								let hash = match editor_state.data {
									EditorData::ResourceOverview { ref hash, .. } => hash,

									_ => {
										Err(anyhow!("Editor {} is not a resource overview", id))?;
										panic!();
									}
								};

								if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
									&& let Some(install) = app_settings.load().game_install.as_ref()
									&& let Some(hash_list) = app_state.hash_list.load().as_ref()
								{
									let (metadata, data) = extract_latest_resource(resource_packages, hash_list, hash)?;
									let metadata_file = generate_rpkg_meta(&metadata)?;

									let data = match app_state
										.game_installs
										.iter()
										.try_find(|x| anyhow::Ok(x.path == *install))?
										.context("No such game install")?
										.version
									{
										GameVersion::H1 => to_vec(
											&h2016_convert_binary_to_factory(&data)
												.context("Couldn't convert binary data to ResourceLib factory")?
										)?,

										GameVersion::H2 => to_vec(
											&h2_convert_binary_to_factory(&data)
												.context("Couldn't convert binary data to ResourceLib factory")?
										)?,

										GameVersion::H3 => to_vec(
											&h3_convert_binary_to_factory(&data)
												.context("Couldn't convert binary data to ResourceLib factory")?
										)?
									};

									let mut dialog = AsyncFileDialog::new().set_title("Extract file");

									if let Some(project) = app_state.project.load().as_ref() {
										dialog = dialog.set_directory(&project.path);
									}

									if let Some(save_handle) =
										dialog.add_filter("TEMP.json file", &["TEMP.json"]).save_file().await
									{
										fs::write(save_handle.path(), data)?;

										fs::write(
											save_handle.path().parent().unwrap().join(format!(
												"{}.meta",
												save_handle.path().file_name().unwrap().to_string_lossy()
											)),
											metadata_file
										)?;
									}
								}
							}

							ResourceOverviewEvent::ExtractTBLUAsFile { id } => {
								let editor_state = app_state.editor_states.read().await;
								let editor_state = editor_state.get(&id).context("No such editor")?;

								let hash = match editor_state.data {
									EditorData::ResourceOverview { ref hash, .. } => hash,

									_ => {
										Err(anyhow!("Editor {} is not a resource overview", id))?;
										panic!();
									}
								};

								if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
									&& let Some(install) = app_settings.load().game_install.as_ref()
									&& let Some(hash_list) = app_state.hash_list.load().as_ref()
								{
									let game_version = app_state
										.game_installs
										.iter()
										.try_find(|x| anyhow::Ok(x.path == *install))?
										.context("No such game install")?
										.version;

									ensure_entity_in_cache(
										resource_packages,
										&app_state.cached_entities,
										game_version,
										hash_list,
										hash
									)?;

									let (metadata, data) = extract_latest_resource(resource_packages, hash_list, &{
										let entity = app_state.cached_entities.read();
										let entity = entity.get(hash).unwrap();
										entity.blueprint_hash.to_owned()
									})?;

									let metadata_file = generate_rpkg_meta(&metadata)?;

									let mut dialog = AsyncFileDialog::new().set_title("Extract file");

									if let Some(project) = app_state.project.load().as_ref() {
										dialog = dialog.set_directory(&project.path);
									}

									if let Some(save_handle) =
										dialog.add_filter("TBLU file", &["TBLU"]).save_file().await
									{
										fs::write(save_handle.path(), data)?;

										fs::write(
											save_handle.path().parent().unwrap().join(format!(
												"{}.meta",
												save_handle.path().file_name().unwrap().to_string_lossy()
											)),
											metadata_file
										)?;
									}
								}
							}

							ResourceOverviewEvent::ExtractTBLUAsRT { id } => {
								let editor_state = app_state.editor_states.read().await;
								let editor_state = editor_state.get(&id).context("No such editor")?;

								let hash = match editor_state.data {
									EditorData::ResourceOverview { ref hash, .. } => hash,

									_ => {
										Err(anyhow!("Editor {} is not a resource overview", id))?;
										panic!();
									}
								};

								if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
									&& let Some(install) = app_settings.load().game_install.as_ref()
									&& let Some(hash_list) = app_state.hash_list.load().as_ref()
								{
									let game_version = app_state
										.game_installs
										.iter()
										.try_find(|x| anyhow::Ok(x.path == *install))?
										.context("No such game install")?
										.version;

									ensure_entity_in_cache(
										resource_packages,
										&app_state.cached_entities,
										game_version,
										hash_list,
										hash
									)?;

									let (metadata, data) = extract_latest_resource(resource_packages, hash_list, &{
										let entity = app_state.cached_entities.read();
										let entity = entity.get(hash).unwrap();
										entity.blueprint_hash.to_owned()
									})?;

									let metadata_file = generate_rpkg_meta(&metadata)?;

									let data = match game_version {
										GameVersion::H1 => to_vec(
											&h2016_convert_binary_to_blueprint(&data)
												.context("Couldn't convert binary data to ResourceLib blueprint")?
										)?,

										GameVersion::H2 => to_vec(
											&h2_convert_binary_to_blueprint(&data)
												.context("Couldn't convert binary data to ResourceLib blueprint")?
										)?,

										GameVersion::H3 => to_vec(
											&h3_convert_binary_to_blueprint(&data)
												.context("Couldn't convert binary data to ResourceLib blueprint")?
										)?
									};

									let mut dialog = AsyncFileDialog::new().set_title("Extract file");

									if let Some(project) = app_state.project.load().as_ref() {
										dialog = dialog.set_directory(&project.path);
									}

									if let Some(save_handle) =
										dialog.add_filter("TBLU.json file", &["TBLU.json"]).save_file().await
									{
										fs::write(save_handle.path(), data)?;

										fs::write(
											save_handle.path().parent().unwrap().join(format!(
												"{}.meta",
												save_handle.path().file_name().unwrap().to_string_lossy()
											)),
											metadata_file
										)?;
									}
								}
							}
						}
					},

					Event::Global(event) => match event {
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
									let mut watcher = notify::recommended_watcher(
										move |evt: Result<notify::Event, notify::Error>| {
											if let Err::<_, Error>(e) = try {
												if let Ok(evt) = evt {
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
																				.context("Create event had no paths")?
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
																				.context("Create event had no path")?
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

														notify::EventKind::Modify(notify::event::ModifyKind::Name(
															notify::event::RenameMode::Both
														)) => {
															send_request(
																&notify_app,
																Request::Tool(ToolRequest::FileBrowser(
																	FileBrowserRequest::Rename {
																		old_path: evt
																			.paths
																			.first()
																			.context(
																				"Rename-both event had no first path"
																			)?
																			.to_owned(),
																		new_path: evt
																			.paths
																			.get(1)
																			.context(
																				"Rename-both event had no second path"
																			)?
																			.to_owned()
																	}
																))
															)?;
														}

														notify::EventKind::Modify(notify::event::ModifyKind::Name(
															notify::event::RenameMode::From
														)) => {
															send_request(
																&notify_app,
																Request::Tool(ToolRequest::FileBrowser(
																	FileBrowserRequest::BeginRename {
																		old_path: evt
																			.paths
																			.first()
																			.context("Rename-from event had no path")?
																			.to_owned()
																	}
																))
															)?;
														}

														notify::EventKind::Modify(notify::event::ModifyKind::Name(
															notify::event::RenameMode::To
														)) => {
															send_request(
																&notify_app,
																Request::Tool(ToolRequest::FileBrowser(
																	FileBrowserRequest::FinishRename {
																		new_path: evt
																			.paths
																			.first()
																			.context("Rename-to event had no paths")?
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

									watcher.watch(&path, notify::RecursiveMode::Recursive)?;

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
									.read()
									.await
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
							let old = app_state
								.editor_states
								.write()
								.await
								.remove(&tab)
								.context("No such editor")?;

							if old.file.is_some() {
								send_request(
									&app,
									Request::Tool(ToolRequest::FileBrowser(FileBrowserRequest::Select(None)))
								)?;
							}

							send_request(&app, Request::Global(GlobalRequest::RemoveTab(tab)))?;
						}

						GlobalEvent::SaveTab(tab) => {
							let mut guard = app_state.editor_states.write().await;
							let editor = guard.get_mut(&tab).context("No such editor")?;

							let data_to_save = match &editor.data {
								EditorData::Nil => {
									Err(anyhow!("Editor is a nil editor"))?;
									panic!();
								}

								EditorData::ResourceOverview { .. } => {
									Err(anyhow!("Editor is a resource overview"))?;
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

											EditorData::QNPatch { .. } => "QuickEntity patch"
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

											EditorData::QNPatch { .. } => "entity.patch.json"
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
					}
				}
			} {
				app.track_event("Error", Some(json!({"error":format!("{:?}", e)})));
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

			cloned_app.track_event("Error", Some(json!({"error":error.to_owned()})));
			send_request(&cloned_app, Request::Global(GlobalRequest::ErrorReport { error }))
				.expect("Couldn't send error report to frontend");
		}
	});
}

#[try_fn]
#[context("Couldn't initialise resource overview {id}")]
pub fn initialise_resource_overview(
	app: &AppHandle,
	app_state: &State<AppState>,
	id: Uuid,
	hash: &String,
	resource_packages: &IndexMap<PathBuf, ResourcePackage>,
	resource_reverse_dependencies: &Arc<HashMap<String, Vec<String>>>,
	install: &PathBuf,
	hash_list: &Arc<HashList>
) -> Result<()> {
	let (filetype, chunk_patch, deps) = extract_latest_overview_info(resource_packages, hash)?;

	send_request(
		app,
		Request::Editor(EditorRequest::ResourceOverview(ResourceOverviewRequest::Initialise {
			id,
			hash: hash.to_owned(),
			filetype,
			chunk_patch,
			path_or_hint: hash_list
				.entries
				.get(hash)
				.and_then(|x| x.path.as_ref().or(x.hint.as_ref()).cloned()),
			dependencies: deps
				.iter()
				.map(|(hash, flag)| {
					(
						hash.to_owned(),
						hash_list
							.entries
							.get(hash)
							.map(|x| x.resource_type.to_owned())
							.unwrap_or("".into()),
						hash_list
							.entries
							.get(hash)
							.and_then(|x| x.path.as_ref().or(x.hint.as_ref()).cloned()),
						flag.to_owned()
					)
				})
				.collect(),
			reverse_dependencies: resource_reverse_dependencies
				.get(hash)
				.map(|hashes| {
					hashes
						.iter()
						.map(|hash| {
							(
								hash.to_owned(),
								hash_list
									.entries
									.get(hash)
									.expect("No entry in hash list for resource")
									.resource_type
									.to_owned(),
								hash_list
									.entries
									.get(hash)
									.and_then(|x| x.path.as_ref().or(x.hint.as_ref()).cloned())
							)
						})
						.collect()
				})
				.unwrap_or_default(),
			data: match hash_list
				.entries
				.get(hash)
				.expect("No entry in hash list for resource")
				.resource_type
				.as_ref()
			{
				"TEMP" => {
					ensure_entity_in_cache(
						resource_packages,
						&app_state.cached_entities,
						app_state
							.game_installs
							.iter()
							.try_find(|x| anyhow::Ok(x.path == *install))?
							.context("No such game install")?
							.version,
						hash_list,
						hash
					)?;

					let entity = app_state.cached_entities.read();
					let entity = entity.get(hash).unwrap();

					ResourceOverviewData::Entity {
						blueprint_hash: entity.blueprint_hash.to_owned(),
						blueprint_path_or_hint: hash_list
							.entries
							.get(&entity.blueprint_hash)
							.and_then(|x| x.path.as_ref().or(x.hint.as_ref()).cloned())
					}
				}

				"AIRG" | "TBLU" | "RTLV" | "ATMD" | "CPPT" | "VIDB" | "CBLU" | "CRMD" | "DSWB" | "GFXF" | "GIDX"
				| "WSGB" | "ECPB" | "UICB" | "ENUM" => ResourceOverviewData::GenericRL,

				"ORES" => ResourceOverviewData::Ores,

				_ => ResourceOverviewData::Generic
			}
		}))
	)?;
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
