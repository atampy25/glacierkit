// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Specta creates non snake case functions
#![allow(non_snake_case)]
#![feature(try_blocks)]

pub mod model;

use std::fmt::Debug;

use anyhow::{Context, Error, Result};
use fn_error_context::context;
use model::{Event, FileBrowserEvent, GlobalEvent, GlobalRequest, Request, ToolEvent, ToolRequest};
use tauri::{async_runtime, AppHandle, Manager};
use tryvial::try_fn;
use uuid::Uuid;

fn main() {
	#[cfg(debug_assertions)]
	{
		tauri_specta::ts::export(specta::collect_types![event], "../src/lib/bindings.ts").unwrap();
	}

	tauri::Builder::default()
		.invoke_handler(tauri::generate_handler![event])
		.run(tauri::generate_context!())
		.expect("error while running tauri application");
}

#[tauri::command]
#[specta::specta]
fn event(app: AppHandle, event: Event) {
	async_runtime::spawn(async move {
		if let Err::<_, Error>(e) = try {
			match event {
				Event::Tool(event) => match event {
					ToolEvent::FileBrowser(event) => match event {
						FileBrowserEvent::Select {} => {}
						FileBrowserEvent::Create {} => {}
						FileBrowserEvent::Delete {} => {}
						FileBrowserEvent::Move {} => {}
						FileBrowserEvent::Rename {} => {}
					}
				},

				// Event::Editor(event) => match event {},
				Event::Global(event) => match event {
					GlobalEvent::WorkspaceLoaded { path } => {
						let task = start_task(&app, "Loading workspace")?;

						tokio::time::sleep(std::time::Duration::from_millis(5000)).await;

						finish_task(&app, task)?;
					}
				}
			}
		} {
			send_request(
				&app,
				Request::Global(GlobalRequest::ErrorReport {
					error: format!("{:?}", e)
				})
			)
			.expect("Couldn't send error report to frontend");
		}
	});
}

#[try_fn]
#[context("Couldn't send task start event for {:?} to frontend", name)]
pub fn start_task(app: &AppHandle, name: &str) -> Result<Uuid> {
	let task_id = Uuid::new_v4();
	app.emit_all("start-task", (&task_id, name))?;
	task_id
}

#[try_fn]
#[context("Couldn't send task finish event for {:?} to frontend", task)]
pub fn finish_task(app: &AppHandle, task: Uuid) -> Result<()> {
	app.emit_all("finish-task", &task)?;
}

#[try_fn]
#[context("Couldn't send request {:?} to frontend", request)]
pub fn send_request(app: &AppHandle, request: Request) -> Result<()> {
	app.emit_all("request", &request)?;
}
