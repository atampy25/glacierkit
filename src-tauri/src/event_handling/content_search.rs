use std::{
	collections::{HashMap, HashSet},
	io::Write,
	ops::Deref
};

use anyhow::{Context, Result, anyhow};
use fn_error_context::context;
use hitman_commons::{
	game::GameVersion,
	metadata::{ResourceMetadata, RuntimeID},
	rpkg_tool::RpkgResourceMeta
};
use itertools::Itertools;
use quickentity_rs::convert_to_qn;
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, ParallelExtend, ParallelIterator};
use rpkg_rs::resource::runtime_resource_id::RuntimeResourceID;
use serde_json::{to_string, to_writer};
use tauri::{AppHandle, Manager};
use tonytools::hmlanguages;
use tryvial::{try_block, try_fn};
use uuid::Uuid;

use crate::{
	bin1::{deserialize_generic_writer, deserialize_modern_blueprint, deserialize_modern_factory},
	finish_task,
	languages::get_language_map,
	model::{AppState, EditorData, EditorState, EditorType, Request, TabRequest, TabRequestData},
	send_request, start_progress, start_task, task_progress
};

#[try_fn]
#[context("Couldn't perform content search")]
pub async fn start_content_search(
	app: &AppHandle,
	query: String,
	filetypes: Vec<String>,
	use_qn_format: bool,
	partitions_to_search: Vec<String>
) -> Result<()> {
	let app_state = app.state::<AppState>();

	let pattern = matchers::Pattern::new(&format!("{query}(?s:.)*")).context("Invalid regex")?;

	let filetypes = filetypes.into_iter().collect::<HashSet<String>>();

	if let Some(game) = app_state.game.load().as_ref() {
		let resources = game
			.partition_manager()
			.partitions
			.iter()
			.filter(|x| partitions_to_search.contains(&x.partition_info().id.to_string()))
			.collect_vec()
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

		let mut task = start_progress(app, format!("Searching game files for \"{query}\""))?;

		for (progress, chunk) in resources.into_iter().chunks(1000).into_iter().enumerate() {
			matching_ids.par_extend(
				chunk
					.collect_vec()
					.into_par_iter()
					.filter(|(resource_id, (partition, resource_info))| {
						let filetype = resource_info.data_type();

						if filetypes.contains(&filetype) {
							let mut matcher = pattern.matcher();

							match filetype.as_ref() {
								"TEMP" => {
									let _: Option<_> = try_block! {
										if use_qn_format {
											let (temp_data, temp_meta) = (
												partition.read_resource(resource_id).ok()?,
												ResourceMetadata::try_from(*resource_info).ok()?
											);

											let factory = deserialize_modern_factory(game.version(), &temp_data).ok()?;

											let blueprint_hash = &temp_meta
												.references
												.get(factory.blueprint_index_in_resource_header as usize)?
												.resource;

											let tblu_rrid = RuntimeResourceID::from(blueprint_hash);

											let (tblu_data, tblu_meta) = (
												partition.read_resource(&tblu_rrid).ok()?,
												ResourceMetadata::try_from(partition.get_resource_info(&tblu_rrid).ok()?).ok()?
											);

											let blueprint = deserialize_modern_blueprint(game.version(), &tblu_data).ok()?;

											let entity =
												convert_to_qn(&factory, &temp_meta, &blueprint, &tblu_meta, false)
													.ok()?;

											to_writer(&mut matcher, &entity).ok()?;
										} else {
											let temp_data = partition.read_resource(resource_id).ok()?;

											match game.version() {
												GameVersion::H1 => to_writer(
														&mut matcher,
														&hitman_bin1::deserialize::<hitman_bin1::game::h1::STemplateEntity>(&temp_data).ok()?
													).ok()?,

												GameVersion::H2 => to_writer(
														&mut matcher,
														&hitman_bin1::deserialize::<hitman_bin1::game::h2::STemplateEntityFactory>(&temp_data).ok()?
													).ok()?,

												GameVersion::H3 => to_writer(
														&mut matcher,
														&hitman_bin1::deserialize::<hitman_bin1::game::h3::STemplateEntityFactory>(&temp_data).ok()?
													).ok()?,
											}
										}
									};
								}

								"TBLU" if !use_qn_format => {
									let _: Option<_> = try_block! {
										let tblu_data = partition.read_resource(resource_id).ok()?;

										match game.version() {
											GameVersion::H1 => to_writer(
													&mut matcher,
													&hitman_bin1::deserialize::<hitman_bin1::game::h1::STemplateEntityBlueprint>(&tblu_data).ok()?
												).ok()?,

											GameVersion::H2 => to_writer(
													&mut matcher,
													&hitman_bin1::deserialize::<hitman_bin1::game::h2::STemplateEntityBlueprint>(&tblu_data).ok()?
												).ok()?,

											GameVersion::H3 => to_writer(
													&mut matcher,
													&hitman_bin1::deserialize::<hitman_bin1::game::h3::STemplateEntityBlueprint>(&tblu_data).ok()?
												).ok()?,
										}
									};
								}

								"AIBB" | "AIRG" | "ASVA" | "ATMD" | "BMSK" | "CBLU" | "CPPT" | "CRMD" | "ENUM"
								| "GFXF" | "GIDX" | "UICB" | "VIDB" | "WSGB" | "WSWB" | "ECPB" | "ORES" | "DSWB" => {
									let _: Option<_> = try_block! {
										deserialize_generic_writer(
											game.version(),
											if filetype == "DSWB" {
												"WSWB".try_into().ok()?
											} else {
												filetype.try_into().ok()?
											},
											&mut matcher,
											&partition.read_resource(resource_id).ok()?
										)
										.ok()?;
									};
								}

								"JSON" | "REPO" => {
									let _: Option<_> = try_block! { matcher.write_all(&partition.read_resource(resource_id).ok()?).ok()?; };
								}

								"CLNG" => {
									let _: Option<_> = try_block! {
										let (res_meta, res_data) = (
											RpkgResourceMeta::try_from(*resource_info).ok()?,
											partition.read_resource(resource_id).ok()?
										);

										let clng = {
											let mut iteration = 0;

											loop {
												if let Ok::<_, anyhow::Error>(x) = try_block! {
													let langmap = get_language_map(game.version(), iteration)
														.context("No more alternate language maps available")?;

													let clng = hmlanguages::clng::CLNG::new(
														game.version().into(),
														langmap.1.to_owned()
													)
													.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

													clng.convert(&res_data, to_string(&res_meta)?)
														.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
												} {
													break x;
												} else {
													iteration += 1;

													if get_language_map(game.version(), iteration).is_none() {
														None?;
													}
												}
											}
										};

										to_writer(&mut matcher, &clng).ok()?;
									};
								}

								"DITL" => {
									let _: Option<_> = try_block! {
										let (res_meta, res_data) = (
											RpkgResourceMeta::try_from(*resource_info).ok()?,
											partition.read_resource(resource_id).ok()?
										);

										let ditl = hmlanguages::ditl::DITL::new(
											app_state.tonytools_hash_list.load().as_ref()?.deref().to_owned()
										)
										.ok()?;

										to_writer(
											&mut matcher,
											&ditl.convert(&res_data, to_string(&res_meta).ok()?).ok()?
										).ok()?;
									};
								}

								"DLGE" => {
									let _: Option<_> = try_block! {
										let (res_meta, res_data) = (
											RpkgResourceMeta::try_from(*resource_info).ok()?,
											partition.read_resource(resource_id).ok()?
										);

										let dlge = {
											let mut iteration = 0;

											loop {
												if let Ok::<_, anyhow::Error>(x) = try_block! {
													let langmap = get_language_map(game.version(), iteration)
														.context("No more alternate language maps available")?;

													let dlge = hmlanguages::dlge::DLGE::new(
														app_state
															.tonytools_hash_list
															.load()
															.as_ref()
															.context("No hash list available")?
															.deref()
															.to_owned(),
														game.version().into(),
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

													if get_language_map(game.version(), iteration).is_none() {
														None?;
													}
												}
											}
										};

										to_writer(&mut matcher, &dlge).ok()?;
									};
								}

								"LOCR" => {
									let _: Option<_> = try_block! {
										let (res_meta, res_data) = (
											RpkgResourceMeta::try_from(*resource_info).ok()?,
											partition.read_resource(resource_id).ok()?
										);

										let locr = {
											let mut iteration = 0;

											loop {
												if let Ok::<_, anyhow::Error>(x) = try_block! {
													let langmap = get_language_map(game.version(), iteration)
														.context("No more alternate language maps available")?;

													let locr = hmlanguages::locr::LOCR::new(
														app_state
															.tonytools_hash_list
															.load()
															.as_ref()
															.context("No hash list available")?
															.deref()
															.to_owned(),
														game.version().into(),
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

													if get_language_map(game.version(), iteration).is_none() {
														None?;
													}
												}
											}
										};

										to_writer(&mut matcher, &locr).ok()?;
									};
								}

								"RTLV" => {
									let _: Option<_> = try_block! {
										let (res_meta, res_data) = (
											RpkgResourceMeta::try_from(*resource_info).ok()?,
											partition.read_resource(resource_id).ok()?
										);

										let rtlv = hmlanguages::rtlv::RTLV::new(game.version().into(), None)
											.map_err(|x| anyhow!("TonyTools error: {x:?}"))
											.ok()?
											.convert(&res_data, to_string(&res_meta).ok()?)
											.map_err(|x| anyhow!("TonyTools error: {x:?}"))
											.ok()?;

										to_writer(&mut matcher, &rtlv).ok()?;
									};
								}

								"LINE" => {
									let _: Option<_> = try_block! {
										let (res_meta, res_data) = (
											RpkgResourceMeta::try_from(*resource_info).ok()?,
											partition.read_resource(resource_id).ok()?
										);

										let (locr_meta, locr_data) = game
											.extract_latest_resource(res_meta.hash_reference_data.first()?.hash)
											.ok()?;

										let locr = {
											let mut iteration = 0;

											loop {
												if let Ok::<_, anyhow::Error>(x) = try_block! {
													let langmap = get_language_map(game.version(), iteration)
														.context("No more alternate language maps available")?;

													let locr = hmlanguages::locr::LOCR::new(
														app_state
															.tonytools_hash_list
															.load()
															.as_ref()
															.context("No hash list available")?
															.deref()
															.to_owned(),
														game.version().into(),
														langmap.1.to_owned(),
														langmap.0
													)
													.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

													locr.convert(
														&locr_data,
														to_string(&RpkgResourceMeta::from_resource_metadata(
															locr_meta.to_owned(),
															false
														))?
													)
													.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
												} {
													break x;
												} else {
													iteration += 1;

													if get_language_map(game.version(), iteration).is_none() {
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
											matcher.write_all(
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
													.as_bytes()
											).ok()?;
										} else {
											matcher.write_all(
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
													.as_bytes()
											).ok()?;
										}
									};
								}

								_ => {}
							}

							matcher.is_matched()
						} else {
							false
						}
					})
					.map(|(x, _)| RuntimeID::try_from(x).unwrap())
			);

			task_progress(app, task, ((progress * 1000) as f32) / (total_resources as f32))?;
		}

		finish_task(app, task)?;
		task = start_task(app, format!("Preparing search results for \"{}\"", query))?;

		let results = matching_ids
			.into_iter()
			.map(|id| {
				let info = id.get_info();
				let filetype = info.as_ref().map(|x| x.resource_type.into()).unwrap_or("".into());
				let path = info.and_then(|x| x.path.or(x.hint)).map(|x| x.into());

				(id.to_hash(), filetype, path)
			})
			.collect();

		let id = Uuid::new_v4();

		app_state
			.editor_states
			.insert(
				id.to_owned(),
				EditorState {
					file: None,
					data: EditorData::ContentSearchResults { results },
					..Default::default()
				}
			)
			.await;

		send_request(
			app,
			Request::Tab(TabRequest {
				tab: id,
				data: TabRequestData::Create {
					name: format!("Search results (\"{query}\")"),
					editor_type: EditorType::ContentSearchResults
				}
			})
		)?;

		finish_task(app, task)?;
	}
}
