use std::path::Path;

#[derive(Debug, Clone)]
pub struct LanguageConfig {
    pub name: &'static str,
    pub test_cmd: (&'static str, Vec<&'static str>),
}

pub fn detect_language(workspace_path: &Path) -> Option<LanguageConfig> {
    // Definimos ayudantes para chequear de forma segura
    let safe_exists = |file_name: &str| -> bool {
        let p = workspace_path.join(file_name);
        if crate::core::security::is_path_allowed(workspace_path, &p) {
            p.exists()
        } else {
            false
        }
    };

    // Rust
    if safe_exists("Cargo.toml") {
        return Some(LanguageConfig {
            name: "Rust",
            test_cmd: ("cargo", vec!["test"]),
        });
    }

    // Go
    if safe_exists("go.mod") {
        return Some(LanguageConfig {
            name: "Go",
            test_cmd: ("go", vec!["test", "./..."]),
        });
    }

    // JavaScript / TypeScript
    if safe_exists("package.json") {
        #[cfg(target_os = "windows")]
        let npm_cmd = "npm.cmd";
        #[cfg(not(target_os = "windows"))]
        let npm_cmd = "npm";

        return Some(LanguageConfig {
            name: "Node.js (JS/TS)",
            test_cmd: (npm_cmd, vec!["test"]),
        });
    }

    // C++
    if safe_exists("CMakeLists.txt") || safe_exists("Makefile") {
        return Some(LanguageConfig {
            name: "C++",
            test_cmd: ("make", vec!["test"]), // Comando genérico de fallback
        });
    }

    // Python (PyTest) - only if there is a real test setup
    let has_test_files = || -> bool {
        if let Ok(entries) = std::fs::read_dir(workspace_path) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("test_") && name.ends_with(".py") { return true; }
                if name.ends_with("_test.py") { return true; }
            }
        }
        false
    };
    if safe_exists("pytest.ini") || safe_exists("pyproject.toml") || has_test_files() {
        return Some(LanguageConfig {
            name: "Python",
            test_cmd: ("python", vec!["-m", "pytest"]),
        });
    }

    None
}
