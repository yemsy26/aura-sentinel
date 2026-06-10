pub mod types;

use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::fs;
use types::FenixMemoryLog;

#[derive(Serialize, Deserialize, Debug)]
pub struct FileNode {
    pub name: String,
    pub path: String,
    pub parent_path: Option<String>,
    pub is_dir: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Cambio {
    pub archivo: String,
    pub buscar: String,
    pub reemplazar: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VectorNode {
    pub path: String,
    pub embedding: Vec<f32>,
}

pub async fn read_vector_index(workspace_path: &str) -> Vec<VectorNode> {
    let index_file = Path::new(workspace_path).join(".fenix_index.json");
    if let Ok(content) = fs::read_to_string(&index_file).await {
        if let Ok(index) = serde_json::from_str(&content) {
            return index;
        }
    }
    vec![]
}

pub async fn write_vector_index(workspace_path: &str, index: &Vec<VectorNode>) -> Result<(), String> {
    let index_file = Path::new(workspace_path).join(".fenix_index.json");
    let json = serde_json::to_string(index).map_err(|e| format!("Error serializando index: {}", e))?;
    fs::write(&index_file, json).await.map_err(|e| format!("Error guardando index: {}", e))?;
    crate::core::hide_file_windows(&index_file).await;
    Ok(())
}

pub async fn get_workspace_tree_internal(path: String) -> Result<Vec<FileNode>, String> {
    tokio::task::spawn_blocking(move || {
        let mut nodes = Vec::new();
        let walker = WalkBuilder::new(&path)
            .hidden(true)
            .git_ignore(true)
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                name != ".git" && name != "node_modules"
            })
            .build();

        for result in walker {
            match result {
                Ok(entry) => {
                    let path_str = entry.path().to_string_lossy().to_string();
                    let name = entry.file_name().to_string_lossy().to_string();
                    let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                    
                    let parent_path = entry.path().parent().map(|p| p.to_string_lossy().to_string());

                    nodes.push(FileNode {
                        name,
                        path: path_str,
                        parent_path,
                        is_dir,
                    });
                }
                Err(err) => eprintln!("Aura-Sentinel Walker Error: {}", err),
            }
        }
        nodes
    })
    .await
    .map_err(|e| format!("Error en la tarea de lectura del workspace: {}", e))
}

pub async fn read_files_safely(workspace_path: &str, files: Vec<String>) -> String {
    let mut combined_content = String::new();
    let max_size = 50 * 1024; // 50KB límite de VRAM safety

    for file_path in files {
        let path_obj = Path::new(&file_path);
        let full_path = if path_obj.is_absolute() {
            path_obj.to_path_buf()
        } else {
            Path::new(workspace_path).join(path_obj)
        };

        if !crate::core::security::is_path_allowed(Path::new(workspace_path), &full_path) {
            combined_content.push_str(&format!(
                "--- ARCHIVO IGNORADO: {} ([SECURITY_VIOLATION] Acceso no permitido fuera del entorno de trabajo) ---\n\n",
                file_path
            ));
            continue;
        }

        match fs::metadata(&full_path).await {
            Ok(meta) => {
                if meta.len() > max_size {
                    combined_content.push_str(&format!(
                        "--- ARCHIVO IGNORADO: {} (Supera los 50KB) ---\n\n",
                        file_path
                    ));
                } else {
                    match fs::read_to_string(&full_path).await {
                        Ok(content) => {
                            combined_content.push_str(&format!(
                                "--- ARCHIVO: {} ---\n{}\n\n",
                                file_path, content
                            ));
                        }
                        Err(e) => {
                            combined_content.push_str(&format!(
                                "--- ERROR LEYENDO ARCHIVO: {} ({}) ---\n\n",
                                file_path, e
                            ));
                        }
                    }
                }
            }
            Err(e) => {
                combined_content.push_str(&format!(
                    "--- ARCHIVO NO ENCONTRADO O INACCESIBLE: {} ({}) ---\n\n",
                    file_path, e
                ));
            }
        }
    }

    combined_content
}

