use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use fn_error_context::context;
use indexmap::IndexMap;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde::Serialize;
use serde_json::{Value, from_str};
use tauri::{AppHandle, Manager};
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	finish_task,
	model::{
		AppState, EditorData, EditorRequest, GlobalRequest, Request, UnlockablesPatchEditorEvent,
		UnlockablesPatchEditorRequest
	},
	ores_repo::{UnlockableInformation, UnlockableItem},
	send_request, start_task
};

#[try_fn]
#[context("Couldn't get information of unlockable {item:?}")]
fn get_unlockable_information(item: &UnlockableItem) -> Result<UnlockableInformation> {
	if let Some(ty) = item.data.get("Type") {
		match ty.as_str().context("Type was not string")? {
			"access" => UnlockableInformation::Access {
				id: item.data.get("Id").and_then(|x| x.as_str()).map(|x| x.into())
			},

			"evergreenmastery" => UnlockableInformation::EvergreenMastery {
				id: item.data.get("Id").and_then(|x| x.as_str()).map(|x| x.into())
			},

			"disguise" => UnlockableInformation::Disguise {
				id: item.data.get("Id").and_then(|x| x.as_str()).map(|x| x.into())
			},

			"agencypickup" => UnlockableInformation::AgencyPickup {
				id: item.data.get("Id").and_then(|x| x.as_str()).map(|x| x.into())
			},

			"weapon" => UnlockableInformation::Weapon {
				id: item.data.get("Id").and_then(|x| x.as_str()).map(|x| x.into())
			},

			"gear" => UnlockableInformation::Gear {
				id: item.data.get("Id").and_then(|x| x.as_str()).map(|x| x.into())
			},

			"location" => UnlockableInformation::Location {
				id: item.data.get("Id").and_then(|x| x.as_str()).map(|x| x.into())
			},

			"package" => UnlockableInformation::Package {
				id: item.data.get("Id").and_then(|x| x.as_str()).map(|x| x.into())
			},

			"loadoutunlock" => UnlockableInformation::LoadoutUnlock {
				id: item.data.get("Id").and_then(|x| x.as_str()).map(|x| x.into())
			},

			_ => UnlockableInformation::Unknown {
				id: item.data.get("Id").and_then(|x| x.as_str()).map(|x| x.into())
			}
		}
	} else {
		UnlockableInformation::Unknown {
			id: item.data.get("Id").and_then(|x| x.as_str()).map(|x| x.into())
		}
	}
}

fn get_modified_items(base: &[UnlockableItem], current: &[UnlockableItem]) -> Vec<Uuid> {
	let base_items = base.iter().map(|x| (&x.id, &x.data)).collect::<HashMap<_, _>>();

	current
		.iter()
		.filter(|&current_item| {
			if let Some(&base_item) = base_items.get(&current_item.id) {
				*base_item != current_item.data
			} else {
				true
			}
		})
		.map(|x| x.id.to_owned())
		.collect()
}

