//! runner_generator.rs — Generador de scripts de ejecución (Script-First Approach)
//!
//! En lugar de ejecutar comandos ad-hoc, el agente crea scripts reutilizables:
//! - run_tests.sh / .bat  → Ejecuta suite de tests
//! - build.sh / .bat      → Compila el proyecto
//! - dev.sh / .bat        → Inicia servidor de desarrollo
//! - lint.sh / .bat       → Ejecuta linters
//! - docker.sh / .bat     → Build y run en Docker
//!
//! Cada script incluye:
//   - Shebang correcto
//   - set -euo pipefail (bash) / error handling (bat)
//   - Logging con timestamps
//   - Validación de prerequisitos
//   - Cleanup en trap EXIT

use std::path::{Path, PathBuf};
use tokio::fs;
use chrono;

/// Tipo de runner a generar
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum RunnerType {
    Test,       // run_tests.sh / run_tests.bat
    Build,      // build.sh / build.bat
    Dev,        // dev.sh / dev.bat (servidor desarrollo)
    Lint,       // lint.sh / lint.bat
    Docker,     // docker.sh / docker.bat
    Custom(String), // nombre personalizado
}

impl RunnerType {
    pub fn base_name(&self) -> String {
        match self {
            RunnerType::Test => "run_tests".to_string(),
            RunnerType::Build => "build".to_string(),
            RunnerType::Dev => "dev".to_string(),
            RunnerType::Lint => "lint".to_string(),
            RunnerType::Docker => "docker".to_string(),
            RunnerType::Custom(name) => name.clone(),
        }
    }

    pub fn extensions(&self) -> (&'static str, &'static str) {
        if cfg!(target_os = "windows") {
            (".bat", ".ps1")
        } else {
            (".sh", "")
        }
    }
}

/// Configuración para generar un runner
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RunnerConfig {
    pub runner_type: RunnerType,
    pub language: String,
    pub project_root: PathBuf,
    pub test_command: Option<String>,
    pub build_command: Option<String>,
    pub dev_command: Option<String>,
    pub lint_command: Option<String>,
    pub docker_config: Option<DockerConfig>,
    pub env_vars: Vec<(String, String)>,
    pub prerequisites: Vec<String>, // comandos a verificar antes de ejecutar
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DockerConfig {
    pub dockerfile: String,
    pub image_name: String,
    pub port: Option<u16>,
    pub env_file: Option<String>,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            runner_type: RunnerType::Test,
            language: "unknown".to_string(),
            project_root: PathBuf::from("."),
            test_command: None,
            build_command: None,
            dev_command: None,
            lint_command: None,
            docker_config: None,
            env_vars: Vec::new(),
            prerequisites: Vec::new(),
        }
    }
}

