use std::ffi::{CStr, CString};

use anyhow::{Context, Result, bail};
use fn_error_context::context;
use hitman_commons::game::GameVersion;
use hitman_commons::metadata::ResourceType;
use hitman_commons::resourcelib::{
	EntityBlueprint, EntityBlueprintLegacy, EntityFactory, EntityFactoryLegacy, Property
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tryvial::try_fn;

mod bindings_2;
mod bindings_2016;
mod bindings_3;

use self::bindings_2::{HM2_GetConverterForResource, HM2_GetGeneratorForResource, JsonString as JsonString2};
use self::bindings_3::{HM3_GetConverterForResource, HM3_GetGeneratorForResource, JsonString as JsonString3};
use self::bindings_2016::{
	HM2016_GetConverterForResource, HM2016_GetGeneratorForResource, JsonString as JsonString2016
};

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib TEMP")]
pub fn h3_convert_binary_to_factory(data: &[u8]) -> Result<EntityFactory> {
	unsafe {
		let converter = HM3_GetConverterForResource(CString::new("TEMP")?.as_ptr());

		if converter.is_null() {
			bail!("Couldn't get ResourceLib converter")
		}

		let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

		if json_string.is_null() {
			bail!("Couldn't convert data to JsonString")
		}

		let res = serde_json::from_str(
			CStr::from_bytes_with_nul(std::slice::from_raw_parts(
				(*json_string).JsonData.cast(),
				(*json_string).StrSize + 1 // include the null byte in the slice
			))
			.context("Couldn't construct CStr from JsonString data")?
			.to_str()
			.context("Couldn't convert CStr to str")?
		)
		.context("Couldn't deserialise returned JsonString as EntityFactory")?;

		(*converter).FreeJsonString.unwrap()(json_string);

		res
	}
}

#[try_fn]
#[context("Couldn't convert ResourceLib TEMP to binary data")]
pub fn h3_convert_factory_to_binary(data: &EntityFactory) -> Result<Vec<u8>> {
	unsafe {
		let generator = HM3_GetGeneratorForResource(CString::new("TEMP")?.as_ptr());

		if generator.is_null() {
			bail!("Couldn't get ResourceLib generator")
		}

		let json_string = CString::new(serde_json::to_string(data)?)?;
		let json_string = JsonString3 {
			JsonData: json_string.as_ptr(),
			StrSize: json_string.as_bytes().len()
		};

		let resource_mem =
			(*generator).FromJsonStringToResourceMem.unwrap()(json_string.JsonData, json_string.StrSize, false);

		if resource_mem.is_null() {
			bail!("Couldn't convert data to ResourceMem")
		}

		let res = std::slice::from_raw_parts((*resource_mem).ResourceData.cast(), (*resource_mem).DataSize).to_owned();

		(*generator).FreeResourceMem.unwrap()(resource_mem);

		res
	}
}

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib TBLU")]
pub fn h3_convert_binary_to_blueprint(data: &[u8]) -> Result<EntityBlueprint> {
	unsafe {
		let converter = HM3_GetConverterForResource(CString::new("TBLU")?.as_ptr());

		if converter.is_null() {
			bail!("Couldn't get ResourceLib converter")
		}

		let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

		if json_string.is_null() {
			bail!("Couldn't convert data to JsonString")
		}

		let res = serde_json::from_str(
			CStr::from_bytes_with_nul(std::slice::from_raw_parts(
				(*json_string).JsonData.cast(),
				(*json_string).StrSize + 1 // include the null byte in the slice
			))
			.context("Couldn't construct CStr from JsonString data")?
			.to_str()
			.context("Couldn't convert CStr to str")?
		)
		.context("Couldn't deserialise returned JsonString as EntityBlueprint")?;

		(*converter).FreeJsonString.unwrap()(json_string);

		res
	}
}

#[try_fn]
#[context("Couldn't convert ResourceLib TBLU to binary data")]
pub fn h3_convert_blueprint_to_binary(data: &EntityBlueprint) -> Result<Vec<u8>> {
	unsafe {
		let generator = HM3_GetGeneratorForResource(CString::new("TBLU")?.as_ptr());

		if generator.is_null() {
			bail!("Couldn't get ResourceLib generator")
		}

		let json_string = CString::new(serde_json::to_string(data)?)?;
		let json_string = JsonString3 {
			JsonData: json_string.as_ptr(),
			StrSize: json_string.as_bytes().len()
		};

		let resource_mem =
			(*generator).FromJsonStringToResourceMem.unwrap()(json_string.JsonData, json_string.StrSize, false);

		if resource_mem.is_null() {
			bail!("Couldn't convert data to ResourceMem")
		}

		let res = std::slice::from_raw_parts((*resource_mem).ResourceData.cast(), (*resource_mem).DataSize).to_owned();

		(*generator).FreeResourceMem.unwrap()(resource_mem);

		res
	}
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SCppEntity {
	pub blueprint_index_in_resource_header: i32,
	pub property_values: Vec<Property>
}

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib CPPT")]
pub fn h3_convert_cppt(data: &[u8]) -> Result<SCppEntity> {
	unsafe {
		let converter = HM3_GetConverterForResource(CString::new("CPPT")?.as_ptr());

		if converter.is_null() {
			bail!("Couldn't get ResourceLib converter")
		}

		let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

		if json_string.is_null() {
			bail!("Couldn't convert data to JsonString")
		}

		let res = serde_json::from_str(
			CStr::from_bytes_with_nul(std::slice::from_raw_parts(
				(*json_string).JsonData.cast(),
				(*json_string).StrSize + 1 // include the null byte in the slice
			))
			.context("Couldn't construct CStr from JsonString data")?
			.to_str()
			.context("Couldn't convert CStr to str")?
		)
		.context("Couldn't deserialise returned JsonString as SCppEntity")?;

		(*converter).FreeJsonString.unwrap()(json_string);

		res
	}
}

#[derive(Serialize, Deserialize)]
pub struct SwitchGroup {
	pub m_aSwitches: Vec<String>
}

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib DSWB")]
pub fn h3_convert_dswb(data: &[u8]) -> Result<SwitchGroup> {
	unsafe {
		let converter = HM3_GetConverterForResource(CString::new("DSWB")?.as_ptr());

		if converter.is_null() {
			bail!("Couldn't get ResourceLib converter")
		}

		let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

		if json_string.is_null() {
			bail!("Couldn't convert data to JsonString")
		}

		let res = serde_json::from_str(
			CStr::from_bytes_with_nul(std::slice::from_raw_parts(
				(*json_string).JsonData.cast(),
				(*json_string).StrSize + 1 // include the null byte in the slice
			))
			.context("Couldn't construct CStr from JsonString data")?
			.to_str()
			.context("Couldn't convert CStr to str")?
		)
		.context("Couldn't deserialise returned JsonString as SwitchGroup")?;

		(*converter).FreeJsonString.unwrap()(json_string);

		res
	}
}

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib WSGB")]
pub fn h3_convert_wsgb(data: &[u8]) -> Result<SwitchGroup> {
	unsafe {
		let converter = HM3_GetConverterForResource(CString::new("WSGB")?.as_ptr());

		if converter.is_null() {
			bail!("Couldn't get ResourceLib converter")
		}

		let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

		if json_string.is_null() {
			bail!("Couldn't convert data to JsonString")
		}

		let res = serde_json::from_str(
			CStr::from_bytes_with_nul(std::slice::from_raw_parts(
				(*json_string).JsonData.cast(),
				(*json_string).StrSize + 1 // include the null byte in the slice
			))
			.context("Couldn't construct CStr from JsonString data")?
			.to_str()
			.context("Couldn't convert CStr to str")?
		)
		.context("Couldn't deserialise returned JsonString as SwitchGroup")?;

		(*converter).FreeJsonString.unwrap()(json_string);

		res
	}
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SExtendedCppEntityBlueprint {
	pub properties: Vec<SExtendedCppEntityProperty>
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SExtendedCppEntityProperty {
	pub property_name: String,
	pub property_type: EExtendedPropertyType,
	pub rt_editable: bool,
	pub extra_data: u64
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
#[allow(non_camel_case_types)]
pub enum EExtendedPropertyType {
	TYPE_RESOURCEPTR,
	TYPE_INT32,
	TYPE_UINT32,
	TYPE_FLOAT,
	TYPE_STRING,
	TYPE_BOOL,
	TYPE_ENTITYREF,
	TYPE_VARIANT
}

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib ECPB")]
pub fn h3_convert_ecpb(data: &[u8]) -> Result<SExtendedCppEntityBlueprint> {
	unsafe {
		let converter = HM3_GetConverterForResource(CString::new("ECPB")?.as_ptr());

		if converter.is_null() {
			bail!("Couldn't get ResourceLib converter")
		}

		let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

		if json_string.is_null() {
			bail!("Couldn't convert data to JsonString")
		}

		let res = serde_json::from_str(
			CStr::from_bytes_with_nul(std::slice::from_raw_parts(
				(*json_string).JsonData.cast(),
				(*json_string).StrSize + 1 // include the null byte in the slice
			))
			.context("Couldn't construct CStr from JsonString data")?
			.to_str()
			.context("Couldn't convert CStr to str")?
		)
		.context("Couldn't deserialise returned JsonString as SExtendedCppEntityBlueprint")?;

		(*converter).FreeJsonString.unwrap()(json_string);

		res
	}
}

#[derive(Serialize, Deserialize)]
pub struct SUIControlBlueprint {
	pub m_aAttributes: Vec<SAttributeInfo>,
	pub m_aSpecialMethods: Vec<String>
}

#[derive(Serialize, Deserialize)]
pub struct SAttributeInfo {
	pub m_eKind: EAttributeKind,
	pub m_eType: EAttributeType,
	pub m_sName: String
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
#[allow(non_camel_case_types)]
pub enum EAttributeKind {
	E_ATTRIBUTE_KIND_PROPERTY,
	E_ATTRIBUTE_KIND_INPUT_PIN,
	E_ATTRIBUTE_KIND_OUTPUT_PIN
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
#[allow(non_camel_case_types)]
pub enum EAttributeType {
	E_ATTRIBUTE_TYPE_VOID,
	E_ATTRIBUTE_TYPE_INT,
	E_ATTRIBUTE_TYPE_FLOAT,
	E_ATTRIBUTE_TYPE_STRING,
	E_ATTRIBUTE_TYPE_BOOL,
	E_ATTRIBUTE_TYPE_ENTITYREF,
	E_ATTRIBUTE_TYPE_OBJECT
}

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib UICB")]
pub fn convert_uicb(data: &[u8]) -> Result<SUIControlBlueprint> {
	unsafe {
		let converter = HM3_GetConverterForResource(CString::new("UICB")?.as_ptr());

		if converter.is_null() {
			bail!("Couldn't get ResourceLib converter")
		}

		let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

		if json_string.is_null() {
			bail!("Couldn't convert data to JsonString")
		}

		let res = serde_json::from_str(
			CStr::from_bytes_with_nul(std::slice::from_raw_parts(
				(*json_string).JsonData.cast(),
				(*json_string).StrSize + 1 // include the null byte in the slice
			))
			.context("Couldn't construct CStr from JsonString data")?
			.to_str()
			.context("Couldn't convert CStr to str")?
		)
		.context("Couldn't deserialise returned JsonString as SUIControlBlueprint")?;

		(*converter).FreeJsonString.unwrap()(json_string);

		res
	}
}

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib TEMP")]
pub fn h2_convert_binary_to_factory(data: &[u8]) -> Result<EntityFactory> {
	unsafe {
		let converter = HM2_GetConverterForResource(CString::new("TEMP")?.as_ptr());

		if converter.is_null() {
			bail!("Couldn't get ResourceLib converter")
		}

		let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

		if json_string.is_null() {
			bail!("Couldn't convert data to JsonString")
		}

		let res = serde_json::from_str(
			CStr::from_bytes_with_nul(std::slice::from_raw_parts(
				(*json_string).JsonData.cast(),
				(*json_string).StrSize + 1 // include the null byte in the slice
			))
			.context("Couldn't construct CStr from JsonString data")?
			.to_str()
			.context("Couldn't convert CStr to str")?
		)
		.context("Couldn't deserialise returned JsonString as EntityFactory")?;

		(*converter).FreeJsonString.unwrap()(json_string);

		res
	}
}

#[try_fn]
#[context("Couldn't convert ResourceLib TEMP to binary data")]
pub fn h2_convert_factory_to_binary(data: &EntityFactory) -> Result<Vec<u8>> {
	unsafe {
		let generator = HM2_GetGeneratorForResource(CString::new("TEMP")?.as_ptr());

		if generator.is_null() {
			bail!("Couldn't get ResourceLib generator")
		}

		let json_string = CString::new(serde_json::to_string(data)?)?;
		let json_string = JsonString2 {
			JsonData: json_string.as_ptr(),
			StrSize: json_string.as_bytes().len()
		};

		let resource_mem =
			(*generator).FromJsonStringToResourceMem.unwrap()(json_string.JsonData, json_string.StrSize, false);

		if resource_mem.is_null() {
			bail!("Couldn't convert data to ResourceMem")
		}

		let res = std::slice::from_raw_parts((*resource_mem).ResourceData.cast(), (*resource_mem).DataSize).to_owned();

		(*generator).FreeResourceMem.unwrap()(resource_mem);

		res
	}
}

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib TBLU")]
pub fn h2_convert_binary_to_blueprint(data: &[u8]) -> Result<EntityBlueprint> {
	unsafe {
		let converter = HM2_GetConverterForResource(CString::new("TBLU")?.as_ptr());

		if converter.is_null() {
			bail!("Couldn't get ResourceLib converter")
		}

		let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

		if json_string.is_null() {
			bail!("Couldn't convert data to JsonString")
		}

		let res = serde_json::from_str(
			CStr::from_bytes_with_nul(std::slice::from_raw_parts(
				(*json_string).JsonData.cast(),
				(*json_string).StrSize + 1 // include the null byte in the slice
			))
			.context("Couldn't construct CStr from JsonString data")?
			.to_str()
			.context("Couldn't convert CStr to str")?
		)
		.context("Couldn't deserialise returned JsonString as EntityBlueprint")?;

		(*converter).FreeJsonString.unwrap()(json_string);

		res
	}
}

#[try_fn]
#[context("Couldn't convert ResourceLib TBLU to binary data")]
pub fn h2_convert_blueprint_to_binary(data: &EntityBlueprint) -> Result<Vec<u8>> {
	unsafe {
		let generator = HM2_GetGeneratorForResource(CString::new("TBLU")?.as_ptr());

		if generator.is_null() {
			bail!("Couldn't get ResourceLib generator")
		}

		let json_string = CString::new(serde_json::to_string(data)?)?;
		let json_string = JsonString2 {
			JsonData: json_string.as_ptr(),
			StrSize: json_string.as_bytes().len()
		};

		let resource_mem =
			(*generator).FromJsonStringToResourceMem.unwrap()(json_string.JsonData, json_string.StrSize, false);

		if resource_mem.is_null() {
			bail!("Couldn't convert data to ResourceMem")
		}

		let res = std::slice::from_raw_parts((*resource_mem).ResourceData.cast(), (*resource_mem).DataSize).to_owned();

		(*generator).FreeResourceMem.unwrap()(resource_mem);

		res
	}
}

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib CPPT")]
pub fn h2_convert_cppt(data: &[u8]) -> Result<SCppEntity> {
	unsafe {
		let converter = HM2_GetConverterForResource(CString::new("CPPT")?.as_ptr());

		if converter.is_null() {
			bail!("Couldn't get ResourceLib converter")
		}

		let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

		if json_string.is_null() {
			bail!("Couldn't convert data to JsonString")
		}

		let res = serde_json::from_str(
			CStr::from_bytes_with_nul(std::slice::from_raw_parts(
				(*json_string).JsonData.cast(),
				(*json_string).StrSize + 1 // include the null byte in the slice
			))
			.context("Couldn't construct CStr from JsonString data")?
			.to_str()
			.context("Couldn't convert CStr to str")?
		)
		.context("Couldn't deserialise returned JsonString as SCppEntity")?;

		(*converter).FreeJsonString.unwrap()(json_string);

		res
	}
}

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib DSWB")]
pub fn h2_convert_dswb(data: &[u8]) -> Result<SwitchGroup> {
	unsafe {
		let converter = HM2_GetConverterForResource(CString::new("DSWB")?.as_ptr());

		if converter.is_null() {
			bail!("Couldn't get ResourceLib converter")
		}

		let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

		if json_string.is_null() {
			bail!("Couldn't convert data to JsonString")
		}

		let res = serde_json::from_str(
			CStr::from_bytes_with_nul(std::slice::from_raw_parts(
				(*json_string).JsonData.cast(),
				(*json_string).StrSize + 1 // include the null byte in the slice
			))
			.context("Couldn't construct CStr from JsonString data")?
			.to_str()
			.context("Couldn't convert CStr to str")?
		)
		.context("Couldn't deserialise returned JsonString as SwitchGroup")?;

		(*converter).FreeJsonString.unwrap()(json_string);

		res
	}
}

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib WSGB")]
pub fn h2_convert_wsgb(data: &[u8]) -> Result<SwitchGroup> {
	unsafe {
		let converter = HM2_GetConverterForResource(CString::new("WSGB")?.as_ptr());

		if converter.is_null() {
			bail!("Couldn't get ResourceLib converter")
		}

		let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

		if json_string.is_null() {
			bail!("Couldn't convert data to JsonString")
		}

		let res = serde_json::from_str(
			CStr::from_bytes_with_nul(std::slice::from_raw_parts(
				(*json_string).JsonData.cast(),
				(*json_string).StrSize + 1 // include the null byte in the slice
			))
			.context("Couldn't construct CStr from JsonString data")?
			.to_str()
			.context("Couldn't convert CStr to str")?
		)
		.context("Couldn't deserialise returned JsonString as SwitchGroup")?;

		(*converter).FreeJsonString.unwrap()(json_string);

		res
	}
}

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib ECPB")]
pub fn h2_convert_ecpb(data: &[u8]) -> Result<SExtendedCppEntityBlueprint> {
	unsafe {
		let converter = HM2_GetConverterForResource(CString::new("ECPB")?.as_ptr());

		if converter.is_null() {
			bail!("Couldn't get ResourceLib converter")
		}

		let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

		if json_string.is_null() {
			bail!("Couldn't convert data to JsonString")
		}

		let res = serde_json::from_str(
			CStr::from_bytes_with_nul(std::slice::from_raw_parts(
				(*json_string).JsonData.cast(),
				(*json_string).StrSize + 1 // include the null byte in the slice
			))
			.context("Couldn't construct CStr from JsonString data")?
			.to_str()
			.context("Couldn't convert CStr to str")?
		)
		.context("Couldn't deserialise returned JsonString as SExtendedCppEntityBlueprint")?;

		(*converter).FreeJsonString.unwrap()(json_string);

		res
	}
}

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib TEMP")]
pub fn h2016_convert_binary_to_factory(data: &[u8]) -> Result<EntityFactoryLegacy> {
	unsafe {
		let converter = HM2016_GetConverterForResource(CString::new("TEMP")?.as_ptr());

		if converter.is_null() {
			bail!("Couldn't get ResourceLib converter")
		}

		let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

		if json_string.is_null() {
			bail!("Couldn't convert data to JsonString")
		}

		let res = serde_json::from_str(
			CStr::from_bytes_with_nul(std::slice::from_raw_parts(
				(*json_string).JsonData.cast(),
				(*json_string).StrSize + 1 // include the null byte in the slice
			))
			.context("Couldn't construct CStr from JsonString data")?
			.to_str()
			.context("Couldn't convert CStr to str")?
		)
		.context("Couldn't deserialise returned JsonString as EntityFactoryLegacy")?;

		(*converter).FreeJsonString.unwrap()(json_string);

		res
	}
}

#[try_fn]
#[context("Couldn't convert ResourceLib TEMP to binary data")]
pub fn h2016_convert_factory_to_binary(data: &EntityFactoryLegacy) -> Result<Vec<u8>> {
	unsafe {
		let generator = HM2016_GetGeneratorForResource(CString::new("TEMP")?.as_ptr());

		if generator.is_null() {
			bail!("Couldn't get ResourceLib generator")
		}

		let json_string = CString::new(serde_json::to_string(data)?)?;
		let json_string = JsonString2016 {
			JsonData: json_string.as_ptr(),
			StrSize: json_string.as_bytes().len()
		};

		let resource_mem =
			(*generator).FromJsonStringToResourceMem.unwrap()(json_string.JsonData, json_string.StrSize, false);

		if resource_mem.is_null() {
			bail!("Couldn't convert data to ResourceMem")
		}

		let res = std::slice::from_raw_parts((*resource_mem).ResourceData.cast(), (*resource_mem).DataSize).to_owned();

		(*generator).FreeResourceMem.unwrap()(resource_mem);

		res
	}
}

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib TBLU")]
pub fn h2016_convert_binary_to_blueprint(data: &[u8]) -> Result<EntityBlueprintLegacy> {
	unsafe {
		let converter = HM2016_GetConverterForResource(CString::new("TBLU")?.as_ptr());

		if converter.is_null() {
			bail!("Couldn't get ResourceLib converter")
		}

		let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

		if json_string.is_null() {
			bail!("Couldn't convert data to JsonString")
		}

		let res = serde_json::from_str(
			CStr::from_bytes_with_nul(std::slice::from_raw_parts(
				(*json_string).JsonData.cast(),
				(*json_string).StrSize + 1 // include the null byte in the slice
			))
			.context("Couldn't construct CStr from JsonString data")?
			.to_str()
			.context("Couldn't convert CStr to str")?
		)
		.context("Couldn't deserialise returned JsonString as EntityBlueprintLegacy")?;

		(*converter).FreeJsonString.unwrap()(json_string);

		res
	}
}

#[try_fn]
#[context("Couldn't convert ResourceLib TBLU to binary data")]
pub fn h2016_convert_blueprint_to_binary(data: &EntityBlueprintLegacy) -> Result<Vec<u8>> {
	unsafe {
		let generator = HM2016_GetGeneratorForResource(CString::new("TBLU")?.as_ptr());

		if generator.is_null() {
			bail!("Couldn't get ResourceLib generator")
		}

		let json_string = CString::new(serde_json::to_string(data)?)?;
		let json_string = JsonString2016 {
			JsonData: json_string.as_ptr(),
			StrSize: json_string.as_bytes().len()
		};

		let resource_mem =
			(*generator).FromJsonStringToResourceMem.unwrap()(json_string.JsonData, json_string.StrSize, false);

		if resource_mem.is_null() {
			bail!("Couldn't convert data to ResourceMem")
		}

		let res = std::slice::from_raw_parts((*resource_mem).ResourceData.cast(), (*resource_mem).DataSize).to_owned();

		(*generator).FreeResourceMem.unwrap()(resource_mem);

		res
	}
}

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib CPPT")]
pub fn h2016_convert_cppt(data: &[u8]) -> Result<SCppEntity> {
	unsafe {
		let converter = HM2016_GetConverterForResource(CString::new("CPPT")?.as_ptr());

		if converter.is_null() {
			bail!("Couldn't get ResourceLib converter")
		}

		let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

		if json_string.is_null() {
			bail!("Couldn't convert data to JsonString")
		}

		let res = serde_json::from_str(
			CStr::from_bytes_with_nul(std::slice::from_raw_parts(
				(*json_string).JsonData.cast(),
				(*json_string).StrSize + 1 // include the null byte in the slice
			))
			.context("Couldn't construct CStr from JsonString data")?
			.to_str()
			.context("Couldn't convert CStr to str")?
		)
		.context("Couldn't deserialise returned JsonString as SCppEntity")?;

		(*converter).FreeJsonString.unwrap()(json_string);

		res
	}
}

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib DSWB")]
pub fn h2016_convert_dswb(data: &[u8]) -> Result<SwitchGroup> {
	unsafe {
		let converter = HM2016_GetConverterForResource(CString::new("DSWB")?.as_ptr());

		if converter.is_null() {
			bail!("Couldn't get ResourceLib converter")
		}

		let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

		if json_string.is_null() {
			bail!("Couldn't convert data to JsonString")
		}

		let res = serde_json::from_str(
			CStr::from_bytes_with_nul(std::slice::from_raw_parts(
				(*json_string).JsonData.cast(),
				(*json_string).StrSize + 1 // include the null byte in the slice
			))
			.context("Couldn't construct CStr from JsonString data")?
			.to_str()
			.context("Couldn't convert CStr to str")?
		)
		.context("Couldn't deserialise returned JsonString as SwitchGroup")?;

		(*converter).FreeJsonString.unwrap()(json_string);

		res
	}
}

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib ECPB")]
pub fn h2016_convert_ecpb(data: &[u8]) -> Result<SExtendedCppEntityBlueprint> {
	unsafe {
		let converter = HM2016_GetConverterForResource(CString::new("ECPB")?.as_ptr());

		if converter.is_null() {
			bail!("Couldn't get ResourceLib converter")
		}

		let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

		if json_string.is_null() {
			bail!("Couldn't convert data to JsonString")
		}

		let res = serde_json::from_str(
			CStr::from_bytes_with_nul(std::slice::from_raw_parts(
				(*json_string).JsonData.cast(),
				(*json_string).StrSize + 1 // include the null byte in the slice
			))
			.context("Couldn't construct CStr from JsonString data")?
			.to_str()
			.context("Couldn't convert CStr to str")?
		)
		.context("Couldn't deserialise returned JsonString as SExtendedCppEntityBlueprint")?;

		(*converter).FreeJsonString.unwrap()(json_string);

		res
	}
}

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib WSGB")]
pub fn h2016_convert_wsgb(data: &[u8]) -> Result<SwitchGroup> {
	unsafe {
		let converter = HM2016_GetConverterForResource(CString::new("WSGB")?.as_ptr());

		if converter.is_null() {
			bail!("Couldn't get ResourceLib converter")
		}

		let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

		if json_string.is_null() {
			bail!("Couldn't convert data to JsonString")
		}

		let res = serde_json::from_str(
			CStr::from_bytes_with_nul(std::slice::from_raw_parts(
				(*json_string).JsonData.cast(),
				(*json_string).StrSize + 1 // include the null byte in the slice
			))
			.context("Couldn't construct CStr from JsonString data")?
			.to_str()
			.context("Couldn't convert CStr to str")?
		)
		.context("Couldn't deserialise returned JsonString as SwitchGroup")?;

		(*converter).FreeJsonString.unwrap()(json_string);

		res
	}
}

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib format")]
pub fn convert_generic<T: DeserializeOwned>(data: &[u8], game: GameVersion, resource_type: ResourceType) -> Result<T> {
	unsafe {
		match game {
			GameVersion::H1 => {
				let converter = HM2016_GetConverterForResource(CString::new(resource_type)?.as_ptr());

				if converter.is_null() {
					bail!("Couldn't get ResourceLib converter")
				}

				let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

				if json_string.is_null() {
					bail!("Couldn't convert data to JsonString")
				}

				let res = serde_json::from_str(
					CStr::from_bytes_with_nul(std::slice::from_raw_parts(
						(*json_string).JsonData.cast(),
						(*json_string).StrSize + 1 // include the null byte in the slice
					))
					.context("Couldn't construct CStr from JsonString data")?
					.to_str()
					.context("Couldn't convert CStr to str")?
				)
				.context("Couldn't deserialise returned JsonString as Value")?;

				(*converter).FreeJsonString.unwrap()(json_string);

				res
			}

			GameVersion::H2 => {
				let converter = HM2_GetConverterForResource(CString::new(resource_type)?.as_ptr());

				if converter.is_null() {
					bail!("Couldn't get ResourceLib converter")
				}

				let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

				if json_string.is_null() {
					bail!("Couldn't convert data to JsonString")
				}

				let res = serde_json::from_str(
					CStr::from_bytes_with_nul(std::slice::from_raw_parts(
						(*json_string).JsonData.cast(),
						(*json_string).StrSize + 1 // include the null byte in the slice
					))
					.context("Couldn't construct CStr from JsonString data")?
					.to_str()
					.context("Couldn't convert CStr to str")?
				)
				.context("Couldn't deserialise returned JsonString as Value")?;

				(*converter).FreeJsonString.unwrap()(json_string);

				res
			}

			GameVersion::H3 => {
				let converter = HM3_GetConverterForResource(CString::new(resource_type)?.as_ptr());

				if converter.is_null() {
					bail!("Couldn't get ResourceLib converter")
				}

				let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

				if json_string.is_null() {
					bail!("Couldn't convert data to JsonString")
				}

				let res = serde_json::from_str(
					CStr::from_bytes_with_nul(std::slice::from_raw_parts(
						(*json_string).JsonData.cast(),
						(*json_string).StrSize + 1 // include the null byte in the slice
					))
					.context("Couldn't construct CStr from JsonString data")?
					.to_str()
					.context("Couldn't convert CStr to str")?
				)
				.context("Couldn't deserialise returned JsonString as Value")?;

				(*converter).FreeJsonString.unwrap()(json_string);

				res
			}
		}
	}
}

#[try_fn]
#[context("Couldn't convert binary data to ResourceLib format")]
pub fn convert_generic_str(data: &[u8], game: GameVersion, resource_type: ResourceType) -> Result<String> {
	unsafe {
		match game {
			GameVersion::H1 => {
				let converter = HM2016_GetConverterForResource(CString::new(resource_type)?.as_ptr());

				if converter.is_null() {
					bail!("Couldn't get ResourceLib converter")
				}

				let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

				if json_string.is_null() {
					bail!("Couldn't convert data to JsonString")
				}

				let res = CStr::from_bytes_with_nul(std::slice::from_raw_parts(
					(*json_string).JsonData.cast(),
					(*json_string).StrSize + 1 // include the null byte in the slice
				))
				.context("Couldn't construct CStr from JsonString data")?
				.to_str()
				.context("Couldn't convert CStr to str")?
				.to_owned();

				(*converter).FreeJsonString.unwrap()(json_string);

				res
			}

			GameVersion::H2 => {
				let converter = HM2_GetConverterForResource(CString::new(resource_type)?.as_ptr());

				if converter.is_null() {
					bail!("Couldn't get ResourceLib converter")
				}

				let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

				if json_string.is_null() {
					bail!("Couldn't convert data to JsonString")
				}

				let res = CStr::from_bytes_with_nul(std::slice::from_raw_parts(
					(*json_string).JsonData.cast(),
					(*json_string).StrSize + 1 // include the null byte in the slice
				))
				.context("Couldn't construct CStr from JsonString data")?
				.to_str()
				.context("Couldn't convert CStr to str")?
				.to_owned();

				(*converter).FreeJsonString.unwrap()(json_string);

				res
			}

			GameVersion::H3 => {
				let converter = HM3_GetConverterForResource(CString::new(resource_type)?.as_ptr());

				if converter.is_null() {
					bail!("Couldn't get ResourceLib converter")
				}

				let json_string = (*converter).FromMemoryToJsonString.unwrap()(data.as_ptr().cast(), data.len());

				if json_string.is_null() {
					bail!("Couldn't convert data to JsonString")
				}

				let res = CStr::from_bytes_with_nul(std::slice::from_raw_parts(
					(*json_string).JsonData.cast(),
					(*json_string).StrSize + 1 // include the null byte in the slice
				))
				.context("Couldn't construct CStr from JsonString data")?
				.to_str()
				.context("Couldn't convert CStr to str")?
				.to_owned();

				(*converter).FreeJsonString.unwrap()(json_string);

				res
			}
		}
	}
}
