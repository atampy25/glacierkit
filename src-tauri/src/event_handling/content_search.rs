use std::{io::Write, ops::Deref};

use anyhow::{Context, Result, anyhow};
use dashmap::DashMap;
use fn_error_context::context;
use glacier_commons::{
	game::GlacierGame,
	metadata::{ResourceMetadata, ResourceType, RuntimeID},
	resource_type,
	rpkg_tool::RpkgResourceMeta
};
use itertools::Itertools;
use quickentity_rs::entity::Entity;
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, ParallelBridge, ParallelIterator};
use regex_automata::dfa::Automaton;
use serde_json::{to_string, to_writer};
use tauri::{AppHandle, Manager};
use tonytools::{hmlanguages, locr::LocrJson};
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	HashMap, HashSet,
	bin1::deserialize_generic_writer,
	finish_task,
	languages::get_language_map,
	model::{AppState, EditorData, EditorState, EditorType, Request, TabRequest, TabRequestData},
	send_request, start_progress, start_task, task_progress
};

struct Pattern {
	automaton: regex_automata::dfa::dense::DFA<Vec<u32>>
}

impl Pattern {
	#[try_fn]
	pub fn new(pattern: &str) -> Result<Self> {
		Self {
			automaton: regex_automata::dfa::dense::DFA::new(pattern)?
		}
	}

	pub fn matcher<'a>(&'a self) -> Result<Matcher<'a>> {
		Ok(Matcher {
			automaton: &self.automaton,
			state: self.automaton.start_state_forward(&regex_automata::Input::new(""))?,
			matched: false
		})
	}
}

struct Matcher<'a> {
	automaton: &'a regex_automata::dfa::dense::DFA<Vec<u32>>,
	state: regex_automata::util::primitives::StateID,
	matched: bool
}

impl<'a> Matcher<'a> {
	pub fn matched(&self) -> bool {
		self.matched
	}
}

impl<'a> Write for Matcher<'a> {
	fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
		for &byte in buf {
			// SAFETY: The state is initialized to a valid start state by Pattern::matcher and only ever updated using the automaton's transition function.
			self.state = unsafe { self.automaton.next_state_unchecked(self.state, byte) };
			if self.automaton.is_match_state(self.state) {
				self.matched = true;
				return Err(std::io::Error::other("pattern matched"));
			} else if self.automaton.is_dead_state(self.state) {
				return Err(std::io::Error::other("pattern not matched"));
			}
		}

		Ok(buf.len())
	}

	fn flush(&mut self) -> std::io::Result<()> {
		Ok(())
	}
}

