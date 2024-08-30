use anyhow::Result;
use fn_error_context::context;
use hashbrown::HashMap;
use hitman_commons::game::GameVersion;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::PathBuf;

#[derive(Deserialize)]
struct SteamLibraryFolder {
	path: String,
	apps: HashMap<String, String>
}

#[derive(Type, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct GameInstall {
	pub version: GameVersion,
	pub platform: String,
	pub path: PathBuf
}

#[context("Couldn't detect installed games")]
pub fn detect_installs() -> Result<Vec<GameInstall>> {
	detection::detect_installs()
}

#[cfg(target_os = "windows")]
mod detection {
	use std::os::windows::process::CommandExt;
	use std::{fs, path::PathBuf};
	use std::{path::Path, process::Command};

	use anyhow::{bail, Context, Result};
	use fn_error_context::context;
	use hashbrown::HashMap;
	use hitman_commons::game::GameVersion;
	use itertools::Itertools;
	use registry::{Data, Hive, Security};
	use serde_json::Value;
	use tryvial::try_fn;

	use super::{GameInstall, SteamLibraryFolder};

	#[try_fn]
	#[context("Couldn't detect installed games")]
	pub fn detect_installs() -> Result<Vec<GameInstall>> {
		let legendary_installed_paths = [
			Path::new(&std::env::var("USERPROFILE").context("%USERPROFILE%")?)
				.join(".config")
				.join("legendary")
				.join("installed.json"),
			Path::new(&std::env::var("APPDATA").context("%APPDATA%")?)
				.join("heroic")
				.join("legendaryConfig")
				.join("legendary")
				.join("installed.json")
		];

		let mut check_paths = vec![];

		// Legendary installs
		for legendary_installed_path in legendary_installed_paths {
			if legendary_installed_path.exists() {
				let legendary_installed_data: Value =
					serde_json::from_slice(&fs::read(legendary_installed_path).context("Reading legendary installed")?)
						.context("Legendary installed as JSON")?;

				// H3
				if let Some(data) = legendary_installed_data.get("Eider") {
					check_paths.push((
						PathBuf::from(
							data.get("install_path")
								.context("install_path")?
								.as_str()
								.context("as_str")?
						),
						"Epic Games"
					));
				}

				// H1
				if let Some(data) = legendary_installed_data.get("Barbet") {
					check_paths.push((
						PathBuf::from(
							data.get("install_path")
								.context("install_path")?
								.as_str()
								.context("as_str")?
						),
						"Epic Games"
					));
				}
			}
		}

		// EGL installs
		if let Ok(hive) = Hive::CurrentUser.open(r#"Software\Epic Games\EOS"#, Security::Read) {
			match hive.value("ModSdkMetadataDir") {
				Ok(Data::String(d)) => {
					if let Ok(entries) = fs::read_dir(d.to_string_lossy()) {
						for entry in entries
							.filter_map(|x| x.ok())
							.filter(|x| x.file_type().ok().map(|x| x.is_file()).unwrap_or(false))
						{
							if let Ok(manifest_data) = serde_json::from_slice::<Value>(
								&fs::read(entry.path())
									.with_context(|| format!("Reading EOS manifest {}", entry.path().display()))?
							) {
								// H3
								if manifest_data
									.get("AppName")
									.context("AppName")?
									.as_str()
									.context("as_str")? == "Eider"
								{
									check_paths.push((
										PathBuf::from(
											manifest_data
												.get("InstallLocation")
												.context("InstallLocation")?
												.as_str()
												.context("as_str")?
										),
										"Epic Games"
									));
								}

								// H1
								if manifest_data
									.get("AppName")
									.context("AppName")?
									.as_str()
									.context("as_str")? == "Barbet"
								{
									check_paths.push((
										PathBuf::from(
											manifest_data
												.get("InstallLocation")
												.context("InstallLocation")?
												.as_str()
												.context("as_str")?
										),
										"Epic Games"
									));
								}
							}
						}
					}
				}

				Ok(_) => Err(anyhow::anyhow!(
					"Registry key ModSdkMetadataDir was not string".to_owned()
				))?,

				Err(_) => {}
			}
		}

		// 	Steam installs
		if let Ok(hive) = Hive::CurrentUser.open(r#"Software\Valve\Steam"#, Security::Read) {
			match hive.value("SteamPath") {
				Ok(Data::String(d)) => {
					if let Ok(s) = fs::read_to_string(
						if Path::new(&d.to_string_lossy())
							.join("config")
							.join("libraryfolders.vdf")
							.exists()
						{
							Path::new(&d.to_string_lossy())
								.join("config")
								.join("libraryfolders.vdf")
						} else {
							Path::new(&d.to_string_lossy())
								.join("steamapps")
								.join("libraryfolders.vdf")
						}
					) {
						let folders: HashMap<String, SteamLibraryFolder> =
							keyvalues_serde::from_str(&s).context("VDF parse")?;

						for folder in folders.values() {
							// H1, H1 free trial
							if folder.apps.contains_key("236870") || folder.apps.contains_key("649780") {
								check_paths.push((
									PathBuf::from(&folder.path)
										.join("steamapps")
										.join("common")
										.join("HITMAN™"),
									"Steam"
								));
							}

							// H2
							if folder.apps.contains_key("863550") {
								check_paths.push((
									PathBuf::from(&folder.path)
										.join("steamapps")
										.join("common")
										.join("HITMAN2"),
									"Steam"
								));
							}

							// H3, H3 demo
							if folder.apps.contains_key("1659040") || folder.apps.contains_key("1847520") {
								check_paths.push((
									PathBuf::from(&folder.path)
										.join("steamapps")
										.join("common")
										.join("HITMAN 3"),
									"Steam"
								));
							}
						}
					};
				}

				Ok(_) => {
					bail!("Registry key SteamPath was not string");
				}

				Err(_) => {}
			}
		}

		// Microsoft install of H3
		if let Ok(proc_out) = Command::new("powershell")
			.args(["-Command", "Get-AppxPackage -Name IOInteractiveAS.PC-HITMAN3-BaseGame"])
			.creation_flags(0x08000000) // CREATE_NO_WINDOW
			.output()
		{
			if let Some(line) = String::from_utf8_lossy(&proc_out.stdout)
				.lines()
				.find(|x| x.starts_with("InstallLocation"))
			{
				check_paths.push((
					fs::read_link(line.split(':').skip(1).collect::<Vec<_>>().join(":").trim())?,
					"Microsoft"
				));
			}
		}

		// GOG install of H1
		if let Ok(hive) = Hive::LocalMachine.open(r#"Software\WOW6432Node\GOG.com\Games\1545448592"#, Security::Read) {
			match hive.value("path") {
				Ok(Data::String(d)) => {
					check_paths.push((PathBuf::from(&d.to_string_lossy()), "GOG"));
				}

				_ => {
					bail!("GOG install path was not string");
				}
			}
		}

		let mut game_installs = vec![];

		for (path, platform) in check_paths {
			// Game folder has Retail
			let subfolder_retail = path.join("Retail").is_dir();

			if subfolder_retail {
				game_installs.push(GameInstall {
					path: path.join("Retail"),
					platform: platform.into(),
					version: if path.join("Retail").join("HITMAN3.exe").is_file() {
						GameVersion::H3
					} else if path.join("Retail").join("HITMAN2.exe").is_file() {
						GameVersion::H2
					} else if path.join("Retail").join("HITMAN.exe").is_file() {
						GameVersion::H1
					} else {
						bail!("Unknown game added to check paths");
					}
				});
			}
		}

		game_installs
			.into_iter()
			.unique_by(|x| x.path.to_owned())
			.sorted_unstable_by_key(|x| x.version)
			.collect()
	}
}

