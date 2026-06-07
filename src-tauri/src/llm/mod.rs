use serde::{Deserialize, Serialize};
use crate::memory::{self, Cambio};
use tauri::AppHandle;

pub mod agent;
pub mod translator;
pub mod router;

#[derive(Serialize)]
struct OllamaRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
    format: &'a str,
}

#[derive(Deserialize)]
struct OllamaResponse {
    response: String,
}

#[derive(Deserialize, Serialize)]
struct OrchestratorOutput {
    #[serde(default)]
    intencion: String,
    #[serde(default)]
    respuesta_conversacional: String,
    #[serde(default)]
    pensamiento: String,
    #[serde(default)]
    archivos_a_analizar: Vec<String>,
    #[serde(default)]
    modelo_sugerido: String,
    #[serde(default)]
    comando_a_ejecutar: Option<String>,
    #[serde(default)]
    url_a_investigar: Option<String>,
    #[serde(default)]
    comando_background: Option<String>,
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    gestionar_background: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct ProgrammerOutput {
    explicacion_tecnica: String,
    cambios: Vec<Cambio>,
}



async fn call_ollama(model: &str, prompt: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .map_err(|e| format!("Error construyendo cliente HTTP: {}", e))?;
    let url = "http://localhost:11434/api/generate";
    
    // Forzamos JSON para el Orquestador (necesita respuesta estructurada)
    let payload = OllamaRequest {
        model,
        prompt,
        stream: false,
        format: "json",
    };

    let res = client.post(url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Error conectando a Ollama: {}", e))?;

    if res.status().is_success() {
        let ollama_res: OllamaResponse = res.json().await
            .map_err(|e| format!("Error parseando la respuesta JSON: {}", e))?;
        Ok(ollama_res.response)
    } else {
        Err(format!("Error de Ollama. Status: {}", res.status()))
    }
}

/// Llamada a Ollama SIN forzar JSON. Usada para reportes de texto libre (Auditoría, Análisis).
pub async fn call_ollama_text(model: &str, prompt: &str) -> Result<String, String> {
    #[derive(serde::Serialize)]
    struct TextRequest<'a> {
        model: &'a str,
        prompt: &'a str,
        stream: bool,
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .map_err(|e| format!("Error construyendo cliente HTTP (texto): {}", e))?;
    let url = "http://localhost:11434/api/generate";

    let payload = TextRequest { model, prompt, stream: false };

    let res = client.post(url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Error conectando a Ollama (texto): {}", e))?;

    if res.status().is_success() {
        let ollama_res: OllamaResponse = res.json().await
            .map_err(|e| format!("Error parseando respuesta de texto: {}", e))?;
        Ok(ollama_res.response)
    } else {
        Err(format!("Error de Ollama texto. Status: {}", res.status()))
    }
}

#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    prompt: &'a str,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    embedding: Vec<f32>,
}

pub async fn get_embedding(text: &str) -> Result<Vec<f32>, String> {
    // Timeout corto (8s) para no bloquear el pipeline si nomic no está disponible
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| format!("Error construyendo cliente de embeddings: {}", e))?;
    let url = "http://localhost:11434/api/embeddings";
    
    let payload = EmbeddingRequest {
        model: "nomic-embed-text",
        prompt: text,
    };

    let res = client.post(url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Timeout/Error en embeddings: {}", e))?;

    if res.status().is_success() {
        let ollama_res: EmbeddingResponse = res.json().await
            .map_err(|e| format!("Error parseando respuesta de embeddings: {}", e))?;
        Ok(ollama_res.embedding)
    } else {
        Err(format!("Error de Ollama Embeddings. Status: {}", res.status()))
    }
}

async fn delegate_to_programmer(task: &str, file_contents: &str, model: &str) -> Result<String, String> {
    let system_prompt = format!(
        "Eres Aura-Sentinel, el Ingeniero Ejecutor. Tu tarea es ESCRIBIR o MODIFICAR el código real basado en la instrucción del usuario y el contexto web/local proporcionado.\n\
        Instrucción: {}\n\n\
        Contexto proporcionado:\n{}\n\n\
        REGLA DE ORO 1: NO uses placeholders (ej. 'Aquí va el código', 'Tu código aquí'). Escribe el código real y funcional COMPLETO.\n\
        REGLA DE RUTAS: USA SOLO RUTAS RELATIVAS (ej. 'src/main.rs', 'test/Test.js'). NUNCA uses rutas absolutas (como C:/Users/...). Si el usuario te pide crear un archivo nuevo en una carpeta específica, usa la ruta relativa. Si la ruta no existe, el sistema la creará automáticamente.\n\
        REGLA DE CREACIÓN: Si estás creando un archivo completamente nuevo, el campo 'buscar' DEBE estar vacío (\" \") y el campo 'reemplazar' debe contener todo el código fuente desde cero.\n\
        REGLA DE ORO 3: Escribe el código COMPLETO sin truncar. No cortes el código a la mitad. Asegúrate de cerrar todas las llaves, paréntesis y funciones.\n\
        REGLA DE ORO 4: Para scripts de Solana en Python, usa SOLO la librería estándar `requests` con `requests.post` (JSON-RPC siempre usa POST), haciendo peticiones HTTP directamente al RPC de Solana en https://api.mainnet-beta.solana.com\n\
        DEBES responder ÚNICAMENTE con un JSON válido con la siguiente estructura, sin texto fuera del JSON:\n\
        {{\n\
          \"explicacion_tecnica\": \"Breve resumen de lo creado\",\n\
          \"cambios\": [\n\
            {{\n\
              \"archivo\": \"nombre_del_archivo.py\",\n\
              \"buscar\": \"\",\n\
              \"reemplazar\": \"código funcional COMPLETO\"\n\
            }}\n\
          ]\n\
        }}",
        task, file_contents
    );

    call_ollama(model, &system_prompt).await
}

async fn delegate_to_auditor(file_contents: &str, model: &str) -> String {
    let audit_prompt = format!(
        "Eres un Arquitecto de Software Senior auditando el código de este proyecto.\n\
        Tu misión es una revisión crítica: encuentra errores lógicos, vulnerabilidades de seguridad,\n\
        problemas de rendimiento y áreas de mejora.\n\n\
        CÓDIGO A AUDITAR:\n{}\n\n\
        REPORTE DE AUDITORÍA:\n\
        Estructura tu respuesta en estas secciones:\n\
        ## 1. Resumen Ejecutivo\n\
        ## 2. Errores Críticos (si existen)\n\
        ## 3. Vulnerabilidades de Seguridad\n\
        ## 4. Problemas de Rendimiento\n\
        ## 5. Recomendaciones Prioritarias\n\n\
        Responde en texto plano estructurado, NO uses JSON.",
        file_contents
    );
    call_ollama_text(model, &audit_prompt).await
        .unwrap_or_else(|e| format!("Error en auditoría: {}", e))
}

#[tauri::command]
pub async fn process_user_prompt(user_message: String, workspace_path: String, app_handle: tauri::AppHandle) -> Result<String, String> {
    let mut enriched_message = String::new();

    // ── Zero-latency meta-command intercept ──────────────────────────────────
    if let Some(action) = crate::core::intent_router::try_handle_meta_command(&user_message, &workspace_path) {
        match action {
            crate::core::intent_router::IntentAction::Finish(msg) => {
                agent::emit_event(&app_handle, 0, "[META-CMD] Consulta de estado detectada. Respondiendo desde la memoria local...", "PLANNING");
                agent::emit_event(&app_handle, 1, "Respuesta generada sin IA.", "SUCCESS");
                let response = serde_json::json!({
                    "status": "FINISH",
                    "respuesta_conversacional": msg
                });
                return Ok(response.to_string());
            },
            crate::core::intent_router::IntentAction::Resume { objetivo, resume_msg } => {
                agent::emit_event(&app_handle, 0, &resume_msg, "INFO");
                // Bypass translator: directly use the saved objective
                enriched_message = objetivo;
            }
        }
    }

    if enriched_message.is_empty() {
        let technical_intent = translator::translate_to_technical_intent(&user_message, &app_handle).await;
        println!("[TRADUCTOR] Input: '{}' -> Intención: '{}'", user_message, technical_intent);
        enriched_message = format!("Petición Original del Usuario: {}\n\nGuía de Traducción Técnica: {}", user_message, technical_intent);
    }

    let workspace_tree_nodes = crate::memory::get_workspace_tree_internal(workspace_path.clone()).await?;
    // Filter out noise directories — node_modules alone can be 4000+ nodes and pollutes
    // the LLM context and embedding index with irrelevant framework internals.
    let ignored_dirs = ["node_modules", ".git", "__pycache__", "target"];
    let files_only: Vec<_> = workspace_tree_nodes.iter().filter(|n| {
        !n.is_dir && !ignored_dirs.iter().any(|d| n.path.contains(d))
    }).collect();
    let mut index = crate::memory::read_vector_index(&workspace_path).await;
    const MAX_NUEVOS_A_VECTORIZAR: usize = 50;
    if index.len() != files_only.len() {
        let nuevos: Vec<_> = files_only.iter().filter(|f| !index.iter().any(|n| n.path == f.path)).collect();
        if nuevos.len() <= MAX_NUEVOS_A_VECTORIZAR {
            let mut new_index = Vec::new();
            for file in &files_only {
                if let Some(existing) = index.iter().find(|n| n.path == file.path) {
                    new_index.push(existing.clone());
                } else {
                    if let Ok(emb) = get_embedding(&file.path).await {
                        new_index.push(crate::memory::VectorNode { path: file.path.clone(), embedding: emb });
                    }
                }
            }
            index = new_index;
            let _ = crate::memory::write_vector_index(&workspace_path, &index).await;
        } else {
            index.clear();
        }
    }
    let user_embedding = get_embedding(&enriched_message).await.unwrap_or_default();
    let tree_json = if user_embedding.is_empty() || index.is_empty() {
        serde_json::to_string(&files_only.iter().map(|n| n.path.clone()).collect::<Vec<String>>()).unwrap_or_default()
    } else {
        let mut scored_nodes: Vec<(&crate::memory::VectorNode, f32)> = index.iter().map(|node| {
            let score = crate::core::cosine_similarity(&user_embedding, &node.embedding);
            (node, score)
        }).collect();
        scored_nodes.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let top_10: Vec<String> = scored_nodes.into_iter().take(10).map(|(n, _)| n.path.clone()).collect();
        serde_json::to_string(&top_10).unwrap_or_default()
    };
    agent::run_agent_loop(enriched_message, workspace_path, tree_json, app_handle).await
}