#[try_fn]
#[context("Couldn't handle unlockables patch event")]
pub async fn handle_unlockables_patch_event(app: &AppHandle, event: UnlockablesPatchEditorEvent) -> Result<()> {
	let app_state = app.state::<AppState>();

	match event {
		UnlockablesPatchEditorEvent::Initialise { id } => {
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let task = start_task(app, "Loading unlockables")?;

			let (base, unlockables) = match editor_state.data {
				EditorData::UnlockablesPatch {
					ref base, ref current, ..
				} => (base, current),

				_ => {
					bail!("Editor {} is not a unlockables patch editor", id);
				}
			};

			let items = unlockables
				.par_iter()
				.map(|item| -> Result<_> { Ok((item.id.to_owned(), get_unlockable_information(item)?)) })
				.collect::<Result<_>>()?;

			send_request(
				app,
				Request::Editor(EditorRequest::UnlockablesPatch(
					UnlockablesPatchEditorRequest::SetUnlockables { id, unlockables: items }
				))
			)?;

			send_request(
				app,
				Request::Editor(EditorRequest::UnlockablesPatch(
					UnlockablesPatchEditorRequest::SetModifiedUnlockables {
						id,
						modified: get_modified_items(base, unlockables)
					}
				))
			)?;

			finish_task(app, task)?;
		}

		UnlockablesPatchEditorEvent::CreateUnlockable { id } => {
			let mut editor_state = app_state.editor_states.get_mut(&id).context("No such editor")?;

			let task = start_task(app, "Creating unlockable")?;

			let (base, unlockables) = match editor_state.data {
				EditorData::UnlockablesPatch {
					ref base,
					ref mut current,
					..
				} => (base, current),

				_ => {
					bail!("Editor {} is not a unlockables patch editor", id);
				}
			};

			let new_id = Uuid::new_v4();

			unlockables.push(UnlockableItem {
				id: new_id.to_owned(),
				data: {
					let mut x = IndexMap::new();

					x.insert(
						"Id".into(),
						Value::String(format!("ITEM_{}", new_id.to_string().to_uppercase().replace('-', "_")))
					);

					x
				}
			});

			send_request(
				app,
				Request::Editor(EditorRequest::UnlockablesPatch(
					UnlockablesPatchEditorRequest::AddNewUnlockable {
						id: id.to_owned(),
						new_unlockable: (
							new_id.to_owned(),
							UnlockableInformation::Unknown {
								id: Some(format!("ITEM_{}", new_id.to_string().to_uppercase().replace('-', "_")))
							}
						)
					}
				))
			)?;

			send_request(
				app,
				Request::Editor(EditorRequest::UnlockablesPatch(
					UnlockablesPatchEditorRequest::SetModifiedUnlockables {
						id,
						modified: get_modified_items(base, unlockables)
					}
				))
			)?;

			send_request(app, Request::Global(GlobalRequest::SetTabUnsaved { id, unsaved: true }))?;

			finish_task(app, task)?;
		}

		UnlockablesPatchEditorEvent::ResetModifications { id, unlockable } => {
			let mut editor_state = app_state.editor_states.get_mut(&id).context("No such editor")?;

			let task = start_task(app, "Resetting changes")?;

			let (base, unlockables) = match editor_state.data {
				EditorData::UnlockablesPatch {
					ref base,
					ref mut current,
					..
				} => (base, current),

				_ => {
					bail!("Editor {} is not a unlockables patch editor", id);
				}
			};

			if let Some(base_unlockable) = base.iter().find(|x| x.id == unlockable) {
				*unlockables
					.iter_mut()
					.find(|x| x.id == unlockable)
					.context("No such item in unlockables")? = base_unlockable.to_owned();
			} else {
				unlockables.retain(|x| x.id != unlockable);

				send_request(
					app,
					Request::Editor(EditorRequest::UnlockablesPatch(
						UnlockablesPatchEditorRequest::RemoveUnlockable {
							id: id.to_owned(),
							unlockable
						}
					))
				)?;
			}

			send_request(
				app,
				Request::Editor(EditorRequest::UnlockablesPatch(
					UnlockablesPatchEditorRequest::SetModifiedUnlockables {
						id: id.to_owned(),
						modified: get_modified_items(base, unlockables)
					}
				))
			)?;

			send_request(
				app,
				Request::Editor(EditorRequest::UnlockablesPatch(
					UnlockablesPatchEditorRequest::DeselectMonaco { id }
				))
			)?;

			send_request(app, Request::Global(GlobalRequest::SetTabUnsaved { id, unsaved: true }))?;

			finish_task(app, task)?;
		}

		UnlockablesPatchEditorEvent::ModifyUnlockable { id, unlockable, data } => {
			let mut editor_state = app_state.editor_states.get_mut(&id).context("No such editor")?;

			let task = start_task(app, "Saving unlockable")?;

			let (base, unlockables) = match editor_state.data {
				EditorData::UnlockablesPatch {
					ref base,
					ref mut current,
					..
				} => (base, current),

				_ => {
					bail!("Editor {} is not a unlockables patch editor", id);
				}
			};

			let itm = unlockables
				.iter_mut()
				.find(|x| x.id == unlockable)
				.context("No such unlockable")?;

			let modified;

			// All items must have an Id to be saved in SMF unlockables format
			if itm.data != from_str::<IndexMap<String, Value>>(&data)?
				&& from_str::<IndexMap<String, Value>>(&data)?.contains_key("Id")
			{
				itm.data = from_str(&data)?;

				modified = true;
			} else {
				modified = false;
			}

			if modified {
				send_request(
					app,
					Request::Editor(EditorRequest::UnlockablesPatch(
						UnlockablesPatchEditorRequest::SetModifiedUnlockables {
							id,
							modified: get_modified_items(base, unlockables)
						}
					))
				)?;

				send_request(
					app,
					Request::Editor(EditorRequest::UnlockablesPatch(
						UnlockablesPatchEditorRequest::ModifyUnlockableInformation {
							id,
							info: get_unlockable_information(unlockables.iter().find(|x| x.id == unlockable).unwrap())?,
							unlockable
						}
					))
				)?;

				send_request(app, Request::Global(GlobalRequest::SetTabUnsaved { id, unsaved: true }))?;
			}

			finish_task(app, task)?;
		}

		UnlockablesPatchEditorEvent::SelectUnlockable { id, unlockable } => {
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let task = start_task(app, "Selecting unlockable")?;

			let (base, unlockables) = match editor_state.data {
				EditorData::UnlockablesPatch {
					ref base, ref current, ..
				} => (base, current),

				_ => {
					bail!("Editor {} is not a unlockables patch editor", id);
				}
			};

			let mut buf_orig = Vec::new();
			let formatter_orig = serde_json::ser::PrettyFormatter::with_indent(b"\t");
			let mut ser_orig = serde_json::Serializer::with_formatter(&mut buf_orig, formatter_orig);

			if let Some(orig_unlockable) = base.iter().find(|x| x.id == unlockable) {
				orig_unlockable.data.serialize(&mut ser_orig)?;
			}

			let mut buf = Vec::new();
			let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
			let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

			unlockables
				.iter()
				.find(|x| x.id == unlockable)
				.context("No such unlockable")?
				.data
				.serialize(&mut ser)?;

			send_request(
				app,
				Request::Editor(EditorRequest::UnlockablesPatch(
					UnlockablesPatchEditorRequest::SetMonacoContent {
						id,
						unlockable,
						orig_data: String::from_utf8(buf_orig)?,
						data: String::from_utf8(buf)?
					}
				))
			)?;

			finish_task(app, task)?;
		}
	}
}
