use std::{
	fs::{self, File},
	io::{BufWriter, Cursor, Write},
	ops::Deref,
	sync::Arc
};

use anyhow::{Context, Result, anyhow, bail};
use arc_swap::ArcSwap;
use ecow::EcoVec;
use fn_error_context::context;
use glacier_texture::{
	enums::{RenderFormat, TextureType},
	mipblock::MipblockData,
	texture_map::TextureMap
};
use hashbrown::HashMap;
use hitman_commons::{
	game::GameVersion,
	metadata::{ResourceType, RuntimeID},
	rpkg_tool::RpkgResourceMeta
};
use hitman_formats::{
	material::{MaterialEntity, MaterialInstance},
	sdef::SoundDefinitions,
	wwev::WwiseEvent
};
use image::{ImageFormat, ImageReader};
use prim_rs::render_primitive::RenderPrimitive;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use rpkg_rs::{GlacierResource, resource::partition_manager::PartitionManager};
use serde::Serialize;
use serde_json::{json, to_string, to_vec};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_aptabase::EventTracker;
use tauri_plugin_dialog::DialogExt;
use tonytools::hmlanguages;
use tryvial::{try_block, try_fn};
use uuid::Uuid;
use ww2ogg::{CodebookLibrary, WwiseRiffVorbis};

