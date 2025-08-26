use anyhow::{Context, Result, anyhow};
use fn_error_context::context;
use tauri::{AppHandle, Manager};
use tryvial::try_fn;

use crate::{
	model::{AppState, EditorData, EditorRequest, EntityEditorRequest, EntityGeneralEvent, EntityTreeRequest, Request},
	send_request
};

#[try_fn]
#[context("Couldn't handle update content event")]
pub async fn handle(app: &AppHandle, event: EntityGeneralEvent) -> Result<()> {
	let app_state = app.state::<AppState>();

	match event {
		EntityGeneralEvent::SetShowReverseParentRefs {
			editor_id,
			show_reverse_parent_refs
		} => {
			let mut editor_state = app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

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
			let mut editor_state = app_state.editor_states.get_mut(&editor_id).context("No such editor")?;

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
				app,
				Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
					EntityTreeRequest::SetShowDiff {
						editor_id,
						show_diff: show_changes_from_original
					}
				)))
			)?;
		}
	}
}
