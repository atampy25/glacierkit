use std::{fmt::Write, fs, io::Cursor, ops::Deref, sync::Arc};

use anyhow::{Context, Result, anyhow, bail};
use arc_swap::ArcSwap;
use fn_error_context::context;
use glacier_texture::{
	enums::{RenderFormat, TextureType},
	mipblock::MipblockData,
	texture_map::TextureMap
};

use hashbrown::HashMap;
use hitman_commons::{game::GameVersion, hash_list::HashList, metadata::RuntimeID, rpkg_tool::RpkgResourceMeta};
use hitman_formats::{
	material::{MaterialEntity, MaterialInstance},
	ores::{parse_hashes_ores, parse_json_ores},
	sdef::SoundDefinitions,
	wwev::WwiseEvent
};
use image::{ImageFormat, ImageReader};
use prim_rs::render_primitive::RenderPrimitive;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use rpkg_rs::{GlacierResource, resource::partition_manager::PartitionManager};
use serde::Serialize;
use serde_json::{Value, json, to_string, to_vec};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_aptabase::EventTracker;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_shell::ShellExt;
use tonytools::hmlanguages;
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	Notification, NotificationKind, RunCommandExt,
	biome::format_json,
	finish_task,
	general::open_in_editor,
	get_loaded_game_version,
	languages::get_language_map,
	model::{
		AppSettings, AppState, EditorData, EditorRequest, EditorState, EditorType, GlobalRequest, Request,
		ResourceOverviewData, ResourceOverviewEvent, ResourceOverviewRequest
	},
	resourcelib::{
		convert_generic, h2_convert_binary_to_blueprint, h2_convert_binary_to_factory, h3_convert_binary_to_blueprint,
		h3_convert_binary_to_factory, h2016_convert_binary_to_blueprint, h2016_convert_binary_to_factory
	},
	rpkg::{extract_entity, extract_latest_overview_info, extract_latest_resource, extract_resource_changelog},
	send_notification, send_request, start_task
};

