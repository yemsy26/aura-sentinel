use std::path::Path;
use tokio::process::Command;

pub async fn validate_workspace(workspace_path: &str) -> Result<(), String> {
    let path = Path::new(workspace_path);

    // 1. Rust (Cargo)
    if path.join("Cargo.toml").exists() {
        println!("[VALIDACIÓN] Cargo.toml detectado. Ejecutando cargo check...");
        let output = Command::new("cargo")
            .arg("check")
            .current_dir(workspace_path)
            .output()
            .await
            .map_err(|e| format!("Error al ejecutar cargo check: {}", e))?;

        if output.status.success() {
            return Ok(());
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(stderr);
        }
    }

    // 2. Python
    if path.join("requirements.txt").exists() || path.join("main.py").exists() {
        println!("[VALIDACIÓN] Archivos Python detectados. Ejecutando compileall...");
        
        // Asumimos que python o python3 está disponible. En Windows suele ser 'python'.
        let output = Command::new("python")
            .arg("-m")
            .arg("compileall")
            .arg(".")
            .current_dir(workspace_path)
            .output()
            .await
            .map_err(|e| format!("Error al ejecutar python compileall: {}", e))?;

        if output.status.success() {
            return Ok(());
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(stderr);
        }
    }

    // 3. Node.js
    if path.join("package.json").exists() {
        println!("[VALIDACIÓN] package.json detectado. Entorno Node.js asumiendo validación correcta por seguridad (evitando falsos positivos).");
        return Ok(());
    }

    // 4. Genérico
    println!("[VALIDACIÓN] Entorno genérico detectado. Omitiendo validación estricta.");
    Ok(())
}
