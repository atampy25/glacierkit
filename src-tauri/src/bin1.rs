use anyhow::{Context, Result, bail};
use ecow::EcoString;
use fn_error_context::context;
use hitman_bin1::game::{conversion::ConvertInto, h1, h2, h3};
use hitman_commons::{game::GameVersion, metadata::ResourceType};
use serde_json::{Value, to_value, to_writer};
use tryvial::try_fn;

/// Deserialize a factory from the game and convert it to the H3 format if necessary.
#[try_fn]
#[context("Couldn't deserialize factory for version {game_version:?}")]
pub fn deserialize_modern_factory(game_version: GameVersion, data: &[u8]) -> Result<h3::STemplateEntityFactory> {
	match game_version {
		GameVersion::H1 => hitman_bin1::deserialize::<h1::STemplateEntity>(data)
			.context("Couldn't deserialise factory")?
			.convert_into()
			.context("Couldn't convert factory to modern")?,

		GameVersion::H2 => hitman_bin1::deserialize::<h2::STemplateEntityFactory>(data)
			.context("Couldn't deserialise factory")?
			.convert_into()
			.context("Couldn't convert factory to modern")?,

		GameVersion::H3 => {
			hitman_bin1::deserialize::<h3::STemplateEntityFactory>(data).context("Couldn't deserialise factory")?
		}
	}
}

/// Deserialize a blueprint from the game and convert it to the H3 format if necessary.
#[try_fn]
#[context("Couldn't deserialize blueprint for version {game_version:?}")]
pub fn deserialize_modern_blueprint(game_version: GameVersion, data: &[u8]) -> Result<h3::STemplateEntityBlueprint> {
	match game_version {
		GameVersion::H1 => hitman_bin1::deserialize::<h1::STemplateEntityBlueprint>(data)
			.context("Couldn't deserialise blueprint")?
			.convert_into()
			.context("Couldn't convert blueprint to modern")?,

		GameVersion::H2 => hitman_bin1::deserialize::<h2::STemplateEntityBlueprint>(data)
			.context("Couldn't deserialise blueprint")?
			.convert_into()
			.context("Couldn't convert blueprint to modern")?,

		GameVersion::H3 => {
			hitman_bin1::deserialize::<h3::STemplateEntityBlueprint>(data).context("Couldn't deserialise blueprint")?
		}
	}
}

