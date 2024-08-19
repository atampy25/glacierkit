fn main() {
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
		let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
		let resourcelib_dir = std::path::Path::new(&dir).join("ResourceLib/ResourceLib-linux-x64");
        
		println!("cargo:rustc-link-search={}", resourcelib_dir.display());        
		println!("cargo:rustc-link-arg=-Wl,-rpath={}", resourcelib_dir.display());

		println!("cargo:rustc-link-lib=dylib:+verbatim=ResourceLib_HM2016.so");
        println!("cargo:rustc-link-lib=dylib:+verbatim=ResourceLib_HM2.so");
        println!("cargo:rustc-link-lib=dylib:+verbatim=ResourceLib_HM3.so");

        println!("cargo:include={}", resourcelib_dir.join("include").display());
    }

    copy_static_assets();
    tauri_build::build();
}

fn copy_static_assets(){
    let out_path = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../build/_app/immutable/assets");
    let static_folder = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../static");

    let files = vec![
        "32px.png",
        "throbber.gif",
    ];

    for file in files{
        std::fs::copy(&static_folder.join(file), &out_path.join(file)).expect("Failed to copy dll to output directory: {}");
    }
}