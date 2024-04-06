use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RepositoryItem {
	#[serde(rename = "ID_")]
	pub id: Uuid,

	#[serde(flatten)]
	pub data: HashMap<String, Value>
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", content = "data")]
pub enum RepositoryItemInformation {
	NPC { name: String },
	Item { name: String },
	Weapon { name: String },
	Modifier { kind: Option<String> },
	StartingLocation { name: String },
	PersistentBool { name: String, id: String },
	Outfit { name: String },
	Setpiece { traits: Vec<String> },
	AgencyPickup { name: String },
	DifficultyParameter { name: String }
}
