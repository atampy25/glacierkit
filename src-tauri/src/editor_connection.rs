use std::{
	sync::{
		atomic::{AtomicBool, Ordering},
		Arc
	},
	time::Duration
};

use anyhow::{anyhow, Context, Error, Result};
use arc_swap::ArcSwap;
use debounced::debounced;
use fn_error_context::context;
use futures_util::{stream::SplitSink, SinkExt, StreamExt};
use indexmap::IndexMap;
use itertools::Itertools;
use quickentity_rs::{
	convert_qn_property_value_to_rt, convert_rt_property_value_to_qn,
	qn_structs::{FullRef, Property, Ref},
	rt_structs::SEntityTemplatePropertyValue
};
use rand::{thread_rng, Rng};
use serde::{Deserialize, Serialize};
use serde_json::{from_value, json, Value};
use specta::Type;
use tauri::{async_runtime::spawn, AppHandle, Manager};
use tokio::{
	net::TcpStream,
	sync::{broadcast, Mutex}
};
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{
	connect_async,
	tungstenite::{protocol::WebSocketConfig, Message},
	MaybeTlsStream, WebSocketStream
};
use tryvial::try_fn;

use crate::{
	handle_event,
	model::{
		AppState, EditorConnectionEvent, EditorData, EditorRequest, EntityEditorRequest, EntityMonacoRequest,
		EntityTreeRequest, Event, GlobalRequest, Request
	},
	rpkg::normalise_to_hash,
	send_notification, send_request, Notification, NotificationKind
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
pub struct Vec3 {
	pub x: f64,
	pub y: f64,
	pub z: f64
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
pub struct Transform {
	position: Vec3,
	rotation: Rotation,
	scale: Vec3
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct QNTransform {
	pub rotation: Vec3,
	pub position: Vec3,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub scale: Option<Vec3>
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
		transform: Transform,
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

	EntityDestroyed {
		entity: EntitySelector
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
	sender: Arc<Mutex<Option<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>>,
	events: broadcast::Sender<SDKEditorEvent>,
	debounced_events: tokio::sync::mpsc::Sender<SDKEditorEvent>,
	entity_tree_loaded: Arc<AtomicBool>,
	app: AppHandle
}

impl EditorConnection {
	pub fn new(app: AppHandle) -> Self {
		let (sender, _) = broadcast::channel(32);

		let (tx, rx) = tokio::sync::mpsc::channel(32);

		let mut recvr = debounced(ReceiverStream::new(rx), Duration::from_millis(500));

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
							Event::EditorConnection(EditorConnectionEvent::EntityTransformUpdated(
								id,
								tblu,
								from_value(
									convert_rt_property_value_to_qn(
										&SEntityTemplatePropertyValue {
											property_type: transform.property_type,
											property_value: transform.data
										},
										&Default::default(),
										&Default::default(),
										&Default::default(),
										false
									)
									.map_err(|x| anyhow!("QuickEntity error: {:?}", x))
									.expect("Couldn't convert transform value to QN")
								)
								.expect("Couldn't parse QN transform")
							))
						);
					}

					SDKEditorEvent::EntityPropertyChanged {
						entity: EntityDetails::Game { id, tblu, .. },
						property,
						value
					} => {
						handle_event(
							&app,
							Event::EditorConnection(EditorConnectionEvent::EntityPropertyChanged(
								id,
								tblu,
								match property {
									PropertyID::Unknown(id) => id.to_string(),
									PropertyID::Known(name) => name
								},
								value.property_type.to_owned(),
								convert_rt_property_value_to_qn(
									&SEntityTemplatePropertyValue {
										property_type: value.property_type,
										property_value: value.data
									},
									&Default::default(),
									&Default::default(),
									&Default::default(),
									false
								)
								.map_err(|x| anyhow!("QuickEntity error: {:?}", x))
								.expect("Couldn't convert new property value to QN")
							))
						);
					}

					_ => panic!("This event kind should not be debounced")
				}
			}
		});

		Self {
			sender: Mutex::new(None).into(),
			events: sender,
			entity_tree_loaded: AtomicBool::new(false).into(),
			debounced_events: tx,
			app
		}
	}

	#[try_fn]
	#[context("Couldn't connect to editor server")]
	pub async fn connect(&self) -> Result<()> {
		let mut sender_guard = self.sender.lock().await;

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
							if let Err::<_, Error>(e) = try {
								match msg {
									Message::Ping(_) => {}
									Message::Pong(_) => {}

									Message::Close(_) => {
										sender.lock().await.take();

										for editor in app.state::<AppState>().editor_states.iter() {
											if let EditorData::QNEntity { .. } | EditorData::QNPatch { .. } =
												editor.data
											{
												send_request(
													&app,
													Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
														EntityTreeRequest::SetEditorConnectionAvailable {
															editor_id: editor.key().to_owned(),
															editor_connection_available: false
														}
													)))
												)?;

												send_request(
													&app,
													Request::Editor(EditorRequest::Entity(
														EntityEditorRequest::Monaco(
															EntityMonacoRequest::SetEditorConnected {
																editor_id: editor.key().to_owned(),
																connected: false
															}
														)
													))
												)?;
											}
										}

										send_notification(
											&app,
											Notification {
												kind: NotificationKind::Info,
												title: "Disconnected from ZHMModSDK editor".into(),
												subtitle: "Editor integration features will no longer be available."
													.into()
											}
										)?;
									}

									_ => {
										let msg = msg.to_text().context("Couldn't convert message to text")?;

										let msg: SDKEditorEvent = serde_json::from_str(msg).with_context(|| {
											format!("Couldn't parse message {msg:?} as SDKEditorEvent")
										})?;

										// It's ok if there are no listeners
										let _ = events.send(msg);
									}
								}
							} {
								send_request(
									&app,
									Request::Global(GlobalRequest::ErrorReport {
										error: format!("{:?}", e.context("Editor connection message handling error"))
									})
								)
								.expect("Couldn't send error report to frontend");
							}
						}

						Err(e) => {
							sender.lock().await.take();

							for editor in app.state::<AppState>().editor_states.iter() {
								if let EditorData::QNEntity { .. } | EditorData::QNPatch { .. } = editor.data {
									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
											EntityTreeRequest::SetEditorConnectionAvailable {
												editor_id: editor.key().to_owned(),
												editor_connection_available: false
											}
										)))
									)
									.expect("Couldn't send data to frontend");

									send_request(
										&app,
										Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
											EntityMonacoRequest::SetEditorConnected {
												editor_id: editor.key().to_owned(),
												connected: false
											}
										)))
									)
									.expect("Couldn't send data to frontend");
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
									Event::EditorConnection(EditorConnectionEvent::EntitySelected(id, tblu))
								);
							}

							SDKEditorEvent::EntityTransformUpdated { .. }
							| SDKEditorEvent::EntityPropertyChanged { .. } => {
								debounced_events
									.send(evt)
									.await
									.expect("Couldn't queue debounced event");
							}

							SDKEditorEvent::Error { message, .. } => {
								if !message.contains("Could not find entity for the given selector") {
									send_request(
										&app,
										Request::Global(GlobalRequest::ErrorReport {
											error: format!("SDK editor error: {:?}", message)
										})
									)
									.expect("Couldn't send error report to frontend");
								}
							}

							_ => {}
						}
					}
				}
			});

			write
				.send(Message::Text(serde_json::to_string(&SDKEditorRequest::Hello {
					identifier: "GlacierKit".into()
				})?))
				.await?;

			*sender_guard = Some(write);

			self.wait_for_event(|evt| matches!(evt, SDKEditorEvent::Welcome))
				.await?;

			for editor in self.app.state::<AppState>().editor_states.iter() {
				if let EditorData::QNEntity { .. } | EditorData::QNPatch { .. } = editor.data {
					send_request(
						&self.app,
						Request::Editor(EditorRequest::Entity(EntityEditorRequest::Tree(
							EntityTreeRequest::SetEditorConnectionAvailable {
								editor_id: editor.key().to_owned(),
								editor_connection_available: true
							}
						)))
					)?;

					send_request(
						&self.app,
						Request::Editor(EditorRequest::Entity(EntityEditorRequest::Monaco(
							EntityMonacoRequest::SetEditorConnected {
								editor_id: editor.key().to_owned(),
								connected: true
							}
						)))
					)?;
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
		let mut sender_guard = self.sender.lock().await;

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
		self.sender.lock().await.is_some()
	}

	#[try_fn]
	async fn send_request(&self, request: SDKEditorRequest) -> Result<()> {
		if !self.entity_tree_loaded.load(Ordering::SeqCst) {
			self.sender
				.lock()
				.await
				.as_mut()
				.context("Not connected")?
				.send(Message::Text(serde_json::to_string(
					&SDKEditorRequest::RebuildEntityTree { msgId: None }
				)?))
				.await?;

			self.wait_for_event(|evt| matches!(evt, SDKEditorEvent::EntityTreeRebuilt))
				.await?;
		}

		self.sender
			.lock()
			.await
			.as_mut()
			.context("Not connected")?
			.send(Message::Text(serde_json::to_string(&request)?))
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
	pub async fn select_entity(&self, entity_id: &str, tblu: &str) -> Result<()> {
		self.send_request(SDKEditorRequest::SelectEntity {
			entity: EntitySelector::Game {
				id: entity_id.to_owned(),
				tblu: tblu.to_owned()
			}
		})
		.await?;
	}

	#[try_fn]
	#[context("Couldn't get player transform")]
	pub async fn get_player_transform(&self) -> Result<QNTransform> {
		let msg_id: i64 = thread_rng().gen();
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
		let transform: Transform = from_value(transform).context("Invalid transform")?;

		QNTransform {
			position: transform.position,
			rotation: Vec3 {
				x: transform.rotation.yaw * 180.0 / std::f64::consts::PI,
				y: transform.rotation.pitch * 180.0 / std::f64::consts::PI,
				z: transform.rotation.roll * 180.0 / std::f64::consts::PI
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
	pub async fn get_camera_transform(&self) -> Result<QNTransform> {
		let msg_id: i64 = thread_rng().gen();
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
		let transform: Transform = from_value(transform).context("Invalid transform")?;

		QNTransform {
			position: transform.position,
			rotation: Vec3 {
				x: transform.rotation.yaw * 180.0 / std::f64::consts::PI,
				y: transform.rotation.pitch * 180.0 / std::f64::consts::PI,
				z: transform.rotation.roll * 180.0 / std::f64::consts::PI
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
	pub async fn set_property(&self, entity_id: &str, tblu: &str, property: &str, value: PropertyValue) -> Result<()> {
		if value.property_type == "SEntityTemplateReference" {
			self.send_request(SDKEditorRequest::SetEntityProperty {
				entity: EntitySelector::Game {
					id: entity_id.to_owned(),
					tblu: tblu.to_owned()
				},
				property: property
					.parse()
					.map(PropertyID::Unknown)
					.unwrap_or(PropertyID::Known(property.to_owned())),
				value: match from_value::<Ref>(value.data)? {
					Ref::Full(FullRef {
						entity_ref,
						external_scene: Some(scene),
						exposed_entity: None
					}) => json!({
						"id": entity_ref,
						"source": "game",
						"tblu": normalise_to_hash(scene)
					}),

					Ref::Short(Some(entity_id)) => json!({
						"id": entity_id,
						"source": "game",
						"tblu": tblu.to_owned()
					}),

					Ref::Short(None) => Value::Null,

					_ => return Ok(()) // Can't set exposed entities
				}
			})
			.await?;
		} else {
			self.send_request(SDKEditorRequest::SetEntityProperty {
				entity: EntitySelector::Game {
					id: entity_id.to_owned(),
					tblu: tblu.to_owned()
				},
				property: property
					.parse()
					.map(PropertyID::Unknown)
					.unwrap_or(PropertyID::Known(property.to_owned())),
				value: convert_qn_property_value_to_rt(
					&Property {
						property_type: value.property_type,
						value: value.data,
						post_init: None
					},
					&Default::default(),
					&Default::default(),
					&Default::default(),
					&Default::default()
				)
				.map_err(|x| anyhow!("QuickEntity error: {:?}", x))?
			})
			.await?;
		}
	}

	#[try_fn]
	#[context("Couldn't signal pin {pin} on {entity_id}")]
	pub async fn signal_pin(&self, entity_id: &str, tblu: &str, pin: &str, output: bool) -> Result<()> {
		self.send_request(SDKEditorRequest::SignalEntityPin {
			entity: EntitySelector::Game {
				id: entity_id.to_owned(),
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
