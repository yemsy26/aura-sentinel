use serde::{Deserialize, Serialize};
use crate::memory::Cambio;


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
        "Eres Aura-Sentinel, el Ingeniero Ejecutor. Tu tarea es ESCRIBIR o MODIFICAR el código real basado en la instrucción.\n\
        Instrucción: {}\n\n\
        Contexto del proyecto actual:\n{}\n\n\
        === REGLAS ABSOLUTAS — LEERLAS ANTES DE GENERAR CÓDIGO ===\n\
        \n\
        [REGLA 1 - CÓDIGO COMPLETO]: Escribe el código COMPLETO y FUNCIONAL. CERO placeholders ('# TODO', '...', 'aquí va el código'). El archivo debe ejecutarse tal como lo escribes.\n\
        \n\
        [REGLA 2 - RUTAS RELATIVAS]: ÚNICAMENTE usa rutas relativas ('src/main.py', 'main.py'). NUNCA rutas absolutas (C:/...).\n\
        \n\
        [REGLA 3 - PYTHON CRÍTICO - LEE ESTO 3 VECES]:\n\
           a) SIEMPRE añade '# -*- coding: utf-8 -*-' como PRIMERA línea de cada archivo .py.\n\
           b) Para strings en Python usa EXCLUSIVAMENTE comillas dobles: \"texto\". JAMÁS uses comillas simples dentro de strings.\n\
           c) Strings multilínea: usa triple comillas dobles: \"\"\"linea1\\nlinea2\"\"\". NUNCA saltos de línea literales dentro de un string.\n\
           d) SIEMPRE cierra TODOS los paréntesis, corchetes y llaves que abras.\n\
           e) Ejemplo CORRECTO de print: print(\"Hola mundo\")  ← correcto\n\
           f) Ejemplo INCORRECTO: print('Hola mundo')  ← PROHIBIDO\n\
           g) Ejemplo INCORRECTO: print('Juego terminado!'  ← PROHIBIDO (paréntesis sin cerrar)\n\
        \n\
        [REGLA 4 - CREACIÓN DE ARCHIVO NUEVO]: Cuando crees un archivo desde cero, el campo 'buscar' debe ser \"\" (vacío).\n\
        \n\
        [REGLA 5 - JSON LIMPIO]: Tu respuesta DEBE ser únicamente JSON válido. Sin texto antes ni después del JSON.\n\
        \n\
        === FORMATO DE RESPUESTA (JSON EXACTO) ===\n\
        {{\n\
          \"explicacion_tecnica\": \"Descripción breve de lo implementado\",\n\
          \"cambios\": [\n\
            {{\n\
              \"archivo\": \"ruta/relativa/archivo.py\",\n\
              \"buscar\": \"\",\n\
              \"reemplazar\": \"# -*- coding: utf-8 -*-\\nimport tkinter as tk\\n\\n# ... código completo ...\"\n\
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

pub(crate) async fn delegate_to_logic_solver(file_contents: &str, model: &str) -> String {
    let solver_prompt = format!(
        "Eres un Motor de Verificación Formal (Logic Solver). Tu misión es demostrar matemáticamente y de manera lógica si el código adjunto contiene fallos lógicos, bucles infinitos, condiciones inalcanzables o dependencias rotas. \
        Debes emular un solver z3.\n\n\
        CÓDIGO A ANALIZAR:\n{}\n\n\
        INSTRUCCIONES:\n\
        1. Analiza el flujo de control rigurosamente.\n\
        2. Identifica cualquier variable que pueda no inicializarse.\n\
        3. Verifica si existe alguna condición lógica que jamás se pueda cumplir (Dead Code).\n\
        4. Comprueba los límites de memoria o recursión.\n\
        5. Devuelve el reporte en formato texto detallado, enfocado en lógica estricta y matemáticas, NO en estilo visual o rendimiento.\n\n\
        REPORTE Z3-LOGIC:",
        file_contents
    );
    call_ollama_text(model, &solver_prompt).await
        .unwrap_or_else(|e| format!("Error en verificación lógica: {}", e))
}

