use anyhow::{Context, Result, anyhow};
use fn_error_context::context;
use quickentity_rs::entity::CommentEntity;
use tauri::{AppHandle, Manager};
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	model::{
		AppState, EditorData, EditorRequest, EditorRequestData, EntityEditorRequest, EntityMetaPaneEvent,
		EntityTreeRequest, Request, TabRequest, TabRequestData
	},
	send_request
};

#[try_fn]
#[context("Couldn't handle entity meta pane event")]
pub async fn handle(app: &AppHandle, editor_id: Uuid, event: EntityMetaPaneEvent) -> Result<()> {
	let app_state = app.state::<AppState>();

	match event {
		EntityMetaPaneEvent::JumpToReference { reference } => {
			send_request(
				app,
				Request::Editor(EditorRequest {
					editor: editor_id,
					data: EditorRequestData::Entity(EntityEditorRequest::Tree(EntityTreeRequest::Select {
						id: Some(reference)
					}))
				})
			)?;
		}

		EntityMetaPaneEvent::SetNotes { entity_id, notes } => {
			let mut editor_state = app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

			let entity = match editor_state.data {
				EditorData::QNEntity { ref mut entity, .. } => entity,
				EditorData::QNPatch { ref mut current, .. } => current,

				_ => {
					Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
					panic!();
				}
			};

			// Remove comment referring to given entity
			entity.comments.retain(|x| x.parent.is_none_or(|x| x != entity_id));

			// Add new comment
			entity.comments.push(CommentEntity {
				parent: Some(entity_id),
				name: "Notes".into(),
				text: notes.into()
			});

			send_request(
				app,
				Request::Tab(TabRequest {
					tab: editor_id.to_owned(),
					data: TabRequestData::SetUnsaved { unsaved: true }
				})
			)?;
		}
	}
}
