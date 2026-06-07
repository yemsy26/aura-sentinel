use std::process::Command;
use std::env;

#[cfg(target_os = "windows")]
const SCOOP_INSTALL_CMD: &str = "Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope CurrentUser; Invoke-RestMethod -Uri https://get.scoop.sh | Invoke-Expression";

pub async fn install_dependency(package: &str) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        install_dependency_windows(package).await
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err(format!("El Módulo de Ingeniería de Entorno actualmente sólo soporta autoinstalación en Windows. Por favor, instala '{}' manualmente.", package))
    }
}

#[cfg(target_os = "windows")]
async fn install_dependency_windows(package: &str) -> Result<String, String> {
    // Verificar si scoop existe leyendo el PATH recargado
    hot_reload_path();
    
    let scoop_check = Command::new("powershell")
        .args(&["-NoProfile", "-Command", "Get-Command scoop -ErrorAction SilentlyContinue"])
        .output();
        
    let has_scoop = match scoop_check {
        Ok(out) => out.status.success(),
        Err(_) => false,
    };
    
    if !has_scoop {
        let install_scoop = Command::new("powershell")
            .args(&["-NoProfile", "-Command", SCOOP_INSTALL_CMD])
            .output();
            
        if let Err(e) = install_scoop {
            return Err(format!("Fallo al instalar el gestor de paquetes Scoop: {}", e));
        }
        // Recargar variables de entorno tras instalar scoop
        hot_reload_path();
    }
    
    let mut resolved_package = package.trim().to_lowercase();
    if resolved_package == "node" {
        resolved_package = "nodejs".to_string();
    }
    
    // Ejecutar instalación con scoop
    let install_pkg = Command::new("powershell")
        .args(&["-NoProfile", "-Command", &format!("scoop install {}", resolved_package)])
        .output();
        
    match install_pkg {
        Ok(out) => {
            if out.status.success() {
                // Hot-reload PATH nuevamente para que el ejecutable instalado esté disponible
                hot_reload_path();
                let mut success_msg = format!("Dependencia '{}' instalada correctamente usando Scoop.", package);
                let stdout_str = String::from_utf8_lossy(&out.stdout);
                success_msg.push_str(&format!("\nLogs: {}", stdout_str));
                Ok(success_msg)
            } else {
                let err_str = String::from_utf8_lossy(&out.stderr);
                let out_str = String::from_utf8_lossy(&out.stdout);
                Err(format!("Fallo al instalar '{}' vía scoop.\nStdout: {}\nStderr: {}", package, out_str, err_str))
            }
        },
        Err(e) => Err(format!("Fallo al ejecutar la instalación: {}", e)),
    }
}

pub fn hot_reload_path() {
    #[cfg(target_os = "windows")]
    {
        // Obtener User PATH
        let user_path = Command::new("powershell")
            .args(&["-NoProfile", "-Command", "[Environment]::GetEnvironmentVariable('PATH', 'User')"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
            
        // Obtener Machine PATH
        let machine_path = Command::new("powershell")
            .args(&["-NoProfile", "-Command", "[Environment]::GetEnvironmentVariable('PATH', 'Machine')"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
            
        let mut new_path = String::new();
        if !machine_path.is_empty() {
            new_path.push_str(&machine_path);
        }
        if !user_path.is_empty() {
            if !new_path.is_empty() && !new_path.ends_with(';') {
                new_path.push(';');
            }
            new_path.push_str(&user_path);
        }
        
        if !new_path.is_empty() {
            env::set_var("PATH", new_path);
        }
    }
}
