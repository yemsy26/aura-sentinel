//! Utilidades del IDE alrededor del agente: estado del sistema (CPU, RAM, VRAM), git, puntos de
//! restauración de Agente Local, capturas de ver_pantalla, búsqueda en el proyecto, comandos del
//! usuario y la copia del proyecto al empezar cada misión (base de "Ver cambios" y "Deshacer").

use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
#[cfg(windows)]
use std::os::windows::process::CommandExt as _;
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

// Leer un repositorio no ejecuta nada de lo que trae en `.git` (m4209): una carpeta copiada o descomprimida llega con
// su configuración y sus hooks. Es lo mismo que hace `motor/agente/git_seguro.py` en las lecturas del agente.
const GIT_OFF: [(&str, &str); 6] = [
    ("safe.bareRepository", "explicit"),
    ("core.fsmonitor", "false"),
    ("log.showSignature", "false"),
    ("status.submoduleSummary", "false"),
    ("diff.submodule", "short"),
    ("core.hooksPath", "<sin-hooks>"), // nombre imposible en Windows
];

/// git sin red y con esa configuración más `config`: (código, stdout) o None si no arranca o tarda más de `secs`.
async fn git(args: &[&str], cwd: &Path, config: &[(String, String)], secs: u64) -> Option<(i32, Vec<u8>)> {
    let mut command = Command::new("git");
    quiet(&mut command).arg("--no-pager").args(args).current_dir(cwd);
    // Un clon parcial descarga lo que le falta con la orden que diga su configuración.
    command.envs([("GIT_NO_LAZY_FETCH", "1"), ("GIT_ALLOW_PROTOCOL", "ninguno"), ("GIT_TERMINAL_PROMPT", "0")]);
    let mut n: usize = std::env::var("GIT_CONFIG_COUNT").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    for (key, value) in GIT_OFF.into_iter().chain(config.iter().map(|(k, v)| (k.as_str(), v.as_str()))) {
        command.env(format!("GIT_CONFIG_KEY_{n}"), key).env(format!("GIT_CONFIG_VALUE_{n}"), value);
        n += 1;
    }
    command.env("GIT_CONFIG_COUNT", n.to_string());
    let out = tokio::time::timeout(Duration::from_secs(secs), command.output()).await.ok()?.ok()?;
    Some((out.status.code().unwrap_or(-1), out.stdout))
}

/// Claves que nombran un programa y no se apagan con una opción fija (filtros, textconv, diff externo, gpg).
fn is_program(key: &str) -> bool {
    let name = key.rsplit('.').next().unwrap_or_default();
    match key.split('.').next().unwrap_or_default() {
        "filter" => matches!(name, "clean" | "smudge" | "process" | "required"),
        "lfs" => key.starts_with("lfs.extension.") && matches!(name, "clean" | "smudge"),
        "diff" => key == "diff.external" || matches!(name, "command" | "textconv"),
        "gpg" => name == "program",
        _ => false,
    }
}

/// De `git config --show-scope --null`: lo que nombra el repositorio, devuelto a lo que digan el sistema y el usuario
/// (su git-lfs sigue funcionando) o apagado si ellos no dicen nada.
fn repo_programs(listing: &str) -> Vec<(String, String)> {
    let (mut user, mut repo) = (HashMap::new(), Vec::new());
    let mut fields = listing.split('\0');
    while let (Some(scope), Some(entry)) = (fields.next(), fields.next()) {
        let (key, value) = entry.split_once('\n').unwrap_or((entry, ""));
        if matches!(scope, "system" | "global" | "command") {
            user.insert(key, value);
        } else if is_program(key) && !repo.contains(&key) {
            repo.push(key);
        }
    }
    let fallback = |key: &str| match key {
        "gpg.program" | "gpg.openpgp.program" => "gpg",
        "gpg.x509.program" => "gpgsm",
        "gpg.ssh.program" => "ssh-keygen",
        _ if key.ends_with(".required") => "false",
        _ if key.ends_with(".textconv") => "cat",
        _ => "",
    };
    repo.into_iter()
        .map(|key| (key.to_string(), user.get(key).copied().unwrap_or_else(|| fallback(key)).to_string()))
        .collect()
}

