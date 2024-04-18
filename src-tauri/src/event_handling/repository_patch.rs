use anyhow::{bail, Context, Result};
use fn_error_context::context;
use hashbrown::HashMap;
use indexmap::IndexMap;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde::Serialize;
use serde_json::{from_str, from_value, Value};
use tauri::{AppHandle, Manager};
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	finish_task,
	model::{
		AppState, EditorData, EditorRequest, GlobalRequest, RepositoryPatchEditorEvent, RepositoryPatchEditorRequest,
		Request
	},
	repository::{RepositoryItem, RepositoryItemInformation},
	send_request, start_task
};

#[try_fn]
#[context("Couldn't get information of repository item {item:?}")]
fn get_repository_item_information(item: &RepositoryItem) -> Result<RepositoryItemInformation> {
	if item.data.contains_key("ItemType") {
		if item.data.contains_key("PrimaryConfiguration") {
			RepositoryItemInformation::Weapon {
				name: item
					.data
					.get("CommonName")
					.context("Weapon had no CommonName")?
					.as_str()
					.context("CommonName was not string")?
					.into()
			}
		} else {
			RepositoryItemInformation::Item {
				name: item
					.data
					.get("CommonName")
					.context("Item had no CommonName")?
					.as_str()
					.context("CommonName was not string")?
					.into()
			}
		}
	} else if item.data.contains_key("OutfitVariationIndex") {
		RepositoryItemInformation::NPC {
			name: item
				.data
				.get("Name")
				.context("NPC had no Name")?
				.as_str()
				.context("Name was not string")?
				.into()
		}
	} else if item.data.contains_key("IsHitmanSuit") {
		RepositoryItemInformation::Outfit {
			name: item
				.data
				.get("CommonName")
				.context("Outfit had no CommonName")?
				.as_str()
				.context("CommonName was not string")?
				.into()
		}
	} else if item.data.contains_key("PersistentBoolId") {
		RepositoryItemInformation::MapArea {
			name: item
				.data
				.get("Name")
				.context("Map area had no Name")?
				.as_str()
				.context("Name was not string")?
				.into()
		}
	} else if item.data.contains_key("OnlineTraits")
		&& (item.data.contains_key("Features_") && item.data.len() == 2 || item.data.len() == 1)
	{
		RepositoryItemInformation::Setpiece {
			traits: from_value::<Vec<String>>(item.data.get("OnlineTraits").unwrap().to_owned())
				.context("OnlineTraits was not string array")?
				.into_iter()
				.filter(|x| x != "Setpiece")
				.collect()
		}
	} else if item.data.contains_key("ModifierType") {
		RepositoryItemInformation::Modifier {
			kind: item
				.data
				.get("ModifierType")
				.unwrap()
				.as_str()
				.context("ModifierType was not string")?
				.into()
		}
	} else if item.data.contains_key("Parameter") {
		RepositoryItemInformation::DifficultyParameter {
			name: item
				.data
				.get("Parameter")
				.unwrap()
				.as_str()
				.context("Parameter was not string")?
				.into()
		}
	} else if item.data.contains_key("AmmoConfig") {
		RepositoryItemInformation::MagazineConfig {
			size: item
				.data
				.get("MagazineSize")
				.context("Magazine config had no Name")?
				.as_f64()
				.context("MagazineSize was not number")?,
			tags: from_value::<Vec<String>>(item.data.get("Tags").context("Magazine config had no Tags")?.to_owned())?
		}
	} else if item.data.contains_key("AmmoImpactEffect") {
		RepositoryItemInformation::AmmoConfig {
			name: item
				.data
				.get("Name")
				.context("Ammo config had no Name")?
				.as_str()
				.context("Name was not string")?
				.into()
		}
	} else if item.data.contains_key("PenetratesEnvironment") || item.data.contains_key("DeathContext") {
		RepositoryItemInformation::AmmoBehaviour {
			name: item
				.data
				.get("Name")
				.context("Ammo behaviour had no Name")?
				.as_str()
				.context("Name was not string")?
				.into()
		}
	} else if item.data.contains_key("Name") && item.data.contains_key("Description") && item.data.contains_key("Image")
	{
		RepositoryItemInformation::MasteryItem {
			name: item
				.data
				.get("Name")
				.unwrap()
				.as_str()
				.context("Name was not string")?
				.into()
		}
	} else if item.data.contains_key("Piercing") {
		RepositoryItemInformation::WeaponConfig
	} else if item.data.contains_key("Multiplier") {
		RepositoryItemInformation::ScoreMultiplier {
			name: item
				.data
				.get("Name")
				.context("Score multiplier had no Name")?
				.as_str()
				.context("Name was not string")?
				.into()
		}
	} else if item.data.contains_key("CommonName") && item.data.contains_key("Items") {
		RepositoryItemInformation::ItemBundle {
			name: item
				.data
				.get("CommonName")
				.unwrap()
				.as_str()
				.context("CommonName was not string")?
				.into()
		}
	} else if item.data.contains_key("Guids") {
		RepositoryItemInformation::ItemList
	} else {
		RepositoryItemInformation::Unknown
	}
}