#[tauri::command]
pub async fn process_user_prompt(user_message: String, workspace_path: String, app_handle: tauri::AppHandle) -> Result<String, String> {
    let mut enriched_message = String::new();
    let mut journal = crate::core::session_journal::load_journal(&workspace_path);

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
        let lower_msg = user_message.to_lowercase();

        // ── Context-aware follow-up for folder creation ──────────────────────────
        let waiting_for_folder_name = journal.chat_history.last().map(|msg| {
            msg.contains("¿Con qué nombre quieres que cree la carpeta? Dime el nombre exacto")
        }).unwrap_or(false);

        if waiting_for_folder_name && !user_message.trim().is_empty() {
            let folder_name: String = user_message.split_whitespace().next().unwrap_or("").chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            
            if !folder_name.is_empty() {
                agent::emit_event(&app_handle, 0, &format!("[FAST-TRACK] Creando carpeta (seguimiento): {}", folder_name), "ACTION");
                let mkdir_cmd = format!("mkdir \"{}\"", folder_name);
                match crate::core::execute_terminal_command(&workspace_path, &mkdir_cmd).await {
                    Ok(_) => {
                        agent::emit_event(&app_handle, 1, &format!("Carpeta '{}' creada exitosamente.", folder_name), "SUCCESS");
                        let resp_msg = format!("✅ Listo. Carpeta `{}` creada en tu workspace.", folder_name);
                        journal.chat_history.push(format!("Usuario: {}", user_message));
                        journal.chat_history.push(format!("Aura: {}", resp_msg));
                        if journal.chat_history.len() > 6 { journal.chat_history.drain(0..journal.chat_history.len() - 6); }
                        crate::core::session_journal::save_journal(&workspace_path, &journal);
                        let response = serde_json::json!({"status": "FINISH", "respuesta_conversacional": resp_msg});
                        return Ok(response.to_string());
                    },
                    Err(e) => {
                        let resp_msg = if e.contains("ya existe") || e.contains("already exists") || e.contains("MKDIR") {
                            format!("ℹ️ La carpeta `{}` ya existe en tu workspace.", folder_name)
                        } else {
                            format!("⚠️ No pude crear la carpeta `{}`. Error: {}", folder_name, e)
                        };
                        let response = serde_json::json!({"status": "FINISH", "respuesta_conversacional": resp_msg});
                        return Ok(response.to_string());
                    }
                }
            }
        }

        // ── Zero-latency folder creation intercept ─────────────────────────────
        // Detect "crea una carpeta X", "crear carpeta X", "make a folder X", etc.
        let mut folder_prefixes = vec![
            "crea una carpeta con nombre", "crear una carpeta con nombre",
            "crea la carpeta con nombre", "crear la carpeta con nombre",
            "crea una carpeta llamada", "crear una carpeta llamada",
            "crea la carpeta llamada", "crear la carpeta llamada",
            "crea una carpeta", "crea la carpeta", "crea carpeta", "crear carpeta",
            "crea el directorio", "crear directorio", "crea directorio",
            "make a folder", "make folder", "create folder", "create directory"
        ];
        // Sort by length descending so longer prefixes match first
        folder_prefixes.sort_by(|a, b| b.len().cmp(&a.len()));
        let folder_name_opt: Option<String> = folder_prefixes.iter().find_map(|prefix| {
            if lower_msg.contains(prefix) {
                // Extract the word(s) after the prefix
                let rest = lower_msg[lower_msg.find(prefix).unwrap() + prefix.len()..].trim().to_string();
                // take first token as the folder name (stop at space or special char)
                let filler_words = ["que", "el", "la", "lo", "un", "una", "te", "pedi",
                                    "me", "mi", "tu", "a", "de", "en", "con", "por",
                                    "para", "como", "al", "del", "se", "le", "les",
                                    "nombre", "llamada", "llamado", "llame", "y"];
                let mut name = String::new();
                for word in rest.split_whitespace() {
                    let clean_word: String = word.chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect();
                    if !clean_word.is_empty() && !filler_words.contains(&clean_word.to_lowercase().as_str()) {
                        name = clean_word;
                        break;
                    }
                }
                if !name.is_empty() {
                    Some(name)
                } else {
                    None
                }
            } else { None }
        });

        let folder_prefix_found = folder_prefixes.iter().any(|p| lower_msg.contains(p));
        if let Some(folder_name) = folder_name_opt {
            agent::emit_event(&app_handle, 0, &format!("[FAST-TRACK] Creando carpeta: {}", folder_name), "ACTION");
            let mkdir_cmd = format!("mkdir \"{}\"", folder_name);
            match crate::core::execute_terminal_command(&workspace_path, &mkdir_cmd).await {
                Ok(_) => {
                    agent::emit_event(&app_handle, 1, &format!("Carpeta '{}' creada exitosamente.", folder_name), "SUCCESS");
                    let resp_msg = format!("✅ Listo. Carpeta `{}` creada en tu workspace.", folder_name);
                    journal.chat_history.push(format!("Usuario: {}", user_message));
                    journal.chat_history.push(format!("Aura: {}", resp_msg));
                    if journal.chat_history.len() > 6 { journal.chat_history.drain(0..journal.chat_history.len() - 6); }
                    crate::core::session_journal::save_journal(&workspace_path, &journal);
                    let response = serde_json::json!({"status": "FINISH", "respuesta_conversacional": resp_msg});
                    return Ok(response.to_string());
                },
                Err(e) => {
                    let resp_msg = if e.contains("ya existe") || e.contains("already exists") || e.contains("MKDIR") {
                        format!("ℹ️ La carpeta `{}` ya existe en tu workspace.", folder_name)
                    } else {
                        format!("⚠️ No pude crear la carpeta `{}`. Error: {}", folder_name, e)
                    };
                    let response = serde_json::json!({"status": "FINISH", "respuesta_conversacional": resp_msg});
                    return Ok(response.to_string());
                }
            }
        } else if folder_prefix_found {
            // Prefix was detected but name was a filler word (e.g. "crea la carpeta que te pedí")
            let resp_msg = "¿Con qué nombre quieres que cree la carpeta? Dime el nombre exacto y la creo al instante.";
            let response = serde_json::json!({"status": "FINISH", "respuesta_conversacional": resp_msg});
            return Ok(response.to_string());
        }

        // ── Hardcoded keyword intercept (faster than NLU for known search verbs) ──
        let search_keywords = ["investiga", "investigue", "investig", "busca", "buscar", "busque", "consulta", "consulte"];
        let forced_search = search_keywords.iter().any(|kw| lower_msg.contains(kw));
        
        if forced_search {
            agent::emit_event(&app_handle, 0, "[INTERCEPT] Verbo de búsqueda detectado. Forzando AGENTIC_TASK.", "INFO");
            enriched_message = format!("Petición Original del Usuario: {}\n\nGuía de Traducción Técnica: El usuario usó un verbo de búsqueda explícito. DEBES usar TOOL_WEB_SEARCH para investigar en internet y luego usar TOOL_FINISH para responder en el chat.", user_message);
            
            journal.chat_history.push(format!("Usuario: {}", user_message));
            crate::core::session_journal::save_journal(&workspace_path, &journal);
        } else {

        let chat_json = crate::memory::load_chat_history(workspace_path.clone()).await.unwrap_or_else(|_| "[]".to_string());
        let mut visual_history = Vec::new();
        if let Ok(messages) = serde_json::from_str::<Vec<serde_json::Value>>(&chat_json) {
            for msg in messages.iter().rev().take(8).rev() {
                let sender = msg.get("sender").and_then(|v| v.as_str()).unwrap_or("unknown");
                let text = msg.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let prefix = if sender == "user" { "Usuario" } else { "Aura" };
                visual_history.push(format!("{}: {}", prefix, text));
            }
        }
        
        let mut combined_history = journal.chat_history.clone();
        for msg in visual_history {
            if !combined_history.contains(&msg) {
                combined_history.push(msg);
            }
        }
        if combined_history.len() > 8 {
            let skip = combined_history.len() - 8;
            combined_history = combined_history.into_iter().skip(skip).collect();
        }

        let mut nlu_response = translator::translate_to_technical_intent(&user_message, &app_handle, &combined_history).await;
        let start_idx = nlu_response.find('{');
        let end_idx = nlu_response.rfind('}');
        if let (Some(s), Some(e)) = (start_idx, end_idx) {
            if e > s { nlu_response = nlu_response[s..=e].to_string(); }
        }
        nlu_response = nlu_response.trim().to_string();
        println!("[NLU] Input: '{}' -> RAW: '{}'", user_message, nlu_response);
        
        let nlu_json: serde_json::Value = serde_json::from_str(&nlu_response).unwrap_or_else(|_| {
            serde_json::json!({
                "intent_type": "AGENTIC_TASK",
                "technical_translation": nlu_response
            })
        });

        let mut intent_type = nlu_json.get("intent_type").and_then(|v| v.as_str()).unwrap_or("AGENTIC_TASK");

        let lower_msg = user_message.to_lowercase();
        if lower_msg.contains("tool_") || lower_msg.contains("script") || lower_msg.contains("reto") || lower_msg.contains("algoritmo") 
        || lower_msg.contains("crea") || lower_msg.contains("procede") || lower_msg.contains("ejecuta") 
        || lower_msg.contains("proyecto") || lower_msg.contains("backend") || lower_msg.contains("frontend")
        || lower_msg.contains("programa") || lower_msg.contains("haz") || lower_msg.contains("prueba") || lower_msg.contains("continua") {
            intent_type = "AGENTIC_TASK";
        }

        if intent_type == "CONVERSATION" {
            let direct_response = nlu_json.get("direct_response").and_then(|v| v.as_str()).unwrap_or("¡Hola! ¿En qué puedo ayudarte?");
            agent::emit_event(&app_handle, 0, "Conversación fluida detectada.", "SUCCESS");
            
            // Save to memory
            journal.chat_history.push(format!("Usuario: {}", user_message));
            journal.chat_history.push(format!("Aura: {}", direct_response));
            if journal.chat_history.len() > 6 {
                journal.chat_history.drain(0..journal.chat_history.len() - 6);
            }
            crate::core::session_journal::save_journal(&workspace_path, &journal);

            let response = serde_json::json!({
                "status": "FINISH",
                "respuesta_conversacional": direct_response
            });
            return Ok(response.to_string());
        }

        if intent_type == "FAST_TRACK_OS" {
            if let Some(cmd) = nlu_json.get("os_command").and_then(|v| v.as_str()) {
                if !cmd.is_empty() && cmd != "null" {
                    agent::emit_event(&app_handle, 0, &format!("[FAST-TRACK] Ejecutando: {}", cmd), "ACTION");
                    match crate::core::execute_terminal_command(&workspace_path, cmd).await {
                        Ok(_) => {
                            agent::emit_event(&app_handle, 0, "[FAST-TRACK] Comando ejecutado con éxito.", "SUCCESS");
                            let resp_msg = format!("✅ Listo. Ejecuté: `{}`", cmd);
                            
                            journal.chat_history.push(format!("Usuario: {}", user_message));
                            journal.chat_history.push(format!("Aura: {}", resp_msg));
                            if journal.chat_history.len() > 6 {
                                journal.chat_history.drain(0..journal.chat_history.len() - 6);
                            }
                            crate::core::session_journal::save_journal(&workspace_path, &journal);

                            let response = serde_json::json!({
                                "status": "FINISH",
                                "respuesta_conversacional": resp_msg
                            });
                            return Ok(response.to_string());
                        },
                        Err(e) => {
                            agent::emit_event(&app_handle, 0, &format!("[FAST-TRACK] Error: {}. Derivando al Agente...", e), "WARNING");
                            // Fall through to AGENTIC_TASK
                        }
                    }
                }
            }
        }

        let technical_intent = nlu_json.get("technical_translation").and_then(|v| v.as_str()).unwrap_or(&user_message);
        enriched_message = format!("Petición Original del Usuario: {}\n\nGuía de Traducción Técnica: {}", user_message, technical_intent);
        } // end else (no keyword intercept)
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
