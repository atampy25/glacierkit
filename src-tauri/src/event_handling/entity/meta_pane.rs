use anyhow::{anyhow, Context, Result};
use fn_error_context::context;
use quickentity_rs::qn_structs::{CommentEntity, Ref};
use tauri::{AppHandle, Manager};
use tryvial::try_fn;

use crate::{
	entity::get_local_reference,
	model::{
		AppState, EditorData, EditorRequest, EntityEditorRequest, EntityMetaPaneEvent, EntityTreeRequest,
		GlobalRequest, Request
	},
	send_request
};

#[try_fn]
#[context("Couldn't handle entity meta pane event")]
pub async fn handle(app: &AppHandle, event: EntityMetaPaneEvent) -> Result<()> {
	let app_state = app.state::<AppState>();

	match event {
		EntityMetaPaneEvent::JumpToReference { editor_id, reference } => {
			send_request(
				app,
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
			entity
				.comments
				.retain(|x| get_local_reference(&x.parent).map(|x| x != entity_id).unwrap_or(true));

			// Add new comment
			entity.comments.push(CommentEntity {
				parent: Ref::Short(Some(entity_id)),
				name: "Notes".into(),
				text: notes
			});

			send_request(
				app,
				Request::Global(GlobalRequest::SetTabUnsaved {
					id: editor_id.to_owned(),
					unsaved: true
				})
			)?;
		}
	}
}
