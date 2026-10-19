fn main() {
    tauri_build::build();
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // Native tests reach Tauri's dialog code too (TaskDialogIndirect).
        // tauri-build only links its Common Controls v6 manifest into binaries.
        let resource =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("resource.lib");
        println!("cargo:rustc-link-arg-tests={}", resource.display());
        println!(
            "cargo:rustc-link-search=native={}",
            resource.parent().unwrap().display()
        );
    }
}