pub async fn apply_code_changes(workspace_path: &str, cambios: Vec<Cambio>) -> Result<String, String> {
    let mut exitosos = 0;
    let mut exitosos_nombres = Vec::new();
    let mut fuzzy_logs = Vec::new();
    
    for cambio in cambios {
        let path_obj = Path::new(&cambio.archivo);
        let full_path = if path_obj.is_absolute() {
            path_obj.to_path_buf()
        } else {
            Path::new(workspace_path).join(path_obj)
        };

        if !crate::core::security::is_path_allowed(Path::new(workspace_path), &full_path) {
            return Err(format!("[SECURITY_VIOLATION] Acceso no permitido fuera del entorno de trabajo para el archivo: {}", cambio.archivo));
        }
        
        let mut contenido_original = String::new();
        let file_exists = full_path.exists();
        
        if file_exists {
            match fs::read_to_string(&full_path).await {
                Ok(c) => contenido_original = c,
                Err(e) => {
                    eprintln!("Aura-Sentinel: Error leyendo {}: {}", cambio.archivo, e);
                    continue;
                }
            }
        } else {
            // Si el archivo no existe, lo creamos. Nos aseguramos de que el directorio padre exista.
            if let Some(parent) = full_path.parent() {
                if let Err(e) = fs::create_dir_all(parent).await {
                    eprintln!("Aura-Sentinel: Error creando directorio {:?}: {}", parent, e);
                    continue;
                }
            }
        }
        
        let mut nuevo_contenido = contenido_original.clone();
        let mut match_encontrado_seguro = false;
        
        let buscar_str = cambio.buscar.as_str();
        
        if !file_exists || contenido_original.trim().is_empty() || buscar_str.trim().is_empty() {
            // Archivo nuevo, original vacío, o búsqueda vacía: no usamos .replace ni Fuzzy Match
            nuevo_contenido = cambio.reemplazar.clone();
            match_encontrado_seguro = true;
        } else {
            // Búsqueda exacta estándar
            let count = contenido_original.matches(buscar_str).count();
            if count == 1 {
                nuevo_contenido = contenido_original.replace(buscar_str, &cambio.reemplazar);
                match_encontrado_seguro = true;
            } else if count > 1 {
                eprintln!("Aura-Sentinel Aviso: Hay {} coincidencias exactas para un bloque. Se delega a Fuzzy Match para mayor seguridad.", count);
                // match_encontrado_seguro = false, así entra en Fuzzy Match
            }
        }
        
        if file_exists && !match_encontrado_seguro {
            let buscar_lines: Vec<&str> = cambio.buscar.lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .collect();
                
            let orig_lines: Vec<(usize, &str)> = contenido_original.lines()
                .enumerate()
                .map(|(idx, l)| (idx, l.trim()))
                .filter(|(_, l)| !l.is_empty())
                .collect();

            let mut matched = false;
            
            if !buscar_lines.is_empty() {
                let window_size = buscar_lines.len();
                let mut match_indices = Vec::new();

                if orig_lines.len() >= window_size {
                    for i in 0..=(orig_lines.len() - window_size) {
                        let mut is_match = true;
                        for j in 0..window_size {
                            if orig_lines[i + j].1 != buscar_lines[j] {
                                is_match = false;
                                break;
                            }
                        }
                        if is_match {
                            match_indices.push(i);
                        }
                    }
                    
                    // SAFETY: Solo aplicamos Fuzzy Match si hay exactamente 1 coincidencia
                    // o si el tamaño de la ventana es suficientemente grande (>= 3 líneas)
                    // para evitar destruir el archivo reemplazando la primera llave "}" que encuentre.
                    if match_indices.len() == 1 || (match_indices.len() > 1 && window_size >= 3) {
                        let best_match_idx = match_indices[0];
                        let start_idx = orig_lines[best_match_idx].0;
                        let end_idx = orig_lines[best_match_idx + window_size - 1].0;
                        
                        let raw_orig_lines: Vec<&str> = contenido_original.lines().collect();
                        
                        let mut pre_block = raw_orig_lines[0..start_idx].join("\n");
                        if !pre_block.is_empty() { pre_block.push('\n'); }
                        
                        let mut post_block = raw_orig_lines[end_idx + 1..].join("\n");
                        if !post_block.is_empty() { post_block.insert(0, '\n'); }
                        
                        nuevo_contenido = format!("{}{}{}", pre_block, cambio.reemplazar, post_block);
                        matched = true;
                        
                        let fuzzy_msg = format!("[FUZZY MATCH] Parche semántico aplicado en {}. (Ventana: {}, Coincidencias: {})", cambio.archivo, window_size, match_indices.len());
                        println!("{}", fuzzy_msg);
                        fuzzy_logs.push(fuzzy_msg);
                    } else if match_indices.len() > 1 {
                        let err_msg = format!("[FUZZY MATCH RECHAZADO] Hay múltiples ({}) coincidencias ambiguas para un bloque muy pequeño ({} líneas) en {}. Reemplazo cancelado por seguridad.", match_indices.len(), window_size, cambio.archivo);
                        eprintln!("{}", err_msg);
                        fuzzy_logs.push(err_msg);
                    }
                }
            }
            
            if !matched {
                eprintln!("Aura-Sentinel Aviso: No se encontró el texto exacto ni difuso aplicable en {}", cambio.archivo);
                continue;
            }
        }
        
        match fs::write(&full_path, nuevo_contenido).await {
            Ok(_) => {
                exitosos += 1;
                exitosos_nombres.push(cambio.archivo.clone());
                
                use std::time::{SystemTime, UNIX_EPOCH};
                let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs().to_string();
                
                let entry = FenixMemoryLog {
                    task_id: format!("TASK-{}", timestamp),
                    timestamp,
                    file_path: cambio.archivo.clone(),
                    summary: "Cambio aplicado por Aura-Sentinel".to_string(),
                    previous_hash: "hash_placeholder".to_string(),
                    compilation_status: "COMPILACIÓN_PENDIENTE".to_string(),
                };
                
                let _ = add_memory_entry(workspace_path.to_string(), entry).await;
            }
            Err(e) => {
                eprintln!("Aura-Sentinel: Error escribiendo {}: {}", cambio.archivo, e);
            }
        }
    }
    
    let base_msg = if exitosos > 0 {
        format!("{} archivos modificados exitosamente: {}", exitosos, exitosos_nombres.join(", "))
    } else {
        "0 archivos modificados exitosamente.".to_string()
    };
    if fuzzy_logs.is_empty() {
        Ok(base_msg)
    } else {
        Ok(format!("{}\n{}", base_msg, fuzzy_logs.join("\n")))
    }
}