#[try_fn]
#[context("Couldn't deserialize {game_version:?} {resource_type} resource as generic")]
pub fn deserialize_generic(game_version: GameVersion, resource_type: ResourceType, data: &[u8]) -> Result<Value> {
	macro_rules! impl_convert {
		($resource_type:ident, $ty:literal, $res:ty) => {
			if $resource_type == $ty {
				return to_value(hitman_bin1::deserialize::<$res>(data).context("Couldn't deserialise resource")?)
					.context("Couldn't convert resource to JSON value");
			}
		};
	}

	macro_rules! impl_all {
		($resource_type:ident, h1) => {{
			impl_all!(generic, $resource_type, h1);

			impl_convert!($resource_type, "TEMP", h1::STemplateEntity);
		}};

		($resource_type:ident, h2) => {{
			impl_all!(generic, $resource_type, h2);

			impl_convert!($resource_type, "TEMP", h2::STemplateEntityFactory);
			impl_convert!($resource_type, "ECPB", h2::SExtendedCppEntityBlueprint);
		}};

		($resource_type:ident, h3) => {{
			impl_all!(generic, $resource_type, h3);

			impl_convert!($resource_type, "TEMP", h3::STemplateEntityFactory);
			impl_convert!($resource_type, "ECPB", h3::SExtendedCppEntityBlueprint);
		}};

		(generic, $resource_type:ident, $game:ident) => {
			impl_convert!($resource_type, "AIBB", $game::SBehaviorTreeInfo);
			impl_convert!($resource_type, "AIRG", $game::SReasoningGrid);
			impl_convert!($resource_type, "ASVA", Vec<$game::SPackedAnimSetEntry>);
			impl_convert!($resource_type, "ATMD", $game::ZAMDTake);
			impl_convert!($resource_type, "BMSK", Vec<u32>);
			impl_convert!($resource_type, "CBLU", $game::SCppEntityBlueprint);
			impl_convert!($resource_type, "CPPT", $game::SCppEntity);
			impl_convert!($resource_type, "CRMD", $game::SCrowdMapData);
			impl_convert!($resource_type, "ENUM", $game::SEnumType);
			impl_convert!($resource_type, "GFXF", $game::SGFxMovieResource);
			impl_convert!($resource_type, "GIDX", $game::SResourceIndex);
			impl_convert!($resource_type, "TBLU", $game::STemplateEntityBlueprint);
			impl_convert!($resource_type, "UICB", $game::SControlTypeInfo);
			impl_convert!($resource_type, "VIDB", $game::SVideoDatabaseData);
			impl_convert!($resource_type, "WSGB", $game::SAudioStateGroupData);
			impl_convert!($resource_type, "WSWB", $game::SAudioSwitchGroupData);
		};
	}

	if resource_type == "ORES" {
		// Guess the right ORES type
		match game_version {
			GameVersion::H1 => {
				if data.windows(11).any(|x| x == b"blobcachedb") {
					to_value(
						hitman_bin1::deserialize::<Vec<h1::SBlobsConfigResourceEntry>>(data)
							.context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else if data.windows(9).any(|x| x == b"GamePrice") {
					to_value(hitman_bin1::deserialize::<EcoString>(data).context("Couldn't deserialise resource")?)
						.context("Couldn't convert resource to JSON value")?
				} else {
					hitman_bin1::deserialize::<Vec<h1::SContractConfigResourceEntry>>(data)
						.context("Couldn't deserialise resource as SContractConfigResourceEntry")
						.map_or_else(
							|_| {
								to_value(
									hitman_bin1::deserialize::<h1::SEnvironmentConfigResource>(data)
										.context("Couldn't deserialise resource as SEnvironmentConfigResource")
										.context("No ORES type matched")?
								)
								.context("Couldn't convert resource to JSON value")
							},
							|x| to_value(x).context("Couldn't convert resource to JSON value")
						)?
				}
			}

			GameVersion::H2 => {
				if data.windows(11).any(|x| x == b"blobcachedb") {
					to_value(
						hitman_bin1::deserialize::<Vec<h2::SBlobsConfigResourceEntry>>(data)
							.context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else if data.windows(9).any(|x| x == b"GamePrice") {
					to_value(hitman_bin1::deserialize::<EcoString>(data).context("Couldn't deserialise resource")?)
						.context("Couldn't convert resource to JSON value")?
				} else {
					hitman_bin1::deserialize::<Vec<h2::SContractConfigResourceEntry>>(data)
						.context("Couldn't deserialise resource as SContractConfigResourceEntry")
						.map_or_else(
							|_| {
								to_value(
									hitman_bin1::deserialize::<h2::SEnvironmentConfigResource>(data)
										.context("Couldn't deserialise resource as SEnvironmentConfigResource")
										.context("No ORES type matched")?
								)
								.context("Couldn't convert resource to JSON value")
							},
							|x| to_value(x).context("Couldn't convert resource to JSON value")
						)?
				}
			}

			GameVersion::H3 => {
				if data.windows(11).any(|x| x == b"blobcachedb") {
					to_value(
						hitman_bin1::deserialize::<Vec<h3::SBlobsConfigResourceEntry>>(data)
							.context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else if data.windows(9).any(|x| x == b"GamePrice") {
					to_value(hitman_bin1::deserialize::<EcoString>(data).context("Couldn't deserialise resource")?)
						.context("Couldn't convert resource to JSON value")?
				} else {
					hitman_bin1::deserialize::<Vec<h3::SContractConfigResourceEntry>>(data)
						.context("Couldn't deserialise resource as SContractConfigResourceEntry")
						.map_or_else(
							|_| {
								hitman_bin1::deserialize::<h3::SEnvironmentConfigResource>(data)
									.context("Couldn't deserialise resource as SEnvironmentConfigResource")
									.map_or_else(
										|_| {
											to_value(
												hitman_bin1::deserialize::<h3::SActivities>(data)
													.context("Couldn't deserialise resource as SActivities")
													.context("No ORES type matched")?
											)
											.context("Couldn't convert resource to JSON value")
										},
										|x| to_value(x).context("Couldn't convert resource to JSON value")
									)
							},
							|x| to_value(x).context("Couldn't convert resource to JSON value")
						)?
				}
			}
		}
	} else {
		match game_version {
			GameVersion::H1 => impl_all!(resource_type, h1),
			GameVersion::H2 => impl_all!(resource_type, h2),
			GameVersion::H3 => impl_all!(resource_type, h3)
		}

		bail!("Resource type {resource_type} is not BIN1 or is unsupported for version {game_version:?}")
	}
}

#[try_fn]
#[context("Couldn't deserialize {game_version:?} {resource_type} resource as generic to writer")]
pub fn deserialize_generic_writer(
	game_version: GameVersion,
	resource_type: ResourceType,
	writer: &mut impl std::io::Write,
	data: &[u8]
) -> Result<()> {
	macro_rules! impl_convert {
		($resource_type:ident, $ty:literal, $res:ty) => {
			if $resource_type == $ty {
				return to_writer(
					writer,
					&hitman_bin1::deserialize::<$res>(data).context("Couldn't deserialise resource")?
				)
				.context("Couldn't convert resource to JSON value");
			}
		};
	}

	macro_rules! impl_all {
		($resource_type:ident, h1) => {{
			impl_all!(generic, $resource_type, h1);

			impl_convert!($resource_type, "TEMP", h1::STemplateEntity);
		}};

		($resource_type:ident, h2) => {{
			impl_all!(generic, $resource_type, h2);

			impl_convert!($resource_type, "TEMP", h2::STemplateEntityFactory);
			impl_convert!($resource_type, "ECPB", h2::SExtendedCppEntityBlueprint);
		}};

		($resource_type:ident, h3) => {{
			impl_all!(generic, $resource_type, h3);

			impl_convert!($resource_type, "TEMP", h3::STemplateEntityFactory);
			impl_convert!($resource_type, "ECPB", h3::SExtendedCppEntityBlueprint);
		}};

		(generic, $resource_type:ident, $game:ident) => {
			impl_convert!($resource_type, "AIBB", $game::SBehaviorTreeInfo);
			impl_convert!($resource_type, "AIRG", $game::SReasoningGrid);
			impl_convert!($resource_type, "ASVA", Vec<$game::SPackedAnimSetEntry>);
			impl_convert!($resource_type, "ATMD", $game::ZAMDTake);
			impl_convert!($resource_type, "BMSK", Vec<u32>);
			impl_convert!($resource_type, "CBLU", $game::SCppEntityBlueprint);
			impl_convert!($resource_type, "CPPT", $game::SCppEntity);
			impl_convert!($resource_type, "CRMD", $game::SCrowdMapData);
			impl_convert!($resource_type, "ENUM", $game::SEnumType);
			impl_convert!($resource_type, "GFXF", $game::SGFxMovieResource);
			impl_convert!($resource_type, "GIDX", $game::SResourceIndex);
			impl_convert!($resource_type, "TBLU", $game::STemplateEntityBlueprint);
			impl_convert!($resource_type, "UICB", $game::SControlTypeInfo);
			impl_convert!($resource_type, "VIDB", $game::SVideoDatabaseData);
			impl_convert!($resource_type, "WSGB", $game::SAudioStateGroupData);
			impl_convert!($resource_type, "WSWB", $game::SAudioSwitchGroupData);
		};
	}

	if resource_type == "ORES" {
		// Guess the right ORES type
		match game_version {
			GameVersion::H1 => {
				if data.windows(11).any(|x| x == b"blobcachedb") {
					to_writer(
						writer,
						&hitman_bin1::deserialize::<Vec<h1::SBlobsConfigResourceEntry>>(data)
							.context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else if data.windows(9).any(|x| x == b"GamePrice") {
					to_writer(
						writer,
						&hitman_bin1::deserialize::<EcoString>(data).context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else {
					if let Ok(x) = hitman_bin1::deserialize::<Vec<h1::SContractConfigResourceEntry>>(data)
						.context("Couldn't deserialise resource as SContractConfigResourceEntry")
					{
						to_writer(writer, &x).context("Couldn't convert resource to JSON value")?
					} else {
						to_writer(
							writer,
							&hitman_bin1::deserialize::<h1::SEnvironmentConfigResource>(data)
								.context("Couldn't deserialise resource as SEnvironmentConfigResource")
								.context("No ORES type matched")?
						)
						.context("Couldn't convert resource to JSON value")?
					}
				}
			}

			GameVersion::H2 => {
				if data.windows(11).any(|x| x == b"blobcachedb") {
					to_writer(
						writer,
						&hitman_bin1::deserialize::<Vec<h2::SBlobsConfigResourceEntry>>(data)
							.context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else if data.windows(9).any(|x| x == b"GamePrice") {
					to_writer(
						writer,
						&hitman_bin1::deserialize::<EcoString>(data).context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else {
					if let Ok(x) = hitman_bin1::deserialize::<Vec<h2::SContractConfigResourceEntry>>(data)
						.context("Couldn't deserialise resource as SContractConfigResourceEntry")
					{
						to_writer(writer, &x).context("Couldn't convert resource to JSON value")?
					} else {
						to_writer(
							writer,
							&hitman_bin1::deserialize::<h2::SEnvironmentConfigResource>(data)
								.context("Couldn't deserialise resource as SEnvironmentConfigResource")
								.context("No ORES type matched")?
						)
						.context("Couldn't convert resource to JSON value")?
					}
				}
			}

			GameVersion::H3 => {
				if data.windows(11).any(|x| x == b"blobcachedb") {
					to_writer(
						writer,
						&hitman_bin1::deserialize::<Vec<h3::SBlobsConfigResourceEntry>>(data)
							.context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else if data.windows(9).any(|x| x == b"GamePrice") {
					to_writer(
						writer,
						&hitman_bin1::deserialize::<EcoString>(data).context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else {
					if let Ok(x) = hitman_bin1::deserialize::<Vec<h3::SContractConfigResourceEntry>>(data)
						.context("Couldn't deserialise resource as SContractConfigResourceEntry")
					{
						to_writer(writer, &x).context("Couldn't convert resource to JSON value")?
					} else if let Ok(x) = hitman_bin1::deserialize::<h3::SEnvironmentConfigResource>(data)
						.context("Couldn't deserialise resource as SEnvironmentConfigResource")
					{
						to_writer(writer, &x).context("Couldn't convert resource to JSON value")?
					} else {
						to_writer(
							writer,
							&hitman_bin1::deserialize::<h3::SActivities>(data)
								.context("Couldn't deserialise resource as SActivities")
								.context("No ORES type matched")?
						)
						.context("Couldn't convert resource to JSON value")?
					}
				}
			}
		}
	} else {
		match game_version {
			GameVersion::H1 => impl_all!(resource_type, h1),
			GameVersion::H2 => impl_all!(resource_type, h2),
			GameVersion::H3 => impl_all!(resource_type, h3)
		}

		bail!("Resource type {resource_type} is not BIN1 or is unsupported for version {game_version:?}")
	}
}
