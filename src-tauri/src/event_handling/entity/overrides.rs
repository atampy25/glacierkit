use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use fn_error_context::context;
use itertools::Itertools;
use quickentity_rs::{
	entity::{Entity, PinConnectionOverride, PinConnectionOverrideDelete, PropertyOverride},
	variant::Variant
};
use serde_json::from_str;
use tauri::{AppHandle, Manager};
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	biome::to_string_clear,
	entity::{get_ref_decoration, visit_variant},
	finish_task,
	game::GameFiles,
	model::{
		AppState, EditorData, EditorRequest, EditorRequestData, EntityEditorRequest, EntityOverridesEvent,
		EntityOverridesRequest, Request, TabRequest, TabRequestData
	},
	send_request, start_task
};

#[try_fn]
#[context("Couldn't get overrides decorations for {}", entity.factory)]
pub fn send_overrides_decorations(app: &AppHandle, editor_id: Uuid, entity: &Entity) -> Result<()> {
	let app_state = app.state::<AppState>();

	if let Some(game) = app_state.game.load().as_ref() {
		let task = start_task(app, "Updating override decorations")?;

		let mut decorations = vec![];

		for property_override in entity.property_overrides.iter() {
			for reference in property_override.entities.iter() {
				if let Some(decoration) = get_ref_decoration(game, entity, Some(reference)) {
					decorations.push(decoration);
				}
			}

			for property_data in property_override.properties.values() {
				visit_variant(property_data, &mut |val| match val {
					Variant::Ref(val) => {
						if let Some(decoration) = get_ref_decoration(game, entity, val.as_ref()) {
							decorations.push(decoration);
						}
					}

					Variant::Uuid(uuid) => {
						if let Some(repo) = game.repository()
							&& let Some(repo_item) = repo.iter().find(|x| x.id == *uuid)
							&& let Some(name) = repo_item.data.get("Name").or(repo_item.data.get("CommonName"))
						{
							decorations
								.push((uuid.to_string(), name.as_str().unwrap_or("Non-string value").to_owned()));
						}
					}

					_ => {}
				});
			}
		}

		for reference in entity.override_deletes.iter() {
			if let Some(decoration) = get_ref_decoration(game, entity, Some(reference)) {
				decorations.push(decoration);
			}
		}

		for pin_connection_override in entity.pin_connection_overrides.iter() {
			if let Some(decoration) = get_ref_decoration(game, entity, Some(&pin_connection_override.from_entity)) {
				decorations.push(decoration);
			}

			if let Some(decoration) = get_ref_decoration(game, entity, Some(&pin_connection_override.to_entity)) {
				decorations.push(decoration);
			}
		}

		for pin_connection_override_delete in entity.pin_connection_override_deletes.iter() {
			if let Some(decoration) =
				get_ref_decoration(game, entity, Some(&pin_connection_override_delete.from_entity))
			{
				decorations.push(decoration);
			}

			if let Some(decoration) = get_ref_decoration(game, entity, Some(&pin_connection_override_delete.to_entity))
			{
				decorations.push(decoration);
			}
		}

		send_request(
			app,
			Request::Editor(EditorRequest {
				editor: editor_id,
				data: EditorRequestData::Entity(EntityEditorRequest::Overrides(
					EntityOverridesRequest::UpdateDecorations {
						decorations: decorations.into_iter().unique().collect()
					}
				))
			})
		)?;

		finish_task(app, task)?;
	}
}

