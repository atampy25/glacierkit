use std::{env, path::PathBuf};

fn main() {
	let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

	// Windows-specific linking
	#[cfg(target_os = "windows")]
	{
		println!("cargo:rustc-link-search=ResourceLib/ResourceLib-win-x64");
		println!("cargo:rustc-link-lib=ResourceLib_HM2016");
		println!("cargo:rustc-link-lib=ResourceLib_HM2");
		println!("cargo:rustc-link-lib=ResourceLib_HM3");
	}

	// Linux-specific linking
	#[cfg(target_os = "linux")]
	{
		let resourcelib_dir = manifest_dir.join("ResourceLib/ResourceLib-linux-x64");

		println!("cargo:rustc-link-search={}", resourcelib_dir.display());
		println!("cargo:rustc-link-arg=-Wl,-rpath={}", resourcelib_dir.display());

		println!("cargo:rustc-link-lib=dylib:+verbatim=ResourceLib_HM2016.so");
		println!("cargo:rustc-link-lib=dylib:+verbatim=ResourceLib_HM2.so");
		println!("cargo:rustc-link-lib=dylib:+verbatim=ResourceLib_HM3.so");

		println!("cargo:include={}", resourcelib_dir.join("include").display());
	}

	let static_folder = manifest_dir.join("../static");
	let out_path = manifest_dir.join("../build/_app/immutable/assets");

	let files = ["32px.png", "throbber.gif"];

	for file in files {
		std::fs::copy(static_folder.join(file), out_path.join(file)).expect("Failed to copy asset to output directory");
	}

	tauri_build::build();
}
