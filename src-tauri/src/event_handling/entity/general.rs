use std::{pin::pin, sync::Arc};

use anyhow::{Context, Result, anyhow};
use fn_error_context::context;
use futures_util::StreamExt;
use glacier_commons::metadata::RuntimeID;
use glacier_geometry::render_primitive::LodLevel;
use itertools::Itertools;
use quickentity_rs::entity::SubType;
use tauri::{AppHandle, Manager};
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	HashMap, HashSet, finish_task,
	game::{EntitiesOverlay, GameFiles},
	geometry::RenderSettings,
	handle_event,
	model::{
		AppState, EditorData, EditorRequest, EditorRequestData, EntityEditorRequest, EntityGeneralEvent,
		EntityTreeRequest, Event, Request, SceneRendererEvent
	},
	send_request, start_task
};

#[try_fn]
#[context("Couldn't handle general event")]
pub async fn handle(app: &AppHandle, editor_id: Uuid, event: EntityGeneralEvent) -> Result<()> {
	let app_state = app.state::<AppState>();

	match event {
		EntityGeneralEvent::SetShowReverseParentRefs {
			show_reverse_parent_refs
		} => {
			let mut editor_state = app_state
				.editor_states
				.get_mut(&editor_id)
				.await
				.context("No such editor")?;

			let settings = match editor_state.data {
				EditorData::QNEntity { ref mut settings, .. } => settings,
				EditorData::QNPatch { ref mut settings, .. } => settings,

				_ => {
					Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
					panic!();
				}
			};

			settings.show_reverse_parent_refs = show_reverse_parent_refs;
		}

		EntityGeneralEvent::SetShowChangesFromOriginal {
			show_changes_from_original
		} => {
			let mut editor_state = app_state
				.editor_states
				.get_mut(&editor_id)
				.await
				.context("No such editor")?;

			let settings = match editor_state.data {
				EditorData::QNEntity { ref mut settings, .. } => settings,
				EditorData::QNPatch { ref mut settings, .. } => settings,

				_ => {
					Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
					panic!();
				}
			};

			settings.show_changes_from_original = show_changes_from_original;

			send_request(
				app,
				Request::Editor(EditorRequest {
					editor: editor_id,
					data: EditorRequestData::Entity(EntityEditorRequest::Tree(EntityTreeRequest::SetShowDiff {
						show_diff: show_changes_from_original
					}))
				})
			)?;
		}

		EntityGeneralEvent::StartSceneRenderer => {
			let (factory, sub_type) = {
				let editor_state = app_state
					.editor_states
					.get(&editor_id)
					.await
					.context("No such editor")?;

				match editor_state.data {
					EditorData::QNEntity { ref entity, .. } => (entity.factory, entity.sub_type.to_owned()),
					EditorData::QNPatch { ref current, .. } => (current.factory, current.sub_type.to_owned()),

					_ => {
						Err(anyhow!("Editor {} is not a QN editor", editor_id))?;
						panic!();
					}
				}
			};

			if let Some(game) = app_state.game.load().as_ref() {
				let task = start_task(app, format!("Preparing to render {}", factory))?;

				let mut entities = HashMap::default();

				let mut editor_states = pin!(app_state.editor_states.stream_shards());
				while let Some(shard) = editor_states.next().await {
					for editor in shard.values() {
						if let EditorData::QNEntity { entity, .. } | EditorData::QNPatch { current: entity, .. } =
							&editor.data
						{
							entities.insert(entity.factory, entity.clone());
						}
					}
				}

				let game = Arc::new(EntitiesOverlay {
					game: game.clone(),
					entities
				});

				let mut scenes = HashSet::default();

				#[try_fn]
				fn process(game: &impl GameFiles, scenes: &mut HashSet<RuntimeID>, scene: RuntimeID) -> Result<()> {
					if scenes.insert(scene) {
						for &scene in &game.extract_entity(scene)?.external_scenes {
							process(game, scenes, scene)?;
						}
					}
				}

				process(&*game, &mut scenes, factory)?;

				app_state.scene_renderer.set_game(Some(game));

				app_state.scene_renderer.render(
					&scenes.into_iter().collect_vec(),
					RenderSettings {
						// Scenes can be quite large so we disable lighting
						lighting: sub_type == SubType::Template,
						lod: LodLevel::LEVEL6
					}
				)?;

				std::thread::spawn({
					let app = app.clone();
					move || {
						let app_state = app.state::<AppState>();

						while let Ok(event) = app_state.scene_renderer.recv::<crate::render::scene::EditorEvent>() {
							match event {
								crate::render::scene::EditorEvent::Select { entities } => {
									handle_event(
										&app,
										Event::SceneRenderer(SceneRendererEvent::EntitiesSelected { entities })
									);
								}

								crate::render::scene::EditorEvent::UpdateTransform { entity, transform } => {
									// TODO
								}
							}
						}
					}
				});

				finish_task(app, task)?;
			}
		}
	}
}
