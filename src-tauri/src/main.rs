// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Specta creates non snake case functions
#![allow(non_snake_case)]
#![feature(try_blocks)]

pub mod entity;
pub mod game_detection;
pub mod hash_list;
pub mod model;
pub mod resourcelib;

use std::{
	collections::{HashMap, HashSet},
	fmt::Debug,
	fs,
	ops::Deref,
	path::Path,
	sync::Arc
};

use anyhow::{anyhow, Context, Error, Result};
use arboard::Clipboard;
use arc_swap::{access::Access, ArcSwap, Guard};
use entity::{
	alter_ref_according_to_changelist, calculate_reverse_references, change_reference_to_local, get_local_reference,
	get_recursive_children, random_entity_id, CopiedEntityData, ReverseReferenceData
};
use fn_error_context::context;
use game_detection::{detect_installs, GameVersion};
use hash_list::HashList;
use itertools::Itertools;
use model::{
	AppSettings, AppState, EditorData, EditorEvent, EditorRequest, EditorState, EditorType, EntityEditorEvent,
	EntityEditorRequest, EntityTreeEvent, EntityTreeRequest, Event, FileBrowserEvent, FileBrowserRequest,
	GameBrowserEntry, GameBrowserEvent, GameBrowserRequest, GlobalEvent, GlobalRequest, Project, ProjectSettings,
	Request, SettingsEvent, SettingsRequest, TextEditorEvent, TextEditorRequest, TextFileType, ToolEvent, ToolRequest
};
use notify::Watcher;
use quickentity_rs::{
	apply_patch, generate_patch,
	patch_structs::{Patch, PatchOperation, SubEntityOperation},
	qn_structs::{Entity, FullRef, Ref, RefMaybeConstantValue, RefWithConstantValue, SubEntity, SubType}
};
use serde::{Deserialize, Serialize};
use serde_json::{from_slice, from_str, from_value, to_string, to_value, to_vec, Value};
use tauri::{async_runtime, AppHandle, Manager};
use tokio::sync::RwLock;
use tryvial::try_fn;
use uuid::Uuid;
use walkdir::WalkDir;

const HASH_LIST_ENDPOINT: &str =
	"https://github.com/glacier-modding/Hitman-Hashes/releases/latest/download/entity_hash_list.sml";

