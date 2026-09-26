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
    tauri_build::build()
}
