use std::sync::Arc;

use anyhow::{Context, Error, Result, bail};
use ecow::{EcoString, eco_format};
use fn_error_context::context;
use glacier_bin1::game::h3::{IRenderMaterialEntity_EModifierOperation, SVector2, SVector3, SVector4, ZVariant};
use hitman_commons::{
	game::GameVersion,
	metadata::{ReferenceFlags, ReferenceType, ResourceReference, RuntimeID},
	rid
};
use hitman_formats::material::{MaterialEntity, MaterialOverride};
use identity_hash::BuildIdentityHasher;
use indexmap::IndexMap;
use itertools::Itertools;
use quickentity_rs::{
	entity::{Entity, EntityID},
	variant::Variant
};
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use tryvial::try_fn;

use crate::{HashMap, PapayaMap, game::Game};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CPPTPinsInfo {
	pub inputs: Vec<CPPTPinInfo>,
	pub outputs: Vec<CPPTPinInfo>
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CPPTPinInfo {
	#[serde(rename = "pin")]
	pub name: EcoString,

	pub description: EcoString
}

pub struct Intellisense {
	/// CPPT -> Property -> Value
	pub cppt_properties: Arc<PapayaMap<RuntimeID, HashMap<EcoString, Variant>, BuildIdentityHasher<u64>>>,

	pub cppt_pins: HashMap<RuntimeID, CPPTPinsInfo, BuildIdentityHasher<u64>>,

	pub matt_properties: Arc<PapayaMap<RuntimeID, IndexMap<EcoString, MaterialOverride>, BuildIdentityHasher<u64>>>
}

impl Default for Intellisense {
	fn default() -> Self {
		Self::new()
	}
}

impl Intellisense {
	pub fn new() -> Self {
		#[derive(Serialize, Deserialize)]
		struct DeserialisedCPPTPinsInfo {
			pub hash: RuntimeID,

			#[serde(rename = "in")]
			pub inputs: Vec<CPPTPinInfo>,

			#[serde(rename = "out")]
			pub outputs: Vec<CPPTPinInfo>
		}

		Self {
			cppt_properties: Arc::new(Default::default()),
			cppt_pins: serde_json::from_slice::<Vec<DeserialisedCPPTPinsInfo>>(include_bytes!("../assets/pins.json"))
				.unwrap()
				.into_iter()
				.map(|info| {
					(
						info.hash,
						CPPTPinsInfo {
							inputs: info.inputs,
							outputs: info.outputs
						}
					)
				})
				.collect(),
			matt_properties: Arc::new(Default::default())
		}
	}

	#[try_fn]
	#[context("Couldn't get properties for CPPT {}", cppt)]
	fn get_cppt_properties(&self, game: &Game, cppt: RuntimeID) -> Result<HashMap<EcoString, Variant>> {
		{
			if let Some(cached) = self.cppt_properties.pin().get(&cppt) {
				return Ok(cached.to_owned());
			}
		}

		let (cppt_meta, cppt_data) = game.extract_latest_resource(cppt)?;

		macro_rules! generate {
			($game:ident) => {{
				let cppt_data = glacier_bin1::deserialize::<glacier_bin1::game::$game::SCppEntity>(&cppt_data)
					.context("Couldn't deserialise CPPT")?;

				self.cppt_properties.pin().insert(
					cppt,
					cppt_data
						.property_values
						.into_iter()
						.map(|property_value| {
							Ok((
								property_value
									.property_id
									.as_name()
									.unwrap_or_else(|| property_value.property_id.0.to_string().into()),
								Variant::from_game(
									&serde_json::from_value(serde_json::to_value(&match property_value
										.value
										.variant_type()
										.as_ref()
									{
										"ZEntityReference" => glacier_bin1::game::$game::ZVariant::new(
											glacier_bin1::game::$game::SEntityTemplateReference {
												entity_id: u64::MAX,
												entity_index: -1,
												exposed_entity: "".into(),
												external_scene_index: -1
											}
										),
										"TArray<ZEntityReference>" => glacier_bin1::game::$game::ZVariant::new(
											vec![] as Vec<glacier_bin1::game::$game::SEntityTemplateReference>
										),
										_ => property_value.value
									})?)?,
									&glacier_bin1::game::h3::STemplateEntityFactory {
										blueprint_index_in_resource_header: 0,
										root_entity_index: 0,
										sub_type: 0,
										sub_entities: vec![],
										external_scene_type_indices_in_resource_header: vec![],
										property_overrides: vec![]
									},
									&cppt_meta.core_info,
									&glacier_bin1::game::h3::STemplateEntityBlueprint {
										sub_type: 0,
										root_entity_index: 0,
										sub_entities: vec![],
										external_scene_type_indices_in_resource_header: vec![],
										pin_connections: vec![],
										input_pin_forwardings: vec![],
										output_pin_forwardings: vec![],
										override_deletes: vec![],
										pin_connection_overrides: vec![],
										pin_connection_override_deletes: vec![]
									},
									false
								)?
							))
						})
						.collect::<Result<_>>()?
				);
			}};
		}

		match game.version() {
			GameVersion::H1 => generate!(h1),
			GameVersion::H2 => generate!(h2),
			GameVersion::H3 => generate!(h3)
		}

		self.cppt_properties
			.pin()
			.get(&cppt)
			.expect("We just added it")
			.to_owned()
	}

	#[try_fn]
	#[context("Couldn't get properties for MATT {}", matt)]
	fn get_matt_properties(&self, game: &Game, matt: RuntimeID) -> Result<IndexMap<EcoString, MaterialOverride>> {
		{
			if let Some(x) = self.matt_properties.pin().get(&matt) {
				return Ok(x.to_owned());
			}
		}

		let (matt_meta, matt_data) = game.extract_latest_resource(matt)?;

		let (matb_meta, matb_data) = game.extract_latest_resource(
			matt_meta
				.core_info
				.references
				.iter()
				.find(|x| game.resource_type(x.resource).is_some_and(|ty| ty == "MATB"))
				.context("MATT has no MATB dependency")?
				.resource
		)?;

		self.matt_properties.pin().insert(
			matt,
			MaterialEntity::parse(&matt_data, &matt_meta.core_info, &matb_data, &matb_meta.core_info)?
				.overrides
				.into_iter()
				.map(|(x, y)| (x.into(), y))
				.collect()
		);

		self.matt_properties
			.pin()
			.get(&matt)
			.expect("We just added it")
			.to_owned()
	}

	/// Get the names, default values and post-init status of all properties of a given sub-entity.
	///
	/// May deadlock if a reference is already held on `cached_entities` by the same thread.
	#[try_fn]
	#[context("Couldn't get properties for sub-entity {} in {}", sub_entity, entity.factory)]
	pub fn get_properties(
		&self,
		game: &Game,
		entity: &Entity,
		sub_entity: EntityID,
		ignore_own: bool
	) -> Result<Vec<(EcoString, Variant, bool)>> {
		let targeted = entity.entities.get(&sub_entity).context("No such sub-entity")?;

		let mut found = vec![];

		if !ignore_own {
			for (property, property_data) in &targeted.properties {
				found.push((
					property.to_owned(),
					property_data.value.to_owned(),
					property_data.post_init
				));
			}
		}

		let (a, b) = rayon::join(
			|| {
				anyhow::Ok(
					targeted
						.property_aliases
						.par_iter()
						.map(|(aliased_name, aliases)| {
							Ok({
								let mut found = vec![];
								for alias in aliases {
									if let Some(data) = self.get_specific_property(
										game,
										entity,
										alias.original_entity,
										&alias.original_property
									)? {
										found.push((aliased_name.to_owned(), data.0, data.1));
										break;
									}
								}

								found
							})
						})
						.collect::<Result<Vec<_>>>()?
						.into_iter()
						.flatten()
						.collect_vec()
				)
			},
			|| {
				let mut found = vec![];

				found.extend(
					{
						if let Some(ty) = game.resource_type(targeted.factory.resource)
							&& ty == "ASET"
						{
							game.extract_latest_metadata(targeted.factory.resource)?
								.core_info
								.references
								.into_iter()
								.rev()
								.skip(1)
								.rev()
								.map(|x| x.resource)
								.collect_vec()
						} else {
							vec![targeted.factory.resource.to_owned()]
						}
					}
					.into_par_iter()
					.map(|factory| {
						Ok({
							let mut found = vec![];

							if let Some(ty) = game.resource_type(factory) {
								match ty.as_ref() {
									"CPPT" => {
										for (prop_name, default_val) in self.get_cppt_properties(game, factory)? {
											found.push((prop_name, default_val, false));
										}
									}

									"UICT" => {
										// All UI controls have the properties of ZUIControlEntity
										for (prop_name, default_val) in self.get_cppt_properties(
											game,
											rid!("[modules:/zuicontrolentity.class].pc_entitytype")
										)? {
											found.push((prop_name, default_val, false));
										}

										macro_rules! generate {
											($game:ident) => {{
												for entry in glacier_bin1::deserialize::<glacier_bin1::game::$game::SControlTypeInfo>(
													&game.extract_latest_resource(
														game.extract_latest_metadata(factory)?
															.core_info
															.references
															.into_iter()
															.find(|x| {
																game.resource_type(x.resource)
																	.is_some_and(|ty| ty == "UICB")
															})
															.context("No blueprint dependency on UICT")?
															.resource
													)?
													.1
												)?
												.attributes
												{
													// Property
													if entry.kind == 0 {
														// We can't get the actual default values, if there are any, so we just use sensible defaults
														found.push((
															entry.name,
															match entry.r#type {
																0 => Variant::from_raw(&ZVariant::new(())),
																1 => Variant::from_raw(&ZVariant::new(0i32)),
																2 => Variant::from_raw(&ZVariant::new(0.0f32)),
																3 => Variant::from_raw(&ZVariant::new(EcoString::new())),
																4 => Variant::from_raw(&ZVariant::new(false)),
																5 => Variant::Ref(None),
																6 => Variant::Ref(None),
																_ => bail!("Unknown UICB property type {}", entry.r#type)
															},
															false
														));
													}
												}
											}}
										}

										match game.version() {
											GameVersion::H1 => generate!(h1),
											GameVersion::H2 => generate!(h2),
											GameVersion::H3 => generate!(h3)
										}
									}

									"MATT" => {
										// All materials have the properties of ZRenderMaterialEntity
										for (prop_name, default_val) in self.get_cppt_properties(
											game,
											rid!("[modules:/zrendermaterialentity.class].pc_entitytype")
										)? {
											found.push((prop_name, default_val, false));
										}

										for (property_name, property_data) in self.get_matt_properties(game, factory)? {
											match property_data {
												MaterialOverride::Texture(texture) => {
													found.push((
														property_name.to_owned(),
														Variant::Resource(texture.map(|texture| ResourceReference {
															resource: texture,
															flags: ReferenceFlags {
																reference_type: ReferenceType::Normal,
																..Default::default()
															}
														})),
														false
													));

													found.push((
														eco_format!("{}_enab", property_name),
														Variant::from_raw(&ZVariant::new(false)),
														false
													));

													found.push((
														eco_format!("{}_dest", property_name),
														Variant::Ref(None),
														false
													));
												}

												MaterialOverride::Color(value) => {
													found.push((
														property_name.to_owned(),
														if value.len() > 7 {
															Variant::ColorRGBA(value.parse().map_err(Error::msg)?)
														} else {
															Variant::ColorRGB(value.parse().map_err(Error::msg)?)
														},
														false
													));

													found.push((
														eco_format!("{}_op", property_name),
														Variant::from_raw(&ZVariant::new(
															IRenderMaterialEntity_EModifierOperation::eLeave
														)),
														false
													));
												}

												MaterialOverride::Float(val) => {
													found.push((
														property_name.to_owned(),
														Variant::from_raw(&ZVariant::new(val)),
														false
													));

													found.push((
														eco_format!("{}_op", property_name),
														Variant::from_raw(&ZVariant::new(
															IRenderMaterialEntity_EModifierOperation::eLeave
														)),
														false
													));
												}

												MaterialOverride::Vector(vec) => {
													found.push((
														property_name.to_owned(),
														match vec.len() {
															2 => Variant::from_raw(&ZVariant::new(SVector2 {
																x: vec[0],
																y: vec[1]
															})),
															3 => Variant::from_raw(&ZVariant::new(SVector3 {
																x: vec[0],
																y: vec[1],
																z: vec[2]
															})),
															4 => Variant::from_raw(&ZVariant::new(SVector4 {
																x: vec[0],
																y: vec[1],
																z: vec[2],
																w: vec[3]
															})),
															_ => bail!("Invalid vector length")
														},
														false
													));

													found.push((
														eco_format!("{}_op", property_name),
														Variant::from_raw(&ZVariant::new(
															IRenderMaterialEntity_EModifierOperation::eLeave
														)),
														false
													));
												}
											}
										}
									}

									"WSWT" => {
										// All switch groups have the properties of ZAudioSwitchEntity
										for (prop_name, default_val) in self.get_cppt_properties(
											game,
											rid!("[modules:/zaudioswitchentity.class].pc_entitytype")
										)? {
											found.push((prop_name, default_val, false));
										}
									}

									"ECPT" => {
										// All extended CPP entities have the properties of ZMaterialOverwriteAspect
										for (prop_name, default_val) in self.get_cppt_properties(
											game,
											rid!("[modules:/zmaterialoverwriteaspect.class].pc_entitytype")
										)? {
											found.push((prop_name, default_val, false));
										}

										let ecpb_data = game
											.extract_latest_resource(
												game.extract_latest_metadata(factory)?
													.core_info
													.references
													.into_iter()
													.find(|x| {
														game.resource_type(x.resource).is_some_and(|ty| ty == "ECPB")
													})
													.context("No blueprint dependency on ECPT")?
													.resource
											)?
											.1;

										macro_rules! generate {
											($game:ident) => {{
												for entry in glacier_bin1::deserialize::<
													glacier_bin1::game::$game::SExtendedCppEntityBlueprint
												>(&ecpb_data)?
												.properties
												{
													use glacier_bin1::game::$game::EExtendedPropertyType;
													found.push((
														entry.property_name.into(),
														match entry.property_type {
															EExtendedPropertyType::Resourceptr => {
																Variant::Resource(None)
															}
															EExtendedPropertyType::Int32 => {
																Variant::from_raw(&ZVariant::new(0i32))
															}
															EExtendedPropertyType::Uint32 => {
																Variant::from_raw(&ZVariant::new(0u32))
															}
															EExtendedPropertyType::Float => {
																Variant::from_raw(&ZVariant::new(0.0f32))
															}
															EExtendedPropertyType::String => {
																Variant::from_raw(&ZVariant::new(EcoString::from("")))
															}
															EExtendedPropertyType::Bool => {
																Variant::from_raw(&ZVariant::new(false))
															}
															EExtendedPropertyType::Entityref => Variant::Ref(None),
															EExtendedPropertyType::Variant => {
																Variant::Variant(Variant::Ref(None).into())
															}
														},
														false
													));
												}
											}};
										}

										match game.version() {
											GameVersion::H1 => {
												// ECPB files don't exist in H1
											}
											GameVersion::H2 => generate!(h2),
											GameVersion::H3 => generate!(h3)
										}
									}

									"AIBX" => {
										// All behaviour trees have the properties of ZBehaviorTreeEntity
										for (prop_name, default_val) in self.get_cppt_properties(
											game,
											rid!("[modules:/zbehaviortreeentity.class].pc_entitytype")
										)? {
											found.push((prop_name, default_val, false));
										}
									}

									"WSGT" => {
										// All state groups have the properties of ZAudioStateEntity
										for (prop_name, default_val) in self.get_cppt_properties(
											game,
											rid!("[modules:/zaudiostateentity.class].pc_entitytype")
										)? {
											found.push((prop_name, default_val, false));
										}
									}

									"TEMP" => {
										let extracted = game.extract_entity(factory)?;

										found.extend(self.get_properties(
											game,
											&extracted,
											extracted.root_entity,
											false
										)?);
									}

									_ => bail!("Unknown factory type")
								}
							}

							found
						})
					})
					.collect::<Result<Vec<_>>>()?
					.into_iter()
					.flatten()
				);

				anyhow::Ok(found)
			}
		);

		found.extend(a?);
		found.extend(b?);

		found.into_iter().unique_by(|x| x.0.to_owned()).collect()
	}

	/// Get the default value and post-init status of a single property of a given sub-entity, by its name.
	///
	/// May deadlock if a reference is already held on `cached_entities` by the same thread.
	#[try_fn]
	#[context("Couldn't get property {} of sub-entity {} in {}", property_to_find, sub_entity, entity.factory)]
	pub fn get_specific_property(
		&self,
		game: &Game,
		entity: &Entity,
		sub_entity: EntityID,
		property_to_find: &str
	) -> Result<Option<(Variant, bool)>> {
		let targeted = entity.entities.get(&sub_entity).context("No such sub-entity")?;

		if let Some(aliases) = targeted.property_aliases.get(property_to_find) {
			for alias in aliases {
				// Avoids issues from an entity having a property alias to itself
				if alias.original_entity != sub_entity
					&& property_to_find == alias.original_property
					&& let Some(data) =
						self.get_specific_property(game, entity, alias.original_entity, &alias.original_property)?
				{
					return Ok(Some(data));
				}
			}
		}

		if let Some(property_data) = targeted.properties.get(property_to_find) {
			return Ok(Some((property_data.value.to_owned(), property_data.post_init)));
		}

		for factory in if let Some(ty) = game.resource_type(targeted.factory.resource)
			&& ty == "ASET"
		{
			game.extract_latest_metadata(targeted.factory.resource)?
				.core_info
				.references
				.into_iter()
				.rev()
				.skip(1)
				.rev()
				.map(|x| x.resource)
				.collect_vec()
		} else {
			vec![targeted.factory.resource.to_owned()]
		} {
			if let Some(ty) = game.resource_type(factory) {
				match ty.as_ref() {
					"CPPT" => {
						for (prop_name, default_val) in self.get_cppt_properties(game, factory)? {
							if prop_name == property_to_find {
								return Ok(Some((default_val, false)));
							}
						}
					}

					"UICT" => {
						// All UI controls have the properties of ZUIControlEntity
						for (prop_name, default_val) in
							self.get_cppt_properties(game, rid!("[modules:/zuicontrolentity.class].pc_entitytype"))?
						{
							if prop_name == property_to_find {
								return Ok(Some((default_val, false)));
							}
						}

						macro_rules! generate {
							($game:ident) => {{
								for entry in glacier_bin1::deserialize::<glacier_bin1::game::$game::SControlTypeInfo>(
									&game.extract_latest_resource(
										game.extract_latest_metadata(factory)?
											.core_info
											.references
											.into_iter()
											.find(|x| game.resource_type(x.resource).is_some_and(|ty| ty == "UICB"))
											.context("No blueprint dependency on UICT")?
											.resource
									)?
									.1
								)?
								.attributes
								{
									// Property
									if entry.kind == 0 && entry.name == property_to_find {
										// We can't get the actual default values, if there are any, so we just use sensible defaults
										return Ok(Some((
											match entry.r#type {
												0 => Variant::from_raw(&ZVariant::new(())),
												1 => Variant::from_raw(&ZVariant::new(0i32)),
												2 => Variant::from_raw(&ZVariant::new(0.0f32)),
												3 => Variant::from_raw(&ZVariant::new(EcoString::new())),
												4 => Variant::from_raw(&ZVariant::new(false)),
												5 => Variant::Ref(None),
												6 => Variant::Ref(None),
												_ => bail!("Unknown UICB property type {}", entry.r#type)
											},
											false
										)));
									}
								}
							}}
						}

						match game.version() {
							GameVersion::H1 => generate!(h1),
							GameVersion::H2 => generate!(h2),
							GameVersion::H3 => generate!(h3)
						}
					}

					"MATT" => {
						// All materials have the properties of ZRenderMaterialEntity
						for (prop_name, default_val) in self
							.get_cppt_properties(game, rid!("[modules:/zrendermaterialentity.class].pc_entitytype"))?
						{
							if prop_name == property_to_find {
								return Ok(Some((default_val, false)));
							}
						}

						for (property_name, property_data) in self.get_matt_properties(game, factory)? {
							match property_data {
								MaterialOverride::Texture(texture) => {
									if property_name == property_to_find {
										return Ok(Some((
											Variant::Resource(texture.map(|texture| ResourceReference {
												resource: texture,
												flags: ReferenceFlags {
													reference_type: ReferenceType::Normal,
													..Default::default()
												}
											})),
											false
										)));
									}

									if format!("{}_enab", property_name) == property_to_find {
										return Ok(Some((Variant::from_raw(&ZVariant::new(false)), false)));
									}

									if format!("{}_dest", property_name) == property_to_find {
										return Ok(Some((Variant::Ref(None), false)));
									}
								}

								MaterialOverride::Color(value) => {
									if property_name == property_to_find {
										return Ok(Some((
											if value.len() > 7 {
												Variant::ColorRGBA(value.parse().map_err(Error::msg)?)
											} else {
												Variant::ColorRGB(value.parse().map_err(Error::msg)?)
											},
											false
										)));
									}

									if format!("{}_op", property_name) == property_to_find {
										return Ok(Some((
											Variant::from_raw(&ZVariant::new(
												IRenderMaterialEntity_EModifierOperation::eLeave
											)),
											false
										)));
									}
								}

								MaterialOverride::Float(val) => {
									if property_name == property_to_find {
										return Ok(Some((Variant::from_raw(&ZVariant::new(val)), false)));
									}

									if format!("{}_op", property_name) == property_to_find {
										return Ok(Some((
											Variant::from_raw(&ZVariant::new(
												IRenderMaterialEntity_EModifierOperation::eLeave
											)),
											false
										)));
									}
								}

								MaterialOverride::Vector(vec) => {
									if property_name == property_to_find {
										return Ok(Some((
											match vec.len() {
												2 => {
													Variant::from_raw(&ZVariant::new(SVector2 { x: vec[0], y: vec[1] }))
												}
												3 => Variant::from_raw(&ZVariant::new(SVector3 {
													x: vec[0],
													y: vec[1],
													z: vec[2]
												})),
												4 => Variant::from_raw(&ZVariant::new(SVector4 {
													x: vec[0],
													y: vec[1],
													z: vec[2],
													w: vec[3]
												})),
												_ => bail!("Invalid vector length")
											},
											false
										)));
									}

									if format!("{}_op", property_name) == property_to_find {
										return Ok(Some((
											Variant::from_raw(&ZVariant::new(
												IRenderMaterialEntity_EModifierOperation::eLeave
											)),
											false
										)));
									}
								}
							}
						}
					}

					"WSWT" => {
						// All switch groups have the properties of ZAudioSwitchEntity
						for (prop_name, default_val) in
							self.get_cppt_properties(game, rid!("[modules:/zaudioswitchentity.class].pc_entitytype"))?
						{
							if prop_name == property_to_find {
								return Ok(Some((default_val, false)));
							}
						}
					}

					"ECPT" => {
						// All extended CPP entities have the properties of ZMaterialOverwriteAspect
						for (prop_name, default_val) in self.get_cppt_properties(
							game,
							rid!("[modules:/zmaterialoverwriteaspect.class].pc_entitytype")
						)? {
							if prop_name == property_to_find {
								return Ok(Some((default_val, false)));
							}
						}

						let ecpb_data = game
							.extract_latest_resource(
								game.extract_latest_metadata(factory)?
									.core_info
									.references
									.into_iter()
									.find(|x| game.resource_type(x.resource).is_some_and(|ty| ty == "ECPB"))
									.context("No blueprint dependency on ECPT")?
									.resource
							)?
							.1;

						macro_rules! generate {
							($game:ident) => {{
								for entry in glacier_bin1::deserialize::<
									glacier_bin1::game::$game::SExtendedCppEntityBlueprint
								>(&ecpb_data)?
								.properties
								{
									use glacier_bin1::game::$game::EExtendedPropertyType;
									if entry.property_name == property_to_find {
										return Ok(Some((
											match entry.property_type {
												EExtendedPropertyType::Resourceptr => Variant::Resource(None),
												EExtendedPropertyType::Int32 => Variant::from_raw(&ZVariant::new(0i32)),
												EExtendedPropertyType::Uint32 => {
													Variant::from_raw(&ZVariant::new(0u32))
												}
												EExtendedPropertyType::Float => {
													Variant::from_raw(&ZVariant::new(0.0f32))
												}
												EExtendedPropertyType::String => {
													Variant::from_raw(&ZVariant::new(EcoString::from("")))
												}
												EExtendedPropertyType::Bool => Variant::from_raw(&ZVariant::new(false)),
												EExtendedPropertyType::Entityref => Variant::Ref(None),
												EExtendedPropertyType::Variant => {
													Variant::Variant(Variant::Ref(None).into())
												}
											},
											false
										)));
									}
								}
							}};
						}

						match game.version() {
							GameVersion::H1 => {
								// ECPB files don't exist in H1
							}
							GameVersion::H2 => generate!(h2),
							GameVersion::H3 => generate!(h3)
						}
					}

					"AIBX" => {
						// All behaviour trees have the properties of ZBehaviorTreeEntity
						for (prop_name, default_val) in
							self.get_cppt_properties(game, rid!("[modules:/zbehaviortreeentity.class].pc_entitytype"))?
						{
							if prop_name == property_to_find {
								return Ok(Some((default_val, false)));
							}
						}
					}

					"WSGT" => {
						// All state groups have the properties of ZAudioStateEntity
						for (prop_name, default_val) in
							self.get_cppt_properties(game, rid!("[modules:/zaudiostateentity.class].pc_entitytype"))?
						{
							if prop_name == property_to_find {
								return Ok(Some((default_val, false)));
							}
						}
					}

					"TEMP" => {
						let extracted = game.extract_entity(factory)?;

						if let Some(data) =
							self.get_specific_property(game, &extracted, extracted.root_entity, property_to_find)?
						{
							return Ok(Some(data));
						}
					}

					_ => bail!("Unknown factory type")
				}
			}
		}

		None
	}

	/// Get the names of all input and output pins of a given sub-entity.
	#[try_fn]
	#[context("Couldn't get pins for sub-entity {} in {}", sub_entity, entity.factory)]
	pub fn get_pins(
		&self,
		game: &Game,
		entity: &Entity,
		sub_entity: EntityID,
		ignore_own: bool
	) -> Result<(Vec<EcoString>, Vec<EcoString>)> {
		let targeted = entity.entities.get(&sub_entity).context("No such sub-entity")?;

		let mut input = vec![];
		let mut output = vec![];

		if !ignore_own {
			input.extend(targeted.input_forwardings.keys().cloned());

			output.extend(targeted.events.keys().cloned());

			output.extend(targeted.output_forwardings.keys().cloned());
		}

		for sub_data in entity.entities.values() {
			for data in sub_data.events.values() {
				for (trigger, refs) in data {
					for reference in refs {
						if reference.entity_ref.as_local().is_some_and(|x| x == sub_entity) {
							input.push(trigger.to_owned());
						}
					}
				}
			}

			for data in sub_data.input_forwardings.values() {
				for (trigger, refs) in data {
					for reference in refs {
						if reference.entity_id == sub_entity {
							input.push(trigger.to_owned());
						}
					}
				}
			}

			for data in sub_data.output_forwardings.values() {
				for (propagate, refs) in data {
					for reference in refs {
						if reference.entity_id == sub_entity {
							output.push(propagate.to_owned());
						}
					}
				}
			}
		}

		let (fac_input, fac_output): (Vec<_>, Vec<_>) = {
			if let Some(ty) = game.resource_type(targeted.factory.resource)
				&& ty == "ASET"
			{
				game.extract_latest_metadata(targeted.factory.resource)?
					.core_info
					.references
					.into_iter()
					.rev()
					.skip(1)
					.rev()
					.map(|x| x.resource)
					.collect_vec()
			} else {
				vec![targeted.factory.resource.to_owned()]
			}
		}
		.into_par_iter()
		.map(|factory| {
			Ok({
				let mut input = vec![];
				let mut output = vec![];

				if let Some(ty) = game.resource_type(factory) {
					match ty.as_ref() {
						"CPPT" => {
							if let Some(cppt_data) = self.cppt_pins.get(&factory) {
								input.extend(cppt_data.inputs.iter().map(|x| &x.name).cloned());
								output.extend(cppt_data.outputs.iter().map(|x| &x.name).cloned());
							}
						}

						"UICT" => {
							// All UI controls have the pins of ZUIControlEntity
							let cppt_data = self
								.cppt_pins
								.get(&rid!("[modules:/zuicontrolentity.class].pc_entitytype"))
								.context("No such CPPT in pins")?;
							input.extend(cppt_data.inputs.iter().map(|x| &x.name).cloned());
							output.extend(cppt_data.outputs.iter().map(|x| &x.name).cloned());

							macro_rules! generate {
								($game:ident) => {{
									for entry in
										glacier_bin1::deserialize::<glacier_bin1::game::$game::SControlTypeInfo>(
											&game
												.extract_latest_resource(
													game.extract_latest_metadata(factory)?
														.core_info
														.references
														.into_iter()
														.find(|x| {
															game.resource_type(x.resource)
																.is_some_and(|ty| ty == "UICB")
														})
														.context("No blueprint dependency on UICT")?
														.resource
												)?
												.1
										)?
										.attributes
									{
										if entry.kind == 1 {
											input.push(entry.name);
										} else if entry.kind == 2 {
											output.push(entry.name);
										}
									}
								}};
							}

							match game.version() {
								GameVersion::H1 => generate!(h1),
								GameVersion::H2 => generate!(h2),
								GameVersion::H3 => generate!(h3)
							}
						}

						"MATT" => {
							// All materials have the pins of ZRenderMaterialEntity
							let cppt_data = self
								.cppt_pins
								.get(&rid!("[modules:/zrendermaterialentity.class].pc_entitytype"))
								.context("No such CPPT in pins")?;

							input.extend(cppt_data.inputs.iter().map(|x| &x.name).cloned());
							output.extend(cppt_data.outputs.iter().map(|x| &x.name).cloned());

							for (property_name, property_data) in self.get_matt_properties(game, factory)? {
								if !matches!(property_data, MaterialOverride::Texture(_)) {
									input.push(property_name);
								}
							}
						}

						"WSWT" => {
							// All switch groups have the pins of ZAudioSwitchEntity
							let cppt_data = self
								.cppt_pins
								.get(&rid!("[modules:/zaudioswitchentity.class].pc_entitytype"))
								.context("No such CPPT in pins")?;

							input.extend(cppt_data.inputs.iter().map(|x| &x.name).cloned());
							output.extend(cppt_data.outputs.iter().map(|x| &x.name).cloned());

							let wswt_meta = game.extract_latest_metadata(factory)?;

							let dswb_hash = wswt_meta
								.core_info
								.references
								.into_iter()
								.find(|x| {
									game.resource_type(x.resource)
										.is_some_and(|ty| ty == "DSWB" || ty == "WSWB")
								})
								.context("No blueprint dependency on WSWT")?
								.resource;

							let dswb_data = game.extract_latest_resource(dswb_hash)?.1;

							input.extend(match game.version() {
								GameVersion::H1 => glacier_bin1::deserialize::<
									glacier_bin1::game::h1::SAudioSwitchGroupData
								>(&dswb_data)?
								.switches
								.into_iter()
								.map(|x| x.name)
								.collect_vec(),
								GameVersion::H2 => glacier_bin1::deserialize::<
									glacier_bin1::game::h2::SAudioSwitchGroupData
								>(&dswb_data)?
								.switches
								.into_iter()
								.map(|x| x.name)
								.collect_vec(),
								GameVersion::H3 => glacier_bin1::deserialize::<
									glacier_bin1::game::h3::SAudioSwitchGroupData
								>(&dswb_data)?
								.switches
								.into_iter()
								.map(|x| x.name)
								.collect_vec()
							});
						}

						"ECPT" => {
							// All extended CPP entities have the pins of ZMaterialOverwriteAspect
							let cppt_data = self
								.cppt_pins
								.get(&rid!("[modules:/zmaterialoverwriteaspect.class].pc_entitytype"))
								.context("No such CPPT in pins")?;

							input.extend(cppt_data.inputs.iter().map(|x| &x.name).cloned());
							output.extend(cppt_data.outputs.iter().map(|x| &x.name).cloned());
						}

						"AIBX" => {
							// All behaviour trees have the pins of ZBehaviorTreeEntity
							let cppt_data = self
								.cppt_pins
								.get(&rid!("[modules:/zbehaviortreeentity.class].pc_entitytype"))
								.context("No such CPPT in pins")?;

							input.extend(cppt_data.inputs.iter().map(|x| &x.name).cloned());
							output.extend(cppt_data.outputs.iter().map(|x| &x.name).cloned());
						}

						"WSGT" => {
							// All state groups have the pins of ZAudioStateEntity
							let cppt_data = self
								.cppt_pins
								.get(&rid!("[modules:/zaudiostateentity.class].pc_entitytype"))
								.context("No such CPPT in pins")?;

							input.extend(cppt_data.inputs.iter().map(|x| &x.name).cloned());
							output.extend(cppt_data.outputs.iter().map(|x| &x.name).cloned());

							let wsgt_meta = game.extract_latest_metadata(factory)?;

							let wsgb_hash = wsgt_meta
								.core_info
								.references
								.into_iter()
								.find(|x| game.resource_type(x.resource).is_some_and(|ty| ty == "WSGB"))
								.context("No blueprint dependency on WSWT")?
								.resource;

							let wsgb_data = game.extract_latest_resource(wsgb_hash)?.1;

							input.extend(match game.version() {
								GameVersion::H1 => glacier_bin1::deserialize::<
									glacier_bin1::game::h1::SAudioStateGroupData
								>(&wsgb_data)?
								.states
								.into_iter()
								.map(|x| x.name)
								.collect_vec(),
								GameVersion::H2 => glacier_bin1::deserialize::<
									glacier_bin1::game::h2::SAudioStateGroupData
								>(&wsgb_data)?
								.states
								.into_iter()
								.map(|x| x.name)
								.collect_vec(),
								GameVersion::H3 => glacier_bin1::deserialize::<
									glacier_bin1::game::h3::SAudioStateGroupData
								>(&wsgb_data)?
								.states
								.into_iter()
								.map(|x| x.name)
								.collect_vec()
							});
						}

						"TEMP" => {
							let extracted = game.extract_entity(factory)?;

							let found = self.get_pins(game, &extracted, extracted.root_entity, false)?;

							input.extend(found.0);
							output.extend(found.1);
						}

						_ => bail!("Unknown factory type")
					}
				}

				(input, output)
			})
		})
		.collect::<Result<Vec<_>>>()?
		.into_iter()
		.unzip();

		input.extend(fac_input.into_iter().flatten());
		output.extend(fac_output.into_iter().flatten());

		(
			input.into_iter().unique().collect(),
			output.into_iter().unique().collect()
		)
	}
}
