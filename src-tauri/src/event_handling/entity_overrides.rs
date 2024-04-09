use anyhow::{anyhow, Context, Result};
use arc_swap::ArcSwap;
use fn_error_context::context;
use quickentity_rs::qn_structs::{Entity, Ref};
use serde::Serialize;
use serde_json::{from_slice, from_str, from_value, Value};
use tauri::{AppHandle, Manager};
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	entity::get_ref_decoration,
	finish_task,
	model::{
		AppSettings, AppState, EditorData, EditorRequest, EntityEditorRequest, EntityOverridesEvent,
		EntityOverridesRequest, GlobalRequest, Request
	},
	rpkg::extract_latest_resource,
	send_request, start_task
};

#[try_fn]
#[context("Couldn't get overrides decorations for {}", entity.factory_hash)]
pub fn send_overrides_decorations(app: &AppHandle, editor_id: Uuid, entity: &Entity) -> Result<()> {
	let app_state = app.state::<AppState>();
	let app_settings = app.state::<ArcSwap<AppSettings>>();

	if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
		&& let Some(hash_list) = app_state.hash_list.load().as_ref()
		&& let Some(install) = app_settings.load().game_install.as_ref()
	{
		let game_version = app_state
			.game_installs
			.iter()
			.try_find(|x| anyhow::Ok(x.path == *install))?
			.context("No such game install")?
			.version;

		let task = start_task(app, "Updating override decorations")?;

		let repository =
			from_slice::<Vec<Value>>(&extract_latest_resource(resource_packages, hash_list, "00204D1AFD76AB13")?.1)?;

		let mut decorations = vec![];

		for property_override in entity.property_overrides.iter() {
			for reference in property_override.entities.iter() {
				if let Some(decoration) = get_ref_decoration(
					resource_packages,
					&app_state.cached_entities,
					game_version,
					hash_list,
					entity,
					reference
				) {
					decorations.push(decoration);
				}
			}

			for property_data in property_override.properties.values() {
				if property_data.property_type == "SEntityTemplateReference" {
					if let Some(decoration) = get_ref_decoration(
						resource_packages,
						&app_state.cached_entities,
						game_version,
						hash_list,
						entity,
						&from_value::<Ref>(property_data.value.to_owned()).context("Invalid reference")?
					) {
						decorations.push(decoration);
					}
				} else if property_data.property_type == "TArray<SEntityTemplateReference>" {
					for reference in
						from_value::<Vec<Ref>>(property_data.value.to_owned()).context("Invalid reference array")?
					{
						if let Some(decoration) = get_ref_decoration(
							resource_packages,
							&app_state.cached_entities,
							game_version,
							hash_list,
							entity,
							&reference
						) {
							decorations.push(decoration);
						}
					}
				} else if property_data.property_type == "ZGuid" {
					let repository_id =
						from_value::<String>(property_data.value.to_owned()).context("Invalid ZGuid")?;

					if let Some(repo_item) = repository.iter().try_find(|x| {
						anyhow::Ok(
							x.get("ID_")
								.context("No ID on repository item")?
								.as_str()
								.context("ID was not string")? == repository_id
						)
					})? {
						if let Some(name) = repo_item.get("Name").or(repo_item.get("CommonName")) {
							decorations.push((
								repository_id,
								name.as_str().context("Name or CommonName was not string")?.to_owned()
							));
						}
					}
				} else if property_data.property_type == "TArray<ZGuid>" {
					for repository_id in
						from_value::<Vec<String>>(property_data.value.to_owned()).context("Invalid ZGuid array")?
					{
						if let Some(repo_item) = repository.iter().try_find(|x| {
							anyhow::Ok(
								x.get("ID_")
									.context("No ID on repository item")?
									.as_str()
									.context("ID was not string")? == repository_id
							)
						})? {
							if let Some(name) = repo_item.get("Name").or(repo_item.get("CommonName")) {
								decorations.push((
									repository_id,
									name.as_str().context("Name or CommonName was not string")?.to_owned()
								));
							}
						}
					}
				}
			}
		}

		for reference in entity.override_deletes.iter() {
			if let Some(decoration) = get_ref_decoration(
				resource_packages,
				&app_state.cached_entities,
				game_version,
				hash_list,
				entity,
				reference
			) {
				decorations.push(decoration);
			}
		}

		for pin_connection_override in entity.pin_connection_overrides.iter() {
			if let Some(decoration) = get_ref_decoration(
				resource_packages,
				&app_state.cached_entities,
				game_version,
				hash_list,
				entity,
				&pin_connection_override.from_entity
			) {
				decorations.push(decoration);
			}

			if let Some(decoration) = get_ref_decoration(
				resource_packages,
				&app_state.cached_entities,
				game_version,
				hash_list,
				entity,
				&pin_connection_override.to_entity
			) {
				decorations.push(decoration);
			}
		}

		for pin_connection_override_delete in entity.pin_connection_override_deletes.iter() {
			if let Some(decoration) = get_ref_decoration(
				resource_packages,
				&app_state.cached_entities,
				game_version,
				hash_list,
				entity,
				&pin_connection_override_delete.from_entity
			) {
				decorations.push(decoration);
			}

			if let Some(decoration) = get_ref_decoration(
				resource_packages,
				&app_state.cached_entities,
				game_version,
				hash_list,
				entity,
				&pin_connection_override_delete.to_entity
			) {
				decorations.push(decoration);
			}
		}

		send_request(
			app,
			Request::Editor(EditorRequest::Entity(EntityEditorRequest::Overrides(
				EntityOverridesRequest::UpdateDecorations { editor_id, decorations }
			)))
		)?;

		finish_task(app, task)?;
	}
}

