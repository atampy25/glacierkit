fn main() {
	println!("cargo:rustc-link-search=ResourceLib");
	println!("cargo:rustc-link-lib=ResourceLib_HM2016");
	println!("cargo:rustc-link-lib=ResourceLib_HM2");
	println!("cargo:rustc-link-lib=ResourceLib_HM3");

	tauri_build::build()
}
