use std::{collections::HashMap, fs, io::Cursor, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use fn_error_context::context;
use image::io::Reader as ImageReader;
use indexmap::IndexMap;
use rpkg_rs::runtime::resource::resource_package::ResourcePackage;
use tauri::{api::process::Command, AppHandle, State};
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	hash_list::HashList,
	model::{AppState, EditorRequest, Request, ResourceOverviewData, ResourceOverviewRequest},
	rpkg::{ensure_entity_in_cache, extract_latest_overview_info, extract_latest_resource},
	send_request,
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
									.args([&format!("{}.wem", temp_file_id), "-o", &format!("{}.wav", temp_file_id)])
									.run()
									.context("VGMStream command failed")?;

								wav_paths.push(("Embedded audio".into(), data_dir.join("temp").join(format!("{}.wav", temp_file_id))))
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
									.args([&format!("{}.wem", temp_file_id), "-o", &format!("{}.wav", temp_file_id)])
									.run()
									.context("VGMStream command failed")?;

								wav_paths.push((wwem_hash.to_owned(), data_dir.join("temp").join(format!("{}.wav", temp_file_id))))
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
						.args([&format!("{}.wem", temp_file_id), "-o", &format!("{}.wav", temp_file_id)])
						.run()
						.context("VGMStream command failed")?;

					ResourceOverviewData::Audio {
						wav_path: data_dir.join("temp").join(format!("{}.wav", temp_file_id))
					}
				}

				_ => ResourceOverviewData::Generic
			}
		}))
	)?;
}
