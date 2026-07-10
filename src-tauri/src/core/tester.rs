use std::path::Path;
use tokio::process::Command;
use std::process::Stdio;

/// The result of a test run.
pub enum TestResult {
    NoTests,          // No test framework detected — do nothing, don't revert
    Passed(String),   // Tests ran and passed
    Failed(String),   // Tests ran and actually failed
}

/// Check if a runner script exists for the given language
fn find_runner_script(workspace_path: &str, runner_name: &str) -> Option<std::path::PathBuf> {
    let exts = if cfg!(target_os = "windows") {
        vec![".bat", ".ps1"]
    } else {
        vec![".sh"]
    };
    
    for ext in exts {
        let path = Path::new(workspace_path).join(format!("{}{}", runner_name, ext));
        if path.exists() {
            // Check if executable on Unix
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if let Ok(meta) = std::fs::metadata(&path) {
                    let mode = meta.mode();
                    if mode & 0o111 == 0 {
                        // Not executable, skip
                        continue;
                    }
                }
            }
            return Some(path);
        }
    }
    None
}

/// Build the command to run a runner script
fn build_runner_command(runner_path: &Path) -> (String, Vec<String>) {
    if cfg!(target_os = "windows") {
        if runner_path.extension().map(|e| e == "ps1").unwrap_or(false) {
            ("powershell".to_string(), vec!["-ExecutionPolicy".to_string(), "Bypass".to_string(), "-File".to_string(), runner_path.to_string_lossy().to_string()])
        } else {
            ("cmd".to_string(), vec!["/C".to_string(), runner_path.to_string_lossy().to_string()])
        }
    } else {
        ("bash".to_string(), vec![runner_path.to_string_lossy().to_string()])
    }
}

/// The main test runner with runner script support
pub async fn run_tests(workspace_path: &str) -> TestResult {
    let path = Path::new(workspace_path);

    // ── Language + test-framework detection ──────────────────────────────────
    if let Some(lang_config) = crate::core::languages::detect_language(path) {
        let runner_name = format!("run_{}", lang_config.name.to_lowercase().replace(" ", "_").replace(".", "_").replace("(", "").replace(")", "").replace("/", "_"));
        
        // Try to find and use runner script first
        if let Some(runner_path) = find_runner_script(workspace_path, &runner_name) {
            emit_tester_info(&format!(
                "[TESTER] Detected language: {}. Using runner script: {}",
                lang_config.name,
                runner_path.display()
            ));
            
            let (cmd, args) = build_runner_command(&runner_path);
            
            let output = Command::new(&cmd)
                .args(&args)
                .current_dir(workspace_path)
                .stdin(Stdio::null())
                .output()
                .await;

            return handle_test_output(output, &lang_config.name).await;
        }
        
        // Fallback to direct command execution
        let (cmd, args) = lang_config.test_cmd;

        emit_tester_info(&format!(
            "[TESTER] Detected language: {}. Running: {} {}",
            lang_config.name,
            cmd,
            args.join(" ")
        ));

        let output = Command::new(cmd)
            .args(&args)
            .current_dir(workspace_path)
            .stdin(Stdio::null())
            .output()
            .await;

        return handle_test_output(output, &lang_config.name).await;
    }

    // No language / test framework detected even after manifest bootstrap
    TestResult::NoTests
}

async fn handle_test_output(output: Result<std::process::Output, std::io::Error>, lang_name: &str) -> TestResult {
    match output {
        // Binary not found → surface as ENV_FAILURE so agent.rs can auto-install
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            TestResult::Failed(format!(
                "[ENV_FAILURE] No se encontró el comando. \
                 El sistema intentará instalarlo automáticamente."
            ))
        }
        Err(e) => {
            TestResult::Failed(format!(
                "Error ejecutando test de {}: {}",
                lang_name, e
            ))
        }
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();

            if output.status.success() {
                TestResult::Passed(format!(
                    "{} — Pruebas completadas con éxito:\n{}",
                    lang_name, stdout
                ))
            } else {
                let combined = format!(
                    "{} — Tests fallaron:\n{}\n{}",
                    lang_name,
                    stdout.trim(),
                    stderr.trim()
                )
                .trim()
                .to_string();
                TestResult::Failed(combined)
            }
        }
    }
}

fn emit_tester_info(msg: &str) {
    println!("{}", msg);
}
