#![allow(dead_code)]
use std::process::Stdio;
use tokio::process::Command;
use std::path::Path;

pub async fn validate_typescript(workspace_path: &str) -> Result<(), String> {
    let path = Path::new(workspace_path);

    let mut ts_files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
                let ext_lower = ext.to_lowercase();
                if ext_lower == "ts" || ext_lower == "tsx" {
                    ts_files.push(entry.path().to_string_lossy().to_string());
                }
            }
        }
    }

    if ts_files.is_empty() {
        return Ok(());
    }

    // If tsconfig.json or ts files exist, attempt validation via tsc or npx tsc
    let has_tsc = Command::new("tsc")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);

    if has_tsc {
        let output = Command::new("tsc")
            .arg("--noEmit")
            .current_dir(workspace_path)
            .stdin(Stdio::null())
            .output()
            .await;

        if let Ok(out) = output {
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                return Err(format!("[TYPESCRIPT_TYPE_ERROR] {} {}", stdout, stderr).trim().to_string());
            }
        }
        return Ok(());
    }

    // Try npx tsc --noEmit if local/npm tsc is present
    let output = Command::new("npx")
        .arg("tsc")
        .arg("--noEmit")
        .current_dir(workspace_path)
        .stdin(Stdio::null())
        .output()
        .await;

    if let Ok(out) = output {
        if !out.status.success() && !out.stderr.is_empty() {
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            // If npx failed because tsc isn't installed in project, don't block
            if !stderr.contains("could not determine executable") && !stderr.contains("command not found") {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                return Err(format!("[TYPESCRIPT_ERROR] {} {}", stdout, stderr).trim().to_string());
            }
        }
    }

    Ok(())
}
