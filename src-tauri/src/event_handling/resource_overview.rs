use std::{collections::HashMap, fs, io::Cursor, path::PathBuf, sync::Arc};

use anyhow::{anyhow, Context, Result};
use arc_swap::ArcSwap;
use fn_error_context::context;
use image::io::Reader as ImageReader;
use indexmap::IndexMap;
use rfd::AsyncFileDialog;
use rpkg_rs::runtime::resource::resource_package::ResourcePackage;
use serde_json::{from_slice, from_value, to_vec, Value};
use tauri::{api::process::Command, AppHandle, Manager, State};
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	finish_task,
	game_detection::GameVersion,
	hash_list::HashList,
	model::{
		AppSettings, AppState, EditorData, EditorRequest, EditorState, EditorType, GlobalRequest, JsonPatchType,
		Request, ResourceOverviewData, ResourceOverviewEvent, ResourceOverviewRequest
	},
	ores::{parse_hashes_ores, parse_json_ores, UnlockableItem},
	repository::RepositoryItem,
	resourcelib::{
		convert_generic, h2016_convert_binary_to_blueprint, h2016_convert_binary_to_factory,
		h2_convert_binary_to_blueprint, h2_convert_binary_to_factory, h3_convert_binary_to_blueprint,
		h3_convert_binary_to_factory
	},
	rpkg::{ensure_entity_in_cache, extract_latest_overview_info, extract_latest_resource},
	rpkg_tool::generate_rpkg_meta,
	send_request, start_task,
	wwev::{parse_wwev, WwiseEventData},
	RunCommandExt
};

