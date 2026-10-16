use std::hash::{Hash, Hasher};

fn main() {
    let sources = [
        "src/llm/agent.rs",
        "src/llm/mod.rs",
        "src/core/mod.rs",
        "src/core/session_journal.rs",
        "src/core/experience.rs",
        "src/core/episodic_memory.rs",
        "src/core/programmer_executor.rs",
        "src/core/mission_runtime.rs",
        "src/core/browser_automation.rs",
        "src/core/vision.rs",
        "src/core/learning/engine.rs",
        "src/core/learning/persistence.rs",
    ];
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for source in sources {
        println!("cargo:rerun-if-changed={source}");
        source.hash(&mut hasher);
        if let Ok(contents) = std::fs::read(source) {
            contents.hash(&mut hasher);
        }
    }
    println!(
        "cargo:rustc-env=AURA_CORE_SOURCE_FINGERPRINT={:016x}",
        hasher.finish()
    );
    // tauri-build solo incrusta el manifiesto (Common Controls v6) en los binarios; sin él los
    // ejecutables de `cargo test` no arrancan en Windows (STATUS_ENTRYPOINT_NOT_FOUND).
    // Se enlaza en todos los objetivos, pruebas incluidas.
    let mut attributes = tauri_build::Attributes::new();
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed=windows-app-manifest.xml");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
    }
    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}
