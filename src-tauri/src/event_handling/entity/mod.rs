use anyhow::Result;
use fn_error_context::context;
use tauri::AppHandle;
use tryvial::try_fn;

use crate::model::EntityEditorEvent;

pub mod general;
pub mod meta_pane;
pub mod metadata;
pub mod monaco;
pub mod overrides;
pub mod tree;

#[try_fn]
#[context("Couldn't handle entity editor event")]
pub async fn handle(app: &AppHandle, event: EntityEditorEvent) -> Result<()> {
	match event {
		EntityEditorEvent::General(event) => {
			general::handle(app, event).await?;
		}

		EntityEditorEvent::Tree(event) => {
			tree::handle(app, event).await?;
		}

		EntityEditorEvent::Monaco(event) => {
			monaco::handle(app, event).await?;
		}

		EntityEditorEvent::MetaPane(event) => {
			meta_pane::handle(app, event).await?;
		}

		EntityEditorEvent::Metadata(event) => {
			metadata::handle(app, event).await?;
		}

		EntityEditorEvent::Overrides(event) => {
			overrides::handle(app, event).await?;
		}
	}
}
