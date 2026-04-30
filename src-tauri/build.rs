use std::{env, path::PathBuf};

fn main() {
	let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

	let static_folder = manifest_dir.join("../static");
	let out_path = manifest_dir.join("../build/_app/immutable/assets");

	std::fs::create_dir_all(&out_path).expect("failed to create output assets directory");

	let files = ["32px.png", "throbber.gif"];

	for file in files {
		std::fs::copy(static_folder.join(file), out_path.join(file)).expect("Failed to copy asset to output directory");
	}

	tauri_build::build();
}