#[try_fn]
#[context("Couldn't handle entity overrides event")]
pub async fn handle(app: &AppHandle, editor_id: Uuid, event: EntityOverridesEvent) -> Result<()> {
	let app_state = app.state::<AppState>();

	match event {
		EntityOverridesEvent::Initialise => {
			let editor_state = app_state
				.editor_states
				.get(&editor_id)
				.await
				.context("No such editor")?;

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
				Request::Editor(EditorRequest {
					editor: editor_id,
					data: EditorRequestData::Entity(EntityEditorRequest::Overrides(
						EntityOverridesRequest::Initialise {
							property_overrides: to_string_clear(&entity.property_overrides)?,
							override_deletes: to_string_clear(&entity.override_deletes)?,
							pin_connection_overrides: to_string_clear(&entity.pin_connection_overrides)?,
							pin_connection_override_deletes: to_string_clear(&entity.pin_connection_override_deletes)?
						}
					))
				})
			)?;

			send_overrides_decorations(app, editor_id, entity)?;
		}

		EntityOverridesEvent::UpdatePropertyOverrides { content } => {
			let mut editor_state = app_state
				.editor_states
				.get_mut(&editor_id)
				.await
				.context("No such editor")?;

			let entity = match editor_state.data {
				EditorData::QNEntity { ref mut entity, .. } => entity,
				EditorData::QNPatch { ref mut current, .. } => current,

				_ => {
					Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
					panic!();
				}
			};

			if let Ok(deserialised) = from_str::<Vec<PropertyOverride>>(&content)
				&& (entity.property_overrides.len() != deserialised.len()
					|| entity.property_overrides.iter().zip(deserialised.iter()).any(|(a, b)| {
						a.entities != b.entities
							|| a.properties.len() != b.properties.len()
							|| a.properties
								.iter()
								.zip(b.properties.iter())
								.any(|(a, b)| a.0 != b.0 || !a.1.rough_eq(b.1))
					})) {
				let entity = Arc::make_mut(entity);

				entity.property_overrides = deserialised;

				send_overrides_decorations(app, editor_id.to_owned(), entity)?;

				send_request(
					app,
					Request::Tab(TabRequest {
						tab: editor_id,
						data: TabRequestData::SetUnsaved { unsaved: true }
					})
				)?;
			}
		}

		EntityOverridesEvent::UpdateOverrideDeletes { content } => {
			let mut editor_state = app_state
				.editor_states
				.get_mut(&editor_id)
				.await
				.context("No such editor")?;

			let entity = match editor_state.data {
				EditorData::QNEntity { ref mut entity, .. } => entity,
				EditorData::QNPatch { ref mut current, .. } => current,

				_ => {
					Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
					panic!();
				}
			};

			if let Ok(deserialised) = from_str(&content)
				&& entity.override_deletes != deserialised
			{
				let entity = Arc::make_mut(entity);

				entity.override_deletes = deserialised;

				send_overrides_decorations(app, editor_id.to_owned(), entity)?;

				send_request(
					app,
					Request::Tab(TabRequest {
						tab: editor_id,
						data: TabRequestData::SetUnsaved { unsaved: true }
					})
				)?;
			}
		}

		EntityOverridesEvent::UpdatePinConnectionOverrides { content } => {
			let mut editor_state = app_state
				.editor_states
				.get_mut(&editor_id)
				.await
				.context("No such editor")?;

			let entity = match editor_state.data {
				EditorData::QNEntity { ref mut entity, .. } => entity,
				EditorData::QNPatch { ref mut current, .. } => current,

				_ => {
					Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
					panic!();
				}
			};

			if let Ok(deserialised) = from_str::<Vec<PinConnectionOverride>>(&content)
				&& (entity.pin_connection_overrides.len() != deserialised.len()
					|| entity
						.pin_connection_overrides
						.iter()
						.zip(deserialised.iter())
						.any(|(a, b)| {
							a.from_entity != b.from_entity
								|| a.to_entity != b.to_entity
								|| a.from_pin != b.from_pin
								|| a.to_pin != b.to_pin || (a.value.is_some() && b.value.is_none())
								|| (a.value.is_none() && b.value.is_some())
								|| (!a
									.value
									.as_ref()
									.zip(b.value.as_ref())
									.is_some_and(|(a, b)| a.rough_eq(b)))
						})) {
				let entity = Arc::make_mut(entity);

				entity.pin_connection_overrides = deserialised;

				send_overrides_decorations(app, editor_id.to_owned(), entity)?;

				send_request(
					app,
					Request::Tab(TabRequest {
						tab: editor_id,
						data: TabRequestData::SetUnsaved { unsaved: true }
					})
				)?;
			}
		}

		EntityOverridesEvent::UpdatePinConnectionOverrideDeletes { content } => {
			let mut editor_state = app_state
				.editor_states
				.get_mut(&editor_id)
				.await
				.context("No such editor")?;

			let entity = match editor_state.data {
				EditorData::QNEntity { ref mut entity, .. } => entity,
				EditorData::QNPatch { ref mut current, .. } => current,

				_ => {
					Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
					panic!();
				}
			};

			if let Ok(deserialised) = from_str::<Vec<PinConnectionOverrideDelete>>(&content)
				&& (entity.pin_connection_override_deletes.len() != deserialised.len()
					|| entity
						.pin_connection_override_deletes
						.iter()
						.zip(deserialised.iter())
						.any(|(a, b)| {
							a.from_entity != b.from_entity
								|| a.to_entity != b.to_entity
								|| a.from_pin != b.from_pin
								|| a.to_pin != b.to_pin || (a.value.is_some() && b.value.is_none())
								|| (a.value.is_none() && b.value.is_some())
								|| (!a
									.value
									.as_ref()
									.zip(b.value.as_ref())
									.is_some_and(|(a, b)| a.rough_eq(b)))
						})) {
				let entity = Arc::make_mut(entity);

				entity.pin_connection_override_deletes = deserialised;

				send_overrides_decorations(app, editor_id.to_owned(), entity)?;

				send_request(
					app,
					Request::Tab(TabRequest {
						tab: editor_id,
						data: TabRequestData::SetUnsaved { unsaved: true }
					})
				)?;
			}
		}
	}
}
