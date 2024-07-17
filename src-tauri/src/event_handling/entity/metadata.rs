use std::fs;

use anyhow::{anyhow, Context, Result};
use fn_error_context::context;
use serde_json::to_vec;
use tauri::{AppHandle, Manager};
use tauri_plugin_aptabase::EventTracker;
use tryvial::try_fn;

use crate::{
	model::{
		AppState, EditorData, EditorRequest, EditorState, EntityEditorRequest, EntityMetadataEvent,
		EntityMetadataRequest, GlobalRequest, Request, SettingsRequest, ToolRequest
	},
	rpkg::normalise_to_hash,
	send_notification, send_request, Notification, NotificationKind
};

#[try_fn]
#[context("Couldn't handle entity metadata event")]
pub async fn handle(app: &AppHandle, event: EntityMetadataEvent) -> Result<()> {
	let app_state = app.state::<AppState>();

	match event {
		EntityMetadataEvent::Initialise { editor_id } => {
			let editor_state = app_state.editor_states.get(&editor_id).context("No such editor")?;

			let entity = match editor_state.data {
				EditorData::QNEntity { ref entity, .. } => entity,
				EditorData::QNPatch { ref current, .. } => current,

				_ => {
					Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
					panic!();
				}
			};

			send_request(
				app,
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

			if let Some(project) = app_state.project.load().as_ref() {
				send_request(
					app,
					Request::Editor(EditorRequest::Entity(EntityEditorRequest::Metadata(
						EntityMetadataRequest::UpdateCustomPaths {
							editor_id: editor_id.to_owned(),
							custom_paths: project.settings.load().custom_paths.to_owned()
						}
					)))
				)?;
			}

			// allow user to modify hash if there is no defined file we're writing to; will automatically convert editor state into entity editor rather than patch editor
			// also allow user to modify hash if it's already an entity
			send_request(
				app,
				Request::Editor(EditorRequest::Entity(EntityEditorRequest::Metadata(
					EntityMetadataRequest::SetHashModificationAllowed {
						editor_id,
						hash_modification_allowed: matches!(editor_state.data, EditorData::QNEntity { .. })
							|| editor_state.file.is_none()
					}
				)))
			)?;
		}

		EntityMetadataEvent::SetFactoryHash {
			editor_id,
			mut factory_hash
		} => {
			let mut is_patch_editor = false;

			if factory_hash != normalise_to_hash(factory_hash.to_owned()) {
				if let Some(project) = app_state.project.load().as_ref() {
					let mut settings = (*project.settings.load_full()).to_owned();
					settings.custom_paths.push(factory_hash.to_owned());

					app.track_event("Save custom path by factory input", None);

					send_request(
						app,
						Request::Editor(EditorRequest::Entity(EntityEditorRequest::Metadata(
							EntityMetadataRequest::UpdateCustomPaths {
								editor_id: editor_id.to_owned(),
								custom_paths: settings.custom_paths.to_owned()
							}
						)))
					)?;

					send_request(
						app,
						Request::Tool(ToolRequest::Settings(SettingsRequest::ChangeProjectSettings(
							settings.to_owned()
						)))
					)?;

					fs::write(project.path.join("project.json"), to_vec(&settings)?)?;
					project.settings.store(settings.into());

					send_notification(
						app,
						Notification {
							kind: NotificationKind::Info,
							title: "Custom path saved".into(),
							subtitle: "The entered path has been saved in your custom paths list.".into()
						}
					)?;
				}

				factory_hash = normalise_to_hash(factory_hash);

				send_request(
					app,
					Request::Editor(EditorRequest::Entity(EntityEditorRequest::Metadata(
						EntityMetadataRequest::SetFactoryHash {
							editor_id: editor_id.to_owned(),
							factory_hash: factory_hash.to_owned()
						}
					)))
				)?;
			}

			{
				let mut editor_state = app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

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
				let (_, state) = app_state.editor_states.remove(&editor_id).context("No such editor")?;

				let EditorState {
					data: EditorData::QNPatch { settings, current, .. },
					file: None
				} = state
				else {
					unreachable!();
				};

				app_state.editor_states.insert(
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
				app,
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
			let mut is_patch_editor = false;

			if blueprint_hash != normalise_to_hash(blueprint_hash.to_owned()) {
				if let Some(project) = app_state.project.load().as_ref() {
					let mut settings = (*project.settings.load_full()).to_owned();
					settings.custom_paths.push(blueprint_hash.to_owned());

					app.track_event("Save custom path by blueprint input", None);

					send_request(
						app,
						Request::Editor(EditorRequest::Entity(EntityEditorRequest::Metadata(
							EntityMetadataRequest::UpdateCustomPaths {
								editor_id: editor_id.to_owned(),
								custom_paths: settings.custom_paths.to_owned()
							}
						)))
					)?;

					send_request(
						app,
						Request::Tool(ToolRequest::Settings(SettingsRequest::ChangeProjectSettings(
							settings.to_owned()
						)))
					)?;

					fs::write(project.path.join("project.json"), to_vec(&settings)?)?;
					project.settings.store(settings.into());

					send_notification(
						app,
						Notification {
							kind: NotificationKind::Info,
							title: "Custom path saved".into(),
							subtitle: "The entered path has been saved in your custom paths list.".into()
						}
					)?;
				}

				blueprint_hash = normalise_to_hash(blueprint_hash);

				send_request(
					app,
					Request::Editor(EditorRequest::Entity(EntityEditorRequest::Metadata(
						EntityMetadataRequest::SetBlueprintHash {
							editor_id: editor_id.to_owned(),
							blueprint_hash: blueprint_hash.to_owned()
						}
					)))
				)?;
			}

			{
				let mut editor_state = app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

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
				let (_, state) = app_state.editor_states.remove(&editor_id).context("No such editor")?;

				let EditorState {
					data: EditorData::QNPatch { settings, current, .. },
					file: None
				} = state
				else {
					unreachable!();
				};

				app_state.editor_states.insert(
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
				app,
				Request::Global(GlobalRequest::SetTabUnsaved {
					id: editor_id,
					unsaved: true
				})
			)?;
		}

		EntityMetadataEvent::SetRootEntity { editor_id, root_entity } => {
			let mut editor_state = app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

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
				app,
				Request::Global(GlobalRequest::SetTabUnsaved {
					id: editor_id,
					unsaved: true
				})
			)?;
		}

		EntityMetadataEvent::SetSubType { editor_id, sub_type } => {
			let mut editor_state = app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

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
				app,
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
			let mut editor_state = app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

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
				app,
				Request::Global(GlobalRequest::SetTabUnsaved {
					id: editor_id,
					unsaved: true
				})
			)?;
		}
	}
}
