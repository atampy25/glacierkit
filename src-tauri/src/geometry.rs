use std::{
	fs,
	io::{Cursor, Write},
	sync::Arc
};

use anyhow::{Context, Result, bail};
use ecow::EcoString;
use fn_error_context::context;
use glacier_commons::{
	game::GlacierGame,
	metadata::{ResourceMetadata, RuntimeID}
};
use glacier_formats::material::{MaterialInstance, MaterialPropertyValue};
use glacier_geometry::render_primitive::{LodLevel, RenderPrimitive};
use glacier_texture::{mipblock::MipblockData, texture_map::TextureMap};
use glam::{Affine3, Vec3};
use itertools::Itertools;
use mesh_tools::{
	GltfBuilder, PbrSpecularGlossiness, TextureInfo, Triangle,
	compat::{Point3, Vector2, Vector3},
	texture::TextureFormat
};
use quickentity_rs::{
	entity::{Entity, EntityID, Ref},
	variant::{RawVariant, Transform, Variant}
};
use rayon::iter::{
	IndexedParallelIterator, IntoParallelIterator, IntoParallelRefIterator, ParallelBridge, ParallelIterator
};
use serde::{Deserialize, Serialize};
use slotmap::SlotMap;
use tryvial::try_fn;

use crate::{
	HashMap, HashSet,
	game::{Game, GameFiles}
};