#[try_fn]
#[context("Couldn't initialise resource overview {id}")]
pub async fn initialise_resource_overview(
	app: &AppHandle,
	app_state: &State<'_, AppState>,
	id: Uuid,
	hash: RuntimeID,
	game_files: &PartitionManager,
	game_version: GameVersion,
	resource_reverse_dependencies: &Arc<HashMap<RuntimeID, Vec<RuntimeID>>>,
	hash_list: &Arc<HashList>
) -> Result<()> {
	let (filetype, chunk_patch, deps) = extract_latest_overview_info(game_files, hash)?;

	send_request(
		app,
		Request::Editor(EditorRequest::ResourceOverview(ResourceOverviewRequest::Initialise {
			id,
			hash: hash.to_string(),
			filetype: filetype.into(),
			chunk_patch,
			path_or_hint: hash_list
				.entries
				.get(&hash)
				.and_then(|x| x.path.as_ref().or(x.hint.as_ref()).cloned()),
			dependencies: deps
				.par_iter()
				.map(|(hash, flag)| {
					(
						hash.to_string(),
						hash_list
							.entries
							.get(hash)
							.map(|x| x.resource_type.into())
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
				.get(&hash)
				.map(|hashes| {
					hashes
						.iter()
						.map(|hash| {
							(
								hash.to_string(),
								hash_list
									.entries
									.get(hash)
									.map(|x| x.resource_type.into())
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
			changelog: extract_resource_changelog(game_files, hash),
			data: match filetype.as_ref() {
				"TEMP" => {
					let entity = extract_entity(game_files, &app_state.cached_entities, game_version, hash_list, hash)?;

					ResourceOverviewData::Entity {
						blueprint_hash: entity.blueprint_hash.to_owned(),
						blueprint_path_or_hint: hash_list
							.entries
							.get(&RuntimeID::from_any(&entity.blueprint_hash)?)
							.and_then(|x| x.path.as_ref().or(x.hint.as_ref()).cloned())
					}
				}

				"AIRG" | "TBLU" | "ATMD" | "CPPT" | "VIDB" | "CBLU" | "CRMD" | "WSWB" | "DSWB" | "GFXF" | "GIDX"
				| "WSGB" | "ECPB" | "UICB" | "ENUM" => {
					let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

					ResourceOverviewData::GenericRL {
						json: {
							let mut buf = Vec::new();
							let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
							let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

							convert_generic::<Value>(
								&res_data,
								game_version,
								if res_meta.core_info.resource_type == "WSWB" {
									"DSWB".try_into()?
								} else {
									res_meta.core_info.resource_type
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

				"ORES" if hash == "0057C2C3941115CA".parse()? => ResourceOverviewData::Unlockables,

				"ORES" => ResourceOverviewData::Ores {
					json: {
						let (_, res_data) = extract_latest_resource(game_files, hash)?;
						let res_data = parse_hashes_ores(&res_data)?;

						let mut buf = Vec::new();
						let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
						let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

						res_data.serialize(&mut ser)?;

						String::from_utf8(buf)?
					}
				},

				"GFXI" => {
					let data_dir = app.path().app_data_dir().expect("Couldn't get data dir");
					let temp_file_id = Uuid::new_v4();

					fs::create_dir_all(data_dir.join("temp"))?;

					let (_, res_data) = extract_latest_resource(game_files, hash)?;

					ImageReader::new(Cursor::new(res_data))
						.with_guessed_format()?
						.decode()?
						.save(data_dir.join("temp").join(format!("{}.png", temp_file_id)))?;

					ResourceOverviewData::Image {
						preview_image_path: data_dir.join("temp").join(format!("{}.png", temp_file_id)),
						dds_data: None
					}
				}

				"PRIM" => {
					let (_, res_data) = extract_latest_resource(game_files, hash)?;

					let model = RenderPrimitive::process_data(game_version.into(), res_data)
						.context("Couldn't process texture data")?;

					// Higher is less detail
					let preferred_lod = 1;

					// Get only the meshes, we don't need weight metadata for the preview
					let meshes = model
						.data
						.objects
						.iter()
						.map(|mesh_obj| match mesh_obj {
							prim_rs::render_primitive::MeshObject::Normal(mesh) => mesh,
							prim_rs::render_primitive::MeshObject::Weighted(mesh) => &mesh.prim_mesh,
							prim_rs::render_primitive::MeshObject::Linked(mesh) => &mesh.prim_mesh
						})
						.collect::<Vec<_>>();

					// Get only the meshes for the preferred LOD level
					let meshes = meshes
						.iter()
						.filter(|mesh| mesh.prim_object.lod_mask & (1 << preferred_lod) == (1 << preferred_lod));

					let mut previous_vertex_count: usize = 1;
					let mut bounding_box: [f32; 6] = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0];

					let mut obj = String::new();

					for (idx, mesh) in meshes.enumerate() {
						writeln!(obj, "o object.00{}", idx)?;

						for position in &mesh.sub_mesh.buffers.position {
							writeln!(obj, "v {} {} {}", position.x, position.y, position.z)?;
						}

						for vm in &mesh.sub_mesh.buffers.main {
							writeln!(obj, "vn {} {} {}", vm.normal.x, vm.normal.y, vm.normal.z)?;
						}

						for idx in mesh.sub_mesh.indices.chunks(3) {
							let [idx1, idx2, idx3] = [
								idx[0] as usize + previous_vertex_count,
								idx[1] as usize + previous_vertex_count,
								idx[2] as usize + previous_vertex_count
							];
							writeln!(obj, "f {}//{} {}//{} {}//{}", idx1, idx1, idx2, idx2, idx3, idx3)?;
						}

						previous_vertex_count += mesh.sub_mesh.buffers.position.len();

						let bb = mesh.sub_mesh.calc_bb();

						bounding_box[0] = bounding_box[0].min(bb.min.x);
						bounding_box[1] = bounding_box[1].min(bb.min.y);
						bounding_box[2] = bounding_box[2].min(bb.min.z);

						bounding_box[3] = bounding_box[3].max(bb.max.x);
						bounding_box[4] = bounding_box[4].max(bb.max.y);
						bounding_box[5] = bounding_box[5].max(bb.max.z);
					}

					ResourceOverviewData::Mesh { obj, bounding_box }
				}

				"TEXT" => {
					let data_dir = app.path().app_data_dir().expect("Couldn't get data dir");
					let temp_file_id = Uuid::new_v4();

					fs::create_dir_all(data_dir.join("temp"))?;

					let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

					let mut texture = TextureMap::process_data(game_version.into(), res_data)
						.context("Couldn't process texture data")?;

					if let Some(texd_depend) = res_meta.core_info.references.first() {
						let (_, texd_data) = extract_latest_resource(game_files, texd_depend.resource.get_id())?;
						let mipblock = MipblockData::from_memory(&texd_data, game_version.into())
							.context("Couldn't process TEXD data")?;
						texture.set_mipblock1(mipblock);
					}

					let image =
						glacier_texture::convert::create_dynamic_image(&texture).context("Couldn't convert texture")?;
					image.save(data_dir.join("temp").join(format!("{}.png", temp_file_id)))?;

					ResourceOverviewData::Image {
						preview_image_path: data_dir.join("temp").join(format!("{}.png", temp_file_id)),
						dds_data: Some((
							match texture.texture_type() {
								TextureType::Colour => "Colour",
								TextureType::Normal => "Normal",
								TextureType::Height => "Height",
								TextureType::CompoundNormal => "Compound Normal",
								TextureType::Billboard => "Billboard",
								TextureType::Projection => "Projection",
								TextureType::Emission => "Emission",
								TextureType::Cubemap => "Cubemap",
								TextureType::UNKNOWN512 => "unknown"
							}
							.into(),
							match texture.format() {
								RenderFormat::R16G16B16A16 => "R16G16B16A16",
								RenderFormat::R8G8B8A8 => "R8G8B8A8",
								RenderFormat::R8G8 => "R8G8",
								RenderFormat::A8 => "A8",
								RenderFormat::BC1 => "BC1",
								RenderFormat::BC2 => "BC2",
								RenderFormat::BC3 => "BC3",
								RenderFormat::BC4 => "BC4",
								RenderFormat::BC5 => "BC5",
								RenderFormat::BC7 => "BC7"
							}
							.into()
						))
					}
				}

				"WWEV" => {
					let data_dir = app.path().app_data_dir().expect("Couldn't get data dir");

					fs::create_dir_all(data_dir.join("temp"))?;

					let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

					let mut wav_paths = vec![];

					let wwev = WwiseEvent::parse(&res_data)?;

					for object in wwev.non_streamed {
						let temp_file_id = Uuid::new_v4();

						fs::write(data_dir.join("temp").join(format!("{}.wem", temp_file_id)), object.data)?;

						app.shell()
							.sidecar("vgmstream-cli")
							.unwrap()
							.current_dir(data_dir.join("temp"))
							.args([
								&format!("{}.wem", temp_file_id),
								"-L",
								"-o",
								&format!("{}.wav", temp_file_id)
							])
							.run()
							.await
							.with_context(|| format!("Couldn't convert non-streamed object {}", object.wem_id))?;

						wav_paths.push((
							"Embedded audio".into(),
							data_dir.join("temp").join(format!("{}.wav", temp_file_id))
						))
					}

					for object in wwev.streamed {
						let temp_file_id = Uuid::new_v4();

						let wwem_hash = res_meta
							.core_info
							.references
							.get(object.dependency_index as usize)
							.context("No such WWEM dependency")?
							.resource
							.get_id();

						let (_, wem_data) = extract_latest_resource(game_files, wwem_hash)?;

						fs::write(data_dir.join("temp").join(format!("{}.wem", temp_file_id)), wem_data)?;

						app.shell()
							.sidecar("vgmstream-cli")
							.unwrap()
							.current_dir(data_dir.join("temp"))
							.args([
								&format!("{}.wem", temp_file_id),
								"-L",
								"-o",
								&format!("{}.wav", temp_file_id)
							])
							.run()
							.await
							.with_context(|| format!("Couldn't convert streamed object {wwem_hash}"))?;

						wav_paths.push((
							wwem_hash.to_string(),
							data_dir.join("temp").join(format!("{}.wav", temp_file_id))
						))
					}

					ResourceOverviewData::MultiAudio {
						name: wwev.name,
						wav_paths
					}
				}

				"WWES" | "WWEM" => {
					let data_dir = app.path().app_data_dir().expect("Couldn't get data dir");
					let temp_file_id = Uuid::new_v4();

					fs::create_dir_all(data_dir.join("temp"))?;

					let (_, res_data) = extract_latest_resource(game_files, hash)?;

					fs::write(data_dir.join("temp").join(format!("{}.wem", temp_file_id)), res_data)?;

					app.shell()
						.sidecar("vgmstream-cli")
						.unwrap()
						.current_dir(data_dir.join("temp"))
						.args([
							&format!("{}.wem", temp_file_id),
							"-L",
							"-o",
							&format!("{}.wav", temp_file_id)
						])
						.run()
						.await
						.context("VGMStream command failed")?;

					ResourceOverviewData::Audio {
						wav_path: data_dir.join("temp").join(format!("{}.wav", temp_file_id))
					}
				}

				"REPO" => ResourceOverviewData::Repository,

				"JSON" => ResourceOverviewData::Json {
					json: format_json(&String::from_utf8(extract_latest_resource(game_files, hash)?.1)?)?
				},

				"CLNG" => ResourceOverviewData::HMLanguages {
					json: {
						let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

						let clng = {
							let mut iteration = 0;

							loop {
								if let Ok::<_, anyhow::Error>(x) = try {
									let langmap = get_language_map(game_version, iteration)
										.context("No more alternate language maps available")?;

									let clng = hmlanguages::clng::CLNG::new(game_version.into(), langmap.1.to_owned())
										.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

									clng.convert(
										&res_data,
										to_string(
											&RpkgResourceMeta::from_resource_metadata(res_meta.to_owned(), false)
												.with_hash_list(&hash_list.entries)?
										)?
									)
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
						let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

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

						ditl.convert(
							&res_data,
							to_string(
								&RpkgResourceMeta::from_resource_metadata(res_meta, false)
									.with_hash_list(&hash_list.entries)?
							)?
						)
						.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
						.serialize(&mut ser)?;

						String::from_utf8(buf)?
					}
				},

				"DLGE" => ResourceOverviewData::HMLanguages {
					json: {
						let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

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
										game_version.into(),
										langmap.1.to_owned(),
										None,
										false
									)
									.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

									dlge.convert(
										&res_data,
										to_string(
											&RpkgResourceMeta::from_resource_metadata(res_meta.to_owned(), false)
												.with_hash_list(&hash_list.entries)?
										)?
									)
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
						let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

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
										game_version.into(),
										langmap.1.to_owned(),
										langmap.0
									)
									.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

									locr.convert(
										&res_data,
										to_string(
											&RpkgResourceMeta::from_resource_metadata(res_meta.to_owned(), false)
												.with_hash_list(&hash_list.entries)?
										)?
									)
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
						let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

						let rtlv = hmlanguages::rtlv::RTLV::new(game_version.into(), None)
							.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
							.convert(
								&res_data,
								to_string(
									&RpkgResourceMeta::from_resource_metadata(res_meta, false)
										.with_hash_list(&hash_list.entries)?
								)?
							)
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
						let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

						let (locr_meta, locr_data) = extract_latest_resource(
							game_files,
							res_meta
								.core_info
								.references
								.first()
								.context("No LOCR dependency on LINE")?
								.resource
								.get_id()
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
										game_version.into(),
										langmap.1.to_owned(),
										langmap.0
									)
									.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

									locr.convert(
										&locr_data,
										to_string(&RpkgResourceMeta::from_resource_metadata(
											locr_meta.to_owned(),
											false
										))?
									)
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

				"MATI" => ResourceOverviewData::MaterialInstance {
					json: {
						let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

						let material =
							MaterialInstance::parse(&res_data, &res_meta.core_info.with_hash_list(&hash_list.entries))
								.context("Couldn't parse material instance")?;

						let mut buf = Vec::new();
						let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
						let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

						material.serialize(&mut ser)?;

						String::from_utf8(buf)?
					}
				},

				"MATT" => ResourceOverviewData::MaterialEntity {
					json: {
						let (matt_meta, matt_data) = extract_latest_resource(game_files, hash)?;
						let (matb_meta, matb_data) = extract_latest_resource(
							game_files,
							matt_meta
								.core_info
								.references
								.get(1)
								.context("No MATB dependency")?
								.resource
								.get_id()
						)?;

						let material = MaterialEntity::parse(
							&matt_data,
							&matt_meta.core_info,
							&matb_data,
							&matb_meta.core_info.with_hash_list(&hash_list.entries)
						)
						.context("Couldn't parse material entity")?;

						let mut buf = Vec::new();
						let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
						let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

						material.serialize(&mut ser)?;

						String::from_utf8(buf)?
					}
				},

				"SDEF" => ResourceOverviewData::SoundDefinitions {
					json: {
						let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

						let sdef = SoundDefinitions::parse(
							&res_data,
							&res_meta.core_info.with_hash_list(&hash_list.entries),
							game_version
						)
						.context("Couldn't parse sound definitions")?;

						let mut buf = Vec::new();
						let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
						let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

						sdef.serialize(&mut ser)?;

						String::from_utf8(buf)?
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
				EditorData::ResourceOverview { hash, .. } => hash,

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
				initialise_resource_overview(
					app,
					&app_state,
					id,
					hash,
					game_files,
					get_loaded_game_version(app, install)?,
					resource_reverse_dependencies,
					hash_list
				)
				.await?;
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

			*hash = RuntimeID::from_any(&new_hash)?;

			let task = start_task(app, format!("Loading resource overview for {}", hash))?;

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(resource_reverse_dependencies) = app_state.resource_reverse_dependencies.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				initialise_resource_overview(
					app,
					&app_state,
					id,
					*hash,
					game_files,
					get_loaded_game_version(app, install)?,
					resource_reverse_dependencies,
					hash_list
				)
				.await?;

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
					data: EditorData::ResourceOverview {
						hash: RuntimeID::from_any(&hash)?
					}
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
					EditorData::ResourceOverview { hash, .. } => hash,

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
				EditorData::ResourceOverview { hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let (metadata, data) = extract_latest_resource(game_files, hash)?;
				let metadata_file = RpkgResourceMeta::from_resource_metadata(metadata, false)
					.to_binary()
					.context("Couldn't serialise meta file")?;

				let file_type = hash_list
					.entries
					.get(&hash)
					.expect("Can only open files from the hash list")
					.resource_type
					.to_owned();

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(&format!("{}.{}", &hash, &file_type))
					.add_filter(format!("{} file", &file_type), &[file_type.as_ref()])
					.blocking_save_file()
				{
					fs::write(path.as_path().context("Invalid path")?, data)?;

					fs::write(
						path.as_path()
							.context("Invalid path")?
							.parent()
							.unwrap()
							.join(format!("{}.{}.meta", hash, file_type)),
						metadata_file
					)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractAsQN { id } => {
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let entity_json = to_vec(&*extract_entity(
					game_files,
					&app_state.cached_entities,
					get_loaded_game_version(app, install)?,
					hash_list,
					hash
				)?)?;

				let mut dialog = app.dialog().file().set_title("Extract entity");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.add_filter("QuickEntity entity", &["entity.json"])
					.blocking_save_file()
				{
					fs::write(path.as_path().context("Invalid path")?, entity_json)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractTEMPAsRT { id } => {
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
			{
				let (metadata, data) = extract_latest_resource(game_files, hash)?;
				let metadata_file = RpkgResourceMeta::from_resource_metadata(metadata, false);

				let data = match get_loaded_game_version(app, install)? {
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

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(&format!("{}.TEMP.json", hash))
					.add_filter("TEMP.json file", &["TEMP.json"])
					.blocking_save_file()
				{
					fs::write(path.as_path().context("Invalid path")?, data)?;

					fs::write(
						path.as_path()
							.context("Invalid path")?
							.join(format!("{}.{}.meta.json", hash, metadata_file.hash_resource_type)),
						to_string(&metadata_file).context("Couldn't serialise meta file")?
					)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractTBLUAsFile { id } => {
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let (metadata, data) = extract_latest_resource(
					game_files,
					RuntimeID::from_any(
						&extract_entity(
							game_files,
							&app_state.cached_entities,
							get_loaded_game_version(app, install)?,
							hash_list,
							hash
						)?
						.blueprint_hash
					)?
				)?;

				let metadata_file = RpkgResourceMeta::from_resource_metadata(metadata.to_owned(), false)
					.to_binary()
					.context("Couldn't serialise meta file")?;

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(&format!("{}.TBLU", metadata.core_info.id))
					.add_filter("TBLU file", &["TBLU"])
					.blocking_save_file()
				{
					fs::write(path.as_path().context("Invalid path")?, data)?;

					fs::write(
						path.as_path()
							.context("Invalid path")?
							.parent()
							.unwrap()
							.join(format!("{}.{}.meta", hash, metadata.core_info.resource_type)),
						metadata_file
					)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractTBLUAsRT { id } => {
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let game_version = get_loaded_game_version(app, install)?;

				let (metadata, data) = extract_latest_resource(
					game_files,
					RuntimeID::from_any(
						&extract_entity(
							game_files,
							&app_state.cached_entities,
							get_loaded_game_version(app, install)?,
							hash_list,
							hash
						)?
						.blueprint_hash
					)?
				)?;

				let metadata_file = RpkgResourceMeta::from_resource_metadata(metadata.to_owned(), false);

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

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(&format!("{}.TBLU.json", metadata.core_info.id))
					.add_filter("TBLU.json file", &["TBLU.json"])
					.blocking_save_file()
				{
					fs::write(path.as_path().context("Invalid path")?, data)?;

					fs::write(
						path.as_path()
							.context("Invalid path")?
							.parent()
							.unwrap()
							.join(format!("{}.{}.meta.json", hash, metadata_file.hash_resource_type)),
						to_string(&metadata_file).context("Couldn't serialise meta file")?
					)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractAsRTGeneric { id } => {
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
			{
				let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(&format!("{}.{}.json", hash, res_meta.core_info.resource_type))
					.add_filter(
						format!("{}.json file", res_meta.core_info.resource_type),
						&[&format!("{}.json", res_meta.core_info.resource_type)]
					)
					.blocking_save_file()
				{
					fs::write(
						path.as_path().context("Invalid path")?,
						to_vec(&convert_generic::<Value>(
							&res_data,
							get_loaded_game_version(app, install)?,
							res_meta.core_info.resource_type
						)?)?
					)?;

					fs::write(
						path.as_path()
							.context("Invalid path")?
							.parent()
							.unwrap()
							.join(format!("{}.{}.meta.json", hash, res_meta.core_info.resource_type)),
						to_string(&RpkgResourceMeta::from_resource_metadata(res_meta, false))
							.context("Couldn't serialise meta file")?
					)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractORESAsJson { id } => {
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref() {
				if hash == "0057C2C3941115CA".parse()? {
					let (_, res_data) = extract_latest_resource(game_files, hash)?;

					let mut dialog = app.dialog().file().set_title("Extract file");

					if let Some(project) = app_state.project.load().as_ref() {
						dialog = dialog.set_directory(&project.path);
					}

					let res_data = parse_json_ores(&res_data)?;

					if let Some(path) = dialog
						.set_file_name(&format!("{}.json", hash))
						.add_filter("JSON file", &["json"])
						.blocking_save_file()
					{
						fs::write(path.as_path().context("Invalid path")?, res_data)?;
					}
				} else {
					let (_, res_data) = extract_latest_resource(game_files, hash)?;

					let mut dialog = app.dialog().file().set_title("Extract file");

					if let Some(project) = app_state.project.load().as_ref() {
						dialog = dialog.set_directory(&project.path);
					}

					let res_data = parse_hashes_ores(&res_data)?;

					if let Some(path) = dialog
						.set_file_name(&format!("{}.json", hash))
						.add_filter("JSON file", &["json"])
						.blocking_save_file()
					{
						fs::write(path.as_path().context("Invalid path")?, to_vec(&res_data)?)?;
					}
				}
			}
		}

		ResourceOverviewEvent::ExtractAsImage { id } => {
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
			{
				let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(&format!("{}.png", hash))
					.add_filter("PNG file", &["png"])
					.add_filter("JPEG file", &["jpg"])
					.add_filter("TGA file", &["tga"])
					.add_filter("DDS file", &["dds"])
					.blocking_save_file()
				{
					app.track_event(
						"Extract image file as image format",
						Some(json!({
							"format": path.as_path().context("Invalid path")?
									.file_name()
									.context("No file name")?
									.to_str()
									.context("Filename was invalid string")?
									.split('.')
									.next_back()
									.unwrap_or("None")
						}))
					)
					.unwrap();

					match res_meta.core_info.resource_type.as_ref() {
						"GFXI" => {
							let reader = ImageReader::new(Cursor::new(res_data.to_owned())).with_guessed_format()?;

							if path
								.as_path()
								.context("Invalid path")?
								.file_name()
								.context("No file name")?
								.to_str()
								.context("Filename was invalid string")?
								.ends_with(".dds")
							{
								match reader.format().context("Couldn't get format")? {
									ImageFormat::Dds => {
										fs::write(path.as_path().context("Invalid path")?, res_data)?;
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
								reader.decode()?.save(path.as_path().context("Invalid path")?)?;
							}
						}

						"TEXT" => {
							let game_version = get_loaded_game_version(app, install)
								.context("Couldn't get loaded game version")?;

							let path_ref = path.as_path().context("Invalid path")?;

							let mut meta = json!({
								"$schema": "https://tonytools.win/schemas/texture-meta.schema.json",
								"text":  app_state.hash_list.load().as_ref().map(|list| list.entries.get(&hash)
								.and_then(|entry| entry.path.to_owned()))
								.flatten()
								.unwrap_or(hash.to_string())
							});

							let mut texture =
								TextureMap::process_data(game_version.into(), res_data)
									.context("Couldn't process texture data")?;

							if let Some(texd_depend) = res_meta.core_info.references.first() {
								let (_, texd_data) =
									extract_latest_resource(game_files, texd_depend.resource.get_id())?;

								let mip_block = MipblockData::from_memory(&texd_data, game_version.into())
									.context("Couldn't process TEXD data")?;
								texture.set_mipblock1(mip_block);

								let texd_path = texd_depend.resource.get_path()
									.map(|s| s.to_owned())
									.or_else(|| {
										app_state.hash_list.load().as_ref()
											.and_then(|list| list.entries.get(&texd_depend.resource.get_id()))
											.and_then(|entry| entry.path.to_owned())
									})
									.unwrap_or_else(|| texd_depend.resource.to_string());

								meta.as_object_mut()
									.unwrap()
									.insert("texd".to_owned(), Value::String(texd_path));
							}

							let ext = path_ref.extension()
								.and_then(|e| e.to_str())
								.map(|s| s.to_ascii_lowercase());

							match ext.as_deref() {
								None => bail!("Failed to read file type"),
								Some("dds") => {
									let dds_data = glacier_texture::convert::create_dds(&texture)
										.context("Couldn't convert texture to DDS")?;
									fs::write(path_ref, dds_data)
										.context("Failed to write DDS file")?;
								}
								_ => {
									let image = glacier_texture::convert::create_dynamic_image(&texture)
										.context("Couldn't convert texture")?;
									image.save(path_ref)
										.context("Failed to save image")?;
								}
							}

							if let Some(obj) = meta.as_object_mut() {
								obj.insert("type".to_owned(), Value::String(format!("{:?}", texture.texture_type())));
								if texture.format() != RenderFormat::BC7 {
									obj.insert("format".to_owned(), Value::String(format!("{:?}", texture.format())));
								}
								if let Some(interpret_as) = texture.interpret_as(){
									obj.insert("interpretAs".to_owned(), Value::String(format!("{:?}", interpret_as)));
								}
							}

							let file_stem = path_ref.file_name()
								.and_then(|f| f.to_str())
								.map(|s| s.split('.').next().unwrap_or(s))
								.context("Invalid path: missing file name")?;

							let meta_path = path_ref.with_file_name(format!("{}.texture.json", file_stem));
							let meta_bytes = serde_json::to_vec(&meta).context("Couldn't serialize metadata")?;

							fs::write(&meta_path, meta_bytes).context("Failed to write metadata file")?;
						}

						_ => bail!("Unsupported resource type")
					}
				}
			}
		}

		ResourceOverviewEvent::ExtractAsWav { id } => {
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref() {
				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(&format!("{}.wav", hash))
					.add_filter("WAV file", &["wav"])
					.blocking_save_file()
				{
					let (_, res_data) = extract_latest_resource(game_files, hash)?;

					let data_dir = app.path().app_data_dir().expect("Couldn't get data dir");

					let temp_file_id = Uuid::new_v4();

					fs::write(data_dir.join("temp").join(format!("{}.wem", temp_file_id)), res_data)?;

					app.shell()
						.sidecar("vgmstream-cli")
						.unwrap()
						.current_dir(data_dir.join("temp"))
						.args([
							&format!("{}.wem", temp_file_id),
							"-L",
							"-o",
							path.as_path().context("Invalid path")?.to_string_lossy().as_ref()
						])
						.run()
						.await
						.context("VGMStream command failed")?;
				}
			}
		}

		ResourceOverviewEvent::ExtractMultiWav { id } => {
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref() {
				let mut dialog = app.dialog().file().set_title("Extract all WAVs to folder");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog.blocking_pick_folder() {
					let data_dir = app.path().app_data_dir().expect("Couldn't get data dir");

					let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

					let wwev = WwiseEvent::parse(&res_data)?;

					let mut idx = 0;

					for object in wwev.non_streamed {
						let temp_file_id = Uuid::new_v4();

						fs::write(data_dir.join("temp").join(format!("{}.wem", temp_file_id)), object.data)?;

						app.shell()
							.sidecar("vgmstream-cli")
							.unwrap()
							.current_dir(data_dir.join("temp"))
							.args([
								&format!("{}.wem", temp_file_id),
								"-L",
								"-o",
								path.as_path()
									.context("Invalid path")?
									.join(format!("{}.wav", idx))
									.to_string_lossy()
									.as_ref()
							])
							.run()
							.await
							.context("VGMStream command failed")?;

						idx += 1;
					}

					for object in wwev.streamed {
						let temp_file_id = Uuid::new_v4();

						let wwem_hash = res_meta
							.core_info
							.references
							.get(object.dependency_index as usize)
							.context("No such WWEM dependency")?
							.resource
							.get_id();

						let (_, wem_data) = extract_latest_resource(game_files, wwem_hash)?;

						fs::write(data_dir.join("temp").join(format!("{}.wem", temp_file_id)), wem_data)?;

						app.shell()
							.sidecar("vgmstream-cli")
							.unwrap()
							.current_dir(data_dir.join("temp"))
							.args([
								&format!("{}.wem", temp_file_id),
								"-L",
								"-o",
								path.as_path()
									.context("Invalid path")?
									.join(format!("{}.wav", idx))
									.to_string_lossy()
									.as_ref()
							])
							.run()
							.await
							.context("VGMStream command failed")?;

						idx += 1;
					}
				}
			}
		}

		ResourceOverviewEvent::ExtractSpecificMultiWav { id, index } => {
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref() {
				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(&format!("{}~{}.wav", hash, index))
					.add_filter("WAV file", &["wav"])
					.blocking_save_file()
				{
					let data_dir = app.path().app_data_dir().expect("Couldn't get data dir");

					let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

					let wwev = WwiseEvent::parse(&res_data)?;

					let temp_file_id = Uuid::new_v4();

					if index < wwev.non_streamed.len() as u32 {
						fs::write(
							data_dir.join("temp").join(format!("{}.wem", temp_file_id)),
							&wwev
								.non_streamed
								.get(index as usize)
								.context("No such audio object")?
								.data
						)?;

						app.shell()
							.sidecar("vgmstream-cli")
							.unwrap()
							.current_dir(data_dir.join("temp"))
							.args([
								&format!("{}.wem", temp_file_id),
								"-L",
								"-o",
								path.as_path().context("Invalid path")?.to_string_lossy().as_ref()
							])
							.run()
							.await
							.context("VGMStream command failed")?;
					} else {
						let wwem_hash = res_meta
							.core_info
							.references
							.get(
								wwev.streamed
									.get(index as usize - wwev.non_streamed.len())
									.context("No such audio object")?
									.dependency_index as usize
							)
							.context("No such WWEM dependency")?
							.resource
							.get_id();

						let (_, wem_data) = extract_latest_resource(game_files, wwem_hash)?;

						fs::write(data_dir.join("temp").join(format!("{}.wem", temp_file_id)), wem_data)?;

						app.shell()
							.sidecar("vgmstream-cli")
							.unwrap()
							.current_dir(data_dir.join("temp"))
							.args([
								&format!("{}.wem", temp_file_id),
								"-L",
								"-o",
								path.as_path().context("Invalid path")?.to_string_lossy().as_ref()
							])
							.run()
							.await
							.context("VGMStream command failed")?;
					}
				}
			}
		}

		ResourceOverviewEvent::ExtractAsHMLanguages { id } => {
			let editor_state = app_state.editor_states.get(&id).context("No such editor")?;

			let hash = match editor_state.data {
				EditorData::ResourceOverview { hash, .. } => hash,

				_ => {
					Err(anyhow!("Editor {} is not a resource overview", id))?;
					panic!();
				}
			};

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
				&& let Some(hash_list) = app_state.hash_list.load().as_ref()
			{
				let game_version = get_loaded_game_version(app, install)?;

				let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(&format!(
						"{}.{}.json",
						hash,
						res_meta.core_info.resource_type.as_ref().to_lowercase()
					))
					.add_filter(
						format!("{}.json file", res_meta.core_info.resource_type.as_ref().to_lowercase()),
						&[&format!(
							"{}.json",
							res_meta.core_info.resource_type.as_ref().to_lowercase()
						)]
					)
					.blocking_save_file()
				{
					fs::write(
						path.as_path().context("Invalid path")?,
						match res_meta.core_info.resource_type.as_ref() {
							"CLNG" => {
								let clng = {
									let mut iteration = 0;

									loop {
										if let Ok::<_, anyhow::Error>(x) = try {
											let langmap = get_language_map(game_version, iteration)
												.context("No more alternate language maps available")?;

											let clng =
												hmlanguages::clng::CLNG::new(game_version.into(), langmap.1.to_owned())
													.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

											clng.convert(
												&res_data,
												to_string(
													&RpkgResourceMeta::from_resource_metadata(
														res_meta.to_owned(),
														false
													)
													.with_hash_list(&hash_list.entries)?
												)?
											)
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

								ditl.convert(
									&res_data,
									to_string(
										&RpkgResourceMeta::from_resource_metadata(res_meta, false)
											.with_hash_list(&hash_list.entries)?
									)?
								)
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
												game_version.into(),
												langmap.1.to_owned(),
												None,
												false
											)
											.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

											dlge.convert(
												&res_data,
												to_string(
													&RpkgResourceMeta::from_resource_metadata(
														res_meta.to_owned(),
														false
													)
													.with_hash_list(&hash_list.entries)?
												)?
											)
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
												game_version.into(),
												langmap.1.to_owned(),
												langmap.0
											)
											.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

											locr.convert(
												&res_data,
												to_string(
													&RpkgResourceMeta::from_resource_metadata(
														res_meta.to_owned(),
														false
													)
													.with_hash_list(&hash_list.entries)?
												)?
											)
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
								let rtlv = hmlanguages::rtlv::RTLV::new(game_version.into(), None)
									.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
									.convert(
										&res_data,
										to_string(
											&RpkgResourceMeta::from_resource_metadata(res_meta, false)
												.with_hash_list(&hash_list.entries)?
										)?
									)
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
