use std::{
	collections::{HashMap, HashSet},
	pin::pin,
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering}
	},
	time::Duration
};

use anyhow::{Context, Error, Result, anyhow, bail};
use debounced::debounced;
use fn_error_context::context;
use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use glacier_bin1::game::h3::{STemplateEntityBlueprint, STemplateEntityFactory, ZVariant};
use glacier_commons::{
	metadata::{ResourceMetadata, RuntimeID},
	resource_type
};
use indexmap::IndexMap;
use log::{error, warn};
use quickentity_rs::{
	entity::{Entity, EntityID, PinConnection, Ref, SubEntity},
	game::{FromQuickEntity, ToQuickEntity},
	variant::{Transform, Variant, Vec3}
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, from_value, json};
use specta::Type;
use tauri::{AppHandle, Manager, async_runtime::spawn};
use tokio::{
	net::TcpStream,
	sync::{
		RwLock,
		broadcast::{self, error::RecvError}
	},
	time::timeout
};
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};
use tryvial::{try_block, try_fn};

use crate::{
	Notification, NotificationKind,
	game::GameFiles,
	general::EMPTY_ID,
	handle_event,
	model::{
		AppState, EditorConnectionEvent, EditorData, EditorRequest, EditorRequestData, EntityEditorRequest,
		EntityMonacoRequest, EntityTreeRequest, Event, GlobalRequest, Hash, Request
	},
	send_notification, send_request
};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "source")]
#[serde(rename_all = "camelCase")]
pub enum EntitySelector {
	Game {
		id: String,
		tblu: String,

		#[serde(default, rename = "exposedEntity", skip_serializing_if = "Option::is_none")]
		exposed_entity: Option<String>
	},
	Editor {
		id: String
	}
}

