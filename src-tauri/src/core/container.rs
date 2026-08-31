use serde::{Deserialize, Serialize};
use std::path::Path;
use chrono::Utc;
use tokio::process::Command;

/// Actions supported by TOOL_CONTAINER
#[derive(Debug, Clone)]
pub enum ContainerAction {
    Run,
    Exec,
    Stop,
    Remove,
    Status,
    Logs,
    ActivateEnv,
}

impl ContainerAction {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "run" => Self::Run,
            "exec" => Self::Exec,
            "stop" => Self::Stop,
            "rm" | "remove" => Self::Remove,
            "status" | "ps" => Self::Status,
            "logs" => Self::Logs,
            "activate" | "env" => Self::ActivateEnv,
            _ => Self::Run,
        }
    }
}

/// Detects docker or podman available in PATH
fn detect_runtime() -> Option<&'static str> {
    let check = |cmd: &str| -> bool {
        std::process::Command::new(cmd)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    if check("docker") { Some("docker") }
    else if check("podman") { Some("podman") }
    else { None }
}

/// Execute a container operation. Returns stdout or error string.
pub async fn container_exec(
    action: ContainerAction,
    image_or_id: &str,
    cmd: &str,
    workspace: &str,
) -> Result<String, String> {
    let runtime = detect_runtime()
        .ok_or_else(|| "❌ TOOL_CONTAINER: Docker ni Podman encontrado en PATH. Instala uno de los dos primero.".to_string())?;

    let output = match action {
        ContainerAction::Run => {
            Command::new(runtime)
                .args(["run", "-d", "--name", &format!("aura_{}", sanitize(image_or_id)),
                       "-v", &format!("{}:/workspace", workspace),
                       image_or_id])
                .output().await
        },
        ContainerAction::Exec => {
            // Exec requires a running container ID + cmd
            Command::new(runtime)
                .args(["exec", image_or_id, "sh", "-c", cmd])
                .output().await
        },
        ContainerAction::Stop => {
            Command::new(runtime)
                .args(["stop", image_or_id])
                .output().await
        },
        ContainerAction::Remove => {
            Command::new(runtime)
                .args(["rm", "-f", image_or_id])
                .output().await
        },
        ContainerAction::Status => {
            Command::new(runtime)
                .args(["ps", "-a", "--filter", &format!("name={}", image_or_id), "--format", "table {{.Names}}\t{{.Status}}\t{{.Ports}}"])
                .output().await
        },
        ContainerAction::Logs => {
            Command::new(runtime)
                .args(["logs", "--tail", "50", image_or_id])
                .output().await
        },
        ContainerAction::ActivateEnv => {
            return activate_project_env(workspace).await;
        },
    };

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            if out.status.success() {
                Ok(format!("✅ [{}] {}", runtime, stdout.trim()))
            } else {
                Err(format!("❌ [{}] Error: {}", runtime, stderr.trim()))
            }
        },
        Err(e) => Err(format!("❌ No se pudo ejecutar {}: {}", runtime, e)),
    }
}

/// Detects project type and activates the correct environment
pub async fn activate_project_env(workspace: &str) -> Result<String, String> {
    let ws = Path::new(workspace);
    let mut activations = Vec::new();

    // Node.js / nvm
    if ws.join("package.json").exists() {
        let node_check = Command::new("node").arg("--version").output().await;
        match node_check {
            Ok(o) if o.status.success() => {
                let ver = String::from_utf8_lossy(&o.stdout).trim().to_string();
                activations.push(format!("✅ Node.js detectado: {}", ver));
            },
            _ => activations.push("⚠️ Node.js no encontrado. Instala nvm o Node.js.".to_string()),
        }
    }

    // Python / venv
    if ws.join("requirements.txt").exists() || ws.join("pyproject.toml").exists() {
        let venv_path = ws.join(".venv");
        if !venv_path.exists() {
            // Crear venv automáticamente
            let _ = Command::new("python")
                .args(["-m", "venv", ".venv"])
                .current_dir(workspace)
                .output().await;
            activations.push("✅ Python venv creado en .venv/".to_string());
        } else {
            activations.push("✅ Python venv existente en .venv/".to_string());
        }
    }

    // Rust / cargo
    if ws.join("Cargo.toml").exists() {
        let rustup_check = Command::new("rustup").arg("show").output().await;
        match rustup_check {
            Ok(o) if o.status.success() => {
                activations.push("✅ Rust/Cargo activo vía rustup.".to_string());
            },
            _ => activations.push("⚠️ rustup no encontrado.".to_string()),
        }
    }

    if activations.is_empty() {
        Ok("ℹ️ No se detectaron archivos de proyecto conocidos (package.json, requirements.txt, Cargo.toml).".to_string())
    } else {
        Ok(format!("🔧 Entorno activado:\n{}", activations.join("\n")))
    }
}

fn sanitize(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-').collect()
}
