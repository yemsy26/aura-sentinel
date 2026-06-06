use std::path::Path;
use tokio::process::Command;
use std::process::Stdio;

/// Detects the testing framework and runs the appropriate test suite.
pub async fn run_tests(workspace_path: &str) -> Result<String, String> {
    let path = Path::new(workspace_path);

    // Python - PyTest Detection
    let is_python = path.join("pytest.ini").exists() 
        || path.join("requirements.txt").exists() 
        || path.join("main.py").exists()
        || path.join("backend.py").exists(); // Special case for this project's convention

    if is_python {
        // Run pytest
        let output = Command::new("python")
            .arg("-m")
            .arg("pytest")
            .current_dir(workspace_path)
            .stdin(Stdio::null())
            .output()
            .await
            .map_err(|e| format!("Error executing pytest: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            return Ok(format!("PyTest completado con éxito:\n{}", stdout));
        } else {
            return Err(format!("PyTest Falló:\n{}\n{}", stdout, stderr).trim().to_string());
        }
    }

    // Node.js - Jest/Mocha Detection via package.json
    if path.join("package.json").exists() {
        // Run npm test
        #[cfg(target_os = "windows")]
        let npm_cmd = "npm.cmd";
        #[cfg(not(target_os = "windows"))]
        let npm_cmd = "npm";

        let output = Command::new(npm_cmd)
            .arg("test")
            .current_dir(workspace_path)
            .stdin(Stdio::null())
            .output()
            .await
            .map_err(|e| format!("Error executing npm test: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            return Ok(format!("NPM Test completado con éxito:\n{}", stdout));
        } else {
            return Err(format!("NPM Test Falló:\n{}\n{}", stdout, stderr).trim().to_string());
        }
    }

    // Fallback if no framework detected
    Err("No se detectó un entorno de pruebas conocido (ni PyTest ni npm test). Verifica que haya archivos de configuración de pruebas.".to_string())
}
