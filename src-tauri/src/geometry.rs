use std::{
	fs,
	io::{Cursor, Write}
};

use anyhow::{Context, Result, bail};
use ecow::EcoVec;
use fn_error_context::context;
use glacier_commons::{
	game::GlacierGame,
	metadata::{ResourceMetadata, RuntimeID}
};
use glacier_formats::material::{MaterialInstance, MaterialPropertyValue};
use glacier_geometry::render_primitive::{LodLevel, RenderPrimitive};
use glacier_texture::{mipblock::MipblockData, texture_map::TextureMap};
use itertools::Itertools;
use mesh_tools::{
	GltfBuilder, PbrSpecularGlossiness, TextureInfo, Triangle,
	compat::{Point3, Vector2, Vector3},
	texture::TextureFormat
};
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use tryvial::try_fn;

use crate::{HashMap, game::Game};

#[try_fn]
#[context("Couldn't convert PRIM file to OBJ")]
pub fn parse_prim_to_obj(game: &Game, res_data: &[u8]) -> Result<(EcoVec<u8>, [f32; 6])> {
	let model = RenderPrimitive::parse_bytes(
		&mut Cursor::new(res_data),
		match game.version() {
			GlacierGame::H1 => glacier_geometry::WoaVersion::HM2016,
			GlacierGame::H2 => glacier_geometry::WoaVersion::HM2,
			GlacierGame::H3 => glacier_geometry::WoaVersion::HM3,
			_ => bail!("Game version not yet supported")
		}
	)
	.context("Couldn't process PRIM data")?;

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

	for (idx, mesh) in model.iter_primitive_of_lod(LodLevel::LEVEL8).enumerate() {
		writeln!(obj, "o object.{idx:03}")?;

		for position in &mesh.get_positions() {
			writeln!(obj, "v {} {} {} {}", position.x, position.y, position.z, position.w)?;
		}

		for vm in &mesh.get_normals() {
			writeln!(obj, "vn {} {} {}", vm.x, vm.y, vm.z)?;
		}

		for [idx1, idx2, idx3] in mesh.get_indices().as_chunks().0 {
			let idx1 = *idx1 as usize + previous_vertex_count;
			let idx2 = *idx2 as usize + previous_vertex_count;
			let idx3 = *idx3 as usize + previous_vertex_count;
			writeln!(obj, "f {idx1}//{idx1} {idx2}//{idx2} {idx3}//{idx3}")?;
		}

		previous_vertex_count += mesh.get_positions().len();

		let bb = mesh.prim_mesh().calc_bb();

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
#[context("Couldn't convert PRIM file to GLTF")]
pub fn parse_prim_to_glb(game: &Game, res_data: &[u8], res_metadata: &ResourceMetadata) -> Result<(Vec<u8>, [f32; 6])> {
	let model = RenderPrimitive::parse_bytes(
		&mut Cursor::new(res_data),
		match game.version() {
			GlacierGame::H1 => glacier_geometry::WoaVersion::HM2016,
			GlacierGame::H2 => glacier_geometry::WoaVersion::HM2,
			GlacierGame::H3 => glacier_geometry::WoaVersion::HM3,
			_ => bail!("Game version not yet supported")
		}
	)
	.context("Couldn't process PRIM data")?;

	let materials = res_metadata
		.references
		.par_iter()
		.enumerate()
		.filter_map(|(idx, reference)| {
			game.resource_type(reference.resource)
				.is_some_and(|x| x == "MATI")
				.then_some((idx, reference.resource))
		})
		.map(|(idx, res)| {
			let (res_meta, res_data) = game.extract_latest_resource(res)?;
			let material = MaterialInstance::parse(&res_data, &res_meta.core_info)?;
			Ok((idx, material))
		})
		.collect::<Result<HashMap<_, _>>>()?;

	let mut gltf = GltfBuilder::new();

	let gltf_materials = materials
		.into_par_iter()
		.map(|(idx, material)| {
			let friendly_names = if let Some(class) = &material.class {
				let mate_data = game.extract_latest_resource(*class)?.1;

				let mut beginning = mate_data.len() - 1;
				while mate_data[beginning] == 0 || (mate_data[beginning] > 31 && mate_data[beginning] < 127) {
					beginning -= 1;
				}
				beginning += 1;

				String::from_utf8(mate_data[beginning..mate_data.len() - 1].into())?
					.split('\x00')
					.filter(|x| x.trim().len() > 2 && x.trim().as_bytes().iter().all(|x| *x > 31 && *x < 127))
					.map(|x| x.trim().to_owned())
					.tuples()
					.map(|(prop, friendly)| {
						(
							if friendly.starts_with("map") {
								friendly.chars().skip(3).collect()
							} else {
								friendly
							},
							prop
						)
					})
					.collect_vec()
			} else {
				vec![]
			};

			let gltf_material = {
				#[try_fn]
				fn get_texture(game: &Game, id: RuntimeID) -> Result<Option<image_0_24::DynamicImage>> {
					if let Ok((res_meta, res_data)) = game.extract_latest_resource(id) {
						let mut texture = TextureMap::from_memory(&res_data, game.version().into())
							.context("Couldn't process texture data")?;

						if let Some(texd_depend) = res_meta.core_info.references.first() {
							let (_, texd_data) = game.extract_latest_resource(texd_depend.resource)?;
							let mipblock = MipblockData::from_memory(&texd_data, game.version().into())
								.context("Couldn't process TEXD data")?;
							texture.set_mipblock1(mipblock);
						}

						if texture.format() == glacier_texture::enums::RenderFormat::BC5 {
							Some(image_0_24::load_from_memory_with_format(
								&glacier_texture::convert::create_tga(&texture)
									.context("Couldn't convert texture to TGA")?,
								image_0_24::ImageFormat::Tga
							)?)
						} else {
							Some(image_0_24::load_from_memory(&{
								let mut bytes = vec![];
								glacier_texture::convert::create_dynamic_image(&texture)
									.context("Couldn't convert texture to dynamic image")?
									.write_to(Cursor::new(&mut bytes), image::ImageFormat::Png)?;
								bytes
							})?)
						}
					} else {
						None
					}
				}

				let ((diffuse_texture, specular_texture), (normal_texture, emissive_texture)) = rayon::join(
					|| {
						rayon::join(
							|| {
								friendly_names
									.iter()
									.filter_map(|(friendly, prop)| {
										friendly.to_lowercase().contains("diffuse").then_some(prop)
									})
									.filter_map(|prop| material.binder.properties.get(prop))
									.filter_map(|prop| {
										if let MaterialPropertyValue::Texture { enabled, value, .. } = prop
											&& *enabled
										{
											*value
										} else {
											None
										}
									})
									.next()
									.and_then(|id| get_texture(game, id).transpose())
									.transpose()
							},
							|| {
								friendly_names
									.iter()
									.filter_map(|(friendly, prop)| {
										friendly.to_lowercase().contains("spec").then_some(prop)
									})
									.filter_map(|prop| material.binder.properties.get(prop))
									.filter_map(|prop| {
										if let MaterialPropertyValue::Texture { enabled, value, .. } = prop
											&& *enabled
										{
											*value
										} else {
											None
										}
									})
									.next()
									.and_then(|id| get_texture(game, id).transpose())
									.transpose()
							}
						)
					},
					|| {
						rayon::join(
							|| {
								friendly_names
									.iter()
									.filter_map(|(friendly, prop)| {
										friendly.to_lowercase().contains("normal").then_some(prop)
									})
									.chain(material.binder.properties.iter().filter_map(|(name, prop)| {
										(name.to_lowercase().contains("normal")
											&& matches!(prop, MaterialPropertyValue::Texture { enabled: true, .. }))
										.then_some(name)
									}))
									.filter_map(|prop| material.binder.properties.get(prop))
									.filter_map(|prop| {
										if let MaterialPropertyValue::Texture { enabled, value, .. } = prop
											&& *enabled
										{
											*value
										} else {
											None
										}
									})
									.next()
									.and_then(|id| get_texture(game, id).transpose())
									.transpose()
							},
							|| {
								friendly_names
									.iter()
									.filter_map(|(friendly, prop)| {
										friendly.to_lowercase().contains("emis").then_some(prop)
									})
									.filter_map(|prop| material.binder.properties.get(prop))
									.filter_map(|prop| {
										if let MaterialPropertyValue::Texture { enabled, value, .. } = prop
											&& *enabled
										{
											*value
										} else {
											None
										}
									})
									.next()
									.and_then(|id| get_texture(game, id).transpose())
							}
						)
					}
				);

				let emissive_factor = if emissive_texture.is_some() {
					friendly_names
						.iter()
						.filter_map(|(friendly, prop)| {
							(friendly.to_lowercase().contains("emis") || friendly.to_lowercase().contains("intensity"))
								.then_some(prop)
						})
						.filter_map(|prop| material.binder.properties.get(prop))
						.filter_map(|prop| {
							if let MaterialPropertyValue::Float { enabled, value, .. } = prop
								&& *enabled
							{
								Some([*value, *value, *value])
							} else if let MaterialPropertyValue::Vector { enabled, value, .. } = prop
								&& *enabled
							{
								(value.len() == 3).then(|| [value[0], value[1], value[2]])
							} else {
								None
							}
						})
						.next()
				} else {
					None
				};

				(
					Some(format!("material_{idx}")),
					diffuse_texture?,
					specular_texture?,
					normal_texture?,
					emissive_texture.transpose()?,
					emissive_factor,
					if material.binder.render_state.blend_enabled.unwrap_or(false) {
						Some("BLEND".into())
					} else if material.class_flags.alpha {
						Some("MASK".into())
					} else {
						None
					},
					material
						.binder
						.render_state
						.alpha_test_enabled
						.unwrap_or(false)
						.then(|| (1 - material.binder.render_state.alpha_reference.unwrap_or(255)) as f32 / 255.0)
				)
			};

			Ok((idx, gltf_material))
		})
		.collect::<Result<Vec<_>>>()?
		.into_iter()
		.map(
			|(
				idx,
				(
					name,
					diffuse_texture,
					specular_texture,
					normal_texture,
					emissive_texture,
					emissive_factor,
					alpha_mode,
					alpha_cutoff
				)
			)| {
				Ok((idx, {
					let diffuse_texture = diffuse_texture
						.map(|image| gltf.create_texture_from_image(None, &image, TextureFormat::PNG))
						.transpose()?;

					let specular_texture = specular_texture
						.map(|image| gltf.create_texture_from_image(None, &image, TextureFormat::PNG))
						.transpose()?;

					let normal_texture = normal_texture
						.map(|image| gltf.create_texture_from_image(None, &image, TextureFormat::PNG))
						.transpose()?;

					let emissive_texture = emissive_texture
						.map(|image| gltf.create_texture_from_image(None, &image, TextureFormat::PNG))
						.transpose()?;

					let index = gltf.add_textured_material(
						name,
						diffuse_texture,
						None,
						normal_texture,
						None,
						emissive_texture,
						emissive_factor,
						None,
						None,
						alpha_mode,
						alpha_cutoff,
						None
					);

					if let Some(specular_texture) = specular_texture {
						gltf.gltf
							.materials
							.get_or_insert_default()
							.get_mut(index)
							.unwrap()
							.extensions
							.get_or_insert_default()
							.pbr_specular_glossiness = Some(PbrSpecularGlossiness {
							diffuse_texture: diffuse_texture.map(|diffuse_texture| TextureInfo {
								index: diffuse_texture,
								..Default::default()
							}),
							specular_glossiness_texture: Some(TextureInfo {
								index: specular_texture,
								..Default::default()
							}),
							..Default::default()
						});
					}

					index
				}))
			}
		)
		.collect::<Result<HashMap<_, _>>>()?;

	gltf.gltf
		.extensions_used
		.get_or_insert_default()
		.push("KHR_materials_pbrSpecularGlossiness".into());

	let mut bounding_box: [f32; 6] = [
		f32::INFINITY,
		f32::INFINITY,
		f32::INFINITY,
		f32::NEG_INFINITY,
		f32::NEG_INFINITY,
		f32::NEG_INFINITY
	];

	let nodes = model
		.iter_primitive_of_lod(LodLevel::LEVEL8)
		.enumerate()
		.map(|(idx, mesh)| {
			let bb = mesh.prim_mesh().calc_bb();

			bounding_box[0] = bounding_box[0].min(bb.min.x);
			bounding_box[1] = bounding_box[1].min(bb.min.y);
			bounding_box[2] = bounding_box[2].min(bb.min.z);

			bounding_box[3] = bounding_box[3].max(bb.max.x);
			bounding_box[4] = bounding_box[4].max(bb.max.y);
			bounding_box[5] = bounding_box[5].max(bb.max.z);

			let gltf_mesh = gltf.create_custom_mesh(
				Some(format!("mesh_{idx}")),
				&mesh
					.get_positions()
					.into_iter()
					.map(|pos| Point3 {
						x: pos.x,
						y: pos.z,
						z: -pos.y
					})
					.collect_vec(),
				&mesh
					.get_indices()
					.as_chunks()
					.0
					.iter()
					.map(|[a, b, c]| Triangle {
						a: *a as u32,
						b: *b as u32,
						c: *c as u32
					})
					.collect_vec(),
				Some(
					mesh.get_normals()
						.into_iter()
						.map(|n| Vector3 {
							x: n.x,
							y: n.z,
							z: -n.y
						})
						.collect_vec()
				),
				Some(
					mesh.get_tex_coords()
						.into_iter()
						.map(|x| x.into_iter().map(|c| Vector2 { x: c.x, y: c.y }).collect())
						.collect()
				),
				gltf_materials
					.get(&(mesh.prim_mesh().prim_object.material_id as usize))
					.copied()
			);

			gltf.add_node(Some(format!("node_{idx}")), Some(gltf_mesh), None, None, None)
		})
		.collect_vec();

	gltf.add_scene(None, Some(nodes));

	let output = tempfile::tempdir()?;
	gltf.export_glb(
		output
			.path()
			.join("model.gltf")
			.to_str()
			.context("Temp path is not valid UTF-8")?
	)?;
	(fs::read(output.path().join("model.gltf"))?, bounding_box)
}
