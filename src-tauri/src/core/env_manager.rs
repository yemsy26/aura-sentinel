use std::process::Command;
use std::env;

// ── Scoop bootstrap command ─────────────────────────────────────────────────
#[cfg(target_os = "windows")]
const SCOOP_INSTALL_CMD: &str =
    "Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope CurrentUser -Force; \
     Invoke-RestMethod -Uri https://get.scoop.sh | Invoke-Expression";

// ── Package-manager availability ────────────────────────────────────────────

/// Which package managers are currently available on the host.
#[derive(Debug, Clone, PartialEq)]
pub enum PackageManager {
    Scoop,
    Winget,
    None,
}

/// Detects which package manager is available, preferring Scoop over winget.
/// Hot-reloads the PATH before checking so freshly-installed managers are seen.
pub fn detect_package_manager() -> PackageManager {
    #[cfg(target_os = "windows")]
    {
        hot_reload_path();

        // 1. Scoop
        let scoop_ok = Command::new("powershell")
            .args(&[
                "-NoProfile",
                "-Command",
                "Get-Command scoop -ErrorAction SilentlyContinue",
            ])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if scoop_ok {
            return PackageManager::Scoop;
        }

        // 2. winget  (ships with Windows 10 1709+ App Installer)
        let winget_ok = Command::new("winget")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if winget_ok {
            return PackageManager::Winget;
        }

        PackageManager::None
    }
    #[cfg(not(target_os = "windows"))]
    {
        PackageManager::None
    }
}

// ── Public entry point ───────────────────────────────────────────────────────

/// Installs a system-level dependency by name.
/// The caller passes ONLY the package name (e.g. `"go"`, `"node"`, `"python"`).
/// The function resolves the correct package manager name and runs the install.
pub async fn install_dependency(package: &str) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        install_dependency_windows(package).await
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err(format!(
            "El Módulo de Ingeniería de Entorno actualmente sólo soporta auto-instalación \
             en Windows. Por favor, instala '{}' manualmente.",
            package
        ))
    }
}

// ── Windows implementation ───────────────────────────────────────────────────

#[cfg(target_os = "windows")]
async fn install_dependency_windows(package_raw: &str) -> Result<String, String> {
    // Sanitize: accept ONLY the package name (last word), never a full command.
    let package = sanitize_package_name(package_raw);

    // Normalise common aliases (node → nodejs for Scoop, node for winget)
    let (scoop_pkg, winget_pkg) = resolve_package_aliases(&package);

    let pm = detect_package_manager();

    match pm {
        PackageManager::Scoop => {
            install_with_scoop(&scoop_pkg).await
        }
        PackageManager::Winget => {
            install_with_winget(&winget_pkg).await
        }
        PackageManager::None => {
            // Try to bootstrap Scoop first, then retry
            eprintln!("[ENV_MANAGER] No package manager found. Bootstrapping Scoop...");
            bootstrap_scoop().await?;
            install_with_scoop(&scoop_pkg).await
        }
    }
}

/// Extracts only the final word of the input to prevent shell injection.
/// e.g. "scoop install python" → "python"
fn sanitize_package_name(raw: &str) -> String {
    raw.split_whitespace()
        .last()
        .unwrap_or(raw)
        .trim()
        .to_lowercase()
}

/// Returns (scoop_name, winget_name) for well-known aliases.
fn resolve_package_aliases(pkg: &str) -> (String, String) {
    let (scoop, winget) = match pkg {
        "node" | "nodejs" => ("nodejs", "OpenJS.NodeJS"),
        "python" | "python3" => ("python", "Python.Python.3"),
        "go" | "golang"  => ("go", "GoLang.Go"),
        "rust"           => ("rustup-init", "Rustlang.Rustup"),
        "java"           => ("openjdk", "EclipseAdoptium.Temurin.21.JDK"),
        "dart"           => ("dart", "Dart.Dart"),
        "flutter"        => ("flutter", "Google.Flutter"),
        "php"            => ("php", "PHP.PHP"),
        "git"            => ("git", "Git.Git"),
        "gh"             => ("gh", "GitHub.cli"),
        other            => (other, other),
    };
    (scoop.to_string(), winget.to_string())
}