impl EntitySelector {
	fn game(id: EntityID, tblu: impl Into<String>) -> Self {
		EntitySelector::Game {
			id: id.to_string(),
			tblu: tblu.into(),
			exposed_entity: None
		}
	}
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum PropertyID {
	Unknown(u32),
	Known(String)
}

impl PropertyID {
	fn from_qn_name(name: &str) -> Self {
		match name.parse::<u32>() {
			Ok(id) => PropertyID::Unknown(id),
			Err(_) => PropertyID::Known(name.to_owned())
		}
	}
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct Rotation {
	yaw: f64,
	pitch: f64,
	roll: f64
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct SDKTransform {
	position: Vec3,
	rotation: Rotation,
	scale: Vec3
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum SDKEditorRequest {
	Hello {
		identifier: String
	},

	SelectEntity {
		entity: EntitySelector
	},

	SetEntityTransform {
		entity: EntitySelector,
		transform: SDKTransform,
		relative: bool
	},

	CreateSubEntities {
		entities: Vec<SubEntityToCreate>
	},

	DestroyEntity {
		entity: EntitySelector
	},

	SetEntityName {
		entity: EntitySelector,
		name: String
	},

	SetEntityProperty {
		entity: EntitySelector,
		property: PropertyID,
		value: Value
	},

	SignalEntityPin {
		entity: EntitySelector,
		pin: PropertyID,
		output: bool
	},

	ListEntities {
		editorOnly: bool,

		#[serde(skip_serializing_if = "Option::is_none")]
		msgId: Option<i64>
	},

	GetEntityDetails {
		entity: EntitySelector,

		#[serde(skip_serializing_if = "Option::is_none")]
		msgId: Option<i64>
	},

	GetHitmanEntity {
		#[serde(skip_serializing_if = "Option::is_none")]
		msgId: Option<i64>
	},

	GetCameraEntity {
		#[serde(skip_serializing_if = "Option::is_none")]
		msgId: Option<i64>
	},

	RebuildEntityTree
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SubEntityToCreate {
	id: String,
	tblu: String,
	parent: EntitySelector,
	name: String,
	factory: String,
	blueprint: String,
	properties: IndexMap<String, SubEntityPropertyValue>,

	/// Only from external entities, internal pins are on the source
	inbound_pins: Vec<PinConnectionToCreate>,
	outbound_pins: Vec<PinConnectionToCreate>
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PinConnectionToCreate {
	from: EntitySelector,
	from_pin: PropertyID,
	to: EntitySelector,
	to_pin: PropertyID,

	#[serde(skip_serializing_if = "Option::is_none")]
	value: Option<PropertyValue>
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SubEntityPropertyValue {
	#[serde(rename = "type")]
	property_type: String,
	data: Value,

	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	post_init: bool
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PropertyValue {
	#[serde(rename = "type")]
	pub property_type: String,
	pub data: Value
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "source")]
#[serde(rename_all = "camelCase")]
pub enum EntityBaseDetails {
	Game {
		id: String,
		tblu: String // name: Option<String>,

		             // #[serde(rename = "type")]
		             // ty: String
	},
	Editor {
		id: String // name: Option<String>,

		           // #[serde(rename = "type")]
		           // ty: String
	}
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "source")]
#[serde(rename_all = "camelCase")]
pub enum EntityDetails {
	Game {
		id: String,
		tblu: String,
		// name: Option<String>,

		// #[serde(rename = "type")]
		// ty: String,
		// parent: Option<EntitySelector>,
		transform: Option<Value>,
		// relativeTransform: Option<Transform>,
		properties: IndexMap<String, PropertyValue> // interfaces: Vec<String>
	},
	Editor {
		id: String,
		// name: Option<String>,

		// #[serde(rename = "type")]
		// ty: String,
		// parent: Option<EntitySelector>,
		transform: Option<Value>,
		// relativeTransform: Option<Transform>,
		properties: IndexMap<String, PropertyValue> // interfaces: Vec<String>
	}
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum SDKEditorEvent {
	Welcome,

	Error {
		message: String,

		#[serde(skip_serializing_if = "Option::is_none")]
		msgId: Option<i64>
	},

	EntitySelected {
		entity: EntityDetails
	},

	EntityDeselected,

	EntityTransformUpdated {
		entity: EntityDetails
	},

	EntityNameUpdated {
		entity: EntityDetails
	},

	EntitySpawned {
		entity: EntityDetails
	},

	EntityDestroying {
		entityId: String
	},

	EntityPropertyChanged {
		entity: EntityDetails,
		property: PropertyID,
		value: PropertyValue
	},

	SceneLoading {
		scene: String,
		bricks: Vec<String>
	},

	SceneClearing {
		forReload: bool
	},

	EntityList {
		entities: Vec<EntityBaseDetails>,
		msgId: Option<i64>
	},

	EntityDetails {
		entity: EntityDetails,

		#[serde(skip_serializing_if = "Option::is_none")]
		msgId: Option<i64>
	},

	HitmanEntity {
		entity: EntityDetails,

		#[serde(skip_serializing_if = "Option::is_none")]
		msgId: Option<i64>
	},

	CameraEntity {
		entity: EntityDetails,

		#[serde(skip_serializing_if = "Option::is_none")]
		msgId: Option<i64>
	},

	EntityTreeRebuilt
}

pub struct EditorConnection {
	sender: Arc<RwLock<Option<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>>,
	events: broadcast::Sender<SDKEditorEvent>,
	debounced_events: tokio::sync::mpsc::Sender<SDKEditorEvent>,
	entity_tree_loaded: Arc<AtomicBool>,
	app: AppHandle
}

const UNSETTABLE_PROPERTY_ERRORS: [&str; 2] =
	["Resource properties can't be set", "Unable to convert JSON to game struct"];

fn report_error(app: &AppHandle, error: Error) {
	error!("{error:?}");

	let _ = send_request(
		app,
		Request::Global(GlobalRequest::ErrorReport {
			error: format!("{error:?}")
		})
	);
}

fn parse_entity_id(id: &str) -> Result<EntityID> {
	id.parse().with_context(|| format!("Couldn't parse entity ID {id:?}"))
}

fn parse_tblu(tblu: String) -> Result<Hash> {
	Ok(Hash(
		tblu.parse()
			.with_context(|| format!("Couldn't parse TBLU hash {tblu:?}"))?
	))
}

fn standalone_factory() -> STemplateEntityFactory {
	STemplateEntityFactory {
		blueprint_index_in_resource_header: 0,
		root_entity_index: 0,
		sub_type: 0,
		sub_entities: vec![],
		external_scene_type_indices_in_resource_header: vec![],
		property_overrides: vec![]
	}
}

fn standalone_blueprint() -> STemplateEntityBlueprint {
	STemplateEntityBlueprint {
		sub_type: 0,
		root_entity_index: 0,
		sub_entities: vec![],
		external_scene_type_indices_in_resource_header: vec![],
		pin_connections: vec![],
		input_pin_forwardings: vec![],
		output_pin_forwardings: vec![],
		override_deletes: vec![],
		pin_connection_overrides: vec![],
		pin_connection_override_deletes: vec![]
	}
}

fn temp_metadata() -> ResourceMetadata {
	ResourceMetadata {
		id: EMPTY_ID,
		resource_type: resource_type!("TEMP"),
		compressed: true,
		scrambled: true,
		references: vec![]
	}
}

#[try_fn]
fn selection_event(id: &str, tblu: String) -> Result<Event> {
	Event::EditorConnection(EditorConnectionEvent::EntitySelected {
		id: parse_entity_id(id)?,
		tblu: parse_tblu(tblu)?
	})
}

/// The SDK reports entity references as runtime typed selectors, rather than as RT JSON.
fn sdk_entity_ref_property(property_type: &str, data: &Value, tblu: &str) -> Option<Result<Variant>> {
	let is_entity_ref = |ty: &str| ty == "ZEntityRef" || (ty.starts_with("TEntityRef<") && ty.ends_with('>'));

	if is_entity_ref(property_type) {
		return Some(sdk_entity_ref(data, tblu).map(Variant::Ref));
	}

	let element_type = property_type.strip_prefix("TArray<")?.strip_suffix('>')?;

	if !is_entity_ref(element_type) {
		return None;
	}

	Some(try_block! {
		let Value::Array(elements) = data else {
			bail!("Expected an array of entity references")
		};

		Variant::Array(
			"SEntityTemplateReference".into(),
			elements
				.iter()
				.map(|x| sdk_entity_ref(x, tblu).map(Variant::Ref))
				.collect::<Result<_>>()?
		)
	})
}

#[try_fn]
fn sdk_entity_ref(selector: &Value, tblu: &str) -> Result<Option<Ref>> {
	if selector.is_null() {
		return Ok(None);
	}

	match from_value::<EntitySelector>(selector.to_owned())? {
		EntitySelector::Game {
			id, tblu: target_tblu, ..
		} if target_tblu == tblu => Some(Ref {
			entity_id: parse_entity_id(&id)?,
			external_scene: None,
			exposed_entity: None
		}),

		other => bail!("{other:?} isn't in the same template")
	}
}

#[try_fn]
fn debounced_event_to_gk_event(evt: SDKEditorEvent) -> Result<Option<Event>> {
	match evt {
		SDKEditorEvent::EntityTransformUpdated {
			entity: EntityDetails::Game {
				id,
				tblu,
				mut properties,
				..
			}
		} => {
			let transform = properties
				.swap_remove("m_mTransform")
				.with_context(|| format!("No m_mTransform on entity {id} whose transform was updated"))?;

			let matrix = from_value::<glacier_bin1::game::h3::SMatrix43>(transform.data)
				.with_context(|| format!("Couldn't parse transform of entity {id} as SMatrix43"))?;

			Some(Event::EditorConnection(EditorConnectionEvent::EntityTransformUpdated {
				id: parse_entity_id(&id)?,
				tblu: parse_tblu(tblu)?,
				transform: matrix
					.to_qn(
						&standalone_factory(),
						&temp_metadata(),
						&standalone_blueprint(),
						&temp_metadata(),
						false
					)
					.into_ok()
			}))
		}

		SDKEditorEvent::EntityPropertyChanged {
			entity: EntityDetails::Game { id, tblu, .. },
			property,
			value
		} => {
			let property_name = match property {
				PropertyID::Unknown(id) => id.to_string(),
				PropertyID::Known(name) => name
			};

			let property_value = match sdk_entity_ref_property(&value.property_type, &value.data, &tblu) {
				Some(Ok(property_value)) => property_value,

				Some(Err(e)) => {
					warn!("Not syncing {property_name} on entity {id} from game: {e:?}");
					return Ok(None);
				}

				None => from_value::<ZVariant>(json!({
					"$type": value.property_type,
					"$val": value.data
				}))
				.with_context(|| format!("Couldn't parse value of {property_name} on entity {id} as ZVariant"))?
				.to_qn(
					&standalone_factory(),
					&temp_metadata(),
					&standalone_blueprint(),
					&temp_metadata(),
					false
				)
				.map_err(|x| anyhow!("QuickEntity error: {:?}", x))
				.with_context(|| format!("Couldn't interpret value of {property_name} on entity {id}"))?
			};

			Some(Event::EditorConnection(EditorConnectionEvent::EntityPropertyChanged {
				id: parse_entity_id(&id)?,
				tblu: parse_tblu(tblu)?,
				property_name,
				property_value
			}))
		}

		// Ignore editor entities
		SDKEditorEvent::EntityTransformUpdated { .. } | SDKEditorEvent::EntityPropertyChanged { .. } => None,

		other => bail!("Event {other:?} should not be debounced")
	}
}

impl EditorConnection {
	pub fn new(app: AppHandle) -> Self {
		let (sender, _) = broadcast::channel(32);

		let (tx, rx) = tokio::sync::mpsc::channel(32);

		let mut recvr = debounced(ReceiverStream::new(rx), Duration::from_millis(200));

		let _app = app.clone();

		spawn(async move {
			let app = _app;

			while let Some(evt) = recvr.next().await {
				match debounced_event_to_gk_event(evt) {
					Ok(Some(event)) => handle_event(&app, event),
					Ok(None) => {}
					Err(e) => report_error(&app, e.context("Couldn't handle SDK editor event"))
				}
			}
		});

		Self {
			sender: RwLock::new(None).into(),
			events: sender,
			entity_tree_loaded: AtomicBool::new(false).into(),
			debounced_events: tx,
			app
		}
	}

	#[try_fn]
	#[context("Couldn't connect to editor server")]
	pub async fn connect(&self) -> Result<()> {
		let mut sender_guard = self.sender.write().await;

		if sender_guard.is_none() {
			let (ws_stream, _) = connect_async("ws://localhost:46735")
				.await
				.context("Couldn't connect to WebSocket server")?;

			let (mut write, read) = ws_stream.split();

			let app = self.app.clone();
			let sender = self.sender.clone();
			let events = self.events.clone();

			self.entity_tree_loaded.store(false, Ordering::SeqCst);

			spawn(async move {
				read.for_each(|msg| async {
					match msg {
						Ok(msg) => {
							match msg {
								Message::Ping(_) => {}
								Message::Pong(_) => {}

								Message::Close(_) => {
									sender.write().await.take();

									let app_state = app.state::<AppState>();
									let mut editor_states = pin!(app_state.editor_states.stream_shards());
									while let Some(shard) = editor_states.next().await {
										for (id, editor) in shard.iter() {
											if let EditorData::QNEntity { .. } | EditorData::QNPatch { .. } =
												editor.data
											{
												let _ = send_request(
													&app,
													Request::Editor(EditorRequest {
														editor: id.to_owned(),
														data: EditorRequestData::Entity(EntityEditorRequest::Tree(
															EntityTreeRequest::SetEditorConnectionAvailable {
																editor_connection_available: false
															}
														))
													})
												);

												let _ = send_request(
													&app,
													Request::Editor(EditorRequest {
														editor: id.to_owned(),
														data: EditorRequestData::Entity(EntityEditorRequest::Monaco(
															EntityMonacoRequest::SetEditorConnected {
																connected: false
															}
														))
													})
												);
											}
										}
									}

									let _ = send_notification(
										&app,
										Notification {
											kind: NotificationKind::Info,
											title: "Disconnected from ZHMModSDK editor".into(),
											subtitle: "Editor integration features will no longer be available.".into()
										}
									);
								}

								_ => {
									if let Err::<_, Error>(e) = try_block! {
										let msg = msg.to_text().context("Couldn't convert message to text")?;

										let msg: SDKEditorEvent = serde_json::from_str(msg)
											.with_context(|| format!("Couldn't parse message {msg:?} as SDKEditorEvent"))?;

										// It's ok if there are no listeners
										let _ = events.send(msg);
									} {
										report_error(&app, e.context("Editor connection message handling error"));
									}
								}
							}
						}

						Err(_) => {
							sender.write().await.take();

							let app_state = app.state::<AppState>();
							let mut editor_states = pin!(app_state.editor_states.stream_shards());
							while let Some(shard) = editor_states.next().await {
								for (id, editor) in shard.iter() {
									if let EditorData::QNEntity { .. } | EditorData::QNPatch { .. } = editor.data {
										let _ = send_request(
											&app,
											Request::Editor(EditorRequest {
												editor: id.to_owned(),
												data: EditorRequestData::Entity(EntityEditorRequest::Tree(
													EntityTreeRequest::SetEditorConnectionAvailable {
														editor_connection_available: false
													}
												))
											})
										);

										let _ = send_request(
											&app,
											Request::Editor(EditorRequest {
												editor: id.to_owned(),
												data: EditorRequestData::Entity(EntityEditorRequest::Monaco(
													EntityMonacoRequest::SetEditorConnected { connected: false }
												))
											})
										);
									}
								}
							}

							let _ = send_notification(
								&app,
								Notification {
									kind: NotificationKind::Info,
									title: "Disconnected from ZHMModSDK editor".into(),
									subtitle: "Editor integration features will no longer be available.".into()
								}
							);
						}
					}
				})
				.await;
			});

			let mut receiver = self.events.subscribe();

			let entity_tree_loaded = self.entity_tree_loaded.clone();

			let app = self.app.clone();

			let debounced_events = self.debounced_events.clone();

			spawn(async move {
				loop {
					if let Ok(evt) = receiver.recv().await {
						match evt {
							SDKEditorEvent::EntityTreeRebuilt => {
								entity_tree_loaded.store(true, Ordering::SeqCst);
							}

							SDKEditorEvent::SceneClearing { .. } | SDKEditorEvent::SceneLoading { .. } => {
								entity_tree_loaded.store(false, Ordering::SeqCst);
							}

							SDKEditorEvent::EntitySelected {
								entity: EntityDetails::Game { id, tblu, .. }
							} => {
								match selection_event(&id, tblu) {
									Ok(event) => handle_event(&app, event),
									Err(e) => report_error(&app, e.context("Couldn't handle SDK entity selection"))
								}
							}

							SDKEditorEvent::EntityTransformUpdated { .. }
							| SDKEditorEvent::EntityPropertyChanged { .. } => {
								if debounced_events.send(evt).await.is_err() {
									report_error(
										&app,
										anyhow!("SDK editor transform and property sync has stopped")
									);
								}
							}

							SDKEditorEvent::Error { message, .. }
								if UNSETTABLE_PROPERTY_ERRORS.iter().any(|x| message.contains(x)) =>
							{
								warn!("SDK editor couldn't set property: {message}");
							}

							SDKEditorEvent::Error { message, .. }
								if !message.contains("Could not find entity for the given selector")
									&& !message.contains("Only spawned entities can be destroyed") =>
							{
								report_error(&app, anyhow!("SDK editor error: {:?}", message));
							}

							_ => {}
						}
					}
				}
			});

			let welcome_receiver = self.events.subscribe();

			write
				.send(Message::Text(
					serde_json::to_string(&SDKEditorRequest::Hello {
						identifier: "GlacierKit".into()
					})?
					.into()
				))
				.await?;

			*sender_guard = Some(write);

			wait_for_event(welcome_receiver, |evt| matches!(evt, SDKEditorEvent::Welcome)).await?;

			let app_state = self.app.state::<AppState>();
			let mut editor_states = pin!(app_state.editor_states.stream_shards());
			while let Some(shard) = editor_states.next().await {
				for (id, editor) in shard.iter() {
					if let EditorData::QNEntity { .. } | EditorData::QNPatch { .. } = editor.data {
						send_request(
							&self.app,
							Request::Editor(EditorRequest {
								editor: id.to_owned(),
								data: EditorRequestData::Entity(EntityEditorRequest::Tree(
									EntityTreeRequest::SetEditorConnectionAvailable {
										editor_connection_available: true
									}
								))
							})
						)?;

						send_request(
							&self.app,
							Request::Editor(EditorRequest {
								editor: id.to_owned(),
								data: EditorRequestData::Entity(EntityEditorRequest::Monaco(
									EntityMonacoRequest::SetEditorConnected { connected: true }
								))
							})
						)?;
					}
				}
			}

			send_notification(
				&self.app,
				Notification {
					kind: NotificationKind::Info,
					title: "Connected to ZHMModSDK editor".into(),
					subtitle: "Selection and property changes will be synced automatically, and the entity context \
					           menu now has additional options."
						.into()
				}
			)?;
		}
	}

	#[try_fn]
	pub async fn disconnect(&self) -> Result<()> {
		let mut sender_guard = self.sender.write().await;

		if sender_guard.is_some() {
			self.entity_tree_loaded.store(false, Ordering::SeqCst);

			sender_guard
				.as_mut()
				.context("Not connected")?
				.send(Message::Close(None))
				.await?;
		}
	}

	pub async fn is_connected(&self) -> bool {
		self.sender.read().await.is_some()
	}

	#[try_fn]
	async fn send_request(&self, request: SDKEditorRequest) -> Result<()> {
		self.ensure_entity_tree_loaded().await?;
		self.send_message(&request).await?;
	}

	async fn request_and_wait(
		&self,
		request: SDKEditorRequest,
		predicate: impl Fn(&SDKEditorEvent) -> bool
	) -> Result<SDKEditorEvent> {
		self.ensure_entity_tree_loaded().await?;

		let receiver = self.events.subscribe();
		self.send_message(&request).await?;

		wait_for_event(receiver, predicate).await
	}

	#[try_fn]
	async fn ensure_entity_tree_loaded(&self) -> Result<()> {
		if !self.entity_tree_loaded.load(Ordering::SeqCst) {
			let receiver = self.events.subscribe();
			self.send_message(&SDKEditorRequest::RebuildEntityTree)
				.await?;

			wait_for_event(receiver, |evt| matches!(evt, SDKEditorEvent::EntityTreeRebuilt)).await?;
		}
	}

	#[try_fn]
	async fn send_message(&self, request: &SDKEditorRequest) -> Result<()> {
		self.sender
			.write()
			.await
			.as_mut()
			.context("Not connected")?
			.send(Message::Text(serde_json::to_string(request)?.into()))
			.await?;
	}

	#[try_fn]
	#[context("Couldn't select entity {:?}", entity_id)]
	pub async fn select_entity(&self, entity_id: EntityID, tblu: &str) -> Result<()> {
		self.send_request(SDKEditorRequest::SelectEntity {
			entity: EntitySelector::game(entity_id, tblu)
		})
		.await?;
	}

	#[try_fn]
	#[context("Couldn't set name of entity {:?}", entity_id)]
	pub async fn set_entity_name(&self, entity_id: EntityID, tblu: &str, name: &str) -> Result<()> {
		self.send_request(SDKEditorRequest::SetEntityName {
			entity: EntitySelector::game(entity_id, tblu),
			name: name.to_owned()
		})
		.await?;
	}

	#[try_fn]
	#[context("Couldn't get player transform")]
	pub async fn get_player_transform(&self) -> Result<Transform> {
		let msg_id: i64 = rand::random();
		let SDKEditorEvent::HitmanEntity { entity, .. } = self
			.request_and_wait(SDKEditorRequest::GetHitmanEntity { msgId: Some(msg_id) }, |evt| {
				matches!(evt, SDKEditorEvent::HitmanEntity { msgId: Some(x), .. } if *x == msg_id)
			})
			.await?
		else {
			unreachable!()
		};

		let EntityDetails::Game { transform, .. } = entity else {
			unreachable!()
		};

		let transform = transform.context("Returned hitman entity had no transform")?;
		let transform: SDKTransform = from_value(transform).context("Invalid transform")?;

		Transform {
			position: transform.position,
			rotation: Vec3 {
				x: (transform.rotation.yaw * 180.0 / std::f64::consts::PI) as f32,
				y: (transform.rotation.pitch * 180.0 / std::f64::consts::PI) as f32,
				z: (transform.rotation.roll * 180.0 / std::f64::consts::PI) as f32
			},
			scale: if (transform.scale.x * 100.0).trunc() == 100.0
				&& (transform.scale.y * 100.0).trunc() == 100.0
				&& (transform.scale.z * 100.0).trunc() == 100.0
			{
				None
			} else {
				Some(transform.scale)
			}
		}
	}

	#[try_fn]
	#[context("Couldn't get camera transform")]
	pub async fn get_camera_transform(&self) -> Result<Transform> {
		let msg_id: i64 = rand::random();
		let SDKEditorEvent::CameraEntity { entity, .. } = self
			.request_and_wait(SDKEditorRequest::GetCameraEntity { msgId: Some(msg_id) }, |evt| {
				matches!(evt, SDKEditorEvent::CameraEntity { msgId: Some(x), .. } if *x == msg_id)
			})
			.await?
		else {
			unreachable!()
		};

		let EntityDetails::Editor { transform, .. } = entity else {
			unreachable!()
		};

		let transform = transform.context("Returned camera entity had no transform")?;
		let transform: SDKTransform = from_value(transform).context("Invalid transform")?;

		Transform {
			position: transform.position,
			rotation: Vec3 {
				x: (transform.rotation.yaw * 180.0 / std::f64::consts::PI) as f32,
				y: (transform.rotation.pitch * 180.0 / std::f64::consts::PI) as f32,
				z: (transform.rotation.roll * 180.0 / std::f64::consts::PI) as f32
			},
			scale: if (transform.scale.x * 100.0).trunc() == 100.0
				&& (transform.scale.y * 100.0).trunc() == 100.0
				&& (transform.scale.z * 100.0).trunc() == 100.0
			{
				None
			} else {
				Some(transform.scale)
			}
		}
	}

	#[try_fn]
	#[context("Couldn't set property {property} on {entity_id}")]
	pub async fn set_property(&self, entity_id: EntityID, tblu: &str, property: &str, value: Variant) -> Result<()> {
		let (_, value) = self.sdk_property_value(&value, tblu)?;

		self.send_request(SDKEditorRequest::SetEntityProperty {
			entity: EntitySelector::game(entity_id, tblu),
			property: PropertyID::from_qn_name(property),
			value
		})
		.await?;
	}

	/// The SDK type name and RT JSON value of a property.
	#[try_fn]
	fn sdk_property_value(&self, value: &Variant, tblu: &str) -> Result<(String, Value)> {
		match value {
			Variant::Ref(reference) => (value.variant_type().to_string(), self.ref_selector(reference, tblu)?),

			Variant::Array(ty, vals) if ty == "SEntityTemplateReference" => vals
				.iter()
				.map(|val| {
					let Variant::Ref(val) = val else {
						bail!("Expected array of SEntityTemplateReference to contain refs")
					};

					self.ref_selector(val, tblu)
				})
				.collect::<Result<Vec<Value>>>()
				.map(|x| (value.variant_type().to_string(), Value::Array(x)))?,

			Variant::Uuid(val) => (
				value.variant_type().to_string(),
				Value::String(val.to_string().to_uppercase())
			),

			_ => {
				let mut resources = Vec::new();
				collect_resources(value, &mut resources);

				// Resources are indexed by their own ID so the RT value holds the real ID rather than a header index
				let resource_indices = resources
					.into_iter()
					.map(|x| (x, x.as_u64() as usize))
					.collect();

				let value =
					ZVariant::from_qn(value, &Default::default(), &resource_indices, &Default::default())
						.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?;

				(value.variant_type().to_owned(), value.to_serde()?)
			}
		}
	}

	#[try_fn]
	fn ref_selector(&self, reference: &Option<Ref>, tblu: &str) -> Result<Value> {
		match reference {
			None => Value::Null,
			Some(reference) => serde_json::to_value(self.ref_entity_selector(reference, tblu)?)?
		}
	}

	#[try_fn]
	fn ref_entity_selector(&self, reference: &Ref, tblu: &str) -> Result<EntitySelector> {
		EntitySelector::Game {
			id: reference.entity_id.to_string(),
			tblu: match reference.external_scene {
				Some(scene) => self.external_scene_tblu(scene)?,
				None => tblu.to_owned()
			},
			exposed_entity: reference.exposed_entity.as_ref().map(|x| x.to_string())
		}
	}

	/// Event pin connections from `source` whose target passes `include`, skipping ones that can't be converted.
	fn event_pin_connections<K>(
		&self,
		source_id: EntityID,
		source: &SubEntity,
		tblu: &str,
		select_target: impl Fn(&Ref) -> Option<K>
	) -> Vec<(K, PinConnectionToCreate)> {
		let from = EntitySelector::game(source_id, tblu);

		let mut pins = vec![];

		for (event, inputs) in &source.events {
			for (input, connections) in inputs {
				for connection in connections {
					let Some(target) = select_target(&connection.entity_ref) else {
						continue;
					};

					match self.pin_connection_to_create(&from, event, connection, input, tblu) {
						Ok(pin) => pins.push((target, pin)),
						Err(e) => warn!("Not sending pin {source_id}.{event} -> {input}: {e:?}")
					}
				}
			}
		}

		pins
	}

	#[try_fn]
	fn pin_connection_to_create(
		&self,
		from: &EntitySelector,
		event: &str,
		connection: &PinConnection,
		input: &str,
		tblu: &str
	) -> Result<PinConnectionToCreate> {
		PinConnectionToCreate {
			from: from.clone(),
			from_pin: PropertyID::from_qn_name(event),
			to: self.ref_entity_selector(&connection.entity_ref, tblu)?,
			to_pin: PropertyID::from_qn_name(input),
			value: connection
				.value
				.as_ref()
				.map(|value| self.sdk_property_value(value, tblu))
				.transpose()?
				.map(|(property_type, data)| PropertyValue { property_type, data })
		}
	}

	#[try_fn]
	#[context("Couldn't find blueprint of external scene {scene}")]
	fn external_scene_tblu(&self, scene: RuntimeID) -> Result<String> {
		let app_state = self.app.state::<AppState>();
		let game = app_state.game.load();
		let game = game.as_ref().context("No game loaded")?;

		app_state.intellisense_game(game).extract_entity(scene)?.blueprint.to_hash()
	}

	#[try_fn]
	#[context("Couldn't signal pin {pin} on {entity_id}")]
	pub async fn signal_pin(&self, entity_id: EntityID, tblu: &str, pin: &str, output: bool) -> Result<()> {
		self.send_request(SDKEditorRequest::SignalEntityPin {
			entity: EntitySelector::game(entity_id, tblu),
			pin: PropertyID::from_qn_name(pin),
			output
		})
		.await?;
	}

	#[try_fn]
	#[context("Couldn't check whether {entity_id} exists in game")]
	pub async fn entity_exists(&self, entity_id: EntityID, tblu: &str) -> Result<bool> {
		let msg_id: i64 = rand::random();
		let response = self
			.request_and_wait(
				SDKEditorRequest::GetEntityDetails {
					entity: EntitySelector::game(entity_id, tblu),
					msgId: Some(msg_id)
				},
				|evt| {
					matches!(
						evt,
						SDKEditorEvent::EntityDetails { msgId: Some(x), .. } | SDKEditorEvent::Error { msgId: Some(x), .. }
							if *x == msg_id
					)
				}
			)
			.await?;

		matches!(response, SDKEditorEvent::EntityDetails { .. })
	}

	#[try_fn]
	#[context("Couldn't spawn entities in game")]
	pub async fn spawn_entities(&self, owner: &Entity, new_entities: &[(EntityID, SubEntity)]) -> Result<()> {
		let tblu = owner.blueprint.to_hash();

		let new_entity_ids: HashSet<EntityID> = new_entities.iter().map(|(id, _)| *id).collect();

		let mut inbound_pins: HashMap<EntityID, Vec<PinConnectionToCreate>> = HashMap::new();
		for (source_id, source) in &owner.sub_entities {
			if new_entity_ids.contains(source_id) {
				continue;
			}

			for (target_id, pin) in self.event_pin_connections(*source_id, source, &tblu, |target| {
				target.as_local().filter(|id| new_entity_ids.contains(id))
			}) {
				inbound_pins.entry(target_id).or_default().push(pin);
			}
		}

		let entities = new_entities
			.iter()
			.map(|(entity_id, sub_entity)| {
				let parent = sub_entity
					.parent
					.as_ref()
					.and_then(Ref::as_local)
					.with_context(|| format!("Entity {entity_id} has no local parent"))?;

				let properties = sub_entity
					.properties
					.iter()
					.filter_map(|(name, property)| match self.sdk_property_value(&property.value, &tblu) {
						Ok((property_type, value)) => Some((
							sdk_property_name(name),
							SubEntityPropertyValue {
								property_type,
								data: value,
								post_init: property.post_init
							}
						)),

						Err(e) => {
							warn!("Not spawning {entity_id} with {name}: {e:?}");
							None
						}
					})
					.collect();

				Ok(SubEntityToCreate {
					id: entity_id.to_string(),
					tblu: tblu.clone(),
					parent: EntitySelector::game(parent, tblu.clone()),
					name: sub_entity.name.to_string(),
					factory: format!("{:016X}", sub_entity.factory.resource.as_u64()),
					blueprint: format!("{:016X}", sub_entity.blueprint.as_u64()),
					properties,
					inbound_pins: inbound_pins.remove(entity_id).unwrap_or_default(),
					outbound_pins: self
						.event_pin_connections(*entity_id, sub_entity, &tblu, |_| Some(()))
						.into_iter()
						.map(|(_, pin)| pin)
						.collect()
				})
			})
			.collect::<Result<Vec<_>>>()?;

		self.send_request(SDKEditorRequest::CreateSubEntities { entities })
			.await?;
	}

	#[try_fn]
	#[context("Couldn't destroy entity {entity_id} in game")]
	pub async fn destroy_entity(&self, entity_id: EntityID, tblu: &str) -> Result<()> {
		self.send_request(SDKEditorRequest::DestroyEntity {
			entity: EntitySelector::game(entity_id, tblu)
		})
		.await?;
	}
}

fn collect_resources(value: &Variant, resources: &mut Vec<RuntimeID>) {
	match value {
		Variant::Resource(_, Some(reference)) => resources.push(reference.resource),
		Variant::Array(_, vals) => vals.iter().for_each(|x| collect_resources(x, resources)),
		Variant::Variant(x) | Variant::PairStringVariant(_, x) => collect_resources(x, resources),
		_ => {}
	}
}

fn sdk_property_name(name: &str) -> String {
	match PropertyID::from_qn_name(name) {
		PropertyID::Unknown(id) => format!("~{id:08X}"),
		PropertyID::Known(name) => name
	}
}

const SDK_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

async fn wait_for_event(
	mut receiver: broadcast::Receiver<SDKEditorEvent>,
	predicate: impl Fn(&SDKEditorEvent) -> bool
) -> Result<SDKEditorEvent> {
	timeout(SDK_RESPONSE_TIMEOUT, async {
		loop {
			match receiver.recv().await {
				Ok(event) if predicate(&event) => return Ok(event),
				Ok(_) | Err(RecvError::Lagged(_)) => {}
				Err(RecvError::Closed) => bail!("Event channel closed")
			}
		}
	})
	.await
	.context("Timed out waiting for a response from the SDK editor")?
}
