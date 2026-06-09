use std::path::Path;
use tokio::process::Command;
use std::process::Stdio;

/// The result of a test run.
pub enum TestResult {
    NoTests,          // No test framework detected — do nothing, don't revert
    Passed(String),   // Tests ran and passed
    Failed(String),   // Tests ran and actually failed
}

// ── Manifest helpers ─────────────────────────────────────────────────────────

// ── Main test runner ─────────────────────────────────────────────────────────

/// Detects the testing framework and runs the appropriate test suite.
pub async fn run_tests(workspace_path: &str) -> TestResult {
    let path = Path::new(workspace_path);

    // ── Language + test-framework detection ──────────────────────────────────
    if let Some(lang_config) = crate::core::languages::detect_language(path) {
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

        match output {
            // Binary not found → surface as ENV_FAILURE so agent.rs can auto-install
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return TestResult::Failed(format!(
                    "[ENV_FAILURE] No se encontró el comando '{}'. \
                     El sistema intentará instalarlo automáticamente.",
                    cmd
                ));
            }
            Err(e) => {
                return TestResult::Failed(format!(
                    "Error ejecutando test de {}: {}",
                    lang_config.name, e
                ));
            }
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                if output.status.success() {
                    return TestResult::Passed(format!(
                        "{} — Pruebas completadas con éxito:\n{}",
                        lang_config.name, stdout
                    ));
                } else {
                    // Combine stdout+stderr so the agent sees the full picture
                    let combined = format!(
                        "{} — Tests fallaron:\n{}\n{}",
                        lang_config.name,
                        stdout.trim(),
                        stderr.trim()
                    )
                    .trim()
                    .to_string();
                    return TestResult::Failed(combined);
                }
            }
        }
    }

    // No language / test framework detected even after manifest bootstrap
    TestResult::NoTests
}

fn emit_tester_info(msg: &str) {
    println!("{}", msg);
}
