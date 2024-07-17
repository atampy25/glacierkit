use std::{fs, io::Cursor, ops::Deref, path::PathBuf, sync::Arc};

use anyhow::{anyhow, bail, Context, Result};
use arc_swap::ArcSwap;
use fn_error_context::context;
use hashbrown::HashMap;
use image::{io::Reader as ImageReader, ImageFormat};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use rfd::AsyncFileDialog;
use rpkg_rs::{resource::partition_manager::PartitionManager, GlacierResource};
use serde::Serialize;
use serde_json::{from_slice, json, to_string, to_vec, Value};
use tauri::{api::process::Command, AppHandle, Manager, State};
use tauri_plugin_aptabase::EventTracker;
use tex_rs::texture_map::TextureMap;
use tonytools::hmlanguages;
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	biome::format_json,
	finish_task,
	game_detection::GameVersion,
	general::open_in_editor,
	hash_list::HashList,
	languages::get_language_map,
	model::{
		AppSettings, AppState, EditorData, EditorRequest, EditorState, EditorType, GlobalRequest, Request,
		ResourceOverviewData, ResourceOverviewEvent, ResourceOverviewRequest
	},
	ores::{parse_hashes_ores, parse_json_ores},
	resourcelib::{
		convert_generic, h2016_convert_binary_to_blueprint, h2016_convert_binary_to_factory,
		h2_convert_binary_to_blueprint, h2_convert_binary_to_factory, h3_convert_binary_to_blueprint,
		h3_convert_binary_to_factory
	},
	rpkg::{ensure_entity_in_cache, extract_latest_overview_info, extract_latest_resource},
	rpkg_tool::generate_rpkg_meta,
	send_notification, send_request, start_task,
	wwev::{parse_wwev, WwiseEventData},
	Notification, NotificationKind, RunCommandExt
};

