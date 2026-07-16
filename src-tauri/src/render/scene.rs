use std::{io::Cursor, ops::Deref, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use bevy::{
	asset::{RenderAssetUsages, io::embedded::GetAssetServer},
	camera::primitives::Aabb,
	camera_controller::free_camera::FreeCamera,
	core_pipeline::prepass::DepthPrepass,
	dev_tools::infinite_grid::InfiniteGrid,
	ecs::{
		lifecycle::HookContext,
		world::{CommandQueue, DeferredWorld}
	},
	image::{CompressedImageFormats, ImageType},
	mesh::{Indices, PrimitiveTopology},
	post_process::bloom::Bloom,
	prelude::*,
	render::occlusion_culling::OcclusionCulling,
	tasks::{AsyncComputeTaskPool, Task, futures::check_ready}
};
#[cfg(feature = "bevy-inspector-egui")]
use bevy_inspector_egui::{
	inspector_egui_impls::{InspectorEguiImpl, InspectorPrimitive},
	reflect_inspector::InspectorUi
};
use bevy_mod_outline::{OutlineVolume, PropagateOutline};
use bevy_panorbit_camera::PanOrbitCamera;
use dashmap::DashMap;
use ecow::EcoString;
use glacier_commons::{game::GlacierGame, metadata::RuntimeID};
use glacier_formats::material::{BlendMode, MaterialInstance, MaterialPropertyValue};
use glacier_geometry::render_primitive::{LodLevel, RenderPrimitive};
use glacier_texture::{mipblock::MipblockData, texture_map::TextureMap};
use glam::{Affine3, Affine3A};
use itertools::Itertools;
use quickentity_rs::{
	entity::SubType,
	variant::{RawVariant, Variant}
};
use tryvial::try_fn;

use crate::{
	HashMap, HashSet,
	game::GameFiles as _,
	geometry::{
		GeomEntityData, GeomEntityID, InstantiatedEntityID, InstantiatedScenes, SceneGeometry, get_scene_geometry,
		instantiate_scenes, resolve_instantiated_ref
	},
	render::{BevyReceiver, BevySender, Clearable, GameFiles, SceneRenderer}
};

/// Messages to Bevy
mod to_bevy {
	use std::sync::Arc;

	use glacier_commons::metadata::RuntimeID;
	use quickentity_rs::entity::EntityID;

	use super::super::to_bevy;
	use crate::geometry::{InstantiatedScenes, RenderSettings, SceneGeometry};

	to_bevy!(
		struct Render {
			pub scenes: Arc<InstantiatedScenes>,
			pub geometry: Arc<SceneGeometry>,
			pub settings: RenderSettings
		}
	);

	to_bevy!(
		struct Select {
			pub factory: RuntimeID,
			pub id: EntityID
		}
	);
}

/// Messages from Bevy
mod from_bevy {
	use ecow::EcoString;
	use glacier_commons::metadata::RuntimeID;
	use quickentity_rs::{entity::EntityID, variant::Variant};

	use super::super::from_bevy;

	from_bevy!(
		#[derive(Debug)]
		enum EditorEvent {
			Select {
				entities: Vec<(RuntimeID, EntityID)>
			},
			UpdateProperty {
				entity: (RuntimeID, EntityID),
				property: EcoString,
				value: Variant
			}
		}
	);
}

pub use from_bevy::EditorEvent;

use crate::geometry::{GeomEntity, InstantiatedEntityFactory, LightKind, RenderSettings, get_property_source};

pub fn configure(app: &mut App) {
	app.insert_resource(ImageCache(Default::default()))
		.insert_resource(SpecularCache(Default::default()))
		.insert_resource(MaterialCache(Default::default()))
		.insert_resource(PrimCache(Default::default()))
		.insert_resource(Scenes(Default::default()))
		.insert_resource(Geometry(Default::default()))
		.insert_resource(TransformDebounce(Timer::from_seconds(0.5, TimerMode::Once), None))
		.add_systems(Startup, init_system)
		.add_systems(
			Update,
			(
				start_render_system,
				handle_select_system,
				handle_tasks_system,
				keybind_center_system,
				keybind_switch_cam_system,
				keybind_switch_transform_mode_system,
				keybind_switch_transform_space_system,
				gizmo_transform_react_system,
				transform_debounce_system
			)
		);

	#[cfg(feature = "bevy-inspector-egui")]
	app.register_type_data::<SourceEntity, InspectorEguiImpl>();
}

pub fn window_closed(commands: &mut Commands) {
	commands.queue(|world: &mut World| {
		{
			let prim = world.remove_resource::<PrimCache>().unwrap();
			let mut meshes = world.resource_mut::<Assets<Mesh>>();

			for prim in prim.0.iter() {
				for (mesh, _) in prim.value() {
					meshes.remove(mesh.id());
				}
			}
		}

		{
			let material = world.remove_resource::<MaterialCache>().unwrap();
			let mut materials = world.resource_mut::<Assets<StandardMaterial>>();

			for material in material.0.iter() {
				materials.remove(material.id());
			}
		}

		{
			let image = world.remove_resource::<ImageCache>().unwrap();
			let specular = world.remove_resource::<SpecularCache>().unwrap();
			let mut images = world.resource_mut::<Assets<Image>>();

			for image in image.0.iter() {
				images.remove(image.id());
			}

			for specular in specular.0.iter() {
				images.remove(specular.0.id());
				images.remove(specular.1.id());
			}
		}

		world.insert_resource(ImageCache(Default::default()));
		world.insert_resource(SpecularCache(Default::default()));
		world.insert_resource(MaterialCache(Default::default()));
		world.insert_resource(PrimCache(Default::default()));
		world.insert_resource(Scenes(Default::default()));
		world.insert_resource(Geometry(Default::default()));
	});
}

impl SceneRenderer {
	#[try_fn]
	pub fn render(&self, scenes: &[RuntimeID], settings: RenderSettings) -> Result<()> {
		self.start();

		if let Some(game) = self.game.read().as_ref() {
			let mut scenes = instantiate_scenes(game, scenes)?;

			// Ignore root transforms of templates
			for (scene_id, (entity, scene)) in &scenes.scenes {
				if entity.sub_type == SubType::Template {
					scenes.property_overrides.insert(
						(*scene_id, scene.root_entity, "m_mTransform".into()),
						Variant::Transform(quickentity_rs::variant::Transform::from_glam(Affine3::IDENTITY, false))
					);
				}
			}

			let geometry = get_scene_geometry(game, &scenes, settings.to_owned())?.into();

			self.send(to_bevy::Render {
				scenes: scenes.into(),
				geometry,
				settings
			});
		}
	}

	pub fn select_entity(&self, factory: RuntimeID, id: quickentity_rs::entity::EntityID) {
		self.send(to_bevy::Select { factory, id });
	}
}

#[derive(Resource)]
struct ImageCache(Arc<DashMap<RuntimeID, Handle<Image>>>);

/// Threshold of number of geometry entities above which low-res textures will be used.
const FULL_RES_THRESHOLD: usize = 15000;

fn process_texture(world: &mut World, text: RuntimeID) -> Handle<Image> {
	let cache = world.resource::<ImageCache>().0.clone();

	cache
		.entry(text)
		.or_insert_with(move || {
			let full_res = if let Some(geometry) = world.resource::<Geometry>().0.as_ref() {
				geometry
					.geometry
					.values()
					.filter(|ent| matches!(ent.data, GeomEntityData::Geometry { .. }))
					.count() < FULL_RES_THRESHOLD
			} else {
				false
			};

			let game = world.resource::<GameFiles>().0.read().as_ref().unwrap().clone();
			world.get_asset_server().add_async(async move {
				(move || {
					let (res_meta, res_data) = game.extract_latest_resource(text)?;

					let mut texture = TextureMap::from_memory(&res_data, game.version().into())
						.context("Couldn't process texture data")?;

					if full_res && let Some(texd_depend) = res_meta.core_info.references.first() {
						let (_, texd_data) = game.extract_latest_resource(texd_depend.resource)?;
						let mipblock = MipblockData::from_memory(&texd_data, game.version().into())
							.context("Couldn't process TEXD data")?;
						texture.set_mipblock1(mipblock);
					}

					anyhow::Ok(if texture.format() == glacier_texture::enums::RenderFormat::BC5 {
						Image::from_buffer(
							&glacier_texture::convert::create_tga(&texture)
								.context("Couldn't convert texture to TGA")?,
							ImageType::Format(ImageFormat::Tga),
							CompressedImageFormats::BC,
							true,
							Default::default(),
							RenderAssetUsages::RENDER_WORLD
						)?
					} else {
						Image::from_buffer(
							&glacier_texture::convert::create_dds(&texture)
								.context("Couldn't convert texture to DDS")?,
							ImageType::Format(ImageFormat::Dds),
							CompressedImageFormats::BC,
							true,
							Default::default(),
							RenderAssetUsages::RENDER_WORLD
						)?
					})
				})()
				.map_err(ErrorWrapper::from)
			})
		})
		.clone()
}

#[derive(Resource)]
struct SpecularCache(Arc<DashMap<RuntimeID, (Handle<Image>, Handle<Image>)>>);

/// Process a specular-glossiness texture into a specular texture plus a metallic-roughness texture.
/// Methodology from https://github.com/donmccurdy/glTF-Transform/blob/main/packages/functions/src/metal-rough.ts
fn process_specular_texture(world: &mut World, text: RuntimeID) -> (Handle<Image>, Handle<Image>) {
	let cache = world.resource::<SpecularCache>().0.clone();

	cache
		.entry(text)
		.or_insert_with(move || {
			let full_res = if let Some(geometry) = world.resource::<Geometry>().0.as_ref() {
				geometry
					.geometry
					.values()
					.filter(|ent| matches!(ent.data, GeomEntityData::Geometry { .. }))
					.count() < FULL_RES_THRESHOLD
			} else {
				false
			};

			(
				world.get_asset_server().add_async({
					let game = world.resource::<GameFiles>().0.read().as_ref().unwrap().clone();
					async move {
						(move || {
							let (res_meta, res_data) = game.extract_latest_resource(text)?;

							let mut texture = TextureMap::from_memory(&res_data, game.version().into())
								.context("Couldn't process texture data")?;

							if full_res && let Some(texd_depend) = res_meta.core_info.references.first() {
								let (_, texd_data) = game.extract_latest_resource(texd_depend.resource)?;
								let mipblock = MipblockData::from_memory(&texd_data, game.version().into())
									.context("Couldn't process TEXD data")?;
								texture.set_mipblock1(mipblock);
							}

							let mut texture = if texture.format() == glacier_texture::enums::RenderFormat::BC5 {
								image::load_from_memory_with_format(
									&glacier_texture::convert::create_tga(&texture)
										.context("Couldn't convert texture to TGA")?,
									image::ImageFormat::Tga
								)?
							} else {
								glacier_texture::convert::create_dynamic_image(&texture)
									.context("Couldn't convert texture to dynamic image")?
							}
							.to_rgba8();

							// Remove glossiness
							for pixel in texture.pixels_mut() {
								pixel[3] = 255;
							}

							anyhow::Ok(Image::from_dynamic(
								texture.into(),
								true,
								RenderAssetUsages::RENDER_WORLD
							))
						})()
						.map_err(ErrorWrapper::from)
					}
				}),
				world.get_asset_server().add_async({
					let game = world.resource::<GameFiles>().0.read().as_ref().unwrap().clone();
					async move {
						(move || {
							let (res_meta, res_data) = game.extract_latest_resource(text)?;

							let mut texture = TextureMap::from_memory(&res_data, game.version().into())
								.context("Couldn't process texture data")?;

							if let Some(texd_depend) = res_meta.core_info.references.first() {
								let (_, texd_data) = game.extract_latest_resource(texd_depend.resource)?;
								let mipblock = MipblockData::from_memory(&texd_data, game.version().into())
									.context("Couldn't process TEXD data")?;
								texture.set_mipblock1(mipblock);
							}

							let mut texture = if texture.format() == glacier_texture::enums::RenderFormat::BC5 {
								image::load_from_memory_with_format(
									&glacier_texture::convert::create_tga(&texture)
										.context("Couldn't convert texture to TGA")?,
									image::ImageFormat::Tga
								)?
							} else {
								glacier_texture::convert::create_dynamic_image(&texture)
									.context("Couldn't convert texture to dynamic image")?
							}
							.to_rgba8();

							// Glossiness to roughness
							for pixel in texture.pixels_mut() {
								let roughness = 255 - pixel[3];
								pixel[0] = 0;
								pixel[1] = roughness;
								pixel[2] = 0;
								pixel[3] = 255;
							}

							anyhow::Ok(Image::from_dynamic(
								texture.into(),
								true,
								RenderAssetUsages::RENDER_WORLD
							))
						})()
						.map_err(ErrorWrapper::from)
					}
				})
			)
		})
		.clone()
}

#[derive(Resource)]
struct MaterialCache(Arc<DashMap<(RuntimeID, Option<Vec<u8>>), Handle<StandardMaterial>>>);

fn process_material(
	world: &mut World,
	mati: RuntimeID,
	global_overrides: Option<&HashMap<EcoString, Variant>>,
	overrides: &HashMap<EcoString, Variant>
) -> Handle<StandardMaterial> {
	let cache = world.resource::<MaterialCache>().0.clone();

	cache
		.entry((
			mati,
			(!overrides.is_empty()).then(|| {
				let mut to_ser = overrides.iter().collect_vec();
				to_ser.sort_unstable_by(|&(a, _), &(b, _)| a.cmp(b));
				serde_brief::to_vec(&to_ser).unwrap()
			})
		))
		.or_insert_with(move || {
			let game = world.resource::<GameFiles>().0.read().as_ref().unwrap().clone();

			let Ok((res_meta, res_data)) = game.extract_latest_resource(mati) else {
				return world.get_asset_server().add(Color::srgb_u8(255, 0, 0).into());
			};

			let Ok(material) = MaterialInstance::parse(&res_data, &res_meta.core_info) else {
				return world.get_asset_server().add(Color::srgb_u8(255, 0, 0).into());
			};

			let friendly_names = if let Some(class) = &material.class {
				let mate_data = game.extract_latest_resource(*class).unwrap().1;

				let mut beginning = mate_data.len() - 1;
				while mate_data[beginning] == 0 || (mate_data[beginning] > 31 && mate_data[beginning] < 127) {
					beginning -= 1;
				}
				beginning += 1;

				String::from_utf8(mate_data[beginning..mate_data.len() - 1].into())
					.unwrap()
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

			let diffuse_texture = friendly_names
				.iter()
				.filter_map(|(friendly, prop)| {
					(friendly.to_lowercase().contains("diffuse") || friendly.to_lowercase().contains("basecolor"))
						.then_some(prop)
				})
				.filter_map(|prop| {
					if let Some(val) = overrides
						.get(prop.as_str())
						.or_else(|| global_overrides.and_then(|overrides| overrides.get(prop.as_str())))
						.or_else(|| {
							prop.strip_prefix("map").and_then(|prop| {
								overrides
									.get(prop)
									.or_else(|| global_overrides.and_then(|overrides| overrides.get(prop)))
							})
						}) {
						if let Variant::Resource(_, value) = val {
							value.as_ref().map(|x| x.resource)
						} else {
							None
						}
					} else {
						material.binder.properties.get(prop).and_then(|prop| {
							if let MaterialPropertyValue::Texture { enabled, value, .. } = prop
								&& *enabled
							{
								*value
							} else {
								None
							}
						})
					}
				})
				.next()
				.filter(|&id| game.resource_exists(id))
				.map(|id| process_texture(world, id));

			let diffuse_color = if diffuse_texture.is_none() {
				friendly_names
					.iter()
					.filter_map(|(friendly, prop)| {
						(friendly.to_lowercase().contains("diffuse") || friendly.to_lowercase().contains("basecolor"))
							.then_some(prop)
					})
					.filter_map(|prop| {
						if let Some(val) = overrides
							.get(prop.as_str())
							.or_else(|| global_overrides.and_then(|overrides| overrides.get(prop.as_str())))
						{
							if let Variant::ColorRGB(value) = val {
								Some(Color::srgb(value.r, value.g, value.b))
							} else if let Variant::ColorRGBA(value) = val {
								Some(Color::srgba(value.r, value.g, value.b, value.a))
							} else {
								None
							}
						} else {
							material.binder.properties.get(prop).and_then(|prop| {
								if let MaterialPropertyValue::Colour { enabled, value } = prop
									&& *enabled
								{
									Some(if value.len() == 7 {
										Color::srgb_u8(
											u8::from_str_radix(&value[1..3], 16).unwrap(),
											u8::from_str_radix(&value[3..5], 16).unwrap(),
											u8::from_str_radix(&value[5..7], 16).unwrap()
										)
									} else if value.len() == 9 {
										Color::srgba_u8(
											u8::from_str_radix(&value[1..3], 16).unwrap(),
											u8::from_str_radix(&value[3..5], 16).unwrap(),
											u8::from_str_radix(&value[5..7], 16).unwrap(),
											u8::from_str_radix(&value[7..9], 16).unwrap()
										)
									} else {
										unreachable!()
									})
								} else {
									None
								}
							})
						}
					})
					.next()
			} else {
				None
			};

			let (specular_texture, metallic_roughness_texture) = if let Some(geometry) =
				world.resource::<Geometry>().0.as_ref()
				&& geometry
					.geometry
					.values()
					.filter(|ent| matches!(ent.data, GeomEntityData::Geometry { .. }))
					.count() < FULL_RES_THRESHOLD
			{
				friendly_names
					.iter()
					.filter_map(|(friendly, prop)| friendly.to_lowercase().contains("spec").then_some(prop))
					.filter_map(|prop| {
						if let Some(val) = overrides
							.get(prop.as_str())
							.or_else(|| global_overrides.and_then(|overrides| overrides.get(prop.as_str())))
							.or_else(|| {
								prop.strip_prefix("map").and_then(|prop| {
									overrides
										.get(prop)
										.or_else(|| global_overrides.and_then(|overrides| overrides.get(prop)))
								})
							}) {
							if let Variant::Resource(_, value) = val {
								value.as_ref().map(|x| x.resource)
							} else {
								None
							}
						} else {
							material.binder.properties.get(prop).and_then(|prop| {
								if let MaterialPropertyValue::Texture { enabled, value, .. } = prop
									&& *enabled
								{
									*value
								} else {
									None
								}
							})
						}
					})
					.next()
					.filter(|&id| game.resource_exists(id))
					.map(|id| process_specular_texture(world, id))
			} else {
				None
			}
			.unzip();

			let normal_texture = friendly_names
				.iter()
				.filter_map(|(friendly, prop)| friendly.to_lowercase().contains("normal").then_some(prop))
				.chain(material.binder.properties.iter().filter_map(|(name, prop)| {
					(name.to_lowercase().contains("normal")
						&& matches!(prop, MaterialPropertyValue::Texture { enabled: true, .. }))
					.then_some(name)
				}))
				.filter_map(|prop| {
					if let Some(val) = overrides
						.get(prop.as_str())
						.or_else(|| global_overrides.and_then(|overrides| overrides.get(prop.as_str())))
						.or_else(|| {
							prop.strip_prefix("map").and_then(|prop| {
								overrides
									.get(prop)
									.or_else(|| global_overrides.and_then(|overrides| overrides.get(prop)))
							})
						}) {
						if let Variant::Resource(_, value) = val {
							value.as_ref().map(|x| x.resource)
						} else {
							None
						}
					} else {
						material.binder.properties.get(prop).and_then(|prop| {
							if let MaterialPropertyValue::Texture { enabled, value, .. } = prop
								&& *enabled
							{
								*value
							} else {
								None
							}
						})
					}
				})
				.next()
				.filter(|&id| game.resource_exists(id))
				.map(|id| process_texture(world, id));

			let emissive_texture = friendly_names
				.iter()
				.filter_map(|(friendly, prop)| friendly.to_lowercase().contains("emis").then_some(prop))
				.filter_map(|prop| {
					if let Some(val) = overrides
						.get(prop.as_str())
						.or_else(|| global_overrides.and_then(|overrides| overrides.get(prop.as_str())))
						.or_else(|| {
							prop.strip_prefix("map").and_then(|prop| {
								overrides
									.get(prop)
									.or_else(|| global_overrides.and_then(|overrides| overrides.get(prop)))
							})
						}) {
						if let Variant::Resource(_, value) = val {
							value.as_ref().map(|x| x.resource)
						} else {
							None
						}
					} else {
						material.binder.properties.get(prop).and_then(|prop| {
							if let MaterialPropertyValue::Texture { enabled, value, .. } = prop
								&& *enabled
							{
								*value
							} else {
								None
							}
						})
					}
				})
				.next()
				.filter(|&id| game.resource_exists(id))
				.map(|id| process_texture(world, id));

			let emissive_color = friendly_names
				.iter()
				.filter_map(|(friendly, prop)| (friendly.to_lowercase().contains("emis")).then_some(prop))
				.filter_map(|prop| {
					if let Some(val) = overrides
						.get(prop.as_str())
						.or_else(|| global_overrides.and_then(|overrides| overrides.get(prop.as_str())))
					{
						if let Variant::ColorRGB(value) = val {
							Some(LinearRgba::rgb(value.r, value.g, value.b))
						} else if let Variant::ColorRGBA(value) = val {
							Some(LinearRgba::new(value.r, value.g, value.b, value.a))
						} else {
							None
						}
					} else {
						material.binder.properties.get(prop).and_then(|prop| {
							if let MaterialPropertyValue::Colour { enabled, value } = prop
								&& *enabled
							{
								Some(if value.len() == 7 {
									LinearRgba::rgb(
										u8::from_str_radix(&value[1..3], 16).unwrap() as f32 / 255.0,
										u8::from_str_radix(&value[3..5], 16).unwrap() as f32 / 255.0,
										u8::from_str_radix(&value[5..7], 16).unwrap() as f32 / 255.0
									)
								} else if value.len() == 9 {
									LinearRgba::new(
										u8::from_str_radix(&value[1..3], 16).unwrap() as f32 / 255.0,
										u8::from_str_radix(&value[3..5], 16).unwrap() as f32 / 255.0,
										u8::from_str_radix(&value[5..7], 16).unwrap() as f32 / 255.0,
										u8::from_str_radix(&value[7..9], 16).unwrap() as f32 / 255.0
									)
								} else {
									unreachable!()
								})
							} else {
								None
							}
						})
					}
				})
				.next();

			let emissive_factor = if emissive_texture.is_some() || emissive_color.is_some() {
				friendly_names
					.iter()
					.filter_map(|(friendly, prop)| friendly.to_lowercase().contains("emis_intensity").then_some(prop))
					.filter_map(|prop| {
						if let Some(val) = overrides
							.get(prop.as_str())
							.or_else(|| global_overrides.and_then(|overrides| overrides.get(prop.as_str())))
						{
							if let Variant::Raw(value) = val
								&& value.variant_type() == "float32"
							{
								Some(value.to_serde().unwrap().as_f64().unwrap() as f32)
							} else {
								None
							}
						} else {
							material.binder.properties.get(prop).and_then(|prop| {
								if let MaterialPropertyValue::Float { enabled, value, .. } = prop
									&& *enabled
								{
									Some(*value)
								} else {
									None
								}
							})
						}
					})
					.next()
			} else {
				None
			};

			let alpha_discard = friendly_names
				.iter()
				.filter_map(|(friendly, prop)| (friendly.to_lowercase().contains("alpha_discard")).then_some(prop))
				.filter_map(|prop| {
					if let Some(val) = overrides
						.get(prop.as_str())
						.or_else(|| global_overrides.and_then(|overrides| overrides.get(prop.as_str())))
					{
						if let Variant::Raw(value) = val
							&& value.variant_type() == "float32"
						{
							Some(value.to_serde().unwrap().as_f64().unwrap() as f32)
						} else {
							None
						}
					} else {
						material.binder.properties.get(prop).and_then(|prop| {
							if let MaterialPropertyValue::Float { enabled, value, .. } = prop
								&& *enabled
							{
								Some(*value)
							} else {
								None
							}
						})
					}
				})
				.next();

			world.get_asset_server().add(StandardMaterial {
				base_color_texture: diffuse_texture,
				base_color: if material.instance_flags.trans_add_emissive {
					Color::NONE
				} else {
					diffuse_color.unwrap_or(Color::WHITE)
				},
				normal_map_texture: normal_texture,
				emissive: (emissive_color.unwrap_or(LinearRgba::rgb(1.0, 1.0, 1.0)) * emissive_factor.unwrap_or(0.0))
					.with_alpha(emissive_factor.unwrap_or(0.0).min(1.0)),
				emissive_texture,
				metallic: 0.0,
				perceptual_roughness: 1.0,
				ior: 1000.0,
				specular_texture,
				metallic_roughness_texture,
				alpha_mode: if material.binder.render_state.blend_enabled.unwrap_or(false)
					|| material.binder.render_state.decal_blend_diffuse.unwrap_or(0) != 0
				{
					match material.binder.render_state.blend_mode.unwrap_or(BlendMode::Trans) {
						BlendMode::Add => AlphaMode::Add,
						BlendMode::Sub => AlphaMode::Blend, // no Sub in bevy
						BlendMode::Trans => AlphaMode::Blend,
						BlendMode::TransOnOpaque => AlphaMode::Multiply,
						BlendMode::Opaque => AlphaMode::Opaque,
						BlendMode::TransPremultipliedAlpha => AlphaMode::Premultiplied
					}
				} else if material.binder.render_state.alpha_test_enabled.unwrap_or(false) {
					AlphaMode::Mask(material.binder.render_state.alpha_reference.unwrap_or(255) as f32 / 255.0)
				} else if let Some(alpha_discard) = alpha_discard {
					AlphaMode::Mask(alpha_discard)
				} else {
					AlphaMode::Opaque
				},
				..default()
			})
		})
		.clone()
}

#[derive(Resource)]
struct PrimCache(Arc<DashMap<(RuntimeID, Option<Vec<u8>>), Vec<(Handle<Mesh>, Handle<StandardMaterial>)>>>);

fn process_prim(
	world: &mut World,
	settings: &RenderSettings,
	prim: RuntimeID,
	global_material_overrides: &HashMap<RuntimeID, (RuntimeID, HashMap<EcoString, Variant>)>,
	material_overrides: HashMap<RuntimeID, (RuntimeID, HashMap<EcoString, Variant>)>
) -> Vec<(Handle<Mesh>, Handle<StandardMaterial>)> {
	let cache = world.resource::<PrimCache>().0.clone();

	cache
		.entry((
			prim,
			(!material_overrides.is_empty()).then(|| {
				let mut to_ser = material_overrides.iter().collect_vec();
				to_ser.sort_unstable_by(|&(a, _), &(b, _)| a.cmp(b));
				serde_brief::to_vec(&to_ser).unwrap()
			})
		))
		.or_insert_with(move || {
			(|| {
				let game = world.resource::<GameFiles>().0.read().as_ref().unwrap().clone();

				let mut result = vec![];

				let (res_meta, res_data) = game.extract_latest_resource(prim)?;

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

				for obj in model.iter_primitive_of_lod(match settings.lod {
					LodLevel::LEVEL1 => LodLevel::LEVEL1,
					LodLevel::LEVEL2 => LodLevel::LEVEL2,
					LodLevel::LEVEL3 => LodLevel::LEVEL3,
					LodLevel::LEVEL4 => LodLevel::LEVEL4,
					LodLevel::LEVEL5 => LodLevel::LEVEL5,
					LodLevel::LEVEL6 => LodLevel::LEVEL6,
					LodLevel::LEVEL7 => LodLevel::LEVEL7,
					LodLevel::LEVEL8 => LodLevel::LEVEL8
				}) {
					let mesh = world.resource_mut::<Assets<Mesh>>().add({
						let mut mesh = Mesh::new(
							PrimitiveTopology::TriangleList,
							RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD
						)
						.with_inserted_attribute(
							Mesh::ATTRIBUTE_POSITION,
							obj.get_positions()
								.into_iter()
								.map(|pos| convert_point(Vec3::new(pos.x, pos.y, pos.z)).to_array())
								.collect_vec()
						)
						.with_inserted_indices(Indices::U16(
							obj.get_indices()
								.as_chunks()
								.0
								.iter()
								.flat_map(|&[a, b, c]| [a, b, c])
								.collect_vec()
						))
						.with_inserted_attribute(
							Mesh::ATTRIBUTE_NORMAL,
							obj.get_normals()
								.into_iter()
								.map(|norm| convert_point(Vec3::new(norm.x, norm.y, norm.z)).to_array())
								.collect_vec()
						)
						.with_inserted_attribute(
							Mesh::ATTRIBUTE_TANGENT,
							obj.get_tangents()
								.into_iter()
								.map(|tangent| {
									let converted = convert_point(Vec3::new(tangent.x, tangent.y, tangent.z));
									[converted.x, converted.y, converted.z, tangent.w]
								})
								.collect_vec()
						);

						if let Some(uvs) = obj.get_tex_coords().first() {
							mesh = mesh.with_inserted_attribute(
								Mesh::ATTRIBUTE_UV_0,
								uvs.into_iter().map(|norm| [norm.x, norm.y]).collect_vec()
							);
						}

						if let Some(uvs) = obj.get_tex_coords().get(1) {
							mesh = mesh.with_inserted_attribute(
								Mesh::ATTRIBUTE_UV_1,
								uvs.into_iter().map(|norm| [norm.x, norm.y]).collect_vec()
							);
						}

						mesh
					});

					let material = if let Some(material_id) = res_meta
						.core_info
						.references
						.get(obj.prim_mesh().prim_object.material_id as usize)
						.map(|x| x.resource)
					{
						if let Some((override_id, override_props)) = material_overrides.get(&material_id) {
							process_material(
								world,
								*override_id,
								global_material_overrides.get(&override_id).as_ref().map(|(_, x)| x),
								override_props
							)
						} else if let Some((override_id, override_props)) = global_material_overrides.get(&material_id)
						{
							process_material(
								world,
								*override_id,
								Some(override_props),
								&material_overrides
									.get(&material_id)
									.as_ref()
									.map(|(_, x)| x)
									.unwrap_or(&Default::default())
							)
						} else {
							process_material(world, material_id, None, &Default::default())
						}
					} else {
						process_material(world, 0.try_into().unwrap(), None, &Default::default())
					};

					result.push((mesh, material));
				}

				anyhow::Ok(result)
			})()
			.unwrap()
		})
		.clone()
}

fn get_box_mesh(world: &mut World) -> (Handle<Mesh>, Handle<StandardMaterial>) {
	let cache = world.resource::<PrimCache>().0.clone();

	cache.entry((1.try_into().unwrap(), None)).or_insert_with(move || {
		vec![(
			world.resource_mut::<Assets<Mesh>>().add(Cuboid::new(1.0, 1.0, 1.0)),
			world.resource_mut::<Assets<StandardMaterial>>().add(Color::NONE)
		)]
	})[0]
		.to_owned()
}

#[derive(Debug)]
struct ErrorWrapper(anyhow::Error);

impl std::fmt::Display for ErrorWrapper {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		std::fmt::Display::fmt(&self.0, f)
	}
}

impl std::error::Error for ErrorWrapper {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		self.0.source()
	}
}

