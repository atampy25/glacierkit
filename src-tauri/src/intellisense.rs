use std::{
	collections::{HashMap, HashSet},
	path::PathBuf,
	sync::Arc
};

use anyhow::{Context, Result};
use async_recursion::async_recursion;
use fn_error_context::context;
use indexmap::IndexMap;
use itertools::Itertools;
use quickentity_rs::{
	qn_structs::{Entity, Ref},
	rt_structs::PropertyID,
	util_structs::ZGuidPropertyValue
};
use rpkg_rs::runtime::resource::resource_package::ResourcePackage;
use serde_json::{from_value, json, to_value, Value};
use tokio::sync::RwLock;
use tryvial::try_fn;

use crate::{
	game_detection::GameVersion,
	hash_list::HashList,
	resourcelib::{h2016_convert_cppt, h2_convert_cppt, h3_convert_cppt},
	rpkg::{extract_entity, extract_latest_metadata, extract_latest_resource, normalise_to_hash}
};

pub struct Intellisense {
	/// CPPT -> Property -> (Type, Value)
	pub cppt_properties: Arc<RwLock<HashMap<String, HashMap<String, (String, Value)>>>>,

	/// CPPT -> (Input, Output)
	pub cppt_pins: HashMap<String, (Vec<String>, Vec<String>)>,

	/// Property type as number -> String version
	pub uicb_prop_types: HashMap<u32, String>,

	pub all_cppts: HashSet<String>,
	pub all_asets: HashSet<String>,
	pub all_uicts: HashSet<String>,
	pub all_matts: HashSet<String>,
	pub all_wswts: HashSet<String>
}

impl Intellisense {
	#[try_fn]
	#[context("Couldn't get properties for CPPT {}", cppt)]
	async fn get_cppt_properties(
		&self,
		resource_packages: &IndexMap<PathBuf, ResourcePackage>,
		hash_list: &HashList,
		game_version: GameVersion,
		cppt: &str
	) -> Result<HashMap<String, (String, Value)>> {
		{
			if let Some(cached) = self.cppt_properties.read().await.get(cppt) {
				return Ok(cached.to_owned());
			}
		}

		let extracted = extract_latest_resource(resource_packages, hash_list, cppt)?;

		let cppt_data = match game_version {
			GameVersion::H1 => h2016_convert_cppt(&extracted.1)?,
			GameVersion::H2 => h2_convert_cppt(&extracted.1)?,
			GameVersion::H3 => h3_convert_cppt(&extracted.1)?
		};

		let mut guard = self.cppt_properties.write().await;
		guard.insert(
			cppt.into(),
			cppt_data
				.property_values
				.into_iter()
				.map(|property_value| {
					Ok((
						match property_value.n_property_id {
							PropertyID::Int(x) => x.to_string(),
							PropertyID::String(x) => x
						},
						(
							match property_value.value.property_type.as_ref() {
								"ZEntityReference" => "SEntityTemplateReference",
								"TArray<ZEntityReference>" => "TArray<SEntityTemplateReference>",
								x => x
							}
							.into(),
							match property_value.value.property_type.as_ref() {
								"ZRuntimeResourceID" => {
									let id_low = property_value
										.value
										.property_value
										.get("m_IDLow")
										.context("Invalid data")?
										.as_u64()
										.context("Invalid data")?;

									if id_low != 4294967295 {
										let reference = extracted
											.0
											.hash_reference_data
											.get(id_low as usize)
											.context("No such referenced resource")?;

										if reference.flag == "1F" {
											json!(reference.hash)
										} else {
											json!({
												"resource": reference.hash,
												"flag": reference.flag
											})
										}
									} else {
										Value::Null
									}
								}

								"ZEntityReference" => Value::Null,

								"TArray<ZEntityReference>" => json!([]),

								"ZGuid" => {
									let guid = from_value::<ZGuidPropertyValue>(property_value.value.property_value)
										.context("Invalid data")?;

									to_value(format!(
										"{:0>8x}-{:0>4x}-{:0>4x}-{:0>2x}{:0>2x}-{:0>2x}{:0>2x}{:0>2x}{:0>2x}{:0>2x}{:\
										 0>2x}",
										guid._a,
										guid._b,
										guid._c,
										guid._d,
										guid._e,
										guid._f,
										guid._g,
										guid._h,
										guid._i,
										guid._j,
										guid._k
									))?
								}

								"SColorRGB" => {
									let map = property_value
										.value
										.property_value
										.as_object()
										.context("SColorRGB was not an object")?;

									to_value(format!(
										"#{:0>2x}{:0>2x}{:0>2x}",
										(map.get("r")
											.context("Colour did not have required key r")?
											.as_f64()
											.context("Invalid data")? * 255.0)
											.round() as u8,
										(map.get("g")
											.context("Colour did not have required key g")?
											.as_f64()
											.context("Invalid data")? * 255.0)
											.round() as u8,
										(map.get("b")
											.context("Colour did not have required key b")?
											.as_f64()
											.context("Invalid data")? * 255.0)
											.round() as u8
									))?
								}

								"SColorRGBA" => {
									let map = property_value
										.value
										.property_value
										.as_object()
										.context("SColorRGBA was not an object")?;

									to_value(format!(
										"#{:0>2x}{:0>2x}{:0>2x}{:0>2x}",
										(map.get("r")
											.context("Colour did not have required key r")?
											.as_f64()
											.context("Invalid data")? * 255.0)
											.round() as u8,
										(map.get("g")
											.context("Colour did not have required key g")?
											.as_f64()
											.context("Invalid data")? * 255.0)
											.round() as u8,
										(map.get("b")
											.context("Colour did not have required key b")?
											.as_f64()
											.context("Invalid data")? * 255.0)
											.round() as u8,
										(map.get("a")
											.context("Colour did not have required key a")?
											.as_f64()
											.context("Invalid data")? * 255.0)
											.round() as u8
									))?
								}

								_ => property_value.value.property_value
							}
						)
					))
				})
				.collect::<Result<_>>()?
		);

		guard.get(cppt).unwrap().to_owned()
	}