fn get_modified_items(base: &[RepositoryItem], current: &[RepositoryItem]) -> Vec<Uuid> {
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
#[context("Couldn't handle repository patch event")]
pub async fn handle_repository_patch_event(app: &AppHandle, event: RepositoryPatchEditorEvent) -> Result<()> {
	let app_state = app.state::<AppState>();

	match event {
		RepositoryPatchEditorEvent::Initialise { id } => {
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let task = start_task(app, "Loading repository items")?;

			let (base, repository) = match editor_state.data {
				EditorData::RepositoryPatch {
					ref base, ref current, ..
				} => (base, current),

				_ => {
					bail!("Editor {} is not a repository patch editor", id);
				}
			};

			let items = repository
				.par_iter()
				.map(|item| -> Result<_> { Ok((item.id.to_owned(), get_repository_item_information(item)?)) })
				.collect::<Result<_>>()?;

			send_request(
				app,
				Request::Editor(EditorRequest::RepositoryPatch(
					RepositoryPatchEditorRequest::SetRepositoryItems { id, items }
				))
			)?;

			send_request(
				app,
				Request::Editor(EditorRequest::RepositoryPatch(
					RepositoryPatchEditorRequest::SetModifiedRepositoryItems {
						id,
						modified: get_modified_items(base, repository)
					}
				))
			)?;

			finish_task(app, task)?;
		}

		RepositoryPatchEditorEvent::CreateRepositoryItem { id } => {
			let mut editor_state = app_state.editor_states.get_mut(&id).context("No such editor")?;

			let task = start_task(app, "Creating repository item")?;

			let (base, repository) = match editor_state.data {
				EditorData::RepositoryPatch {
					ref base,
					ref mut current,
					..
				} => (base, current),

				_ => {
					bail!("Editor {} is not a repository patch editor", id);
				}
			};

			let new_id = Uuid::new_v4();

			repository.push(RepositoryItem {
				id: new_id.to_owned(),
				data: IndexMap::new()
			});

			send_request(
				app,
				Request::Editor(EditorRequest::RepositoryPatch(
					RepositoryPatchEditorRequest::AddNewRepositoryItem {
						id: id.to_owned(),
						new_item: (new_id.to_owned(), RepositoryItemInformation::Unknown)
					}
				))
			)?;

			send_request(
				app,
				Request::Editor(EditorRequest::RepositoryPatch(
					RepositoryPatchEditorRequest::SetModifiedRepositoryItems {
						id,
						modified: get_modified_items(base, repository)
					}
				))
			)?;

			send_request(app, Request::Global(GlobalRequest::SetTabUnsaved { id, unsaved: true }))?;

			finish_task(app, task)?;
		}

		RepositoryPatchEditorEvent::ResetModifications { id, item } => {
			let mut editor_state = app_state.editor_states.get_mut(&id).context("No such editor")?;

			let task = start_task(app, "Resetting changes")?;

			let (base, repository) = match editor_state.data {
				EditorData::RepositoryPatch {
					ref base,
					ref mut current,
					..
				} => (base, current),

				_ => {
					bail!("Editor {} is not a repository patch editor", id);
				}
			};

			if let Some(base_item) = base.iter().find(|x| x.id == item) {
				*repository
					.iter_mut()
					.find(|x| x.id == item)
					.context("No such item in repository")? = base_item.to_owned();
			} else {
				repository.retain(|x| x.id != item);

				send_request(
					app,
					Request::Editor(EditorRequest::RepositoryPatch(
						RepositoryPatchEditorRequest::RemoveRepositoryItem {
							id: id.to_owned(),
							item
						}
					))
				)?;
			}

			send_request(
				app,
				Request::Editor(EditorRequest::RepositoryPatch(
					RepositoryPatchEditorRequest::SetModifiedRepositoryItems {
						id: id.to_owned(),
						modified: get_modified_items(base, repository)
					}
				))
			)?;

			send_request(
				app,
				Request::Editor(EditorRequest::RepositoryPatch(
					RepositoryPatchEditorRequest::DeselectMonaco { id }
				))
			)?;

			send_request(app, Request::Global(GlobalRequest::SetTabUnsaved { id, unsaved: true }))?;

			finish_task(app, task)?;
		}

		RepositoryPatchEditorEvent::ModifyItem { id, item, data } => {
			let mut editor_state = app_state.editor_states.get_mut(&id).context("No such editor")?;

			let task = start_task(app, "Saving repository item")?;

			let (base, repository) = match editor_state.data {
				EditorData::RepositoryPatch {
					ref base,
					ref mut current,
					..
				} => (base, current),

				_ => {
					bail!("Editor {} is not a repository patch editor", id);
				}
			};

			let itm = repository
				.iter_mut()
				.find(|x| x.id == item)
				.context("No such repository item")?;

			let modified;

			if itm.data != from_str::<IndexMap<String, Value>>(&data)? {
				itm.data = from_str(&data)?;

				modified = true;
			} else {
				modified = false;
			}

			if modified {
				send_request(
					app,
					Request::Editor(EditorRequest::RepositoryPatch(
						RepositoryPatchEditorRequest::SetModifiedRepositoryItems {
							id,
							modified: get_modified_items(base, repository)
						}
					))
				)?;

				send_request(
					app,
					Request::Editor(EditorRequest::RepositoryPatch(
						RepositoryPatchEditorRequest::ModifyItemInformation {
							id,
							info: get_repository_item_information(repository.iter().find(|x| x.id == item).unwrap())?,
							item
						}
					))
				)?;

				send_request(app, Request::Global(GlobalRequest::SetTabUnsaved { id, unsaved: true }))?;
			}

			finish_task(app, task)?;
		}

		RepositoryPatchEditorEvent::SelectItem { id, item } => {
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let task = start_task(app, "Selecting repository item")?;

			let (base, repository) = match editor_state.data {
				EditorData::RepositoryPatch {
					ref base, ref current, ..
				} => (base, current),

				_ => {
					bail!("Editor {} is not a repository patch editor", id);
				}
			};

			let mut buf_orig = Vec::new();
			let formatter_orig = serde_json::ser::PrettyFormatter::with_indent(b"\t");
			let mut ser_orig = serde_json::Serializer::with_formatter(&mut buf_orig, formatter_orig);

			if let Some(orig_item) = base.iter().find(|x| x.id == item) {
				orig_item.data.serialize(&mut ser_orig)?;
			}

			let mut buf = Vec::new();
			let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
			let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

			repository
				.iter()
				.find(|x| x.id == item)
				.context("No such repository item")?
				.data
				.serialize(&mut ser)?;

			send_request(
				app,
				Request::Editor(EditorRequest::RepositoryPatch(
					RepositoryPatchEditorRequest::SetMonacoContent {
						id,
						item,
						orig_data: String::from_utf8(buf_orig)?,
						data: String::from_utf8(buf)?
					}
				))
			)?;

			finish_task(app, task)?;
		}
	}
}