impl From<anyhow::Error> for ErrorWrapper {
	fn from(err: anyhow::Error) -> Self {
		ErrorWrapper(err)
	}
}

/// Convert a transform from the game's Z+ up system to Bevy's Y+ up system.
fn convert_transform(transform: Affine3) -> Transform {
	let (scale, rotation, translation) = transform.to_scale_rotation_translation();

	Transform {
		scale: convert_size(scale),
		rotation: convert_rotation(rotation),
		translation: convert_point(translation)
	}
}

/// Convert a Vec3 describing measurements in the game's Z+ up system to Bevy's Y+ up system.
fn convert_size(vec: Vec3) -> Vec3 {
	// Prevent degenerate transformation
	Vec3::new(
		if vec.x == 0.0 { 0.001 } else { vec.x },
		if vec.z == 0.0 { 0.001 } else { vec.z },
		if vec.y == 0.0 { 0.001 } else { vec.y }
	)
}

/// Convert a Quat describing a rotation in the game's Z+ up system to Bevy's Y+ up system.
fn convert_rotation(quat: Quat) -> Quat {
	let (axis, angle) = quat.conjugate().to_axis_angle();
	Quat::from_axis_angle(convert_point(axis), angle)
}

/// Convert a Vec3 describing a point or direction in the game's Z+ up system to Bevy's Y+ up system.
fn convert_point(vec: Vec3) -> Vec3 {
	Vec3::new(vec.x, vec.z, -vec.y)
}

