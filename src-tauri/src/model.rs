use std::{path::PathBuf, sync::Arc};

use arc_swap::{ArcSwap, ArcSwapOption};

use dashmap::DashMap;
use derivative::Derivative;
use hashbrown::HashMap;
use hitman_commons::{
	game_detection::GameInstall,
	hash_list::HashList,
	metadata::{ResourceType, RuntimeID}
};
use notify::RecommendedWatcher;
use notify_debouncer_full::FileIdMap;
use quickentity_rs::qn_structs::{Entity, Ref, SubEntity, SubType};
use rpkg_rs::resource::partition_manager::PartitionManager;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use structstruck::strike;
use uuid::Uuid;

use crate::{
	editor_connection::{EditorConnection, QNTransform},
	entity::{CopiedEntityData, ReverseReference},
	intellisense::Intellisense,
	ores_repo::{RepositoryItem, RepositoryItemInformation, UnlockableInformation, UnlockableItem}
};

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
	pub extract_modded_files: bool,
	pub game_install: Option<PathBuf>,
	pub colourblind_mode: bool,
	pub editor_connection: bool,
	pub seen_announcements: Vec<String>
}

impl Default for AppSettings {
	fn default() -> Self {
		Self {
			extract_modded_files: false,
			game_install: None,
			colourblind_mode: false,
			editor_connection: true,
			seen_announcements: vec![]
		}
	}
}

pub struct AppState {
	pub game_installs: Vec<GameInstall>,
	pub project: ArcSwapOption<Project>,
	pub hash_list: ArcSwapOption<HashList>,
	pub tonytools_hash_list: ArcSwapOption<tonytools::hashlist::HashList>,
	pub fs_watcher: ArcSwapOption<notify_debouncer_full::Debouncer<RecommendedWatcher, FileIdMap>>,
	pub editor_states: Arc<DashMap<Uuid, EditorState>>,
	pub game_files: ArcSwapOption<PartitionManager>,

	/// Resource -> Resources which depend on it
	pub resource_reverse_dependencies: ArcSwapOption<HashMap<RuntimeID, Vec<RuntimeID>>>,

	pub cached_entities: Arc<DashMap<RuntimeID, Entity>>,
	pub repository: ArcSwapOption<Vec<RepositoryItem>>,
	pub intellisense: ArcSwapOption<Intellisense>,

	pub editor_connection: EditorConnection
}

#[derive(Debug)]
pub struct EditorState {
	pub file: Option<PathBuf>,
	pub data: EditorData
}

#[derive(Debug, Clone)]
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
		settings: EphemeralQNSettings,
		entity: Box<Entity>
	},
	QNPatch {
		settings: EphemeralQNSettings,
		base: Box<Entity>,
		current: Box<Entity>
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
		results: Vec<(String, String, Option<String>)>
	}
}

#[derive(Type, Serialize, Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EphemeralQNSettings {
	pub show_reverse_parent_refs: bool,
	pub show_changes_from_original: bool
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Project {
	pub path: PathBuf,
	pub settings: ArcSwap<ProjectSettings>
}

#[derive(Type, Serialize, Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSettings {
	pub custom_paths: Vec<String>
}

#[derive(Type, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct GameBrowserEntry {
	pub hash: RuntimeID,
	pub path: Option<String>,
	pub hint: Option<String>,
	pub filetype: ResourceType,
	pub partition: (String, String)
}

