use std::io::{Cursor, Read, Seek, SeekFrom, Write};

use anyhow::{Context, Result};
use fn_error_context::context;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use tryvial::try_fn;
use uuid::Uuid;

#[try_fn]
#[context("Couldn't read data as hashes ORES")]
pub fn parse_hashes_ores(bin_data: &[u8]) -> Result<IndexMap<String, String>> {
	let mut data = IndexMap::new();

	let mut cursor = Cursor::new(bin_data);

	cursor.seek(SeekFrom::Start(8))?;

	let end_of_strings = i32::from_be_bytes({
		let mut x = [0u8; 4];
		cursor.read_exact(&mut x)?;
		x
	});

	cursor.seek(SeekFrom::Start(u64::try_from(end_of_strings)? + 24))?;

	let number_of_entries = i32::from_le_bytes({
		let mut x = [0u8; 4];
		cursor.read_exact(&mut x)?;
		x
	});

	let mut offsets = Vec::new();
	for _ in 0..number_of_entries {
		offsets.push(i32::from_le_bytes({
			let mut x = [0u8; 4];
			cursor.read_exact(&mut x)?;
			x
		}));
	}

	for i in 3..number_of_entries {
		let i = usize::try_from(i)?;

		cursor.seek(SeekFrom::Start(u64::try_from(offsets[i] + 16)?))?;

		let offset_of_data = i32::from_le_bytes({
			let mut x = [0u8; 4];
			cursor.read_exact(&mut x)?;
			x
		});

		cursor.seek(SeekFrom::Current(4))?;

		let hash_bytes = {
			let mut x = [0u8; 8];
			cursor.read_exact(&mut x)?;
			x
		};

		let hash = format!(
			"{}{}",
			(hash_bytes[0..4]
				.iter()
				.rev()
				.map(|x| format!("{:0>2X}", x))
				.collect::<Vec<_>>()
				.join("")),
			(hash_bytes[4..8]
				.iter()
				.rev()
				.map(|x| format!("{:0>2X}", x))
				.collect::<Vec<_>>()
				.join(""))
		);

		cursor.seek(SeekFrom::Start(u64::try_from(offset_of_data + 12)?))?;

		let len = i32::from_le_bytes({
			let mut x = [0u8; 4];
			cursor.read_exact(&mut x)?;
			x
		});

		let str_bytes = {
			let mut x = vec![0u8; usize::try_from(len)? - 1];
			cursor.read_exact(&mut x)?;
			x
		};

		data.insert(hash, String::from_utf8(str_bytes)?);
	}

	data
}

fn offset_of_string(values: &[&String], curValue: usize) -> usize {
	let mut offset = 0;

	for value in values.iter().take(curValue) {
		offset += 4 + value.len() + 1;
		offset += (4 - (value.len() + 1) % 4) % 4;
	}

	offset
}