/// Convert a transform from Bevy's Y+ up system to the game's Z+ up system.
fn unconvert_transform(transform: Affine3A) -> Affine3 {
	let (scale, rotation, translation) = transform.to_scale_rotation_translation();

	Affine3::from_scale_rotation_translation(
		unconvert_size(scale),
		unconvert_rotation(rotation),
		unconvert_point(translation)
	)
}

/// Convert a Vec3 describing measurements in Bevy's Y+ up system to the game's Z+ up system.
fn unconvert_size(vec: Vec3) -> Vec3 {
	Vec3::new(vec.x, vec.z, vec.y)
}

/// Convert a Quat describing a rotation in Bevy's Y+ up system to the game's Z+ up system.
fn unconvert_rotation(quat: Quat) -> Quat {
	let (axis, angle) = quat.to_axis_angle();
	Quat::from_axis_angle(unconvert_point(axis), angle).conjugate()
}

/// Convert a Vec3 describing a point or direction in Bevy's Y+ up system to the game's Z+ up system.
fn unconvert_point(vec: Vec3) -> Vec3 {
	Vec3::new(vec.x, -vec.z, vec.y)
}

#[derive(Clone, Copy, Reflect, Component)]
#[reflect(opaque)]
struct SourceEntity(GeomEntityID);