pub async fn update_last_memory_status(workspace_path: &str, status: &str) -> Result<(), String> {
    let memory_file = Path::new(workspace_path).join(".fenix_memory.json");
    
    let current_data = fs::read_to_string(&memory_file)
        .await
        .unwrap_or_else(|_| "[]".to_string());
    
    let mut logs: Vec<FenixMemoryLog> = serde_json::from_str(&current_data)
        .unwrap_or_else(|_| vec![]);
        
    if let Some(last_log) = logs.last_mut() {
        last_log.compilation_status = status.to_string();
        
        let new_json = serde_json::to_string_pretty(&logs)
            .map_err(|e| format!("Error al serializar: {}", e))?;
            
            fs::write(&memory_file, new_json)
                .await
                .map_err(|e| format!("Error al guardar la entrada en el archivo: {}", e))?;
            crate::core::hide_file_windows(&memory_file).await;
        }
            
        Ok(())
    }

#[tauri::command]
pub fn get_current_directory() -> String {
    // Apuntamos al directorio workspace/ FUERA de src-tauri para evitar que el
    // file watcher de Tauri reinicie la app cuando el AI crea nuevos archivos.
    let current = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "C:\\".to_string());

    // Si estamos dentro de src-tauri (modo dev de Tauri), subimos un nivel al proyecto raíz
    let project_root = if current.ends_with("src-tauri") || current.ends_with("src-tauri\\") {
        std::path::Path::new(&current)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(current)
    } else {
        current
    };

    // El workspace del AI vive en <raiz_proyecto>/workspace/ — invisible para Tauri
    let workspace_dir = std::path::Path::new(&project_root).join("workspace");
    let _ = std::fs::create_dir_all(&workspace_dir);
    workspace_dir.to_string_lossy().to_string()
}