#[try_fn]
#[context("Couldn't initialise resource overview {id}")]
pub fn initialise_resource_overview(
	app: &AppHandle,
	app_state: &State<AppState>,
	id: Uuid,
	hash: &String,
	resource_packages: &IndexMap<PathBuf, ResourcePackage>,
	resource_reverse_dependencies: &Arc<HashMap<String, Vec<String>>>,
	install: &PathBuf,
	hash_list: &Arc<HashList>
) -> Result<()> {
	let (filetype, chunk_patch, deps) = extract_latest_overview_info(resource_packages, hash)?;

	send_request(
		app,
		Request::Editor(EditorRequest::ResourceOverview(ResourceOverviewRequest::Initialise {
			id,
			hash: hash.to_owned(),
			filetype,
			chunk_patch,
			path_or_hint: hash_list
				.entries
				.get(hash)
				.and_then(|x| x.path.as_ref().or(x.hint.as_ref()).cloned()),
			dependencies: deps
				.iter()
				.map(|(hash, flag)| {
					(
						hash.to_owned(),
						hash_list
							.entries
							.get(hash)
							.map(|x| x.resource_type.to_owned())
							.unwrap_or("".into()),
						hash_list
							.entries
							.get(hash)
							.and_then(|x| x.path.as_ref().or(x.hint.as_ref()).cloned()),
						flag.to_owned()
					)
				})
				.collect(),
			reverse_dependencies: resource_reverse_dependencies
				.get(hash)
				.map(|hashes| {
					hashes
						.iter()
						.map(|hash| {
							(
								hash.to_owned(),
								hash_list
									.entries
									.get(hash)
									.expect("No entry in hash list for resource")
									.resource_type
									.to_owned(),
								hash_list
									.entries
									.get(hash)
									.and_then(|x| x.path.as_ref().or(x.hint.as_ref()).cloned())
							)
						})
						.collect()
				})
				.unwrap_or_default(),
			data: match hash_list
				.entries
				.get(hash)
				.expect("No entry in hash list for resource")
				.resource_type
				.as_ref()
			{
				"TEMP" => {
					ensure_entity_in_cache(
						resource_packages,
						&app_state.cached_entities,
						app_state
							.game_installs
							.iter()
							.try_find(|x| anyhow::Ok(x.path == *install))?
							.context("No such game install")?
							.version,
						hash_list,
						hash
					)?;

					let entity = app_state.cached_entities.read();
					let entity = entity.get(hash).unwrap();

					ResourceOverviewData::Entity {
						blueprint_hash: entity.blueprint_hash.to_owned(),
						blueprint_path_or_hint: hash_list
							.entries
							.get(&entity.blueprint_hash)
							.and_then(|x| x.path.as_ref().or(x.hint.as_ref()).cloned())
					}
				}

				"AIRG" | "TBLU" | "RTLV" | "ATMD" | "CPPT" | "VIDB" | "CBLU" | "CRMD" | "DSWB" | "GFXF" | "GIDX"
				| "WSGB" | "ECPB" | "UICB" | "ENUM" => ResourceOverviewData::GenericRL,

				"ORES" if hash == "0057C2C3941115CA" => ResourceOverviewData::Unlockables,

				"ORES" => ResourceOverviewData::Ores,

				"GFXI" => {
					let data_dir = app.path_resolver().app_data_dir().expect("Couldn't get data dir");
					let temp_file_id = Uuid::new_v4();

					fs::create_dir_all(data_dir.join("temp"))?;

					let (_, res_data) = extract_latest_resource(resource_packages, hash_list, hash)?;

					ImageReader::new(Cursor::new(res_data))
						.with_guessed_format()?
						.decode()?
						.save(data_dir.join("temp").join(format!("{}.png", temp_file_id)))?;

					ResourceOverviewData::Image {
						image_path: data_dir.join("temp").join(format!("{}.png", temp_file_id))
					}
				}

				"WWEV" => {
					let data_dir = app.path_resolver().app_data_dir().expect("Couldn't get data dir");

					fs::create_dir_all(data_dir.join("temp"))?;

					let (res_meta, res_data) = extract_latest_resource(resource_packages, hash_list, hash)?;

					let mut wav_paths = vec![];

					let wwev = parse_wwev(&res_data)?;

					match wwev.data {
						WwiseEventData::NonStreamed(objects) => {
							for object in objects {
								let temp_file_id = Uuid::new_v4();

								fs::write(data_dir.join("temp").join(format!("{}.wem", temp_file_id)), object.data)?;

								Command::new_sidecar("vgmstream-cli")?
									.current_dir(data_dir.join("temp"))
									.args([
										&format!("{}.wem", temp_file_id),
										"-L",
										"-o",
										&format!("{}.wav", temp_file_id)
									])
									.run()
									.context("VGMStream command failed")?;

								wav_paths.push((
									"Embedded audio".into(),
									data_dir.join("temp").join(format!("{}.wav", temp_file_id))
								))
							}
						}

						WwiseEventData::Streamed(objects) => {
							for object in objects {
								let temp_file_id = Uuid::new_v4();

								let wwem_hash = &res_meta
									.hash_reference_data
									.get(object.dependency_index as usize)
									.context("No such WWEM dependency")?
									.hash;

								let (_, wem_data) = extract_latest_resource(resource_packages, hash_list, wwem_hash)?;

								fs::write(data_dir.join("temp").join(format!("{}.wem", temp_file_id)), wem_data)?;

								Command::new_sidecar("vgmstream-cli")?
									.current_dir(data_dir.join("temp"))
									.args([
										&format!("{}.wem", temp_file_id),
										"-L",
										"-o",
										&format!("{}.wav", temp_file_id)
									])
									.run()
									.context("VGMStream command failed")?;

								wav_paths.push((
									wwem_hash.to_owned(),
									data_dir.join("temp").join(format!("{}.wav", temp_file_id))
								))
							}
						}
					}

					ResourceOverviewData::MultiAudio {
						name: wwev.name,
						wav_paths
					}
				}

				"WWES" | "WWEM" => {
					let data_dir = app.path_resolver().app_data_dir().expect("Couldn't get data dir");
					let temp_file_id = Uuid::new_v4();

					fs::create_dir_all(data_dir.join("temp"))?;

					let (_, res_data) = extract_latest_resource(resource_packages, hash_list, hash)?;

					fs::write(data_dir.join("temp").join(format!("{}.wem", temp_file_id)), res_data)?;

					Command::new_sidecar("vgmstream-cli")?
						.current_dir(data_dir.join("temp"))
						.args([
							&format!("{}.wem", temp_file_id),
							"-L",
							"-o",
							&format!("{}.wav", temp_file_id)
						])
						.run()
						.context("VGMStream command failed")?;

					ResourceOverviewData::Audio {
						wav_path: data_dir.join("temp").join(format!("{}.wav", temp_file_id))
					}
				}

				"REPO" => ResourceOverviewData::Repository,

				_ => ResourceOverviewData::Generic
			}
		}))
	)?;
}

