use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::fs;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProjectMemory {
    pub workspace_path: String,
    pub timestamp: String,
    pub chunks: Vec<MemoryChunk>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MemoryChunk {
    pub file_path: String,
    pub content: String,
    pub embedding: Vec<f32>,
}

/// Helper function to create the data/memory directory
async fn get_memory_file_path() -> String {
    let current = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());

    let project_root = if current.ends_with("src-tauri") || current.ends_with("src-tauri\\") {
        Path::new(&current)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(current)
    } else {
        current
    };

    let mem_dir = Path::new(&project_root).join("data").join("memory");
    let _ = fs::create_dir_all(&mem_dir).await;
    mem_dir.join("vectors.json").to_string_lossy().to_string()
}

/// Lee la base de datos de memoria global
pub async fn read_global_memory() -> Vec<ProjectMemory> {
    let path = get_memory_file_path().await;
    if let Ok(content) = fs::read_to_string(&path).await {
        if let Ok(data) = serde_json::from_str(&content) {
            return data;
        }
    }
    vec![]
}

/// Guarda la base de datos de memoria global
pub async fn save_global_memory(memory: &Vec<ProjectMemory>) -> Result<(), String> {
    let path = get_memory_file_path().await;
    let json = serde_json::to_string(memory).map_err(|e| format!("Error serializando memoria global: {}", e))?;
    fs::write(&path, json).await.map_err(|e| format!("Error escribiendo memoria global: {}", e))?;
    Ok(())
}

/// Vectoriza e indexa un proyecto de forma silenciosa. Solo debe llamarse en proyectos probados.
pub async fn index_project(workspace_path: &str) -> Result<String, String> {
    // 1. Obtener archivos del proyecto
    let tree = crate::memory::get_workspace_tree_internal(workspace_path.to_string()).await?;
    let files: Vec<_> = tree.into_iter().filter(|n| !n.is_dir).collect();
    
    let mut chunks = Vec::new();
    
    // 2. Leer contenido y vectorizar
    for file in files {
        let full_path = Path::new(workspace_path).join(&file.path);
        if let Ok(content) = fs::read_to_string(&full_path).await {
            // No indexamos binarios ni node_modules
            if content.contains('\0') || file.path.contains("node_modules") || file.path.contains(".git") {
                continue;
            }
            
            // Limitamos tamaño de archivo a vectorizar para no saturar
            let chunk_content = if content.len() > 10000 {
                content[0..10000].to_string()
            } else {
                content.clone()
            };
            
            if let Ok(embedding) = crate::llm::get_embedding(&chunk_content).await {
                chunks.push(MemoryChunk {
                    file_path: file.path,
                    content: chunk_content,
                    embedding,
                });
            }
        }
    }
    
    // 3. Crear registro
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs().to_string();
    
    let project_mem = ProjectMemory {
        workspace_path: workspace_path.to_string(),
        timestamp,
        chunks: chunks.clone(),
    };
    
    // 4. Añadir a la base global
    let mut global_mem = read_global_memory().await;
    // Evitar duplicados del mismo path, reemplazando
    global_mem.retain(|m| m.workspace_path != workspace_path);
    global_mem.push(project_mem);
    
    save_global_memory(&global_mem).await?;
    
    // SPRINT 2: Silently consolidate knowledge after indexing
    let _ = consolidate_knowledge(workspace_path, &chunks).await;
    
    Ok(format!("Proyecto '{}' indexado exitosamente en la memoria permanente.", workspace_path))
}