#[cfg(feature = "bevy-inspector-egui")]
impl InspectorPrimitive for SourceEntity {
	fn ui(&mut self, ui: &mut egui::Ui, _: &dyn std::any::Any, _: egui::Id, _: InspectorUi<'_, '_>) -> bool {
		ui.label(format!("{:?}", self.0));
		false
	}

	fn ui_readonly(&self, ui: &mut egui::Ui, _: &dyn std::any::Any, _: egui::Id, _: InspectorUi<'_, '_>) {
		ui.label(format!("{:?}", self.0));
	}
}

#[derive(Clone, Reflect, Component)]
#[reflect(opaque)]
struct SourceEntityHierarchy(Vec<(RuntimeID, quickentity_rs::entity::EntityID)>);

#[cfg(feature = "bevy-inspector-egui")]
impl InspectorPrimitive for SourceEntityHierarchy {
	fn ui(&mut self, ui: &mut egui::Ui, _: &dyn std::any::Any, _: egui::Id, _: InspectorUi<'_, '_>) -> bool {
		for (ent, id) in &self.0 {
			ui.label(format!("{:?} in {}", id, ent));
		}

		false
	}

	fn ui_readonly(&self, ui: &mut egui::Ui, _: &dyn std::any::Any, _: egui::Id, _: InspectorUi<'_, '_>) {
		for (ent, id) in &self.0 {
			ui.label(format!("{:?} in {}", id, ent));
		}
	}
}

