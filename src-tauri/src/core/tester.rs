use std::path::Path;
use tokio::process::Command;
use std::process::Stdio;

/// Detects the testing framework and runs the appropriate test suite.
pub async fn run_tests(workspace_path: &str) -> Result<String, String> {
    let path = Path::new(workspace_path);

    if let Some(lang_config) = crate::core::languages::detect_language(path) {
        let (cmd, args) = lang_config.test_cmd;
        
        let output = Command::new(cmd)
            .args(&args)
            .current_dir(workspace_path)
            .stdin(Stdio::null())
            .output()
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    format!("[ENV_FAILURE] No se encontró el comando '{}'. Asegúrate de que el lenguaje está instalado y en el PATH.", cmd)
                } else {
                    format!("Error ejecutando test de {}: {}", lang_config.name, e)
                }
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            return Ok(format!("{} Test completado con éxito:\n{}", lang_config.name, stdout));
        } else {
            return Err(format!("{} Test Falló:\n{}\n{}", lang_config.name, stdout, stderr).trim().to_string());
        }
    }

    // Fallback if no framework detected
    Err("No se detectó un entorno de pruebas conocido (ni Cargo, ni PyTest, ni npm test, ni go test). Verifica que haya archivos de configuración de pruebas.".to_string())
}
