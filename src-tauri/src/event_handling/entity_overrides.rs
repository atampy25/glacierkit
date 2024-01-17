use anyhow::{Context, Result};
use fn_error_context::context;
use quickentity_rs::qn_structs::{Entity, Ref};
use serde_json::{from_slice, from_value, Value};
use tauri::{AppHandle, Manager};
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	entity::get_ref_decoration,
	finish_task,
	model::{AppState, EditorRequest, EntityEditorRequest, EntityOverridesRequest, Request},
	rpkg::{extract_latest_resource, hash_list_mapping},
	send_request, start_task
};

#[try_fn]
#[context("Couldn't get overrides decorations for {}", entity.factory_hash)]
pub fn send_overrides_decorations(app: &AppHandle, editor_id: Uuid, entity: &Entity) -> Result<()> {
	let app_state = app.state::<AppState>();

	if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
		&& let Some(hash_list) = app_state.hash_list.load().as_ref()
	{
		let game_version = app_state
			.game_installs
			.iter()
			.try_find(|x| {
				anyhow::Ok(
					x.path
						== *app_state
							.project
							.load()
							.as_ref()
							.unwrap()
							.settings
							.load()
							.game_install
							.as_ref()
							.unwrap()
							.as_path()
				)
			})?
			.context("No such game install")?
			.version;

		let task = start_task(app, "Updating override decorations")?;

		let mapping = hash_list_mapping(hash_list);

		let repository =
			from_slice::<Vec<Value>>(&extract_latest_resource(resource_packages, &mapping, "00204D1AFD76AB13")?.1)?;

		let mut decorations = vec![];

		for property_override in entity.property_overrides.iter() {
			for reference in property_override.entities.iter() {
				if let Some(decoration) = get_ref_decoration(
					resource_packages,
					&app_state.cached_entities,
					game_version,
					&mapping,
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
						&mapping,
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
							&mapping,
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
				&mapping,
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
				&mapping,
				entity,
				&pin_connection_override.from_entity
			) {
				decorations.push(decoration);
			}

			if let Some(decoration) = get_ref_decoration(
				resource_packages,
				&app_state.cached_entities,
				game_version,
				&mapping,
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
				&mapping,
				entity,
				&pin_connection_override_delete.from_entity
			) {
				decorations.push(decoration);
			}

			if let Some(decoration) = get_ref_decoration(
				resource_packages,
				&app_state.cached_entities,
				game_version,
				&mapping,
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
