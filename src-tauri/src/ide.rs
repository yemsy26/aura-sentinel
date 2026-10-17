//! Utilidades del IDE alrededor del agente: estado del sistema (CPU, RAM, VRAM), git, puntos de
//! restauración de Agente Local, capturas de ver_pantalla, búsqueda en el proyecto, comandos del
//! usuario y la copia del proyecto al empezar cada misión (base de "Ver cambios" y "Deshacer").

use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tokio::process::Command;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const SNAPSHOT_FILE_LIMIT: u64 = 512 * 1024;
const SNAPSHOT_TOTAL_LIMIT: usize = 40 * 1024 * 1024;

fn quiet(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command.stdin(Stdio::null()).kill_on_drop(true)
}

/// Salida de un programa (stdout y stderr juntos) o None si no arranca o tarda más de `secs`.
async fn output(program: &str, args: &[&str], cwd: &Path, secs: u64) -> Option<(i32, String)> {
    let mut command = Command::new(program);
    quiet(&mut command).args(args).current_dir(cwd);
    let out = tokio::time::timeout(Duration::from_secs(secs), command.output()).await.ok()?.ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Some((out.status.code().unwrap_or(-1), text))
}

// ── Sistema ─────────────────────────────────────────────────────────────────

fn parse_gpu(csv: &str) -> Option<Value> {
    let fields: Vec<f64> = csv.lines().next()?.split(',').map(|f| f.trim().parse().ok()).collect::<Option<_>>()?;
    let [used, total, util] = fields[..] else { return None };
    Some(json!({"used": used, "total": total, "util": util}))
}

/// CPU y RAM del sistema y VRAM de la GPU NVIDIA (None sin nvidia-smi).
#[tauri::command]
pub async fn get_system_stats() -> Value {
    static SYSTEM: OnceLock<Mutex<sysinfo::System>> = OnceLock::new();
    let (cpu, used, total) = {
        let mut sys = SYSTEM.get_or_init(|| Mutex::new(sysinfo::System::new())).lock().unwrap();
        sys.refresh_cpu_usage(); // la primera lectura es 0; se pide cada pocos segundos
        sys.refresh_memory();
        (sys.global_cpu_usage(), sys.used_memory(), sys.total_memory())
    };
    let gb = |bytes: u64| bytes as f64 / 1_073_741_824.0;
    let query = ["--query-gpu=memory.used,memory.total,utilization.gpu", "--format=csv,noheader,nounits"];
    let gpu = output("nvidia-smi", &query, &std::env::temp_dir(), 5).await.and_then(|(_, text)| parse_gpu(&text));
    json!({"cpu": cpu, "ramUsed": gb(used), "ramTotal": gb(total), "gpu": gpu})
}

// ── Git ─────────────────────────────────────────────────────────────────────

/// Rama, raíz del repositorio, archivos cambiados (`git status --porcelain`, rutas relativas a la raíz,
/// que puede estar por encima de la carpeta abierta) y últimos commits; `branch` null fuera de git.
#[tauri::command]
pub async fn get_git_info(workspace_path: String) -> Value {
    let dir = Path::new(&workspace_path);
    let Some((0, branch)) = output("git", &["rev-parse", "--abbrev-ref", "HEAD"], dir, 10).await else {
        return json!({"branch": null, "root": null, "changes": [], "log": []});
    };
    let root = output("git", &["rev-parse", "--show-toplevel"], dir, 10).await.map(|o| o.1).unwrap_or_default();
    let root = root.trim().replace('/', "\\");
    let status = output("git", &["status", "--porcelain=v1", "-uall"], dir, 20).await.map(|o| o.1).unwrap_or_default();
    let changes: Vec<Value> = status
        .lines()
        .filter(|line| line.len() > 3)
        .map(|line| json!({"status": line[..2].trim(), "path": line[3..].trim_matches('"')}))
        .collect();
    let log = output("git", &["log", "--oneline", "-12"], dir, 10).await.map(|o| o.1).unwrap_or_default();
    json!({"branch": branch.trim(), "root": root, "changes": changes, "log": log.lines().collect::<Vec<_>>()})
}

// ── Puntos de restauración de Agente Local ──────────────────────────────────

/// `.agente/checkpoints.json`: versiones que pasaron verificar sin errores (el agente guarda 3).
#[tauri::command]
pub fn list_checkpoints(workspace_path: String) -> Vec<Value> {
    let path = Path::new(&workspace_path).join(".agente").join("checkpoints.json");
    let list: Vec<Value> =
        std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
    list.iter()
        .enumerate()
        .map(|(i, cp)| {
            let files: Vec<&String> = cp["archivos"].as_object().map(|m| m.keys().collect()).unwrap_or_default();
            json!({"numero": i + 1, "paso": cp["paso"], "fecha": cp["fecha"], "archivos": files})
        })
        .rev()
        .collect()
}

// ── Capturas y vista previa ─────────────────────────────────────────────────

/// Imagen local como data URL (capturas de ver_pantalla). Solo imágenes de hasta 15 MB.
#[tauri::command]
pub fn read_image_data_url(path: String) -> Result<String, String> {
    let path = PathBuf::from(path);
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => return Err("Solo se muestran imágenes PNG, JPG, WebP o GIF.".into()),
    };
    let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
    if size > 15 * 1024 * 1024 {
        return Err("La imagen pasa de 15 MB.".into());
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

pub(crate) fn is_local_url(url: &str) -> bool {
    let rest = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://")).unwrap_or_default();
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host =
        host.rsplit_once(':').filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit())).map_or(host, |(h, _)| h);
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

