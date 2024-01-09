// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Specta creates non snake case functions
#![allow(non_snake_case)]
#![feature(try_blocks)]

pub mod entity;
pub mod event_handling;
pub mod game_detection;
pub mod hash_list;
pub mod model;
pub mod resourcelib;
pub mod show_in_folder;

use std::{
	collections::{HashMap, HashSet},
	fs,
	ops::Deref,
	sync::Arc
};

use anyhow::{anyhow, Context, Error, Result};
use arboard::Clipboard;
use arc_swap::ArcSwap;
use entity::{
	calculate_reverse_references, check_local_references_exist, get_local_reference, get_recursive_children,
	CopiedEntityData
};
use event_handling::entity_tree::{handle_delete, handle_paste};
use fn_error_context::context;
use game_detection::{detect_installs, GameVersion};
use hash_list::HashList;
use itertools::Itertools;
use model::{
	AppSettings, AppState, EditorData, EditorEvent, EditorRequest, EditorState, EditorType, EditorValidity,
	EntityEditorEvent, EntityEditorRequest, EntityMetaPaneEvent, EntityMetaPaneRequest, EntityMonacoEvent,
	EntityMonacoRequest, EntityTreeEvent, EntityTreeRequest, Event, FileBrowserEvent, FileBrowserRequest,
	GameBrowserEntry, GameBrowserEvent, GameBrowserRequest, GlobalEvent, GlobalRequest, Project, ProjectSettings,
	Request, SettingsEvent, SettingsRequest, TextEditorEvent, TextEditorRequest, TextFileType, ToolEvent, ToolRequest
};
use notify::Watcher;
use quickentity_rs::{
	generate_patch,
	qn_structs::{CommentEntity, Entity, Ref, SubEntity, SubType}
};
use serde::{Deserialize, Serialize};
use serde_json::{from_slice, from_str, to_string, to_vec};
use show_in_folder::show_in_folder;
use tauri::{async_runtime, AppHandle, Manager};
use tokio::sync::RwLock;
use tryvial::try_fn;
use uuid::Uuid;
use walkdir::WalkDir;

const HASH_LIST_ENDPOINT: &str =
	"https://github.com/glacier-modding/Hitman-Hashes/releases/latest/download/entity_hash_list.sml";

fn main() {
	let specta = {
		let specta_builder =
			tauri_specta::ts::builder().commands(tauri_specta::collect_commands![event, show_in_folder]);

		#[cfg(debug_assertions)]
		let specta_builder = specta_builder.path("../src/lib/bindings.ts");

		#[cfg(debug_assertions)]
		specta::export::ts("../src/lib/bindings-types.ts").unwrap();

		specta_builder.into_plugin()
	};

	tauri::Builder::default()
		.plugin(specta)
		.setup(|app| {
			let app_data_path = app.path_resolver().app_data_dir().unwrap();

			let mut invalid = true;
			if let Ok(read) = fs::read(app_data_path.join("settings.json")) {
				if let Ok(settings) = from_slice::<AppSettings>(&read) {
					invalid = false;
					app.manage(ArcSwap::new(settings.into()));
				}
			}

			if invalid {
				let settings = AppSettings::default();
				fs::create_dir_all(&app_data_path).unwrap();
				fs::write(app_data_path.join("settings.json"), to_vec(&settings).unwrap()).unwrap();
				app.manage(ArcSwap::new(settings.into()));
			}

			app.manage(AppState {
				game_installs: detect_installs().unwrap(),
				project: None.into(),
				hash_list: fs::read(app_data_path.join("hash_list.sml"))
					.ok()
					.and_then(|x| serde_smile::from_slice(&x).ok())
					.into(),
				fs_watcher: None.into(),
				editor_states: RwLock::new(HashMap::new()).into()
			});

			Ok(())
		})
		.run(tauri::generate_context!())
		.expect("error while running tauri application");
}