fn get_entity_hierarchy(
	scenes: &InstantiatedScenes,
	source: (RuntimeID, InstantiatedEntityID)
) -> Vec<(RuntimeID, quickentity_rs::entity::EntityID)> {
	let mut entities = vec![];
	let mut check = Some(source);
	while let Some((scene, id)) = check {
		let entity = &scenes.scenes[&scene].1.entities[id];

		if !entities.last().is_some_and(|x| *x == entity.source) {
			entities.push(entity.source);
		}

		check = entity
			.parent
			.as_ref()
			.map(|parent| resolve_instantiated_ref(&scenes.scenes, scene, parent))
			.and_then(|mut parents| (!parents.is_empty()).then(|| parents.swap_remove(0)));
	}

	entities
}

#[derive(Component)]
#[require(
		OutlineVolume{ width: 5.0, visible: true, colour: Color::srgb_u32(0x4ac7ed) }
	)]
#[require(PropagateOutline)]
#[component(on_remove = remove_selected)]
struct Selected;

fn remove_selected(mut world: DeferredWorld, context: HookContext) {
	let entity = context.entity;

	world
		.commands()
		.entity(entity)
		.try_remove::<OutlineVolume>()
		.try_remove::<PropagateOutline>()
		.try_remove::<TransformGizmoFocus>();
}