#[try_fn]
#[context("Couldn't handle resource overview event")]
pub async fn handle_resource_overview_event(app: &AppHandle, event: ResourceOverviewEvent) -> Result<()> {
	let app_settings = app.state::<ArcSwap<AppSettings>>();
	let app_state = app.state::<AppState>();

	match event {
		ResourceOverviewEvent::Initialise { id } => {
			let editor_state = app_state.editor_states.read().await;
			let editor_state = editor_state.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			let task = start_task(&app, format!("Loading resource overview for {}", hash))?;

			if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
				&& let Some(resource_reverse_dependencies) = app_state.resource_reverse_dependencies.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				initialise_resource_overview(
					&app,
					&app_state,
					id,
					hash,
					resource_packages,
					resource_reverse_dependencies,
					install,
					hash_list
				)?;
			}

			finish_task(&app, task)?;
		}

		ResourceOverviewEvent::FollowDependency { id, new_hash } => {
			let mut editor_state = app_state.editor_states.write().await;
			let editor_state = editor_state.get_mut(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref mut hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			*hash = new_hash.to_owned();

			let task = start_task(&app, format!("Loading resource overview for {}", hash))?;

			if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
				&& let Some(resource_reverse_dependencies) = app_state.resource_reverse_dependencies.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				initialise_resource_overview(
					&app,
					&app_state,
					id,
					hash,
					resource_packages,
					resource_reverse_dependencies,
					install,
					hash_list
				)?;

				send_request(
					&app,
					Request::Global(GlobalRequest::RenameTab {
						id,
						new_name: format!("Resource overview ({new_hash})")
					})
				)?;
			}

			finish_task(&app, task)?;
		}

		ResourceOverviewEvent::FollowDependencyInNewTab { hash, .. } => {
			let id = Uuid::new_v4();

			app_state.editor_states.write().await.insert(
				id.to_owned(),
				EditorState {
					file: None,
					data: EditorData::ResourceOverview { hash: hash.to_owned() }
				}
			);

			send_request(
				&app,
				Request::Global(GlobalRequest::CreateTab {
					id,
					name: format!("Resource overview ({hash})"),
					editor_type: EditorType::ResourceOverview
				})
			)?;
		}

		ResourceOverviewEvent::OpenInEditor { id } => {
			let hash = {
				let editor_state = app_state.editor_states.read().await;
				let editor_state = editor_state.get(&id).context("No such editor")?;
				match editor_state.data {
					EditorData::ResourceOverview { ref hash, .. } => hash,

					_ => {
						Err(anyhow!("Editor {} is not a resource overview", id))?;
						panic!();
					}
				}
				.to_owned()
			};

			// Only available for entities, the repository and unlockables currently

			if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				match hash_list
					.entries
					.get(&hash)
					.context("Not in hash list")?
					.resource_type
					.as_ref()
				{
					"TEMP" => {
						let task = start_task(app, format!("Loading entity {}", hash))?;

						let game_install_data = app_state
							.game_installs
							.iter()
							.try_find(|x| anyhow::Ok(x.path == *install))?
							.context("No such game install")?;

						ensure_entity_in_cache(
							resource_packages,
							&app_state.cached_entities,
							game_install_data.version,
							hash_list,
							&hash
						)?;

						let entity = app_state.cached_entities.read().get(&hash).unwrap().to_owned();

						let default_tab_name = format!(
							"{} ({})",
							entity
								.entities
								.get(&entity.root_entity)
								.context("Root entity doesn't exist")?
								.name,
							hash
						);

						let tab_name = if let Some(entry) = hash_list.entries.get(&hash) {
							if let Some(path) = entry.path.as_ref() {
								path.replace("].pc_entitytype", "")
									.replace("].pc_entitytemplate", "")
									.split('/')
									.last()
									.map(|x| x.to_owned())
									.unwrap_or(default_tab_name)
							} else if let Some(hint) = entry.hint.as_ref() {
								format!("{} ({})", hint, hash)
							} else {
								default_tab_name
							}
						} else {
							default_tab_name
						};

						let id = Uuid::new_v4();

						app_state.editor_states.write().await.insert(
							id.to_owned(),
							EditorState {
								file: None,
								data: EditorData::QNPatch {
									base: Box::new(entity.to_owned()),
									current: Box::new(entity),
									settings: Default::default()
								}
							}
						);

						send_request(
							app,
							Request::Global(GlobalRequest::CreateTab {
								id,
								name: tab_name,
								editor_type: EditorType::QNPatch
							})
						)?;

						finish_task(app, task)?;
					}

					"REPO" => {
						let task = start_task(&app, "Loading repository")?;

						let id = Uuid::new_v4();

						let repository: Vec<RepositoryItem> =
							from_slice(&extract_latest_resource(resource_packages, hash_list, "00204D1AFD76AB13")?.1)?;

						app_state.editor_states.write().await.insert(
							id.to_owned(),
							EditorState {
								file: None,
								data: EditorData::RepositoryPatch {
									base: repository.to_owned(),
									current: repository,
									patch_type: JsonPatchType::MergePatch
								}
							}
						);

						send_request(
							app,
							Request::Global(GlobalRequest::CreateTab {
								id,
								name: "pro.repo".into(),
								editor_type: EditorType::RepositoryPatch {
									patch_type: JsonPatchType::MergePatch
								}
							})
						)?;

						finish_task(app, task)?;
					}

					"ORES" if hash == "0057C2C3941115CA" => {
						let task = start_task(app, "Loading unlockables")?;

						let id = Uuid::new_v4();

						let unlockables: Vec<UnlockableItem> = from_value(parse_json_ores(
							&extract_latest_resource(resource_packages, hash_list, "0057C2C3941115CA")?.1
						)?)?;

						app_state.editor_states.write().await.insert(
							id.to_owned(),
							EditorState {
								file: None,
								data: EditorData::UnlockablesPatch {
									base: unlockables.to_owned(),
									current: unlockables,
									patch_type: JsonPatchType::MergePatch
								}
							}
						);

						send_request(
							app,
							Request::Global(GlobalRequest::CreateTab {
								id,
								name: "config.unlockables".into(),
								editor_type: EditorType::UnlockablesPatch {
									patch_type: JsonPatchType::MergePatch
								}
							})
						)?;

						finish_task(app, task)?;
					}

					x => panic!("Opening {x} files in editor is not supported")
				}
			}
		}

		ResourceOverviewEvent::ExtractAsFile { id } => {
			let editor_state = app_state.editor_states.read().await;
			let editor_state = editor_state.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let (metadata, data) = extract_latest_resource(resource_packages, hash_list, hash)?;
				let metadata_file = generate_rpkg_meta(&metadata)?;

				let file_type = hash_list
					.entries
					.get(hash)
					.expect("Can only open files from the hash list")
					.resource_type
					.to_owned();

				let mut dialog = AsyncFileDialog::new().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(save_handle) = dialog
					.set_file_name(&format!("{}.{}", hash, file_type))
					.add_filter(&format!("{} file", file_type), &[&file_type])
					.save_file()
					.await
				{
					fs::write(save_handle.path(), data)?;

					fs::write(
						save_handle.path().parent().unwrap().join(format!(
							"{}.meta",
							save_handle.path().file_name().unwrap().to_string_lossy()
						)),
						metadata_file
					)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractAsQN { id } => {
			let editor_state = app_state.editor_states.read().await;
			let editor_state = editor_state.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				ensure_entity_in_cache(
					resource_packages,
					&app_state.cached_entities,
					app_state
						.game_installs
						.iter()
						.try_find(|x| anyhow::Ok(x.path == *install))?
						.context("No such game install")?
						.version,
					hash_list,
					hash
				)?;

				let entity_json = {
					let entity = app_state.cached_entities.read();
					let entity = entity.get(hash).unwrap();
					to_vec(entity)?
				};

				let mut dialog = AsyncFileDialog::new().set_title("Extract entity");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(save_handle) = dialog
					.add_filter("QuickEntity entity", &["entity.json"])
					.save_file()
					.await
				{
					fs::write(save_handle.path(), entity_json)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractTEMPAsRT { id } => {
			let editor_state = app_state.editor_states.read().await;
			let editor_state = editor_state.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let (metadata, data) = extract_latest_resource(resource_packages, hash_list, hash)?;
				let metadata_file = generate_rpkg_meta(&metadata)?;

				let data = match app_state
					.game_installs
					.iter()
					.try_find(|x| anyhow::Ok(x.path == *install))?
					.context("No such game install")?
					.version
				{
					GameVersion::H1 => to_vec(
						&h2016_convert_binary_to_factory(&data)
							.context("Couldn't convert binary data to ResourceLib factory")?
					)?,

					GameVersion::H2 => to_vec(
						&h2_convert_binary_to_factory(&data)
							.context("Couldn't convert binary data to ResourceLib factory")?
					)?,

					GameVersion::H3 => to_vec(
						&h3_convert_binary_to_factory(&data)
							.context("Couldn't convert binary data to ResourceLib factory")?
					)?
				};

				let mut dialog = AsyncFileDialog::new().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(save_handle) = dialog
					.set_file_name(&format!("{}.TEMP.json", hash))
					.add_filter("TEMP.json file", &["TEMP.json"])
					.save_file()
					.await
				{
					fs::write(save_handle.path(), data)?;

					fs::write(
						save_handle.path().parent().unwrap().join(format!(
							"{}.meta",
							save_handle.path().file_name().unwrap().to_string_lossy()
						)),
						metadata_file
					)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractTBLUAsFile { id } => {
			let editor_state = app_state.editor_states.read().await;
			let editor_state = editor_state.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let game_version = app_state
					.game_installs
					.iter()
					.try_find(|x| anyhow::Ok(x.path == *install))?
					.context("No such game install")?
					.version;

				ensure_entity_in_cache(
					resource_packages,
					&app_state.cached_entities,
					game_version,
					hash_list,
					hash
				)?;

				let (metadata, data) = extract_latest_resource(resource_packages, hash_list, &{
					let entity = app_state.cached_entities.read();
					let entity = entity.get(hash).unwrap();
					entity.blueprint_hash.to_owned()
				})?;

				let metadata_file = generate_rpkg_meta(&metadata)?;

				let mut dialog = AsyncFileDialog::new().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(save_handle) = dialog
					.set_file_name(&format!("{}.TBLU", metadata.hash_value))
					.add_filter("TBLU file", &["TBLU"])
					.save_file()
					.await
				{
					fs::write(save_handle.path(), data)?;

					fs::write(
						save_handle.path().parent().unwrap().join(format!(
							"{}.meta",
							save_handle.path().file_name().unwrap().to_string_lossy()
						)),
						metadata_file
					)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractTBLUAsRT { id } => {
			let editor_state = app_state.editor_states.read().await;
			let editor_state = editor_state.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let game_version = app_state
					.game_installs
					.iter()
					.try_find(|x| anyhow::Ok(x.path == *install))?
					.context("No such game install")?
					.version;

				ensure_entity_in_cache(
					resource_packages,
					&app_state.cached_entities,
					game_version,
					hash_list,
					hash
				)?;

				let (metadata, data) = extract_latest_resource(resource_packages, hash_list, &{
					let entity = app_state.cached_entities.read();
					let entity = entity.get(hash).unwrap();
					entity.blueprint_hash.to_owned()
				})?;

				let metadata_file = generate_rpkg_meta(&metadata)?;

				let data = match game_version {
					GameVersion::H1 => to_vec(
						&h2016_convert_binary_to_blueprint(&data)
							.context("Couldn't convert binary data to ResourceLib blueprint")?
					)?,

					GameVersion::H2 => to_vec(
						&h2_convert_binary_to_blueprint(&data)
							.context("Couldn't convert binary data to ResourceLib blueprint")?
					)?,

					GameVersion::H3 => to_vec(
						&h3_convert_binary_to_blueprint(&data)
							.context("Couldn't convert binary data to ResourceLib blueprint")?
					)?
				};

				let mut dialog = AsyncFileDialog::new().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(save_handle) = dialog
					.set_file_name(&format!("{}.TBLU", metadata.hash_value))
					.add_filter("TBLU.json file", &["TBLU.json"])
					.save_file()
					.await
				{
					fs::write(save_handle.path(), data)?;

					fs::write(
						save_handle.path().parent().unwrap().join(format!(
							"{}.meta",
							save_handle.path().file_name().unwrap().to_string_lossy()
						)),
						metadata_file
					)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractAsRTGeneric { id } => {
			let editor_state = app_state.editor_states.read().await;
			let editor_state = editor_state.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let game_version = app_state
					.game_installs
					.iter()
					.try_find(|x| anyhow::Ok(x.path == *install))?
					.context("No such game install")?
					.version;

				let (res_meta, res_data) = extract_latest_resource(resource_packages, hash_list, hash)?;

				let mut dialog = AsyncFileDialog::new().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(save_handle) = dialog
					.set_file_name(&format!("{}.{}.json", hash, res_meta.hash_resource_type))
					.add_filter(
						&format!("{}.json file", res_meta.hash_resource_type),
						&[&format!("{}.json", res_meta.hash_resource_type)]
					)
					.save_file()
					.await
				{
					fs::write(
						save_handle.path(),
						to_vec(&convert_generic::<Value>(
							&res_data,
							game_version,
							&res_meta.hash_resource_type
						)?)?
					)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractORESAsJson { id } => {
			let editor_state = app_state.editor_states.read().await;
			let editor_state = editor_state.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let (_, res_data) = extract_latest_resource(resource_packages, hash_list, hash)?;

				let mut dialog = AsyncFileDialog::new().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				let res_data = parse_hashes_ores(&res_data)?;

				if let Some(save_handle) = dialog
					.set_file_name(&format!("{}.json", hash))
					.add_filter("JSON file", &["json"])
					.save_file()
					.await
				{
					fs::write(save_handle.path(), to_vec(&res_data)?)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractAsPng { id } => {
			let editor_state = app_state.editor_states.read().await;
			let editor_state = editor_state.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let (_, res_data) = extract_latest_resource(resource_packages, hash_list, hash)?;

				let mut dialog = AsyncFileDialog::new().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(save_handle) = dialog
					.set_file_name(&format!("{}.png", hash))
					.add_filter("PNG file", &["png"])
					.save_file()
					.await
				{
					ImageReader::new(Cursor::new(res_data))
						.with_guessed_format()?
						.decode()?
						.save(save_handle.path())?;
				}
			}
		}

		ResourceOverviewEvent::ExtractAsWav { id } => {
			let editor_state = app_state.editor_states.read().await;
			let editor_state = editor_state.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let mut dialog = AsyncFileDialog::new().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(save_handle) = dialog
					.set_file_name(&format!("{}.wav", hash))
					.add_filter("WAV file", &["wav"])
					.save_file()
					.await
				{
					let (_, res_data) = extract_latest_resource(resource_packages, hash_list, hash)?;

					let data_dir = app.path_resolver().app_data_dir().expect("Couldn't get data dir");

					let temp_file_id = Uuid::new_v4();

					fs::write(data_dir.join("temp").join(format!("{}.wem", temp_file_id)), res_data)?;

					Command::new_sidecar("vgmstream-cli")?
						.current_dir(data_dir.join("temp"))
						.args([
							&format!("{}.wem", temp_file_id),
							"-L",
							"-o",
							save_handle.path().to_string_lossy().as_ref()
						])
						.run()
						.context("VGMStream command failed")?;
				}
			}
		}

		ResourceOverviewEvent::ExtractMultiWav { id } => {
			let editor_state = app_state.editor_states.read().await;
			let editor_state = editor_state.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let mut dialog = AsyncFileDialog::new().set_title("Extract all WAVs to folder");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(save_handle) = dialog.pick_folder().await {
					let data_dir = app.path_resolver().app_data_dir().expect("Couldn't get data dir");

					let (res_meta, res_data) = extract_latest_resource(resource_packages, hash_list, hash)?;

					let wwev = parse_wwev(&res_data)?;

					let mut idx = 0;

					match wwev.data {
						WwiseEventData::NonStreamed(objects) => {
							for object in objects {
								let temp_file_id = Uuid::new_v4();

								fs::write(data_dir.join("temp").join(format!("{}.wem", temp_file_id)), object.data)?;

								Command::new_sidecar("vgmstream-cli")?
									.current_dir(data_dir.join("temp"))
									.args([
										&format!("{}.wem", temp_file_id),
										"-L",
										"-o",
										save_handle
											.path()
											.join(format!("{}.wav", idx))
											.to_string_lossy()
											.as_ref()
									])
									.run()
									.context("VGMStream command failed")?;

								idx += 1;
							}
						}

						WwiseEventData::Streamed(objects) => {
							for object in objects {
								let temp_file_id = Uuid::new_v4();

								let wwem_hash = &res_meta
									.hash_reference_data
									.get(object.dependency_index as usize)
									.context("No such WWEM dependency")?
									.hash;

								let (_, wem_data) = extract_latest_resource(resource_packages, hash_list, wwem_hash)?;

								fs::write(data_dir.join("temp").join(format!("{}.wem", temp_file_id)), wem_data)?;

								Command::new_sidecar("vgmstream-cli")?
									.current_dir(data_dir.join("temp"))
									.args([
										&format!("{}.wem", temp_file_id),
										"-L",
										"-o",
										save_handle
											.path()
											.join(format!("{}.wav", idx))
											.to_string_lossy()
											.as_ref()
									])
									.run()
									.context("VGMStream command failed")?;

								idx += 1;
							}
						}
					}
				}
			}
		}

		ResourceOverviewEvent::ExtractSpecificMultiWav { id, index } => {
			let editor_state = app_state.editor_states.read().await;
			let editor_state = editor_state.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(resource_packages) = app_state.resource_packages.load().as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let mut dialog = AsyncFileDialog::new().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(save_handle) = dialog
					.set_file_name(&format!("{}~{}.wav", hash, index))
					.add_filter("WAV file", &["wav"])
					.save_file()
					.await
				{
					let data_dir = app.path_resolver().app_data_dir().expect("Couldn't get data dir");

					let (res_meta, res_data) = extract_latest_resource(resource_packages, hash_list, hash)?;

					let wwev = parse_wwev(&res_data)?;

					match wwev.data {
						WwiseEventData::NonStreamed(objects) => {
							let temp_file_id = Uuid::new_v4();

							fs::write(
								data_dir.join("temp").join(format!("{}.wem", temp_file_id)),
								&objects.get(index as usize).context("No such audio object")?.data
							)?;

							Command::new_sidecar("vgmstream-cli")?
								.current_dir(data_dir.join("temp"))
								.args([
									&format!("{}.wem", temp_file_id),
									"-L",
									"-o",
									save_handle.path().to_string_lossy().as_ref()
								])
								.run()
								.context("VGMStream command failed")?;
						}

						WwiseEventData::Streamed(objects) => {
							let temp_file_id = Uuid::new_v4();

							let wwem_hash = &res_meta
								.hash_reference_data
								.get(
									objects
										.get(index as usize)
										.context("No such audio object")?
										.dependency_index as usize
								)
								.context("No such WWEM dependency")?
								.hash;

							let (_, wem_data) = extract_latest_resource(resource_packages, hash_list, wwem_hash)?;

							fs::write(data_dir.join("temp").join(format!("{}.wem", temp_file_id)), wem_data)?;

							Command::new_sidecar("vgmstream-cli")?
								.current_dir(data_dir.join("temp"))
								.args([
									&format!("{}.wem", temp_file_id),
									"-L",
									"-o",
									save_handle.path().to_string_lossy().as_ref()
								])
								.run()
								.context("VGMStream command failed")?;
						}
					}
				}
			}
		}
	}
}
