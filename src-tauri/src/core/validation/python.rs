#![allow(dead_code)]
use std::process::Stdio;
use tokio::process::Command;
use std::path::Path;

pub async fn validate_python(workspace_path: &str) -> Result<(), String> {
    let path = Path::new(workspace_path);

    let has_py = || -> bool {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                if let Some(ext) = entry.path().extension() {
                    if ext == "py" { return true; }
                }
            }
        }
        false
    };

    if !has_py() {
        return Ok(());
    }

    let output = Command::new("python")
        .arg("-m")
        .arg("compileall")
        .arg("-q")
        .arg("-x")
        .arg("node_modules|\\.git|__pycache__|venv|\\.venv")
        .arg(".")
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .current_dir(workspace_path)
        .stdin(Stdio::null())
        .output()
        .await;

    match output {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            Err(format!("[PYTHON_SYNTAX_ERROR] {} {}", stdout, stderr).trim().to_string())
        },
        Err(_) => {
            // python not found in PATH — treat as a no-op
            Ok(())
        }
    }
}
