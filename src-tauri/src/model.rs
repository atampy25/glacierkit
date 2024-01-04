use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use specta::Type;
use structstruck::strike;

strike! {
	#[strikethrough[derive(Type, Serialize, Deserialize, Clone, Debug)]]
	#[strikethrough[serde(rename_all = "camelCase", tag = "type", content = "data")]]
	pub enum Event {
		Tool(pub enum ToolEvent {
			FileBrowser(pub enum FileBrowserEvent {
				Select {},
				Create {},
				Delete {},
				Move {},
				Rename {}
			})
		}),

		// Editor(pub enum EditorEvent {}),

		Global(pub enum GlobalEvent {
			WorkspaceLoaded {
				path: PathBuf
			}
		})
	}
}

strike! {
	#[strikethrough[derive(Type, Serialize, Deserialize, Clone, Debug)]]
	#[strikethrough[serde(rename_all = "camelCase", tag = "type", content = "data")]]
	pub enum Request {
		Tool(pub enum ToolRequest {
			FileBrowser(pub enum FileBrowserRequest {
				AddFile {},
				DeleteFile {},
				MoveFile {},
				RenameFile {},
				ReplaceTree {}
			})
		}),

		Global(pub enum GlobalRequest {
			ErrorReport { error: String }
		})
	}
}
