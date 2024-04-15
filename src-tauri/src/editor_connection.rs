use anyhow::{Context, Error, Result};
use fn_error_context::context;
use futures_util::{
	stream::{SplitSink},
	StreamExt
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{async_runtime::spawn, AppHandle};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use tryvial::try_fn;

use crate::{
	model::{GlobalRequest, Request},
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
	Unknown(f64),
	Known(String)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Vec3 {
	x: f64,
	y: f64,
	z: f64
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
	value: Value
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum EntityBaseDetails {
	GameEntity {
		id: String,
		tblu: String,
		name: Option<String>,

		#[serde(rename = "type")]
		ty: String
	},

	EditorEntity {
		id: String,
		byEditor: bool,
		name: Option<String>,

		#[serde(rename = "type")]
		ty: String
	}
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum EntityDetails {
	GameEntity {
		id: String,
		tblu: String,
		name: Option<String>,

		#[serde(rename = "type")]
		ty: String,

		parent: Option<EntitySelector>,
		transform: Option<Transform>,
		relativeTransform: Option<Transform>,
		properties: IndexMap<String, PropertyValue>,
		interfaces: Vec<String>
	},

	EditorEntity {
		id: String,
		byEditor: bool,
		name: Option<String>,

		#[serde(rename = "type")]
		ty: String,

		parent: Option<EntitySelector>,
		transform: Option<Transform>,
		relativeTransform: Option<Transform>,
		properties: IndexMap<String, PropertyValue>,
		interfaces: Vec<String>
	}
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

	EntityListResponse {
		entities: Vec<EntityBaseDetails>
	},

	EntityDetailsResponse {
		entity: EntityDetails
	},

	HitmanEntityResponse {
		entity: EntityDetails
	},

	CameraEntityResponse {
		entity: EntityDetails
	},

	EntityTreeRebuilt
}

pub struct EditorConnection {
	pub ws: Option<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>
}

impl EditorConnection {
	#[try_fn]
	#[context("Couldn't connect to editor server")]
	pub async fn connect(&mut self, app: &AppHandle) -> Result<()> {
		let (ws_stream, _) = connect_async("ws://localhost:46735")
			.await
			.context("Couldn't connect to WebSocket server")?;

		let (write, read) = ws_stream.split();

		let app = app.clone();

		spawn(async move {
			read.for_each(|msg| async {
				if let Err::<_, Error>(e) = try {
					let msg = msg.context("Couldn't receive message from WebSocket")?;

					match msg {
						Message::Ping(_) => {}
						Message::Pong(_) => {}
						Message::Close(_) => {}

						_ => {
							let msg = msg.to_text().context("Couldn't convert message to text")?;
							let msg: SDKEditorEvent = serde_json::from_str(msg).context("Couldn't parse message")?;

							match msg {
								SDKEditorEvent::Welcome => {
									println!("Connected to editor server");
								}

								SDKEditorEvent::Error { message } => {
									eprintln!("Error from editor server: {}", message);
								}

								_ => {
									println!("Received message: {:?}", msg);
								}
							}
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
			})
			.await;
		});

		self.ws = Some(write);
	}

	pub fn is_connected(&self) -> bool {
		self.ws.is_some()
	}
}
