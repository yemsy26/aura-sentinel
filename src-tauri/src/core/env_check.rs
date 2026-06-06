use std::path::Path;
use std::net::{TcpStream, SocketAddr};
use std::time::Duration;
use tokio::process::Command;
use sysinfo::Disks;

/// Realiza una auditoría ambiental rápida antes de que el agente comience a trabajar.
/// Retorna `Ok(modelos_disponibles)` si todo está correcto, o `Err` con una lista de problemas encontrados.
pub async fn validate_environment(workspace_path: &str) -> Result<Vec<String>, Vec<String>> {
    let mut errors = Vec::new();

    // 1. Verificación de comandos en el PATH
    let commands_to_check = vec!["git", "python", "npm", "cargo"];
    for cmd in commands_to_check {
        #[cfg(target_os = "windows")]
        let check_cmd = format!("where {}", cmd);
        #[cfg(not(target_os = "windows"))]
        let check_cmd = format!("which {}", cmd);

        #[cfg(target_os = "windows")]
        let shell = "cmd";
        #[cfg(target_os = "windows")]
        let shell_arg = "/C";

        #[cfg(not(target_os = "windows"))]
        let shell = "sh";
        #[cfg(not(target_os = "windows"))]
        let shell_arg = "-c";

        let result = Command::new(shell)
            .args([shell_arg, &check_cmd])
            .output()
            .await;

        if let Ok(output) = result {
            if !output.status.success() {
                errors.push(format!("No encuentro '{}' en el PATH del sistema.", cmd));
            }
        } else {
            errors.push(format!("Fallo al intentar verificar '{}' en el PATH.", cmd));
        }
    }

    // 2. Permisos de escritura en el workspace
    let test_file = Path::new(workspace_path).join(".aura_test_write");
    if let Err(_) = std::fs::write(&test_file, "test") {
        errors.push(format!("No tengo permisos de escritura en el directorio: {}", workspace_path));
    } else {
        let _ = std::fs::remove_file(&test_file);
    }

    // 3. Espacio libre en disco (> 500MB)
    let disks = Disks::new_with_refreshed_list();
    let workspace_path_obj = Path::new(workspace_path);
    let mut found_disk = false;

    // Buscar el disco que contiene el workspace
    for disk in disks.list() {
        if workspace_path_obj.starts_with(disk.mount_point()) {
            found_disk = true;
            let free_mb = disk.available_space() / (1024 * 1024);
            if free_mb < 500 {
                errors.push(format!("Espacio en disco insuficiente en {}. Libre: {} MB. Mínimo requerido: 500 MB.", disk.mount_point().display(), free_mb));
            }
            break;
        }
    }

    // Si no encontramos el disco exacto (por rutas relativas raras), comprobamos el primero (normalmente C:/)
    if !found_disk {
        if let Some(disk) = disks.list().first() {
            let free_mb = disk.available_space() / (1024 * 1024);
            if free_mb < 500 {
                errors.push(format!("Espacio en disco insuficiente en el disco principal. Libre: {} MB. Mínimo requerido: 500 MB.", free_mb));
            }
        }
    }

    // 4. Conectividad Básica de Red (Ping TCP a 8.8.8.8 puerto 53 con timeout de 1 segundo)
    let addr: SocketAddr = "8.8.8.8:53".parse().unwrap();
    if let Err(_) = TcpStream::connect_timeout(&addr, Duration::from_secs(1)) {
        errors.push("No hay conectividad a Internet. Falló el ping TCP a 8.8.8.8:53.".to_string());
    }

    // 5. Verificar modelos de Ollama disponibles
    let mut available_models = Vec::new();
    let ollama_cmd = Command::new("ollama").arg("list").output().await;
    
    match ollama_cmd {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().skip(1) { // Skip header
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(model_name) = parts.first() {
                    available_models.push(model_name.to_string());
                }
            }
            if available_models.is_empty() {
                errors.push("Ollama está instalado, pero no tienes ningún modelo descargado. Debes descargar los modelos requeridos.".to_string());
            }
        }
        _ => {
            errors.push("Fallo al ejecutar 'ollama list'. Asegúrate de que Ollama esté instalado y el servicio esté corriendo.".to_string());
        }
    }

    if errors.is_empty() {
        Ok(available_models)
    } else {
        Err(errors)
    }
}
