use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use fn_error_context::context;
use tauri::{AppHandle, Manager};
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	model::{
		AppState, EditorData, EditorRequest, EditorRequestData, EditorState, EntityEditorRequest, EntityMetadataEvent,
		EntityMetadataRequest, Request, TabRequest, TabRequestData
	},
	send_request
};

#[try_fn]
#[context("Couldn't handle entity metadata event")]
pub async fn handle(app: &AppHandle, editor_id: Uuid, event: EntityMetadataEvent) -> Result<()> {
	let app_state = app.state::<AppState>();

	match event {
		EntityMetadataEvent::Initialise => {
			let editor_state = app_state
				.editor_states
				.get(&editor_id)
				.await
				.context("No such editor")?;

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
				Request::Editor(EditorRequest {
					editor: editor_id.to_owned(),
					data: EditorRequestData::Entity(EntityEditorRequest::Metadata(EntityMetadataRequest::Initialise {
						factory: entity.factory.to_owned(),
						blueprint: entity.blueprint.to_owned(),
						root_entity: entity.root_entity.to_owned(),
						sub_type: entity.sub_type.to_owned(),
						external_scenes: entity.external_scenes.to_owned()
					}))
				})
			)?;

			// allow user to modify hash if there is no defined file we're writing to; will automatically convert editor state into entity editor rather than patch editor
			// also allow user to modify hash if it's already an entity
			send_request(
				app,
				Request::Editor(EditorRequest {
					editor: editor_id.to_owned(),
					data: EditorRequestData::Entity(EntityEditorRequest::Metadata(
						EntityMetadataRequest::SetHashModificationAllowed {
							hash_modification_allowed: matches!(editor_state.data, EditorData::QNEntity { .. })
								|| editor_state.file.is_none()
						}
					))
				})
			)?;
		}

		EntityMetadataEvent::SetFactory { factory } => {
			let mut is_patch_editor = false;

			{
				let mut editor_state = app_state
					.editor_states
					.get_mut(&editor_id)
					.await
					.context("No such editor")?;

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

				let entity = Arc::make_mut(entity);

				entity.factory = factory;
			}

			// If it was a patch editor, we should convert it into an entity editor since now we're working on a new entity
			if is_patch_editor {
				let state = app_state
					.editor_states
					.remove(&editor_id)
					.await
					.context("No such editor")?;

				let EditorState {
					data: EditorData::QNPatch { settings, current, .. },
					file: None,
					assets
				} = state
				else {
					unreachable!();
				};

				app_state
					.editor_states
					.insert(
						editor_id.to_owned(),
						EditorState {
							data: EditorData::QNEntity {
								settings,
								entity: current
							},
							file: None,
							assets
						}
					)
					.await;
			}

			send_request(
				app,
				Request::Tab(TabRequest {
					tab: editor_id,
					data: TabRequestData::SetUnsaved { unsaved: true }
				})
			)?;
		}

		EntityMetadataEvent::SetBlueprint { blueprint } => {
			let mut is_patch_editor = false;

			{
				let mut editor_state = app_state
					.editor_states
					.get_mut(&editor_id)
					.await
					.context("No such editor")?;

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

				let entity = Arc::make_mut(entity);

				entity.blueprint = blueprint;
			}

			// If it was a patch editor, we should convert it into an entity editor since now we're working on a new entity
			if is_patch_editor {
				let state = app_state
					.editor_states
					.remove(&editor_id)
					.await
					.context("No such editor")?;

				let EditorState {
					data: EditorData::QNPatch { settings, current, .. },
					file: None,
					assets
				} = state
				else {
					unreachable!();
				};

				app_state
					.editor_states
					.insert(
						editor_id.to_owned(),
						EditorState {
							data: EditorData::QNEntity {
								settings,
								entity: current
							},
							file: None,
							assets
						}
					)
					.await;
			}

			send_request(
				app,
				Request::Tab(TabRequest {
					tab: editor_id,
					data: TabRequestData::SetUnsaved { unsaved: true }
				})
			)?;
		}

		EntityMetadataEvent::SetRootEntity { root_entity } => {
			let mut editor_state = app_state
				.editor_states
				.get_mut(&editor_id)
				.await
				.context("No such editor")?;

			let entity = match editor_state.data {
				EditorData::QNEntity { ref mut entity, .. } => entity,
				EditorData::QNPatch { ref mut current, .. } => current,

				_ => {
					Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
					panic!();
				}
			};

			let entity = Arc::make_mut(entity);

			entity.root_entity = root_entity;

			send_request(
				app,
				Request::Tab(TabRequest {
					tab: editor_id,
					data: TabRequestData::SetUnsaved { unsaved: true }
				})
			)?;
		}

		EntityMetadataEvent::SetSubType { sub_type } => {
			let mut editor_state = app_state
				.editor_states
				.get_mut(&editor_id)
				.await
				.context("No such editor")?;

			let entity = match editor_state.data {
				EditorData::QNEntity { ref mut entity, .. } => entity,
				EditorData::QNPatch { ref mut current, .. } => current,

				_ => {
					Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
					panic!();
				}
			};

			let entity = Arc::make_mut(entity);

			entity.sub_type = sub_type;

			send_request(
				app,
				Request::Tab(TabRequest {
					tab: editor_id,
					data: TabRequestData::SetUnsaved { unsaved: true }
				})
			)?;
		}

		EntityMetadataEvent::SetExternalScenes { external_scenes } => {
			let mut editor_state = app_state
				.editor_states
				.get_mut(&editor_id)
				.await
				.context("No such editor")?;

			let entity = match editor_state.data {
				EditorData::QNEntity { ref mut entity, .. } => entity,
				EditorData::QNPatch { ref mut current, .. } => current,

				_ => {
					Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
					panic!();
				}
			};

			let entity = Arc::make_mut(entity);

			entity.external_scenes = external_scenes;

			send_request(
				app,
				Request::Tab(TabRequest {
					tab: editor_id,
					data: TabRequestData::SetUnsaved { unsaved: true }
				})
			)?;
		}
	}
}