/// Abre en el navegador del sistema una página local (servidores que lanzó el agente).
#[tauri::command]
pub async fn open_external(url: String) -> Result<(), String> {
    if !is_local_url(&url) {
        return Err("Solo se abren direcciones locales (http://localhost:PUERTO).".into());
    }
    let mut command = Command::new("rundll32");
    quiet(&mut command).args(["url.dll,FileProtocolHandler", &url]);
    command.spawn().map(|_| ()).map_err(|e| e.to_string())
}

// ── Búsqueda y comandos ─────────────────────────────────────────────────────

/// Líneas que contienen `query` (sin distinguir mayúsculas) en los archivos de texto del proyecto.
#[tauri::command]
pub async fn search_workspace(workspace_path: String, query: String) -> Vec<Value> {
    let needle = query.trim().to_lowercase();
    if needle.len() < 2 {
        return Vec::new();
    }
    let root = PathBuf::from(&workspace_path);
    tokio::task::spawn_blocking(move || {
        let mut hits = Vec::new();
        for entry in crate::workspace::walk(&root) {
            let path = entry.path();
            if !path.is_file() || std::fs::metadata(path).map_or(true, |m| m.len() > 1024 * 1024) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(path) else { continue }; // binarios: no son UTF-8
            for (n, line) in text.lines().enumerate() {
                if line.to_lowercase().contains(&needle) {
                    let preview: String = line.trim().chars().take(200).collect();
                    hits.push(json!({"path": path.to_string_lossy(), "line": n + 1, "text": preview}));
                    if hits.len() >= 300 {
                        return hits;
                    }
                }
            }
        }
        hits
    })
    .await
    .unwrap_or_default()
}

/// Comando del usuario en la carpeta del proyecto (`cmd /C`), con 2 minutos como máximo.
#[tauri::command]
pub async fn run_command(workspace_path: String, command: String) -> Value {
    let started = std::time::Instant::now();
    let result = output("cmd", &["/D", "/S", "/C", &command], Path::new(&workspace_path), 120).await;
    let (code, text) = result.unwrap_or((-1, "No terminó en 2 minutos o no se pudo ejecutar.".into()));
    let start = text.char_indices().rev().nth(20_000).map_or(0, |(i, _)| i);
    json!({"code": code, "output": &text[start..], "ms": started.elapsed().as_millis() as u64})
}

// ── Copia del proyecto al empezar la misión ─────────────────────────────────

fn snapshot() -> &'static Mutex<HashMap<PathBuf, String>> {
    static SNAPSHOT: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();
    SNAPSHOT.get_or_init(|| Mutex::new(HashMap::new()))
}

fn normalized(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().replace('/', "\\").to_lowercase())
}

/// Guarda el texto de los archivos del proyecto antes de que el agente los toque.
pub fn snapshot_workspace(workspace: &str) {
    let mut files = HashMap::new();
    let mut total = 0;
    for entry in crate::workspace::walk(Path::new(workspace)) {
        let path = entry.path();
        if !path.is_file() || std::fs::metadata(path).map_or(true, |m| m.len() > SNAPSHOT_FILE_LIMIT) {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(path) {
            total += text.len();
            if total > SNAPSHOT_TOTAL_LIMIT {
                break;
            }
            files.insert(normalized(path), text);
        }
    }
    *snapshot().lock().unwrap() = files;
}

/// Contenido del archivo al empezar la última misión; None si el agente lo creó.
#[tauri::command]
pub fn get_mission_baseline(path: String) -> Option<String> {
    snapshot().lock().unwrap().get(&normalized(Path::new(&path))).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_line_is_parsed() {
        assert_eq!(parse_gpu("7183, 8188, 12\n").unwrap(), json!({"used": 7183.0, "total": 8188.0, "util": 12.0}));
        assert!(parse_gpu("[N/A], 8188, 0").is_none());
    }

    #[test]
    fn only_local_urls_open() {
        assert!(is_local_url("http://localhost:8000/index.html"));
        assert!(is_local_url("http://127.0.0.1:5173"));
        assert!(!is_local_url("https://example.com"));
        assert!(!is_local_url("http://localhost.evil.com/"));
        assert!(!is_local_url("file:///C:/Windows"));
    }

    #[test]
    fn checkpoints_newest_first_and_baseline_snapshot() {
        let ws = std::env::temp_dir().join(format!("aura_ide_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(ws.join(".agente")).unwrap();
        std::fs::write(
            ws.join(".agente").join("checkpoints.json"),
            r#"[{"paso": 4, "fecha": "2026-10-16 10:00", "archivos": {"a.py": "x"}},
                {"paso": 9, "fecha": "2026-10-16 10:30", "archivos": {"a.py": "y", "b.py": "z"}}]"#,
        )
        .unwrap();
        let list = list_checkpoints(ws.to_string_lossy().into_owned());
        assert_eq!(list[0]["numero"], 2);
        assert_eq!(list[0]["archivos"], json!(["a.py", "b.py"]));
        std::fs::write(ws.join("a.py"), "antes").unwrap();
        snapshot_workspace(&ws.to_string_lossy());
        std::fs::write(ws.join("a.py"), "después").unwrap();
        let a = ws.join("a.py").to_string_lossy().replace('\\', "/");
        assert_eq!(get_mission_baseline(a).as_deref(), Some("antes"));
        assert!(get_mission_baseline(ws.join("nuevo.py").to_string_lossy().into_owned()).is_none());
        let _ = std::fs::remove_dir_all(&ws);
    }
}