#[tauri::command]
#[specta::specta]
fn event(app: AppHandle, event: Event) {
	async_runtime::spawn(async move {
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
													data: EditorData::QNEntity(Box::new(entity))
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
													editor_type: EditorType::QNEntity,
													file: Some(path)
												})
											)?;
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
													editor_type: EditorType::Text { file_type },
													file: Some(path)
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
													},
													file: Some(path)
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
													},
													file: Some(path)
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
													editor_type: EditorType::Nil,
													file: Some(path)
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
					},

					ToolEvent::GameBrowser(event) => match event {
						GameBrowserEvent::Select(path) => {
							// TODO
						}

						GameBrowserEvent::Search(query) => {
							let task = start_task(&app, format!("Searching game files for {}", query))?;

							if let Some(install) = app_state
								.project
								.load()
								.as_ref()
								.unwrap()
								.settings
								.load()
								.game_install
								.as_ref()
							{
								let install = app_state
									.game_installs
									.iter()
									.find(|x| x.path == *install)
									.context("No such game install as specified in project.json")?;

								let game_flag = match install.version {
									GameVersion::H1 => 0b000010,
									GameVersion::H2 => 0b000100,
									GameVersion::H3 => 0b001000
								};

								if let Some(x) = app_state.hash_list.load().deref() {
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
											entries: x
												.entries
												.iter()
												.filter(|x| x.game_flags & game_flag == game_flag)
												.filter(|x| query.split(' ').all(|y| x.path.contains(y)))
												.map(|x| GameBrowserEntry {
													hash: x.hash.to_owned(),
													path: x.path.to_owned(),
													hint: x.hint.to_owned()
												})
												.collect()
										}))
									)?;
								}
							}

							finish_task(&app, task)?;
						}
					},

					ToolEvent::Settings(event) => match event {
						SettingsEvent::Initialise => {
							send_request(
								&app,
								Request::Tool(ToolRequest::Settings(SettingsRequest::Initialise {
									game_installs: app_state.game_installs.to_owned(),
									settings: (*app_settings.inner().load_full()).to_owned()
								}))
							)?;
						}

						SettingsEvent::ChangeGameInstall(path) => {
							send_request(
								&app,
								Request::Tool(ToolRequest::GameBrowser(GameBrowserRequest::SetEnabled(path.is_some())))
							)?;

							if let Some(project) = app_state.project.load().deref() {
								let mut settings = (*project.settings.load_full()).to_owned();
								settings.game_install = path;
								fs::write(project.path.join("project.json"), to_vec(&settings).unwrap()).unwrap();
								project.settings.store(settings.into());
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
								to_vec(&settings).unwrap()
							)
							.unwrap();
							app_settings.store(settings.into());
						}

						SettingsEvent::ChangeGFEPath(value) => {
							let mut settings = (*app_settings.load_full()).to_owned();
							settings.game_file_extensions_path = value;
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
						EntityEditorEvent::Tree(event) => match event {
							EntityTreeEvent::Initialise { editor_id } => {
								let mut editor_state = app_state.editor_states.write().await;
								let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

								let entity = match editor_state.data {
									EditorData::QNEntity(ref ent) => ent,
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
										EntityTreeRequest::NewTree { editor_id, entities }
									)))
								)?;
							}

							EntityTreeEvent::Select { editor_id, id } => {
								let editor_state = app_state.editor_states.read().await;
								let editor_state = editor_state.get(&editor_id).context("No such editor")?;

								let entity = match editor_state.data {
									EditorData::QNEntity(ref ent) => ent,
									EditorData::QNPatch { ref current, .. } => current,

									_ => {
										Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
										panic!();
									}
								};

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

								let reverse_refs = calculate_reverse_references(entity)?
									.remove(&id)
									.context("No such entity")?;

								send_request(
									&app,
									Request::Editor(EditorRequest::Entity(EntityEditorRequest::MetaPane(
										EntityMetaPaneRequest::SetReverseRefs {
											editor_id: editor_id.to_owned(),
											entity_names: reverse_refs
												.iter()
												.map(|x| {
													(
														x.from.to_owned(),
														entity.entities.get(&x.from).unwrap().name.to_owned()
													)
												})
												.collect(),
											reverse_refs
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

								// TODO: intellisense
							}

							EntityTreeEvent::Create { editor_id, id, content } => {
								let mut editor_state = app_state.editor_states.write().await;
								let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

								let entity = match editor_state.data {
									EditorData::QNEntity(ref mut ent) => ent,
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
									EditorData::QNEntity(ref mut ent) => ent,
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
							}

							EntityTreeEvent::Reparent {
								editor_id,
								id,
								new_parent
							} => {
								let mut editor_state = app_state.editor_states.write().await;
								let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

								let entity = match editor_state.data {
									EditorData::QNEntity(ref mut ent) => ent,
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
							}

							EntityTreeEvent::Copy { editor_id, id } => {
								let task = start_task(&app, format!("Copying entity {} and its children", id))?;

								let editor_state = app_state.editor_states.read().await;
								let editor_state = editor_state.get(&editor_id).context("No such editor")?;

								let entity = match editor_state.data {
									EditorData::QNEntity(ref ent) => ent,
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
								handle_paste(&app, editor_id, parent_id).await?;
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
									EditorData::QNEntity(ref mut ent) => ent,
									EditorData::QNPatch { ref mut current, .. } => current,

									_ => {
										Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
										panic!();
									}
								};

								match from_str(&content) {
									Ok(sub_entity) => match check_local_references_exist(&sub_entity, entity) {
										Ok(EditorValidity::Valid) => {
											entity.entities.insert(entity_id, sub_entity);

											send_request(
												&app,
												Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
													EntityMonacoRequest::UpdateValidity {
														editor_id,
														validity: EditorValidity::Valid
													}
												)))
											)?;

											send_request(
												&app,
												Request::Global(GlobalRequest::SetTabUnsaved {
													id: editor_id,
													unsaved: true
												})
											)?;
										}

										Ok(EditorValidity::Invalid(reason)) => {
											send_request(
												&app,
												Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
													EntityMonacoRequest::UpdateValidity {
														editor_id,
														validity: EditorValidity::Invalid(reason)
													}
												)))
											)?;
										}

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
						},

						EntityEditorEvent::MetaPane(event) => match event {
							EntityMetaPaneEvent::JumpToReference { editor_id, reference } => todo!(),

							EntityMetaPaneEvent::SetNotes {
								editor_id,
								entity_id,
								notes
							} => {
								let mut editor_state = app_state.editor_states.write().await;
								let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

								let entity = match editor_state.data {
									EditorData::QNEntity(ref mut ent) => ent,
									EditorData::QNPatch { ref mut current, .. } => current,

									_ => {
										Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
										panic!();
									}
								};

								// Remove comment referring to given entity
								entity
									.comments
									.retain(|x| get_local_reference(&x.parent).map(|x| x != entity_id).unwrap_or(true));

								// Add new comment
								entity.comments.push(CommentEntity {
									parent: Ref::Short(Some(entity_id)),
									name: "Notes".into(),
									text: notes
								});
							}
						}
					}
				},

				Event::Global(event) => match event {
					GlobalEvent::LoadWorkspace(path) => {
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
								fs::create_dir_all(&path).unwrap();
								fs::write(path.join("project.json"), to_vec(&settings).unwrap()).unwrap();
							}
						} else {
							settings = ProjectSettings::default();
							fs::create_dir_all(&path).unwrap();
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
								let mut watcher =
									notify::recommended_watcher(move |evt: Result<notify::Event, notify::Error>| {
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
																				.context("Create event had no paths")?
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
																		.context("Rename-both event had no first path")?
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
																FileBrowserRequest::Delete(
																	evt.paths
																		.first()
																		.context("Rename-from event had no path")?
																		.to_owned()
																)
															))
														)?;
													}

													notify::EventKind::Modify(notify::event::ModifyKind::Name(
														notify::event::RenameMode::To
													)) => {
														if let Ok(metadata) = fs::metadata(
															evt.paths
																.first()
																.context("Rename-to event had no paths")?
														) {
															send_request(
																&notify_app,
																Request::Tool(ToolRequest::FileBrowser(
																	FileBrowserRequest::Create {
																		path: evt
																			.paths
																			.first()
																			.context("Rename-to event had no paths")?
																			.to_owned(),
																		is_folder: metadata.is_dir()
																	}
																))
															)?;
														}
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
									})?;

								watcher.watch(&path, notify::RecursiveMode::Recursive)?;

								watcher
							}
							.into()
						));

						finish_task(&app, task)?;

						let task = start_task(&app, "Acquiring latest hash list")?;

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

						finish_task(&app, task)?;

						send_request(
							&app,
							Request::Tool(ToolRequest::GameBrowser(GameBrowserRequest::SetEnabled(
								settings.game_install.is_some() && app_state.hash_list.load().is_some()
							)))
						)?;
					}

					GlobalEvent::SelectTab(tab) => {
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
					}

					GlobalEvent::SaveTab(tab) => {
						let guard = app_state.editor_states.read().await;
						let editor = guard.get(&tab).context("No such editor")?;

						fs::write(
							editor.file.as_ref().context("Tab has no intended file")?,
							match &editor.data {
								EditorData::Nil => {
									Err(anyhow!("Editor is a nil editor"))?;
									panic!();
								}

								EditorData::Text { content, .. } => content.as_bytes().to_owned(),

								EditorData::QNEntity(entity) => {
									serde_json::to_vec(&entity).context("Entity is invalid")?
								}

								EditorData::QNPatch { base, current } => serde_json::to_vec(
									&generate_patch(base, current)
										.map_err(|x| anyhow!(x))
										.context("Couldn't generate patch")?
								)
								.context("Entity is invalid")?
							}
						)
						.context("Couldn't write file")?;

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
		} {
			send_request(
				&app,
				Request::Global(GlobalRequest::ErrorReport {
					error: format!("{:?}", e)
				})
			)
			.expect("Couldn't send error report to frontend");
		}
	});
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