#[try_fn]
#[context("Couldn't perform content search")]
pub async fn start_content_search(
	app: &AppHandle,
	query: String,
	filetypes: Vec<ResourceType>,
	partitions_to_search: Vec<String>
) -> Result<()> {
	let app_state = app.state::<AppState>();

	let pattern = Pattern::new(&query).context("Invalid regex")?;

	let filetypes = filetypes.into_iter().collect::<HashSet<ResourceType>>();

	let matching_ids = tokio::task::spawn_blocking({
		let app = app.clone();
		let query = query.to_owned();
		move || {
			let app_state = app.state::<AppState>();

			if let Some(game) = app_state.game.load().as_ref() {
				let task = start_progress(&app, format!("Searching game files for \"{query}\""))?;

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

				let total_resources = resources.len();

				let mut locr = vec![];
				if filetypes.contains(&resource_type!("LOCR")) || filetypes.contains(&resource_type!("LINE")) {
					let mut iteration = 0;
					while let Some((symmetric, lang_map)) = get_language_map(game.version(), iteration) {
						locr.push(
							hmlanguages::locr::LOCR::new(
								app_state
									.tonytools_hash_list
									.load()
									.as_ref()
									.context("No hash list available")?
									.deref()
									.to_owned(),
								game.version().into(),
								lang_map,
								symmetric
							)
							.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
						);

						iteration += 1;
					}
				}

				let mut dlge = vec![];
				if filetypes.contains(&resource_type!("DLGE")) {
					let mut iteration = 0;
					while let Some((_, lang_map)) = get_language_map(game.version(), iteration) {
						dlge.push(
							hmlanguages::dlge::DLGE::new(
								app_state
									.tonytools_hash_list
									.load()
									.as_ref()
									.context("No hash list available")?
									.deref()
									.to_owned(),
								game.version().into(),
								lang_map,
								None,
								false
							)
							.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
						);

						iteration += 1;
					}
				}

				let ditl = hmlanguages::ditl::DITL::new(
					app_state
						.tonytools_hash_list
						.load()
						.as_ref()
						.context("No hash list available")?
						.deref()
						.to_owned()
				)
				.map_err(|x| anyhow!("TonyTools error: {x:?}"))?;

				let mut clng = vec![];
				if filetypes.contains(&resource_type!("CLNG")) {
					let mut iteration = 0;
					while let Some((_, lang_map)) = get_language_map(game.version(), iteration) {
						clng.push(
							hmlanguages::clng::CLNG::new(game.version().into(), lang_map)
								.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
						);

						iteration += 1;
					}
				}

				let mut rtlv = vec![];
				if filetypes.contains(&resource_type!("RTLV")) {
					let mut iteration = 0;
					while let Some((_, lang_map)) = get_language_map(game.version(), iteration) {
						rtlv.push(
							hmlanguages::rtlv::RTLV::new(game.version().into(), lang_map)
								.map_err(|x| anyhow!("TonyTools error: {x:?}"))?
						);

						iteration += 1;
					}
				}

				let cached_locrs: DashMap<RuntimeID, LocrJson> = DashMap::default();

				let search_rest = filetypes.contains(&resource_type!("REST"));

				let matching_ids = resources
					.into_iter()
					.enumerate()
					.par_bridge()
					.filter_map(|(progress, (resource_id, (partition, resource_info)))| {
						if progress.is_multiple_of(1000) {
							let _ = task_progress(&app, task, (progress as f32) / (total_resources as f32));
						}

						let filetype = resource_info.data_type().try_into().ok()?;

						if filetypes.contains(&filetype) || search_rest {
							let mut matcher = pattern.matcher().ok()?;

							match filetype.as_ref() {
								"TEMP" => {
									let (temp_data, temp_meta) = (
										partition.read_resource(resource_id).ok()?,
										ResourceMetadata::try_from(resource_info).ok()?
									);

									macro_rules! impl_game {
										($ty:ty) => {{
											let factory = glacier_bin1::deserialize::<$ty>(&temp_data).ok()?;

											let blueprint_hash = temp_meta
												.references
												.get(factory.blueprint_index_in_resource_header as usize)?
												.resource;

											let tblu_rrid = game.to_rrid(blueprint_hash);

											let (tblu_data, tblu_meta) = (
												partition.read_resource(&tblu_rrid).ok()?,
												ResourceMetadata::try_from(
													partition.get_resource_info(&tblu_rrid).ok()?
												)
												.ok()?
											);

											let blueprint = glacier_bin1::deserialize(&tblu_data).ok()?;

											Entity::from_game(&factory, &temp_meta, &blueprint, &tblu_meta, false)
												.ok()?
										}};
									}

									let mut entity = match game.version() {
										GlacierGame::H1 => impl_game!(glacier_bin1::game::h1::STemplateEntity),
										GlacierGame::H2 => impl_game!(glacier_bin1::game::h2::STemplateEntityFactory),
										GlacierGame::H3 => impl_game!(glacier_bin1::game::h3::STemplateEntityFactory),
										GlacierGame::FL => impl_game!(glacier_bin1::game::fl::STemplateEntityFactory)
									};

									entity.extra_factory_references.clear();
									entity.extra_blueprint_references.clear();

									let _ = to_writer(&mut matcher, &entity);
								}

								"AIBB" | "AIRG" | "ASVA" | "ATMD" | "BMSK" | "CBLU" | "CPPT" | "CRMD" | "ECPB"
								| "ENUM" | "GFXF" | "GIDX" | "UICB" | "VIDB" | "WSGB" | "WSWB" | "DSWB" | "CLRP"
								| "GFXA" | "KWOR" | "TDAT" | "TDPK" | "WEMD" => {
									deserialize_generic_writer(
										game.version(),
										if filetype == "DSWB" {
											"WSWB".try_into().ok()?
										} else {
											filetype
										},
										&mut matcher,
										&partition.read_resource(resource_id).ok()?
									)
									.ok()?;
								}

								"JSON" | "REPO" => {
									let _ = matcher.write_all(&partition.read_resource(resource_id).ok()?);
								}

								"CLNG" => {
									let (res_meta, res_data) = (
										RpkgResourceMeta::try_from(resource_info).ok()?,
										partition.read_resource(resource_id).ok()?
									);

									let clng = clng
										.iter()
										.find_map(|clng| clng.convert(&res_data, to_string(&res_meta).ok()?).ok())?;

									let _ = to_writer(&mut matcher, &clng);
								}

								"DITL" => {
									let (res_meta, res_data) = (
										RpkgResourceMeta::try_from(resource_info).ok()?,
										partition.read_resource(resource_id).ok()?
									);

									let _ = to_writer(
										&mut matcher,
										&ditl.convert(&res_data, to_string(&res_meta).ok()?).ok()?
									);
								}

								"DLGE" => {
									let (res_meta, res_data) = (
										RpkgResourceMeta::try_from(resource_info).ok()?,
										partition.read_resource(resource_id).ok()?
									);

									let dlge = dlge
										.iter()
										.find_map(|dlge| dlge.convert(&res_data, to_string(&res_meta).ok()?).ok())?;

									let _ = to_writer(&mut matcher, &dlge);
								}

								"LOCR" => {
									let locr = cached_locrs
										.entry(resource_id.try_into().ok()?)
										.or_try_insert_with(|| {
											let (res_meta, res_data) = (
												RpkgResourceMeta::try_from(resource_info)?,
												partition.read_resource(resource_id)?
											);

											let locr = locr
												.iter()
												.find_map(|locr| {
													locr.convert(&res_data, to_string(&res_meta).ok()?).ok()
												})
												.context("Failed to convert LOCR resource")?;

											anyhow::Ok(locr)
										})
										.ok()?
										.downgrade();

									let _ = to_writer(&mut matcher, &*locr);
								}

								"RTLV" => {
									let (res_meta, res_data) = (
										RpkgResourceMeta::try_from(resource_info).ok()?,
										partition.read_resource(resource_id).ok()?
									);

									let rtlv = rtlv
										.iter()
										.find_map(|rtlv| rtlv.convert(&res_data, to_string(&res_meta).ok()?).ok())?;

									let _ = to_writer(&mut matcher, &rtlv);
								}

								"LINE" => {
									let (res_meta, res_data) = (
										ResourceMetadata::try_from(resource_info).ok()?,
										partition.read_resource(resource_id).ok()?
									);

									let locr_id = res_meta.references.first()?.resource;

									let locr = cached_locrs
										.entry(locr_id)
										.or_try_insert_with(|| {
											let (locr_meta, locr_data) = game.extract_latest_resource(locr_id)?;

											let locr_meta = RpkgResourceMeta::from_resource_metadata(locr_meta, false);

											let locr = locr
												.iter()
												.find_map(|locr| {
													locr.convert(&locr_data, to_string(&locr_meta).ok()?).ok()
												})
												.context("Failed to convert LOCR resource")?;

											anyhow::Ok(locr)
										})
										.ok()?
										.downgrade();

									let line_id = u32::from_le_bytes(res_data[0..4].try_into().unwrap());

									if let Some(line_str) = app_state
										.tonytools_hash_list
										.load()
										.as_ref()?
										.lines
										.get_by_left(&line_id)
									{
										let _ = matcher.write_all(line_str.as_bytes());
										let _ = matcher.write_all(b"\n");
										for line in locr.languages.iter().filter_map(|(_, keys)| {
											if let serde_json::Value::String(val) = keys.get(line_str)? {
												Some(val)
											} else {
												None
											}
										}) {
											let _ = matcher.write_all(line.as_bytes());
											let _ = matcher.write_all(b"\n");
										}
									} else {
										let line_hash = format!("{:0>8X}", line_id);
										let _ = matcher.write_all(line_hash.as_bytes());
										let _ = matcher.write_all(b"\n");
										for line in locr.languages.iter().filter_map(|(_, keys)| {
											if let serde_json::Value::String(val) = keys.get(&line_hash)? {
												Some(val)
											} else {
												None
											}
										}) {
											let _ = matcher.write_all(line.as_bytes());
											let _ = matcher.write_all(b"\n");
										}
									}
								}

								_ if search_rest => {
									let _ = matcher.write_all(&partition.read_resource(resource_id).ok()?);
								}

								_ => {}
							}

							matcher.matched().then(|| RuntimeID::try_from(resource_id).unwrap())
						} else {
							None
						}
					})
					.collect::<Vec<_>>();

				finish_task(&app, task)?;

				Ok(matching_ids)
			} else {
				anyhow::bail!("No game loaded");
			}
		}
	})
	.await??;

	if let Some(game) = app_state.game.load().as_ref() {
		let task = start_task(app, format!("Preparing search results for \"{}\"", query))?;

		let results = matching_ids
			.into_iter()
			.map(|id| {
				let filetype = game.resource_type(id).map(|x| x.into()).unwrap_or("".into());
				let path_or_hint = id.get_path_or_hint().map(|x| x.into());

				(id.to_hash(), filetype, path_or_hint)
			})
			.collect();

		let id = Uuid::new_v4();

		app_state
			.editor_states
			.insert(
				id.to_owned(),
				EditorState {
					file: None,
					data: EditorData::ContentSearchResults {
						query: query.to_owned(),
						results
					},
					..Default::default()
				}
			)
			.await;

		send_request(
			app,
			Request::Tab(TabRequest {
				tab: id,
				data: TabRequestData::Create {
					name: format!("Search results for \"{query}\""),
					editor_type: EditorType::ContentSearchResults
				}
			})
		)?;

		finish_task(app, task)?;
	}
}
