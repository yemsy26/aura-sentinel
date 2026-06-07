use std::path::Path;

#[derive(Debug, Clone)]
pub struct LanguageConfig {
    pub name: &'static str,
    pub test_cmd: (&'static str, Vec<&'static str>),
}

pub fn detect_language(workspace_path: &Path) -> Option<LanguageConfig> {
    let safe_exists = |file_name: &str| -> bool {
        let p = workspace_path.join(file_name);
        if crate::core::security::is_path_allowed(workspace_path, &p) {
            p.exists()
        } else {
            false
        }
    };

    // Helper: scan all files in workspace matching a predicate
    let any_file = |check: &dyn Fn(&str) -> bool| -> bool {
        if let Ok(entries) = std::fs::read_dir(workspace_path) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if check(name_str.as_ref()) {
                    return true;
                }
            }
        }
        false
    };

    // ── Rust ────────────────────────────────────────────────────────────────
    if safe_exists("Cargo.toml") {
        // Only run cargo test if there are actual #[test] functions
        let has_tests = any_file(&|name| name.ends_with(".rs"))
            && {
                // Walk all .rs files looking for #[test]
                let mut found = false;
                if let Ok(entries) = std::fs::read_dir(workspace_path) {
                    'outer: for entry in entries.flatten() {
                        if entry.path().extension().map(|e| e == "rs").unwrap_or(false) {
                            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                                if content.contains("#[test]") {
                                    found = true;
                                    break 'outer;
                                }
                            }
                        }
                    }
                }
                found
            };

        if has_tests {
            return Some(LanguageConfig {
                name: "Rust",
                test_cmd: ("cargo", vec!["test"]),
            });
        } else {
            return None; // Rust project but no test functions — skip TOOL_TESTER
        }
    }

    // ── Go ──────────────────────────────────────────────────────────────────
    let has_go_test_files = {
        fn scan_dir_for_go_tests(dir: &std::path::Path) -> bool {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                    if name.starts_with('.') || name == "vendor" { continue; }
                    if path.is_dir() {
                        if scan_dir_for_go_tests(&path) { return true; }
                    } else if name.ends_with("_test.go") {
                        return true;
                    }
                }
            }
            false
        }
        scan_dir_for_go_tests(workspace_path)
    };

    if safe_exists("go.mod") || has_go_test_files {
        // go test ./... exits 0 even with no _test.go files, so this is safe
        return Some(LanguageConfig {
            name: "Go",
            test_cmd: ("go", vec!["test", "./..."]),
        });
    }

    // ── JavaScript / TypeScript ─────────────────────────────────────────────
    // Recursive scan for JS/TS test files — runs REGARDLESS of package.json
    // because Qwen may create test files before (or without) creating package.json
    fn scan_dir_for_js_tests(dir: &std::path::Path) -> bool {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                if name.starts_with('.') || name == "node_modules" { continue; }
                if path.is_dir() {
                    if scan_dir_for_js_tests(&path) { return true; }
                } else if name.ends_with(".test.js")
                    || name.ends_with(".test.ts")
                    || name.ends_with(".spec.js")
                    || name.ends_with(".spec.ts") {
                    return true;
                }
            }
        }
        false
    }
    let has_js_test_files = scan_dir_for_js_tests(workspace_path);

    if safe_exists("package.json") || has_js_test_files {
        // BUG-1 FIX: Only activate full npm test if there is a real test setup
        let has_test_config = safe_exists("jest.config.js")
            || safe_exists("jest.config.ts")
            || safe_exists("vitest.config.js")
            || safe_exists("vitest.config.ts")
            || safe_exists(".mocharc.js")
            || safe_exists(".mocharc.yml");

        // Also check if package.json has a non-empty "test" script
        let has_test_script = std::fs::read_to_string(workspace_path.join("package.json"))
            .ok()
            .map(|content| {
                content.contains("\"test\"")
                    && !content.contains("Error: no test specified")
                    && !content.contains("echo \\\"Error: no test specified\\\"")
            })
            .unwrap_or(false);

        if has_test_config || has_js_test_files || has_test_script {
            #[cfg(target_os = "windows")]
            let npm_cmd = "npm.cmd";
            #[cfg(not(target_os = "windows"))]
            let npm_cmd = "npm";

            return Some(LanguageConfig {
                name: "Node.js (JS/TS)",
                test_cmd: (npm_cmd, vec!["test"]),
            });
        }
        // Has package.json or test files but no test runner configured yet
        return None;
    }

    // ── C++ ─────────────────────────────────────────────────────────────────
    if safe_exists("CMakeLists.txt") || safe_exists("Makefile") {
        // BUG-2 FIX: Only activate if there is a tests/ folder or the Makefile has a test target
        let has_test_dir = workspace_path.join("tests").is_dir()
            || workspace_path.join("test").is_dir();

        let makefile_has_test = std::fs::read_to_string(workspace_path.join("Makefile"))
            .or_else(|_| std::fs::read_to_string(workspace_path.join("makefile")))
            .ok()
            .map(|content| content.contains("\ntest:") || content.contains("\ntest "))
            .unwrap_or(false);

        if has_test_dir || makefile_has_test {
            return Some(LanguageConfig {
                name: "C++",
                test_cmd: ("make", vec!["test"]),
            });
        }
        return None; // C++ project but no test target
    }

    // ── Python (PyTest) ──────────────────────────────────────────────────────
    // Only activate if there is a real test setup (recursive scan)
    let has_python_test_files = {
        fn scan_dir_for_py_tests(dir: &std::path::Path) -> bool {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                    if name.starts_with('.') || name == "__pycache__" { continue; }
                    if path.is_dir() {
                        if scan_dir_for_py_tests(&path) { return true; }
                    } else if (name.starts_with("test_") && name.ends_with(".py"))
                        || name.ends_with("_test.py") {
                        return true;
                    }
                }
            }
            false
        }
        scan_dir_for_py_tests(workspace_path)
    };

    if safe_exists("pytest.ini") || safe_exists("pyproject.toml") || has_python_test_files {
        return Some(LanguageConfig {
            name: "Python",
            test_cmd: ("python", vec!["-m", "pytest"]),
        });
    }

    // ── Java (Maven / Gradle) ────────────────────────────────────────────────
    if safe_exists("pom.xml") {
        // Maven project
        #[cfg(target_os = "windows")]
        let mvn_cmd = "mvn.cmd";
        #[cfg(not(target_os = "windows"))]
        let mvn_cmd = "mvn";
        return Some(LanguageConfig {
            name: "Java (Maven)",
            test_cmd: (mvn_cmd, vec!["test", "-q"]),
        });
    }
    if safe_exists("build.gradle") || safe_exists("build.gradle.kts") {
        // Gradle project
        #[cfg(target_os = "windows")]
        let gradle_cmd = "gradlew.bat";
        #[cfg(not(target_os = "windows"))]
        let gradle_cmd = "./gradlew";
        return Some(LanguageConfig {
            name: "Java (Gradle)",
            test_cmd: (gradle_cmd, vec!["test"]),
        });
    }

    // ── C (gcc + simple test runner) ────────────────────────────────────────
    // Detect standalone C test files (e.g. test_*.c or *_test.c)
    let has_c_test_files = {
        fn scan_dir_for_c_tests(dir: &std::path::Path) -> bool {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                    if name.starts_with('.') { continue; }
                    if path.is_dir() {
                        if scan_dir_for_c_tests(&path) { return true; }
                    } else if (name.starts_with("test_") && name.ends_with(".c"))
                        || name.ends_with("_test.c") {
                        return true;
                    }
                }
            }
            false
        }
        scan_dir_for_c_tests(workspace_path)
    };

    if has_c_test_files && !safe_exists("CMakeLists.txt") && !safe_exists("Makefile") {
        // Standalone C test — compile and run all test_*.c files
        #[cfg(target_os = "windows")]
        return Some(LanguageConfig {
            name: "C",
            test_cmd: ("cmd", vec!["/C", "for %f in (test_*.c) do (gcc %f -o %~nf_test.exe && %~nf_test.exe)"]),
        });
        #[cfg(not(target_os = "windows"))]
        return Some(LanguageConfig {
            name: "C",
            test_cmd: ("bash", vec!["-c", "for f in test_*.c; do gcc \"$f\" -o \"${f%.c}_test\" && ./\"${f%.c}_test\"; done"]),
        });
    }

    None
}