fn process_geom(
	scenes: Arc<InstantiatedScenes>,
	geometry: Arc<SceneGeometry>,
	settings: Arc<RenderSettings>,
	entity: Entity,
	geom_entity_id: GeomEntityID,
	geom_entity: GeomEntity
) -> CommandQueue {
	let mut commands = CommandQueue::default();

	let transform = geom_entity.transform;

	match geom_entity.data {
		GeomEntityData::Spatial => {
			let transform = convert_transform(transform);
			commands.push(move |world: &mut World| {
				world.entity_mut(entity).insert(transform);
			});
		}

		GeomEntityData::BoxVolume { size } => {
			let transform = convert_transform(transform).with_scale(convert_size(size));
			commands.push(move |world: &mut World| {
				world.entity_mut(entity).insert(transform);

				let (mesh, material) = get_box_mesh(world);
				world.spawn((
					Transform::IDENTITY,
					Visibility::default(),
					ChildOf(entity),
					Mesh3d(mesh),
					MeshMaterial3d(material),
					Pickable::IGNORE
				));
			});
		}

		GeomEntityData::CoverPlane { length, depth } => {
			let transform = convert_transform(transform).with_scale(convert_size(Vec3::new(length, depth, 0.5)));
			commands.push(move |world: &mut World| {
				world.entity_mut(entity).insert(transform);

				let (mesh, material) = get_box_mesh(world);
				world.spawn((
					Transform {
						translation: Vec3::new(0.0, 0.5, 0.0),
						..default()
					},
					Visibility::default(),
					ChildOf(entity),
					Mesh3d(mesh),
					MeshMaterial3d(material),
					Pickable::IGNORE
				));
			});
		}

		GeomEntityData::Geometry {
			scale,
			prim,
			material_overrides
		} => {
			let transform = convert_transform(transform).with_scale(convert_size(scale));
			let geometry = geometry.clone();
			let settings = settings.clone();
			commands.push(move |world: &mut World| {
				world.entity_mut(entity).insert(transform).observe(
					move |event: On<Pointer<Click>>,
					      query: Query<&SourceEntityHierarchy>,
					      selected: Query<Entity, With<Selected>>,
					      mut commands: Commands,
					      sender: Res<BevySender<from_bevy::EditorEvent>>| {
						if event.button != PointerButton::Primary || event.duration > Duration::from_millis(250) {
							return;
						}

						for entity in &selected {
							commands.entity(entity).remove::<Selected>();
						}

						commands.entity(entity).insert((Selected, TransformGizmoFocus));

						let SourceEntityHierarchy(entities) = query.get(entity).unwrap();
						let _ = sender.0.send(from_bevy::EditorEvent::Select {
							entities: entities.to_owned()
						});

						log::info!("Selected {entity:?}: {entities:?}");
					}
				);

				let objects = process_prim(
					world,
					&settings,
					prim,
					&geometry.global_material_overrides,
					material_overrides
				);
				for (mesh, material) in objects {
					world.spawn((
						Transform::IDENTITY,
						Visibility::default(),
						ChildOf(entity),
						Mesh3d(mesh),
						MeshMaterial3d(material)
					));
				}
			});
		}

		GeomEntityData::Light {
			diffuse_color,
			diffuse_power,
			cast_shadows,
			light_kind
		} => {
			let transform = convert_transform(transform);
			let color = Color::srgb(diffuse_color.x, diffuse_color.y, diffuse_color.z);
			commands.push(move |world: &mut World| {
				world.entity_mut(entity).insert(transform.to_owned());

				match light_kind {
					LightKind::Point => {
						world.entity_mut(entity).insert(PointLight {
							color,
							intensity: diffuse_power * 150.0,
							..default()
						});
					}

					LightKind::Directional => {
						world.entity_mut(entity).insert(DirectionalLight {
							color,
							illuminance: diffuse_power,
							shadow_maps_enabled: cast_shadows,
							..default()
						});
					}

					LightKind::Spot {
						inner_angle,
						outer_angle
					} => {
						world.entity_mut(entity).insert(SpotLight {
							color,
							intensity: diffuse_power * 150.0,
							inner_angle,
							outer_angle,
							..default()
						});
					}

					LightKind::Ambient => {
						world
							.entity_mut(entity)
							.insert(transform.looking_at(Vec3::ZERO, Dir3::Y))
							.insert(DirectionalLight {
								color,
								illuminance: diffuse_power,
								shadow_maps_enabled: cast_shadows,
								..default()
							});
					}
				}
			});
		}
	}

	for (child_id, child_geom_entity) in geometry.geometry.iter().filter_map(|(id, geom_entity)| {
		geom_entity
			.parent
			.is_some_and(|id| id == geom_entity_id)
			.then(|| (id, geom_entity.to_owned()))
	}) {
		let scenes = scenes.clone();
		let geometry = geometry.clone();
		let settings = settings.clone();
		commands.push(move |world: &mut World| {
			let child = world
				.spawn((
					Clearable,
					Transform::IDENTITY,
					Visibility::default(),
					ChildOf(entity),
					SourceEntity(child_id),
					SourceEntityHierarchy(get_entity_hierarchy(&scenes, child_geom_entity.source))
				))
				.id();

			let task = AsyncComputeTaskPool::get()
				.spawn(async move { process_geom(scenes, geometry, settings, child, child_id, child_geom_entity) });

			world.entity_mut(child).insert(Loading(task));
		});
	}

	commands
}

#[derive(Resource)]
struct Scenes(Option<Arc<InstantiatedScenes>>);

#[derive(Resource)]
struct Geometry(Option<Arc<SceneGeometry>>);

fn init_system(mut commands: Commands) {
	commands.spawn(InfiniteGrid);

	#[cfg(feature = "bevy-inspector-egui")]
	{
		commands.spawn(bevy::dev_tools::diagnostics_overlay::DiagnosticsOverlay::fps());
		commands.spawn(bevy::dev_tools::diagnostics_overlay::DiagnosticsOverlay::mesh_and_standard_material());
	}
}

