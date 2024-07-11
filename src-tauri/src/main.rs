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

pub mod biome;
pub mod editor_connection;
pub mod entity;
pub mod event_handling;
pub mod game_detection;
pub mod general;
pub mod hash_list;
pub mod intellisense;
pub mod languages;
pub mod material;
pub mod model;
pub mod ores;
pub mod repository;
pub mod resourcelib;
pub mod rpkg;
pub mod rpkg_tool;
pub mod show_in_folder;
pub mod wwev;

use std::{fs, path::Path, sync::Arc, time::Duration};

use anyhow::{anyhow, bail, Context, Error, Result};
use arboard::Clipboard;
use arc_swap::ArcSwap;
use biome::format_json;
use dashmap::DashMap;
use editor_connection::EditorConnection;
use entity::{
	calculate_reverse_references, get_diff_info, get_local_reference, get_recursive_children, CopiedEntityData
};
use event_handling::{
	entity_metadata::handle_entity_metadata_event,
	entity_monaco::{handle_openfactory, handle_updatecontent},
	entity_overrides::handle_entity_overrides_event,
	entity_tree::{
		handle_delete, handle_gamebrowseradd, handle_helpmenu, handle_moveentitytocamera, handle_moveentitytoplayer,
		handle_paste, handle_restoretooriginal, handle_rotateentityascamera, handle_rotateentityasplayer,
		handle_select, handle_selectentityineditor
	},
	repository_patch::handle_repository_patch_event,
	resource_overview::handle_resource_overview_event,
	tools::handle_tool_event,
	unlockables_patch::handle_unlockables_patch_event
};
use fn_error_context::context;
use game_detection::detect_installs;
use general::open_file;
use hashbrown::{HashMap, HashSet};
use indexmap::IndexMap;
use log::{info, trace, LevelFilter};
use model::{
	AppSettings, AppState, ContentSearchResultsEvent, ContentSearchResultsRequest, EditorConnectionEvent, EditorData,
	EditorEvent, EditorRequest, EditorState, EditorType, EntityEditorEvent, EntityEditorRequest, EntityGeneralEvent,
	EntityGeneralRequest, EntityMetaPaneEvent, EntityMetadataRequest, EntityMonacoEvent, EntityMonacoRequest,
	EntityTreeEvent, EntityTreeRequest, Event, FileBrowserRequest, GlobalEvent, GlobalRequest, JsonPatchType, Project,
	ProjectSettings, Request, SettingsRequest, TextEditorEvent, TextEditorRequest, TextFileType, ToolRequest
};
use notify::Watcher;
use quickentity_rs::{
	generate_patch,
	qn_structs::{CommentEntity, Property, Ref}
};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use rfd::AsyncFileDialog;
use serde::{Deserialize, Serialize};
use serde_json::{from_slice, from_str, json, to_string, to_value, to_vec, Value};
use show_in_folder::show_in_folder;
use tauri::{api::process::Command, async_runtime, AppHandle, Manager};
use tauri_plugin_aptabase::{EventTracker, InitOptions};
use tauri_plugin_log::LogTarget;
use tryvial::try_fn;
use uuid::Uuid;
use velcro::vec;
use walkdir::WalkDir;

pub const HASH_LIST_VERSION_ENDPOINT: &str =
	"https://github.com/glacier-modding/Hitman-Hashes/releases/latest/download/version";

pub const HASH_LIST_ENDPOINT: &str =
	"https://github.com/glacier-modding/Hitman-Hashes/releases/latest/download/hash_list.sml";

pub const TONYTOOLS_HASH_LIST_VERSION_ENDPOINT: &str =
	"https://github.com/glacier-modding/Hitman-l10n-Hashes/releases/latest/download/version.json";

