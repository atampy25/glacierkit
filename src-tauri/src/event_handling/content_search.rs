use std::{ops::Deref, time::Instant};

use anyhow::{anyhow, Context, Result};
use arc_swap::ArcSwap;
use fn_error_context::context;
use hashbrown::{HashMap, HashSet};
use itertools::Itertools;
use quickentity_rs::{convert_2016_blueprint_to_modern, convert_2016_factory_to_modern, convert_to_qn};
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, ParallelExtend, ParallelIterator};
use regex::bytes::Regex;
use rpkg_rs::resource::runtime_resource_id::RuntimeResourceID;
use serde::Serialize;
use serde_json::{to_string, to_vec};
use tauri::{AppHandle, Manager};
use tonytools::hmlanguages;
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	finish_task,
	game_detection::GameVersion,
	languages::get_language_map,
	model::{AppSettings, AppState, EditorData, EditorState, EditorType, GlobalRequest, Request},
	ores::{parse_hashes_ores, parse_json_ores},
	resourcelib::{
		convert_generic_str, h2016_convert_binary_to_blueprint, h2016_convert_binary_to_factory,
		h2_convert_binary_to_blueprint, h2_convert_binary_to_factory, h3_convert_binary_to_blueprint,
		h3_convert_binary_to_factory
	},
	rpkg::{convert_resource_info_to_rpkg_meta_no_hl, extract_latest_resource_no_hl},
	send_request, start_task
};

