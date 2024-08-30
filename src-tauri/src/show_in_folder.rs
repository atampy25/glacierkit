#[cfg(target_os = "linux")]
use fork::{daemon, Fork};

use std::process::Command;
#[cfg(target_os = "linux")]
use std::{fs::metadata, path::PathBuf};
use tauri::AppHandle;
use tauri_plugin_aptabase::EventTracker;

// from https://github.com/tauri-apps/tauri/issues/4062#issuecomment-1338048169

#[tauri::command]
#[specta::specta]
pub fn show_in_folder(app: AppHandle, path: String) {
	app.track_event("Show in folder", None);

	#[cfg(target_os = "windows")]
	{
		Command::new("explorer")
			.args(["/select,", &path]) // The comma after select is not a typo
			.spawn()
			.unwrap();
	}

	#[cfg(target_os = "linux")]
	{
		if path.contains(",") {
			let new_path = match metadata(&path).unwrap().is_dir() {
				true => path,
				false => {
					let mut path2 = PathBuf::from(path);
					path2.pop();
					path2.into_os_string().into_string().unwrap()
				}
			};

			Command::new("xdg-open").arg(&new_path).spawn().unwrap();
		} else if let Ok(Fork::Child) = daemon(false, false) {
			Command::new("dbus-send")
				.args([
					"--session",
					"--dest=org.freedesktop.FileManager1",
					"--type=method_call",
					"/org/freedesktop/FileManager1",
					"org.freedesktop.FileManager1.ShowItems",
					format!("array:string:\"file://{path}\"").as_str(),
					"string:\"\""
				])
				.spawn()
				.unwrap();
		}
	}

	#[cfg(target_os = "macos")]
	{
		Command::new("open").args(["-R", &path]).spawn().unwrap();
	}
}