#[try_fn]
#[context("Couldn't handle entity overrides event")]
pub async fn handle_entity_overrides_event(app: &AppHandle, event: EntityOverridesEvent) -> Result<()> {
	let app_state = app.state::<AppState>();

	match event {
		EntityOverridesEvent::Initialise { editor_id } => {
			let editor_state = app_state.editor_states.read().await;
			let editor_state = editor_state.get(&editor_id).context("No such editor")?;

			let entity = match editor_state.data {
				EditorData::QNEntity { ref entity, .. } => entity,
				EditorData::QNPatch { ref current, .. } => current,

				_ => {
					Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
					panic!();
				}
			};

			send_request(
				app,
				Request::Editor(EditorRequest::Entity(EntityEditorRequest::Overrides(
					EntityOverridesRequest::Initialise {
						editor_id,
						property_overrides: {
							let mut buf = Vec::new();
							let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
							let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

							entity.property_overrides.serialize(&mut ser)?;

							String::from_utf8(buf)?
						},
						override_deletes: {
							let mut buf = Vec::new();
							let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
							let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

							entity.override_deletes.serialize(&mut ser)?;

							String::from_utf8(buf)?
						},
						pin_connection_overrides: {
							let mut buf = Vec::new();
							let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
							let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

							entity.pin_connection_overrides.serialize(&mut ser)?;

							String::from_utf8(buf)?
						},
						pin_connection_override_deletes: {
							let mut buf = Vec::new();
							let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
							let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

							entity.pin_connection_override_deletes.serialize(&mut ser)?;

							String::from_utf8(buf)?
						}
					}
				)))
			)?;

			send_overrides_decorations(app, editor_id, entity)?;
		}

		EntityOverridesEvent::UpdatePropertyOverrides { editor_id, content } => {
			let mut editor_state = app_state.editor_states.write().await;
			let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

			let entity = match editor_state.data {
				EditorData::QNEntity { ref mut entity, .. } => entity,
				EditorData::QNPatch { ref mut current, .. } => current,

				_ => {
					Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
					panic!();
				}
			};

			if let Ok(deserialised) = from_str(&content) {
				if entity.property_overrides != deserialised {
					entity.property_overrides = deserialised;

					send_overrides_decorations(app, editor_id.to_owned(), entity)?;

					send_request(
						app,
						Request::Global(GlobalRequest::SetTabUnsaved {
							id: editor_id,
							unsaved: true
						})
					)?;
				}
			}
		}

		EntityOverridesEvent::UpdateOverrideDeletes { editor_id, content } => {
			let mut editor_state = app_state.editor_states.write().await;
			let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

			let entity = match editor_state.data {
				EditorData::QNEntity { ref mut entity, .. } => entity,
				EditorData::QNPatch { ref mut current, .. } => current,

				_ => {
					Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
					panic!();
				}
			};

			if let Ok(deserialised) = from_str(&content) {
				if entity.override_deletes != deserialised {
					entity.override_deletes = deserialised;

					send_overrides_decorations(app, editor_id.to_owned(), entity)?;

					send_request(
						app,
						Request::Global(GlobalRequest::SetTabUnsaved {
							id: editor_id,
							unsaved: true
						})
					)?;
				}
			}
		}

		EntityOverridesEvent::UpdatePinConnectionOverrides { editor_id, content } => {
			let mut editor_state = app_state.editor_states.write().await;
			let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

			let entity = match editor_state.data {
				EditorData::QNEntity { ref mut entity, .. } => entity,
				EditorData::QNPatch { ref mut current, .. } => current,

				_ => {
					Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
					panic!();
				}
			};

			if let Ok(deserialised) = from_str(&content) {
				if entity.pin_connection_overrides != deserialised {
					entity.pin_connection_overrides = deserialised;

					send_overrides_decorations(app, editor_id.to_owned(), entity)?;

					send_request(
						app,
						Request::Global(GlobalRequest::SetTabUnsaved {
							id: editor_id,
							unsaved: true
						})
					)?;
				}
			}
		}

		EntityOverridesEvent::UpdatePinConnectionOverrideDeletes { editor_id, content } => {
			let mut editor_state = app_state.editor_states.write().await;
			let editor_state = editor_state.get_mut(&editor_id).context("No such editor")?;

			let entity = match editor_state.data {
				EditorData::QNEntity { ref mut entity, .. } => entity,
				EditorData::QNPatch { ref mut current, .. } => current,

				_ => {
					Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
					panic!();
				}
			};

			if let Ok(deserialised) = from_str(&content) {
				if entity.pin_connection_override_deletes != deserialised {
					entity.pin_connection_override_deletes = deserialised;

					send_overrides_decorations(app, editor_id.to_owned(), entity)?;

					send_request(
						app,
						Request::Global(GlobalRequest::SetTabUnsaved {
							id: editor_id,
							unsaved: true
						})
					)?;
				}
			}
		}
	}
}
