fn main() {
    // Tauri embeds its Windows manifest (Common Controls v6) only in the app's
    // .exe. The unit-test binaries link the same code — which imports
    // TaskDialogIndirect, a v6-only function — and failed to start
    // (0xc0000139). So the manifest is linked into every binary instead.
    let attrs = tauri_build::Attributes::new().windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
    tauri_build::try_build(attrs).expect("tauri build");

    let windows_msvc = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if windows_msvc {
        let manifest = std::env::current_dir().expect("build dir").join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed=windows-app-manifest.xml");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
}