use crate::{
	Notification, NotificationKind,
	bin1::deserialize_generic,
	biome::format_json,
	finish_task,
	general::open_in_editor,
	get_loaded_game_version,
	languages::get_language_map,
	model::{
		AppSettings, AppState, EditorData, EditorRequest, EditorRequestData, EditorState, EditorType, Hash, Request,
		ResourceOverviewData, ResourceOverviewEvent, ResourceOverviewRequest, TabRequest, TabRequestData
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
	file_types: &Arc<HashMap<RuntimeID, ResourceType>>
) -> Result<()> {
	let (filetype, chunk_patch, deps) = extract_latest_overview_info(game_files, hash)?;

	send_request(
		app,
		Request::Editor(EditorRequest {
			editor: id,
			data: EditorRequestData::ResourceOverview(ResourceOverviewRequest::Initialise {
				hash: Hash(hash),
				filetype: filetype.into(),
				chunk_patch,
				path_or_hint: hash
					.get_info()
					.and_then(|x| x.path.as_ref().or(x.hint.as_ref()).cloned()),
				dependencies: deps
					.into_par_iter()
					.map(|dep| {
						(
							Hash(dep.resource),
							file_types.get(&dep.resource).copied(),
							dep.resource.get_info().and_then(|x| x.path.or(x.hint)),
							dep.flags,
							resource_reverse_dependencies.contains_key(&dep.resource)
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
									Hash(*hash),
									*file_types.get(hash).unwrap(),
									hash.get_info().and_then(|x| x.path.or(x.hint))
								)
							})
							.collect()
					})
					.unwrap_or_default(),
				changelog: extract_resource_changelog(game_files, hash),
				data: match filetype.as_ref() {
					"TEMP" => {
						let entity = extract_entity(game_files, &app_state.cached_entities, game_version, hash)?;

						ResourceOverviewData::Entity {
							blueprint_hash: Hash(entity.blueprint),
							blueprint_path_or_hint: entity
								.blueprint
								.get_info()
								.and_then(|x| x.path.as_ref().or(x.hint.as_ref()).cloned())
						}
					}

					"ORES" if hash == "0057C2C3941115CA".parse()? => ResourceOverviewData::Unlockables,

					"AIBB" | "AIRG" | "ASVA" | "ATMD" | "BMSK" | "CBLU" | "CPPT" | "CRMD" | "ENUM" | "GFXF"
					| "GIDX" | "UICB" | "VIDB" | "WSGB" | "WSWB" | "ECPB" | "DSWB" | "ORES" => {
						let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

						ResourceOverviewData::GenericRL {
							json: {
								let mut buf = Vec::new();
								let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
								let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

								deserialize_generic(
									game_version,
									if res_meta.core_info.resource_type == "DSWB" {
										"WSWB".try_into()?
									} else {
										res_meta.core_info.resource_type
									},
									&res_data
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

					"GFXI" => {
						let asset_id = Uuid::new_v4();

						let (_, res_data) = extract_latest_resource(game_files, hash)?;

						let mut image_data = vec![];

						ImageReader::new(Cursor::new(res_data))
							.with_guessed_format()?
							.decode()?
							.write_to(Cursor::new(&mut image_data), image::ImageFormat::WebP)?;

						app_state
							.editor_states
							.get(&id)
							.context("No such editor")?
							.assets
							.insert(asset_id, ("image/webp".into(), image_data.into()));

						ResourceOverviewData::Image {
							asset_id,
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

						let mut obj = EcoVec::new();

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

						let asset_id = Uuid::new_v4();

						app_state
							.editor_states
							.get(&id)
							.context("No such editor")?
							.assets
							.insert(asset_id, ("model/obj".into(), obj));

						ResourceOverviewData::Mesh { asset_id, bounding_box }
					}

					"TEXT" => {
						let asset_id = Uuid::new_v4();

						let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

						let mut texture = TextureMap::process_data(game_version.into(), res_data)
							.context("Couldn't process texture data")?;

						if let Some(texd_depend) = res_meta.core_info.references.first() {
							let (_, texd_data) = extract_latest_resource(game_files, texd_depend.resource)?;
							let mipblock = MipblockData::from_memory(&texd_data, game_version.into())
								.context("Couldn't process TEXD data")?;
							texture.set_mipblock1(mipblock);
						}

						let tga_data = glacier_texture::convert::create_tga(&texture)
							.context("Couldn't convert texture to TGA")?;

						let mut reader = ImageReader::new(Cursor::new(tga_data.to_owned()));

						reader.set_format(image::ImageFormat::Tga);

						let mut image_data = vec![];

						reader
							.decode()?
							.write_to(Cursor::new(&mut image_data), image::ImageFormat::WebP)?;

						app_state
							.editor_states
							.get(&id)
							.context("No such editor")?
							.assets
							.insert(asset_id, ("image/webp".into(), image_data.into()));

						ResourceOverviewData::Image {
							asset_id,
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
						let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

						let mut audios = vec![];

						let wwev = WwiseEvent::parse(&res_data, &res_meta.core_info)?;

						for object in wwev.non_streamed {
							let asset_id = Uuid::new_v4();

							let mut wav_data = EcoVec::new();

							WwiseRiffVorbis::new(Cursor::new(object.data), CodebookLibrary::aotuv_codebooks()?)?
								.generate_ogg(&mut wav_data)?;

							app_state
								.editor_states
								.get(&id)
								.context("No such editor")?
								.assets
								.insert(asset_id, ("audio/wav".into(), wav_data));

							audios.push(("Embedded audio".into(), asset_id))
						}

						for object in wwev.streamed {
							let asset_id = Uuid::new_v4();

							let (_, wem_data) = extract_latest_resource(game_files, object.source)?;

							let mut wav_data = EcoVec::new();

							WwiseRiffVorbis::new(Cursor::new(wem_data), CodebookLibrary::aotuv_codebooks()?)?
								.generate_ogg(&mut wav_data)?;

							app_state
								.editor_states
								.get(&id)
								.context("No such editor")?
								.assets
								.insert(asset_id, ("audio/wav".into(), wav_data));

							audios.push((object.source.to_string(), asset_id))
						}

						ResourceOverviewData::MultiAudio {
							name: wwev.name,
							audios
						}
					}

					"WWES" | "WWEM" => {
						let asset_id = Uuid::new_v4();

						let (_, res_data) = extract_latest_resource(game_files, hash)?;

						let mut wav_data = EcoVec::new();

						WwiseRiffVorbis::new(Cursor::new(res_data), CodebookLibrary::aotuv_codebooks()?)?
							.generate_ogg(&mut wav_data)?;

						app_state
							.editor_states
							.get(&id)
							.context("No such editor")?
							.assets
							.insert(asset_id, ("audio/wav".into(), wav_data));

						ResourceOverviewData::Audio { asset_id }
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
									if let Ok::<_, anyhow::Error>(x) = try_block! {
										let langmap = get_language_map(game_version, iteration)
											.context("No more alternate language maps available")?;

										let clng = hmlanguages::clng::CLNG::new(game_version.into(), langmap.1.to_owned())
											.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

										clng.convert(
											&res_data,
											to_string(
												&RpkgResourceMeta::from_resource_metadata(res_meta.to_owned(), false)

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
								to_string(&RpkgResourceMeta::from_resource_metadata(res_meta, false))?
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
									if let Ok::<_, anyhow::Error>(x) = try_block! {
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
									if let Ok::<_, anyhow::Error>(x) = try_block! {
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
									to_string(&RpkgResourceMeta::from_resource_metadata(res_meta, false))?
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
							)?;

							let locr = {
								let mut iteration = 0;

								loop {
									if let Ok::<_, anyhow::Error>(x) = try_block! {
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

							let res_data: [u8; 5] =
								res_data.try_into().ok().context("Couldn't read LINE data as u32")?;

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

							let material = MaterialInstance::parse(&res_data, &res_meta.core_info)
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
							)?;

							let material = MaterialEntity::parse(
								&matt_data,
								&matt_meta.core_info,
								&matb_data,
								&matb_meta.core_info
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

							let sdef = SoundDefinitions::parse(&res_data, &res_meta.core_info, game_version)
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
			})
		})
	)?;
}

#[try_fn]
#[context("Couldn't handle resource overview event")]
pub async fn handle_resource_overview_event(app: &AppHandle, id: Uuid, event: ResourceOverviewEvent) -> Result<()> {
	let app_settings = app.state::<ArcSwap<AppSettings>>();
	let app_state = app.state::<AppState>();

	let hash = match app_state.editor_states.get(&id).context("No such editor")?.data {
		EditorData::ResourceOverview { hash, .. } => hash,

		_ => bail!("Editor {id} is not a resource overview")
	};

	match event {
		ResourceOverviewEvent::Initialise => {
			let task = start_task(app, format!("Loading resource overview for {}", hash))?;

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(resource_reverse_dependencies) = app_state.resource_reverse_dependencies.load().as_ref()
				&& let Some(file_types) = app_state.file_types.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
			{
				initialise_resource_overview(
					app,
					&app_state,
					id,
					hash,
					game_files,
					get_loaded_game_version(app, install)?,
					resource_reverse_dependencies,
					file_types
				)
				.await?;
			}

			finish_task(app, task)?;
		}

		ResourceOverviewEvent::FollowDependency { new_hash } => {
			match app_state.editor_states.get_mut(&id).context("No such editor")?.data {
				EditorData::ResourceOverview { ref mut hash, .. } => {
					*hash = new_hash;
				}

				_ => bail!("Editor {id} is not a resource overview")
			};

			let task = start_task(app, format!("Loading resource overview for {}", hash))?;

			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(resource_reverse_dependencies) = app_state.resource_reverse_dependencies.load().as_ref()
				&& let Some(file_types) = app_state.file_types.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
			{
				initialise_resource_overview(
					app,
					&app_state,
					id,
					new_hash,
					game_files,
					get_loaded_game_version(app, install)?,
					resource_reverse_dependencies,
					file_types
				)
				.await?;

				send_request(
					app,
					Request::Tab(TabRequest {
						tab: id,
						data: TabRequestData::Rename {
							new_name: format!("Resource overview ({new_hash})")
						}
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
					data: EditorData::ResourceOverview { hash },
					..Default::default()
				}
			);

			send_request(
				app,
				Request::Tab(TabRequest {
					tab: id,
					data: TabRequestData::Create {
						name: format!("Resource overview ({hash})"),
						editor_type: EditorType::ResourceOverview
					}
				})
			)?;
		}

		ResourceOverviewEvent::OpenInEditor => {
			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
			{
				open_in_editor(app, game_files, install, hash).await?;
			}
		}

		ResourceOverviewEvent::ExtractAsFile => {
			if let Some(game_files) = app_state.game_files.load().as_ref() {
				let (metadata, data) = extract_latest_resource(game_files, hash)?;

				let file_type = hash
					.get_info()
					.expect("Can only open files from the hash list")
					.resource_type
					.to_owned();

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(format!("{}.{}", hash.to_hash(), &file_type))
					.add_filter(format!("{} file", &file_type), &[file_type.as_ref()])
					.blocking_save_file()
				{
					fs::write(path.as_path().context("Invalid path")?, data)?;

					fs::write(
						path.as_path()
							.context("Invalid path")?
							.with_added_extension("metadata.json"),
						format_json(&to_string(&metadata.core_info)?)?
					)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractAsQN => {
			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
			{
				let entity_json = to_vec(&*extract_entity(
					game_files,
					&app_state.cached_entities,
					get_loaded_game_version(app, install)?,
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

		ResourceOverviewEvent::ExtractTEMPAsRT => {
			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
			{
				let (metadata, data) = extract_latest_resource(game_files, hash)?;

				let data = match get_loaded_game_version(app, install)? {
					GameVersion::H1 => to_vec(
						&hitman_bin1::deserialize::<hitman_bin1::game::h1::STemplateEntity>(&data)
							.context("Couldn't deserialise factory")?
					)?,

					GameVersion::H2 => to_vec(
						&hitman_bin1::deserialize::<hitman_bin1::game::h2::STemplateEntityFactory>(&data)
							.context("Couldn't deserialise factory")?
					)?,

					GameVersion::H3 => to_vec(
						&hitman_bin1::deserialize::<hitman_bin1::game::h3::STemplateEntityFactory>(&data)
							.context("Couldn't deserialise factory")?
					)?
				};

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(format!("{}.TEMP.json", hash.to_hash()))
					.add_filter("TEMP.json file", &["TEMP.json"])
					.blocking_save_file()
				{
					fs::write(path.as_path().context("Invalid path")?, data)?;

					fs::write(
						path.as_path().context("Invalid path")?.with_extension("metadata.json"),
						format_json(&to_string(&metadata.core_info)?)?
					)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractTBLUAsFile => {
			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
			{
				let (metadata, data) = extract_latest_resource(
					game_files,
					extract_entity(
						game_files,
						&app_state.cached_entities,
						get_loaded_game_version(app, install)?,
						hash
					)?
					.blueprint
				)?;

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(format!("{}.TBLU", metadata.core_info.id.to_hash()))
					.add_filter("TBLU file", &["TBLU"])
					.blocking_save_file()
				{
					fs::write(path.as_path().context("Invalid path")?, data)?;

					fs::write(
						path.as_path()
							.context("Invalid path")?
							.with_added_extension("metadata.json"),
						format_json(&to_string(&metadata.core_info)?)?
					)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractTBLUAsRT => {
			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
			{
				let game_version = get_loaded_game_version(app, install)?;

				let (metadata, data) = extract_latest_resource(
					game_files,
					extract_entity(game_files, &app_state.cached_entities, game_version, hash)?.blueprint
				)?;

				let data = match game_version {
					GameVersion::H1 => to_vec(
						&hitman_bin1::deserialize::<hitman_bin1::game::h1::STemplateEntityBlueprint>(&data)
							.context("Couldn't deserialise blueprint")?
					)?,

					GameVersion::H2 => to_vec(
						&hitman_bin1::deserialize::<hitman_bin1::game::h2::STemplateEntityBlueprint>(&data)
							.context("Couldn't deserialise blueprint")?
					)?,

					GameVersion::H3 => to_vec(
						&hitman_bin1::deserialize::<hitman_bin1::game::h3::STemplateEntityBlueprint>(&data)
							.context("Couldn't deserialise blueprint")?
					)?
				};

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(format!("{}.TBLU.json", metadata.core_info.id.to_hash()))
					.add_filter("TBLU.json file", &["TBLU.json"])
					.blocking_save_file()
				{
					fs::write(path.as_path().context("Invalid path")?, data)?;

					fs::write(
						path.as_path().context("Invalid path")?.with_extension("metadata.json"),
						format_json(&to_string(&metadata.core_info)?)?
					)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractAsRTGeneric => {
			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
			{
				let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(format!("{}.{}.json", hash.to_hash(), res_meta.core_info.resource_type))
					.add_filter(
						format!("{}.json file", res_meta.core_info.resource_type),
						&[&format!("{}.json", res_meta.core_info.resource_type)]
					)
					.blocking_save_file()
				{
					fs::write(
						path.as_path().context("Invalid path")?,
						to_vec(&deserialize_generic(
							get_loaded_game_version(app, install)?,
							res_meta.core_info.resource_type,
							&res_data
						)?)?
					)?;

					fs::write(
						path.as_path().context("Invalid path")?.with_extension("metadata.json"),
						format_json(&to_string(&res_meta.core_info)?)?
					)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractAsImage => {
			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
			{
				let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(format!("{}.png", hash.to_hash()))
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
							let mut texture =
								TextureMap::process_data(get_loaded_game_version(app, install)?.into(), res_data)
									.context("Couldn't process texture data")?;

							if let Some(texd_depend) = res_meta.core_info.references.first() {
								let (_, texd_data) = extract_latest_resource(game_files, texd_depend.resource)?;

								let mip_block = MipblockData::from_memory(
									&texd_data,
									get_loaded_game_version(app, install)?.into()
								)
								.context("Couldn't process TEXD data")?;
								texture.set_mipblock1(mip_block);
							}

							if path
								.as_path()
								.context("Invalid path")?
								.file_name()
								.context("No file name")?
								.to_str()
								.context("Filename was invalid string")?
								.ends_with(".dds")
							{
								let dds_data = glacier_texture::convert::create_dds(&texture)
									.context("Couldn't convert texture to DDS")?;

								fs::write(path.as_path().context("Invalid path")?, dds_data)?;
							} else {
								let tga_data = glacier_texture::convert::create_tga(&texture)
									.context("Couldn't convert texture to TGA")?;

								let mut reader = ImageReader::new(Cursor::new(tga_data.to_owned()));

								reader.set_format(image::ImageFormat::Tga);

								if path
									.as_path()
									.context("Invalid path")?
									.file_name()
									.context("No file name")?
									.to_str()
									.context("Filename was invalid string")?
									.ends_with(".tga")
								{
									fs::write(path.as_path().context("Invalid path")?, tga_data)?;
								} else {
									reader.decode()?.save(path.as_path().context("Invalid path")?)?;
								}
							}
						}

						_ => bail!("Unsupported resource type")
					}
				}
			}
		}

		ResourceOverviewEvent::ExtractAsWav => {
			if let Some(game_files) = app_state.game_files.load().as_ref() {
				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(format!("{}.wav", hash.to_hash()))
					.add_filter("WAV file", &["wav"])
					.blocking_save_file()
				{
					let (_, res_data) = extract_latest_resource(game_files, hash)?;

					WwiseRiffVorbis::new(Cursor::new(res_data), CodebookLibrary::aotuv_codebooks()?)?.generate_ogg(
						BufWriter::new(File::create(
							path.as_path().context("Invalid path")?.to_string_lossy().as_ref()
						)?)
					)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractMultiWav => {
			if let Some(game_files) = app_state.game_files.load().as_ref() {
				let mut dialog = app.dialog().file().set_title("Extract all WAVs to folder");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog.blocking_pick_folder() {
					let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

					let wwev = WwiseEvent::parse(&res_data, &res_meta.core_info)?;

					let mut idx = 0;

					for object in wwev.non_streamed {
						WwiseRiffVorbis::new(Cursor::new(object.data), CodebookLibrary::aotuv_codebooks()?)?
							.generate_ogg(BufWriter::new(File::create(
								path.as_path()
									.context("Invalid path")?
									.join(format!("{}.wav", idx))
									.to_string_lossy()
									.as_ref()
							)?))?;

						idx += 1;
					}

					for object in wwev.streamed {
						let (_, wem_data) = extract_latest_resource(game_files, object.source)?;

						WwiseRiffVorbis::new(Cursor::new(wem_data), CodebookLibrary::aotuv_codebooks()?)?
							.generate_ogg(BufWriter::new(File::create(
								path.as_path()
									.context("Invalid path")?
									.join(format!("{}.wav", idx))
									.to_string_lossy()
									.as_ref()
							)?))?;

						idx += 1;
					}
				}
			}
		}

		ResourceOverviewEvent::ExtractSpecificMultiWav { index } => {
			if let Some(game_files) = app_state.game_files.load().as_ref() {
				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(format!("{}~{}.wav", hash.to_hash(), index))
					.add_filter("WAV file", &["wav"])
					.blocking_save_file()
				{
					let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

					let wwev = WwiseEvent::parse(&res_data, &res_meta.core_info)?;

					if index < wwev.non_streamed.len() as u32 {
						WwiseRiffVorbis::new(
							Cursor::new(
								&wwev
									.non_streamed
									.get(index as usize)
									.context("No such audio object")?
									.data
							),
							CodebookLibrary::aotuv_codebooks()?
						)?
						.generate_ogg(BufWriter::new(File::create(
							path.as_path().context("Invalid path")?.to_string_lossy().as_ref()
						)?))?;
					} else {
						let wwem_hash = wwev
							.streamed
							.get(index as usize - wwev.non_streamed.len())
							.context("No such audio object")?
							.source;

						let (_, wem_data) = extract_latest_resource(game_files, wwem_hash)?;

						WwiseRiffVorbis::new(Cursor::new(wem_data), CodebookLibrary::aotuv_codebooks()?)?
							.generate_ogg(BufWriter::new(File::create(
								path.as_path().context("Invalid path")?.to_string_lossy().as_ref()
							)?))?;
					}
				}
			}
		}

		ResourceOverviewEvent::ExtractAsHMLanguages => {
			if let Some(game_files) = app_state.game_files.load().as_ref()
				&& let Some(install) = app_settings.load().game_install.as_ref()
			{
				let game_version = get_loaded_game_version(app, install)?;

				let (res_meta, res_data) = extract_latest_resource(game_files, hash)?;

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(format!(
						"{}.{}.json",
						hash.to_hash(),
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
										if let Ok::<_, anyhow::Error>(x) = try_block! {
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
									to_string(&RpkgResourceMeta::from_resource_metadata(res_meta, false))?
								)
								.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
								.serialize(&mut ser)?;

								buf
							}

							"DLGE" => {
								let dlge = {
									let mut iteration = 0;

									loop {
										if let Ok::<_, anyhow::Error>(x) = try_block! {
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
										if let Ok::<_, anyhow::Error>(x) = try_block! {
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
										to_string(&RpkgResourceMeta::from_resource_metadata(res_meta, false))?
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
