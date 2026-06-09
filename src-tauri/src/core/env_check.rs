use std::path::Path;
use std::net::{TcpStream, SocketAddr};
use std::time::Duration;
use tokio::process::Command;
use sysinfo::Disks;

/// Checks if a single command is available in the PATH.
async fn is_cmd_available(cmd: &str) -> bool {
    #[cfg(target_os = "windows")]
    let (shell, flag, check) = ("cmd", "/C", format!("where {}", cmd));
    #[cfg(not(target_os = "windows"))]
    let (shell, flag, check) = ("sh", "-c", format!("which {}", cmd));

    Command::new(shell)
        .args([flag, &check])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Injects Scoop shims into the current process PATH so commands installed
/// via Scoop are visible to subsequent `is_cmd_available` checks.
fn inject_scoop_path() {
    if let Ok(profile) = std::env::var("USERPROFILE") {
        let shims = format!("{}\\scoop\\shims", profile);
        let scripts = format!("{}\\scoop\\apps\\python\\current\\Scripts", profile);
        let python_dir = format!("{}\\scoop\\apps\\python\\current", profile);
        let node_dir = format!("{}\\scoop\\apps\\nodejs\\current", profile);

        let current = std::env::var("PATH").unwrap_or_default();
        let extras = [shims.as_str(), scripts.as_str(), python_dir.as_str(), node_dir.as_str()];
        let mut new_path = current.clone();
        for extra in &extras {
            if !current.contains(extra) {
                new_path = format!("{};{}", extra, new_path);
            }
        }
        std::env::set_var("PATH", new_path);
    }
}

/// Detects which language runtime is needed based on workspace contents.
/// Returns a list of required commands for THIS specific project type.
fn detect_required_commands(workspace_path: &str) -> Vec<&'static str> {
    let path = Path::new(workspace_path);
    let mut required = vec!["git"]; // git is always useful but not blocking

    if path.join("Cargo.toml").exists() {
        required.push("cargo");
    }
    if path.join("package.json").exists() {
        required.push("npm");
    }
    if path.join("requirements.txt").exists()
        || path.join("main.py").exists()
        || path.join("pyproject.toml").exists()
    {
        required.push("python");
    }
    if path.join("go.mod").exists() {
        required.push("go");
    }
    if path.join("pom.xml").exists() {
        required.push("mvn");
    }
    if path.join("build.gradle").exists() || path.join("build.gradle.kts").exists() {
        required.push("gradle");
    }

    required
}

/// Realiza una auditoría ambiental rápida antes de que el agente comience a trabajar.
/// Retorna `Ok(modelos_disponibles)` si todo está correcto, o `Err` con una lista de problemas encontrados.
pub async fn validate_environment(workspace_path: &str) -> Result<Vec<String>, Vec<String>> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    // 0. Inject Scoop shims into PATH so installed tools are visible
    inject_scoop_path();

    // 1. Check only the commands relevant to this workspace type.
    //    Other languages are WARNINGS, not hard errors.
    let workspace_specific = detect_required_commands(workspace_path);

    // Always check Ollama (core requirement)
    // Always check git (useful for Git-Shield)
    // Only check language tools if the workspace actually uses that language

    // Separate into "blocking" (language runtime for this project) vs "soft" (git, others)
    let blocking_cmds: Vec<&str> = workspace_specific
        .iter()
        .filter(|&&c| c != "git") // git is soft — missing git won't stop agent
        .copied()
        .collect();

    for cmd in &blocking_cmds {
        if !is_cmd_available(cmd).await {
            errors.push(format!(
                "No encuentro '{}' en el PATH. Este proyecto lo requiere. Usa TOOL_ENV_MANAGER para instalarlo.",
                cmd
            ));
        }
    }

    if !is_cmd_available("git").await {
        warnings.push("'git' no está en el PATH. Git-Shield (rollback) no estará disponible, pero la ejecución puede continuar.".to_string());
    }

    // 2. Write permissions in workspace
    let test_file = Path::new(workspace_path).join(".aura_test_write");
    if std::fs::write(&test_file, "test").is_err() {
        errors.push(format!("No tengo permisos de escritura en el directorio: {}", workspace_path));
    } else {
        let _ = std::fs::remove_file(&test_file);
    }

    // 3. Disk space (>500MB) — hard check
    let disks = Disks::new_with_refreshed_list();
    let workspace_path_obj = Path::new(workspace_path);
    let mut found_disk = false;
    for disk in disks.list() {
        if workspace_path_obj.starts_with(disk.mount_point()) {
            found_disk = true;
            let free_mb = disk.available_space() / (1024 * 1024);
            if free_mb < 500 {
                errors.push(format!(
                    "Espacio en disco insuficiente en {}. Libre: {} MB. Mínimo requerido: 500 MB.",
                    disk.mount_point().display(), free_mb
                ));
            }
            break;
        }
    }
    if !found_disk {
        if let Some(disk) = disks.list().first() {
            let free_mb = disk.available_space() / (1024 * 1024);
            if free_mb < 500 {
                errors.push(format!(
                    "Espacio en disco insuficiente en el disco principal. Libre: {} MB. Mínimo requerido: 500 MB.",
                    free_mb
                ));
            }
        }
    }

    // 4. Network connectivity (soft warning, not blocking)
    let addr: SocketAddr = "8.8.8.8:53".parse().unwrap();
    if TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_err() {
        warnings.push("No hay conectividad a Internet. TOOL_WEB_SCRAPER no funcionará, pero el resto sí.".to_string());
    }

    // 5. Ollama — REQUIRED (the agent itself depends on this)
    let mut available_models = Vec::new();
    
    let client = reqwest::Client::new();
    match client.get("http://127.0.0.1:11434/api/tags").send().await {
        Ok(res) if res.status().is_success() => {
            if let Ok(json) = res.json::<serde_json::Value>().await {
                if let Some(models) = json.get("models").and_then(|m| m.as_array()) {
                    for model in models {
                        if let Some(name) = model.get("name").and_then(|n| n.as_str()) {
                            available_models.push(name.to_string());
                        }
                    }
                }
            }
            if available_models.is_empty() {
                errors.push("Ollama está instalado, pero no tienes ningún modelo descargado. Descarga al menos qwen2.5-coder:7b con: ollama pull qwen2.5-coder:7b".to_string());
            } else if !available_models.iter().any(|m| m.starts_with("nomic-embed-text")) {
                errors.push("Falta el modelo de embeddings: 'nomic-embed-text'. Necesario para la memoria RAG. Ejecuta: ollama pull nomic-embed-text".to_string());
            }
        }
        Ok(res) => {
            errors.push(format!("El servicio de Ollama devolvió HTTP {}", res.status()));
        }
        Err(e) => {
            errors.push(format!("No se pudo conectar a la API de Ollama (http://127.0.0.1:11434). Asegúrate de que el servicio de Ollama esté corriendo. Detalle: {}", e));
        }
    }

    // Append warnings as context (non-blocking)
    if !warnings.is_empty() {
        // Prepend to available_models metadata so agent gets them in context
        available_models.insert(0, format!("[WARN] {}", warnings.join(" | ")));
    }

    if errors.is_empty() {
        Ok(available_models)
    } else {
        Err(errors)
    }
}