/// Consulta la memoria global buscando fragmentos relevantes por Similitud de Coseno.
pub async fn query_memory(query: &str) -> Result<String, String> {
    let global_mem = read_global_memory().await;
    if global_mem.is_empty() {
        return Ok("La memoria a largo plazo está vacía. No hay contexto histórico.".to_string());
    }
    
    let query_embedding = crate::llm::get_embedding(query).await?;
    if query_embedding.is_empty() {
        return Err("No se pudo obtener el embedding de la consulta.".to_string());
    }
    
    let mut scored_chunks: Vec<(&MemoryChunk, &str, f32)> = Vec::new();
    
    for project in &global_mem {
        for chunk in &project.chunks {
            let score = crate::core::cosine_similarity(&query_embedding, &chunk.embedding);
            scored_chunks.push((chunk, &project.workspace_path, score));
        }
    }
    
    // Ordenar de mayor a menor similitud
    scored_chunks.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    
    // Tomar los top 5 más relevantes
    let mut context_result = String::from("[CONTEXTO HISTÓRICO RECUPERADO]\n\n");
    
    // SPRINT 2: Load and prepend synthesized knowledge (Lessons Learned)
    let knowledge = load_knowledge_index().await;
    let mut lessons_added = 0;
    for entry in &knowledge {
        // Simple heuristic: if query contains keywords from lessons or just dump top generic lessons
        // For this sprint, we just add the first 5 lessons across projects to guide the LLM
        for lesson in &entry.lessons {
            if lessons_added == 0 {
                context_result.push_str("💡 [LECCIONES APRENDIDAS DE PROYECTOS SIMILARES]:\n");
            }
            context_result.push_str(&format!("- {}\n", lesson));
            lessons_added += 1;
            if lessons_added >= 5 { break; }
        }
        if lessons_added >= 5 { break; }
    }
    if lessons_added > 0 {
        context_result.push_str("\n");
    }

    let mut added = 0;
    
    for (chunk, workspace, score) in scored_chunks {
        if score > 0.6 { // Umbral de relevancia aceptable
            context_result.push_str(&format!(
                "Proyecto: {}\nArchivo: {}\nSimilitud: {:.2}\nContenido Parcial:\n{}\n\n---\n",
                workspace, chunk.file_path, score, chunk.content
            ));
            added += 1;
            if added >= 5 {
                break;
            }
        }
    }
    
    if added == 0 {
        Ok("No se encontró contexto histórico relevante (Similitud baja).".to_string())
    } else {
        Ok(context_result)
    }
}


// ─── Sprint 2: Memory Consolidation ──────────────────────────────────────────
// In addition to raw vector chunks, we synthesize "lessons learned".

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct KnowledgeEntry {
    pub workspace_path: String,
    pub lessons: Vec<String>,
    pub timestamp: String,
}

async fn get_knowledge_file_path() -> String {
    let current = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());

    let project_root = if current.ends_with("src-tauri") || current.ends_with("src-tauri\\") {
        Path::new(&current)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(current)
    } else {
        current
    };

    let mem_dir = Path::new(&project_root).join("data").join("memory");
    let _ = fs::create_dir_all(&mem_dir).await;
    mem_dir.join("knowledge_index.json").to_string_lossy().to_string()
}

pub async fn load_knowledge_index() -> Vec<KnowledgeEntry> {
    let path = get_knowledge_file_path().await;
    if let Ok(content) = fs::read_to_string(&path).await {
        if let Ok(data) = serde_json::from_str(&content) {
            return data;
        }
    }
    vec![]
}

pub async fn save_knowledge_index(knowledge: &Vec<KnowledgeEntry>) -> Result<(), String> {
    let path = get_knowledge_file_path().await;
    let json = serde_json::to_string_pretty(knowledge).map_err(|e| format!("Error serializando knowledge: {}", e))?;
    fs::write(&path, json).await.map_err(|e| format!("Error escribiendo knowledge: {}", e))?;
    Ok(())
}

/// Analiza los chunks crudos de un proyecto y sintetiza reglas generales (lecciones).
pub async fn consolidate_knowledge(workspace_path: &str, chunks: &[MemoryChunk]) -> Result<(), String> {
    if chunks.is_empty() { return Ok(()); }
    
    // Tomar solo una muestra para no saturar al LLM
    let sample_size = chunks.len().min(5);
    let mut sample_content = String::new();
    for i in 0..sample_size {
        sample_content.push_str(&format!("Archivo: {}\n{}\n\n", chunks[i].file_path, &chunks[i].content[..chunks[i].content.len().min(1000)]));
    }
    
    let prompt = format!(
        "Basandote en el codigo de este proyecto, extrae 3 lecciones clave o patrones arquitectonicos utiles para el futuro. Responde SOLO con una lista de bullet points.\n\nCODIGO:\n{}",
        sample_content
    );
    
    // Usamos el ORCHESTRATOR_MODEL (asumiendo phi3/llama3) para sintetizar
    if let Ok(synthesis) = crate::llm::call_ollama(crate::llm::agent::DEFAULT_ORCHESTRATOR_MODEL, &prompt).await {
        let lessons: Vec<String> = synthesis.lines()
            .map(|l| l.trim().trim_start_matches("-").trim_start_matches("*").trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
            
        use std::time::{SystemTime, UNIX_EPOCH};
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs().to_string();
        
        let mut index = load_knowledge_index().await;
        index.retain(|e| e.workspace_path != workspace_path);
        index.push(KnowledgeEntry {
            workspace_path: workspace_path.to_string(),
            lessons,
            timestamp,
        });
        
        let _ = save_knowledge_index(&index).await;
    }
    
    Ok(())
}
