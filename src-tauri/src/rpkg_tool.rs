use std::io::{Cursor, Read};

use anyhow::{Context, Result};
use fn_error_context::context;
use quickentity_rs::rpkg_structs::{ResourceDependency, ResourceMeta};
use tryvial::try_fn;

use crate::rpkg::normalise_to_hash;

#[try_fn]
#[context("Couldn't parse RPKG tool meta file")]
pub fn parse_rpkg_meta(content: &[u8]) -> Result<ResourceMeta> {
	let mut cursor = Cursor::new(content);

	let mut hash_value = [0; 8];
	cursor.read_exact(&mut hash_value)?;
	let hash_value = format!("{:0>16X}", u64::from_le_bytes(hash_value));

	let mut hash_offset = [0; 8];
	cursor.read_exact(&mut hash_offset)?;
	let hash_offset = u64::from_le_bytes(hash_offset);

	let mut hash_size = [0; 4];
	cursor.read_exact(&mut hash_size)?;
	let hash_size = u32::from_le_bytes(hash_size);

	let mut hash_resource_type = [0; 4];
	cursor.read_exact(&mut hash_resource_type)?;
	let hash_resource_type = String::from_utf8_lossy(&hash_resource_type).to_string();

	let mut hash_reference_table_size = [0; 4];
	cursor.read_exact(&mut hash_reference_table_size)?;
	let hash_reference_table_size = u32::from_le_bytes(hash_reference_table_size);

	let mut hash_reference_table_dummy = [0; 4];
	cursor.read_exact(&mut hash_reference_table_dummy)?;
	let hash_reference_table_dummy = u32::from_le_bytes(hash_reference_table_dummy);

	let mut hash_size_final = [0; 4];
	cursor.read_exact(&mut hash_size_final)?;
	let hash_size_final = u32::from_le_bytes(hash_size_final);

	let mut hash_size_in_memory = [0; 4];
	cursor.read_exact(&mut hash_size_in_memory)?;
	let hash_size_in_memory = u32::from_le_bytes(hash_size_in_memory);

	let mut hash_size_in_video_memory = [0; 4];
	cursor.read_exact(&mut hash_size_in_video_memory)?;
	let hash_size_in_video_memory = u32::from_le_bytes(hash_size_in_video_memory);

	let mut dependencies: Vec<ResourceDependency> = vec![];

	if hash_reference_table_size != 0 {
		let mut hash_reference_count = [0; 4];
		cursor.read_exact(&mut hash_reference_count)?;
		let hash_reference_count = u32::from_le_bytes(hash_reference_count);
		let hash_reference_count = hash_reference_count & 0x3FFFFFFF;

		let mut flags = vec![];
		let mut references = vec![];

		for _ in 0..hash_reference_count {
			let mut flag = [0; 1];
			cursor.read_exact(&mut flag)?;
			flags.push(flag[0]);
		}

		for _ in 0..hash_reference_count {
			let mut reference = [0; 8];
			cursor.read_exact(&mut reference)?;
			references.push(u64::from_le_bytes(reference));
		}

		dependencies.extend(
			flags
				.iter()
				.zip(references)
				.map(|(flag, reference)| ResourceDependency {
					hash: format!("{:0>16X}", reference),
					flag: format!("{:X}", flag)
				})
		)
	}

	ResourceMeta {
		hash_offset,
		hash_reference_data: dependencies,
		hash_reference_table_dummy,
		hash_reference_table_size,
		hash_resource_type,
		hash_size,
		hash_size_final,
		hash_size_in_memory,
		hash_size_in_video_memory,
		hash_value
	}
}

#[try_fn]
#[context("Couldn't generate RPKG tool meta file")]
pub fn generate_rpkg_meta(meta: &ResourceMeta) -> Result<Vec<u8>> {
	let mut data = Vec::with_capacity(44);

	// Note: hash_path is not considered here despite it technically existing; this is in line with RPKG Tool's behaviour
	data.extend(u64::from_str_radix(&normalise_to_hash(meta.hash_value.to_owned()), 16)?.to_le_bytes());
	data.extend(meta.hash_offset.to_le_bytes());
	data.extend(meta.hash_size.to_le_bytes());
	data.extend(meta.hash_resource_type.as_bytes());

	data.extend(if meta.hash_reference_data.is_empty() {
		[0; 4]
	} else {
		u32::try_from(meta.hash_reference_data.len() * 9 + 4)
			.context("usize does not fit into u32")?
			.to_le_bytes()
	}); // Recalculate hash_reference_table_size

	data.extend(meta.hash_reference_table_dummy.to_le_bytes());
	data.extend(meta.hash_size_final.to_le_bytes());
	data.extend(meta.hash_size_in_memory.to_le_bytes());
	data.extend(meta.hash_size_in_video_memory.to_le_bytes());

	if !meta.hash_reference_data.is_empty() {
		data.extend((u32::try_from(meta.hash_reference_data.len())? | 0xC0000000).to_le_bytes());

		for reference in &meta.hash_reference_data {
			data.push(u8::from_str_radix(&reference.flag, 16)?);
		}

		for reference in &meta.hash_reference_data {
			data.extend(u64::from_str_radix(&normalise_to_hash(reference.hash.to_owned()), 16)?.to_le_bytes());
		}
	}

	data
}