fn start_render_system(
	mut commands: Commands,
	receiver: Res<BevyReceiver<to_bevy::Render>>,
	game_files: Res<GameFiles>,
	scenes: Res<Scenes>,
	geometry: Res<Geometry>,
	existing_query: Query<(Entity, &SourceEntity, &SourceEntityHierarchy, Option<&ChildOf>), With<Clearable>>,
	descendants: Query<&Children>,
	mut camera: Query<Entity, With<TransformGizmoCamera>>
) {
	if let Ok(render) = receiver.0.try_recv() {
		// Initialise basics
		let ambient_light = {
			let game_files = game_files.0.read().as_ref().unwrap().clone();

			AmbientLight {
				color: if render.settings.lighting
					&& let Some(entity) = render
						.scenes
						.scenes
						.values()
						.flat_map(|(_, scene)| scene.entities.values())
						.find(|entity| {
							entity.factory
								== InstantiatedEntityFactory::Factory(crate::class_for_game!(
									game_files,
									"zgloballightentity"
								))
						}) && let Some(value) = game_files.extract_entity(entity.source.0).unwrap().sub_entities
					[&entity.source.1]
					.properties
					.get("m_GlobalLightFrontColor")
					&& let Variant::ColorRGB(value) = &value.value
				{
					Color::srgb(value.r, value.g, value.b)
				} else {
					Color::WHITE
				},
				brightness: if render.settings.lighting
					&& let Some(entity) = render
						.scenes
						.scenes
						.values()
						.flat_map(|(_, scene)| scene.entities.values())
						.find(|entity| {
							entity.factory
								== InstantiatedEntityFactory::Factory(crate::class_for_game!(
									game_files,
									"zgloballightentity"
								))
						}) && let Some(value) = game_files.extract_entity(entity.source.0).unwrap().sub_entities
					[&entity.source.1]
					.properties
					.get("m_fGlobalLightFrontIntensity")
					&& let Variant::Raw(value) = &value.value
					&& let Some(value) = value.to_serde().unwrap().as_f64()
				{
					value as f32
				} else {
					250.0
				},
				..default()
			}
		};

		let mut view = if let Ok(camera) = camera.single_mut() {
			commands.entity(camera).insert(ambient_light);
			commands.entity(camera)
		} else {
			commands.spawn((
				Clearable,
				Transform::from_xyz(5.0, 5.0, 5.0),
				Camera3d::default(),
				TransformGizmoCamera,
				DepthPrepass,
				OcclusionCulling,
				#[cfg(feature = "bevy-inspector-egui")]
				bevy_inspector_egui::bevy_egui::PrimaryEguiContext::default(),
				PanOrbitCamera {
					button_orbit: MouseButton::Right,
					modifier_pan: Some(KeyCode::ShiftLeft),
					..default()
				},
				ambient_light
			))
		};

		if render.settings.lighting {
			view.insert(Bloom::NATURAL);
		} else {
			view.try_remove::<Bloom>();
		}

		// Clear scene
		let mut to_spawn = vec![];

		fn get_hierarchy(
			scenes: &InstantiatedScenes,
			hierarchy: &mut HashMap<
				(RuntimeID, InstantiatedEntityID),
				Vec<(RuntimeID, quickentity_rs::entity::EntityID)>
			>,
			(scene, id): (RuntimeID, InstantiatedEntityID)
		) -> Vec<(RuntimeID, quickentity_rs::entity::EntityID)> {
			if let Some(cached) = hierarchy.get(&(scene, id)) {
				cached.to_owned()
			} else {
				let entity = &scenes.scenes[&scene].1.entities[id];

				let cached = if let Some(parent) = entity
					.parent
					.as_ref()
					.map(|parent| resolve_instantiated_ref(&scenes.scenes, scene, parent))
					.and_then(|mut parents| (!parents.is_empty()).then(|| parents.swap_remove(0)))
				{
					std::iter::once(entity.source)
						.chain(get_hierarchy(scenes, hierarchy, parent))
						.collect()
				} else {
					vec![entity.source]
				};

				hierarchy.insert((scene, id), cached.to_owned());
				cached
			}
		}

		let mut new_entities_hierarchy = HashMap::default();

		if let Some(existing_scenes) = scenes.0.as_ref()
			&& let Some(existing_geometry) = geometry.0.as_ref()
		{
			// Key entities by hierarchy and factory since that should be unique and stable

			let new_entities_data = render
				.geometry
				.geometry
				.iter()
				.map(|(id, geom)| {
					(
						id,
						(
							get_hierarchy(&render.scenes, &mut new_entities_hierarchy, geom.source),
							match &render.scenes.scenes[&geom.source.0].1.entities[geom.source.1].factory {
								InstantiatedEntityFactory::Factory(id) => Some(*id),
								_ => None
							}
						)
					)
				})
				.collect::<HashMap<_, _>>();

			let new_entities = render
				.geometry
				.geometry
				.iter()
				.map(|(id, geom)| (new_entities_data[&id].to_owned(), (id, geom.to_owned())))
				.collect::<HashMap<_, _>>();

			let old_entities_data = existing_query
				.iter()
				.map(|(_, source, hierarchy, _)| {
					let instance_source = existing_geometry.geometry[source.0].source;

					(
						source.0,
						(
							hierarchy.0.to_owned(),
							match &existing_scenes.scenes[&instance_source.0].1.entities[instance_source.1].factory {
								InstantiatedEntityFactory::Factory(id) => Some(*id),
								_ => None
							}
						)
					)
				})
				.collect::<HashMap<_, _>>();

			let old_entities = existing_geometry
				.geometry
				.keys()
				.map(|id| old_entities_data[&id].to_owned())
				.collect::<HashSet<_>>();

			let mut existing_data_to_id: HashMap<
				(Vec<(RuntimeID, quickentity_rs::entity::EntityID)>, Option<RuntimeID>),
				Entity
			> = HashMap::default();

			let mut removed: HashSet<Entity> = HashSet::default();

			for (entity, source, hierarchy, parent) in existing_query.iter() {
				// Despawn if data is different or no longer exists

				let instance_source = existing_geometry.geometry[source.0].source;

				let data = (
					hierarchy.0.to_owned(),
					match &existing_scenes.scenes[&instance_source.0].1.entities[instance_source.1].factory {
						InstantiatedEntityFactory::Factory(id) => Some(*id),
						_ => None
					}
				);

				if let Some((new_id, new)) = new_entities.get(&data) {
					let old = &existing_geometry.geometry[source.0];
					if new.parent.map(|parent| new_entities_data.get(&parent))
						!= old.parent.map(|parent| old_entities_data.get(&parent))
						|| new.transform != old.transform
						|| new.data != old.data
					{
						if !removed.contains(&entity) {
							log::info!("Despawning {entity:?} (modified)");
							commands.entity(entity).try_despawn();
							to_spawn.push((parent.map(|x| x.0), *new_id));
							removed.insert(entity);
							removed.extend(descendants.iter_descendants(entity));
						}
					} else {
						if !removed.contains(&entity) {
							existing_data_to_id.insert(data, entity);
							commands.entity(entity).insert(SourceEntity(*new_id));
						}
					}
				} else {
					if !removed.contains(&entity) {
						log::info!("Despawning {entity:?} (removed)");
						commands.entity(entity).try_despawn();
						removed.insert(entity);
						removed.extend(descendants.iter_descendants(entity));
					}
				}
			}

			// Add any new entities to list
			for (new_entity_key, (id, geom)) in &new_entities {
				if !old_entities.contains(new_entity_key) {
					if let Some(parent) = geom.parent {
						if let Some(parent) = existing_data_to_id.get(&new_entities_data[&parent]).copied() {
							// Only spawn if we can find the parent, because otherwise that means the parent is also a new entity, so we'll spawn it recursively
							to_spawn.push((Some(parent), *id));
						}
					} else {
						to_spawn.push((None, *id));
					}
				}
			}

			to_spawn.retain(|(parent, _)| parent.is_none_or(|parent| !removed.contains(&parent)));
		} else {
			for (entity, _, _, _) in existing_query.iter() {
				log::info!("Despawning {entity:?}");
				commands.entity(entity).try_despawn();
			}

			to_spawn.extend(
				render
					.geometry
					.geometry
					.iter()
					.filter_map(|(id, geom)| geom.parent.is_none().then(|| (None, id)))
			);
		}

		commands.insert_resource(Scenes(Some(render.scenes.clone())));
		commands.insert_resource(Geometry(Some(render.geometry.clone())));

		let settings = Arc::new(render.settings);

		let pool = AsyncComputeTaskPool::get();

		log::info!("Spawning {} top-level entities", to_spawn.len());

		for (parent_id, geom_entity_id) in to_spawn {
			let geom_entity = render.geometry.geometry[geom_entity_id].to_owned();

			let hierarchy = get_hierarchy(&render.scenes, &mut new_entities_hierarchy, geom_entity.source);

			// log::info!(
			// 	"Spawning top-level entity {parent_id:?} -> {geom_entity_id:?} {:?}",
			// 	hierarchy.iter().take(5).collect_vec()
			// );

			let mut entity = commands.spawn((
				Clearable,
				Transform::IDENTITY,
				Visibility::default(),
				SourceEntity(geom_entity_id),
				SourceEntityHierarchy(hierarchy)
			));

			if let Some(parent_id) = parent_id {
				entity.insert(ChildOf(parent_id));
			}

			let entity = entity.id();

			let task = pool.spawn({
				let scenes = render.scenes.clone();
				let geometry = render.geometry.clone();
				let settings = settings.clone();
				async move { process_geom(scenes, geometry, settings, entity, geom_entity_id, geom_entity) }
			});

			commands.entity(entity).insert(Loading(task));
		}
	}
}

fn handle_select_system(
	mut commands: Commands,
	receiver: Res<BevyReceiver<to_bevy::Select>>,
	query: Query<(Entity, &SourceEntityHierarchy)>,
	selected: Query<Entity, With<Selected>>,
	descendants: Query<&Children>
) {
	if let Ok(event) = receiver.0.try_recv() {
		for entity in &selected {
			commands.entity(entity).remove::<Selected>();
		}

		let mut to_select = query
			.iter()
			.filter_map(|(entity, source)| source.0.contains(&(event.factory, event.id)).then_some(entity))
			.collect_vec();

		for entity in to_select.to_owned() {
			let descendants = descendants.iter_descendants(entity).collect_vec();
			to_select.retain(|x| !descendants.contains(x));
		}

		if let Some(entity) = to_select.first() {
			commands.entity(*entity).insert(TransformGizmoFocus);
		}

		for entity in to_select {
			commands.entity(entity).insert(Selected);
		}
	}
}

#[derive(Component)]
struct Loading(Task<CommandQueue>);

fn handle_tasks_system(mut commands: Commands, mut tasks: Query<(Entity, &mut Loading)>) {
	for (entity, mut task) in &mut tasks {
		if let Some(mut commands_queue) = check_ready(&mut task.0) {
			commands.append(&mut commands_queue);
			commands.entity(entity).remove::<Loading>();
		}
	}
}

