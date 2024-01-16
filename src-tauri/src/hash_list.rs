use std::io::Read;

use anyhow::{Context, Result};
use fn_error_context::context;
use serde::{Deserialize, Serialize};
use tryvial::try_fn;

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HashList {
	pub version: u16,
	pub entries: Vec<Entry>
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
	pub resource_type: String,
	pub hash: String,
	pub path: String,
	pub hint: String,
	pub game_flags: u8
}

impl HashList {
	#[try_fn]
	#[context("Couldn't deserialise hash list from compressed binary data")]
	pub fn from_slice(slice: &[u8]) -> Result<Self> {
		let mut decompressed = vec![];

		brotli_decompressor::Decompressor::new(slice, 4096)
			.read_to_end(&mut decompressed)
			.context("Decompression failed")?;

		let mut hash_list: HashList = serde_smile::from_slice(&decompressed).context("Deserialisation failed")?;

		hash_list
			.entries
			.sort_by_cached_key(|x| format!("{}{}{}", x.path, x.hint, x.hash));

		hash_list
	}
}