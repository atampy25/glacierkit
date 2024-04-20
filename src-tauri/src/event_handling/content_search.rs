use std::{fs, time::Instant};

use anyhow::{bail, Context, Result};
use arc_swap::ArcSwap;
use fn_error_context::context;
use hashbrown::{HashMap, HashSet};
use itertools::Itertools;
use quickentity_rs::{convert_2016_blueprint_to_modern, convert_2016_factory_to_modern, convert_to_qn};
use rayon::{
	iter::{IndexedParallelIterator, IntoParallelIterator, ParallelExtend, ParallelIterator},
	ThreadPoolBuilder
};
use regex::bytes::Regex;
use rpkg_rs::runtime::resource::runtime_resource_id::RuntimeResourceID;
use serde_json::{to_string, to_vec};
use tauri::{api::path::app_data_dir, AppHandle, Manager};
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	finish_task,
	game_detection::GameVersion,
	model::{AppSettings, AppState, EditorData, EditorState, EditorType, GlobalRequest, Request},
	ores::{parse_hashes_ores, parse_json_ores},
	resourcelib::{
		convert_generic_str, h2016_convert_binary_to_blueprint, h2016_convert_binary_to_factory,
		h2_convert_binary_to_blueprint, h2_convert_binary_to_factory, h3_convert_binary_to_blueprint,
		h3_convert_binary_to_factory
	},
	rpkg::convert_resource_info_to_rpkg_meta_no_hl,
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
			.get_all_partitions()
			.into_par_iter()
			.rev()
			.flat_map(|partition| {
				partition
					.get_latest_resources()
					.into_par_iter()
					.map(move |(resource, _)| (resource.get_rrid(), (partition, resource)))
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
						let filetype = resource_info.get_type();

						if filetypes.contains(&filetype) {
							match filetype.as_ref() {
								"TEMP" => {
									let s: Option<Vec<u8>> = try {
										if use_qn_format {
											let (temp_data, temp_meta) = (
												partition.get_resource(resource_id).ok()?,
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
												partition.get_resource(&tblu_rrid).ok()?,
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
												convert_to_qn(&factory, &temp_meta, &blueprint, &tblu_meta, true)
													.ok()?;

											to_vec(&entity).ok()?
										} else {
											let temp_data = partition.get_resource(resource_id).ok()?;

											let factory = match game_version {
												GameVersion::H1 => convert_2016_factory_to_modern(
													&h2016_convert_binary_to_factory(&temp_data).ok()?
												),

												GameVersion::H2 => h2_convert_binary_to_factory(&temp_data).ok()?,

												GameVersion::H3 => h3_convert_binary_to_factory(&temp_data).ok()?
											};

											let (tblu_rrid, _) = &resource_info
												.get_reference(factory.blueprint_index_in_resource_header as usize)?;

											let tblu_data = partition.get_resource(tblu_rrid).ok()?;

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

								"AIRG" | "RTLV" | "ATMD" | "VIDB" | "UICB" | "CPPT" | "CRMD" | "DSWB" | "WSWB"
								| "GFXF" | "GIDX" | "WSGB" | "ECPB" | "ENUM" => {
									let s: Option<_> = try {
										convert_generic_str(
											&partition.get_resource(resource_id).ok()?,
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
									let s: Option<_> = try { partition.get_resource(resource_id).ok()? };

									if let Some(s) = s {
										query.is_match(&s)
									} else {
										false
									}
								}

								"ORES" => {
									let s: Option<_> = try {
										let data = partition.get_resource(resource_id).ok()?;

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
				progress_task = start_task(
					app,
					format!(
						"Searching game files for \"{}\": {}%, {} remaining",
						query,
						last_percent,
						hrtime::from_sec(
							((Instant::now() - start_time).as_secs_f32() / ((progress * 1000) as f32)
								* ((total_resources - (progress * 1000)) as f32)) as u64
						)
					)
				)?;
			}
		}

		finish_task(app, progress_task)?;

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
	}
}
