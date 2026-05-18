use std::path::PathBuf;

use tauri::AppHandle;
use tauri_plugin_aptabase::EventTracker;

#[tauri::command(async)]
#[specta::specta]
pub fn show_in_folder(app: AppHandle, path: PathBuf) {
	app.track_event("Show in folder", None).unwrap();

	showfile::show_path_in_file_manager(path);
}