/// Genera el contenido del script para Unix (bash)
fn generate_bash_script(config: &RunnerConfig) -> String {
    let name = config.runner_type.base_name();
    let mut script = String::new();

    // Header
    script.push_str("#!/usr/bin/env bash\n");
    script.push_str(&format!("# {name} — Auto-generado por Aura-Sentinel\n"));
    script.push_str(&format!("# Generado: {}\n", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")));
    script.push_str("set -euo pipefail\n\n");

    // Logging functions
    script.push_str(r#"
log() {
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"
}

error() {
    log "❌ ERROR: $*" >&2
}

success() {
    log "✅ $*"
}

warn() {
    log "⚠️  $*"
}
"#);

    // Prerequisitos
    if !config.prerequisites.is_empty() {
        script.push_str("# Verificar prerequisitos\n");
        for prereq in &config.prerequisites {
            script.push_str(&format!("if ! command -v {} &> /dev/null; then\n", prereq));
            script.push_str(&format!("    error \"{} no encontrado en PATH\"\n", prereq));
            script.push_str("    exit 1\n");
            script.push_str("fi\n");
        }
        script.push_str("\n");
    }

    // Environment variables
    if !config.env_vars.is_empty() {
        script.push_str("# Variables de entorno\n");
        for (k, v) in &config.env_vars {
            script.push_str(&format!("export {}={}\n", k, v));
        }
        script.push_str("\n");
    }

    // Main execution
    script.push_str(&format!("log \"Iniciando {}...\"\n\n", config.runner_type.base_name()));

    // Comando principal según tipo
    match config.runner_type {
        RunnerType::Test => {
            if let Some(cmd) = &config.test_command {
                script.push_str(&format!("log \"Ejecutando tests: {}\"\n", cmd));
                script.push_str(&format!("{}\n", cmd));
                script.push_str("TEST_EXIT=$?\n");
                script.push_str("if [ $TEST_EXIT -eq 0 ]; then\n");
                script.push_str("    success \"Tests pasaron correctamente\"\n");
                script.push_str("else\n");
                script.push_str("    error \"Tests fallaron con código $TEST_EXIT\"\n");
                script.push_str("fi\n");
                script.push_str("exit $TEST_EXIT\n");
            } else {
                script.push_str("error \"No hay comando de test configurado\"\n");
                script.push_str("exit 1\n");
            }
        }
        RunnerType::Build => {
            if let Some(cmd) = &config.build_command {
                script.push_str(&format!("log \"Compilando: {}\"\n", cmd));
                script.push_str(&format!("{}\n", cmd));
                script.push_str("BUILD_EXIT=$?\n");
                script.push_str("if [ $BUILD_EXIT -eq 0 ]; then\n");
                script.push_str("    success \"Build completado\"\n");
                script.push_str("else\n");
                script.push_str("    error \"Build falló con código $BUILD_EXIT\"\n");
                script.push_str("fi\n");
                script.push_str("exit $BUILD_EXIT\n");
            } else {
                script.push_str("error \"No hay comando de build configurado\"\n");
                script.push_str("exit 1\n");
            }
        }
        RunnerType::Dev => {
            if let Some(cmd) = &config.dev_command {
                script.push_str(&format!("log \"Iniciando servidor de desarrollo: {}\"\n", cmd));
                script.push_str(&format!("exec {}\n", cmd));
            } else {
                script.push_str("error \"No hay comando de dev configurado\"\n");
                script.push_str("exit 1\n");
            }
        }
        RunnerType::Lint => {
            if let Some(cmd) = &config.lint_command {
                script.push_str(&format!("log \"Ejecutando linter: {}\"\n", cmd));
                script.push_str(&format!("{}\n", cmd));
                script.push_str("LINT_EXIT=$?\n");
                script.push_str("if [ $LINT_EXIT -eq 0 ]; then\n");
                script.push_str("    success \"Lint pasó sin errores\"\n");
                script.push_str("else\n");
                script.push_str("    error \"Lint falló con código $LINT_EXIT\"\n");
                script.push_str("fi\n");
                script.push_str("exit $LINT_EXIT\n");
            } else {
                script.push_str("error \"No hay comando de lint configurado\"\n");
                script.push_str("exit 1\n");
            }
        }
        RunnerType::Docker => {
            if let Some(docker) = &config.docker_config {
                script.push_str(&format!("log \"Building Docker image: {}\"\n", docker.image_name));
                script.push_str(&format!("docker build -t {} -f {} .\n", docker.image_name, docker.dockerfile));
                script.push_str("BUILD_EXIT=$?\n");
                script.push_str("if [ $BUILD_EXIT -ne 0 ]; then\n");
                script.push_str("    error \"Docker build falló\"\n");
                script.push_str("    exit $BUILD_EXIT\n");
                script.push_str("fi\n");
                
                if let Some(port) = docker.port {
                    script.push_str(&format!("log \"Running container on port {}\"\n", port));
                    script.push_str(&format!("docker run --rm -p {}:{} {}\n", port, port, docker.image_name));
                } else {
                    script.push_str(&format!("docker run --rm {}\n", docker.image_name));
                }
            } else {
                script.push_str("error \"No hay configuración Docker\"\n");
                script.push_str("exit 1\n");
            }
        }
        RunnerType::Custom(ref name) => {
            script.push_str(&format!("# Custom runner: {}\n", name));
            script.push_str("# TODO: Implementar lógica personalizada\n");
            script.push_str("exit 1\n");
        }
    }

    script.push_str("\n# Fin del script\n");
    script
}

/// Genera el contenido del script para Windows (batch)
fn generate_batch_script(config: &RunnerConfig) -> String {
    let name = config.runner_type.base_name();
    let mut script = String::new();

    script.push_str("@echo off\n");
    script.push_str(&format!("REM {name} — Auto-generado por Aura-Sentinel\n"));
    script.push_str(&format!("REM Generado: {}\n", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")));
    script.push_str("\n");

    // Error handling
    script.push_str("setlocal enabledelayedexpansion\n");
    script.push_str("set EXIT_CODE=0\n\n");

    // Logging macro
    script.push_str("REM Logging helper\n");
    script.push_str("set LOG_PREFIX=[%%DATE%% %%TIME%%]\n\n");

    // Prerequisitos
    if !config.prerequisites.is_empty() {
        script.push_str("REM Verificar prerequisitos\n");
        for prereq in &config.prerequisites {
            script.push_str(&format!("where {} >nul 2>nul\n", prereq));
            script.push_str("if errorlevel 1 (\n");
            script.push_str(&format!("    echo %LOG_PREFIX% ERROR: {} no encontrado en PATH\n", prereq));
            script.push_str("    exit /b 1\n");
            script.push_str(")\n\n");
        }
    }

    // Env vars
    if !config.env_vars.is_empty() {
        script.push_str("REM Variables de entorno\n");
        for (k, v) in &config.env_vars {
            script.push_str(&format!("set {}={}\n", k, v));
        }
        script.push_str("\n");
    }

    // Main
    script.push_str(&format!("echo %LOG_PREFIX% Iniciando {}...\n\n", config.runner_type.base_name()));

    match config.runner_type {
        RunnerType::Test => {
            if let Some(cmd) = &config.test_command {
                script.push_str(&format!("echo %LOG_PREFIX% Ejecutando tests: {}\n", cmd));
                script.push_str(&format!("{}\n", cmd));
                script.push_str("set EXIT_CODE=%ERRORLEVEL%\n");
                script.push_str("if %EXIT_CODE% equ 0 (\n");
                script.push_str("    echo %LOG_PREFIX% SUCCESS: Tests pasaron correctamente\n");
                script.push_str(") else (\n");
                script.push_str("    echo %LOG_PREFIX% ERROR: Tests fallaron con código %EXIT_CODE%\n");
                script.push_str(")\n");
                script.push_str("exit /b %EXIT_CODE%\n");
            } else {
                script.push_str("echo %LOG_PREFIX% ERROR: No hay comando de test configurado\n");
                script.push_str("exit /b 1\n");
            }
        }
        RunnerType::Build => {
            if let Some(cmd) = &config.build_command {
                script.push_str(&format!("echo %LOG_PREFIX% Compilando: {}\n", cmd));
                script.push_str(&format!("{}\n", cmd));
                script.push_str("set EXIT_CODE=%ERRORLEVEL%\n");
                script.push_str("if %EXIT_CODE% equ 0 (\n");
                script.push_str("    echo %LOG_PREFIX% SUCCESS: Build completado\n");
                script.push_str(") else (\n");
                script.push_str("    echo %LOG_PREFIX% ERROR: Build falló con código %EXIT_CODE%\n");
                script.push_str(")\n");
                script.push_str("exit /b %EXIT_CODE%\n");
            } else {
                script.push_str("echo %LOG_PREFIX% ERROR: No hay comando de build configurado\n");
                script.push_str("exit /b 1\n");
            }
        }
        RunnerType::Dev => {
            if let Some(cmd) = &config.dev_command {
                script.push_str(&format!("echo %LOG_PREFIX% Iniciando servidor de desarrollo: {}\n", cmd));
                script.push_str(&format!("{}\n", cmd));
            } else {
                script.push_str("echo %LOG_PREFIX% ERROR: No hay comando de dev configurado\n");
                script.push_str("exit /b 1\n");
            }
        }
        RunnerType::Lint => {
            if let Some(cmd) = &config.lint_command {
                script.push_str(&format!("echo %LOG_PREFIX% Ejecutando linter: {}\n", cmd));
                script.push_str(&format!("{}\n", cmd));
                script.push_str("set EXIT_CODE=%ERRORLEVEL%\n");
                script.push_str("if %EXIT_CODE% equ 0 (\n");
                script.push_str("    echo %LOG_PREFIX% SUCCESS: Lint pasó sin errores\n");
                script.push_str(") else (\n");
                script.push_str("    echo %LOG_PREFIX% ERROR: Lint falló con código %EXIT_CODE%\n");
                script.push_str(")\n");
                script.push_str("exit /b %EXIT_CODE%\n");
            } else {
                script.push_str("echo %LOG_PREFIX% ERROR: No hay comando de lint configurado\n");
                script.push_str("exit /b 1\n");
            }
        }
        RunnerType::Docker => {
            if let Some(docker) = &config.docker_config {
                script.push_str(&format!("echo %LOG_PREFIX% Building Docker image: {}\n", docker.image_name));
                script.push_str(&format!("docker build -t {} -f {} .\n", docker.image_name, docker.dockerfile));
                script.push_str("set EXIT_CODE=%ERRORLEVEL%\n");
                script.push_str("if %EXIT_CODE% neq 0 (\n");
                script.push_str("    echo %LOG_PREFIX% ERROR: Docker build falló\n");
                script.push_str("    exit /b %EXIT_CODE%\n");
                script.push_str(")\n");
                
                if let Some(port) = docker.port {
                    script.push_str(&format!("echo %LOG_PREFIX% Running container on port {}\n", port));
                    script.push_str(&format!("docker run --rm -p {}:{} {}\n", port, port, docker.image_name));
                } else {
                    script.push_str(&format!("docker run --rm {}\n", docker.image_name));
                }
            } else {
                script.push_str("echo %LOG_PREFIX% ERROR: No hay configuración Docker\n");
                script.push_str("exit /b 1\n");
            }
        }
        RunnerType::Custom(ref name) => {
            script.push_str(&format!("REM Custom runner: {}\n", name));
            script.push_str("REM TODO: Implementar lógica personalizada\n");
            script.push_str("exit /b 1\n");
        }
    }

    script.push_str("\nREM Fin del script\n");
    script
}

/// Genera el contenido del script para PowerShell
fn generate_powershell_script(config: &RunnerConfig) -> String {
    let name = config.runner_type.base_name();
    let mut script = String::new();

    script.push_str(&format!("# {name} — Auto-generado por Aura-Sentinel\n"));
    script.push_str(&format!("# Generado: {}\n", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")));
    script.push_str("$ErrorActionPreference = \"Stop\"\n");
    script.push_str("Set-StrictMode -Version Latest\n\n");

    // Logging
    script.push_str("function Log { param($msg) Write-Host \"[$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')] $msg\" }\n");
    script.push_str("function Success { param($msg) Write-Host \"✅ $msg\" -ForegroundColor Green }\n");
    script.push_str("function Error { param($msg) Write-Host \"❌ ERROR: $msg\" -ForegroundColor Red >&2 }\n");
    script.push_str("function Warn { param($msg) Write-Host \"⚠️  $msg\" -ForegroundColor Yellow }\n\n");

    // Prerequisitos
    if !config.prerequisites.is_empty() {
        script.push_str("# Verificar prerequisitos\n");
        for prereq in &config.prerequisites {
            script.push_str(&format!("if (-not (Get-Command '{}' -ErrorAction SilentlyContinue)) {{\n", prereq));
            script.push_str(&format!("    Error \"{} no encontrado en PATH\"\n", prereq));
            script.push_str("    exit 1\n");
            script.push_str("}\n");
        }
        script.push_str("\n");
    }

    // Env vars
    if !config.env_vars.is_empty() {
        script.push_str("# Variables de entorno\n");
        for (k, v) in &config.env_vars {
            script.push_str(&format!("$env:{} = '{}'\n", k, v));
        }
        script.push_str("\n");
    }

    // Main
    script.push_str(&format!("Log \"Iniciando {}...\"\n\n", config.runner_type.base_name()));

    match config.runner_type {
        RunnerType::Test => {
            if let Some(cmd) = &config.test_command {
                script.push_str(&format!("Log \"Ejecutando tests: {}\"\n", cmd));
                script.push_str(&format!("{}\n", cmd));
                script.push_str("if ($LASTEXITCODE -eq 0) {\n");
                script.push_str("    Success \"Tests pasaron correctamente\"\n");
                script.push_str("} else {\n");
                script.push_str("    Error \"Tests fallaron con código $LASTEXITCODE\"\n");
                script.push_str("    exit $LASTEXITCODE\n");
                script.push_str("}\n");
            } else {
                script.push_str("Error \"No hay comando de test configurado\"\n");
                script.push_str("exit 1\n");
            }
        }
        RunnerType::Build => {
            if let Some(cmd) = &config.build_command {
                script.push_str(&format!("Log \"Compilando: {}\"\n", cmd));
                script.push_str(&format!("{}\n", cmd));
                script.push_str("if ($LASTEXITCODE -eq 0) {\n");
                script.push_str("    Success \"Build completado\"\n");
                script.push_str("} else {\n");
                script.push_str("    Error \"Build falló con código $LASTEXITCODE\"\n");
                script.push_str("    exit $LASTEXITCODE\n");
                script.push_str("}\n");
            } else {
                script.push_str("Error \"No hay comando de build configurado\"\n");
                script.push_str("exit 1\n");
            }
        }
        RunnerType::Dev => {
            if let Some(cmd) = &config.dev_command {
                script.push_str(&format!("Log \"Iniciando servidor de desarrollo: {}\"\n", cmd));
                script.push_str(&format!("{}\n", cmd));
            } else {
                script.push_str("Error \"No hay comando de dev configurado\"\n");
                script.push_str("exit 1\n");
            }
        }
        RunnerType::Lint => {
            if let Some(cmd) = &config.lint_command {
                script.push_str(&format!("Log \"Ejecutando linter: {}\"\n", cmd));
                script.push_str(&format!("{}\n", cmd));
                script.push_str("if ($LASTEXITCODE -eq 0) {\n");
                script.push_str("    Success \"Lint pasó sin errores\"\n");
                script.push_str("} else {\n");
                script.push_str("    Error \"Lint falló con código $LASTEXITCODE\"\n");
                script.push_str("    exit $LASTEXITCODE\n");
                script.push_str("}\n");
            } else {
                script.push_str("Error \"No hay comando de lint configurado\"\n");
                script.push_str("exit 1\n");
            }
        }
        RunnerType::Docker => {
            if let Some(docker) = &config.docker_config {
                script.push_str(&format!("Log \"Building Docker image: {}\"\n", docker.image_name));
                script.push_str(&format!("docker build -t {} -f {} .\n", docker.image_name, docker.dockerfile));
                script.push_str("if ($LASTEXITCODE -ne 0) {\n");
                script.push_str("    Error \"Docker build falló\"\n");
                script.push_str("    exit $LASTEXITCODE\n");
                script.push_str("}\n");
                
                if let Some(port) = docker.port {
                    script.push_str(&format!("Log \"Running container on port {}\"\n", port));
                    script.push_str(&format!("docker run --rm -p {}:{} {}\n", port, port, docker.image_name));
                } else {
                    script.push_str(&format!("docker run --rm {}\n", docker.image_name));
                }
            } else {
                script.push_str("Error \"No hay configuración Docker\"\n");
                script.push_str("exit 1\n");
            }
        }
        RunnerType::Custom(ref name) => {
            script.push_str(&format!("# Custom runner: {}\n", name));
            script.push_str("# TODO: Implementar lógica personalizada\n");
            script.push_str("exit 1\n");
        }
    }

    script.push_str("\n# Fin del script\n");
    script
}

/// Punto de entrada público: genera todos los runners para un proyecto
pub async fn generate_runners(config: RunnerConfig) -> Result<Vec<PathBuf>, String> {
    let mut generated = Vec::new();
    let project_root = &config.project_root;

    // Asegurar directorio
    fs::create_dir_all(project_root).await
        .map_err(|e| format!("Error creando directorio: {}", e))?;

    // Generar según OS
    if cfg!(target_os = "windows") {
        // .bat
        let bat_path = project_root.join(format!("{}.bat", config.runner_type.base_name()));
        let bat_content = generate_batch_script(&config);
        fs::write(&bat_path, bat_content).await
            .map_err(|e| format!("Error escribiendo .bat: {}", e))?;
        generated.push(bat_path);

        // .ps1
        let ps1_path = project_root.join(format!("{}.ps1", config.runner_type.base_name()));
        let ps1_content = generate_powershell_script(&config);
        fs::write(&ps1_path, ps1_content).await
            .map_err(|e| format!("Error escribiendo .ps1: {}", e))?;
        generated.push(ps1_path);
    } else {
        // .sh
        let sh_path = project_root.join(format!("{}.sh", config.runner_type.base_name()));
        let sh_content = generate_bash_script(&config);
        fs::write(&sh_path, sh_content).await
            .map_err(|e| format!("Error escribiendo .sh: {}", e))?;
        
        // Hacer ejecutable (solo Unix)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&sh_path).await
                .map_err(|e| format!("Error leyendo metadata: {}", e))?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&sh_path, perms).await
                .map_err(|e| format!("Error estableciendo permisos: {}", e))?;
        }
        
        generated.push(sh_path);
    }

    Ok(generated)
}

/// Genera el set completo de runners estándar para un proyecto
pub async fn generate_standard_runners(
    project_root: &Path,
    language: &str,
    test_cmd: Option<String>,
    build_cmd: Option<String>,
    dev_cmd: Option<String>,
    lint_cmd: Option<String>,
) -> Result<Vec<PathBuf>, String> {
    let mut all_generated = Vec::new();

    // Test runner
    if test_cmd.is_some() {
        let config = RunnerConfig {
            runner_type: RunnerType::Test,
            language: language.to_string(),
            project_root: project_root.to_path_buf(),
            test_command: test_cmd,
            prerequisites: detect_prerequisites(language),
            ..Default::default()
        };
        all_generated.extend(generate_runners(config).await?);
    }

    // Build runner
    if build_cmd.is_some() {
        let config = RunnerConfig {
            runner_type: RunnerType::Build,
            language: language.to_string(),
            project_root: project_root.to_path_buf(),
            build_command: build_cmd,
            prerequisites: detect_prerequisites(language),
            ..Default::default()
        };
        all_generated.extend(generate_runners(config).await?);
    }

    // Dev runner
    if dev_cmd.is_some() {
        let config = RunnerConfig {
            runner_type: RunnerType::Dev,
            language: language.to_string(),
            project_root: project_root.to_path_buf(),
            dev_command: dev_cmd,
            prerequisites: detect_prerequisites(language),
            ..Default::default()
        };
        all_generated.extend(generate_runners(config).await?);
    }

    // Lint runner
    if lint_cmd.is_some() {
        let config = RunnerConfig {
            runner_type: RunnerType::Lint,
            language: language.to_string(),
            project_root: project_root.to_path_buf(),
            lint_command: lint_cmd,
            prerequisites: detect_prerequisites(language),
            ..Default::default()
        };
        all_generated.extend(generate_runners(config).await?);
    }

    Ok(all_generated)
}

/// Detecta prerequisitos según lenguaje
fn detect_prerequisites(language: &str) -> Vec<String> {
    match language.to_lowercase().as_str() {
        "rust" => vec!["cargo".to_string()],
        "python" => vec!["python".to_string(), "pip".to_string()],
        "javascript" | "typescript" => vec!["node".to_string(), "npm".to_string()],
        "go" => vec!["go".to_string()],
        "java" => vec!["java".to_string(), "mvn".to_string()],
        "kotlin" => vec!["java".to_string(), "gradle".to_string()],
        "c" | "cpp" => vec!["gcc".to_string(), "make".to_string()],
        "csharp" => vec!["dotnet".to_string()],
        "php" => vec!["php".to_string(), "composer".to_string()],
        "ruby" => vec!["ruby".to_string(), "bundler".to_string()],
        "swift" => vec!["swift".to_string()],
        "dart" => vec!["dart".to_string()],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_bash_test_runner() {
        let config = RunnerConfig {
            runner_type: RunnerType::Test,
            language: "rust".to_string(),
            project_root: PathBuf::from("."),
            test_command: Some("cargo test".to_string()),
            prerequisites: vec!["cargo".to_string()],
            ..Default::default()
        };

        let script = generate_bash_script(&config);
        assert!(script.contains("#!/usr/bin/env bash"));
        assert!(script.contains("set -euo pipefail"));
        assert!(script.contains("cargo test"));
        assert!(script.contains("command -v cargo"));
    }

    #[test]
    fn test_generate_batch_build_runner() {
        let config = RunnerConfig {
            runner_type: RunnerType::Build,
            language: "python".to_string(),
            project_root: PathBuf::from("."),
            build_command: Some("python -m py_compile src/**/*.py".to_string()),
            prerequisites: vec!["python".to_string()],
            ..Default::default()
        };

        let script = generate_batch_script(&config);
        assert!(script.contains("@echo off"));
        assert!(script.contains("where python"));
        assert!(script.contains("python -m py_compile"));
    }

    #[test]
    fn test_detect_prerequisites() {
        assert_eq!(detect_prerequisites("rust"), vec!["cargo"]);
        assert_eq!(detect_prerequisites("python"), vec!["python", "pip"]);
        assert_eq!(detect_prerequisites("javascript"), vec!["node", "npm"]);
    }
}