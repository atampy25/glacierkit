use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RepositoryItem {
	#[serde(rename = "ID_")]
	pub id: Uuid,

	#[serde(flatten)]
	pub data: IndexMap<String, Value>
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", content = "data")]
pub enum RepositoryItemInformation {
	NPC { name: String },
	Item { name: String },
	Weapon { name: String },
	Modifier { kind: String },
	MapArea { name: String },
	Outfit { name: String },
	Setpiece { traits: Vec<String> },
	DifficultyParameter { name: String },
	AmmoConfig { name: String },
	MagazineConfig { size: f64, tags: Vec<String> },
	AmmoBehaviour { name: String },
	MasteryItem { name: String },
	ScoreMultiplier { name: String },
	ItemBundle { name: String },
	ItemList,
	WeaponConfig,
	Unknown
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UnlockableItem {
	#[serde(rename = "Guid")]
	pub id: Uuid,

	#[serde(flatten)]
	pub data: IndexMap<String, Value>
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", content = "data")]
pub enum UnlockableInformation {
	Access { id: Option<String> },
	EvergreenMastery { id: Option<String> },
	Disguise { id: Option<String> },
	AgencyPickup { id: Option<String> },
	Weapon { id: Option<String> },
	Gear { id: Option<String> },
	Location { id: Option<String> },
	Package { id: Option<String> },
	LoadoutUnlock { id: Option<String> },
	Unknown { id: Option<String> }
}
