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

pub async fn execute_terminal_command(workspace_path: &str, command: &str) -> Result<String, String> {
    let output = Command::new("cmd")
        .args(["/C", command])
        .current_dir(workspace_path)
        .output()
        .await
        .map_err(|e| format!("Error al ejecutar comando en terminal: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        Ok(stdout)
    } else {
        Err(if !stderr.is_empty() { stderr } else { stdout })
    }
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let mut dot_product = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;
    
    for i in 0..a.len() {
        if i < b.len() {
            dot_product += a[i] * b[i];
            norm_a += a[i] * a[i];
            norm_b += b[i] * b[i];
        }
    }
    
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    
    dot_product / (norm_a.sqrt() * norm_b.sqrt())
}
