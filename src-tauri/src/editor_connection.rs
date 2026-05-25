use std::{
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
use hitman_bin1::game::h3::{STemplateEntityBlueprint, STemplateEntityFactory, ZVariant};
use hitman_commons::{game::GameVersion, metadata::ResourceMetadata, resource_type};
use indexmap::IndexMap;
use quickentity_rs::{
	entity::{EntityID, Ref},
	variant::{Transform, Variant, Vec3}
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, from_value, json};
use specta::Type;
use tauri::{AppHandle, Manager, async_runtime::spawn};
use tokio::{
	net::TcpStream,
	sync::{RwLock, broadcast}
};
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};
use tryvial::{try_block, try_fn};

use crate::{
	Notification, NotificationKind,
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
	Game { id: String, tblu: String },
	Editor { id: String }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum PropertyID {
	Unknown(i32),
	Known(String)
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

	/// Unimplemented on SDK side.
	SpawnEntity {
		templateId: String,
		entityId: String,
		name: String
	},

	/// Unimplemented on SDK side.
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

	RebuildEntityTree {
		#[serde(skip_serializing_if = "Option::is_none")]
		msgId: Option<i64>
	}
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

impl EditorConnection {
	pub fn new(app: AppHandle) -> Self {
		let (sender, _) = broadcast::channel(32);

		let (tx, rx) = tokio::sync::mpsc::channel(32);

		let mut recvr = debounced(ReceiverStream::new(rx), Duration::from_millis(200));

		let _app = app.clone();

		spawn(async move {
			let app = _app;

			while let Some(evt) = recvr.next().await {
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
							.expect("No m_mTransform on entity whose transform was updated");

						handle_event(
							&app,
							Event::EditorConnection(EditorConnectionEvent::EntityTransformUpdated {
								id: id.parse().expect("Couldn't parse entity ID"),
								tblu: Hash(tblu.parse().expect("Couldn't parse TBLU hash")),
								transform: {
									let matrix = from_value::<ZVariant>(json!({
										"$type": transform.property_type,
										"$val": transform.data
									}))
									.expect("Couldn't parse transform as ZVariant");

									Transform::from_game(matrix.as_ref().expect("Transform was not SMatrix43"), false)
								}
							})
						);
					}

					SDKEditorEvent::EntityPropertyChanged {
						entity: EntityDetails::Game { id, tblu, .. },
						property,
						value
					} => {
						handle_event(
							&app,
							Event::EditorConnection(EditorConnectionEvent::EntityPropertyChanged {
								id: id.parse().expect("Couldn't parse entity ID"),
								tblu: Hash(tblu.parse().expect("Couldn't parse TBLU hash")),
								property_name: match property {
									PropertyID::Unknown(id) => id.to_string(),
									PropertyID::Known(name) => name
								},
								property_value: match Variant::from_game(
									&from_value::<ZVariant>(json!({
										"$type": value.property_type,
										"$val": value.data
									}))
									.expect("Couldn't parse value as ZVariant"),
									&STemplateEntityFactory {
										blueprint_index_in_resource_header: 0,
										root_entity_index: 0,
										sub_type: 0,
										sub_entities: vec![],
										external_scene_type_indices_in_resource_header: vec![],
										property_overrides: vec![]
									},
									&ResourceMetadata {
										id: EMPTY_ID,
										resource_type: resource_type!("TEMP"),
										compressed: true,
										scrambled: true,
										references: vec![]
									},
									&STemplateEntityBlueprint {
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
									},
									false
								)
								.map_err(|x| anyhow!("QuickEntity error: {:?}", x))
								{
									Ok(x) => x,
									Err(e) => {
										send_request(
											&app,
											Request::Global(GlobalRequest::ErrorReport {
												error: format!(
													"{:?}",
													e.context("Couldn't interpret SDK property changed value")
												)
											})
										)
										.expect("Couldn't send error report to frontend");

										continue;
									}
								}
							})
						);
					}

					// Ignore editor entities
					SDKEditorEvent::EntityTransformUpdated { .. } => {}

					// Ignore editor entities
					SDKEditorEvent::EntityPropertyChanged { .. } => {}

					_ => panic!("This event kind should not be debounced")
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
										send_request(
											&app,
											Request::Global(GlobalRequest::ErrorReport {
												error: format!(
													"{:?}",
													e.context("Editor connection message handling error")
												)
											})
										)
										.expect("Couldn't send error report to frontend");
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
										send_request(
											&app,
											Request::Editor(EditorRequest {
												editor: id.to_owned(),
												data: EditorRequestData::Entity(EntityEditorRequest::Tree(
													EntityTreeRequest::SetEditorConnectionAvailable {
														editor_connection_available: false
													}
												))
											})
										)
										.expect("Couldn't send data to frontend");

										send_request(
											&app,
											Request::Editor(EditorRequest {
												editor: id.to_owned(),
												data: EditorRequestData::Entity(EntityEditorRequest::Monaco(
													EntityMonacoRequest::SetEditorConnected { connected: false }
												))
											})
										)
										.expect("Couldn't send data to frontend");
									}
								}
							}

							send_notification(
								&app,
								Notification {
									kind: NotificationKind::Info,
									title: "Disconnected from ZHMModSDK editor".into(),
									subtitle: "Editor integration features will no longer be available.".into()
								}
							)
							.expect("Couldn't send data to frontend");
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
								handle_event(
									&app,
									Event::EditorConnection(EditorConnectionEvent::EntitySelected {
										id: id.parse().expect("Couldn't parse entity ID"),
										tblu: Hash(tblu.parse().expect("Couldn't parse TBLU hash"))
									})
								);
							}

							SDKEditorEvent::EntityTransformUpdated { .. }
							| SDKEditorEvent::EntityPropertyChanged { .. } => {
								debounced_events
									.send(evt)
									.await
									.expect("Couldn't queue debounced event");
							}

							SDKEditorEvent::Error { message, .. }
								if !message.contains("Could not find entity for the given selector") =>
							{
								send_request(
									&app,
									Request::Global(GlobalRequest::ErrorReport {
										error: format!("SDK editor error: {:?}", message)
									})
								)
								.expect("Couldn't send error report to frontend");
							}

							_ => {}
						}
					}
				}
			});

			write
				.send(Message::Text(
					serde_json::to_string(&SDKEditorRequest::Hello {
						identifier: "GlacierKit".into()
					})?
					.into()
				))
				.await?;

			*sender_guard = Some(write);

			self.wait_for_event(|evt| matches!(evt, SDKEditorEvent::Welcome))
				.await?;

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
		if !self.entity_tree_loaded.load(Ordering::SeqCst) {
			self.sender
				.write()
				.await
				.as_mut()
				.context("Not connected")?
				.send(Message::Text(
					serde_json::to_string(&SDKEditorRequest::RebuildEntityTree { msgId: None })?.into()
				))
				.await?;

			self.wait_for_event(|evt| matches!(evt, SDKEditorEvent::EntityTreeRebuilt))
				.await?;
		}

		self.sender
			.write()
			.await
			.as_mut()
			.context("Not connected")?
			.send(Message::Text(serde_json::to_string(&request)?.into()))
			.await?;
	}

	async fn wait_for_event(&self, predicate: impl Fn(&SDKEditorEvent) -> bool) -> Result<SDKEditorEvent> {
		let mut receiver = self.events.subscribe();

		loop {
			let event = receiver.recv().await.context("Event channel closed")?;

			if predicate(&event) {
				return Ok(event);
			}
		}
	}

	#[try_fn]
	#[context("Couldn't select entity {:?}", entity_id)]
	pub async fn select_entity(&self, entity_id: EntityID, tblu: &str) -> Result<()> {
		self.send_request(SDKEditorRequest::SelectEntity {
			entity: EntitySelector::Game {
				id: entity_id.to_string(),
				tblu: tblu.to_owned()
			}
		})
		.await?;
	}

	#[try_fn]
	#[context("Couldn't get player transform")]
	pub async fn get_player_transform(&self) -> Result<Transform> {
		let msg_id: i64 = rand::random();
		self.send_request(SDKEditorRequest::GetHitmanEntity { msgId: Some(msg_id) })
			.await?;

		let SDKEditorEvent::HitmanEntity { entity, .. } = self
			.wait_for_event(|evt| matches!(evt, SDKEditorEvent::HitmanEntity { msgId: Some(x), .. } if *x == msg_id))
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
		self.send_request(SDKEditorRequest::GetCameraEntity { msgId: Some(msg_id) })
			.await?;

		let SDKEditorEvent::CameraEntity { entity, .. } = self
			.wait_for_event(|evt| matches!(evt, SDKEditorEvent::CameraEntity { msgId: Some(x), .. } if *x == msg_id))
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
		match value {
			Variant::Ref(val) => {
				self.send_request(SDKEditorRequest::SetEntityProperty {
					entity: EntitySelector::Game {
						id: entity_id.to_string(),
						tblu: tblu.to_owned()
					},
					property: property
						.parse()
						.map(PropertyID::Unknown)
						.unwrap_or(PropertyID::Known(property.to_owned())),
					value: match val {
						Some(Ref {
							entity_id,
							external_scene: Some(scene),
							exposed_entity: None
						}) => json!({
							"id": entity_id,
							"source": "game",
							"tblu": scene.to_hash()
						}),

						Some(Ref {
							entity_id,
							external_scene: None,
							exposed_entity: None
						}) => json!({
							"id": entity_id,
							"source": "game",
							"tblu": tblu.to_owned()
						}),

						None => Value::Null,

						_ => return Ok(()) // Can't set exposed entities
					}
				})
				.await?;
			}

			Variant::Array(ty, vals) if ty == "SEntityTemplateReference" => {
				self.send_request(SDKEditorRequest::SetEntityProperty {
					entity: EntitySelector::Game {
						id: entity_id.to_string(),
						tblu: tblu.to_owned()
					},
					property: property
						.parse()
						.map(PropertyID::Unknown)
						.unwrap_or(PropertyID::Known(property.to_owned())),
					value: match vals
						.into_iter()
						.map(|val| {
							let Variant::Ref(val) = val else {
								bail!("Expected array of SEntityTemplateReference to contain refs")
							};

							Ok(match val {
								Some(Ref {
									entity_id,
									external_scene: Some(scene),
									exposed_entity: None
								}) => Some(json!({
									"id": entity_id,
									"source": "game",
									"tblu": scene.to_hash()
								})),

								Some(Ref {
									entity_id,
									external_scene: None,
									exposed_entity: None
								}) => Some(json!({
									"id": entity_id,
									"source": "game",
									"tblu": tblu.to_owned()
								})),

								None => Some(Value::Null),

								_ => None // Can't set exposed entities
							})
						})
						.collect::<Result<Option<Vec<Value>>>>()?
					{
						Some(x) => Value::Array(x),
						None => return Ok(())
					}
				})
				.await?;
			}

			Variant::Uuid(val) => {
				self.send_request(SDKEditorRequest::SetEntityProperty {
					entity: EntitySelector::Game {
						id: entity_id.to_string(),
						tblu: tblu.to_owned()
					},
					property: property
						.parse()
						.map(PropertyID::Unknown)
						.unwrap_or(PropertyID::Known(property.to_owned())),
					value: Value::String(val.to_string().to_uppercase())
				})
				.await?;
			}

			_ => {
				self.send_request(SDKEditorRequest::SetEntityProperty {
					entity: EntitySelector::Game {
						id: entity_id.to_string(),
						tblu: tblu.to_owned()
					},
					property: property
						.parse()
						.map(PropertyID::Unknown)
						.unwrap_or(PropertyID::Known(property.to_owned())),
					value: value
						.to_game(
							GameVersion::H3,
							&STemplateEntityFactory {
								blueprint_index_in_resource_header: 0,
								root_entity_index: 0,
								sub_type: 0,
								sub_entities: vec![],
								external_scene_type_indices_in_resource_header: vec![],
								property_overrides: vec![]
							},
							&ResourceMetadata {
								id: EMPTY_ID,
								resource_type: resource_type!("TEMP"),
								compressed: true,
								scrambled: true,
								references: vec![]
							},
							&Default::default(),
							&Default::default()
						)
						.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?
						.to_serde()?
				})
				.await?;
			}
		}
	}

	#[try_fn]
	#[context("Couldn't signal pin {pin} on {entity_id}")]
	pub async fn signal_pin(&self, entity_id: EntityID, tblu: &str, pin: &str, output: bool) -> Result<()> {
		self.send_request(SDKEditorRequest::SignalEntityPin {
			entity: EntitySelector::Game {
				id: entity_id.to_string(),
				tblu: tblu.to_owned()
			},
			pin: pin
				.parse()
				.map(PropertyID::Unknown)
				.unwrap_or(PropertyID::Known(pin.to_owned())),
			output
		})
		.await?;
	}
}
