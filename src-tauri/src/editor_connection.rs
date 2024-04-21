use std::sync::{
	atomic::{AtomicBool, Ordering},
	Arc
};

use anyhow::{anyhow, Context, Error, Result};
use fn_error_context::context;
use futures_util::{stream::SplitSink, SinkExt, StreamExt};
use indexmap::IndexMap;
use quickentity_rs::{convert_qn_property_value_to_rt, qn_structs::Property};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{async_runtime::spawn, AppHandle, Manager};
use tokio::{
	net::TcpStream,
	sync::{broadcast, Mutex}
};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use tryvial::try_fn;

use crate::{
	model::{AppState, EditorData, EditorRequest, EntityEditorRequest, EntityTreeRequest, GlobalRequest, Request},
	send_request
};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum EntitySelector {
	GameEntity { id: String, tblu: String },
	EditorEntity { id: String, byEditor: bool }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum PropertyID {
	Unknown(i32),
	Known(String)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Vec3 {
	pub x: f64,
	pub y: f64,
	pub z: f64
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Rotation {
	yaw: f64,
	pitch: f64,
	roll: f64
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Transform {
	position: Vec3,
	rotation: Rotation,
	scale: Vec3
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QNTransform {
	pub position: Vec3,
	pub rotation: Vec3,
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
		editor_only: bool
	},

	GetEntityDetails {
		entity: EntitySelector
	},

	GetHitmanEntity,

	GetCameraEntity,

	RebuildEntityTree
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PropertyValue {
	#[serde(rename = "type")]
	property_type: String,
	data: Value
}

/// No support for editor entities currently, but that's fine as they don't exist anyway
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EntityBaseDetails {
	id: String,
	tblu: String,
	name: Option<String>,

	#[serde(rename = "type")]
	ty: String
}

/// No support for editor entities currently, but that's fine as they don't exist anyway
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EntityDetails {
	id: String,
	tblu: String,
	name: Option<String>,

	#[serde(rename = "type")]
	ty: String,

	parent: Option<EntitySelector>,
	transform: Option<Transform>,
	relative_transform: Option<Transform>,
	properties: IndexMap<String, PropertyValue>,
	interfaces: Vec<String>
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum SDKEditorEvent {
	Welcome,

	Error {
		message: String
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
		entities: Vec<EntityBaseDetails>
	},

	EntityDetails {
		entity: EntityDetails
	},

	HitmanEntity {
		entity: EntityDetails
	},

	CameraEntity {
		entity: EntityDetails
	},

	EntityTreeRebuilt
}

pub struct EditorConnection {
	sender: Arc<Mutex<Option<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>>,
	events: broadcast::Sender<SDKEditorEvent>,
	entity_tree_loaded: Arc<AtomicBool>,
	app: AppHandle
}

impl EditorConnection {
	pub fn new(app: AppHandle) -> Self {
		let (sender, _) = broadcast::channel(32);

		Self {
			sender: Mutex::new(None).into(),
			events: sender,
			entity_tree_loaded: AtomicBool::new(false).into(),
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
					if let Ok(msg) = msg {
						if let Err::<_, Error>(e) = try {
							match msg {
								Message::Ping(_) => {}
								Message::Pong(_) => {}

								Message::Close(_) => {
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
											)?;
										}
									}
								}

								_ => {
									let msg = msg.to_text().context("Couldn't convert message to text")?;

									// serde_json is apparently broken and will error if you don't deserialise as Value first
									let msg: Value =
										serde_json::from_str(msg).context("Couldn't parse message as JSON")?;

									let msg = serde_json::from_value(msg)
										.context("Couldn't parse message as SDKEditorEvent")?;

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
					} else {
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
							}
						}
					}
				})
				.await;
			});

			let mut receiver = self.events.subscribe();

			let entity_tree_loaded = self.entity_tree_loaded.clone();

			spawn(async move {
				loop {
					if let Ok(event) = receiver.recv().await {
						match event {
							SDKEditorEvent::EntityTreeRebuilt => {
								entity_tree_loaded.store(true, Ordering::SeqCst);
							}

							SDKEditorEvent::SceneClearing { .. } | SDKEditorEvent::SceneLoading { .. } => {
								entity_tree_loaded.store(false, Ordering::SeqCst);
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
				}
			}
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
					&SDKEditorRequest::RebuildEntityTree
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
			entity: EntitySelector::GameEntity {
				id: entity_id.to_owned(),
				tblu: tblu.to_owned()
			}
		})
		.await?;
	}

	#[try_fn]
	#[context("Couldn't get player transform")]
	pub async fn get_player_transform(&self) -> Result<QNTransform> {
		self.send_request(SDKEditorRequest::GetHitmanEntity).await?;

		let SDKEditorEvent::HitmanEntity { entity } = self
			.wait_for_event(|evt| matches!(evt, SDKEditorEvent::HitmanEntity { .. }))
			.await?
		else {
			unreachable!()
		};

		let transform = entity.transform.context("Returned hitman entity had no transform")?;

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
		self.send_request(SDKEditorRequest::GetCameraEntity).await?;

		let SDKEditorEvent::CameraEntity { entity } = self
			.wait_for_event(|evt| matches!(evt, SDKEditorEvent::CameraEntity { .. }))
			.await?
		else {
			unreachable!()
		};

		let transform = entity.transform.context("Returned camera entity had no transform")?;

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
		self.send_request(SDKEditorRequest::SetEntityProperty {
			entity: EntitySelector::GameEntity {
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
