use std::{
	fs::{self, File},
	io::{BufWriter, Cursor, Write},
	ops::Deref
};

use anyhow::{Context, Result, anyhow, bail};
use ecow::EcoVec;
use fn_error_context::context;
use glacier_texture::{
	enums::{InterpretAs, RenderFormat, TextureType},
	mipblock::MipblockData,
	texture_map::TextureMap
};
use hitman_commons::{game::GameVersion, metadata::RuntimeID, rpkg_tool::RpkgResourceMeta};
use hitman_formats::{
	material::{MaterialEntity, MaterialInstance},
	sdef::SoundDefinitions,
	texture::TextureMetadata,
	wwev::WwiseEvent
};
use image::{ImageFormat, ImageReader};
use optivorbis::{OggToOgg, Remuxer};
use prim_rs::render_primitive::RenderPrimitive;
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, ParallelIterator};
use rpkg_rs::GlacierResource;
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
	game::Game,
	general::{UNLOCKABLES_ID, get_name, open_in_editor},
	languages::get_language_map,
	model::{
		AppState, EditorData, EditorRequest, EditorRequestData, EditorState, EditorType, Hash, Request,
		ResourceOverviewData, ResourceOverviewEvent, ResourceOverviewRequest, TabRequest, TabRequestData
	},
	send_notification, send_request, start_task
};

#[try_fn]
#[context("Couldn't open resource overview for {resource}")]
pub async fn open_resource_overview(app: &AppHandle, resource: RuntimeID) -> Result<()> {
	let app_state = app.state::<AppState>();
	let id = Uuid::new_v4();

	app_state
		.editor_states
		.insert(
			id.to_owned(),
			EditorState {
				file: None,
				data: EditorData::ResourceOverview { hash: resource },
				..Default::default()
			}
		)
		.await;

	send_request(
		app,
		Request::Tab(TabRequest {
			tab: id,
			data: TabRequestData::Create {
				name: format!("Resource overview for {}", get_name(&resource.to_string())),
				editor_type: EditorType::ResourceOverview
			}
		})
	)?;
}

#[try_fn]
#[context("Couldn't parse PRIM file")]
pub fn parse_prim(game: &Game, res_data: &[u8]) -> Result<(EcoVec<u8>, [f32; 6])> {
	let model = RenderPrimitive::process_data(game.version().into(), res_data).context("Couldn't process PRIM data")?;

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
	let mut bounding_box: [f32; 6] = [
		f32::INFINITY,
		f32::INFINITY,
		f32::INFINITY,
		f32::NEG_INFINITY,
		f32::NEG_INFINITY,
		f32::NEG_INFINITY
	];

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

	(obj, bounding_box)
}

