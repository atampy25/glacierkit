use std::io::{Cursor, Read};

use anyhow::{bail, Context, Result};
use fn_error_context::context;
use quickentity_rs::rpkg_structs::ResourceMeta;
use tryvial::try_fn;

#[derive(Clone, Debug)]
pub struct MaterialProperty {
	pub name: String,
	pub data: MaterialPropertyData
}

#[derive(Clone, Debug)]
pub enum MaterialPropertyData {
	Texture(Option<String>),
	ColorRGB(f32, f32, f32),
	ColorRGBA(f32, f32, f32, f32),
	Float(f32),
	Vector2(f32, f32),
	Vector3(f32, f32, f32),
	Vector4(f32, f32, f32, f32)
}

/// Get the properties of a material entity.
#[try_fn]
#[context("Couldn't get properties for material")]
pub fn get_material_properties(
	matt_data: &[u8],
	matt_meta: &ResourceMeta,
	matb_data: &[u8]
) -> Result<Vec<MaterialProperty>> {
	let mut properties = vec![];

	let mut matt = Cursor::new(matt_data);
	let mut matb = Cursor::new(matb_data);

	let mut prop_names = vec![];

	while !matb.is_empty() {
		// All MATB entries are strings apparently so this type field is useless
		let _ = {
			let mut x = [0u8; 1];
			matb.read_exact(&mut x)?;
			x[0]
		};

		let matb_string_length = u32::from_le_bytes({
			let mut x = [0u8; 4];
			matb.read_exact(&mut x)?;
			x
		});

		// I'm assuming that no one is using a 16-bit computer
		let mut string_data = vec![0; matb_string_length as usize];
		matb.read_exact(&mut string_data)?;

		prop_names.push(
			std::str::from_utf8(&string_data[0..string_data.len() - 1])
				.context("Invalid string in MATB entry")?
				.to_owned()
		);
	}

	let mut cur_entry = 0;

	while !matt.is_empty() {
		let entry_type = {
			let mut x = [0u8; 1];
			matt.read_exact(&mut x)?;
			x[0]
		};

		properties.push(MaterialProperty {
			name: prop_names
				.get(cur_entry)
				.context("Mismatched MATT/MATB entry count")?
				.into(),
			data: match entry_type {
				// A texture.
				1 => {
					let texture_dependency_index = u32::from_le_bytes({
						let mut x = [0u8; 4];
						matt.read_exact(&mut x)?;
						x
					});

					if texture_dependency_index != u32::MAX {
						MaterialPropertyData::Texture(Some(
							matt_meta
								.hash_reference_data
								.get(usize::try_from(texture_dependency_index)?)
								.context("No such texture dependency")?
								.hash
								.to_owned()
						))
					} else {
						MaterialPropertyData::Texture(None)
					}
				}

				// An RGB colour.
				2 => {
					let x = f32::from_le_bytes({
						let mut x = [0u8; 4];
						matt.read_exact(&mut x)?;
						x
					});

					let y = f32::from_le_bytes({
						let mut x = [0u8; 4];
						matt.read_exact(&mut x)?;
						x
					});

					let z = f32::from_le_bytes({
						let mut x = [0u8; 4];
						matt.read_exact(&mut x)?;
						x
					});

					MaterialPropertyData::ColorRGB(x, y, z)
				}

				// An RGBA colour.
				3 => {
					let x = f32::from_le_bytes({
						let mut x = [0u8; 4];
						matt.read_exact(&mut x)?;
						x
					});

					let y = f32::from_le_bytes({
						let mut x = [0u8; 4];
						matt.read_exact(&mut x)?;
						x
					});

					let z = f32::from_le_bytes({
						let mut x = [0u8; 4];
						matt.read_exact(&mut x)?;
						x
					});

					let w = f32::from_le_bytes({
						let mut x = [0u8; 4];
						matt.read_exact(&mut x)?;
						x
					});

					MaterialPropertyData::ColorRGBA(x, y, z, w)
				}

				// A float.
				4 => {
					let val = f32::from_le_bytes({
						let mut x = [0u8; 4];
						matt.read_exact(&mut x)?;
						x
					});

					MaterialPropertyData::Float(val)
				}

				// A Vector2.
				5 => {
					let x = f32::from_le_bytes({
						let mut x = [0u8; 4];
						matt.read_exact(&mut x)?;
						x
					});

					let y = f32::from_le_bytes({
						let mut x = [0u8; 4];
						matt.read_exact(&mut x)?;
						x
					});

					MaterialPropertyData::Vector2(x, y)
				}

				// A Vector3.
				6 => {
					let x = f32::from_le_bytes({
						let mut x = [0u8; 4];
						matt.read_exact(&mut x)?;
						x
					});

					let y = f32::from_le_bytes({
						let mut x = [0u8; 4];
						matt.read_exact(&mut x)?;
						x
					});

					let z = f32::from_le_bytes({
						let mut x = [0u8; 4];
						matt.read_exact(&mut x)?;
						x
					});

					MaterialPropertyData::Vector3(x, y, z)
				}

				// A Vector4.
				7 => {
					let x = f32::from_le_bytes({
						let mut x = [0u8; 4];
						matt.read_exact(&mut x)?;
						x
					});

					let y = f32::from_le_bytes({
						let mut x = [0u8; 4];
						matt.read_exact(&mut x)?;
						x
					});

					let z = f32::from_le_bytes({
						let mut x = [0u8; 4];
						matt.read_exact(&mut x)?;
						x
					});

					let w = f32::from_le_bytes({
						let mut x = [0u8; 4];
						matt.read_exact(&mut x)?;
						x
					});

					MaterialPropertyData::Vector4(x, y, z, w)
				}

				_ => bail!("Unrecognised MATT entry type: {}", entry_type)
			}
		});

		cur_entry += 1;
	}

	properties
}