#[try_fn]
#[context("Couldn't perform content search")]
pub fn start_content_search(app: &AppHandle, query: String, filetypes: Vec<String>, use_qn_format: bool) -> Result<()> {
	let app_settings = app.state::<ArcSwap<AppSettings>>();
	let app_state = app.state::<AppState>();

	let query = Regex::new(&query).context("Invalid regex")?;

	let filetypes = filetypes.into_iter().collect::<HashSet<String>>();

	if let Some(game_files) = app_state.game_files.load().as_ref()
		&& let Some(hash_list) = app_state.hash_list.load().as_ref()
		&& let Some(install) = app_settings.load().game_install.as_ref()
	{
		let game_version = app_state
			.game_installs
			.iter()
			.try_find(|x| anyhow::Ok(x.path == *install))?
			.context("No such game install")?
			.version;

		let resources = game_files
			.partitions()
			.into_par_iter()
			.rev()
			.flat_map(|partition| {
				partition
					.latest_resources()
					.into_par_iter()
					.map(move |(resource, _)| (resource.rrid(), (partition, resource)))
			})
			.collect::<HashMap<_, _>>();

		let mut matching_ids = vec![];

		let total_resources = resources.len();

		let mut progress_task = start_task(app, format!("Searching game files for \"{query}\": 0%"))?;
		let mut last_percent = 0;

		let start_time = Instant::now();

		for (progress, chunk) in resources.into_iter().chunks(1000).into_iter().enumerate() {
			matching_ids.par_extend(
				chunk
					.collect_vec()
					.into_par_iter()
					.filter(|(resource_id, (partition, resource_info))| {
						let filetype = resource_info.data_type();

						if filetypes.contains(&filetype) {
							match filetype.as_ref() {
								"TEMP" => {
									let s: Option<Vec<u8>> = try {
										if use_qn_format {
											let (temp_data, temp_meta) = (
												partition.read_resource(resource_id).ok()?,
												convert_resource_info_to_rpkg_meta_no_hl(resource_info)
											);

											let factory = match game_version {
												GameVersion::H1 => convert_2016_factory_to_modern(
													&h2016_convert_binary_to_factory(&temp_data).ok()?
												),

												GameVersion::H2 => h2_convert_binary_to_factory(&temp_data).ok()?,

												GameVersion::H3 => h3_convert_binary_to_factory(&temp_data).ok()?
											};

											let blueprint_hash = &temp_meta
												.hash_reference_data
												.get(factory.blueprint_index_in_resource_header as usize)?
												.hash;

											let tblu_rrid = RuntimeResourceID::from_hex_string(blueprint_hash).ok()?;

											let (tblu_data, tblu_meta) = (
												partition.read_resource(&tblu_rrid).ok()?,
												convert_resource_info_to_rpkg_meta_no_hl(
													partition.get_resource_info(&tblu_rrid).ok()?
												)
											);

											let blueprint = match game_version {
												GameVersion::H1 => convert_2016_blueprint_to_modern(
													&h2016_convert_binary_to_blueprint(&tblu_data).ok()?
												),

												GameVersion::H2 => h2_convert_binary_to_blueprint(&tblu_data).ok()?,

												GameVersion::H3 => h3_convert_binary_to_blueprint(&tblu_data).ok()?
											};

											let entity =
												convert_to_qn(&factory, &temp_meta, &blueprint, &tblu_meta, false)
													.ok()?;

											to_vec(&entity).ok()?
										} else {
											let temp_data = partition.read_resource(resource_id).ok()?;

											let factory = match game_version {
												GameVersion::H1 => convert_2016_factory_to_modern(
													&h2016_convert_binary_to_factory(&temp_data).ok()?
												),

												GameVersion::H2 => h2_convert_binary_to_factory(&temp_data).ok()?,

												GameVersion::H3 => h3_convert_binary_to_factory(&temp_data).ok()?
											};

											let (tblu_rrid, _) = &resource_info
												.references()
												.get(factory.blueprint_index_in_resource_header as usize)?;

											let tblu_data = partition.read_resource(tblu_rrid).ok()?;

											let blueprint = match game_version {
												GameVersion::H1 => convert_2016_blueprint_to_modern(
													&h2016_convert_binary_to_blueprint(&tblu_data).ok()?
												),

												GameVersion::H2 => h2_convert_binary_to_blueprint(&tblu_data).ok()?,

												GameVersion::H3 => h3_convert_binary_to_blueprint(&tblu_data).ok()?
											};

											let mut s = to_vec(&factory).ok()?;
											s.append(&mut to_vec(&blueprint).ok()?);

											s
										}
									};

									if let Some(s) = s {
										query.is_match(&s)
									} else {
										false
									}
								}

								"AIRG" | "ATMD" | "VIDB" | "UICB" | "CPPT" | "CRMD" | "DSWB" | "WSWB" | "GFXF"
								| "GIDX" | "WSGB" | "ECPB" | "ENUM" => {
									let s: Option<_> = try {
										convert_generic_str(
											&partition.read_resource(resource_id).ok()?,
											game_version,
											&filetype
										)
										.ok()?
									};

									if let Some(s) = s {
										query.is_match(s.as_ref())
									} else {
										false
									}
								}

								"JSON" | "REPO" => {
									let s: Option<_> = try { partition.read_resource(resource_id).ok()? };

									if let Some(s) = s {
										query.is_match(&s)
									} else {
										false
									}
								}

								"ORES" => {
									let s: Option<_> = try {
										let data = partition.read_resource(resource_id).ok()?;

										if resource_id.to_hex_string() == "0057C2C3941115CA" {
											to_vec(&parse_json_ores(&data).ok()?).ok()?
										} else {
											to_vec(&parse_hashes_ores(&data).ok()?).ok()?
										}
									};

									if let Some(s) = s {
										query.is_match(&s)
									} else {
										false
									}
								}

								"CLNG" => {
									let s: Option<_> = try {
										let (res_meta, res_data) = (
											convert_resource_info_to_rpkg_meta_no_hl(resource_info),
											partition.read_resource(resource_id).ok()?
										);

										let clng = {
											let mut iteration = 0;

											loop {
												if let Ok::<_, anyhow::Error>(x) = try {
													let langmap = get_language_map(game_version, iteration)
														.context("No more alternate language maps available")?;

													let clng = hmlanguages::clng::CLNG::new(
														match game_version {
															GameVersion::H1 => tonytools::Version::H2016,
															GameVersion::H2 => tonytools::Version::H2,
															GameVersion::H3 => tonytools::Version::H3
														},
														langmap.1.to_owned()
													)
													.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

													clng.convert(&res_data, to_string(&res_meta)?)
														.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
												} {
													break x;
												} else {
													iteration += 1;

													if get_language_map(game_version, iteration).is_none() {
														None?;
													}
												}
											}
										};

										let mut buf = Vec::new();
										let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
										let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

										clng.serialize(&mut ser).ok()?;

										buf
									};

									if let Some(s) = s {
										query.is_match(&s)
									} else {
										false
									}
								}

								"DITL" => {
									let s: Option<_> = try {
										let (res_meta, res_data) = (
											convert_resource_info_to_rpkg_meta_no_hl(resource_info),
											partition.read_resource(resource_id).ok()?
										);

										let ditl = hmlanguages::ditl::DITL::new(
											app_state.tonytools_hash_list.load().as_ref()?.deref().to_owned()
										)
										.ok()?;

										let mut buf = Vec::new();
										let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
										let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

										ditl.convert(&res_data, to_string(&res_meta).ok()?)
											.ok()?
											.serialize(&mut ser)
											.ok()?;

										buf
									};

									if let Some(s) = s {
										query.is_match(&s)
									} else {
										false
									}
								}

								"DLGE" => {
									let s: Option<_> = try {
										let (res_meta, res_data) = (
											convert_resource_info_to_rpkg_meta_no_hl(resource_info),
											partition.read_resource(resource_id).ok()?
										);

										let dlge = {
											let mut iteration = 0;

											loop {
												if let Ok::<_, anyhow::Error>(x) = try {
													let langmap = get_language_map(game_version, iteration)
														.context("No more alternate language maps available")?;

													let dlge = hmlanguages::dlge::DLGE::new(
														app_state
															.tonytools_hash_list
															.load()
															.as_ref()
															.context("No hash list available")?
															.deref()
															.to_owned(),
														match game_version {
															GameVersion::H1 => tonytools::Version::H2016,
															GameVersion::H2 => tonytools::Version::H2,
															GameVersion::H3 => tonytools::Version::H3
														},
														langmap.1.to_owned(),
														None,
														false
													)
													.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

													dlge.convert(&res_data, to_string(&res_meta)?)
														.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
												} {
													break x;
												} else {
													iteration += 1;

													if get_language_map(game_version, iteration).is_none() {
														None?;
													}
												}
											}
										};

										let mut buf = Vec::new();
										let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
										let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

										dlge.serialize(&mut ser).ok()?;

										buf
									};

									if let Some(s) = s {
										query.is_match(&s)
									} else {
										false
									}
								}

								"LOCR" => {
									let s: Option<_> = try {
										let (res_meta, res_data) = (
											convert_resource_info_to_rpkg_meta_no_hl(resource_info),
											partition.read_resource(resource_id).ok()?
										);

										let locr = {
											let mut iteration = 0;

											loop {
												if let Ok::<_, anyhow::Error>(x) = try {
													let langmap = get_language_map(game_version, iteration)
														.context("No more alternate language maps available")?;

													let locr = hmlanguages::locr::LOCR::new(
														app_state
															.tonytools_hash_list
															.load()
															.as_ref()
															.context("No hash list available")?
															.deref()
															.to_owned(),
														match game_version {
															GameVersion::H1 => tonytools::Version::H2016,
															GameVersion::H2 => tonytools::Version::H2,
															GameVersion::H3 => tonytools::Version::H3
														},
														langmap.1.to_owned(),
														langmap.0
													)
													.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

													locr.convert(&res_data, to_string(&res_meta)?)
														.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
												} {
													break x;
												} else {
													iteration += 1;

													if get_language_map(game_version, iteration).is_none() {
														None?;
													}
												}
											}
										};

										let mut buf = Vec::new();
										let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
										let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

										locr.serialize(&mut ser).ok()?;

										buf
									};

									if let Some(s) = s {
										query.is_match(&s)
									} else {
										false
									}
								}

								"RTLV" => {
									let s: Option<_> = try {
										let (res_meta, res_data) = (
											convert_resource_info_to_rpkg_meta_no_hl(resource_info),
											partition.read_resource(resource_id).ok()?
										);

										let rtlv = hmlanguages::rtlv::RTLV::new(
											match game_version {
												GameVersion::H1 => tonytools::Version::H2016,
												GameVersion::H2 => tonytools::Version::H2,
												GameVersion::H3 => tonytools::Version::H3
											},
											None
										)
										.map_err(|x| anyhow!("TonyTools error: {x:?}"))
										.ok()?
										.convert(&res_data, to_string(&res_meta).ok()?)
										.map_err(|x| anyhow!("TonyTools error: {x:?}"))
										.ok()?;

										let mut buf = Vec::new();
										let formatter = serde_json::ser::PrettyFormatter::with_indent(b"\t");
										let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);

										rtlv.serialize(&mut ser).ok()?;

										buf
									};

									if let Some(s) = s {
										query.is_match(&s)
									} else {
										false
									}
								}

								"LINE" => {
									let s: Option<_> = try {
										let (res_meta, res_data) = (
											convert_resource_info_to_rpkg_meta_no_hl(resource_info),
											partition.read_resource(resource_id).ok()?
										);

										let (locr_meta, locr_data) = extract_latest_resource_no_hl(
											game_files,
											&res_meta.hash_reference_data.first()?.hash
										)
										.ok()?;

										let locr = {
											let mut iteration = 0;

											loop {
												if let Ok::<_, anyhow::Error>(x) = try {
													let langmap = get_language_map(game_version, iteration)
														.context("No more alternate language maps available")?;

													let locr = hmlanguages::locr::LOCR::new(
														app_state
															.tonytools_hash_list
															.load()
															.as_ref()
															.context("No hash list available")?
															.deref()
															.to_owned(),
														match game_version {
															GameVersion::H1 => tonytools::Version::H2016,
															GameVersion::H2 => tonytools::Version::H2,
															GameVersion::H3 => tonytools::Version::H3
														},
														langmap.1.to_owned(),
														langmap.0
													)
													.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

													locr.convert(&locr_data, to_string(&locr_meta)?)
														.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
												} {
													break x;
												} else {
													iteration += 1;

													if get_language_map(game_version, iteration).is_none() {
														None?;
													}
												}
											}
										};

										let res_data: [u8; 5] = res_data.try_into().ok()?;

										let line_id = u32::from_le_bytes(res_data[0..4].try_into().unwrap());

										let line_hash = format!("{:0>8X}", line_id);

										let line_str = app_state
											.tonytools_hash_list
											.load()
											.as_ref()?
											.lines
											.get_by_left(&line_id)
											.cloned();

										if let Some(line_str) = line_str {
											locr.languages
												.into_iter()
												.filter_map(|(_, keys)| {
													if let serde_json::Value::String(val) = keys.get(&line_str)? {
														Some(val.to_owned())
													} else {
														None
													}
												})
												.collect::<Vec<_>>()
												.join("\n")
										} else {
											locr.languages
												.into_iter()
												.filter_map(|(_, keys)| {
													if let serde_json::Value::String(val) = keys.get(&line_hash)? {
														Some(val.to_owned())
													} else {
														None
													}
												})
												.collect::<Vec<_>>()
												.join("\n")
										}
									};

									if let Some(s) = s {
										query.is_match(s.as_bytes())
									} else {
										false
									}
								}

								_ => false
							}
						} else {
							false
						}
					})
					.map(|(x, _)| x.to_hex_string())
			);

			let percent = ((((progress * 1000) as f32) / (total_resources as f32)) * 100.0).round() as u8;
			if percent != last_percent {
				last_percent = percent;

				finish_task(app, progress_task)?;

				let secs_remaining = ((Instant::now() - start_time).as_secs_f32() / ((progress * 1000) as f32)
					* ((total_resources - (progress * 1000)) as f32)) as u64;

				progress_task = start_task(
					app,
					format!(
						"Searching game files for \"{}\": {}%, {}{} remaining",
						query,
						last_percent,
						hrtime::from_sec(secs_remaining),
						if secs_remaining <= 60 { "s" } else { "" }
					)
				)?;
			}
		}

		finish_task(app, progress_task)?;
		progress_task = start_task(app, format!("Preparing search results for \"{}\"", query))?;

		let results = matching_ids
			.into_iter()
			.map(|hash| {
				let filetype = hash_list
					.entries
					.get(&hash)
					.map(|x| x.resource_type.to_owned())
					.unwrap_or("".into());

				let path = hash_list
					.entries
					.get(&hash)
					.and_then(|x| x.path.as_ref().or(x.hint.as_ref()).cloned());

				(hash, filetype, path)
			})
			.collect();

		let id = Uuid::new_v4();

		app_state.editor_states.insert(
			id.to_owned(),
			EditorState {
				file: None,
				data: EditorData::ContentSearchResults { results }
			}
		);

		send_request(
			app,
			Request::Global(GlobalRequest::CreateTab {
				id,
				name: format!("Search results (\"{query}\")"),
				editor_type: EditorType::ContentSearchResults
			})
		)?;

		finish_task(app, progress_task)?;
	}
}