fn main() {
	let specta = {
		let specta_builder = tauri_specta::ts::builder().commands(tauri_specta::collect_commands![event]);

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

											let entity: Entity =
												from_slice(&fs::read(&path).context("Couldn't read file")?)
													.context("Invalid entity")?;

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

							EntityTreeEvent::Select { editor_id, id } => todo!(),

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
							}

							EntityTreeEvent::Delete { editor_id, id } => {
								let task = start_task(&app, format!("Deleting entity {}", id))?;

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

								let reverse_refs = calculate_reverse_references(entity)?;

								let entities_to_delete = get_recursive_children(entity, &id, &reverse_refs)?
									.into_iter()
									.collect::<HashSet<_>>();

								let mut patch = Patch {
									factory_hash: String::new(),
									blueprint_hash: String::new(),
									patch: vec![],
									patch_version: 6
								};

								let mut refs_deleted = 0;

								for entity_to_delete in &entities_to_delete {
									for reverse_ref in reverse_refs.get(entity_to_delete).context("No such entity")? {
										match &reverse_ref.data {
											ReverseReferenceData::Parent => {
												// The entity itself will be deleted later
											}

											ReverseReferenceData::Property { property_name } => {
												let entity_props = entity
													.entities
													.get_mut(&reverse_ref.from)
													.unwrap()
													.properties
													.as_mut()
													.unwrap();

												if entity_props.get(property_name).unwrap().property_type
													== "SEntityTemplateReference"
												{
													entity_props.shift_remove(property_name).unwrap();
												} else {
													entity_props
														.get_mut(property_name)
														.unwrap()
														.value
														.as_array_mut()
														.unwrap()
														.retain(|item| {
															if let Some(local_ref) = get_local_reference(
																&from_value::<Ref>(item.to_owned()).expect(
																	"Already done in reverse refs so no error here"
																)
															) {
																local_ref != *entity_to_delete
															} else {
																true
															}
														});
												}
											}

											ReverseReferenceData::PlatformSpecificProperty {
												property_name,
												platform
											} => {
												let entity_props = entity
													.entities
													.get_mut(&reverse_ref.from)
													.unwrap()
													.platform_specific_properties
													.as_mut()
													.unwrap()
													.get_mut(platform)
													.unwrap();

												if entity_props.get(property_name).unwrap().property_type
													== "SEntityTemplateReference"
												{
													entity_props.shift_remove(property_name).unwrap();
												} else {
													entity_props
														.get_mut(property_name)
														.unwrap()
														.value
														.as_array_mut()
														.unwrap()
														.retain(|item| {
															if let Some(local_ref) = get_local_reference(
																&from_value::<Ref>(item.to_owned()).expect(
																	"Already done in reverse refs so no error here"
																)
															) {
																local_ref != *entity_to_delete
															} else {
																true
															}
														});
												}
											}

											ReverseReferenceData::Event { event, trigger } => {
												patch.patch.push(PatchOperation::SubEntityOperation(
													reverse_ref.from.to_owned(),
													SubEntityOperation::RemoveEventConnection(
														event.to_owned(),
														trigger.to_owned(),
														entity
															.entities
															.get(&reverse_ref.from)
															.unwrap()
															.events
															.as_ref()
															.unwrap()
															.get(event)
															.unwrap()
															.get(trigger)
															.unwrap()
															.iter()
															.find(|x| {
																get_local_reference(match x {
																	RefMaybeConstantValue::Ref(ref x) => x,
																	RefMaybeConstantValue::RefWithConstantValue(
																		RefWithConstantValue { ref entity_ref, .. }
																	) => entity_ref
																})
																.map(|x| x == *entity_to_delete)
																.unwrap_or(false)
															})
															.unwrap()
															.to_owned()
													)
												));
											}

											ReverseReferenceData::InputCopy { trigger, propagate } => {
												patch.patch.push(PatchOperation::SubEntityOperation(
													reverse_ref.from.to_owned(),
													SubEntityOperation::RemoveInputCopyConnection(
														trigger.to_owned(),
														propagate.to_owned(),
														entity
															.entities
															.get(&reverse_ref.from)
															.unwrap()
															.input_copying
															.as_ref()
															.unwrap()
															.get(trigger)
															.unwrap()
															.get(propagate)
															.unwrap()
															.iter()
															.find(|x| {
																get_local_reference(match x {
																	RefMaybeConstantValue::Ref(ref x) => x,
																	RefMaybeConstantValue::RefWithConstantValue(
																		RefWithConstantValue { ref entity_ref, .. }
																	) => entity_ref
																})
																.map(|x| x == *entity_to_delete)
																.unwrap_or(false)
															})
															.unwrap()
															.to_owned()
													)
												));
											}

											ReverseReferenceData::OutputCopy { event, propagate } => {
												patch.patch.push(PatchOperation::SubEntityOperation(
													reverse_ref.from.to_owned(),
													SubEntityOperation::RemoveOutputCopyConnection(
														event.to_owned(),
														propagate.to_owned(),
														entity
															.entities
															.get(&reverse_ref.from)
															.unwrap()
															.input_copying
															.as_ref()
															.unwrap()
															.get(event)
															.unwrap()
															.get(propagate)
															.unwrap()
															.iter()
															.find(|x| {
																get_local_reference(match x {
																	RefMaybeConstantValue::Ref(ref x) => x,
																	RefMaybeConstantValue::RefWithConstantValue(
																		RefWithConstantValue { ref entity_ref, .. }
																	) => entity_ref
																})
																.map(|x| x == *entity_to_delete)
																.unwrap_or(false)
															})
															.unwrap()
															.to_owned()
													)
												));
											}

											ReverseReferenceData::PropertyAlias { aliased_name, .. } => {
												entity
													.entities
													.get_mut(&reverse_ref.from)
													.unwrap()
													.property_aliases
													.as_mut()
													.unwrap()
													.get_mut(aliased_name)
													.unwrap()
													.retain(|x| {
														get_local_reference(&x.original_entity)
															.map(|x| x != *entity_to_delete)
															.unwrap_or(false)
													});
											}

											ReverseReferenceData::ExposedEntity { exposed_name } => {
												entity
													.entities
													.get_mut(&reverse_ref.from)
													.unwrap()
													.exposed_entities
													.as_mut()
													.unwrap()
													.get_mut(exposed_name)
													.unwrap()
													.refers_to
													.retain(|x| {
														get_local_reference(x)
															.map(|x| x != *entity_to_delete)
															.unwrap_or(false)
													});

												if entity
													.entities
													.get_mut(&reverse_ref.from)
													.unwrap()
													.exposed_entities
													.as_mut()
													.unwrap()
													.get_mut(exposed_name)
													.unwrap()
													.refers_to
													.is_empty()
												{
													entity
														.entities
														.get_mut(&reverse_ref.from)
														.unwrap()
														.exposed_entities
														.as_mut()
														.unwrap()
														.shift_remove(exposed_name)
														.unwrap();
												}
											}

											ReverseReferenceData::ExposedInterface { interface } => {
												entity
													.entities
													.get_mut(&reverse_ref.from)
													.unwrap()
													.exposed_interfaces
													.as_mut()
													.unwrap()
													.shift_remove(interface)
													.unwrap();
											}

											ReverseReferenceData::Subset { subset } => {
												entity
													.entities
													.get_mut(&reverse_ref.from)
													.unwrap()
													.subsets
													.as_mut()
													.unwrap()
													.get_mut(subset)
													.unwrap()
													.retain(|x| x != entity_to_delete);
											}
										}

										refs_deleted += 1;
									}
								}

								apply_patch(entity, patch, false).map_err(|x| anyhow!(x))?;

								entity.entities.retain(|x, _| !entities_to_delete.contains(x));

								finish_task(&app, task)?;

								send_notification(
									&app,
									Notification {
										kind: NotificationKind::Info,
										title: format!("Deleted {} entities", entities_to_delete.len()),
										subtitle: format!(
											"The entity, its children and {} reference{} have been deleted",
											refs_deleted,
											if refs_deleted == 1 { "" } else { "s" }
										)
									}
								)?;
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
								let mut paste_data = from_str::<CopiedEntityData>(&Clipboard::new()?.get_text()?)?;

								let task = start_task(
									&app,
									format!(
										"Pasting entity {}",
										paste_data
											.data
											.get(&paste_data.root_entity)
											.context("No such root entity")?
											.name
									)
								)?;

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

								let mut changed_entity_ids = HashMap::new();
								let mut added_external_scenes = 0;

								// Randomise new entity IDs for all subentities contained in the paste data
								for id in paste_data.data.keys() {
									changed_entity_ids.insert(id.to_owned(), random_entity_id());
								}

								// The IDs of all entities in the paste, in both changed and original forms.
								let all_paste_contents = paste_data
									.data
									.keys()
									.cloned()
									.chain(changed_entity_ids.values().cloned())
									.collect::<HashSet<_>>();

								// Change all internal references so they match with the new randomised entity IDs, and also remove any local references that don't exist in the entity we're pasting into
								for sub_entity in paste_data.data.values_mut() {
									// Parent refs are all internal to the paste since the paste is created based on parent hierarchy
									sub_entity.parent = change_reference_to_local(
										&sub_entity.parent,
										changed_entity_ids
											.get(&get_local_reference(&sub_entity.parent).unwrap())
											.unwrap()
											.to_owned()
									);

									for property_data in sub_entity
										.properties
										.as_mut()
										.unwrap_or(&mut Default::default())
										.values_mut()
									{
										if property_data.property_type == "SEntityTemplateReference" {
											let entity_ref = alter_ref_according_to_changelist(
												&from_value::<Ref>(property_data.value.to_owned())
													.context("Invalid reference")?,
												&changed_entity_ids
											);

											property_data.value = to_value(&entity_ref)?;

											// If the ref is external, add the external scene
											if let Ref::Full(FullRef {
												external_scene: Some(ref scene),
												..
											}) = entity_ref
											{
												entity.external_scenes.push(scene.to_owned());
												added_external_scenes += 1;
											}

											// If the ref is local but to a sub-entity that doesn't exist in the entity we're pasting into (and isn't an internal reference within the paste), set the property to null
											if get_local_reference(&entity_ref)
												.map(|x| {
													!entity.entities.contains_key(&x)
														&& !all_paste_contents.contains(&x)
												})
												.unwrap_or(false)
											{
												property_data.value = Value::Null;
											}
										} else if property_data.property_type == "TArray<SEntityTemplateReference>" {
											property_data.value = to_value(
												from_value::<Vec<Ref>>(property_data.value.to_owned())
													.context("Invalid reference array")?
													.into_iter()
													.map(|entity_ref| {
														if let Ref::Full(FullRef {
															external_scene: Some(ref scene),
															..
														}) = entity_ref
														{
															entity.external_scenes.push(scene.to_owned());
															added_external_scenes += 1;
														}

														alter_ref_according_to_changelist(
															&entity_ref,
															&changed_entity_ids
														)
													})
													.filter(|entity_ref| {
														!get_local_reference(entity_ref)
															.map(|x| {
																!entity.entities.contains_key(&x)
																	&& !all_paste_contents.contains(&x)
															})
															.unwrap_or(false)
													})
													.collect_vec()
											)?;
										}
									}

									for properties in sub_entity
										.platform_specific_properties
										.as_mut()
										.unwrap_or(&mut Default::default())
										.values_mut()
									{
										for property_data in properties.values_mut() {
											if property_data.property_type == "SEntityTemplateReference" {
												let entity_ref = alter_ref_according_to_changelist(
													&from_value::<Ref>(property_data.value.to_owned())
														.context("Invalid reference")?,
													&changed_entity_ids
												);

												property_data.value = to_value(&entity_ref)?;

												// If the ref is external, add the external scene
												if let Ref::Full(FullRef {
													external_scene: Some(ref scene),
													..
												}) = entity_ref
												{
													entity.external_scenes.push(scene.to_owned());
													added_external_scenes += 1;
												}

												// If the ref is local but to a sub-entity that doesn't exist in the entity we're pasting into (and isn't an internal reference within the paste), set the property to null
												if get_local_reference(&entity_ref)
													.map(|x| {
														!entity.entities.contains_key(&x)
															&& !all_paste_contents.contains(&x)
													})
													.unwrap_or(false)
												{
													property_data.value = Value::Null;
												}
											} else if property_data.property_type == "TArray<SEntityTemplateReference>"
											{
												property_data.value = to_value(
													from_value::<Vec<Ref>>(property_data.value.to_owned())
														.context("Invalid reference array")?
														.into_iter()
														.map(|entity_ref| {
															if let Ref::Full(FullRef {
																external_scene: Some(ref scene),
																..
															}) = entity_ref
															{
																entity.external_scenes.push(scene.to_owned());
																added_external_scenes += 1;
															}

															alter_ref_according_to_changelist(
																&entity_ref,
																&changed_entity_ids
															)
														})
														.filter(|entity_ref| {
															!get_local_reference(entity_ref)
																.map(|x| {
																	!entity.entities.contains_key(&x)
																		&& !all_paste_contents.contains(&x)
																})
																.unwrap_or(false)
														})
														.collect_vec()
												)?;
											}
										}
									}

									for values in sub_entity
										.events
										.as_mut()
										.unwrap_or(&mut Default::default())
										.values_mut()
									{
										for refs in values.values_mut() {
											for reference in refs.iter_mut() {
												let underlying_ref = match reference {
													RefMaybeConstantValue::Ref(x) => x,
													RefMaybeConstantValue::RefWithConstantValue(
														RefWithConstantValue { entity_ref, .. }
													) => entity_ref
												};

												if let Ref::Full(FullRef {
													external_scene: Some(ref scene),
													..
												}) = underlying_ref
												{
													entity.external_scenes.push(scene.to_owned());
													added_external_scenes += 1;
												}

												*reference = match reference {
													RefMaybeConstantValue::Ref(x) => RefMaybeConstantValue::Ref(
														alter_ref_according_to_changelist(x, &changed_entity_ids)
													),
													RefMaybeConstantValue::RefWithConstantValue(
														RefWithConstantValue { entity_ref, value }
													) => RefMaybeConstantValue::RefWithConstantValue(
														RefWithConstantValue {
															entity_ref: alter_ref_according_to_changelist(
																entity_ref,
																&changed_entity_ids
															),
															value: value.to_owned()
														}
													)
												};
											}

											refs.retain(|reference| {
												let underlying_ref = match reference {
													RefMaybeConstantValue::Ref(x) => x,
													RefMaybeConstantValue::RefWithConstantValue(
														RefWithConstantValue { entity_ref, .. }
													) => entity_ref
												};

												!get_local_reference(underlying_ref)
													.map(|x| {
														!entity.entities.contains_key(&x)
															&& !all_paste_contents.contains(&x)
													})
													.unwrap_or(false)
											});
										}
									}

									for values in sub_entity
										.input_copying
										.as_mut()
										.unwrap_or(&mut Default::default())
										.values_mut()
									{
										for refs in values.values_mut() {
											for reference in refs.iter_mut() {
												let underlying_ref = match reference {
													RefMaybeConstantValue::Ref(x) => x,
													RefMaybeConstantValue::RefWithConstantValue(
														RefWithConstantValue { entity_ref, .. }
													) => entity_ref
												};

												if let Ref::Full(FullRef {
													external_scene: Some(ref scene),
													..
												}) = underlying_ref
												{
													entity.external_scenes.push(scene.to_owned());
													added_external_scenes += 1;
												}

												*reference = match reference {
													RefMaybeConstantValue::Ref(x) => RefMaybeConstantValue::Ref(
														alter_ref_according_to_changelist(x, &changed_entity_ids)
													),
													RefMaybeConstantValue::RefWithConstantValue(
														RefWithConstantValue { entity_ref, value }
													) => RefMaybeConstantValue::RefWithConstantValue(
														RefWithConstantValue {
															entity_ref: alter_ref_according_to_changelist(
																entity_ref,
																&changed_entity_ids
															),
															value: value.to_owned()
														}
													)
												};
											}

											refs.retain(|reference| {
												let underlying_ref = match reference {
													RefMaybeConstantValue::Ref(x) => x,
													RefMaybeConstantValue::RefWithConstantValue(
														RefWithConstantValue { entity_ref, .. }
													) => entity_ref
												};

												!get_local_reference(underlying_ref)
													.map(|x| {
														!entity.entities.contains_key(&x)
															&& !all_paste_contents.contains(&x)
													})
													.unwrap_or(false)
											});
										}
									}

									for values in sub_entity
										.output_copying
										.as_mut()
										.unwrap_or(&mut Default::default())
										.values_mut()
									{
										for refs in values.values_mut() {
											for reference in refs.iter_mut() {
												let underlying_ref = match reference {
													RefMaybeConstantValue::Ref(x) => x,
													RefMaybeConstantValue::RefWithConstantValue(
														RefWithConstantValue { entity_ref, .. }
													) => entity_ref
												};

												if let Ref::Full(FullRef {
													external_scene: Some(ref scene),
													..
												}) = underlying_ref
												{
													entity.external_scenes.push(scene.to_owned());
													added_external_scenes += 1;
												}

												*reference = match reference {
													RefMaybeConstantValue::Ref(x) => RefMaybeConstantValue::Ref(
														alter_ref_according_to_changelist(x, &changed_entity_ids)
													),
													RefMaybeConstantValue::RefWithConstantValue(
														RefWithConstantValue { entity_ref, value }
													) => RefMaybeConstantValue::RefWithConstantValue(
														RefWithConstantValue {
															entity_ref: alter_ref_according_to_changelist(
																entity_ref,
																&changed_entity_ids
															),
															value: value.to_owned()
														}
													)
												};
											}

											refs.retain(|reference| {
												let underlying_ref = match reference {
													RefMaybeConstantValue::Ref(x) => x,
													RefMaybeConstantValue::RefWithConstantValue(
														RefWithConstantValue { entity_ref, .. }
													) => entity_ref
												};

												!get_local_reference(underlying_ref)
													.map(|x| {
														!entity.entities.contains_key(&x)
															&& !all_paste_contents.contains(&x)
													})
													.unwrap_or(false)
											});
										}
									}

									for aliases in sub_entity
										.property_aliases
										.as_mut()
										.unwrap_or(&mut Default::default())
										.values_mut()
									{
										for alias_data in aliases.iter_mut() {
											alias_data.original_entity = alter_ref_according_to_changelist(
												&alias_data.original_entity,
												&changed_entity_ids
											);

											if let Ref::Full(FullRef {
												external_scene: Some(ref scene),
												..
											}) = alias_data.original_entity
											{
												entity.external_scenes.push(scene.to_owned());
												added_external_scenes += 1;
											}
										}

										aliases.retain(|alias_data| {
											!get_local_reference(&alias_data.original_entity)
												.map(|x| {
													!entity.entities.contains_key(&x)
														&& !all_paste_contents.contains(&x)
												})
												.unwrap_or(false)
										});
									}

									for exposed_entity in sub_entity
										.exposed_entities
										.as_mut()
										.unwrap_or(&mut Default::default())
										.values_mut()
									{
										for reference in exposed_entity.refers_to.iter_mut() {
											*reference =
												alter_ref_according_to_changelist(reference, &changed_entity_ids);

											if let Ref::Full(FullRef {
												external_scene: Some(ref scene),
												..
											}) = reference
											{
												entity.external_scenes.push(scene.to_owned());
												added_external_scenes += 1;
											}
										}

										exposed_entity.refers_to.retain(|x| {
											// Only retain those not meeting the criteria for deletion (local ref, not in entity we're pasting into or the paste itself)
											!get_local_reference(x)
												.map(|x| {
													!entity.entities.contains_key(&x)
														&& !all_paste_contents.contains(&x)
												})
												.unwrap_or(false)
										});
									}

									for referenced_entity in sub_entity
										.exposed_interfaces
										.as_mut()
										.unwrap_or(&mut Default::default())
										.values_mut()
									{
										*referenced_entity = changed_entity_ids
											.get(referenced_entity)
											.unwrap_or(referenced_entity)
											.to_owned();
									}

									sub_entity
										.exposed_interfaces
										.as_mut()
										.unwrap_or(&mut Default::default())
										.retain(|_, x| {
											entity.entities.contains_key(x) || all_paste_contents.contains(x)
										});

									for member_of in sub_entity
										.subsets
										.as_mut()
										.unwrap_or(&mut Default::default())
										.values_mut()
									{
										for parental_entity in member_of.iter_mut() {
											*parental_entity = changed_entity_ids
												.get(parental_entity)
												.unwrap_or(parental_entity)
												.to_owned();
										}

										member_of.retain(|x| {
											entity.entities.contains_key(x) || all_paste_contents.contains(x)
										});
									}
								}

								// Change the actual entity IDs in the paste data
								paste_data.data = paste_data
									.data
									.into_iter()
									.map(|(x, y)| (changed_entity_ids.remove(&x).unwrap(), y))
									.collect();

								entity.entities.extend(paste_data.data);

								entity
									.entities
									.get_mut(changed_entity_ids.get(&paste_data.root_entity).unwrap())
									.unwrap()
									.parent = change_reference_to_local(
									&entity
										.entities
										.get_mut(changed_entity_ids.get(&paste_data.root_entity).unwrap())
										.unwrap()
										.parent,
									parent_id
								);

								finish_task(&app, task)?;
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
