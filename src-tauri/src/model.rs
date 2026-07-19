use std::{
	fmt::{Display, Formatter},
	fs,
	ops::Deref,
	path::{Path, PathBuf},
	sync::Arc
};

use anyhow::{Context, Result};
use arc_swap::{ArcSwap, ArcSwapOption};
use ecow::EcoString;
use glacier_commons::{
	game::GlacierGame,
	game_detection::GameInstall,
	metadata::{ReferenceFlags, ResourceType, RuntimeID}
};
use notify::RecommendedWatcher;
use notify_debouncer_full::FileIdMap;
use quickentity_rs::{
	entity::{Entity, EntityID, Ref, SubEntity, SubType},
	variant::{Transform, Variant}
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use tryvial::try_fn;
use uuid::Uuid;

use crate::{
	HashMap, ShardMap,
	editor_connection::EditorConnection,
	entity::{CopiedEntityData, ReverseReference},
	game::{Game, valid_game_path},
	geometry::SceneGeometry,
	ores_repo::{
		RepositoryItem, RepositoryItemInformation, RepositoryItemKind, UnlockableInformation, UnlockableItem,
		UnlockableKind
	},
	render::SceneRenderer
};

pub struct AppSettings {
	data_dir: PathBuf,
	settings: ArcSwap<SettingsInner>
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SettingsInner {
	pub game_path: Option<PathBuf>,
	pub custom_game_paths: Vec<PathBuf>,
	pub extract_modded_files: bool,
	pub colourblind_mode: bool,
	pub editor_connection: bool,
	pub seen_announcements: Vec<String>
}

impl Default for SettingsInner {
	fn default() -> Self {
		Self {
			game_path: None,
			custom_game_paths: vec![],
			extract_modded_files: false,
			colourblind_mode: false,
			editor_connection: true,
			seen_announcements: vec![]
		}
	}
}

macro_rules! impl_set {
	($fn_name:ident, $field:ident, $ty:ty) => {
		#[try_fn]
		pub fn $fn_name(&self, value: $ty) -> Result<()> {
			let mut handle = self.settings.load_full();
			let settings = Arc::make_mut(&mut handle);
			if settings.$field != value {
				settings.$field = value;
				self.settings.store(handle);
				self.persist()?;
			}
		}
	};
}

impl AppSettings {
	#[try_fn]
	pub fn load(data_dir: impl AsRef<Path>) -> Result<Self> {
		let data_dir = data_dir.as_ref();
		if let Ok(read) = fs::read(data_dir.join("settings.json"))
			&& let Ok(mut settings) = serde_json::from_slice::<SettingsInner>(&read)
		{
			settings.game_path = settings.game_path.filter(|path| valid_game_path(path));
			settings.custom_game_paths.retain(|path| valid_game_path(path));

			Self {
				data_dir: data_dir.to_owned(),
				settings: Arc::new(settings).into()
			}
		} else {
			let settings = Self {
				data_dir: data_dir.to_owned(),
				settings: Default::default()
			};

			fs::create_dir_all(data_dir).context("Couldn't create app data dir")?;
			fs::write(
				data_dir.join("settings.json"),
				serde_json::to_vec(settings.settings.load().deref())
					.context("Couldn't serialise default app settings")?
			)
			.context("Couldn't write default app settings")?;
			settings
		}
	}

	pub fn settings(&self) -> arc_swap::Guard<Arc<SettingsInner>> {
		self.settings.load()
	}

	#[try_fn]
	fn persist(&self) -> Result<()> {
		fs::write(
			self.data_dir.join("settings.json"),
			serde_json::to_vec(self.settings.load().deref()).context("Couldn't serialise app settings")?
		)
		.context("Couldn't write app settings")?;
	}

	/// Set the game path. Does not check if the path is valid.
	#[try_fn]
	pub fn set_game_path(&self, path: Option<PathBuf>) -> Result<()> {
		let mut handle = self.settings.load_full();
		let settings = Arc::make_mut(&mut handle);
		if settings.game_path != path {
			settings.game_path = path;
			self.settings.store(handle);
			self.persist()?;
		}
	}

	impl_set!(set_custom_game_paths, custom_game_paths, Vec<PathBuf>);
	impl_set!(set_extract_modded_files, extract_modded_files, bool);
	impl_set!(set_colourblind_mode, colourblind_mode, bool);
	impl_set!(set_editor_connection, editor_connection, bool);
	impl_set!(set_seen_announcements, seen_announcements, Vec<String>);
}

pub struct AppState {
	pub game_installs: Vec<GameInstall>,
	pub project: ArcSwapOption<Project>,
	pub tonytools_hash_list: ArcSwapOption<tonytools::hashlist::HashList>,
	pub fs_watcher: ArcSwapOption<notify_debouncer_full::Debouncer<RecommendedWatcher, FileIdMap>>,
	pub editor_states: Arc<ShardMap<Uuid, EditorState>>,

	/// Synchronises removals to prevent tasks trying to access the data of an editor that has been closed
	pub editor_removal: Arc<ShardMap<Uuid, tokio::sync::RwLock<()>>>,

	pub game: ArcSwapOption<Game>,

	pub editor_connection: EditorConnection,

	pub scene_renderer: SceneRenderer
}

pub struct EditorState {
	pub file: Option<PathBuf>,
	pub data: EditorData,

	/// For the frontend to access without costly serialisation/deserialisation; ID -> (MIME type, data)
	pub assets: ShardMap<Uuid, (EcoString, Vec<u8>)>
}

impl Default for EditorState {
	fn default() -> Self {
		Self {
			file: None,
			data: EditorData::Nil,
			assets: ShardMap::with_hasher(Default::default())
		}
	}
}

pub enum EditorData {
	Nil,
	ResourceOverview {
		hash: RuntimeID
	},
	Text {
		content: String,
		file_type: TextFileType
	},
	QNEntity {
		settings: QNEditorSettings,
		entity: Arc<Entity>
	},
	QNPatch {
		settings: QNEditorSettings,
		base: Arc<Entity>,
		current: Arc<Entity>
	},
	RepositoryPatch {
		base: Vec<RepositoryItem>,
		current: Vec<RepositoryItem>,
		patch_type: JsonPatchType
	},
	UnlockablesPatch {
		base: Vec<UnlockableItem>,
		current: Vec<UnlockableItem>,
		patch_type: JsonPatchType
	},
	ContentSearchResults {
		query: SearchQuery,
		results: Vec<(String, String, Option<String>)>
	}
}

#[derive(Type, Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct QNEditorSettings {
	pub show_reverse_parent_refs: bool,
	pub show_changes_from_original: bool
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Project {
	pub path: PathBuf,
	pub settings: ArcSwap<ProjectSettings>
}

#[derive(Type, Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSettings {
	pub custom_paths: Vec<String>
}

#[derive(Type, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(from = "HashProxy", into = "HashProxy")]
pub struct Hash(pub RuntimeID);

struct HashProxy(RuntimeID);

impl From<Hash> for HashProxy {
	fn from(value: Hash) -> Self {
		Self(value.0)
	}
}

impl From<HashProxy> for Hash {
	fn from(value: HashProxy) -> Self {
		Self(value.0)
	}
}

impl Serialize for HashProxy {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer
	{
		serializer.serialize_str(&self.0.to_hash())
	}
}

impl Type for HashProxy {
	fn definition(types: &mut specta::Types) -> specta::datatype::DataType {
		String::definition(types)
	}
}

impl<'de> Deserialize<'de> for HashProxy {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>
	{
		use serde::de::Error;

		String::deserialize(deserializer)?
			.parse()
			.map_err(D::Error::custom)
			.map(Self)
	}
}

#[derive(Type, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct GameBrowserEntry {
	pub hash: Hash,
	#[specta(type = Option<String>)]
	pub path: Option<EcoString>,
	#[specta(type = Option<String>)]
	pub hint: Option<EcoString>,
	pub resource_type: ResourceType,
	pub partition: (String, String),
	pub order: Option<u32>,
	pub extract_kinds: Vec<(String, ExtractKind)>
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
pub enum TextFileType {
	Json,
	ManifestJson,
	Xml,
	PlainText,
	Markdown
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", content = "data")]
pub enum EditorType {
	Nil,
	ResourceOverview,
	Text { file_type: TextFileType },
	QNEntity,
	QNPatch,
	RepositoryPatch { patch_type: JsonPatchType },
	UnlockablesPatch { patch_type: JsonPatchType },
	ContentSearchResults
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
pub enum JsonPatchType {
	MergePatch,
	JsonPatch
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", content = "data")]
pub enum EditorValidity {
	Valid,
	Invalid(String)
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PastableTemplate {
	pub name: String,
	pub icon: String,
	pub paste_data: CopiedEntityData
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PastableTemplateCategory {
	pub name: String,
	pub icon: String,
	pub templates: Vec<PastableTemplate>
}

#[derive(Type, Serialize, Deserialize, Clone, derive_more::Debug)]
#[serde(
	tag = "type",
	content = "data",
	rename_all = "camelCase",
	rename_all_fields = "camelCase"
)]
pub enum ResourceOverviewData {
	Generic,
	GenericData {
		asset_id: Uuid
	},
	Entity {
		root_entity_name: String,
		blueprint_hash: Hash,
		#[specta(type = Option<String>)]
		blueprint_path_or_hint: Option<EcoString>,
		#[specta(type = Option<(Value, Value)>)]
		#[debug(skip)]
		preview: Option<(SceneGeometry, HashMap<RuntimeID, Uuid>)>
	},
	GenericRL {
		#[debug(skip)]
		json: String
	},
	Json {
		#[debug(skip)]
		json: String
	},
	Xml {
		#[debug(skip)]
		xml: String
	},
	Image {
		asset_id: Uuid,
		texture_data: Option<(String, String, Option<String>)>
	},
	Audio {
		asset_id: Option<Uuid>
	},
	Mesh {
		asset_id: Uuid,
		bounding_box: [f32; 6]
	},
	MultiAudio {
		name: String,

		#[debug(skip)]
		audios: Vec<(String, Option<Uuid>)>
	},
	Repository,
	Unlockables,
	HMLanguages {
		#[debug(skip)]
		json: String
	},
	LocalisedLine {
		key: String,
		languages: Vec<(String, String)>
	},
	MaterialInstance {
		#[debug(skip)]
		json: String
	},
	MaterialEntity {
		#[debug(skip)]
		json: String
	},
	SoundDefinitions {
		#[debug(skip)]
		json: String
	},
	BehaviorTree {
		#[debug(skip)]
		pseudocode: String
	},
	Error {
		message: String
	}
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ResourceChangelogEntry {
	pub operation: ResourceChangelogOperation,
	pub partition: String,
	pub patch: String,
	pub description: String
}

#[derive(Type, Serialize, Deserialize, Clone, Debug, PartialEq, PartialOrd, Eq, Ord)]
pub enum ResourceChangelogOperation {
	Delete,
	Init,
	Edit
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
pub enum SearchFilter {
	All,
	Templates,
	Classes,
	Models,
	Textures,
	Sound
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
pub enum SearchSort {
	Size
}

#[derive(Type, Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Dynamics {
	pub announcements: Vec<Announcement>
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Announcement {
	pub id: String,
	pub kind: AnnouncementKind,
	pub title: String,
	pub description: String,
	pub persistent: bool,
	pub until: Option<u32>
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub enum AnnouncementKind {
	Info,
	Success,
	Warning,
	Error
}

nesting::nest! {
	#![derive(Type, Serialize, Deserialize, Clone, derive_more::Debug)]
	#![enums(serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "type", content = "data"))]
	#![structs(serde(rename_all = "camelCase"))]
	#[derive(PartialEq, Eq, Hash)]
	pub enum ExtractKind {
		Raw,

		QN,

		TBLUAsRaw,

		TBLUAsBin1Json,

		Bin1Json,

		Image {
			format: ImageExtractFormat
		},

		#[derive(PartialEq, Eq, Hash)]
		pub enum ImageExtractFormat {
			Png,
			Jpeg,
			Tga,
			Dds
		}

		Ogg,

		MultiOgg,

		HMLanguages,

		MaterialInstance,

		MaterialEntity,

		SoundDefs,

		Obj,

		Glb,

		Texture {
			format: TextureExtractFormat
		},

		#[derive(PartialEq, Eq, Hash)]
		pub enum TextureExtractFormat {
			Dds,
			Png,
			Tga
		}

		Pseudocode
	}
}

impl Display for ExtractKind {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			ExtractKind::Raw => write!(f, "as file"),
			ExtractKind::QN => write!(f, "as QuickEntity JSON"),
			ExtractKind::TBLUAsRaw => write!(f, "blueprint as file"),
			ExtractKind::TBLUAsBin1Json => write!(f, "blueprint as JSON"),
			ExtractKind::Bin1Json => write!(f, "as JSON"),
			ExtractKind::Image { format } => match format {
				ImageExtractFormat::Png => write!(f, "as PNG"),
				ImageExtractFormat::Jpeg => write!(f, "as JPEG"),
				ImageExtractFormat::Tga => write!(f, "as TGA"),
				ImageExtractFormat::Dds => write!(f, "as DDS")
			},
			ExtractKind::Ogg => write!(f, "as OGG"),
			ExtractKind::MultiOgg => write!(f, "all as OGGs"),
			ExtractKind::HMLanguages => write!(f, "as JSON"),
			ExtractKind::MaterialInstance => write!(f, "as JSON"),
			ExtractKind::MaterialEntity => write!(f, "as JSON"),
			ExtractKind::SoundDefs => write!(f, "as JSON"),
			ExtractKind::Obj => write!(f, "as OBJ"),
			ExtractKind::Glb => write!(f, "as GLB"),
			ExtractKind::Texture { format } => match format {
				TextureExtractFormat::Png => write!(f, "as PNG"),
				TextureExtractFormat::Tga => write!(f, "as TGA"),
				TextureExtractFormat::Dds => write!(f, "as DDS")
			},
			ExtractKind::Pseudocode => write!(f, "as TXT")
		}
	}
}

nesting::nest! {
	#![derive(Type, Serialize, Deserialize, Clone, derive_more::Debug)]
	#![enums(serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "type", content = "data"))]
	#![structs(serde(rename_all = "camelCase"))]
	pub enum SearchQuery {
		/// The query must be contained exactly as written.
		Raw(#[specta(type = String)] EcoString)

		/// All components of the query (separated by spaces, optionally double-quoted) must be contained somewhere.
		Simple(#[specta(type = String)] EcoString),

		/// The query as regex must match.
		Regex(#[specta(type = String)] EcoString)
	}
}

nesting::nest! {
	#![derive(Type, Serialize, Deserialize, Clone, derive_more::Debug)]
	#![enums(serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "type", content = "data"))]
	#![structs(serde(rename_all = "camelCase"))]
	pub enum Event {
		Tool(ToolEvent)

		pub enum ToolEvent {
			FileBrowser(FileBrowserEvent)
			pub enum FileBrowserEvent {
				Select {
					path: Option<PathBuf>
				},

				Create {
					path: PathBuf,
					is_folder: bool
				},

				Delete {
					path: PathBuf
				},

				Rename {
					old_path: PathBuf,
					new_path: PathBuf
				},

				NormaliseQNFile {
					path: PathBuf
				},

				ConvertEntityToPatch {
					path: PathBuf
				},

				ConvertPatchToEntity {
					path: PathBuf
				},

				ConvertRepoPatchToMergePatch {
					path: PathBuf
				},

				ConvertRepoPatchToJsonPatch {
					path: PathBuf
				},

				ConvertUnlockablesPatchToMergePatch {
					path: PathBuf
				},

				ConvertUnlockablesPatchToJsonPatch {
					path: PathBuf
				}
			}

			GameBrowser(GameBrowserEvent)
			pub enum GameBrowserEvent {
				Select {
					resource: Hash
				},
				Search {
					query: SearchQuery,
					filter: SearchFilter,
					sort: Option<(SearchSort, bool)>
				},
				OpenInEditor {
					resource: Hash
				},
				Extract {
					resource: Hash,
					kind: ExtractKind
				},
				MassExtract {
					resources: Vec<Hash>,
					kinds: Vec<(ResourceType, ExtractKind)>,
					use_paths: bool
				}
			}

			Settings(SettingsEvent)
			pub enum SettingsEvent {
				Initialise,

				ChangeGameInstall { path: Option<PathBuf> },
				RemoveCustomGamePath { path: PathBuf },
				ChangeExtractModdedFiles { value: bool },
				ChangeColourblindMode { value: bool },
				ChangeEditorConnection { value: bool },

				ChangeCustomPaths {
					#[debug(skip)]
					value: Vec<String>
				}
			}

			ContentSearch(ContentSearchEvent)
			pub enum ContentSearchEvent {
				Search {
					query: SearchQuery,
					resource_types: Vec<ResourceType>,
					partitions_to_search: Vec<String>
				}
			}
		}

		Editor(EditorEvent)

		pub struct EditorEvent {
			pub editor: Uuid,
			pub data: EditorEventData

			pub enum EditorEventData {
				Text(TextEditorEvent)
				pub enum TextEditorEvent {
					Initialise,

					UpdateContent {
						#[debug(skip)]
						content: String
					}
				}

				Entity(EntityEditorEvent)
				pub enum EntityEditorEvent {
					General(EntityGeneralEvent)
					pub enum EntityGeneralEvent {
						SetShowReverseParentRefs {
							show_reverse_parent_refs: bool
						},

						SetShowChangesFromOriginal {
							show_changes_from_original: bool
						},

						StartSceneRenderer
					}

					Tree(EntityTreeEvent)
					pub enum EntityTreeEvent {
						Initialise,

						Select {
							id: EntityID
						},

						Create {
							id: EntityID,
							content: Box<SubEntity>
						},

						Delete {
							id: EntityID
						},

						Rename {
							id: EntityID,
							new_name: String
						},

						Reparent {
							id: EntityID,
							new_parent: Option<EntityID>
						},

						Copy {
							id: EntityID
						},

						Paste {
							parent_id: String
						},

						Search {
							query: SearchQuery
						},

						ShowHelpMenu {
							entity_id: EntityID
						},

						UseTemplate {
							parent_id: String,

							#[debug(skip)]
							template: CopiedEntityData
						},

						AddGameBrowserItem {
							parent_id: String,
							file: Hash
						},

						SelectEntityInEditor {
							entity_id: EntityID
						},

						MoveEntityToPlayer {
							entity_id: EntityID
						},

						RotateEntityAsPlayer {
							entity_id: EntityID
						},

						MoveEntityToCamera {
							entity_id: EntityID
						},

						RotateEntityAsCamera {
							entity_id: EntityID
						},

						RestoreToOriginal {
							entity_id: EntityID
						}
					}

					Monaco(EntityMonacoEvent)
					pub enum EntityMonacoEvent {
						UpdateContent {
							entity_id: EntityID,

							#[debug(skip)]
							content: String
						},

						FollowReference {
							reference: EntityID
						},

						OpenFactory {
							factory: RuntimeID
						},

						SignalPin {
							entity_id: EntityID,
							pin: String,
							output: bool
						},

						OpenResourceOverview {
							resource: RuntimeID
						}
					}

					MetaPane(EntityMetaPaneEvent)
					pub enum EntityMetaPaneEvent {
						JumpToReference {
							reference: EntityID
						},

						SetNotes {
							entity_id: EntityID,
							notes: String
						}
					}

					Metadata(EntityMetadataEvent)
					pub enum EntityMetadataEvent {
						Initialise,

						SetFactory {
							factory: RuntimeID
						},

						SetBlueprint {
							blueprint: RuntimeID
						},

						SetRootEntity {
							root_entity: EntityID
						},

						SetSubType {
							sub_type: SubType
						},

						SetExternalScenes {
							#[debug(skip)]
							external_scenes: Vec<RuntimeID>
						}
					}

					Overrides(EntityOverridesEvent)
					pub enum EntityOverridesEvent {
						Initialise,

						UpdatePropertyOverrides {
							#[debug(skip)]
							content: String
						},

						UpdateOverrideDeletes {
							#[debug(skip)]
							content: String
						},

						UpdatePinConnectionOverrides {
							#[debug(skip)]
							content: String
						},

						UpdatePinConnectionOverrideDeletes {
							#[debug(skip)]
							content: String
						}
					}
				}

				ResourceOverview(ResourceOverviewEvent)
				pub enum ResourceOverviewEvent {
					Initialise,

					FollowDependency {
						new_hash: RuntimeID
					},

					FollowDependencyInNewTab {
						hash: RuntimeID
					},

					OpenInEditor,

					ShowEntityPreview,

					Extract {
						kind: ExtractKind
					},

					ExtractSpecificMultiOgg {
						index: u32
					}
				}

				RepositoryPatch(RepositoryPatchEditorEvent)
				pub enum RepositoryPatchEditorEvent {
					Initialise,

					CreateRepositoryItem,

					ResetModifications {
						item: Uuid
					},

					ModifyItem {
						item: Uuid,
						data: String
					},

					SelectItem {
						item: Uuid
					},

					Search {
						query: SearchQuery,
						ty: Option<RepositoryItemKind>
					}
				}

				UnlockablesPatch(UnlockablesPatchEditorEvent)
				pub enum UnlockablesPatchEditorEvent {
					Initialise,

					CreateUnlockable,

					ResetModifications {
						unlockable: Uuid
					},

					ModifyUnlockable {
						unlockable: Uuid,
						data: String
					},

					SelectUnlockable {
						unlockable: Uuid
					},

					Search {
						query: SearchQuery,
						ty: Option<UnlockableKind>
					}
				}

				ContentSearchResults(ContentSearchResultsEvent)
				pub enum ContentSearchResultsEvent {
					Initialise,

					OpenResourceOverview {
						hash: Hash
					}
				}
			}
		}

		Global(GlobalEvent)
		pub enum GlobalEvent {
			SetSeenAnnouncements(Vec<String>),
			LoadWorkspace(PathBuf),
			SelectAndOpenFile,
			SelectTab(Option<Uuid>),
			RemoveTab(Uuid),
			SaveTab(Uuid),
			UploadLogAndReport(String),
			UploadLastPanic,
			ClearLastPanic
		}

		EditorConnection(EditorConnectionEvent)
		pub enum EditorConnectionEvent {
			EntitySelected {
				id: EntityID,
				tblu: Hash
			},

			EntityTransformUpdated {
				id: EntityID,
				tblu: Hash,
				transform: Transform
			},

			EntityPropertyChanged {
				id: EntityID,
				tblu: Hash,
				property_name: String,
				property_value: Variant
			}
		}

		SceneRenderer(SceneRendererEvent)
		pub enum SceneRendererEvent {
			EntitiesSelected {
				entities: Vec<(RuntimeID, EntityID)>
			},

			EntityPropertyChanged {
				entity: (RuntimeID, EntityID),
				#[specta(type = String)]
				property: EcoString,
				value: Variant
			}
		}
	}
}

nesting::nest! {
	#![derive(Type, Serialize, Deserialize, Clone, derive_more::Debug)]
	#![enums(serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "type", content = "data"))]
	pub enum Request {
		Tool(ToolRequest)
		pub enum ToolRequest {
			FileBrowser(FileBrowserRequest)
			pub enum FileBrowserRequest {
				Create {
					path: PathBuf,
					is_folder: bool
				},

				Delete {
					path: PathBuf
				},

				Rename {
					old_path: PathBuf,
					new_path: PathBuf
				},

				BeginRename {
					old_path: PathBuf
				},

				FinishRename {
					new_path: PathBuf
				},

				Select {
					path: Option<PathBuf>
				},

				NewTree {
					base_path: PathBuf,

					/// Relative path, is folder
					#[debug(skip)]
					files: Vec<(PathBuf, bool)>
				}
			}

			GameBrowser(GameBrowserRequest)
			pub enum GameBrowserRequest {
				SetEnabled {
					enabled: bool
				},

				NewTree {
					game_description: String,

					#[debug(skip)]
					entries: Vec<GameBrowserEntry>
				}
			}

			Settings(SettingsRequest)
			pub enum SettingsRequest {
				Initialise {
					game_installs: Vec<GameInstall>,
					settings: SettingsInner
				},

				ChangeProjectSettings {
					settings: ProjectSettings
				}
			}

			ContentSearch(ContentSearchRequest)
			pub enum ContentSearchRequest {
				SetEnabled {
					enabled: bool
				},

				SetPartitions {
					/// Name, ID
					partitions: Vec<(String, String)>
				}
			}
		}

		Editor(EditorRequest)
		pub struct EditorRequest {
			pub editor: Uuid,
			pub data: EditorRequestData

			pub enum EditorRequestData {
				Text(TextEditorRequest)
				pub enum TextEditorRequest {
					ReplaceContent {
						#[debug(skip)]
						content: String
					},

					SetFileType {
						file_type: TextFileType
					}
				}

				Entity(EntityEditorRequest)
				pub enum EntityEditorRequest {
					General(EntityGeneralRequest)
					pub enum EntityGeneralRequest {
						SetIsPatchEditor {
							is_patch_editor: bool
						}
					}

					Tree(EntityTreeRequest)
					pub enum EntityTreeRequest {
						/// Will trigger a Select event from the tree - ensure this doesn't end up in a loop
						Select {
							id: Option<EntityID>
						},

						NewTree {
							/// ID, parent, name, factory, has reverse parent refs
							#[debug(skip)]
							#[specta(type = Vec<(EntityID, Option<Ref>, String, RuntimeID, bool)>)]
							entities: Vec<(EntityID, Option<Ref>, EcoString, RuntimeID, bool)>
						},

						/// Instructs the frontend to take the list of new entities, add any new ones and update any ones that already exist (by ID) with the new information.
						/// This is used for pasting, and for ensuring that icons/parent status/name are updated when a sub-entity is updated.
						NewItems {
							/// ID, parent, name, factory, has reverse parent refs
							#[debug(skip)]
							#[specta(type = Vec<(EntityID, Option<Ref>, String, RuntimeID, bool)>)]
							new_entities: Vec<(EntityID, Option<Ref>, EcoString, RuntimeID, bool)>
						},

						SearchResults {
							/// The IDs of the entities matching the query
							#[debug(skip)]
							results: Vec<EntityID>
						},

						ShowHelpMenu {
							factory: RuntimeID,
							#[specta(type = Vec<String>)]
							input_pins: Vec<EcoString>,
							#[specta(type = Vec<String>)]
							output_pins: Vec<EcoString>,
							default_properties_json: String,
							#[specta(type = Vec<String>)]
							subsets: Vec<EcoString>
						},

						SetTemplates {
							#[debug(skip)]
							templates: Vec<PastableTemplateCategory>
						},

						SetEditorConnectionAvailable {
							editor_connection_available: bool
						},

						SetShowDiff {
							show_diff: bool
						},

						SetDiffInfo {
							#[debug(skip)]
							new: Vec<EntityID>,

							#[debug(skip)]
							modified: Vec<EntityID>,

							#[debug(skip)]
							#[specta(type = Vec<(EntityID, Option<Ref>, String, RuntimeID, bool)>)]
							removed: Vec<(EntityID, Option<Ref>, EcoString, RuntimeID, bool)>
						}
					}

					Monaco(EntityMonacoRequest)
					pub enum EntityMonacoRequest {
						DeselectIfSelected {
							entity_ids: Vec<EntityID>
						},

						ReplaceContent {
							entity_id: EntityID,

							#[debug(skip)]
							content: String
						},

						ReplaceContentIfSameEntityID {
							entity_id: EntityID,

							#[debug(skip)]
							content: String
						},

						UpdateIntellisense {
							entity_id: EntityID,
							#[specta(type = Vec<(String, Variant, bool)>)]
							properties: Vec<(EcoString, Variant, bool)>,
							#[specta(type = Vec<String>)]
							input_pins: Vec<EcoString>,
							#[specta(type = Vec<String>)]
							output_pins: Vec<EcoString>,
							#[specta(type = Vec<String>)]
							subsets: Vec<EcoString>
						},

						UpdateDecorationsAndMonacoInfo {
							entity_id: EntityID,

							#[debug(skip)]
							decorations: Vec<(String, String)>,

							#[debug(skip)]
							local_ref_entity_ids: Vec<EntityID>
						},

						UpdateValidity {
							validity: EditorValidity
						},

						SetEditorConnected {
							connected: bool
						}
					}

					MetaPane(EntityMetaPaneRequest)
					pub enum EntityMetaPaneRequest {
						SetReverseRefs {
							#[debug(skip)]
							#[specta(type = std::collections::HashMap<EntityID, String>)]
							entity_names: HashMap<EntityID, EcoString>,

							reverse_refs: Vec<ReverseReference>
						},

						SetNotes {
							entity_id: EntityID,

							#[specta(type = String)]
							notes: EcoString
						}
					}

					Metadata(EntityMetadataRequest)
					pub enum EntityMetadataRequest {
						Initialise {
							factory: RuntimeID,
							blueprint: RuntimeID,
							root_entity: EntityID,
							sub_type: SubType,

							#[debug(skip)]
							external_scenes: Vec<RuntimeID>
						},

						SetHashModificationAllowed {
							hash_modification_allowed: bool
						},

						SetFactory {
							factory: RuntimeID
						},

						SetBlueprint {
							blueprint: RuntimeID
						}
					}

					Overrides(EntityOverridesRequest)
					pub enum EntityOverridesRequest {
						Initialise {
							#[debug(skip)]
							property_overrides: String,

							#[debug(skip)]
							override_deletes: String,

							#[debug(skip)]
							pin_connection_overrides: String,

							#[debug(skip)]
							pin_connection_override_deletes: String
						},

						UpdateDecorations {
							#[debug(skip)]
							decorations: Vec<(String, String)>,
						}
					}
				}

				ResourceOverview(ResourceOverviewRequest)
				pub enum ResourceOverviewRequest {
					Initialise {
						hash: Hash,
						filetype: String,
						chunk_patch: String,

						#[specta(type = Option<String>)]
						path_or_hint: Option<EcoString>,

						size: u32,

						/// Hash, type, path/hint, flags, is actually in current game version
						#[debug(skip)]
						#[specta(type = Vec<(String, String, Option<String>, ReferenceFlags, bool)>)]
						dependencies: Vec<(Hash, Option<ResourceType>, Option<EcoString>, ReferenceFlags, bool)>,

						/// Hash, type, path/hint
						#[debug(skip)]
						#[specta(type = Vec<(String, String, Option<String>)>)]
						reverse_dependencies: Vec<(Hash, ResourceType, Option<EcoString>)>,

						changelog: Vec<ResourceChangelogEntry>,

						data: ResourceOverviewData,

						extract_kinds: Vec<(String, ExtractKind)>
					}
				}

				RepositoryPatch(RepositoryPatchEditorRequest)
				pub enum RepositoryPatchEditorRequest {
					SetRepositoryItems {
						#[debug(skip)]
						items: Vec<(Uuid, RepositoryItemInformation)>
					},

					SetModifiedRepositoryItems {
						modified: Vec<Uuid>
					},

					AddNewRepositoryItem {
						new_item: (Uuid, RepositoryItemInformation)
					},

					RemoveRepositoryItem {
						item: Uuid
					},

					SetMonacoContent {
						item: Uuid,
						orig_data: String,
						data: String
					},

					DeselectMonaco,

					ModifyItemInformation {
						item: Uuid,
						info: RepositoryItemInformation
					},

					SearchResults {
						#[debug(skip)]
						items: Vec<Uuid>
					}
				}

				UnlockablesPatch(UnlockablesPatchEditorRequest)
				pub enum UnlockablesPatchEditorRequest {
					SetUnlockables {
						#[debug(skip)]
						unlockables: Vec<(Uuid, UnlockableInformation)>
					},

					SetModifiedUnlockables {
						modified: Vec<Uuid>
					},

					AddNewUnlockable {
						new_unlockable: (Uuid, UnlockableInformation)
					},

					RemoveUnlockable {
						unlockable: Uuid
					},

					SetMonacoContent {
						unlockable: Uuid,
						orig_data: String,
						data: String
					},

					DeselectMonaco,

					ModifyUnlockableInformation {
						unlockable: Uuid,
						info: UnlockableInformation
					},

					SearchResults {
						#[debug(skip)]
						items: Vec<Uuid>
					}
				}

				ContentSearchResults(ContentSearchResultsRequest)
				pub enum ContentSearchResultsRequest {
					Initialise {
						query: String,

						/// Hash, type, path/hint
						#[debug(skip)]
						results: Vec<(String, String, Option<String>)>
					}
				}
			}
		}

		Tab(TabRequest)
		pub struct TabRequest {
			pub tab: Uuid,
			pub data: TabRequestData

			pub enum TabRequestData {
				Create {
					name: String,
					editor_type: EditorType
				},
				Rename {
					new_name: String
				},
				Select,
				SetUnsaved {
					unsaved: bool
				},
				Remove
			}
		}

		Global(GlobalRequest)
		pub enum GlobalRequest {
			ErrorReport {
				error: String
			},
			SetWindowTitle {
				title: String
			},
			InitialiseDynamics {
				dynamics: Dynamics,
				seen_announcements: Vec<String>
			},
			ComputeJSONPatchAndSave {
				base: Value,
				current: Value,
				save_path: PathBuf,
				id: RuntimeID
			},
			RequestLastPanicUpload,
			LogUploadRejected,
			SetGameVersion {
				version: Option<GlacierGame>,

				#[debug(skip)]
				#[specta(type = std::collections::HashMap<String, Vec<String>>)]
				enums: HashMap<String, Vec<String>>
			}
		}
	}
}
