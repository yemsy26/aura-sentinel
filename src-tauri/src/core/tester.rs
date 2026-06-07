use std::path::Path;
use tokio::process::Command;
use std::process::Stdio;

/// The result of a test run, distinguishing between "no test suite found",
/// "tests passed", and "tests failed". This prevents Git-Shield from reverting
/// code just because there are no tests defined.
pub enum TestResult {
    NoTests,          // No test framework detected — do nothing, don't revert
    Passed(String),   // Tests ran and passed
    Failed(String),   // Tests ran and actually failed
}

/// Detects the testing framework and runs the appropriate test suite.
pub async fn run_tests(workspace_path: &str) -> TestResult {
    let path = Path::new(workspace_path);

    if let Some(lang_config) = crate::core::languages::detect_language(path) {
        let (cmd, args) = lang_config.test_cmd;
        
        let output = Command::new(cmd)
            .args(&args)
            .current_dir(workspace_path)
            .stdin(Stdio::null())
            .output()
            .await;

        match output {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // The test runner binary itself is missing — this is an env issue, not a code failure
                return TestResult::Failed(format!(
                    "[ENV_FAILURE] No se encontró el comando '{}'. Asegúrate de que el lenguaje esté instalado y en el PATH.",
                    cmd
                ));
            },
            Err(e) => {
                return TestResult::Failed(format!("Error ejecutando test de {}: {}", lang_config.name, e));
            },
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                if output.status.success() {
                    return TestResult::Passed(format!("{} Test completado con éxito:\n{}", lang_config.name, stdout));
                } else {
                    return TestResult::Failed(format!("{} Test Falló:\n{}\n{}", lang_config.name, stdout, stderr).trim().to_string());
                }
            }
        }
    }

    // No test framework detected in this workspace
    TestResult::NoTests
}