#[try_fn]
#[context("Couldn't initialise resource overview {id}")]
pub async fn initialise_resource_overview(
	app: &AppHandle,
	app_state: &State<'_, AppState>,
	id: Uuid,
	hash: RuntimeID,
	game: &Game
) -> Result<()> {
	let (filetype, chunk_patch, deps) = game.extract_latest_overview_info(hash)?;

	send_request(
		app,
		Request::Editor(EditorRequest {
			editor: id,
			data: EditorRequestData::ResourceOverview(ResourceOverviewRequest::Initialise {
				hash: Hash(hash),
				filetype: filetype.into(),
				chunk_patch,
				path_or_hint: hash.get_path_or_hint(),
				dependencies: deps
					.into_par_iter()
					.map(|dep| {
						(
							Hash(dep.resource),
							game.resource_type(dep.resource),
							dep.resource.get_path_or_hint(),
							dep.flags,
							game.resource_exists(dep.resource)
						)
					})
					.collect(),
				reverse_dependencies: game
					.resource_reverse_references(hash)
					.map(|hashes| {
						hashes
							.iter()
							.map(|&hash| (Hash(hash), game.resource_type(hash).unwrap(), hash.get_path_or_hint()))
							.collect()
					})
					.unwrap_or_default(),
				changelog: game.extract_resource_changelog(hash),
				data: match filetype.as_ref() {
					"TEMP" => {
						let entity = game.extract_entity(hash)?;

						ResourceOverviewData::Entity {
							root_entity_name: entity
								.entities
								.get(&entity.root_entity)
								.map_or_else(|| "Unknown".into(), |x| x.name.as_str().into()),
							blueprint_hash: Hash(entity.blueprint),
							blueprint_path_or_hint: entity.blueprint.get_path_or_hint()
						}
					}

					"ORES" if hash == UNLOCKABLES_ID => ResourceOverviewData::Unlockables,

					"AIBB" | "AIRG" | "ASVA" | "ATMD" | "BMSK" | "CBLU" | "CPPT" | "CRMD" | "ENUM" | "GFXF"
					| "GIDX" | "UICB" | "VIDB" | "WSGB" | "WSWB" | "ECPB" | "DSWB" | "ORES" | "TBLU" => {
						let (res_meta, res_data) = game.extract_latest_resource(hash)?;

						ResourceOverviewData::GenericRL {
							json: {
								let mut buf = Vec::new();
								let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
								let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

								deserialize_generic(
									game.version(),
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

						let (_, res_data) = game.extract_latest_resource(hash)?;

						let mut image_data = vec![];

						ImageReader::new(Cursor::new(res_data))
							.with_guessed_format()?
							.decode()?
							.write_to(Cursor::new(&mut image_data), image::ImageFormat::WebP)?;

						app_state
							.editor_states
							.get(&id)
							.await
							.context("No such editor")?
							.assets
							.insert(asset_id, ("image/webp".into(), image_data.into()))
							.await;

						ResourceOverviewData::Image {
							asset_id,
							texture_data: None
						}
					}

					"PRIM" => {
						let (_, res_data) = game.extract_latest_resource(hash)?;

						let (obj, bounding_box) = parse_prim(game, &res_data)?;

						let asset_id = Uuid::new_v4();

						app_state
							.editor_states
							.get(&id)
							.await
							.context("No such editor")?
							.assets
							.insert(asset_id, ("model/obj".into(), obj))
							.await;

						ResourceOverviewData::Mesh { asset_id, bounding_box }
					}

					"TEXT" => {
						let asset_id = Uuid::new_v4();

						let (res_meta, res_data) = game.extract_latest_resource(hash)?;

						let mut texture = TextureMap::process_data(game.version().into(), res_data)
							.context("Couldn't process texture data")?;

						if let Some(texd_depend) = res_meta.core_info.references.first() {
							let (_, texd_data) = game.extract_latest_resource(texd_depend.resource)?;
							let mipblock = MipblockData::from_memory(&texd_data, game.version().into())
								.context("Couldn't process TEXD data")?;
							texture.set_mipblock1(mipblock);
						}

						let image = glacier_texture::convert::create_dynamic_image(&texture)
							.context("Couldn't convert texture to dynamic image")?;

						let mut image_data = vec![];

						image.write_to(Cursor::new(&mut image_data), image::ImageFormat::WebP)?;

						app_state
							.editor_states
							.get(&id)
							.await
							.context("No such editor")?
							.assets
							.insert(asset_id, ("image/webp".into(), image_data.into()))
							.await;

						ResourceOverviewData::Image {
							asset_id,
							texture_data: Some((
								match texture.texture_type() {
									TextureType::Colour => "Colour".into(),
									TextureType::Normal => "Normal".into(),
									TextureType::Height => "Height".into(),
									TextureType::CompoundNormal => "Compound Normal".into(),
									TextureType::Billboard => "Billboard".into(),
									TextureType::Projection => "Projection".into(),
									TextureType::Emission => "Emission".into(),
									TextureType::Cubemap => "Cubemap".into(),
									TextureType::UNKNOWN5 => "Unknown (5)".into(),
									TextureType::UNKNOWN517 => "Unknown (517)".into(),
									x => format!("{x:?}")
								},
								match texture.format() {
									RenderFormat::R16G16B16A16 => "R16G16B16A16".into(),
									RenderFormat::R8G8B8A8 => "R8G8B8A8".into(),
									RenderFormat::R8G8 => "R8G8".into(),
									RenderFormat::A8 => "A8".into(),
									RenderFormat::BC1 => "BC1".into(),
									RenderFormat::BC2 => "BC2".into(),
									RenderFormat::BC3 => "BC3".into(),
									RenderFormat::BC4 => "BC4".into(),
									RenderFormat::BC5 => "BC5".into(),
									RenderFormat::BC7 => "BC7".into(),
									x => format!("{x:?}")
								},
								texture.interpret_as().map(|interpret_as| {
									match interpret_as {
										InterpretAs::Colour => "Colour",
										InterpretAs::Normal => "Normal",
										InterpretAs::Height => "Height",
										InterpretAs::CompoundNormal => "CompoundNormal",
										InterpretAs::Billboard => "Billboard",
										InterpretAs::Cubemap => "Cubemap",
										InterpretAs::Emission => "Emission",
										InterpretAs::Volume => "Volume"
									}
									.into()
								})
							))
						}
					}

					"WWEV" => {
						let (res_meta, res_data) = game.extract_latest_resource(hash)?;

						let mut audios = vec![];

						let wwev = WwiseEvent::parse(&res_data, &res_meta.core_info)?;

						for (name, data) in wwev
							.non_streamed
							.into_par_iter()
							.map(|object| {
								if object.data.starts_with(b"RIFF") {
									let mut ogg_data = vec![];

									WwiseRiffVorbis::new(
										Cursor::new(object.data),
										CodebookLibrary::aotuv_codebooks()?
									)?
									.generate_ogg(&mut ogg_data)?;

									let mut optimised_ogg = EcoVec::new();

									OggToOgg::new_with_defaults()
										.remux(&mut Cursor::new(ogg_data), &mut optimised_ogg)?;

									Ok(("Embedded audio".into(), Some(optimised_ogg)))
								} else {
									Ok(("Embedded audio".into(), None))
								}
							})
							.chain(wwev.streamed.into_par_iter().map(|object| {
								let (_, wem_data) = game.extract_latest_resource(object.source)?;

								if wem_data.starts_with(b"RIFF") {
									let mut ogg_data = vec![];

									WwiseRiffVorbis::new(Cursor::new(wem_data), CodebookLibrary::aotuv_codebooks()?)?
										.generate_ogg(&mut ogg_data)?;

									let mut optimised_ogg = EcoVec::new();

									OggToOgg::new_with_defaults()
										.remux(&mut Cursor::new(ogg_data), &mut optimised_ogg)?;

									Ok((object.source.to_string(), Some(optimised_ogg)))
								} else {
									Ok((object.source.to_string(), None))
								}
							}))
							.collect::<Result<Vec<_>>>()?
						{
							if let Some(data) = data {
								let asset_id = Uuid::new_v4();

								app_state
									.editor_states
									.get(&id)
									.await
									.context("No such editor")?
									.assets
									.insert(asset_id, ("audio/ogg".into(), data))
									.await;

								audios.push((name, Some(asset_id)));
							} else {
								audios.push((name, None));
							}
						}

						ResourceOverviewData::MultiAudio {
							name: wwev.name,
							audios
						}
					}

					"WWES" | "WWEM" => {
						let asset_id = Uuid::new_v4();

						let (_, res_data) = game.extract_latest_resource(hash)?;

						if res_data.starts_with(b"RIFF") {
							let mut ogg_data = EcoVec::new();

							WwiseRiffVorbis::new(Cursor::new(res_data), CodebookLibrary::aotuv_codebooks()?)?
								.generate_ogg(&mut ogg_data)?;

							let mut optimised_ogg = EcoVec::new();

							OggToOgg::new_with_defaults().remux(&mut Cursor::new(ogg_data), &mut optimised_ogg)?;

							app_state
								.editor_states
								.get(&id)
								.await
								.context("No such editor")?
								.assets
								.insert(asset_id, ("audio/ogg".into(), optimised_ogg))
								.await;

							ResourceOverviewData::Audio {
								asset_id: Some(asset_id)
							}
						} else {
							ResourceOverviewData::Audio { asset_id: None }
						}
					}

					"REPO" => ResourceOverviewData::Repository,

					"JSON" => ResourceOverviewData::Json {
						json: format_json(&String::from_utf8(game.extract_latest_resource(hash)?.1)?)?
					},

					"CLNG" => ResourceOverviewData::HMLanguages {
						json: {
							let (res_meta, res_data) = game.extract_latest_resource(hash)?;

							let clng = {
								let mut iteration = 0;

								loop {
									if let Ok::<_, anyhow::Error>(x) = try_block! {
										let langmap = get_language_map(game.version(), iteration)
											.context("No more alternate language maps available")?;

										let clng = hmlanguages::clng::CLNG::new(game.version().into(), langmap.1.to_owned())
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

										if get_language_map(game.version(), iteration).is_none() {
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
							let (res_meta, res_data) = game.extract_latest_resource(hash)?;

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
							let (res_meta, res_data) = game.extract_latest_resource(hash)?;

							let dlge = {
								let mut iteration = 0;

								loop {
									if let Ok::<_, anyhow::Error>(x) = try_block! {
										let langmap = get_language_map(game.version(), iteration)
											.context("No more alternate language maps available")?;

										let dlge = hmlanguages::dlge::DLGE::new(
											app_state
												.tonytools_hash_list
												.load()
												.as_ref()
												.context("No TonyTools hash list available")?
												.deref()
												.to_owned(),
											game.version().into(),
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

										if get_language_map(game.version(), iteration).is_none() {
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
							let (res_meta, res_data) = game.extract_latest_resource(hash)?;

							let locr = {
								let mut iteration = 0;

								loop {
									if let Ok::<_, anyhow::Error>(x) = try_block! {
										let langmap = get_language_map(game.version(), iteration)
											.context("No more alternate language maps available")?;

										let locr = hmlanguages::locr::LOCR::new(
											app_state
												.tonytools_hash_list
												.load()
												.as_ref()
												.context("No TonyTools hash list available")?
												.deref()
												.to_owned(),
											game.version().into(),
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

										if get_language_map(game.version(), iteration).is_none() {
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
							let (res_meta, res_data) = game.extract_latest_resource(hash)?;

							let rtlv = hmlanguages::rtlv::RTLV::new(game.version().into(), None)
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

					"LINE" => {
						let (res_meta, res_data) = game.extract_latest_resource(hash)?;

						let (locr_meta, locr_data) = game.extract_latest_resource(
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
									let langmap = get_language_map(game.version(), iteration)
										.context("No more alternate language maps available")?;

									let locr = hmlanguages::locr::LOCR::new(
										app_state
											.tonytools_hash_list
											.load()
											.as_ref()
											.context("No TonyTools hash list available")?
											.deref()
											.to_owned(),
										game.version().into(),
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

									if get_language_map(game.version(), iteration).is_none() {
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

						ResourceOverviewData::LocalisedLine {
							key: line_str.to_owned().unwrap_or_else(|| line_hash.to_owned()),
							languages: {
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
						}
					}

					"MATI" => ResourceOverviewData::MaterialInstance {
						json: {
							let (res_meta, res_data) = game.extract_latest_resource(hash)?;

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
							let (matt_meta, matt_data) = game.extract_latest_resource(hash)?;
							let (matb_meta, matb_data) = game.extract_latest_resource(
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
							let (res_meta, res_data) = game.extract_latest_resource(hash)?;

							let sdef = SoundDefinitions::parse(&res_data, &res_meta.core_info, game.version())
								.context("Couldn't parse sound definitions")?;

							let mut buf = Vec::new();
							let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
							let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

							sdef.serialize(&mut ser)?;

							String::from_utf8(buf)?
						}
					},

					"AIBZ" => ResourceOverviewData::BehaviorTree {
						pseudocode: {
							let (_, res_data) = game.extract_latest_resource(hash)?;

							match game.version() {
								GameVersion::H1 => format!(
									"{}\n---\n{:?}",
									hash,
									hitman_behavior::h1::BehaviorTree::from_raw(tokio::task::block_in_place(
										move || {
											std::thread::Builder::new()
												.stack_size(64 * 1024 * 1024)
												.spawn(move || {
													glacier_bin1::deserialize(&res_data)
														.map_err(|e| format!("{e}"))
														.unwrap()
												})
												.unwrap()
												.join()
												.unwrap()
										}
									))?
									.root
								),

								GameVersion::H2 => format!(
									"{}\n---\n{:?}",
									hash,
									hitman_behavior::h2::BehaviorTree::from_raw(tokio::task::block_in_place(
										move || {
											std::thread::Builder::new()
												.stack_size(64 * 1024 * 1024)
												.spawn(move || {
													glacier_bin1::deserialize(&res_data)
														.map_err(|e| format!("{e}"))
														.unwrap()
												})
												.unwrap()
												.join()
												.unwrap()
										}
									))?
									.root
								),

								GameVersion::H3 => format!(
									"{}\n---\n{:?}",
									hash,
									hitman_behavior::h3::BehaviorTree::from_raw(tokio::task::block_in_place(
										move || {
											std::thread::Builder::new()
												.stack_size(64 * 1024 * 1024)
												.spawn(move || {
													glacier_bin1::deserialize(&res_data)
														.map_err(|e| format!("{e}"))
														.unwrap()
												})
												.unwrap()
												.join()
												.unwrap()
										}
									))?
									.root
								)
							}
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
	let app_state = app.state::<AppState>();

	let hash = match app_state.editor_states.get(&id).await.context("No such editor")?.data {
		EditorData::ResourceOverview { hash, .. } => hash,

		_ => bail!("Editor {id} is not a resource overview")
	};

	match event {
		ResourceOverviewEvent::Initialise => {
			let task = start_task(app, format!("Loading resource overview for {}", hash))?;

			if let Some(game) = app_state.game.load().as_ref() {
				initialise_resource_overview(app, &app_state, id, hash, game).await?;
			}

			finish_task(app, task)?;
		}

		ResourceOverviewEvent::FollowDependency { new_hash } => {
			match app_state
				.editor_states
				.get_mut(&id)
				.await
				.context("No such editor")?
				.data
			{
				EditorData::ResourceOverview { ref mut hash, .. } => {
					*hash = new_hash;
				}

				_ => bail!("Editor {id} is not a resource overview")
			};

			let task = start_task(app, format!("Loading resource overview for {}", hash))?;

			if let Some(game) = app_state.game.load().as_ref() {
				initialise_resource_overview(app, &app_state, id, new_hash, game).await?;

				send_request(
					app,
					Request::Tab(TabRequest {
						tab: id,
						data: TabRequestData::Rename {
							new_name: format!("Resource overview for {}", get_name(&new_hash.to_string()))
						}
					})
				)?;
			}

			finish_task(app, task)?;
		}

		ResourceOverviewEvent::FollowDependencyInNewTab { hash, .. } => {
			open_resource_overview(app, hash).await?;
		}

		ResourceOverviewEvent::OpenInEditor => {
			if let Some(game) = app_state.game.load().as_ref() {
				open_in_editor(app, game, hash).await?;
			}
		}

		ResourceOverviewEvent::ExtractAsFile => {
			if let Some(game) = app_state.game.load().as_ref() {
				let (metadata, data) = game.extract_latest_resource(hash)?;

				let resource_type = game.resource_type(hash).context("Nonexistent resource")?;

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(format!("{}.{}", hash.to_hash(), resource_type))
					.add_filter(format!("{} file", resource_type), &[resource_type.as_ref()])
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
			if let Some(game) = app_state.game.load().as_ref() {
				let entity_json = to_vec(&*game.extract_entity(hash)?)?;

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
			if let Some(game) = app_state.game.load().as_ref() {
				let (metadata, data) = game.extract_latest_resource(hash)?;

				let data = match game.version() {
					GameVersion::H1 => to_vec(
						&glacier_bin1::deserialize::<glacier_bin1::game::h1::STemplateEntity>(&data)
							.context("Couldn't deserialise factory")?
					)?,

					GameVersion::H2 => to_vec(
						&glacier_bin1::deserialize::<glacier_bin1::game::h2::STemplateEntityFactory>(&data)
							.context("Couldn't deserialise factory")?
					)?,

					GameVersion::H3 => to_vec(
						&glacier_bin1::deserialize::<glacier_bin1::game::h3::STemplateEntityFactory>(&data)
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
			if let Some(game) = app_state.game.load().as_ref() {
				let (metadata, data) = game.extract_latest_resource(game.extract_entity(hash)?.blueprint)?;

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
			if let Some(game) = app_state.game.load().as_ref() {
				let (metadata, data) = game.extract_latest_resource(game.extract_entity(hash)?.blueprint)?;

				let data = match game.version() {
					GameVersion::H1 => to_vec(
						&glacier_bin1::deserialize::<glacier_bin1::game::h1::STemplateEntityBlueprint>(&data)
							.context("Couldn't deserialise blueprint")?
					)?,

					GameVersion::H2 => to_vec(
						&glacier_bin1::deserialize::<glacier_bin1::game::h2::STemplateEntityBlueprint>(&data)
							.context("Couldn't deserialise blueprint")?
					)?,

					GameVersion::H3 => to_vec(
						&glacier_bin1::deserialize::<glacier_bin1::game::h3::STemplateEntityBlueprint>(&data)
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
			if let Some(game) = app_state.game.load().as_ref() {
				let (res_meta, res_data) = game.extract_latest_resource(hash)?;

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
							game.version(),
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
			if let Some(game) = app_state.game.load().as_ref() {
				let (res_meta, res_data) = game.extract_latest_resource(hash)?;

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
							let mut texture = TextureMap::process_data(game.version().into(), res_data)
								.context("Couldn't process texture data")?;

							if let Some(texd_depend) = res_meta.core_info.references.first() {
								let (_, texd_data) = game.extract_latest_resource(texd_depend.resource)?;

								let mip_block = MipblockData::from_memory(&texd_data, game.version().into())
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
							} else if path
								.as_path()
								.context("Invalid path")?
								.file_name()
								.context("No file name")?
								.to_str()
								.context("Filename was invalid string")?
								.ends_with(".tga")
							{
								let tga_data = glacier_texture::convert::create_tga(&texture)
									.context("Couldn't convert texture to TGA")?;

								fs::write(path.as_path().context("Invalid path")?, tga_data)?;
							} else {
								let image = glacier_texture::convert::create_dynamic_image(&texture)
									.context("Couldn't convert texture to dynamic image")?;

								image.save(path.as_path().context("Invalid path")?)?;
							}
						}

						_ => bail!("Unsupported resource type")
					}
				}
			}
		}

		ResourceOverviewEvent::ExtractAsOgg => {
			if let Some(game) = app_state.game.load().as_ref() {
				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(format!("{}.ogg", hash.to_hash()))
					.add_filter("OGG file", &["ogg"])
					.blocking_save_file()
				{
					let (_, res_data) = game.extract_latest_resource(hash)?;

					WwiseRiffVorbis::new(Cursor::new(res_data), CodebookLibrary::aotuv_codebooks()?)?
						.generate_ogg(BufWriter::new(File::create(path.as_path().context("Invalid path")?)?))?;
				}
			}
		}

		ResourceOverviewEvent::ExtractMultiOgg => {
			if let Some(game) = app_state.game.load().as_ref() {
				let mut dialog = app.dialog().file().set_title("Extract all OGGs to folder");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog.blocking_pick_folder() {
					let task = start_task(app, format!("Extracting {hash} as OGGs"))?;

					let (res_meta, res_data) = game.extract_latest_resource(hash)?;

					let wwev = WwiseEvent::parse(&res_data, &res_meta.core_info)?;

					let non_streamed_count = wwev.non_streamed.len();

					wwev.non_streamed
						.into_par_iter()
						.enumerate()
						.try_for_each(|(idx, object)| {
							if object.data.starts_with(b"RIFF") {
								let mut ogg_data = vec![];

								WwiseRiffVorbis::new(Cursor::new(object.data), CodebookLibrary::aotuv_codebooks()?)?
									.generate_ogg(&mut ogg_data)?;

								OggToOgg::new_with_defaults().remux(
									&mut Cursor::new(ogg_data),
									&mut BufWriter::new(File::create(
										path.as_path().context("Invalid path")?.join(format!(
											"{}~{}.ogg",
											hash.to_hash(),
											idx
										))
									)?)
								)?;
							} else {
								fs::write(
									path.as_path().context("Invalid path")?.join(format!(
										"{}~{}.wem",
										hash.to_hash(),
										idx
									)),
									object.data
								)?;
							}

							anyhow::Ok(())
						})?;

					wwev.streamed
						.into_par_iter()
						.enumerate()
						.try_for_each(|(idx, object)| {
							let (_, wem_data) = game.extract_latest_resource(object.source)?;

							if wem_data.starts_with(b"RIFF") {
								let mut ogg_data = vec![];

								WwiseRiffVorbis::new(Cursor::new(wem_data), CodebookLibrary::aotuv_codebooks()?)?
									.generate_ogg(&mut ogg_data)?;

								OggToOgg::new_with_defaults().remux(
									&mut Cursor::new(ogg_data),
									&mut BufWriter::new(File::create(
										path.as_path().context("Invalid path")?.join(format!(
											"{}~{}.ogg",
											hash.to_hash(),
											non_streamed_count + idx
										))
									)?)
								)?;
							} else {
								fs::write(
									path.as_path().context("Invalid path")?.join(format!(
										"{}~{}.wem",
										hash.to_hash(),
										non_streamed_count + idx
									)),
									wem_data
								)?;
							}

							anyhow::Ok(())
						})?;

					finish_task(app, task)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractSpecificMultiOgg { index } => {
			if let Some(game) = app_state.game.load().as_ref() {
				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				let (res_meta, res_data) = game.extract_latest_resource(hash)?;

				let wwev = WwiseEvent::parse(&res_data, &res_meta.core_info)?;

				if index < wwev.non_streamed.len() as u32 {
					let object = wwev.non_streamed.get(index as usize).context("No such audio object")?;
					if object.data.starts_with(b"RIFF") {
						if let Some(path) = dialog
							.set_file_name(format!("{}~{}.ogg", hash.to_hash(), index))
							.add_filter("OGG file", &["ogg"])
							.blocking_save_file()
						{
							let mut ogg_data = vec![];

							WwiseRiffVorbis::new(Cursor::new(&object.data), CodebookLibrary::aotuv_codebooks()?)?
								.generate_ogg(&mut ogg_data)?;

							OggToOgg::new_with_defaults().remux(
								&mut Cursor::new(ogg_data),
								&mut BufWriter::new(File::create(path.as_path().context("Invalid path")?)?)
							)?;
						}
					} else {
						if let Some(path) = dialog
							.set_file_name(format!("{}~{}.wem", hash.to_hash(), index))
							.add_filter("WEM file", &["wem"])
							.blocking_save_file()
						{
							fs::write(path.as_path().context("Invalid path")?, &object.data)?;
						}
					}
				} else {
					let wwem_hash = wwev
						.streamed
						.get(index as usize - wwev.non_streamed.len())
						.context("No such audio object")?
						.source;

					let (_, wem_data) = game.extract_latest_resource(wwem_hash)?;

					if wem_data.starts_with(b"RIFF") {
						if let Some(path) = dialog
							.set_file_name(format!("{}~{}.ogg", hash.to_hash(), index))
							.add_filter("OGG file", &["ogg"])
							.blocking_save_file()
						{
							let mut ogg_data = vec![];

							WwiseRiffVorbis::new(Cursor::new(wem_data), CodebookLibrary::aotuv_codebooks()?)?
								.generate_ogg(&mut ogg_data)?;

							OggToOgg::new_with_defaults().remux(
								&mut Cursor::new(ogg_data),
								&mut BufWriter::new(File::create(path.as_path().context("Invalid path")?)?)
							)?;
						}
					} else {
						if let Some(path) = dialog
							.set_file_name(format!("{}~{}.wem", hash.to_hash(), index))
							.add_filter("WEM file", &["wem"])
							.blocking_save_file()
						{
							fs::write(path.as_path().context("Invalid path")?, wem_data)?;
						}
					}
				}
			}
		}

		ResourceOverviewEvent::ExtractAsHMLanguages => {
			if let Some(game) = app_state.game.load().as_ref() {
				let (res_meta, res_data) = game.extract_latest_resource(hash)?;

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
											let langmap = get_language_map(game.version(), iteration)
												.context("No more alternate language maps available")?;

											let clng =
												hmlanguages::clng::CLNG::new(game.version().into(), langmap.1.to_owned())
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

											if get_language_map(game.version(), iteration).is_none() {
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
											let langmap = get_language_map(game.version(), iteration)
												.context("No more alternate language maps available")?;

											let dlge = hmlanguages::dlge::DLGE::new(
												app_state
													.tonytools_hash_list
													.load()
													.as_ref()
													.context("No TonyTools hash list available")?
													.deref()
													.to_owned(),
												game.version().into(),
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

											if get_language_map(game.version(), iteration).is_none() {
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
											let langmap = get_language_map(game.version(), iteration)
												.context("No more alternate language maps available")?;

											let locr = hmlanguages::locr::LOCR::new(
												app_state
													.tonytools_hash_list
													.load()
													.as_ref()
													.context("No TonyTools hash list available")?
													.deref()
													.to_owned(),
												game.version().into(),
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

											if get_language_map(game.version(), iteration).is_none() {
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
								let rtlv = hmlanguages::rtlv::RTLV::new(game.version().into(), None)
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

		ResourceOverviewEvent::ExtractAsObj => {
			if let Some(game) = app_state.game.load().as_ref() {
				let (_, res_data) = game.extract_latest_resource(hash)?;
				let (obj, _) = parse_prim(game, &res_data)?;

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog.add_filter("OBJ mesh file", &["obj"]).blocking_save_file() {
					fs::write(path.as_path().context("Invalid path")?, obj)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractAsMaterialInstance => {
			if let Some(game) = app_state.game.load().as_ref() {
				let (res_meta, res_data) = game.extract_latest_resource(hash)?;

				let material = MaterialInstance::parse(&res_data, &res_meta.core_info)
					.context("Couldn't parse material instance")?;

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.add_filter("Material JSON", &["material.json"])
					.blocking_save_file()
				{
					fs::write(
						path.as_path().context("Invalid path")?,
						format_json(&to_string(&material)?)?
					)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractAsMaterialEntity => {
			if let Some(game) = app_state.game.load().as_ref() {
				let (matt_meta, matt_data) = game.extract_latest_resource(hash)?;
				let (matb_meta, matb_data) = game.extract_latest_resource(
					matt_meta
						.core_info
						.references
						.get(1)
						.context("No MATB dependency")?
						.resource
				)?;

				let material =
					MaterialEntity::parse(&matt_data, &matt_meta.core_info, &matb_data, &matb_meta.core_info)
						.context("Couldn't parse material entity")?;

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.add_filter("Material entity JSON", &["material.entity.json"])
					.blocking_save_file()
				{
					fs::write(
						path.as_path().context("Invalid path")?,
						format_json(&to_string(&material)?)?
					)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractAsSoundDefs => {
			if let Some(game) = app_state.game.load().as_ref() {
				let (res_meta, res_data) = game.extract_latest_resource(hash)?;

				let sdef = SoundDefinitions::parse(&res_data, &res_meta.core_info, game.version())
					.context("Couldn't parse sound definitions")?;

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.add_filter("Sound definitions JSON", &["sounddefs.json"])
					.blocking_save_file()
				{
					fs::write(
						path.as_path().context("Invalid path")?,
						format_json(&to_string(&sdef)?)?
					)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractAsTexture => {
			if let Some(game) = app_state.game.load().as_ref() {
				let (res_meta, res_data) = game.extract_latest_resource(hash)?;

				let mut texture = TextureMap::process_data(game.version().into(), res_data)
					.context("Couldn't process texture data")?;

				if let Some(texd_depend) = res_meta.core_info.references.first() {
					let (_, texd_data) = game.extract_latest_resource(texd_depend.resource)?;
					let mipblock = MipblockData::from_memory(&texd_data, game.version().into())
						.context("Couldn't process TEXD data")?;
					texture.set_mipblock1(mipblock);
				}

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.set_file_name(format!("{}.texture.dds", hash.to_hash()))
					.add_filter("DDS texture", &["texture.dds"])
					.add_filter("TGA texture", &["texture.tga"])
					.add_filter("PNG texture", &["texture.png"])
					.blocking_save_file()
				{
					let path = path.as_path().context("Invalid path")?;

					app.track_event(
						"Extract texture as image format",
						Some(json!({
							"format": path
								.extension()
								.context("No file extension")?
								.to_str()
								.context("File extension was invalid string")?
						}))
					)
					.unwrap();

					if path
						.file_name()
						.context("No file name")?
						.to_str()
						.context("Filename was invalid string")?
						.ends_with(".dds")
					{
						let dds_data = glacier_texture::convert::create_dds(&texture)
							.context("Couldn't convert texture to DDS")?;

						fs::write(path, dds_data)?;
					} else if path
						.file_name()
						.context("No file name")?
						.to_str()
						.context("Filename was invalid string")?
						.ends_with(".tga")
					{
						let tga_data = glacier_texture::convert::create_tga(&texture)
							.context("Couldn't convert texture to TGA")?;

						fs::write(path, tga_data)?;
					} else {
						let image = glacier_texture::convert::create_dynamic_image(&texture)
							.context("Couldn't convert texture to dynamic image")?;

						image.save(path)?;
					}

					let meta = TextureMetadata {
						text: hash,
						texd: res_meta.core_info.references.first().map(|x| x.resource),
						texture_type: texture.texture_type().into(),
						format: texture.format().into(),
						interpret_as: texture.interpret_as().unwrap_or(InterpretAs::Normal).into()
					};

					fs::write(path.with_extension("json"), format_json(&to_string(&meta)?)?)?;
				}
			}
		}

		ResourceOverviewEvent::ExtractAsPseudocode => {
			if let Some(game) = app_state.game.load().as_ref() {
				let (_, res_data) = game.extract_latest_resource(hash)?;

				let pseudocode = match game.version() {
					GameVersion::H1 => format!(
						"{}\n---\n{:?}",
						hash,
						hitman_behavior::h1::BehaviorTree::from_raw(tokio::task::block_in_place(move || {
							std::thread::Builder::new()
								.stack_size(64 * 1024 * 1024)
								.spawn(move || {
									glacier_bin1::deserialize(&res_data)
										.map_err(|e| format!("{e}"))
										.unwrap()
								})
								.unwrap()
								.join()
								.unwrap()
						}))?
						.root
					),

					GameVersion::H2 => format!(
						"{}\n---\n{:?}",
						hash,
						hitman_behavior::h2::BehaviorTree::from_raw(tokio::task::block_in_place(move || {
							std::thread::Builder::new()
								.stack_size(64 * 1024 * 1024)
								.spawn(move || {
									glacier_bin1::deserialize(&res_data)
										.map_err(|e| format!("{e}"))
										.unwrap()
								})
								.unwrap()
								.join()
								.unwrap()
						}))?
						.root
					),

					GameVersion::H3 => format!(
						"{}\n---\n{:?}",
						hash,
						hitman_behavior::h3::BehaviorTree::from_raw(tokio::task::block_in_place(move || {
							std::thread::Builder::new()
								.stack_size(64 * 1024 * 1024)
								.spawn(move || {
									glacier_bin1::deserialize(&res_data)
										.map_err(|e| format!("{e}"))
										.unwrap()
								})
								.unwrap()
								.join()
								.unwrap()
						}))?
						.root
					)
				};

				let mut dialog = app.dialog().file().set_title("Extract file");

				if let Some(project) = app_state.project.load().as_ref() {
					dialog = dialog.set_directory(&project.path);
				}

				if let Some(path) = dialog
					.add_filter("Behavior tree", &["behavior.txt"])
					.blocking_save_file()
				{
					fs::write(path.as_path().context("Invalid path")?, pseudocode)?;
				}
			}
		}
	}
}