#[try_fn]
#[context("Couldn't convert PRIM file to OBJ")]
pub fn parse_prim_to_obj(game: &Game, res_data: &[u8]) -> Result<(Vec<u8>, [f32; 6])> {
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

	let mut obj = vec![];

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
										(friendly.to_lowercase().contains("diffuse")
											|| friendly.to_lowercase().contains("basecolor"))
										.then_some(prop)
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
						.then(|| material.binder.render_state.alpha_reference.unwrap_or(255) as f32 / 255.0)
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

slotmap::new_key_type! { pub struct InstantiatedEntityID; }

#[derive(Debug, Clone)]
pub struct InstantiatedScene {
	pub root_entity: InstantiatedEntityID,
	pub entities: SlotMap<InstantiatedEntityID, InstantiatedEntity>
}

#[derive(Debug, Clone, PartialEq)]
pub struct InstantiatedEntity {
	pub source: (RuntimeID, EntityID),
	pub parent: Option<InstantiatedEntityRef>,
	pub is_parent_factory: bool,
	pub factory: InstantiatedEntityFactory,
	pub properties: HashMap<EcoString, PropertySource>,
	pub ref_properties: HashMap<EcoString, Vec<Option<InstantiatedEntityRef>>>,
	pub exposed_entities: HashMap<EcoString, Vec<InstantiatedEntityRef>>
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InstantiatedEntityFactory {
	Factory(RuntimeID),
	Factories(Vec<InstantiatedEntityID>)
}

#[derive(Debug, Clone, PartialEq)]
pub enum InstantiatedEntityRef {
	Local {
		entity: InstantiatedEntityID,
		exposed_entity: Option<EcoString>
	},
	External {
		external_scene: RuntimeID,
		entity_id: EntityID,
		exposed_entity: Option<EcoString>
	}
}

#[derive(Debug, Clone, PartialEq)]
pub struct PropertySource {
	pub entity: InstantiatedEntityID,
	pub property: EcoString
}

#[try_fn]
#[context("Couldn't instantiate scene")]
pub fn instantiate_scene(game: &impl GameFiles, entity: &Entity) -> Result<InstantiatedScene> {
	let mut entities = SlotMap::default();

	#[try_fn]
	fn process_entity(
		game: &impl GameFiles,
		entity: &Entity,
		entities: &mut SlotMap<InstantiatedEntityID, InstantiatedEntity>
	) -> Result<InstantiatedEntityID> {
		let mut entity_instantiations = HashMap::new();

		let mut exclude = HashSet::default();
		for (id, sub_entity) in &entity.sub_entities {
			if sub_entity.editor_only {
				exclude.insert(*id);

				fn remove_children(exclude: &mut HashSet<EntityID>, entity: &Entity, parent: EntityID) {
					for (id, sub_entity) in &entity.sub_entities {
						if sub_entity.parent.as_ref().is_some_and(|x| x.as_local() == Some(parent)) {
							exclude.insert(*id);
							remove_children(exclude, entity, *id);
						}
					}
				}

				remove_children(&mut exclude, entity, *id);
			}
		}

		for (id, sub_entity) in entity.sub_entities.iter().filter(|(id, _)| !exclude.contains(id)) {
			let factories = if let Some(ty) = game.resource_type(sub_entity.factory.resource)
				&& ty == "ASET"
			{
				game.extract_latest_metadata(sub_entity.factory.resource)?
					.core_info
					.references
					.into_iter()
					.rev()
					.skip(1)
					.rev()
					.map(|x| x.resource)
					.collect_vec()
			} else {
				vec![sub_entity.factory.resource]
			};

			if factories.len() == 1 && !game.resource_type(factories[0]).is_some_and(|x| x == "TEMP") {
				let instance = entities.insert(InstantiatedEntity {
					source: (entity.factory, *id),
					parent: None,
					factory: InstantiatedEntityFactory::Factory(factories[0]),
					is_parent_factory: false,
					properties: Default::default(),
					ref_properties: Default::default(),
					exposed_entities: Default::default()
				});

				entity_instantiations.insert(*id, instance);

				entities[instance]
					.properties
					.extend(sub_entity.properties.keys().map(|prop| {
						(
							prop.to_owned(),
							PropertySource {
								entity: instance,
								property: prop.to_owned()
							}
						)
					}));
			} else {
				let instance = entities.insert(InstantiatedEntity {
					source: (entity.factory, *id),
					parent: None,
					factory: InstantiatedEntityFactory::Factories(vec![]),
					is_parent_factory: false,
					properties: Default::default(),
					ref_properties: Default::default(),
					exposed_entities: Default::default()
				});

				entity_instantiations.insert(*id, instance);

				entities[instance]
					.properties
					.extend(sub_entity.properties.keys().map(|prop| {
						(
							prop.to_owned(),
							PropertySource {
								entity: instance,
								property: prop.to_owned()
							}
						)
					}));

				for factory in factories {
					if game.resource_type(factory).is_some_and(|x| x == "TEMP") {
						let new_added = process_entity(game, &*game.extract_entity(factory)?, entities)?;

						entities[new_added].parent = Some(InstantiatedEntityRef::Local {
							entity: instance,
							exposed_entity: None
						});

						entities[new_added].is_parent_factory = true;

						let InstantiatedEntityFactory::Factories(facs) = &mut entities[instance].factory else {
							unreachable!();
						};

						facs.push(new_added);

						entities[instance].exposed_entities = entities[new_added].exposed_entities.to_owned();
					} else {
						let new_added = entities.insert(InstantiatedEntity {
							source: (entity.factory, *id),
							parent: Some(InstantiatedEntityRef::Local {
								entity: instance,
								exposed_entity: None
							}),
							factory: InstantiatedEntityFactory::Factory(factory),
							is_parent_factory: true,
							properties: sub_entity
								.properties
								.keys()
								.map(|prop| {
									(
										prop.to_owned(),
										PropertySource {
											entity: instance,
											property: prop.to_owned()
										}
									)
								})
								.collect(),
							ref_properties: Default::default(),
							exposed_entities: Default::default()
						});

						let InstantiatedEntityFactory::Factories(facs) = &mut entities[instance].factory else {
							unreachable!();
						};

						facs.push(new_added);
					}
				}
			}
		}

		for (id, sub_entity) in entity.sub_entities.iter().filter(|(id, _)| !exclude.contains(id)) {
			let instance = entity_instantiations[id];

			entities[instance].parent = sub_entity.parent.as_ref().map(|parent| {
				if let Some(external_scene) = parent.external_scene {
					InstantiatedEntityRef::External {
						external_scene,
						entity_id: parent.entity_id,
						exposed_entity: parent.exposed_entity.to_owned()
					}
				} else {
					InstantiatedEntityRef::Local {
						entity: entity_instantiations[&parent.entity_id],
						exposed_entity: parent.exposed_entity.to_owned()
					}
				}
			});

			entities[instance]
				.ref_properties
				.extend(sub_entity.properties.iter().filter_map(|(prop, val)| {
					if let Variant::Ref(reference) = &val.value {
						Some((
							prop.to_owned(),
							vec![if let Some(reference) = reference {
								if let Some(external_scene) = reference.external_scene {
									Some(InstantiatedEntityRef::External {
										external_scene,
										entity_id: reference.entity_id,
										exposed_entity: reference.exposed_entity.to_owned()
									})
								} else {
									entity_instantiations.get(&reference.entity_id).copied().map(|entity| {
										InstantiatedEntityRef::Local {
											entity,
											exposed_entity: reference.exposed_entity.to_owned()
										}
									})
								}
							} else {
								None
							}]
						))
					} else if let Variant::Array(ty, vals) = &val.value
						&& ty == "SEntityTemplateReference"
					{
						Some((
							prop.to_owned(),
							vals.iter()
								.map(|reference| {
									let Variant::Ref(reference) = reference else {
										unreachable!();
									};

									if let Some(reference) = reference {
										if let Some(external_scene) = reference.external_scene {
											Some(InstantiatedEntityRef::External {
												external_scene,
												entity_id: reference.entity_id,
												exposed_entity: reference.exposed_entity.to_owned()
											})
										} else {
											entity_instantiations.get(&reference.entity_id).copied().map(|entity| {
												InstantiatedEntityRef::Local {
													entity,
													exposed_entity: reference.exposed_entity.to_owned()
												}
											})
										}
									} else {
										None
									}
								})
								.collect()
						))
					} else {
						None
					}
				}));

			for (alias_name, aliases) in &sub_entity.property_aliases {
				for alias in aliases {
					if let Some(original) = entity_instantiations.get(&alias.original_entity).copied() {
						entities[original].properties.insert(
							alias.original_property.to_owned(),
							PropertySource {
								entity: instance,
								property: alias_name.to_owned()
							}
						);
					}
				}
			}

			// Treat events like property aliases if the property is known
			// Allows for better handling of LampCore
			for (event_name, connections) in sub_entity
				.events
				.iter()
				.filter(|&(x, _)| x == "DiffuseColor" || x == "DiffusePower" || x == "GlowPower")
			{
				for (input, targets) in connections {
					for target in targets {
						if let Some(local) = target.entity_ref.as_local()
							&& let Some(original) = entity_instantiations.get(&local).copied()
						{
							entities[original].properties.insert(
								input.to_owned(),
								PropertySource {
									entity: instance,
									property: event_name.to_owned()
								}
							);
						}
					}
				}
			}

			for (name, exposed_entity) in &sub_entity.exposed_entities {
				entities[instance].exposed_entities.insert(
					name.to_owned(),
					exposed_entity
						.refers_to
						.iter()
						.filter_map(|entity_ref| {
							if let Some(external_scene) = entity_ref.external_scene {
								Some(InstantiatedEntityRef::External {
									external_scene,
									entity_id: entity_ref.entity_id,
									exposed_entity: entity_ref.exposed_entity.to_owned()
								})
							} else {
								entity_instantiations.get(&entity_ref.entity_id).copied().map(|entity| {
									InstantiatedEntityRef::Local {
										entity,
										exposed_entity: entity_ref.exposed_entity.to_owned()
									}
								})
							}
						})
						.collect()
				);
			}
		}

		entity_instantiations[&entity.root_entity]
	}

	let root_entity = process_entity(game, entity, &mut entities)?;

	InstantiatedScene { root_entity, entities }
}

#[derive(Debug, Clone)]
pub struct InstantiatedScenes {
	pub scenes: HashMap<RuntimeID, (Arc<Entity>, InstantiatedScene)>,
	pub property_overrides: HashMap<(RuntimeID, InstantiatedEntityID, EcoString), Variant>
}

#[try_fn]
#[context("Couldn't resolve reference")]
pub fn resolve_instantiated_ref(
	scenes: &HashMap<RuntimeID, (Arc<Entity>, InstantiatedScene)>,
	scene_id: RuntimeID,
	reference: InstantiatedEntityRef
) -> Result<Vec<(RuntimeID, InstantiatedEntityID)>> {
	let mut check = vec![(scene_id, reference)];

	while check.iter().any(|check| {
		matches!(
			check,
			(
				_,
				InstantiatedEntityRef::Local {
					exposed_entity: Some(_),
					..
				}
			) | (_, InstantiatedEntityRef::External { .. })
		)
	}) {
		let mut new = vec![];
		for check in check {
			if let (
				_,
				InstantiatedEntityRef::External {
					external_scene,
					entity_id,
					exposed_entity
				}
			) = check
			{
				let Some((_, scene)) = scenes.get(&external_scene) else {
					continue;
				};

				let Some(entity) = scene
					.entities
					.iter()
					.find_map(|(id, x)| (x.source.1 == entity_id && !x.is_parent_factory).then_some(id))
				else {
					bail!("Couldn't find referenced entity");
				};

				new.push((external_scene, InstantiatedEntityRef::Local { entity, exposed_entity }));
			} else if let (
				scene_id,
				InstantiatedEntityRef::Local {
					entity,
					exposed_entity: Some(exposed_entity)
				}
			) = check
			{
				let Some((_, scene)) = scenes.get(&scene_id) else {
					continue;
				};

				new.extend(
					scene.entities[entity]
						.exposed_entities
						.get(&exposed_entity)
						.unwrap_or(&vec![])
						.iter()
						.map(|x| (scene_id, x.to_owned()))
				);
			}
		}
		check = new;
	}

	check
		.into_iter()
		.map(|(id, reference)| {
			let InstantiatedEntityRef::Local { entity, .. } = reference else {
				unreachable!()
			};

			(id, entity)
		})
		.collect()
}

#[try_fn]
#[context("Couldn't resolve reference")]
pub fn resolve_ref(
	scenes: &HashMap<RuntimeID, (Arc<Entity>, InstantiatedScene)>,
	scene_id: RuntimeID,
	reference: &Ref
) -> Result<Vec<(RuntimeID, InstantiatedEntityID)>> {
	if let Some(external_scene) = reference.external_scene {
		resolve_instantiated_ref(
			scenes,
			scene_id,
			InstantiatedEntityRef::External {
				entity_id: reference.entity_id,
				external_scene,
				exposed_entity: reference.exposed_entity.to_owned()
			}
		)?
	} else {
		let Some((_, scene)) = scenes.get(&scene_id) else {
			bail!("Couldn't find scene for reference");
		};

		let Some(entity) = scene
			.entities
			.iter()
			.find_map(|(id, x)| (x.source.1 == reference.entity_id && !x.is_parent_factory).then_some(id))
		else {
			bail!("Couldn't find referenced entity");
		};

		resolve_instantiated_ref(
			scenes,
			scene_id,
			InstantiatedEntityRef::Local {
				entity,
				exposed_entity: reference.exposed_entity.to_owned()
			}
		)?
	}
}

#[try_fn]
#[context("Couldn't instantiate scenes")]
pub fn instantiate_scenes(game: &impl GameFiles, scenes: &[RuntimeID]) -> Result<InstantiatedScenes> {
	let mut scenes = scenes
		.par_iter()
		.map(|&id| {
			Ok((id, {
				let entity = game.extract_entity(id)?;
				let scene = instantiate_scene(game, &entity)?;
				(entity, scene)
			}))
		})
		.collect::<Result<HashMap<_, _>>>()?;

	// Override deletes
	{
		let override_deletes = scenes
			.iter()
			.flat_map(|(id, (entity, _))| entity.override_deletes.iter().map(|x| (*id, x.to_owned())))
			.collect_vec();

		for (scene_id, override_delete) in override_deletes {
			let override_delete = resolve_ref(&scenes, scene_id, &override_delete)
				.context("Couldn't resolve reference for override delete")?;

			for (scene_id, entity) in override_delete {
				let Some((_, scene)) = scenes.get_mut(&scene_id) else {
					continue;
				};

				let mut reverse_parents = scene
					.entities
					.iter()
					.filter_map(|(id, entity)| {
						if let Some(InstantiatedEntityRef::Local { entity: parent, .. }) = &entity.parent {
							Some((*parent, id))
						} else {
							None
						}
					})
					.into_group_map();

				// Delete entity and all its children
				let mut to_delete = vec![entity];
				while let Some(entity) = to_delete.pop() {
					if let Some(children) = reverse_parents.remove(&entity) {
						to_delete.extend(children);
					}

					scene.entities.remove(entity);
				}
			}
		}
	}

	// Property overrides
	let mut property_overrides = HashMap::default();
	{
		let scene_property_overrides = scenes
			.iter()
			.flat_map(|(id, (entity, _))| entity.property_overrides.iter().map(|x| (*id, x.to_owned())))
			.collect_vec();

		for (scene_id, property_override) in scene_property_overrides {
			for overriden_entity in property_override.entities {
				let overridden_entity = resolve_ref(&scenes, scene_id, &overriden_entity)
					.context("Couldn't resolve reference for property override")?;

				for (scene_id, overridden_entity) in overridden_entity {
					for (overridden_property, val) in &property_override.properties {
						property_overrides.insert(
							(scene_id, overridden_entity, overridden_property.to_owned()),
							val.to_owned()
						);
					}
				}
			}
		}
	}

	InstantiatedScenes {
		scenes,
		property_overrides
	}
}

slotmap::new_key_type! { pub struct GeomEntityID; }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SceneGeometry {
	pub geometry: SlotMap<GeomEntityID, GeomEntity>,
	pub global_material_overrides: HashMap<RuntimeID, (RuntimeID, HashMap<EcoString, Variant>)>
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct GeomEntity {
	/// The instantiated entity which this geometry entity was created from.
	pub source: (RuntimeID, InstantiatedEntityID),

	/// Transform parent of the entity. NOT the logical parent.
	pub parent: Option<GeomEntityID>,

	/// Transform relative to parent (or world if no parent).
	pub transform: Affine3,

	#[serde(flatten)]
	pub data: GeomEntityData
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type")]
pub enum GeomEntityData {
	Spatial,

	Geometry {
		/// Primitive scale.
		scale: Vec3,

		/// PRIM resource containing geometry for this entity.
		prim: RuntimeID,

		/// Overridden materials and properties, set by material entities and overwrite aspects.
		/// TODO: Consider operations, Enab
		material_overrides: HashMap<RuntimeID, (RuntimeID, HashMap<EcoString, Variant>)>
	},

	Light {
		diffuse_color: Vec3,

		diffuse_power: f32,

		cast_shadows: bool,

		light_kind: LightKind
	}
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type")]
pub enum LightKind {
	Point,
	Spot { inner_angle: f32, outer_angle: f32 },
	Directional,
	Ambient
}

pub struct RenderSettings {
	/// Whether to add light entities.
	pub lighting: bool,

	/// LOD level to use for geometry.
	pub lod: LodLevel
}

impl Default for RenderSettings {
	fn default() -> Self {
		RenderSettings {
			lighting: false,
			lod: LodLevel::LEVEL7
		}
	}
}

impl Clone for RenderSettings {
	fn clone(&self) -> Self {
		Self {
			lighting: self.lighting,
			lod: match self.lod {
				LodLevel::LEVEL1 => LodLevel::LEVEL1,
				LodLevel::LEVEL2 => LodLevel::LEVEL2,
				LodLevel::LEVEL3 => LodLevel::LEVEL3,
				LodLevel::LEVEL4 => LodLevel::LEVEL4,
				LodLevel::LEVEL5 => LodLevel::LEVEL5,
				LodLevel::LEVEL6 => LodLevel::LEVEL6,
				LodLevel::LEVEL7 => LodLevel::LEVEL7,
				LodLevel::LEVEL8 => LodLevel::LEVEL8
			}
		}
	}
}

#[try_fn]
#[context("Couldn't get scene geometry")]
pub fn get_scene_geometry(
	game: &impl GameFiles,
	scenes: &InstantiatedScenes,
	settings: RenderSettings
) -> Result<SceneGeometry> {
	#[try_fn]
	fn get_property_value(
		game: &impl GameFiles,
		scenes: &InstantiatedScenes,
		scene_id: RuntimeID,
		scene: &InstantiatedScene,
		no_traverse: Option<InstantiatedEntityID>,
		entity: InstantiatedEntityID,
		property: &EcoString
	) -> Result<Option<Variant>> {
		if let Some(override_val) = scenes.property_overrides.get(&(scene_id, entity, property.into())) {
			return Ok(Some(override_val.to_owned()));
		}

		let instance = &scene.entities[entity];

		let mut value = None;

		// If this entity is an expansion of a parent, the parent takes precedence
		if instance.is_parent_factory {
			let Some(InstantiatedEntityRef::Local {
				entity: parent_entity, ..
			}) = &instance.parent
			else {
				unreachable!();
			};

			if !no_traverse.is_some_and(|x| x == *parent_entity) {
				value = get_property_value(game, scenes, scene_id, scene, Some(entity), *parent_entity, property)?;
			}
		}

		// Otherwise there may be a property alias from elsewhere
		if value.is_none()
			&& let Some(source) = instance.properties.get(property)
			&& !(source.entity == entity && source.property == *property)
			&& !no_traverse.is_some_and(|x| x == source.entity)
		{
			value = get_property_value(
				game,
				scenes,
				scene_id,
				scene,
				Some(entity),
				source.entity,
				&source.property
			)?;
		}

		// Otherwise use the entity's own property value
		if value.is_none()
			&& let Some(val) = game.extract_entity(instance.source.0)?.sub_entities[&instance.source.1]
				.properties
				.get(property)
		{
			value = Some(val.value.to_owned());
		}

		// Finally, try child expansions of this entity
		if value.is_none()
			&& let InstantiatedEntityFactory::Factories(children) = &instance.factory
		{
			for child in children {
				if value.is_none() && !no_traverse.is_some_and(|x| x == *child) {
					value = get_property_value(game, scenes, scene_id, scene, Some(entity), *child, &property)?;
				}
			}
		}

		value
	}

	#[try_fn]
	fn get_ref_property_value(
		scenes: &InstantiatedScenes,
		scene_id: RuntimeID,
		scene: &InstantiatedScene,
		no_traverse: Option<InstantiatedEntityID>,
		entity: InstantiatedEntityID,
		property: &EcoString
	) -> Result<Option<Option<(RuntimeID, InstantiatedEntityID)>>> {
		if let Some(override_val) = scenes.property_overrides.get(&(scene_id, entity, property.into())) {
			let Variant::Ref(reference) = override_val else {
				bail!("Value should be a reference");
			};

			if let Some(reference) = reference {
				return Ok(Some(resolve_ref(&scenes.scenes, scene_id, reference)?.first().copied()));
			} else {
				return Ok(Some(None));
			}
		}

		let instance = &scene.entities[entity];

		let mut value = None;

		// If this entity is an expansion of a parent, the parent takes precedence
		if instance.is_parent_factory {
			let Some(InstantiatedEntityRef::Local {
				entity: parent_entity, ..
			}) = &instance.parent
			else {
				unreachable!();
			};

			if !no_traverse.is_some_and(|x| x == *parent_entity) {
				value = get_ref_property_value(scenes, scene_id, scene, Some(entity), *parent_entity, property)?;
			}
		}

		// Otherwise there may be a property alias from elsewhere
		if value.is_none()
			&& let Some(source) = instance.properties.get(property)
			&& !(source.entity == entity && source.property == *property)
			&& !no_traverse.is_some_and(|x| x == source.entity)
		{
			value = get_ref_property_value(scenes, scene_id, scene, Some(entity), source.entity, &source.property)?;
		}

		// Otherwise use the entity's own property value
		if value.is_none()
			&& let Some(val) = instance.ref_properties.get(property)
		{
			if let Some(reference) = val.first().context("Value should be a reference")?.to_owned() {
				value = Some(
					resolve_instantiated_ref(&scenes.scenes, scene_id, reference)?
						.first()
						.copied()
				);
			} else {
				value = Some(None);
			}
		}

		// Finally, try child expansions of this entity
		if value.is_none()
			&& let InstantiatedEntityFactory::Factories(children) = &instance.factory
		{
			for child in children {
				if value.is_none() && !no_traverse.is_some_and(|x| x == *child) {
					value = get_ref_property_value(scenes, scene_id, scene, Some(entity), *child, &property)?;
				}
			}
		}

		value
	}

	#[try_fn]
	fn get_ref_array_property_value(
		scenes: &InstantiatedScenes,
		scene_id: RuntimeID,
		scene: &InstantiatedScene,
		no_traverse: Option<InstantiatedEntityID>,
		entity: InstantiatedEntityID,
		property: &EcoString
	) -> Result<Option<Vec<Option<(RuntimeID, InstantiatedEntityID)>>>> {
		if let Some(override_val) = scenes.property_overrides.get(&(scene_id, entity, property.into())) {
			let Variant::Array(_, items) = override_val else {
				bail!("Value should be an array of references");
			};

			return Ok(Some(
				items
					.iter()
					.map(|item| {
						let Variant::Ref(reference) = item else {
							bail!("Value should be an array of references");
						};

						reference
							.as_ref()
							.map(|reference| resolve_ref(&scenes.scenes, scene_id, reference))
							.transpose()
							.map(|x| match x {
								None => vec![None],
								Some(x) => x.into_iter().map(Some).collect()
							})
					})
					.flatten_ok()
					.collect::<Result<_>>()?
			));
		}

		let instance = &scene.entities[entity];

		let mut value = None;

		// If this entity is an expansion of a parent, the parent takes precedence
		if instance.is_parent_factory {
			let Some(InstantiatedEntityRef::Local {
				entity: parent_entity, ..
			}) = &instance.parent
			else {
				unreachable!();
			};

			if !no_traverse.is_some_and(|x| x == *parent_entity) {
				value = get_ref_array_property_value(scenes, scene_id, scene, Some(entity), *parent_entity, property)?;
			}
		}

		// Otherwise there may be a property alias from elsewhere
		if value.is_none()
			&& let Some(source) = instance.properties.get(property)
			&& !(source.entity == entity && source.property == *property)
			&& !no_traverse.is_some_and(|x| x == source.entity)
		{
			value =
				get_ref_array_property_value(scenes, scene_id, scene, Some(entity), source.entity, &source.property)?;
		}

		// Otherwise use the entity's own property value
		if value.is_none()
			&& let Some(val) = instance.ref_properties.get(property)
		{
			value = Some(
				val.iter()
					.map(|reference| {
						reference
							.to_owned()
							.map(|reference| resolve_instantiated_ref(&scenes.scenes, scene_id, reference))
							.transpose()
							.map(|x| match x {
								None => vec![None],
								Some(x) => x.into_iter().map(Some).collect()
							})
					})
					.flatten_ok()
					.collect::<Result<_>>()?
			);
		}

		// Finally, try child expansions of this entity
		if value.is_none()
			&& let InstantiatedEntityFactory::Factories(children) = &instance.factory
		{
			for child in children {
				if value.is_none() && !no_traverse.is_some_and(|x| x == *child) {
					value = get_ref_array_property_value(scenes, scene_id, scene, Some(entity), *child, &property)?;
				}
			}
		}

		value
	}

	let mut geometry = SlotMap::default();
	let mut ids: HashMap<(RuntimeID, InstantiatedEntityID), GeomEntityID> = HashMap::default();

	let mut global_material_overrides: HashMap<RuntimeID, (RuntimeID, HashMap<EcoString, Variant>)> =
		HashMap::default();

	let matt_props = game
		.intellisense()
		.get_cppt_properties(game, crate::class_for_game!(game, "zrendermaterialentity"))?
		.into_iter()
		.map(|(x, _)| x)
		.collect::<HashSet<_>>();

	for (scene_id, (_, scene)) in &scenes.scenes {
		for (id, instance) in &scene.entities {
			if let InstantiatedEntityFactory::Factory(factory) = instance.factory
				&& game.resource_type(factory).is_some_and(|ty| ty == "MATT")
				&& get_property_value(game, scenes, *scene_id, scene, None, id, &"Global".into())?
					.map(|val| {
						if let Variant::Raw(val) = val
							&& val.variant_type() == "bool"
						{
							val.to_serde().unwrap().as_bool().unwrap()
						} else {
							false
						}
					})
					.unwrap_or(false)
				&& get_property_value(game, scenes, *scene_id, scene, None, id, &"Active".into())?
					.map(|val| {
						if let Variant::Raw(val) = val
							&& val.variant_type() == "bool"
						{
							val.to_serde().unwrap().as_bool().unwrap()
						} else {
							true
						}
					})
					.unwrap_or(true)
			{
				let this_material = game
					.extract_latest_metadata(factory)?
					.core_info
					.references
					.iter()
					.find_map(|x| {
						game.resource_type(x.resource)
							.is_some_and(|ty| ty == "MATI")
							.then_some(x.resource)
					})
					.context("No MATI present on MATT")?;

				let overridden_material =
					if get_property_value(game, scenes, *scene_id, scene, None, id, &"Override".into())?
						.map(|val| {
							if let Variant::Raw(val) = val
								&& val.variant_type() == "bool"
							{
								val.to_serde().unwrap().as_bool().unwrap()
							} else {
								false
							}
						})
						.unwrap_or(false) && let Some(dest) = get_property_value(
						game,
						scenes,
						*scene_id,
						scene,
						None,
						id,
						&"MaterialInstanceOverrideDest".into()
					)?
					.and_then(|val| if let Variant::Resource(_, val) = val { val } else { None })
					{
						dest.resource
					} else {
						this_material
					};

				let overrides = &mut global_material_overrides
					.entry(overridden_material)
					.or_insert_with(|| (this_material, Default::default()))
					.1;

				for property in instance.properties.keys().filter(|x| !matt_props.contains(*x)) {
					if let Some(value) = get_property_value(game, scenes, *scene_id, scene, None, id, property)? {
						overrides.insert(property.to_owned(), value);
					}
				}
			}
		}
	}

	#[try_fn]
	fn process_entity(
		game: &impl GameFiles,
		scenes: &InstantiatedScenes,
		settings: &RenderSettings,
		scene_id: RuntimeID,
		scene: &InstantiatedScene,
		id: InstantiatedEntityID,
		instance: &InstantiatedEntity,
		ids: &mut HashMap<(RuntimeID, InstantiatedEntityID), GeomEntityID>,
		geometry: &mut SlotMap<GeomEntityID, GeomEntity>
	) -> Result<GeomEntityID> {
		if let Some(existing) = ids.get(&(scene_id, id)) {
			return Ok(*existing);
		}

		// Return a geomentity if possible, otherwise spatial/light
		if let InstantiatedEntityFactory::Factories(children) = &instance.factory {
			fn find_geom(
				geometry: &SlotMap<GeomEntityID, GeomEntity>,
				ids: &HashMap<(RuntimeID, InstantiatedEntityID), GeomEntityID>,
				scene_id: RuntimeID,
				scene: &InstantiatedScene,
				id: InstantiatedEntityID
			) -> Option<GeomEntityID> {
				if let InstantiatedEntityFactory::Factories(children) = &scene.entities[id].factory {
					children
						.iter()
						.find_map(|&child| find_geom(geometry, ids, scene_id, scene, child))
				} else if let Some(existing) = ids.get(&(scene_id, id))
					&& matches!(geometry[*existing].data, GeomEntityData::Geometry { .. })
				{
					Some(*existing)
				} else {
					None
				}
			}

			if let Some(existing) = children
				.iter()
				.find_map(|&child| find_geom(geometry, ids, scene_id, scene, child))
			{
				return Ok(existing);
			}

			fn find_existing(
				ids: &HashMap<(RuntimeID, InstantiatedEntityID), GeomEntityID>,
				scene_id: RuntimeID,
				scene: &InstantiatedScene,
				id: InstantiatedEntityID
			) -> Option<GeomEntityID> {
				if let InstantiatedEntityFactory::Factories(children) = &scene.entities[id].factory {
					children
						.iter()
						.find_map(|&child| find_existing(ids, scene_id, scene, child))
				} else if let Some(existing) = ids.get(&(scene_id, id)) {
					Some(*existing)
				} else {
					None
				}
			}

			if let Some(existing) = children
				.iter()
				.find_map(|&child| find_existing(ids, scene_id, scene, child))
			{
				return Ok(existing);
			}
		}

		let transform = if let Some(transform) =
			get_property_value(game, scenes, scene_id, scene, None, id, &"m_mTransform".into())?
		{
			let Variant::Transform(transform) = transform else {
				bail!("m_mTransform should be a transform")
			};

			transform.to_glam()
		} else {
			Affine3::IDENTITY
		};

		let parent = if let Some(value) =
			get_ref_property_value(scenes, scene_id, scene, None, id, &"m_eidParent".into())?
			&& let Some((parent_scene_id, parent_id)) = value
		{
			let parent_scene = &scenes.scenes[&parent_scene_id].1;
			Some(process_entity(
				game,
				scenes,
				settings,
				parent_scene_id,
				parent_scene,
				parent_id,
				&parent_scene.entities[parent_id],
				ids,
				geometry
			)?)
		} else {
			None
		};

		if let InstantiatedEntityFactory::Factory(factory) = instance.factory
			&& [
				crate::class_for_game!(game, "zgeomentity"),
				crate::class_for_game!(game, "zlinkedentity"),
				crate::class_for_game!(game, "zprimitiveproxyentity")
			]
			.contains(&factory)
			&& let Variant::Raw(visible) =
				get_property_value(game, scenes, scene_id, scene, None, id, &"m_bVisible".into())?
					.unwrap_or_else(|| Variant::Raw(RawVariant::H3(glacier_bin1::game::h3::ZVariant::new(true))))
			&& visible.variant_type() == "bool"
			&& visible.to_serde()?.as_bool().unwrap()
			&& let Some(prim) = get_property_value(game, scenes, scene_id, scene, None, id, &"m_ResourceID".into())?
			&& let Variant::Resource(_, Some(prim)) = prim
		{
			let scale = if let Some(scale) =
				get_property_value(game, scenes, scene_id, scene, None, id, &"m_PrimitiveScale".into())?
			{
				let Variant::Raw(scale) = scale else {
					bail!("m_PrimitiveScale should be a vector")
				};

				let scale = scale.to_serde()?;
				Vec3 {
					x: scale
						.get("x")
						.context("Scale should have an x component")?
						.as_f64()
						.context("Scale x component should be a float")? as f32,
					y: scale
						.get("y")
						.context("Scale should have a y component")?
						.as_f64()
						.context("Scale y component should be a float")? as f32,
					z: scale
						.get("z")
						.context("Scale should have a z component")?
						.as_f64()
						.context("Scale z component should be a float")? as f32
				}
			} else {
				Vec3::ONE
			};

			let mut material_overrides: HashMap<RuntimeID, (RuntimeID, HashMap<EcoString, Variant>)> =
				Default::default();

			if instance.is_parent_factory {
				for (parent_scene, parent_id) in
					resolve_instantiated_ref(&scenes.scenes, scene_id, instance.parent.to_owned().unwrap())?
				{
					if let InstantiatedEntityFactory::Factories(siblings) =
						&scenes.scenes[&parent_scene].1.entities[parent_id].factory
					{
						for sibling in siblings {
							let sibling = &scenes.scenes[&parent_scene].1.entities[*sibling];
							if let InstantiatedEntityFactory::Factory(factory) = &sibling.factory
								&& game.resource_type(*factory).is_some_and(|ty| ty == "ECPT")
							{
								let ecpb_data = game
									.extract_latest_resource(
										game.extract_latest_metadata(*factory)?
											.core_info
											.references
											.into_iter()
											.find(|x| game.resource_type(x.resource).is_some_and(|ty| ty == "ECPB"))
											.context("No blueprint dependency on ECPT")?
											.resource
									)?
									.1;

								let mut properties = vec![];

								macro_rules! generate {
									($game:ident) => {{
										for entry in glacier_bin1::deserialize::<
											glacier_bin1::game::$game::SExtendedCppEntityBlueprint
										>(&ecpb_data)?
										.properties
										{
											use glacier_bin1::game::$game::EExtendedPropertyType;
											if entry.property_type == EExtendedPropertyType::Resourceptr {
												properties.push((entry.property_name, entry.extra_data));
											}
										}
									}};
								}

								match game.version() {
									GlacierGame::H1 => {
										// ECPB files don't exist in H1
									}
									GlacierGame::H2 => generate!(h2),
									GlacierGame::H3 => generate!(h3),
									GlacierGame::FL => generate!(fl)
								}

								for (property, to_overwrite) in properties {
									if let Some(value) =
										get_property_value(game, scenes, scene_id, scene, None, id, &property)?
										&& let Variant::Resource(_, Some(new_material)) = value
									{
										material_overrides.insert(
											to_overwrite.try_into()?,
											(new_material.resource, Default::default())
										);
									}
								}
							}
						}
					}
				}
			}

			let added = geometry.insert(GeomEntity {
				source: (scene_id, id),
				parent,
				transform,
				data: GeomEntityData::Geometry {
					scale,
					prim: prim.resource,
					material_overrides
				}
			});

			ids.insert((scene_id, id), added);

			added
		} else if settings.lighting
			&& let InstantiatedEntityFactory::Factory(factory) = instance.factory
			&& factory == crate::class_for_game!(game, "zlightentity")
			&& let Variant::Raw(visible) =
				get_property_value(game, scenes, scene_id, scene, None, id, &"m_bVisible".into())?
					.unwrap_or_else(|| Variant::Raw(RawVariant::H3(glacier_bin1::game::h3::ZVariant::new(true))))
			&& visible.variant_type() == "bool"
			&& visible.to_serde()?.as_bool().unwrap()
		{
			let light_type = if let Some(Variant::Raw(visible)) =
				get_property_value(game, scenes, scene_id, scene, None, id, &"m_eLightType".into())?
				&& let Some(light_type) = visible.to_serde()?.as_str()
			{
				light_type.to_owned()
			} else {
				"LT_SPOT".into()
			};

			let diffuse_color = if let Some(Variant::ColorRGB(diffuse_color)) =
				get_property_value(game, scenes, scene_id, scene, None, id, &"m_diffuseColor".into())?
			{
				Vec3::new(diffuse_color.r, diffuse_color.g, diffuse_color.b)
			} else {
				Vec3::new(1.0, 1.0, 1.0)
			};

			let diffuse_power = if let Some(Variant::Raw(diffuse_power)) =
				get_property_value(game, scenes, scene_id, scene, None, id, &"m_fDiffusePower".into())?
				&& let Some(diffuse_power) = diffuse_power.to_serde()?.as_f64()
			{
				diffuse_power as f32
			} else {
				1.0
			};

			let cast_shadows = if let Some(Variant::Raw(value)) =
				get_property_value(game, scenes, scene_id, scene, None, id, &"m_bCastShadows".into())?
			{
				value.to_serde()?.as_bool().unwrap()
			} else {
				true
			};

			let added = geometry.insert(GeomEntity {
				source: (scene_id, id),
				parent,
				transform,
				data: GeomEntityData::Light {
					diffuse_color,
					diffuse_power,
					cast_shadows,
					light_kind: match light_type.as_ref() {
						"LT_DIRECTIONAL" => LightKind::Directional,

						"LT_SPOT" => {
							let (inner_angle, outer_angle) = if let Some(Variant::Raw(angular_attenuation)) =
								get_property_value(
									game,
									scenes,
									scene_id,
									scene,
									None,
									id,
									&"m_vAngularAttenuation".into()
								)? {
								let angular_attenuation = angular_attenuation.to_serde()?;
								(
									angular_attenuation
										.get("x")
										.context("Angular attenuation should be Vector2")?
										.as_f64()
										.context("Angular attenuation should be Vector2")? as f32,
									angular_attenuation
										.get("y")
										.context("Angular attenuation should be Vector2")?
										.as_f64()
										.context("Angular attenuation should be Vector2")? as f32
								)
							} else {
								(0.6981316804885864, std::f32::consts::FRAC_PI_4)
							};

							LightKind::Spot {
								inner_angle,
								outer_angle
							}
						}

						"LT_OMNI" => LightKind::Point,

						"LT_ENVIRONMENT" => LightKind::Ambient,

						_ => LightKind::Point
					}
				}
			});

			ids.insert((scene_id, id), added);

			added
		} else {
			let added = geometry.insert(GeomEntity {
				source: (scene_id, id),
				parent,
				transform,
				data: GeomEntityData::Spatial
			});

			ids.insert((scene_id, id), added);

			added
		}
	}

	for (scene_id, (_, scene)) in &scenes.scenes {
		for (id, instance) in &scene.entities {
			if let InstantiatedEntityFactory::Factory(factory) = instance.factory {
				if [
					crate::class_for_game!(game, "zgeomentity"),
					crate::class_for_game!(game, "zlinkedentity"),
					crate::class_for_game!(game, "zprimitiveproxyentity")
				]
				.contains(&factory)
					|| (settings.lighting && factory == crate::class_for_game!(game, "zlightentity"))
				{
					process_entity(
						game,
						scenes,
						&settings,
						*scene_id,
						scene,
						id,
						instance,
						&mut ids,
						&mut geometry
					)?;
				}
			}
		}
	}

	for (scene_id, (_, scene)) in &scenes.scenes {
		for (id, instance) in &scene.entities {
			if let InstantiatedEntityFactory::Factory(factory) = instance.factory {
				if game.resource_type(factory).is_some_and(|ty| ty == "MATT")
					&& get_property_value(game, scenes, *scene_id, scene, None, id, &"Active".into())?
						.map(|val| {
							if let Variant::Raw(val) = val
								&& val.variant_type() == "bool"
							{
								val.to_serde().unwrap().as_bool().unwrap()
							} else {
								true
							}
						})
						.unwrap_or(true)
					&& let Some(clients) =
						get_ref_array_property_value(scenes, *scene_id, scene, None, id, &"Clients".into())?
					&& !clients.is_empty()
				{
					let this_material = game
						.extract_latest_metadata(factory)?
						.core_info
						.references
						.iter()
						.find_map(|x| {
							game.resource_type(x.resource)
								.is_some_and(|ty| ty == "MATI")
								.then_some(x.resource)
						})
						.context("No MATI present on MATT")?;

					let overridden_material =
						if get_property_value(game, scenes, *scene_id, scene, None, id, &"Override".into())?
							.map(|val| {
								if let Variant::Raw(val) = val
									&& val.variant_type() == "bool"
								{
									val.to_serde().unwrap().as_bool().unwrap()
								} else {
									false
								}
							})
							.unwrap_or(false) && let Some(dest) = get_property_value(
							game,
							scenes,
							*scene_id,
							scene,
							None,
							id,
							&"MaterialInstanceOverrideDest".into()
						)?
						.and_then(|val| if let Variant::Resource(_, val) = val { val } else { None })
						{
							dest.resource
						} else {
							this_material
						};

					for client in clients {
						if let Some((client_scene_id, client_id)) = client {
							let client = {
								let client_scene = &scenes.scenes[&client_scene_id].1;
								process_entity(
									game,
									scenes,
									&settings,
									client_scene_id,
									client_scene,
									client_id,
									&client_scene.entities[client_id],
									&mut ids,
									&mut geometry
								)?
							};

							if let GeomEntityData::Geometry { material_overrides, .. } = &mut geometry[client].data {
								let overrides = &mut material_overrides
									.entry(overridden_material)
									.or_insert_with(|| (this_material, Default::default()))
									.1;

								for property in instance.properties.keys().filter(|x| !matt_props.contains(*x)) {
									if let Some(value) =
										get_property_value(game, scenes, *scene_id, scene, None, id, property)?
									{
										overrides.insert(property.to_owned(), value);
									}
								}
							}
						}
					}
				}
			}
		}
	}

	SceneGeometry {
		geometry,
		global_material_overrides
	}
}

#[try_fn]
#[context("Couldn't convert scene to GLTF")]
pub fn parse_scene_to_glb(
	game: &Game,
	scenes: &[RuntimeID],
	ignore_root_transforms: bool
) -> Result<(SceneGeometry, HashMap<RuntimeID, Vec<u8>>)> {
	let mut scenes = instantiate_scenes(game, scenes)?;

	if ignore_root_transforms {
		for (scene_id, (_, scene)) in &scenes.scenes {
			scenes.property_overrides.insert(
				(*scene_id, scene.root_entity, "m_mTransform".into()),
				Variant::Transform(Transform::from_glam(Affine3::IDENTITY, false))
			);
		}
	}

	let geometry = get_scene_geometry(
		game,
		&scenes,
		RenderSettings {
			lighting: false,
			..Default::default()
		}
	)?;

	let glbs = geometry
		.geometry
		.values()
		.filter_map(|ent| {
			if let GeomEntityData::Geometry { prim, .. } = &ent.data {
				Some(prim)
			} else {
				None
			}
		})
		.unique()
		.par_bridge()
		.map(|prim| {
			let (res_meta, res_data) = game.extract_latest_resource(*prim)?;

			Ok((
				*prim,
				parse_prim_to_glb(game, &res_data, &res_meta.core_info)
					.context("Couldn't parse PRIM to GLB")?
					.0
			))
		})
		.collect::<Result<_>>()?;

	(geometry, glbs)
}