#[try_fn]
#[context("Couldn't serialise data to hashes ORES")]
pub fn generate_hashes_ores(data: &IndexMap<String, String>) -> Result<Vec<u8>> {
	let (hashes, values): (Vec<_>, Vec<_>) = data.into_iter().unzip();

	let mut ores = vec![];
	let mut cursor = Cursor::new(&mut ores);

	let start_of_strings = 0x30 + 0x18 * values.len();

	let end_of_strings = start_of_strings + offset_of_string(&values, values.len())
		- (4 - (values.last().context("Hashes ORES must have data")?.len() + 1) % 4) % 4;

	cursor.write_all(b"\x42\x49\x4E\x31\x00\x08\x01\x00")?;
	cursor.write_all(&(i32::try_from(end_of_strings)? - 0x10).to_be_bytes())?;
	cursor.write_all(b"\x00\x00\x00\x00\x20\x00\x00\x00\x00\x00\x00\x00")?;
	cursor.write_all(&(i32::try_from(start_of_strings)? - 0x10).to_le_bytes())?;
	cursor.write_all(b"\x00\x00\x00\x00")?;
	cursor.write_all(&(i32::try_from(start_of_strings)? - 0x10).to_le_bytes())?;
	cursor.write_all(b"\x00\x00\x00\x00\x00\x00\x00\x00")?;
	cursor.write_all(&(i32::try_from(values.len())?).to_le_bytes())?;

	for (i, value) in values.iter().enumerate() {
		cursor.write_all(&i32::try_from(value.len())?.to_le_bytes())?;
		cursor.seek(SeekFrom::Current(-1))?;
		cursor.write_all(b"\x40\x00\x00\x00\x00")?;
		cursor.write_all(&i32::try_from(start_of_strings - 12 + offset_of_string(&values, i))?.to_le_bytes())?;
		cursor.write_all(b"\x00\x00\x00\x00")?;

		let hash = hex::decode(hashes[i])?;

		cursor.write_all(&hex::decode(
			hash[0..4]
				.iter()
				.rev()
				.map(|x| format!("{:0>2X}", x))
				.chain(hash[4..8].iter().rev().map(|x| format!("{:0>2X}", x)))
				.collect::<Vec<_>>()
				.join("")
		)?)?;
	}

	for (i, value) in values.iter().enumerate() {
		cursor.write_all(&i32::try_from(value.len() + 1)?.to_le_bytes())?;
		cursor.write_all(value.as_bytes())?;
		cursor.write_all(b"\x00")?;

		if i != values.len() - 1 {
			cursor.write_all(&vec![0u8; (4 - (value.len() + 1) % 4) % 4])?;
		}
	}

	cursor.write_all(b"\xED\xA5\xEB\x12")?;
	cursor.write_all(&i32::try_from(4 + (values.len() + 3) * 4)?.to_le_bytes())?;
	cursor.write_all(&i32::try_from(values.len() + 3)?.to_le_bytes())?;
	cursor.write_all(b"\x00\x00\x00\x00\x08\x00\x00\x00\x10\x00\x00\x00")?;

	for i in 0..values.len() {
		cursor.write_all(&i32::try_from(40 + i * 24)?.to_le_bytes())?;
	}

	ores
}

#[try_fn]
#[context("Couldn't read data as JSON ORES")]
pub fn parse_json_ores(bin_data: &[u8]) -> Result<Value> {
	let mut cursor = Cursor::new(bin_data);
	cursor.seek(SeekFrom::Start(36))?;

	let mut data = vec![0u8; bin_data.len() - 36 - 17];
	cursor.read_exact(&mut data)?;

	serde_json::from_slice(&data)?
}

#[try_fn]
#[context("Couldn't serialise data to JSON ORES")]
pub fn generate_json_ores(data: &Value) -> Result<Vec<u8>> {
	let str_data = serde_json::to_string(data)?;

	let mut ores = vec![];
	let mut cursor = Cursor::new(&mut ores);

	cursor.write_all(b"\x42\x49\x4E\x31\x00\x08\x01\x00")?;
	cursor.write_all(&i32::try_from(str_data.len() + 21)?.to_be_bytes())?;
	cursor.write_all(b"\x00\x00\x00\x00")?;
	cursor.write_all(&i32::try_from(str_data.len() | 0x40000000)?.to_le_bytes())?;
	cursor.write_all(b"\x00\x00\x00\x00\x14\x00\x00\x00\x00\x00\x00\x00")?;
	cursor.write_all(&i32::try_from(str_data.len() + 1)?.to_le_bytes())?;
	cursor.write_all(str_data.as_bytes())?;
	cursor.write_all(b"\x00\xED\xA5\xEB\x12\x08\x00\x00\x00\x01\x00\x00\x00\x08\x00\x00\x00")?;

	ores
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UnlockableItem {
	#[serde(rename = "Guid")]
	pub id: Uuid,

	#[serde(flatten)]
	pub data: IndexMap<String, Value>
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", content = "data")]
pub enum UnlockableInformation {
	Access { id: Option<String> },
	EvergreenMastery { id: Option<String> },
	Disguise { id: Option<String> },
	AgencyPickup { id: Option<String> },
	Weapon { id: Option<String> },
	Gear { id: Option<String> },
	Location { id: Option<String> },
	Package { id: Option<String> },
	LoadoutUnlock { id: Option<String> },
	Unknown { id: Option<String> }
}