pub const TONYTOOLS_HASH_LIST_ENDPOINT: &str =
	"https://github.com/glacier-modding/Hitman-l10n-Hashes/releases/latest/download/hash_list.hmla";

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
		.plugin(
			tauri_plugin_log::Builder::default()
				.targets([LogTarget::LogDir, LogTarget::Stdout, LogTarget::Webview])
				.level_for("tauri_plugin_aptabase", LevelFilter::Off)
				.level_for("quickentity_rs", LevelFilter::Off)
				.build()
		)
		.plugin(specta)
		.setup(|app| {
			app.track_event("App started", None);

			info!("Starting app");

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

			info!("Loaded settings");

			if app_data_path.join("temp").exists() {
				fs::remove_dir_all(app_data_path.join("temp"))?;
			}

			info!("Removed temp folder");

			app.manage(AppState {
				game_installs: detect_installs().expect("Couldn't detect game installs"),
				project: None.into(),
				hash_list: fs::read(app_data_path.join("hash_list.sml"))
					.ok()
					.and_then(|x| serde_smile::from_slice(&x).ok())
					.into(),
				tonytools_hash_list: fs::read(app_data_path.join("tonytools_hash_list.hlma"))
					.ok()
					.and_then(|x| tonytools::hashlist::HashList::load(&x).ok().map(|x| x.into()))
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

			info!("Managed state");

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
		trace!("Handling event: {:?}", event);

		let cloned_app = app.clone();

		if let Err(e) = async_runtime::spawn(async move {
			let app_settings = app.state::<ArcSwap<AppSettings>>();
			let app_state = app.state::<AppState>();

			if let Err::<_, Error>(e) = try {
				match event {
					Event::Tool(event) => {
						handle_tool_event(&app, event).await?;
					}

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
														let mut s = format!("{}{}", id, to_string(ent).unwrap());
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

									send_request(
										&app,
										Request::Global(GlobalRequest::SetTabUnsaved {
											id: editor_id.to_owned(),
											unsaved: true
										})
									)?;
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

						GlobalEvent::SelectAndOpenFile => {
							let mut dialog = AsyncFileDialog::new().set_title("Open file");

							if let Some(project) = app_state.project.load().as_ref() {
								dialog = dialog.set_directory(&project.path);
							}

							if let Some(open_handle) = dialog.pick_file().await {
								open_file(&app, open_handle.path()).await?;
							}
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

							let task = start_task(
								&app,
								format!(
									"Saving {}",
									editor
										.file
										.as_ref()
										.and_then(|x| x.file_name())
										.map(|x| x.to_string_lossy().to_string())
										.unwrap_or("tab".into())
								)
							)?;

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

									let unformatted = serde_json::to_string(&entity).context("Entity is invalid")?;

									if unformatted.len() < 1024 * 1024 {
										format_json(&unformatted)?.into_bytes()
									} else {
										unformatted.into_bytes()
									}
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

									let unformatted = serde_json::to_string(
										&generate_patch(base, current)
											.map_err(|x| anyhow!(x))
											.context("Couldn't generate patch")?
									)
									.context("Entity is invalid")?;

									if unformatted.len() < 1024 * 1024 {
										format_json(&unformatted)?.into_bytes()
									} else {
										unformatted.into_bytes()
									}
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

											finish_task(&app, task)?;

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
																		.iter()
																		.filter(|(key, _)| *key != "Id")
																		.map(|(x, y)| (x.to_owned(), y.to_owned()))
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
																		.iter()
																		.filter(|(key, _)| *key != "Id")
																		.map(|(x, y)| (x.to_owned(), y.to_owned()))
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
																		.iter()
																		.filter(|(key, _)| *key != "Id")
																		.map(|(x, y)| (x.to_owned(), y.to_owned()))
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
																		.iter()
																		.filter(|(key, _)| *key != "Id")
																		.map(|(x, y)| (x.to_owned(), y.to_owned()))
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

											finish_task(&app, task)?;

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

							finish_task(&app, task)?;
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
												entity,
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
#[context("Couldn't send task start event for {:?} to frontend", name.as_ref())]
pub fn start_task(app: &AppHandle, name: impl AsRef<str>) -> Result<Uuid> {
	let task_id = Uuid::new_v4();
	trace!("Starting task {}: {}", task_id, name.as_ref());
	app.emit_all("start-task", (&task_id, name.as_ref()))?;
	task_id
}

#[try_fn]
#[context("Couldn't send task finish event for {:?} to frontend", task)]
pub fn finish_task(app: &AppHandle, task: Uuid) -> Result<()> {
	trace!("Ending task {}", task);
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
	trace!("Sending notification: {:?}", notification);
	app.emit_all("send-notification", (Uuid::new_v4(), &notification))?;
}

#[try_fn]
#[context("Couldn't send request {:?} to frontend", request)]
pub fn send_request(app: &AppHandle, request: Request) -> Result<()> {
	trace!("Sending request: {:?}", request);
	app.emit_all("request", &request)?;
}
