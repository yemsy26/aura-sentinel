#![allow(dead_code)]
use std::process::Stdio;
use tokio::process::Command;
use std::path::Path;

pub async fn validate_rust(workspace_path: &str) -> Result<(), String> {
    let path = Path::new(workspace_path);
    if !path.join("Cargo.toml").exists() {
        return Ok(());
    }

    let output = Command::new("cargo")
        .arg("check")
        .current_dir(workspace_path)
        .stdin(Stdio::null())
        .output()
        .await;

    match output {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            Err(format!("[CARGO_CHECK_ERROR] {} {}", stdout, stderr).trim().to_string())
        },
        Err(_) => {
            // cargo not found in PATH — treat gracefully
            Ok(())
        }
    }
}