	/// Get the names, types, default values and post-init status of all properties of a given sub-entity.
	#[try_fn]
	#[context("Couldn't get properties for sub-entity {} in {}", sub_entity, entity.factory_hash)]
	#[async_recursion]
	pub async fn get_properties(
		&self,
		resource_packages: &IndexMap<PathBuf, ResourcePackage>,
		cached_entities: &RwLock<HashMap<String, Entity>>,
		hash_list: &HashList,
		game_version: GameVersion,
		entity: &Entity,
		sub_entity: &str,
		ignore_own: bool
	) -> Result<Vec<(String, String, Value, bool)>> {
		let targeted = entity.entities.get(sub_entity).context("No such sub-entity")?;

		let mut found = vec![];

		for (aliased_name, aliases) in targeted.property_aliases.as_ref().unwrap_or(&Default::default()) {
			for alias in aliases {
				if let Ref::Short(Some(ent)) = &alias.original_entity {
					let data = self
						.get_properties(
							resource_packages,
							cached_entities,
							hash_list,
							game_version,
							entity,
							ent,
							false
						)
						.await?
						.into_iter()
						.find(|(x, _, _, _)| *x == alias.original_property)
						.context("Couldn't find property data for aliased property")?;

					found.push((aliased_name.to_owned(), data.1, data.2, data.3));
					break;
				}
			}
		}

		if !ignore_own {
			for (property, property_data) in targeted.properties.as_ref().unwrap_or(&Default::default()) {
				found.push((
					property.to_owned(),
					property_data.property_type.to_owned(),
					property_data.value.to_owned(),
					property_data.post_init.unwrap_or(false)
				));
			}
		}

		for factory in {
			if self.all_asets.contains(&normalise_to_hash(&targeted.factory)) {
				extract_latest_metadata(resource_packages, hash_list, &normalise_to_hash(&targeted.factory))?
					.hash_reference_data
					.into_iter()
					.rev()
					.skip(1)
					.rev()
					.map(|x| x.hash.to_owned())
					.collect_vec()
			} else {
				vec![normalise_to_hash(&targeted.factory)]
			}
		} {
			if self.all_cppts.contains(&factory) {
				for (prop_name, (prop_type, default_val)) in self
					.get_cppt_properties(resource_packages, hash_list, game_version, &factory)
					.await?
				{
					found.push((prop_name, prop_type, default_val, false));
				}
			} else if self.all_uicts.contains(&factory) {
				// All UI controls have the properties of ZUIControlEntity
				for (prop_name, (prop_type, default_val)) in self
					.get_cppt_properties(resource_packages, hash_list, game_version, "002C4526CC9753E6")
					.await?
				{
					found.push((prop_name, prop_type, default_val, false));
				}

			// TODO: Read UICB
			} else if self.all_matts.contains(&factory) {
				// All materials have the properties of ZRenderMaterialEntity
				for (prop_name, (prop_type, default_val)) in self
					.get_cppt_properties(resource_packages, hash_list, game_version, "00B4B11DA327CAD0")
					.await?
				{
					found.push((prop_name, prop_type, default_val, false));
				}

			// TODO: Read material info
			} else if self.all_wswts.contains(&factory) {
				// All switch groups have the properties of ZAudioSwitchEntity
				for (prop_name, (prop_type, default_val)) in self
					.get_cppt_properties(resource_packages, hash_list, game_version, "00797DC916520C4D")
					.await?
				{
					found.push((prop_name, prop_type, default_val, false));
				}
			} else {
				let extracted =
					extract_entity(resource_packages, cached_entities, game_version, hash_list, &factory).await?;

				found.extend(
					self.get_properties(
						resource_packages,
						cached_entities,
						hash_list,
						game_version,
						&extracted,
						&extracted.root_entity,
						false
					)
					.await?
				);
			}
		}

		found
	}
}