#[tauri::command]
pub async fn get_workspace_tree(path: String) -> Result<String, String> {
    let tree_result = get_workspace_tree_internal(path).await?;
    serde_json::to_string(&tree_result).map_err(|e| format!("Error al serializar el mapa: {}", e))
}

#[tauri::command]
pub async fn init_memory_log(workspace_path: String) -> Result<String, String> {
    // DOC-FIX #14:
    // .fenix_memory.json -> Registro semántico y RAG de acciones del agente (cambios de código, comandos).
    // .fenix_chat.json   -> Historial conversacional puro para renderizado en la UI.
    let memory_file = Path::new(&workspace_path).join(".fenix_memory.json");
    if !memory_file.exists() {
        let empty_logs: Vec<FenixMemoryLog> = vec![];
        let json = serde_json::to_string_pretty(&empty_logs)
            .map_err(|e| format!("Error serializando memoria vacía: {}", e))?;
        
        fs::write(&memory_file, json)
            .await
            .map_err(|e| format!("Error creando archivo de memoria: {}", e))?;
        crate::core::hide_file_windows(&memory_file).await;
            
        Ok("Memoria inicializada correctamente.".to_string())
    } else {
        Ok("El archivo de memoria ya existe en este workspace.".to_string())
    }
}

#[tauri::command]
pub async fn add_memory_entry(workspace_path: String, entry: FenixMemoryLog) -> Result<(), String> {
    let memory_file = Path::new(&workspace_path).join(".fenix_memory.json");
    
    let current_data = fs::read_to_string(&memory_file)
        .await
        .unwrap_or_else(|_| "[]".to_string());
    
    let mut logs: Vec<FenixMemoryLog> = serde_json::from_str(&current_data)
        .unwrap_or_else(|_| vec![]);
        
    logs.push(entry);
    
    let new_json = serde_json::to_string_pretty(&logs)
        .map_err(|e| format!("Error al serializar la nueva entrada: {}", e))?;
        
    fs::write(&memory_file, new_json)
        .await
        .map_err(|e| format!("Error al guardar la entrada en el archivo: {}", e))?;
    crate::core::hide_file_windows(&memory_file).await;
        
    Ok(())
}

#[tauri::command]
pub async fn read_memory_logs(workspace_path: String) -> Result<String, String> {
    let memory_file = Path::new(&workspace_path).join(".fenix_memory.json");
    if memory_file.exists() {
        match fs::read_to_string(&memory_file).await {
            Ok(content) => Ok(content),
            Err(_) => Ok("[]".to_string()),
        }
    } else {
        Ok("[]".to_string())
    }
}

#[tauri::command]
pub async fn load_chat_history(workspace_path: String) -> Result<String, String> {
    let chat_file = Path::new(&workspace_path).join(".fenix_chat.json");
    if chat_file.exists() {
        match fs::read_to_string(&chat_file).await {
            Ok(content) => Ok(content),
            Err(_) => Ok("[]".to_string()),
        }
    } else {
        Ok("[]".to_string())
    }
}

#[tauri::command]
pub async fn save_chat_message(workspace_path: String, message: types::ChatMessage) -> Result<(), String> {
    let chat_file = Path::new(&workspace_path).join(".fenix_chat.json");
    
    let current_data = fs::read_to_string(&chat_file)
        .await
        .unwrap_or_else(|_| "[]".to_string());
    
    let mut messages: Vec<types::ChatMessage> = serde_json::from_str(&current_data)
        .unwrap_or_else(|_| vec![]);
        
    messages.push(message);
    
    let new_json = serde_json::to_string_pretty(&messages)
        .map_err(|e| format!("Error serializando chat: {}", e))?;
        
    fs::write(&chat_file, new_json)
        .await
        .map_err(|e| format!("Error guardando chat: {}", e))?;
    crate::core::hide_file_windows(&chat_file).await;
        
    Ok(())
}

#[tauri::command]
pub async fn clear_chat_history(workspace_path: String) -> Result<(), String> {
    let chat_file = Path::new(&workspace_path).join(".fenix_chat.json");
    if chat_file.exists() {
        let _ = fs::remove_file(&chat_file).await;
    }
    // Escribir un array vacío inmediatamente
    let _ = fs::write(&chat_file, "[]").await;
    crate::core::hide_file_windows(&chat_file).await;
    Ok(())
}
