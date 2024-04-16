use std::{collections::HashMap, io::Read};

use anyhow::{Context, Result};
use enumset::EnumSet;
use fn_error_context::context;
use serde::{Deserialize, Serialize};
use tryvial::try_fn;

use crate::game_detection::GameVersion;

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DeserialisedHashList {
	pub version: u16,
	pub entries: Vec<DeserialisedEntry>
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DeserialisedEntry {
	pub resource_type: String,
	pub hash: String,
	pub path: String,
	pub hint: String,
	pub game_flags: u8
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HashList {
	pub version: u16,
	pub entries: HashMap<String, HashData>
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HashData {
	pub resource_type: String,
	pub path: Option<String>,
	pub hint: Option<String>
}

impl HashList {
	#[try_fn]
	#[context("Couldn't deserialise hash list from compressed binary data")]
	pub fn from_slice(slice: &[u8]) -> Result<Self> {
		let mut decompressed = vec![];

		brotli_decompressor::Decompressor::new(slice, 4096)
			.read_to_end(&mut decompressed)
			.context("Decompression failed")?;

		let mut hash_list: DeserialisedHashList =
			serde_smile::from_slice(&decompressed).context("Deserialisation failed")?;

		hash_list
			.entries
			.sort_by_cached_key(|x| format!("{}{}{}", x.path, x.hint, x.hash));

		HashList {
			version: hash_list.version,
			entries: hash_list
				.entries
				.into_iter()
				.map(|entry| {
					(
						entry.hash,
						HashData {
							resource_type: entry.resource_type,
							path: (!entry.path.is_empty()).then_some(entry.path),
							hint: (!entry.hint.is_empty()).then_some(entry.hint)
						}
					)
				})
				.collect()
		}
	}
}