// ── Scoop helpers ────────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
async fn install_with_scoop(package: &str) -> Result<String, String> {
    eprintln!("[ENV_MANAGER] Installing '{}' via Scoop...", package);
    let result = Command::new("powershell")
        .args(&[
            "-NoProfile",
            "-Command",
            &format!("scoop install {}", package),
        ])
        .output()
        .map_err(|e| format!("Fallo al ejecutar scoop install: {}", e))?;

    hot_reload_path();

    if result.status.success() {
        let stdout = String::from_utf8_lossy(&result.stdout);
        Ok(format!(
            "✅ '{}' instalado correctamente vía Scoop. PATH recargado en caliente.\nLogs: {}",
            package, stdout
        ))
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let stdout = String::from_utf8_lossy(&result.stdout);
        Err(format!(
            "❌ Scoop falló al instalar '{}'.\nStdout: {}\nStderr: {}",
            package, stdout, stderr
        ))
    }
}

#[cfg(target_os = "windows")]
async fn bootstrap_scoop() -> Result<(), String> {
    let result = Command::new("powershell")
        .args(&["-NoProfile", "-Command", SCOOP_INSTALL_CMD])
        .output()
        .map_err(|e| format!("Fallo al bootstrapear Scoop: {}", e))?;

    hot_reload_path();

    if result.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        Err(format!(
            "No se pudo instalar Scoop automáticamente.\nDetalle: {}\n\
             Por favor instala Scoop manualmente desde https://scoop.sh y reintenta.",
            stderr
        ))
    }
}

// ── Winget helpers ───────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
async fn install_with_winget(package_id: &str) -> Result<String, String> {
    eprintln!("[ENV_MANAGER] Installing '{}' via winget...", package_id);
    // --accept-* flags make winget non-interactive
    let result = Command::new("winget")
        .args(&[
            "install",
            "--id", package_id,
            "--silent",
            "--accept-package-agreements",
            "--accept-source-agreements",
        ])
        .output()
        .map_err(|e| format!("Fallo al ejecutar winget install: {}", e))?;

    hot_reload_path();

    if result.status.success() {
        let stdout = String::from_utf8_lossy(&result.stdout);
        Ok(format!(
            "✅ '{}' instalado correctamente vía winget. PATH recargado en caliente.\nLogs: {}",
            package_id, stdout
        ))
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let stdout = String::from_utf8_lossy(&result.stdout);
        Err(format!(
            "❌ winget falló al instalar '{}'.\nStdout: {}\nStderr: {}",
            package_id, stdout, stderr
        ))
    }
}

// ── PATH hot-reload (public, called from agent.rs after any install) ─────────

/// Merges Machine + User PATH from the Windows registry into the current process
/// environment so newly-installed binaries are immediately available without
/// restarting the shell.
pub fn hot_reload_path() {
    #[cfg(target_os = "windows")]
    {
        let machine_path = read_env_var_from_registry("Machine");
        let user_path    = read_env_var_from_registry("User");

        // Also pick up Scoop's shims directory explicitly in case it was just installed
        let scoop_shims = {
            let profile = env::var("USERPROFILE").unwrap_or_default();
            format!("{}\\scoop\\shims", profile)
        };

        let mut parts: Vec<String> = Vec::new();

        if !machine_path.is_empty() {
            parts.push(machine_path);
        }
        if !user_path.is_empty() {
            parts.push(user_path);
        }
        // Prepend scoop shims so they override stale system entries
        parts.insert(0, scoop_shims);

        // De-duplicate while preserving order
        let mut seen = std::collections::HashSet::new();
        let merged: String = parts
            .iter()
            .flat_map(|seg| seg.split(';'))
            .filter(|entry| {
                let e = entry.trim().to_lowercase();
                !e.is_empty() && seen.insert(e)
            })
            .collect::<Vec<_>>()
            .join(";");

        if !merged.is_empty() {
            env::set_var("PATH", &merged);
        }
    }
}

#[cfg(target_os = "windows")]
fn read_env_var_from_registry(scope: &str) -> String {
    let cmd = format!(
        "[Environment]::ExpandEnvironmentVariables(\
         [Environment]::GetEnvironmentVariable('PATH', '{}'))",
        scope
    );
    Command::new("powershell")
        .args(&["-NoProfile", "-Command", &cmd])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .replace('\r', "")
                .replace('\n', "")
                .trim()
                .to_string()
        })
        .unwrap_or_default()
}