#[cfg(target_os = "linux")]
mod detection {
	use std::{fs, path::PathBuf};

	use anyhow::{bail, Context, Result};
	use fn_error_context::context;
	use hashbrown::HashMap;
	use hitman_commons::game::GameVersion;
	use itertools::Itertools;
	use serde_json::Value;
	use tryvial::try_fn;

	use super::{GameInstall, SteamLibraryFolder};

	#[try_fn]
	#[context("Couldn't detect installed games")]
	pub fn detect_installs() -> Result<Vec<GameInstall>> {
		let mut check_paths = vec![];

		// Legendary installs
		if let Some(home_dir) = home::home_dir() {
			let legendary_installed_path = home_dir
				.join(".config/legendary/installed.json")
				.exists()
				.then_some(home_dir.join(".config/legendary/installed.json"));

			if let Some(legendary_installed_path) = legendary_installed_path {
				let legendary_installed_data: Value =
					serde_json::from_slice(&fs::read(legendary_installed_path).context("Reading legendary installed")?)
						.context("Legendary installed as JSON")?;

				// H3
				if let Some(data) = legendary_installed_data.get("Eider") {
					check_paths.push((
						PathBuf::from(
							data.get("install_path")
								.context("install_path")?
								.as_str()
								.context("as_str")?
						),
						"Epic Games"
					));
				}

				// H1
				if let Some(data) = legendary_installed_data.get("Barbet") {
					check_paths.push((
						PathBuf::from(
							data.get("install_path")
								.context("install_path")?
								.as_str()
								.context("as_str")?
						),
						"Epic Games"
					));
				}
			}
		}

		// Steam installs
		if let Some(home_dir) = home::home_dir() {
			let steam_path = match home_dir {
				home if home_dir.join(".local/share/Steam").exists() => Some(home.join(".local/share/Steam")),
				home if home_dir.join(".steam/steam").exists() => Some(home.join(".steam/steam")),
				_ => None
			};

			if let Some(steam_path) = steam_path {
				if let Ok(s) = fs::read_to_string(if steam_path.join("config").join("libraryfolders.vdf").exists() {
					steam_path.join("config").join("libraryfolders.vdf")
				} else {
					steam_path.join("steamapps").join("libraryfolders.vdf")
				}) {
					let folders: HashMap<String, SteamLibraryFolder> =
						keyvalues_serde::from_str(&s).context("VDF parse")?;

					for folder in folders.values() {
						// H1, H1 free trial
						if folder.apps.contains_key("236870") || folder.apps.contains_key("649780") {
							check_paths.push((
								PathBuf::from(&folder.path)
									.join("steamapps")
									.join("common")
									.join("HITMAN™"),
								"Steam"
							));
							check_paths.push((
								PathBuf::from(&folder.path)
									.join("steamapps")
									.join("common")
									.join("Hitman™"),
								"Steam"
							));
							check_paths.push((
								PathBuf::from(&folder.path)
									.join("steamapps")
									.join("common")
									.join("Hitman™")
									.join("share")
									.join("data"),
								"Steam"
							));
						}

						// H2
						if folder.apps.contains_key("863550") {
							check_paths.push((
								PathBuf::from(&folder.path)
									.join("steamapps")
									.join("common")
									.join("HITMAN2"),
								"Steam"
							));
						}

						// H3, H3 demo
						if folder.apps.contains_key("1659040") || folder.apps.contains_key("1847520") {
							check_paths.push((
								PathBuf::from(&folder.path)
									.join("steamapps")
									.join("common")
									.join("HITMAN 3"),
								"Steam"
							));
						}
					}
				};
			}
		}

		let mut game_installs = vec![];

		for (path, platform) in check_paths {
			let retail_folder = ["Retail", "retail"]
				.iter()
				.map(|folder| path.join(folder))
				.find(|joined_path| joined_path.exists());

			if let Some(retail_folder) = retail_folder {
				let version = if retail_folder.join("HITMAN3.exe").is_file() {
					GameVersion::H3
				} else if retail_folder.join("HITMAN2.exe").is_file() {
					GameVersion::H2
				} else if retail_folder.join("HITMAN.exe").is_file() || retail_folder.join("hitman.dll").is_file() {
					GameVersion::H1
				} else {
					bail!("Unknown game added to check paths");
				};

				game_installs.push(GameInstall {
					path: retail_folder,
					platform: platform.into(),
					version
				});
			}
		}

		game_installs
			.into_iter()
			.unique_by(|x| x.path.to_owned())
			.sorted_unstable_by_key(|x| x.version)
			.collect()
	}
}
