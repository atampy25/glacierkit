use anyhow::{Context, Result, bail};
use ecow::EcoString;
use fn_error_context::context;
use glacier_bin1::game::{fl, h1, h2, h3};
use glacier_commons::{game::GlacierGame, metadata::ResourceType};
use serde_json::{Value, to_value, to_writer};
use tryvial::try_fn;

#[try_fn]
#[context("Couldn't deserialize {game_version:?} {resource_type} resource as BIN1")]
pub fn deserialize_generic(game_version: GlacierGame, resource_type: ResourceType, data: &[u8]) -> Result<Value> {
	macro_rules! impl_convert {
		($resource_type:ident, $ty:literal, $res:ty) => {
			if $resource_type == $ty {
				return to_value(glacier_bin1::deserialize::<$res>(data).context("Couldn't deserialise resource")?)
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

		($resource_type:ident, fl) => {{
			impl_convert!($resource_type, "CBLU", fl::SCppEntityBlueprint);
			impl_convert!($resource_type, "CLRP", fl::SColorPalette);
			impl_convert!($resource_type, "CPPT", fl::SCppEntity);
			impl_convert!($resource_type, "CRMD", fl::SCrowdMapData);
			impl_convert!($resource_type, "ECPB", fl::SExtendedCppEntityBlueprint);
			impl_convert!($resource_type, "ENUM", fl::SEnumType);
			impl_convert!($resource_type, "GFXA", fl::SGFxAtlas);
			impl_convert!($resource_type, "GFXF", fl::SGFxMovieResource);
			impl_convert!($resource_type, "GIDX", fl::SResourceIndex);
			impl_convert!($resource_type, "KWOR", fl::SSerializedKeyword);
			impl_convert!($resource_type, "TBLU", fl::STemplateEntityBlueprint);
			impl_convert!($resource_type, "TDAT", fl::STerrainResource);
			impl_convert!($resource_type, "TDPK", fl::STerrainDataPackage);
			impl_convert!($resource_type, "TEMP", fl::STemplateEntityFactory);
			impl_convert!($resource_type, "UICB", fl::SControlTypeInfo);
			impl_convert!($resource_type, "WEMD", Vec<fl::SAudioEventMetadata>);
			impl_convert!($resource_type, "WSGB", fl::SAudioStateGroupData);
			impl_convert!($resource_type, "WSWB", fl::SAudioSwitchGroupData);
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
			GlacierGame::H1 => {
				if data.windows(11).any(|x| x == b"blobcachedb") {
					to_value(
						glacier_bin1::deserialize::<Vec<h1::SBlobsConfigResourceEntry>>(data)
							.context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else if data.windows(9).any(|x| x == b"GamePrice") {
					to_value(glacier_bin1::deserialize::<EcoString>(data).context("Couldn't deserialise resource")?)
						.context("Couldn't convert resource to JSON value")?
				} else {
					glacier_bin1::deserialize::<Vec<h1::SContractConfigResourceEntry>>(data)
						.context("Couldn't deserialise resource as SContractConfigResourceEntry")
						.map_or_else(
							|_| {
								to_value(
									glacier_bin1::deserialize::<h1::SEnvironmentConfigResource>(data)
										.context("Couldn't deserialise resource as SEnvironmentConfigResource")
										.context("No ORES type matched")?
								)
								.context("Couldn't convert resource to JSON value")
							},
							|x| to_value(x).context("Couldn't convert resource to JSON value")
						)?
				}
			}

			GlacierGame::H2 => {
				if data.windows(11).any(|x| x == b"blobcachedb") {
					to_value(
						glacier_bin1::deserialize::<Vec<h2::SBlobsConfigResourceEntry>>(data)
							.context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else if data.windows(9).any(|x| x == b"GamePrice") {
					to_value(glacier_bin1::deserialize::<EcoString>(data).context("Couldn't deserialise resource")?)
						.context("Couldn't convert resource to JSON value")?
				} else {
					glacier_bin1::deserialize::<Vec<h2::SContractConfigResourceEntry>>(data)
						.context("Couldn't deserialise resource as SContractConfigResourceEntry")
						.map_or_else(
							|_| {
								to_value(
									glacier_bin1::deserialize::<h2::SEnvironmentConfigResource>(data)
										.context("Couldn't deserialise resource as SEnvironmentConfigResource")
										.context("No ORES type matched")?
								)
								.context("Couldn't convert resource to JSON value")
							},
							|x| to_value(x).context("Couldn't convert resource to JSON value")
						)?
				}
			}

			GlacierGame::H3 => {
				if data.windows(11).any(|x| x == b"blobcachedb") {
					to_value(
						glacier_bin1::deserialize::<Vec<h3::SBlobsConfigResourceEntry>>(data)
							.context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else if data.windows(9).any(|x| x == b"GamePrice") {
					to_value(glacier_bin1::deserialize::<EcoString>(data).context("Couldn't deserialise resource")?)
						.context("Couldn't convert resource to JSON value")?
				} else {
					glacier_bin1::deserialize::<Vec<h3::SContractConfigResourceEntry>>(data)
						.context("Couldn't deserialise resource as SContractConfigResourceEntry")
						.map_or_else(
							|_| {
								glacier_bin1::deserialize::<h3::SEnvironmentConfigResource>(data)
									.context("Couldn't deserialise resource as SEnvironmentConfigResource")
									.map_or_else(
										|_| {
											to_value(
												glacier_bin1::deserialize::<h3::SActivities>(data)
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

			GlacierGame::FL => {
				if data.windows(11).any(|x| x == b"blobcachedb") {
					to_value(
						glacier_bin1::deserialize::<Vec<fl::SBlobsConfigResourceEntry>>(data)
							.context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else if data.windows(9).any(|x| x == b"GamePrice") {
					to_value(glacier_bin1::deserialize::<EcoString>(data).context("Couldn't deserialise resource")?)
						.context("Couldn't convert resource to JSON value")?
				} else {
					glacier_bin1::deserialize::<Vec<fl::SContractConfigResourceEntry>>(data)
						.context("Couldn't deserialise resource as SContractConfigResourceEntry")
						.map_or_else(
							|_| {
								glacier_bin1::deserialize::<fl::SEnvironmentConfigResource>(data)
									.context("Couldn't deserialise resource as SEnvironmentConfigResource")
									.map_or_else(
										|_| {
											to_value(
												glacier_bin1::deserialize::<fl::SActivities>(data)
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
			GlacierGame::H1 => impl_all!(resource_type, h1),
			GlacierGame::H2 => impl_all!(resource_type, h2),
			GlacierGame::H3 => impl_all!(resource_type, h3),
			GlacierGame::FL => impl_all!(resource_type, fl)
		}

		bail!("Resource type {resource_type} is not BIN1 or is unsupported for version {game_version:?}")
	}
}

#[try_fn]
#[context("Couldn't deserialize {game_version:?} {resource_type} resource as BIN1 to writer")]
pub fn deserialize_generic_writer(
	game_version: GlacierGame,
	resource_type: ResourceType,
	writer: &mut impl std::io::Write,
	data: &[u8]
) -> Result<()> {
	macro_rules! impl_convert {
		($resource_type:ident, $ty:literal, $res:ty) => {
			if $resource_type == $ty {
				return to_writer(
					writer,
					&glacier_bin1::deserialize::<$res>(data).context("Couldn't deserialise resource")?
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

		($resource_type:ident, fl) => {{
			impl_convert!($resource_type, "CBLU", fl::SCppEntityBlueprint);
			impl_convert!($resource_type, "CLRP", fl::SColorPalette);
			impl_convert!($resource_type, "CPPT", fl::SCppEntity);
			impl_convert!($resource_type, "CRMD", fl::SCrowdMapData);
			impl_convert!($resource_type, "ECPB", fl::SExtendedCppEntityBlueprint);
			impl_convert!($resource_type, "ENUM", fl::SEnumType);
			impl_convert!($resource_type, "GFXA", fl::SGFxAtlas);
			impl_convert!($resource_type, "GFXF", fl::SGFxMovieResource);
			impl_convert!($resource_type, "GIDX", fl::SResourceIndex);
			impl_convert!($resource_type, "KWOR", fl::SSerializedKeyword);
			impl_convert!($resource_type, "TBLU", fl::STemplateEntityBlueprint);
			impl_convert!($resource_type, "TDAT", fl::STerrainResource);
			impl_convert!($resource_type, "TDPK", fl::STerrainDataPackage);
			impl_convert!($resource_type, "TEMP", fl::STemplateEntityFactory);
			impl_convert!($resource_type, "UICB", fl::SControlTypeInfo);
			impl_convert!($resource_type, "WEMD", Vec<fl::SAudioEventMetadata>);
			impl_convert!($resource_type, "WSGB", fl::SAudioStateGroupData);
			impl_convert!($resource_type, "WSWB", fl::SAudioSwitchGroupData);
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
			GlacierGame::H1 => {
				if data.windows(11).any(|x| x == b"blobcachedb") {
					to_writer(
						writer,
						&glacier_bin1::deserialize::<Vec<h1::SBlobsConfigResourceEntry>>(data)
							.context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else if data.windows(9).any(|x| x == b"GamePrice") {
					to_writer(
						writer,
						&glacier_bin1::deserialize::<EcoString>(data).context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else {
					if let Ok(x) = glacier_bin1::deserialize::<Vec<h1::SContractConfigResourceEntry>>(data)
						.context("Couldn't deserialise resource as SContractConfigResourceEntry")
					{
						to_writer(writer, &x).context("Couldn't convert resource to JSON value")?
					} else {
						to_writer(
							writer,
							&glacier_bin1::deserialize::<h1::SEnvironmentConfigResource>(data)
								.context("Couldn't deserialise resource as SEnvironmentConfigResource")
								.context("No ORES type matched")?
						)
						.context("Couldn't convert resource to JSON value")?
					}
				}
			}

			GlacierGame::H2 => {
				if data.windows(11).any(|x| x == b"blobcachedb") {
					to_writer(
						writer,
						&glacier_bin1::deserialize::<Vec<h2::SBlobsConfigResourceEntry>>(data)
							.context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else if data.windows(9).any(|x| x == b"GamePrice") {
					to_writer(
						writer,
						&glacier_bin1::deserialize::<EcoString>(data).context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else {
					if let Ok(x) = glacier_bin1::deserialize::<Vec<h2::SContractConfigResourceEntry>>(data)
						.context("Couldn't deserialise resource as SContractConfigResourceEntry")
					{
						to_writer(writer, &x).context("Couldn't convert resource to JSON value")?
					} else {
						to_writer(
							writer,
							&glacier_bin1::deserialize::<h2::SEnvironmentConfigResource>(data)
								.context("Couldn't deserialise resource as SEnvironmentConfigResource")
								.context("No ORES type matched")?
						)
						.context("Couldn't convert resource to JSON value")?
					}
				}
			}

			GlacierGame::H3 => {
				if data.windows(11).any(|x| x == b"blobcachedb") {
					to_writer(
						writer,
						&glacier_bin1::deserialize::<Vec<h3::SBlobsConfigResourceEntry>>(data)
							.context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else if data.windows(9).any(|x| x == b"GamePrice") {
					to_writer(
						writer,
						&glacier_bin1::deserialize::<EcoString>(data).context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else {
					if let Ok(x) = glacier_bin1::deserialize::<Vec<h3::SContractConfigResourceEntry>>(data)
						.context("Couldn't deserialise resource as SContractConfigResourceEntry")
					{
						to_writer(writer, &x).context("Couldn't convert resource to JSON value")?
					} else if let Ok(x) = glacier_bin1::deserialize::<h3::SEnvironmentConfigResource>(data)
						.context("Couldn't deserialise resource as SEnvironmentConfigResource")
					{
						to_writer(writer, &x).context("Couldn't convert resource to JSON value")?
					} else {
						to_writer(
							writer,
							&glacier_bin1::deserialize::<h3::SActivities>(data)
								.context("Couldn't deserialise resource as SActivities")
								.context("No ORES type matched")?
						)
						.context("Couldn't convert resource to JSON value")?
					}
				}
			}

			GlacierGame::FL => {
				if data.windows(11).any(|x| x == b"blobcachedb") {
					to_writer(
						writer,
						&glacier_bin1::deserialize::<Vec<fl::SBlobsConfigResourceEntry>>(data)
							.context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else if data.windows(9).any(|x| x == b"GamePrice") {
					to_writer(
						writer,
						&glacier_bin1::deserialize::<EcoString>(data).context("Couldn't deserialise resource")?
					)
					.context("Couldn't convert resource to JSON value")?
				} else {
					if let Ok(x) = glacier_bin1::deserialize::<Vec<fl::SContractConfigResourceEntry>>(data)
						.context("Couldn't deserialise resource as SContractConfigResourceEntry")
					{
						to_writer(writer, &x).context("Couldn't convert resource to JSON value")?
					} else if let Ok(x) = glacier_bin1::deserialize::<fl::SEnvironmentConfigResource>(data)
						.context("Couldn't deserialise resource as SEnvironmentConfigResource")
					{
						to_writer(writer, &x).context("Couldn't convert resource to JSON value")?
					} else {
						to_writer(
							writer,
							&glacier_bin1::deserialize::<fl::SActivities>(data)
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
			GlacierGame::H1 => impl_all!(resource_type, h1),
			GlacierGame::H2 => impl_all!(resource_type, h2),
			GlacierGame::H3 => impl_all!(resource_type, h3),
			GlacierGame::FL => impl_all!(resource_type, fl)
		}

		bail!("Resource type {resource_type} is not BIN1 or is unsupported for version {game_version:?}")
	}
}