#[try_fn]
#[context("Couldn't initialise resource overview {id}")]
pub fn initialise_resource_overview(
	app: &AppHandle,
	app_state: &State<AppState>,
	id: Uuid,
	hash: &String,
	game_files: &PartitionManager,
	game_version: GameVersion,
	resource_reverse_dependencies: &Arc<HashMap<String, Vec<String>>>,
	install: &PathBuf,
	hash_list: &Arc<HashList>
) -> Result<()> {
	let (filetype, chunk_patch, deps) = extract_latest_overview_info(game_files, hash)?;

	send_request(
		app,
		Request::Editor(EditorRequest::ResourceOverview(ResourceOverviewRequest::Initialise {
			id,
			hash: hash.to_owned(),
			filetype: filetype.to_owned(),
			chunk_patch,
			path_or_hint: hash_list
				.entries
				.get(hash)
				.and_then(|x| x.path.as_ref().or(x.hint.as_ref()).cloned()),
			dependencies: deps
				.par_iter()
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
						flag.to_owned(),
						resource_reverse_dependencies.contains_key(hash)
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
									.map(|x| x.resource_type.to_owned())
									.unwrap_or("".into()),
								hash_list
									.entries
									.get(hash)
									.and_then(|x| x.path.as_ref().or(x.hint.as_ref()).cloned())
							)
						})
						.collect()
				})
				.unwrap_or_default(),
			data: match filetype.as_ref() {
				"TEMP" => {
					ensure_entity_in_cache(
						game_files,
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

					let entity = app_state.cached_entities.get(hash).unwrap();

					ResourceOverviewData::Entity {
						blueprint_hash: entity.blueprint_hash.to_owned(),
						blueprint_path_or_hint: hash_list
							.entries
							.get(&entity.blueprint_hash)
							.and_then(|x| x.path.as_ref().or(x.hint.as_ref()).cloned())
					}
				}

				"AIRG" | "TBLU" | "ATMD" | "CPPT" | "VIDB" | "CBLU" | "CRMD" | "WSWB" | "DSWB" | "GFXF" | "GIDX"
				| "WSGB" | "ECPB" | "UICB" | "ENUM" => {
					let (res_meta, res_data) = extract_latest_resource(game_files, hash_list, hash)?;

					ResourceOverviewData::GenericRL {
						json: {
							let mut buf = Vec::new();
							let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
							let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

							convert_generic::<Value>(
								&res_data,
								game_version,
								if res_meta.hash_resource_type == "WSWB" {
									"DSWB"
								} else {
									&res_meta.hash_resource_type
								}
							)?
							.serialize(&mut ser)?;

							if buf.len() < 1024 * 512 {
								String::from_utf8(buf)?
							} else {
								"Too large to preview".into()
							}
						}
					}
				}

				"ORES" if hash == "0057C2C3941115CA" => ResourceOverviewData::Unlockables,

				"ORES" => ResourceOverviewData::Ores {
					json: {
						let (_, res_data) = extract_latest_resource(game_files, hash_list, hash)?;
						let res_data = parse_hashes_ores(&res_data)?;

						let mut buf = Vec::new();
						let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
						let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

						res_data.serialize(&mut ser)?;

						String::from_utf8(buf)?
					}
				},

				"GFXI" => {
					let data_dir = app.path_resolver().app_data_dir().expect("Couldn't get data dir");
					let temp_file_id = Uuid::new_v4();

					fs::create_dir_all(data_dir.join("temp"))?;

					let (_, res_data) = extract_latest_resource(game_files, hash_list, hash)?;

					ImageReader::new(Cursor::new(res_data))
						.with_guessed_format()?
						.decode()?
						.save(data_dir.join("temp").join(format!("{}.png", temp_file_id)))?;

					ResourceOverviewData::Image {
						image_path: data_dir.join("temp").join(format!("{}.png", temp_file_id)),
						dds_data: None
					}
				}

				"TEXT" => {
					let data_dir = app.path_resolver().app_data_dir().expect("Couldn't get data dir");
					let temp_file_id = Uuid::new_v4();

					fs::create_dir_all(data_dir.join("temp"))?;

					let (res_meta, res_data) = extract_latest_resource(game_files, hash_list, hash)?;

					let mut texture = TextureMap::process_data(
						match game_version {
							GameVersion::H1 => rpkg_rs::WoaVersion::HM2016,
							GameVersion::H2 => rpkg_rs::WoaVersion::HM2,
							GameVersion::H3 => rpkg_rs::WoaVersion::HM3
						},
						res_data
					)
					.context("Couldn't process texture data")?;

					if let Some(texd_depend) = res_meta.hash_reference_data.first() {
						let (_, texd_data) = extract_latest_resource(game_files, hash_list, &texd_depend.hash)?;

						texture
							.set_mipblock1_data(
								&texd_data,
								match game_version {
									GameVersion::H1 => tex_rs::WoaVersion::HM2016,
									GameVersion::H2 => tex_rs::WoaVersion::HM2,
									GameVersion::H3 => tex_rs::WoaVersion::HM3
								}
							)
							.context("Couldn't process TEXD data")?;
					}

					let tga_data = tex_rs::convert::create_tga(&texture).context("Couldn't convert texture to TGA")?;

					let mut reader = ImageReader::new(Cursor::new(tga_data.to_owned()));

					reader.set_format(image::ImageFormat::Tga);

					reader
						.decode()?
						.save(data_dir.join("temp").join(format!("{}.png", temp_file_id)))?;

					ResourceOverviewData::Image {
						image_path: data_dir.join("temp").join(format!("{}.png", temp_file_id)),
						dds_data: Some((
							match texture.get_header().type_ {
								tex_rs::texture_map::TextureType::Colour => "Colour",
								tex_rs::texture_map::TextureType::Normal => "Normal",
								tex_rs::texture_map::TextureType::Height => "Height",
								tex_rs::texture_map::TextureType::CompoundNormal => "Compound Normal",
								tex_rs::texture_map::TextureType::Billboard => "Billboard",
								tex_rs::texture_map::TextureType::Projection => "Projection",
								tex_rs::texture_map::TextureType::Emission => "Emission",
								tex_rs::texture_map::TextureType::UNKNOWN64 => "Unknown"
							}
							.into(),
							match texture.get_header().format {
								tex_rs::texture_map::RenderFormat::R16G16B16A16 => "R16G16B16A16",
								tex_rs::texture_map::RenderFormat::R8G8B8A8 => "R8G8B8A8",
								tex_rs::texture_map::RenderFormat::R8G8 => "R8G8",
								tex_rs::texture_map::RenderFormat::A8 => "A8",
								tex_rs::texture_map::RenderFormat::DXT1 => "DXT1",
								tex_rs::texture_map::RenderFormat::DXT3 => "DXT3",
								tex_rs::texture_map::RenderFormat::DXT5 => "DXT5",
								tex_rs::texture_map::RenderFormat::BC4 => "BC4",
								tex_rs::texture_map::RenderFormat::BC5 => "BC5",
								tex_rs::texture_map::RenderFormat::BC7 => "BC7"
							}
							.into()
						))
					}
				}

				"WWEV" => {
					let data_dir = app.path_resolver().app_data_dir().expect("Couldn't get data dir");

					fs::create_dir_all(data_dir.join("temp"))?;

					let (res_meta, res_data) = extract_latest_resource(game_files, hash_list, hash)?;

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

								let (_, wem_data) = extract_latest_resource(game_files, hash_list, wwem_hash)?;

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

					let (_, res_data) = extract_latest_resource(game_files, hash_list, hash)?;

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

				"JSON" => ResourceOverviewData::Json {
					json: format_json(&String::from_utf8(
						extract_latest_resource(game_files, hash_list, hash)?.1
					)?)?
				},

				"CLNG" => ResourceOverviewData::HMLanguages {
					json: {
						let (res_meta, res_data) = extract_latest_resource(game_files, hash_list, hash)?;

						let clng = {
							let mut iteration = 0;

							loop {
								if let Ok::<_, anyhow::Error>(x) = try {
									let langmap = get_language_map(game_version, iteration)
										.context("No more alternate language maps available")?;

									let clng = hmlanguages::clng::CLNG::new(
										match game_version {
											GameVersion::H1 => tonytools::Version::H2016,
											GameVersion::H2 => tonytools::Version::H2,
											GameVersion::H3 => tonytools::Version::H3
										},
										langmap.1.to_owned()
									)
									.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

									clng.convert(&res_data, to_string(&res_meta)?)
										.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
								} {
									break x;
								} else {
									iteration += 1;

									if get_language_map(game_version, iteration).is_none() {
										bail!("No more alternate language maps available");
									}
								}
							}
						};

						let mut buf = Vec::new();
						let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
						let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

						clng.serialize(&mut ser)?;

						String::from_utf8(buf)?
					}
				},

				"DITL" => ResourceOverviewData::HMLanguages {
					json: {
						let (res_meta, res_data) = extract_latest_resource(game_files, hash_list, hash)?;

						let ditl = hmlanguages::ditl::DITL::new(
							app_state
								.tonytools_hash_list
								.load()
								.as_ref()
								.context("No TonyTools hash list available")?
								.deref()
								.to_owned()
						)
						.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

						let mut buf = Vec::new();
						let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
						let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

						ditl.convert(&res_data, to_string(&res_meta)?)
							.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
							.serialize(&mut ser)?;

						String::from_utf8(buf)?
					}
				},

				"DLGE" => ResourceOverviewData::HMLanguages {
					json: {
						let (res_meta, res_data) = extract_latest_resource(game_files, hash_list, hash)?;

						let dlge = {
							let mut iteration = 0;

							loop {
								if let Ok::<_, anyhow::Error>(x) = try {
									let langmap = get_language_map(game_version, iteration)
										.context("No more alternate language maps available")?;

									let dlge = hmlanguages::dlge::DLGE::new(
										app_state
											.tonytools_hash_list
											.load()
											.as_ref()
											.context("No TonyTools hash list available")?
											.deref()
											.to_owned(),
										match game_version {
											GameVersion::H1 => tonytools::Version::H2016,
											GameVersion::H2 => tonytools::Version::H2,
											GameVersion::H3 => tonytools::Version::H3
										},
										langmap.1.to_owned(),
										None,
										false
									)
									.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

									dlge.convert(&res_data, to_string(&res_meta)?)
										.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
								} {
									break x;
								} else {
									iteration += 1;

									if get_language_map(game_version, iteration).is_none() {
										bail!("No more alternate language maps available");
									}
								}
							}
						};

						let mut buf = Vec::new();
						let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
						let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

						dlge.serialize(&mut ser)?;

						String::from_utf8(buf)?
					}
				},

				"LOCR" => ResourceOverviewData::HMLanguages {
					json: {
						let (res_meta, res_data) = extract_latest_resource(game_files, hash_list, hash)?;

						let locr = {
							let mut iteration = 0;

							loop {
								if let Ok::<_, anyhow::Error>(x) = try {
									let langmap = get_language_map(game_version, iteration)
										.context("No more alternate language maps available")?;

									let locr = hmlanguages::locr::LOCR::new(
										app_state
											.tonytools_hash_list
											.load()
											.as_ref()
											.context("No TonyTools hash list available")?
											.deref()
											.to_owned(),
										match game_version {
											GameVersion::H1 => tonytools::Version::H2016,
											GameVersion::H2 => tonytools::Version::H2,
											GameVersion::H3 => tonytools::Version::H3
										},
										langmap.1.to_owned(),
										langmap.0
									)
									.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

									locr.convert(&res_data, to_string(&res_meta)?)
										.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
								} {
									break x;
								} else {
									iteration += 1;

									if get_language_map(game_version, iteration).is_none() {
										bail!("No more alternate language maps available");
									}
								}
							}
						};

						let mut buf = Vec::new();
						let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
						let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

						locr.serialize(&mut ser)?;

						String::from_utf8(buf)?
					}
				},

				"RTLV" => ResourceOverviewData::HMLanguages {
					json: {
						let (res_meta, res_data) = extract_latest_resource(game_files, hash_list, hash)?;

						let rtlv = hmlanguages::rtlv::RTLV::new(
							match game_version {
								GameVersion::H1 => tonytools::Version::H2016,
								GameVersion::H2 => tonytools::Version::H2,
								GameVersion::H3 => tonytools::Version::H3
							},
							None
						)
						.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
						.convert(&res_data, to_string(&res_meta)?)
						.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

						let mut buf = Vec::new();
						let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
						let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

						rtlv.serialize(&mut ser)?;

						String::from_utf8(buf)?
					}
				},

				"LINE" => ResourceOverviewData::LocalisedLine {
					languages: {
						let (res_meta, res_data) = extract_latest_resource(game_files, hash_list, hash)?;

						let (locr_meta, locr_data) = extract_latest_resource(
							game_files,
							hash_list,
							&res_meta
								.hash_reference_data
								.first()
								.context("No LOCR dependency on LINE")?
								.hash
						)?;

						let locr = {
							let mut iteration = 0;

							loop {
								if let Ok::<_, anyhow::Error>(x) = try {
									let langmap = get_language_map(game_version, iteration)
										.context("No more alternate language maps available")?;

									let locr = hmlanguages::locr::LOCR::new(
										app_state
											.tonytools_hash_list
											.load()
											.as_ref()
											.context("No TonyTools hash list available")?
											.deref()
											.to_owned(),
										match game_version {
											GameVersion::H1 => tonytools::Version::H2016,
											GameVersion::H2 => tonytools::Version::H2,
											GameVersion::H3 => tonytools::Version::H3
										},
										langmap.1.to_owned(),
										langmap.0
									)
									.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

									locr.convert(&locr_data, to_string(&locr_meta)?)
										.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
								} {
									break x;
								} else {
									iteration += 1;

									if get_language_map(game_version, iteration).is_none() {
										bail!("No more alternate language maps available");
									}
								}
							}
						};

						let res_data: [u8; 5] = res_data.try_into().ok().context("Couldn't read LINE data as u32")?;

						let line_id = u32::from_le_bytes(res_data[0..4].try_into().unwrap());

						let line_hash = format!("{:0>8X}", line_id);

						let line_str = app_state
							.tonytools_hash_list
							.load()
							.as_ref()
							.context("No TonyTools hash list available")?
							.lines
							.get_by_left(&line_id)
							.cloned();

						if let Some(line_str) = line_str {
							locr.languages
								.into_iter()
								.filter_map(|(lang, keys)| {
									if let serde_json::Value::String(val) = keys.get(&line_str)? {
										Some((lang.to_owned(), val.to_owned()))
									} else {
										None
									}
								})
								.collect::<Vec<_>>()
						} else {
							locr.languages
								.into_iter()
								.filter_map(|(lang, keys)| {
									if let serde_json::Value::String(val) = keys.get(&line_hash)? {
										Some((lang.to_owned(), val.to_owned()))
									} else {
										None
									}
								})
								.collect::<Vec<_>>()
						}
					}
				},

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
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			let task = start_task(app, format!("Loading resource overview for {}", hash))?;

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(resource_reverse_dependencies) = app_state.resource_reverse_dependencies.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let game_version = app_state
					.game_installs
					.iter()
					.try_find(|x| anyhow::Ok(x.path == *install))?
					.context("No such game install")?
					.version;

				initialise_resource_overview(
					app,
					&app_state,
					id,
					hash,
					game_files,
					game_version,
					resource_reverse_dependencies,
					install,
					hash_list
				)?;
			}

			finish_task(app, task)?;
		}

		ResourceOverviewEvent::FollowDependency { id, new_hash } => {
			let mut editor_state = app_state.editor_states.get_mut(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref mut hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			*hash = new_hash.to_owned();

			let task = start_task(app, format!("Loading resource overview for {}", hash))?;

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(resource_reverse_dependencies) = app_state.resource_reverse_dependencies.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let game_version = app_state
					.game_installs
					.iter()
					.try_find(|x| anyhow::Ok(x.path == *install))?
					.context("No such game install")?
					.version;

				initialise_resource_overview(
					app,
					&app_state,
					id,
					hash,
					game_files,
					game_version,
					resource_reverse_dependencies,
					install,
					hash_list
				)?;

				send_request(
					app,
					Request::Global(GlobalRequest::RenameTab {
						id,
						new_name: format!("Resource overview ({new_hash})")
					})
				)?;
			}

			finish_task(app, task)?;
		}

		ResourceOverviewEvent::FollowDependencyInNewTab { hash, .. } => {
			let id = Uuid::new_v4();

			app_state.editor_states.insert(
				id.to_owned(),
				EditorState {
					file: None,
					data: EditorData::ResourceOverview { hash: hash.to_owned() }
				}
			);

			send_request(
				app,
				Request::Global(GlobalRequest::CreateTab {
					id,
					name: format!("Resource overview ({hash})"),
					editor_type: EditorType::ResourceOverview
				})
			)?;
		}

		ResourceOverviewEvent::OpenInEditor { id } => {
			let hash = {
				let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

				match editor_state.data {
					EditorData::ResourceOverview { ref hash, .. } => hash,

					_ => {
						Err(anyhow!("Editor {} is not a resource overview", id))?;
						panic!();
					}
				}
				.to_owned()
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				open_in_editor(app, game_files, install, hash_list, hash).await?;
			}
		}

		ResourceOverviewEvent::ExtractAsFile { id } => {
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let (metadata, data) = extract_latest_resource(game_files, hash_list, hash)?;
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
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				ensure_entity_in_cache(
					game_files,
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
					let entity = app_state.cached_entities.get(hash).unwrap();
					to_vec(&*entity)?
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
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let (metadata, data) = extract_latest_resource(game_files, hash_list, hash)?;
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
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let game_version = app_state
					.game_installs
					.iter()
					.try_find(|x| anyhow::Ok(x.path == *install))?
					.context("No such game install")?
					.version;

				ensure_entity_in_cache(game_files, &app_state.cached_entities, game_version, hash_list, hash)?;

				let (metadata, data) = extract_latest_resource(game_files, hash_list, &{
					let entity = app_state.cached_entities.get(hash).unwrap();
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
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let game_version = app_state
					.game_installs
					.iter()
					.try_find(|x| anyhow::Ok(x.path == *install))?
					.context("No such game install")?
					.version;

				ensure_entity_in_cache(game_files, &app_state.cached_entities, game_version, hash_list, hash)?;

				let (metadata, data) = extract_latest_resource(game_files, hash_list, &{
					let entity = app_state.cached_entities.get(hash).unwrap();
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
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let game_version = app_state
					.game_installs
					.iter()
					.try_find(|x| anyhow::Ok(x.path == *install))?
					.context("No such game install")?
					.version;

				let (res_meta, res_data) = extract_latest_resource(game_files, hash_list, hash)?;

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
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				if hash == "0057C2C3941115CA" {
					let (_, res_data) = extract_latest_resource(game_files, hash_list, hash)?;

					let mut dialog = AsyncFileDialog::new().set_title("Extract file");

					if let Some(project) = app_state.project.load().as_ref() {
						dialog = dialog.set_directory(&project.path);
					}

					let res_data = parse_json_ores(&res_data)?;

					if let Some(save_handle) = dialog
						.set_file_name(&format!("{}.json", hash))
						.add_filter("JSON file", &["json"])
						.save_file()
						.await
					{
						fs::write(save_handle.path(), to_vec(&res_data)?)?;
					}
				} else {
					let (_, res_data) = extract_latest_resource(game_files, hash_list, hash)?;

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
		}

		ResourceOverviewEvent::ExtractAsImage { id } => {
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
			{
				let game_version = app_state
					.game_installs
					.iter()
					.try_find(|x| anyhow::Ok(x.path == *install))?
					.context("No such game install")?
					.version;

				let (res_meta, res_data) = extract_latest_resource(game_files, hash_list, hash)?;

				let mut dialog = AsyncFileDialog::new().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(save_handle) = dialog
					.set_file_name(&format!("{}.png", hash))
					.add_filter("PNG file", &["png"])
					.add_filter("JPEG file", &["jpg"])
					.add_filter("TGA file", &["tga"])
					.add_filter("DDS file", &["dds"])
					.save_file()
					.await
				{
					app.track_event(
						"Extract image file as image format",
						Some(json!({
							"format": save_handle
									.path()
									.file_name()
									.context("No file name")?
									.to_str()
									.context("Filename was invalid string")?
									.split('.')
									.last()
									.unwrap_or("None")
						}))
					);

					match res_meta.hash_resource_type.as_str() {
						"GFXI" => {
							let reader = ImageReader::new(Cursor::new(res_data.to_owned())).with_guessed_format()?;

							if save_handle
								.path()
								.file_name()
								.context("No file name")?
								.to_str()
								.context("Filename was invalid string")?
								.ends_with(".dds")
							{
								match reader.format().context("Couldn't get format")? {
									ImageFormat::Dds => {
										fs::write(save_handle.path(), res_data)?;
									}

									_ => {
										send_notification(
											app,
											Notification {
												kind: NotificationKind::Error,
												title: "DDS encoding not supported".into(),
												subtitle: "The image is not natively in DDS format and cannot be \
												           re-encoded as DDS. Please choose another format."
													.into()
											}
										)?;
									}
								}
							} else {
								reader.decode()?.save(save_handle.path())?;
							}
						}

						"TEXT" => {
							let mut texture = TextureMap::process_data(
								match game_version {
									GameVersion::H1 => rpkg_rs::WoaVersion::HM2016,
									GameVersion::H2 => rpkg_rs::WoaVersion::HM2,
									GameVersion::H3 => rpkg_rs::WoaVersion::HM3
								},
								res_data
							)
							.context("Couldn't process texture data")?;

							if let Some(texd_depend) = res_meta.hash_reference_data.first() {
								let (_, texd_data) = extract_latest_resource(game_files, hash_list, &texd_depend.hash)?;

								texture
									.set_mipblock1_data(
										&texd_data,
										match game_version {
											GameVersion::H1 => tex_rs::WoaVersion::HM2016,
											GameVersion::H2 => tex_rs::WoaVersion::HM2,
											GameVersion::H3 => tex_rs::WoaVersion::HM3
										}
									)
									.context("Couldn't process TEXD data")?;
							}

							if save_handle
								.path()
								.file_name()
								.context("No file name")?
								.to_str()
								.context("Filename was invalid string")?
								.ends_with(".dds")
							{
								let dds_data =
									tex_rs::convert::create_dds(&texture).context("Couldn't convert texture to DDS")?;

								fs::write(save_handle.path(), dds_data)?;
							} else {
								let tga_data =
									tex_rs::convert::create_tga(&texture).context("Couldn't convert texture to TGA")?;

								let mut reader = ImageReader::new(Cursor::new(tga_data.to_owned()));

								reader.set_format(image::ImageFormat::Tga);

								if save_handle
									.path()
									.file_name()
									.context("No file name")?
									.to_str()
									.context("Filename was invalid string")?
									.ends_with(".tga")
								{
									fs::write(save_handle.path(), tga_data)?;
								} else {
									reader.decode()?.save(save_handle.path())?;
								}
							}
						}

						_ => bail!("Unsupported resource type")
					}
				}
			}
		}

		ResourceOverviewEvent::ExtractAsWav { id } => {
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
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
					let (_, res_data) = extract_latest_resource(game_files, hash_list, hash)?;

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
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let mut dialog = AsyncFileDialog::new().set_title("Extract all WAVs to folder");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(save_handle) = dialog.pick_folder().await {
					let data_dir = app.path_resolver().app_data_dir().expect("Couldn't get data dir");

					let (res_meta, res_data) = extract_latest_resource(game_files, hash_list, hash)?;

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

								let (_, wem_data) = extract_latest_resource(game_files, hash_list, wwem_hash)?;

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
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
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

					let (res_meta, res_data) = extract_latest_resource(game_files, hash_list, hash)?;

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

							let (_, wem_data) = extract_latest_resource(game_files, hash_list, wwem_hash)?;

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

		ResourceOverviewEvent::ExtractAsHMLanguages { id } => {
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { ref hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let game_version = app_state
					.game_installs
					.iter()
					.try_find(|x| anyhow::Ok(x.path == *install))?
					.context("No such game install")?
					.version;

				let (res_meta, res_data) = extract_latest_resource(game_files, hash_list, hash)?;

				let mut dialog = AsyncFileDialog::new().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(save_handle) = dialog
					.set_file_name(&format!("{}.{}.json", hash, res_meta.hash_resource_type.to_lowercase()))
					.add_filter(
						&format!("{}.json file", res_meta.hash_resource_type.to_lowercase()),
						&[&format!("{}.json", res_meta.hash_resource_type.to_lowercase())]
					)
					.save_file()
					.await
				{
					fs::write(
						save_handle.path(),
						match res_meta.hash_resource_type.as_ref() {
							"CLNG" => {
								let clng = {
									let mut iteration = 0;

									loop {
										if let Ok::<_, anyhow::Error>(x) = try {
											let langmap = get_language_map(game_version, iteration)
												.context("No more alternate language maps available")?;

											let clng = hmlanguages::clng::CLNG::new(
												match game_version {
													GameVersion::H1 => tonytools::Version::H2016,
													GameVersion::H2 => tonytools::Version::H2,
													GameVersion::H3 => tonytools::Version::H3
												},
												langmap.1.to_owned()
											)
											.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

											clng.convert(&res_data, to_string(&res_meta)?)
												.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
										} {
											break x;
										} else {
											iteration += 1;

											if get_language_map(game_version, iteration).is_none() {
												bail!("No more alternate language maps available");
											}
										}
									}
								};

								let mut buf = Vec::new();
								let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
								let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

								clng.serialize(&mut ser)?;

								buf
							}

							"DITL" => {
								let ditl = hmlanguages::ditl::DITL::new(
									app_state
										.tonytools_hash_list
										.load()
										.as_ref()
										.context("No TonyTools hash list available")?
										.deref()
										.to_owned()
								)
								.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

								let mut buf = Vec::new();
								let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
								let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

								ditl.convert(&res_data, to_string(&res_meta)?)
									.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
									.serialize(&mut ser)?;

								buf
							}

							"DLGE" => {
								let dlge = {
									let mut iteration = 0;

									loop {
										if let Ok::<_, anyhow::Error>(x) = try {
											let langmap = get_language_map(game_version, iteration)
												.context("No more alternate language maps available")?;

											let dlge = hmlanguages::dlge::DLGE::new(
												app_state
													.tonytools_hash_list
													.load()
													.as_ref()
													.context("No TonyTools hash list available")?
													.deref()
													.to_owned(),
												match game_version {
													GameVersion::H1 => tonytools::Version::H2016,
													GameVersion::H2 => tonytools::Version::H2,
													GameVersion::H3 => tonytools::Version::H3
												},
												langmap.1.to_owned(),
												None,
												false
											)
											.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

											dlge.convert(&res_data, to_string(&res_meta)?)
												.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
										} {
											break x;
										} else {
											iteration += 1;

											if get_language_map(game_version, iteration).is_none() {
												bail!("No more alternate language maps available");
											}
										}
									}
								};

								let mut buf = Vec::new();
								let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
								let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

								dlge.serialize(&mut ser)?;

								buf
							}

							"LOCR" => {
								let locr = {
									let mut iteration = 0;

									loop {
										if let Ok::<_, anyhow::Error>(x) = try {
											let langmap = get_language_map(game_version, iteration)
												.context("No more alternate language maps available")?;

											let locr = hmlanguages::locr::LOCR::new(
												app_state
													.tonytools_hash_list
													.load()
													.as_ref()
													.context("No TonyTools hash list available")?
													.deref()
													.to_owned(),
												match game_version {
													GameVersion::H1 => tonytools::Version::H2016,
													GameVersion::H2 => tonytools::Version::H2,
													GameVersion::H3 => tonytools::Version::H3
												},
												langmap.1.to_owned(),
												langmap.0
											)
											.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

											locr.convert(&res_data, to_string(&res_meta)?)
												.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
										} {
											break x;
										} else {
											iteration += 1;

											if get_language_map(game_version, iteration).is_none() {
												bail!("No more alternate language maps available");
											}
										}
									}
								};

								let mut buf = Vec::new();
								let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
								let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

								locr.serialize(&mut ser)?;

								buf
							}

							"RTLV" => {
								let rtlv = hmlanguages::rtlv::RTLV::new(
									match game_version {
										GameVersion::H1 => tonytools::Version::H2016,
										GameVersion::H2 => tonytools::Version::H2,
										GameVersion::H3 => tonytools::Version::H3
									},
									None
								)
								.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
								.convert(&res_data, to_string(&res_meta)?)
								.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

								let mut buf = Vec::new();
								let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
								let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

								rtlv.serialize(&mut ser)?;

								buf
							}

							_ => bail!("Not a valid HMLanguages resource type")
						}
					)?;
				}
			}
		}
	}
}
