//! Espacio de trabajo para la interfaz: árbol de archivos, lectura y guardado desde el editor e
//! historial del chat (`.fenix_chat.json`, oculto en Windows).

use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::fs;

#[derive(Serialize, Deserialize, Debug)]
pub struct FileNode {
    pub name: String,
    pub path: String,
    pub parent_path: Option<String>,
    pub is_dir: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChatMessage {
    pub sender: String,
    pub text: String,
    pub timestamp: String,
}

const IGNORED: [&str; 5] = [".git", "node_modules", "__pycache__", ".venv", ".agente"];
const BINARY: [&str; 15] = [
    "pyc", "exe", "dll", "bin", "iso", "png", "jpg", "jpeg", "gif", "ico", "wasm", "pdf", "zip", "tar", "gz",
];

/// Marca el archivo como oculto en Windows (el historial no debe ensuciar la carpeta del usuario).
fn hide_file(path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        wide.push(0);
        extern "system" {
            fn GetFileAttributesW(lpFileName: *const u16) -> u32;
            fn SetFileAttributesW(lpFileName: *const u16, dwFileAttributes: u32) -> i32;
        }
        const INVALID_FILE_ATTRIBUTES: u32 = 0xFFFF_FFFF;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        // SAFETY: `wide` es una ruta UTF-16 terminada en cero que vive durante ambas llamadas.
        unsafe {
            let attr = GetFileAttributesW(wide.as_ptr());
            if attr != INVALID_FILE_ATTRIBUTES && attr & FILE_ATTRIBUTE_HIDDEN == 0 {
                SetFileAttributesW(wide.as_ptr(), attr | FILE_ATTRIBUTE_HIDDEN);
            }
        }
    }
    #[cfg(not(windows))]
    let _ = path;
}

fn is_system_path(path: &Path) -> bool {
    let text = path.to_string_lossy().to_lowercase();
    ["\\windows\\", "/windows/", "\\system32", "/etc/"].iter().any(|p| text.contains(p))
}

/// Recorre la carpeta respetando .gitignore. Las carpetas ilegibles (p. ej. marcadores de OneDrive
/// sin descargar) se omiten en vez de dejar el árbol vacío.
pub fn workspace_tree(path: &str) -> Vec<FileNode> {
    WalkBuilder::new(path)
        .hidden(true)
        .git_ignore(true)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !IGNORED.contains(&name.as_ref()) && !name.ends_with(".pyc")
        })
        .build()
        .filter_map(Result::ok)
        .map(|entry| FileNode {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: entry.path().to_string_lossy().into_owned(),
            parent_path: entry.path().parent().map(|p| p.to_string_lossy().into_owned()),
            is_dir: entry.file_type().is_some_and(|t| t.is_dir()),
        })
        .collect()
}

#[tauri::command]
pub fn get_current_directory() -> String {
    // El espacio por defecto vive en <proyecto>/workspace, fuera de src-tauri, para que el
    // vigilante de Tauri no reinicie la app cuando el agente crea archivos.
    let current = std::env::current_dir().unwrap_or_else(|_| "C:\\".into());
    let root = if current.ends_with("src-tauri") { current.parent().unwrap_or(&current).to_path_buf() } else { current };
    let workspace = root.join("workspace");
    let _ = std::fs::create_dir_all(&workspace);
    workspace.to_string_lossy().into_owned()
}

#[tauri::command]
pub async fn get_workspace_tree(path: String) -> Result<String, String> {
    let nodes = tokio::task::spawn_blocking(move || workspace_tree(&path))
        .await
        .map_err(|e| format!("Error leyendo el espacio de trabajo: {e}"))?;
    serde_json::to_string(&nodes).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn load_chat_history(workspace_path: String) -> Result<String, String> {
    let file = Path::new(&workspace_path).join(".fenix_chat.json");
    Ok(fs::read_to_string(file).await.unwrap_or_else(|_| "[]".into()))
}

#[tauri::command]
pub async fn save_chat_message(workspace_path: String, message: ChatMessage) -> Result<(), String> {
    let file = Path::new(&workspace_path).join(".fenix_chat.json");
    let mut messages: Vec<ChatMessage> = fs::read_to_string(&file)
        .await
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    messages.push(message);
    let json = serde_json::to_string_pretty(&messages).map_err(|e| e.to_string())?;
    fs::write(&file, json).await.map_err(|e| format!("Error guardando chat: {e}"))?;
    hide_file(&file);
    Ok(())
}

#[tauri::command]
pub async fn clear_chat_history(workspace_path: String) -> Result<(), String> {
    let file = Path::new(&workspace_path).join(".fenix_chat.json");
    fs::write(&file, "[]").await.map_err(|e| e.to_string())?;
    hide_file(&file);
    Ok(())
}

#[tauri::command]
pub async fn read_file_content(path: String) -> Result<String, String> {
    let file = Path::new(&path);
    let canonical = file.canonicalize().map_err(|_| "El archivo no existe.".to_string())?;
    if is_system_path(&canonical) {
        return Err("[SECURITY_VIOLATION] Acceso a ruta del sistema prohibido.".into());
    }
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if BINARY.contains(&ext.as_str()) {
        return Err("Archivo binario no editable en editor de texto.".into());
    }
    let bytes = fs::read(&canonical).await.map_err(|e| format!("Error al leer archivo: {e}"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[tauri::command]
pub async fn save_file_content(path: String, content: String) -> Result<(), String> {
    let file = Path::new(&path);
    if let Some(parent) = file.parent().and_then(|p| p.canonicalize().ok()) {
        if is_system_path(&parent) {
            return Err("[SECURITY_VIOLATION] Escritura en ruta del sistema prohibida.".into());
        }
    }
    fs::write(file, content).await.map_err(|e| format!("Error al guardar archivo: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_skips_tool_folders_and_lists_files() {
        let dir = std::env::temp_dir().join(format!("aura_ws_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join(".agente")).unwrap();
        std::fs::write(dir.join("src").join("app.js"), "1").unwrap();
        std::fs::write(dir.join(".agente").join("plan.md"), "x").unwrap();
        let names: Vec<String> = workspace_tree(&dir.to_string_lossy()).into_iter().map(|n| n.name).collect();
        assert!(names.contains(&"app.js".to_string()));
        assert!(!names.iter().any(|n| n == "plan.md"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn system_paths_are_refused() {
        assert!(is_system_path(Path::new("C:\\Windows\\System32\\drivers")));
        assert!(!is_system_path(Path::new("C:\\Users\\yemsy\\Proyectos\\web")));
    }
}