/// Rama, raíz del repositorio, archivos cambiados (`git status --porcelain`, rutas relativas a la raíz,
/// que puede estar por encima de la carpeta abierta) y últimos commits; `branch` null fuera de git.
#[tauri::command]
pub async fn get_git_info(workspace_path: String) -> Value {
    let none = json!({"branch": null, "root": null, "changes": [], "log": []});
    let dir = Path::new(&workspace_path);
    // Código 1: no hay ninguno. Si no se puede saber qué trae el repositorio, no se lee.
    let query = ["config", "--show-scope", "--null", "--get-regexp", r"^(filter|lfs\.extension|gpg|diff)\."];
    let Some((0 | 1, listing)) = git(&query, dir, &[], 10).await else { return none };
    let Ok(listing) = String::from_utf8(listing) else { return none };
    let off = repo_programs(&listing);
    let text = |out: Vec<u8>| String::from_utf8_lossy(&out).into_owned();
    let Some((0, branch)) = git(&["rev-parse", "--abbrev-ref", "HEAD"], dir, &off, 10).await else { return none };
    let root = git(&["rev-parse", "--show-toplevel"], dir, &off, 10).await.map(|o| text(o.1)).unwrap_or_default();
    let root = root.trim().replace('/', "\\");
    // Mirar dentro de un submódulo lanza git con la configuración del submódulo.
    let args = ["status", "--porcelain=v1", "-uall", "--ignore-submodules=dirty"];
    let status = git(&args, dir, &off, 20).await.map(|o| text(o.1)).unwrap_or_default();
    let changes: Vec<Value> = status
        .lines()
        .filter(|line| line.len() > 3)
        .map(|line| json!({"status": line[..2].trim(), "path": line[3..].trim_matches('"')}))
        .collect();
    let args = ["log", "--oneline", "-12", "--no-ext-diff", "--no-textconv"];
    let log = git(&args, dir, &off, 10).await.map(|o| text(o.1)).unwrap_or_default();
    json!({"branch": text(branch).trim(), "root": root, "changes": changes, "log": log.lines().collect::<Vec<_>>()})
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
    // Con `quiet()` (kill_on_drop) rundll32 moría al soltar el proceso, antes de abrir nada: se lanza con std y se suelta.
    std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", &url])
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Archivo que existe y está dentro de la carpeta de trabajo, con la ruta normal de Windows (sin el prefijo `\\?\`).
fn inside_workspace(workspace: &str, path: &str) -> Result<PathBuf, String> {
    let root = std::fs::canonicalize(workspace).map_err(|_| "No encuentro la carpeta de trabajo.".to_string())?;
    let file = std::fs::canonicalize(path).map_err(|_| "El archivo ya no existe.".to_string())?;
    if file == root || !file.starts_with(&root) {
        return Err("Solo se muestran archivos de la carpeta de trabajo.".into());
    }
    Ok(PathBuf::from(file.to_string_lossy().trim_start_matches(r"\\?\")))
}

/// Muestra en el Explorador de Windows, seleccionado, un archivo del proyecto (el .bat que abre lo que hizo el agente).
/// Solo rutas que existen dentro de la carpeta de trabajo: cualquier otra devuelve un error y no ejecuta nada.
#[tauri::command]
pub async fn reveal_in_folder(workspace_path: String, path: String) -> Result<(), String> {
    let file = inside_workspace(&workspace_path, &path)?;
    // Con `quiet()` (kill_on_drop) el Explorador moría al soltar el proceso y no abría ninguna ventana (4.13.9): se lanza con std y se suelta.
    let mut command = std::process::Command::new("explorer.exe");
    #[cfg(windows)]
    command.raw_arg(format!("/select,\"{}\"", file.display()));
    #[cfg(not(windows))]
    command.arg(file.parent().unwrap_or(&file));
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
    fn only_files_inside_the_workspace_are_revealed() {
        let base = std::env::temp_dir().join(format!("aura_reveal_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let ws = base.join("Mi Trabajo");
        std::fs::create_dir_all(ws.join("sub")).unwrap();
        std::fs::write(ws.join("run.bat"), "@echo off").unwrap();
        std::fs::write(ws.join("sub").join("a.bat"), "x").unwrap();
        std::fs::write(base.join("fuera.bat"), "x").unwrap();
        let hermana = base.join("Mi Trabajo2");
        std::fs::create_dir_all(&hermana).unwrap();
        std::fs::write(hermana.join("run.bat"), "x").unwrap();
        let w = ws.to_string_lossy().into_owned();
        let dentro = inside_workspace(&w, &ws.join("run.bat").to_string_lossy()).unwrap();
        assert!(dentro.ends_with("run.bat") && !dentro.to_string_lossy().starts_with(r"\\?\"));
        assert!(inside_workspace(&w, &ws.join("sub").join("a.bat").to_string_lossy().replace('\\', "/")).is_ok());
        #[cfg(windows)]
        assert!(inside_workspace(&w, &ws.join("sub").join("a.bat").to_string_lossy().to_uppercase()).is_ok());
        assert!(inside_workspace(&w, &base.join("fuera.bat").to_string_lossy()).is_err());
        assert!(
            inside_workspace(&w, &ws.join("sub").join("..").join("..").join("fuera.bat").to_string_lossy()).is_err()
        );
        assert!(inside_workspace(&w, &hermana.join("run.bat").to_string_lossy()).is_err());
        assert!(inside_workspace(&w, &ws.join("no_existe.bat").to_string_lossy()).is_err());
        assert!(inside_workspace(&w, &w).is_err());
        assert!(
            inside_workspace(&base.join("no_hay").to_string_lossy(), &ws.join("run.bat").to_string_lossy()).is_err()
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Abre de verdad el Explorador (deja una ventana): `cargo test -- --ignored reveal_opens_explorer`, y mirar que se abre con run.bat elegido.
    #[test]
    #[ignore]
    fn reveal_opens_explorer() {
        let ws = std::env::temp_dir().join("aura_reveal_real").join("Mi Trabajo");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(ws.join("run.bat"), "@echo off").unwrap();
        let (w, p) = (ws.to_string_lossy().into_owned(), ws.join("run.bat").to_string_lossy().into_owned());
        tokio::runtime::Runtime::new().unwrap().block_on(reveal_in_folder(w, p)).unwrap();
    }

    /// m4209: abrir una carpeta copiada con su `.git` ejecutaba las órdenes que trae.
    #[test]
    fn git_info_runs_nothing_the_repository_brings() {
        let base = std::env::temp_dir().join(format!("aura_git_{}", std::process::id()));
        let repo = base.join("repo");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| std::process::Command::new("git").args(args).current_dir(&repo).output();
        if git(&["init", "-q"]).is_err() {
            return; // sin git no hay nada que probar
        }
        std::fs::write(repo.join(".gitattributes"), "*.txt filter=x\n").unwrap();
        std::fs::write(repo.join("a.txt"), "1\n").unwrap();
        let ident = ["-c", "user.name=a", "-c", "user.email=a@a", "-c", "commit.gpgsign=false"];
        git(&["add", "-A"]).unwrap();
        assert!(git(&[&ident[..], &["commit", "-qm", "uno"]].concat()).unwrap().status.success());

        let marks = base.display().to_string().replace('\\', "/");
        let filter = format!("echo m >> '{marks}/filtro'; cat");
        let monitor = format!("echo m >> '{marks}/monitor'; false");
        git(&["config", "filter.x.clean", &filter]).unwrap();
        git(&["config", "core.fsmonitor", &monitor]).unwrap();
        let hook = format!("#!/bin/sh\necho m >> '{marks}/hook'\n");
        std::fs::write(repo.join(".git").join("hooks").join("post-index-change"), hook).unwrap();
        std::fs::write(repo.join("a.txt"), "1\n").unwrap(); // mismo contenido: status tiene que releerlo
        std::fs::write(repo.join("nuevo.md"), "x").unwrap();

        let path = repo.display().to_string();
        let info = tokio::runtime::Runtime::new().unwrap().block_on(get_git_info(path));
        for mark in ["filtro", "monitor", "hook"] {
            assert!(!base.join(mark).exists(), "se ejecutó: {mark}");
        }
        assert_eq!(info["changes"], json!([{"status": "??", "path": "nuevo.md"}]));
        assert_eq!(info["log"].as_array().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn repository_programs_fall_back_to_the_users() {
        let listing = "global\0filter.lfs.clean\ngit-lfs clean -- %f\0local\0filter.lfs.clean\nmalo\0\
                       local\0filter.x.required\ntrue\0local\0diff.y.textconv\nmalo\0local\0diff.external\nmalo\0\
                       worktree\0gpg.ssh.program\nmalo\0local\0diff.algorithm\nhistogram\0";
        let expected = [
            ("filter.lfs.clean", "git-lfs clean -- %f"),
            ("filter.x.required", "false"),
            ("diff.y.textconv", "cat"),
            ("diff.external", ""),
            ("gpg.ssh.program", "ssh-keygen"),
        ];
        assert_eq!(repo_programs(listing), expected.map(|(k, v)| (k.to_string(), v.to_string())));
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