fn keybind_center_system(
	mut commands: Commands,
	keyboard_input: Res<ButtonInput<KeyCode>>,
	mut camera: Query<(Entity, Option<&mut PanOrbitCamera>), With<TransformGizmoCamera>>,
	free_camera: Query<(&Transform, &PreferredRadius), With<FreeCamera>>,
	selected: Query<Entity, With<Selected>>,
	transform: Query<&GlobalTransform>,
	descendants: Query<&Children>,
	aabb: Query<&Aabb>
) {
	if keyboard_input.just_pressed(KeyCode::KeyZ) {
		if let Some(selected) = selected.iter().next() {
			if let Ok((camera_entity, orbit_camera)) = camera.single_mut() {
				let mut min = Vec3::splat(f32::MAX);
				let mut max = Vec3::splat(f32::MIN);
				for child in descendants.iter_descendants(selected) {
					if let Ok(aabb) = aabb.get(child) {
						if let Ok(global_transform) = transform.get(child) {
							let child_min = aabb.min();
							let child_max = aabb.max();

							fn get_box_corners(min: Vec3A, max: Vec3A) -> [Vec3; 8] {
								[
									Vec3::new(min.x, min.y, min.z),
									Vec3::new(max.x, min.y, min.z),
									Vec3::new(min.x, max.y, min.z),
									Vec3::new(min.x, min.y, max.z),
									Vec3::new(max.x, max.y, min.z),
									Vec3::new(min.x, max.y, max.z),
									Vec3::new(max.x, min.y, max.z),
									Vec3::new(max.x, max.y, max.z)
								]
							}

							for corner in get_box_corners(child_min, child_max) {
								let world_pos = global_transform.transform_point(corner);
								min = min.min(world_pos);
								max = max.max(world_pos);
							}
						}
					}
				}

				if min != Vec3::splat(f32::MAX) && max != Vec3::splat(f32::MIN) {
					if let Some(mut camera) = orbit_camera {
						camera.target_focus = (min + max) / 2.0;
						camera.target_radius = (max - min).max_element() + 1.0;
					} else if let Ok((transform, preferred_radius)) = free_camera.single() {
						commands
							.entity(camera_entity)
							.remove::<FreeCamera>()
							.insert(PanOrbitCamera {
								focus: transform.translation + (transform.forward() * preferred_radius.0),
								button_orbit: MouseButton::Right,
								modifier_pan: Some(KeyCode::ShiftLeft),
								..default()
							})
							.remove::<PreferredRadius>();

						commands
							.delayed()
							.secs(0.2)
							.entity(camera_entity)
							.queue(move |mut entity: EntityWorldMut| {
								if let Some(mut camera) = entity.get_mut::<PanOrbitCamera>() {
									camera.target_focus = (min + max) / 2.0;
									camera.target_radius = (max - min).max_element() + 1.0;
								}
							});
					}
				}
			}
		}
	}
}

#[derive(Component)]
struct PreferredRadius(f32);

fn keybind_switch_cam_system(
	keyboard_input: Res<ButtonInput<KeyCode>>,
	orbit_camera: Query<(Entity, &PanOrbitCamera)>,
	free_camera: Query<(Entity, &Transform, &PreferredRadius), With<FreeCamera>>,
	mut commands: Commands
) {
	if keyboard_input.just_pressed(KeyCode::KeyF) {
		if let Ok((camera, orbit)) = orbit_camera.single() {
			commands
				.entity(camera)
				.insert(PreferredRadius(orbit.radius.unwrap_or(10.0)))
				.remove::<PanOrbitCamera>()
				.insert(FreeCamera::default());
		} else if let Ok((camera, transform, preferred_radius)) = free_camera.single() {
			commands
				.entity(camera)
				.remove::<FreeCamera>()
				.insert(PanOrbitCamera {
					focus: transform.translation + (transform.forward() * preferred_radius.0),
					button_orbit: MouseButton::Right,
					modifier_pan: Some(KeyCode::ShiftLeft),
					..default()
				})
				.remove::<PreferredRadius>();
		}
	}
}

fn keybind_switch_transform_mode_system(
	keyboard_input: Res<ButtonInput<KeyCode>>,
	mut transform_settings: ResMut<TransformGizmoSettings>
) {
	if keyboard_input.just_pressed(KeyCode::Tab) {
		transform_settings.mode = match transform_settings.mode {
			TransformGizmoMode::Translate => TransformGizmoMode::Rotate,
			TransformGizmoMode::Rotate => TransformGizmoMode::Scale,
			TransformGizmoMode::Scale => TransformGizmoMode::Translate
		};
	}
}

fn keybind_switch_transform_space_system(
	keyboard_input: Res<ButtonInput<KeyCode>>,
	mut transform_settings: ResMut<TransformGizmoSettings>
) {
	if keyboard_input.just_pressed(KeyCode::Space) {
		transform_settings.space = match transform_settings.space {
			TransformGizmoSpace::Local => TransformGizmoSpace::World,
			TransformGizmoSpace::World => TransformGizmoSpace::Local
		};
	}
}

#[derive(Resource)]
struct TransformDebounce(Timer, Option<Vec<EditorEvent>>);

fn gizmo_transform_react_system(
	query: Single<(&SourceEntity, &Transform), (With<TransformGizmoFocus>, Changed<Transform>)>,
	game_files: Res<GameFiles>,
	scenes: Res<Scenes>,
	geometry: Res<Geometry>,
	mut debounce: ResMut<TransformDebounce>
) {
	if let Some(scenes) = scenes.0.as_ref()
		&& let Some(geometry) = geometry.0.as_ref()
		&& let Some(game_files) = game_files.0.read().as_ref()
	{
		let (source, transform) = query.deref();
		let geom = &geometry.geometry[source.0];
		let (scene, id) = geom.source;

		let mut events = vec![];

		if let Ok(Some((factory, id, property))) = get_property_source(
			game_files,
			scenes,
			scene,
			&scenes.scenes[&scene].1,
			None,
			id,
			&"m_mTransform".into()
		) {
			let mut transform =
				quickentity_rs::variant::Transform::from_glam(unconvert_transform(transform.compute_affine()), false);

			transform.scale = None;

			events.push(EditorEvent::UpdateProperty {
				entity: (factory, id),
				property,
				value: Variant::Transform(transform)
			});
		}

		if let GeomEntityData::CoverPlane { .. } = &geom.data {
			if let Ok(Some((factory, id, property))) = get_property_source(
				game_files,
				scenes,
				scene,
				&scenes.scenes[&scene].1,
				None,
				id,
				&"m_fCoverLength".into()
			) {
				let (Vec3 { x, .. }, _, _) =
					unconvert_transform(transform.compute_affine()).to_scale_rotation_translation();

				if (x * 100.0).round() != 100.0 {
					events.push(EditorEvent::UpdateProperty {
						entity: (factory, id),
						property,
						value: Variant::Raw(RawVariant::Unknown("float32".into(), serde_json::json!(x)))
					});
				}
			}

			if let Ok(Some((factory, id, property))) = get_property_source(
				game_files,
				scenes,
				scene,
				&scenes.scenes[&scene].1,
				None,
				id,
				&"m_fCoverDepth".into()
			) {
				let (Vec3 { mut y, .. }, _, _) =
					unconvert_transform(transform.compute_affine()).to_scale_rotation_translation();

				if y < 0.02 {
					y = 0.0;
				}

				if (y * 100.0).round() != 0.0 {
					events.push(EditorEvent::UpdateProperty {
						entity: (factory, id),
						property,
						value: Variant::Raw(RawVariant::Unknown("float32".into(), serde_json::json!(y)))
					});
				}
			}
		} else {
			let scale_prop = if let GeomEntityData::BoxVolume { .. } = &geom.data {
				"m_vGlobalSize"
			} else {
				"m_PrimitiveScale"
			};

			if let Ok(Some((factory, id, property))) = get_property_source(
				game_files,
				scenes,
				scene,
				&scenes.scenes[&scene].1,
				None,
				id,
				&scale_prop.into()
			) {
				let (Vec3 { x, y, z }, _, _) =
					unconvert_transform(transform.compute_affine()).to_scale_rotation_translation();

				if (x * 100.0).round() != 100.0 || (y * 100.0).round() != 100.0 || (z * 100.0).round() != 100.0 {
					events.push(EditorEvent::UpdateProperty {
						entity: (factory, id),
						property,
						value: Variant::Raw(RawVariant::Unknown("SVector3".into(), {
							serde_json::json!({
								"x": x,
								"y": y,
								"z": z
							})
						}))
					});
				}
			}
		}

		debounce.0.reset();
		debounce.1 = Some(events);
	}
}

fn transform_debounce_system(
	mut debounce: ResMut<TransformDebounce>,
	time: Res<Time>,
	sender: Res<BevySender<EditorEvent>>
) {
	if debounce.1.is_some() {
		debounce.0.tick(time.delta());

		if debounce.0.just_finished() {
			if let Some(events) = debounce.1.take() {
				for event in events {
					let _ = sender.0.send(event);
				}
			}
		}
	}
}