#[derive(Type, Serialize, Deserialize, Clone, Debug)]
pub enum TextFileType {
	Json,
	ManifestJson,
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

#[derive(Type, Serialize, Deserialize, Clone, Derivative)]
#[serde(tag = "type", content = "data")]
#[derivative(Debug)]
pub enum ResourceOverviewData {
	Generic,
	Entity {
		blueprint_hash: String,
		blueprint_path_or_hint: Option<String>
	},
	GenericRL {
		json: String
	},
	Json {
		json: String
	},
	Ores {
		json: String
	},
	Image {
		image_path: PathBuf,
		dds_data: Option<(String, String)>
	},
	Audio {
		wav_path: PathBuf
	},
	Mesh {
		#[derivative(Debug = "ignore")]
		obj: String,
		bounding_box: [f32; 6]
	},
	MultiAudio {
		name: String,
		wav_paths: Vec<(String, PathBuf)>
	},
	Repository,
	Unlockables,
	HMLanguages {
		json: String
	},
	LocalisedLine {
		languages: Vec<(String, String)>
	},
	MaterialInstance {
		json: String
	},
	MaterialEntity {
		json: String
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

strike! {
	#[strikethrough[derive(Type, Serialize, Deserialize, Clone, Debug)]]
	#[strikethrough[serde(rename_all = "camelCase", tag = "type", content = "data")]]
	pub enum Event {
		Tool(pub enum ToolEvent {
			FileBrowser(pub enum FileBrowserEvent {
				Select(Option<PathBuf>),

				Create {
					path: PathBuf,
					is_folder: bool
				},

				Delete(PathBuf),

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
			}),

			GameBrowser(pub enum GameBrowserEvent {
				Select(RuntimeID),
				Search(String, SearchFilter),
				OpenInEditor(RuntimeID)
			}),

			Settings(pub enum SettingsEvent {
				Initialise,

				ChangeGameInstall(Option<PathBuf>),
				ChangeExtractModdedFiles(bool),
				ChangeColourblind(bool),
				ChangeEditorConnection(bool),

				ChangeCustomPaths(Vec<String>)
			}),

			ContentSearch(pub enum ContentSearchEvent {
				Search(String, Vec<String>, bool, Vec<String>)
			})
		}),

		Editor(pub enum EditorEvent {
			Text(pub enum TextEditorEvent {
				Initialise {
					id: Uuid
				},

				UpdateContent {
					id: Uuid,
					content: String
				}
			}),

			Entity(pub enum EntityEditorEvent {
				General(pub enum EntityGeneralEvent {
					SetShowReverseParentRefs {
						editor_id: Uuid,
						show_reverse_parent_refs: bool
					},

					SetShowChangesFromOriginal {
						editor_id: Uuid,
						show_changes_from_original: bool
					}
				}),

				Tree(pub enum EntityTreeEvent {
					Initialise {
						editor_id: Uuid
					},

					Select {
						editor_id: Uuid,
						id: String
					},

					Create {
						editor_id: Uuid,
						id: String,
						content: SubEntity
					},

					Delete {
						editor_id: Uuid,
						id: String
					},

					Rename {
						editor_id: Uuid,
						id: String,
						new_name: String
					},

					Reparent {
						editor_id: Uuid,
						id: String,
						new_parent: Ref
					},

					Copy {
						editor_id: Uuid,
						id: String
					},

					Paste {
						editor_id: Uuid,
						parent_id: String
					},

					Search {
						editor_id: Uuid,
						query: String
					},

					ShowHelpMenu {
						editor_id: Uuid,
						entity_id: String
					},

					UseTemplate {
						editor_id: Uuid,
						parent_id: String,
						template: CopiedEntityData
					},

					AddGameBrowserItem {
						editor_id: Uuid,
						parent_id: String,
						file: RuntimeID
					},

					SelectEntityInEditor {
						editor_id: Uuid,
						entity_id: String
					},

					MoveEntityToPlayer {
						editor_id: Uuid,
						entity_id: String
					},

					RotateEntityAsPlayer {
						editor_id: Uuid,
						entity_id: String
					},

					MoveEntityToCamera {
						editor_id: Uuid,
						entity_id: String
					},

					RotateEntityAsCamera {
						editor_id: Uuid,
						entity_id: String
					},

					RestoreToOriginal {
						editor_id: Uuid,
						entity_id: String
					}
				}),

				Monaco(pub enum EntityMonacoEvent {
					UpdateContent {
						editor_id: Uuid,
						entity_id: String,
						content: String
					},

					FollowReference {
						editor_id: Uuid,
						reference: String
					},

					OpenFactory {
						editor_id: Uuid,
						factory: String
					},

					SignalPin {
						editor_id: Uuid,
						entity_id: String,
						pin: String,
						output: bool
					},

					OpenResourceOverview {
						editor_id: Uuid,
						resource: String
					}
				}),

				MetaPane(pub enum EntityMetaPaneEvent {
					JumpToReference {
						editor_id: Uuid,
						reference: String
					},

					SetNotes {
						editor_id: Uuid,
						entity_id: String,
						notes: String
					}
				}),

				Metadata(pub enum EntityMetadataEvent {
					Initialise {
						editor_id: Uuid
					},

					SetFactoryHash {
						editor_id: Uuid,
						factory_hash: String
					},

					SetBlueprintHash {
						editor_id: Uuid,
						blueprint_hash: String
					},

					SetRootEntity {
						editor_id: Uuid,
						root_entity: String
					},

					SetSubType {
						editor_id: Uuid,
						sub_type: SubType
					},

					SetExternalScenes {
						editor_id: Uuid,
						external_scenes: Vec<String>
					}
				}),

				Overrides(pub enum EntityOverridesEvent {
					Initialise {
						editor_id: Uuid
					},

					UpdatePropertyOverrides {
						editor_id: Uuid,
						content: String
					},

					UpdateOverrideDeletes {
						editor_id: Uuid,
						content: String
					},

					UpdatePinConnectionOverrides {
						editor_id: Uuid,
						content: String
					},

					UpdatePinConnectionOverrideDeletes {
						editor_id: Uuid,
						content: String
					}
				})
			}),

			ResourceOverview(pub enum ResourceOverviewEvent {
				Initialise {
					id: Uuid
				},

				FollowDependency {
					id: Uuid,
					new_hash: String
				},

				FollowDependencyInNewTab {
					id: Uuid,
					hash: String
				},

				OpenInEditor {
					id: Uuid
				},

				ExtractAsQN {
					id: Uuid
				},

				ExtractAsFile {
					id: Uuid
				},

				ExtractTEMPAsRT {
					id: Uuid
				},

				ExtractTBLUAsFile {
					id: Uuid
				},

				ExtractTBLUAsRT {
					id: Uuid
				},

				ExtractAsRTGeneric {
					id: Uuid
				},

				ExtractAsImage {
					id: Uuid
				},

				ExtractAsWav {
					id: Uuid
				},

				ExtractMultiWav {
					id: Uuid
				},

				ExtractSpecificMultiWav {
					id: Uuid,
					index: u32
				},

				ExtractORESAsJson {
					id: Uuid
				},

				ExtractAsHMLanguages {
					id: Uuid
				}
			}),

			RepositoryPatch(pub enum RepositoryPatchEditorEvent {
				Initialise {
					id: Uuid
				},

				CreateRepositoryItem {
					id: Uuid
				},

				ResetModifications {
					id: Uuid,
					item: Uuid
				},

				ModifyItem {
					id: Uuid,
					item: Uuid,
					data: String
				},

				SelectItem {
					id: Uuid,
					item: Uuid
				}
			}),

			UnlockablesPatch(pub enum UnlockablesPatchEditorEvent {
				Initialise {
					id: Uuid
				},

				CreateUnlockable {
					id: Uuid
				},

				ResetModifications {
					id: Uuid,
					unlockable: Uuid
				},

				ModifyUnlockable {
					id: Uuid,
					unlockable: Uuid,
					data: String
				},

				SelectUnlockable {
					id: Uuid,
					unlockable: Uuid
				}
			}),

			ContentSearchResults(pub enum ContentSearchResultsEvent {
				Initialise {
					id: Uuid
				},

				OpenResourceOverview {
					id: Uuid,
					hash: RuntimeID
				}
			})
		}),

		Global(pub enum GlobalEvent {
			SetSeenAnnouncements(Vec<String>),
			LoadWorkspace(PathBuf),
			SelectAndOpenFile,
			SelectTab(Option<Uuid>),
			RemoveTab(Uuid),
			SaveTab(Uuid),
			UploadLogAndReport(String),
			UploadLastPanic,
			ClearLastPanic
		}),

		EditorConnection(pub enum EditorConnectionEvent {
			// Entity ID, TBLU hash
			EntitySelected(String, String),

			// Entity ID, TBLU hash, transform
			EntityTransformUpdated(String, String, QNTransform),

			// Entity ID, TBLU hash, property name, property type, new value
			EntityPropertyChanged(String, String, String, String, Value)
		})
	}
}

strike! {
	#[strikethrough[derive(Type, Serialize, Deserialize, Clone, Derivative)]]
	#[strikethrough[derivative(Debug)]]
	#[strikethrough[serde(rename_all = "camelCase", tag = "type", content = "data")]]
	pub enum Request {
		Tool(pub enum ToolRequest {
			FileBrowser(pub enum FileBrowserRequest {
				Create {
					path: PathBuf,
					is_folder: bool
				},

				Delete(PathBuf),

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

				Select(Option<PathBuf>),

				NewTree {
					base_path: PathBuf,

					/// Relative path, is folder
					#[derivative(Debug = "ignore")]
					files: Vec<(PathBuf, bool)>
				}
			}),

			GameBrowser(pub enum GameBrowserRequest {
				SetEnabled(bool),

				NewTree {
					game_description: String,

					#[derivative(Debug = "ignore")]
					entries: Vec<GameBrowserEntry>
				}
			}),

			Settings(pub enum SettingsRequest {
				Initialise {
					game_installs: Vec<GameInstall>,
					settings: AppSettings
				},
				ChangeProjectSettings(ProjectSettings)
			}),

			ContentSearch(pub enum ContentSearchRequest {
				SetEnabled(bool),
				SetPartitions(Vec<(String, String)>)
			})
		}),

		Editor(pub enum EditorRequest {
			Text(pub enum TextEditorRequest {
				ReplaceContent {
					id: Uuid,
					content: String
				},

				SetFileType {
					id: Uuid,
					file_type: TextFileType
				},
			}),

			Entity(pub enum EntityEditorRequest {
				General(pub enum EntityGeneralRequest {
					SetIsPatchEditor {
						editor_id: Uuid,
						is_patch_editor: bool
					}
				}),

				Tree(pub enum EntityTreeRequest {
					/// Will trigger a Select event from the tree - ensure this doesn't end up in a loop
					Select {
						editor_id: Uuid,
						id: Option<String>
					},

					NewTree {
						editor_id: Uuid,

						/// ID, parent, name, factory, has reverse parent refs
						#[derivative(Debug = "ignore")]
						entities: Vec<(String, Ref, String, String, bool)>
					},

					/// Instructs the frontend to take the list of new entities, add any new ones and update any ones that already exist (by ID) with the new information.
					/// This is used for pasting, and for ensuring that icons/parent status/name are updated when a sub-entity is updated.
					NewItems {
						editor_id: Uuid,

						/// ID, parent, name, factory, has reverse parent refs
						#[derivative(Debug = "ignore")]
						new_entities: Vec<(String, Ref, String, String, bool)>
					},

					SearchResults {
						editor_id: Uuid,

						/// The IDs of the entities matching the query
						#[derivative(Debug = "ignore")]
						results: Vec<String>
					},

					ShowHelpMenu {
						editor_id: Uuid,
						factory: String,
						input_pins: Vec<String>,
						output_pins: Vec<String>,
						default_properties_json: String
					},

					SetTemplates {
						editor_id: Uuid,
						templates: Vec<PastableTemplateCategory>
					},

					SetEditorConnectionAvailable {
						editor_id: Uuid,
						editor_connection_available: bool
					},

					SetShowDiff {
						editor_id: Uuid,
						show_diff: bool
					},

					SetDiffInfo {
						editor_id: Uuid,
						diff_info: (Vec<String>, Vec<String>, Vec<(String, String, Ref, String, bool)>)
					}
				}),

				Monaco(pub enum EntityMonacoRequest {
					DeselectIfSelected {
						editor_id: Uuid,
						entity_ids: Vec<String>
					},

					ReplaceContent {
						editor_id: Uuid,
						entity_id: String,
						content: String
					},

					ReplaceContentIfSameEntityID {
						editor_id: Uuid,
						entity_id: String,
						content: String
					},

					UpdateIntellisense {
						editor_id: Uuid,
						entity_id: String,
						properties: Vec<(String, String, Value, bool)>,
						pins: (Vec<String>, Vec<String>)
					},

					UpdateDecorationsAndMonacoInfo {
						editor_id: Uuid,
						entity_id: String,
						decorations: Vec<(String, String)>,
						local_ref_entity_ids: Vec<String>
					},

					UpdateValidity {
						editor_id: Uuid,
						validity: EditorValidity
					},

					SetEditorConnected {
						editor_id: Uuid,
						connected: bool
					}
				}),

				MetaPane(pub enum EntityMetaPaneRequest {
					SetReverseRefs {
						editor_id: Uuid,
						entity_names: std::collections::HashMap<String, String>,
						reverse_refs: Vec<ReverseReference>
					},

					SetNotes {
						editor_id: Uuid,
						entity_id: String,
						notes: String
					}
				}),

				Metadata(pub enum EntityMetadataRequest {
					Initialise {
						editor_id: Uuid,
						factory_hash: String,
						blueprint_hash: String,
						root_entity: String,
						sub_type: SubType,
						external_scenes: Vec<String>
					},

					SetHashModificationAllowed {
						editor_id: Uuid,
						hash_modification_allowed: bool
					},

					SetFactoryHash {
						editor_id: Uuid,
						factory_hash: String
					},

					SetBlueprintHash {
						editor_id: Uuid,
						blueprint_hash: String
					},

					UpdateCustomPaths {
						editor_id: Uuid,
						custom_paths: Vec<String>
					}
				}),

				Overrides(pub enum EntityOverridesRequest {
					Initialise {
						editor_id: Uuid,
						property_overrides: String,
						override_deletes: String,
						pin_connection_overrides: String,
						pin_connection_override_deletes: String
					},

					UpdateDecorations {
						editor_id: Uuid,
						decorations: Vec<(String, String)>,
					}
				})
			}),

			ResourceOverview(pub enum ResourceOverviewRequest {
				Initialise {
					id: Uuid,
					hash: String,
					filetype: String,
					chunk_patch: String,
					path_or_hint: Option<String>,

					/// Hash, type, path/hint, flag, is actually in current game version
					#[derivative(Debug = "ignore")]
					dependencies: Vec<(String, String, Option<String>, String, bool)>,

					/// Hash, type, path/hint
					#[derivative(Debug = "ignore")]
					reverse_dependencies: Vec<(String, String, Option<String>)>,

					changelog: Vec<ResourceChangelogEntry>,

					data: ResourceOverviewData
				}
			}),

			RepositoryPatch(pub enum RepositoryPatchEditorRequest {
				SetRepositoryItems {
					id: Uuid,

					#[derivative(Debug = "ignore")]
					items: Vec<(Uuid, RepositoryItemInformation)>
				},

				SetModifiedRepositoryItems {
					id: Uuid,
					modified: Vec<Uuid>
				},

				AddNewRepositoryItem {
					id: Uuid,
					new_item: (Uuid, RepositoryItemInformation)
				},

				RemoveRepositoryItem {
					id: Uuid,
					item: Uuid
				},

				SetMonacoContent {
					id: Uuid,
					item: Uuid,
					orig_data: String,
					data: String
				},

				DeselectMonaco {
					id: Uuid
				},

				ModifyItemInformation {
					id: Uuid,
					item: Uuid,
					info: RepositoryItemInformation
				}
			}),

			UnlockablesPatch(pub enum UnlockablesPatchEditorRequest {
				SetUnlockables {
					id: Uuid,

					#[derivative(Debug = "ignore")]
					unlockables: Vec<(Uuid, UnlockableInformation)>
				},

				SetModifiedUnlockables {
					id: Uuid,
					modified: Vec<Uuid>
				},

				AddNewUnlockable {
					id: Uuid,
					new_unlockable: (Uuid, UnlockableInformation)
				},

				RemoveUnlockable {
					id: Uuid,
					unlockable: Uuid
				},

				SetMonacoContent {
					id: Uuid,
					unlockable: Uuid,
					orig_data: String,
					data: String
				},

				DeselectMonaco {
					id: Uuid
				},

				ModifyUnlockableInformation {
					id: Uuid,
					unlockable: Uuid,
					info: UnlockableInformation
				}
			}),

			ContentSearchResults(pub enum ContentSearchResultsRequest {
				Initialise {
					id: Uuid,

					/// Hash, type, path/hint
					#[derivative(Debug = "ignore")]
					results: Vec<(String, String, Option<String>)>
				}
			})
		}),

		Global(pub enum GlobalRequest {
			ErrorReport { error: String },
			SetWindowTitle(String),
			InitialiseDynamics { dynamics: Dynamics, seen_announcements: Vec<String> },
			CreateTab {
				id: Uuid,
				name: String,
				editor_type: EditorType
			},
			RenameTab {
				id: Uuid,
				new_name: String
			},
			SelectTab(Uuid),
			SetTabUnsaved {
				id: Uuid,
				unsaved: bool
			},
			RemoveTab(Uuid),
			ComputeJSONPatchAndSave {
				base: Value,
				current: Value,
				save_path: PathBuf,
				file_and_type: (String, String)
			},
			RequestLastPanicUpload,
			LogUploadRejected
		})
	}
}
