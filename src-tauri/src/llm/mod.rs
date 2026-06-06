use serde::{Deserialize, Serialize};
use crate::{memory::{self, Cambio}, core::validate_workspace};

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
}

#[derive(Deserialize, Serialize)]
struct ProgrammerOutput {
    explicacion_tecnica: String,
    cambios: Vec<Cambio>,
}

#[derive(Serialize)]
struct PipelineResponse {
    orquestador: OrchestratorOutput,
    programador: serde_json::Value,
    operacion_fisica: String,
    eventos_validacion: Vec<String>,
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
async fn call_ollama_text(model: &str, prompt: &str) -> Result<String, String> {
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
        REGLA DE RUTAS: Si el usuario te pide crear un archivo nuevo en una carpeta específica (ej. 'mi_carpeta/api.js'), DEBES poner exactamente esa ruta en el campo 'archivo'. NO sobrescribas los archivos que te paso como contexto a menos que el usuario te pida explícitamente modificarlos. Si la ruta no existe, el sistema la creará automáticamente.\n\
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
pub async fn process_user_prompt(user_message: String, workspace_path: String) -> Result<String, String> {
    let orchestrator_model = "llama3.1:8b";
    let mut eventos_validacion: Vec<String> = Vec::new();
    
    // Paso 1: Orquestador y RAG
    let workspace_tree_nodes = memory::get_workspace_tree_internal(workspace_path.clone()).await?;
    let files_only: Vec<_> = workspace_tree_nodes.iter().filter(|n| !n.is_dir).collect();
    
    let mut index = memory::read_vector_index(&workspace_path).await;
    
    // Reconstrucción incremental: solo vectoriza archivos NUEVOS, no todos.
    // Si hay más de 50 archivos nuevos (proyecto masivo o nuevo workspace), saltamos
    // directamente al fallback para no bloquear el pipeline.
    const MAX_NUEVOS_A_VECTORIZAR: usize = 50;
    if index.len() != files_only.len() {
        let nuevos: Vec<_> = files_only.iter()
            .filter(|f| !index.iter().any(|n| n.path == f.path))
            .collect();
        
        if nuevos.len() <= MAX_NUEVOS_A_VECTORIZAR {
            let mut new_index = Vec::new();
            for file in &files_only {
                if let Some(existing) = index.iter().find(|n| n.path == file.path) {
                    new_index.push(existing.clone());
                } else {
                    if let Ok(emb) = get_embedding(&file.path).await {
                        new_index.push(memory::VectorNode {
                            path: file.path.clone(),
                            embedding: emb,
                        });
                    }
                }
            }
            index = new_index;
            let _ = memory::write_vector_index(&workspace_path, &index).await;
        } else {
            // Demasiados archivos nuevos: limpiamos el índice y usamos fallback directo
            eprintln!("Aura-Sentinel [RAG]: {} archivos nuevos detectados. Usando fallback sin vectorización.", nuevos.len());
            index.clear();
        }
    }
    
    // Si los embeddings fallan (modelo no disponible/timeout), usamos todos los archivos sin filtrar
    let user_embedding = get_embedding(&user_message).await.unwrap_or_default();
    
    let tree_json = if user_embedding.is_empty() || index.is_empty() {
        // Fallback rápido: mandamos todos los nombres de archivo sin ranking semántico
        eprintln!("Aura-Sentinel [RAG]: Embeddings no disponibles, usando listado completo.");
        let all_paths: Vec<String> = files_only.iter().take(15).map(|f| f.path.clone()).collect();
        serde_json::to_string(&all_paths).unwrap_or_else(|_| "[]".to_string())
    } else {
        let mut top_files: Vec<(String, f32)> = index.iter().map(|n| {
            let sim = crate::core::cosine_similarity(&user_embedding, &n.embedding);
            (n.path.clone(), sim)
        }).collect();
        top_files.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let top_10: Vec<String> = top_files.into_iter().take(10).map(|(p, _)| p).collect();
        serde_json::to_string(&top_10).unwrap_or_else(|_| "[]".to_string())
    };

    let system_prompt = format!(
        "Rol: Eres Aura-Sentinel, el Orquestador Arquitectónico.\n\
        Contexto: Este es el mapa actual del proyecto: {}\n\
        Tarea del Usuario: {}\n\
        REGLA DE TOLERANCIA COGNITIVA: El usuario escribirá rápido. Ignora faltas de ortografía (ej. 'rvisa' = revisa, 'bn' = bien, 'analia' = analiza). Extrae la intención real.\n\
        Instrucción: Clasifica la intención del usuario. AHORA ERES MULTI-TAREA. Puedes combinar varias acciones (ej. ejecutar un comando Y programar algo). \n\
        Rellena los campos JSON correspondientes según lo que pida el usuario:\n\
        1. 'CHAT': SOLO para saludos ('hola') o preguntas teóricas que no requieren operar.\n\
        2. 'COMANDO': Si pide ejecutar algo, rellena 'comando_a_ejecutar' (npm, cargo, python).\n\
        3. 'INVESTIGACION': Si pide leer URLs, rellena 'url_a_investigar'.\n\
        4. 'AUDITORIA': REGLA ABSOLUTA: Si el usuario usa palabras como 'revisa', 'analiza', 'errores', o 'describe', rellena 'archivos_a_analizar' con los archivos a revisar e indica esta intención.\n\
        5. 'ACCION': Para crear o modificar código físico.\n\
        Si pide varias cosas, pon 'ACCION' en intención y rellena todos los campos que hagan falta.\n\
        Tu respuesta DEBE ser ÚNICAMENTE un objeto JSON válido con la siguiente estructura, sin texto adicional ni markdown:\n\
        {{\n\
          \"intencion\": \"<INTENCION_SELECCIONADA>\",\n\
          \"respuesta_conversacional\": \"Tu respuesta natural al usuario\",\n\
          \"pensamiento\": \"Breve justificación de tu decisión\",\n\
          \"archivos_a_analizar\": [\"ruta/completa/al/archivo.py\"],\n\
          \"modelo_sugerido\": \"qwen2.5-coder:7b\",\n\
          \"comando_a_ejecutar\": null,\n\
          \"url_a_investigar\": null\n\
        }}",
        tree_json, user_message
    );

    let orchestrator_res = call_ollama(orchestrator_model, &system_prompt).await?;

    // Sanitizamos la respuesta por si el LLM incluyó bloques de código markdown
    let mut clean_json = orchestrator_res.trim().to_string();
    if clean_json.starts_with("```json") {
        clean_json = clean_json.trim_start_matches("```json").to_string();
    } else if clean_json.starts_with("```") {
        clean_json = clean_json.trim_start_matches("```").to_string();
    }
    if clean_json.ends_with("```") {
        clean_json = clean_json.trim_end_matches("```").to_string();
    }
    clean_json = clean_json.trim().to_string();

    let orch_output: OrchestratorOutput = match serde_json::from_str(&clean_json) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Aura-Sentinel: Error deserializando orquestador: {}", e);
            eprintln!("Raw JSON: {}", orchestrator_res);
            return Ok(orchestrator_res);
        }
    };

    let intent_str = orch_output.intencion.to_uppercase();
    // AUDITORIA absorbe ANALISIS: cualquier petición de leer/entender/revisar código va al auditor
    let is_chat          = intent_str.contains("CHAT")         && !intent_str.contains("ACCION") && !intent_str.contains("COMANDO") && !intent_str.contains("INVESTIGACION") && !intent_str.contains("AUDITORIA") && !intent_str.contains("ANALISIS");
    let is_auditoria     = (intent_str.contains("AUDITORIA") || intent_str.contains("ANALISIS")) && !intent_str.contains("ACCION");
    let is_accion        = intent_str.contains("ACCION");

    let mut operacion_fisica_log = String::new();
    let mut file_contents = String::new();
    let mut terminal_web_context = String::new();
    
    // Si no hay campos operativos llenos y es CHAT, terminamos temprano
    if is_chat && orch_output.comando_a_ejecutar.is_none() && orch_output.url_a_investigar.is_none() && orch_output.archivos_a_analizar.is_empty() {
        let pipeline_res = PipelineResponse {
            orquestador: orch_output,
            programador: serde_json::Value::Null,
            operacion_fisica: "Modo CHAT: Sin operaciones de disco ni sistema.".to_string(),
            eventos_validacion: vec![],
        };
        return serde_json::to_string(&pipeline_res).map_err(|e| format!("Error serializando pipeline: {}", e));
    }


    // --- PASO 1: COMANDO DE TERMINAL ---
    if let Some(ref comando) = orch_output.comando_a_ejecutar {
        if !comando.trim().is_empty() {
            match crate::core::execute_terminal_command(&workspace_path, comando).await {
                Ok(out) => {
                    operacion_fisica_log.push_str(&format!("Comando ejecutado con éxito:\n{}\n\n", out));
                    terminal_web_context.push_str(&format!("--- SALIDA DE TERMINAL ({}) ---\n{}\n\n", comando, out));
                },
                Err(err) => {
                    operacion_fisica_log.push_str(&format!("Error ejecutando comando:\n{}\n\n", err));
                    terminal_web_context.push_str(&format!("--- ERROR DE TERMINAL ({}) ---\n{}\n\n", comando, err));
                }
            }
            use std::time::{SystemTime, UNIX_EPOCH};
            let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs().to_string();
            let entry = memory::types::FenixMemoryLog {
                task_id: format!("CMD-{}", timestamp),
                timestamp,
                file_path: "TERMINAL".to_string(),
                summary: format!("Comando ejecutado: {}", comando),
                previous_hash: "none".to_string(),
                compilation_status: "COMPLETADO".to_string(),
            };
            let _ = memory::add_memory_entry(workspace_path.clone(), entry).await;
        }
    }

    // --- PASO 2: INVESTIGACIÓN WEB ---
    if let Some(ref url) = orch_output.url_a_investigar {
        if !url.trim().is_empty() {
            match crate::net::fetch_url_text(url).await {
                Ok(content) => {
                    let preview = if content.len() > 300 { format!("{}...", &content[..300]) } else { content.clone() };
                    operacion_fisica_log.push_str(&format!("Contenido extraído de la web ({}):\n{}\n\n", url, preview));
                    terminal_web_context.push_str(&format!("--- CONTEXTO WEB ({}) ---\n{}\n\n", url, content));
                    
                    use std::time::{SystemTime, UNIX_EPOCH};
                    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs().to_string();
                    let entry = memory::types::FenixMemoryLog {
                        task_id: format!("WEB-{}", timestamp),
                        timestamp,
                        file_path: "WEB_SCRAPER".to_string(),
                        summary: format!("URL leída: {}", url),
                        previous_hash: "none".to_string(),
                        compilation_status: "COMPLETADO".to_string(),
                    };
                    let _ = memory::add_memory_entry(workspace_path.clone(), entry).await;
                },
                Err(e) => {
                    operacion_fisica_log.push_str(&format!("Error al extraer contenido de {}: {}\n", url, e));
                }
            }
        }
    }

    // --- PASO 3: ARCHIVOS LOCALES ---
    if !orch_output.archivos_a_analizar.is_empty() {
        let safe_files = memory::read_files_safely(&workspace_path, orch_output.archivos_a_analizar.clone()).await;
        file_contents.push_str(&safe_files);
    }
    
    let combined_programmer_context = format!("{}{}", file_contents, terminal_web_context);
    
    let programmer_model = if orch_output.modelo_sugerido.trim().is_empty() {
        "qwen2.5-coder:7b"
    } else {
        &orch_output.modelo_sugerido
    };

    let mut prog_output_final: serde_json::Value = serde_json::json!({});

    // --- PASO 4: AUDITORÍA ---
    // Si requiere auditoría, inyectamos el combined_context en el prompt de auditoría
    if is_auditoria {
        let contenido = if !combined_programmer_context.trim().is_empty() {
            combined_programmer_context.clone()
        } else {
            let fallback_files: Vec<String> = files_only.iter()
                .filter(|f| f.path.to_lowercase().ends_with(".rs") || f.path.to_lowercase().ends_with(".py") || f.path.to_lowercase().ends_with(".js"))
                .take(8)
                .map(|f| f.path.clone())
                .collect();
            memory::read_files_safely(&workspace_path, fallback_files).await
        };

        let modelo = if orch_output.modelo_sugerido.trim().is_empty() { "qwen2.5-coder:7b" } else { &orch_output.modelo_sugerido };
        let reporte_auditoria = delegate_to_auditor(&contenido, modelo).await;

        let mut orch_con_auditoria = orch_output;
        orch_con_auditoria.respuesta_conversacional = reporte_auditoria;

        let pipeline_res = PipelineResponse {
            orquestador: orch_con_auditoria,
            programador: serde_json::Value::Null,
            operacion_fisica: format!("{}Modo AUDITORIA: Reporte generado.\n", operacion_fisica_log),
            eventos_validacion: vec![],
        };
        return serde_json::to_string(&pipeline_res).map_err(|e| format!("Error serializando pipeline: {}", e));
    }

    // --- PASO 5: ACCIÓN (Programador) ---
    if is_accion || (!combined_programmer_context.trim().is_empty() && (orch_output.comando_a_ejecutar.is_some() || orch_output.url_a_investigar.is_some())) {
        // Delegamos al Ejecutor inyectando todo el contexto
        let final_json_str = delegate_to_programmer(&user_message, &combined_programmer_context, programmer_model).await?;

        // Intentamos parsear el JSON del programador
        if let Ok(mut prog_output) = serde_json::from_str::<ProgrammerOutput>(&final_json_str) {
            
            let mut max_intentos = 2;
            let mut loop_activo = true;

            while loop_activo && max_intentos > 0 {
                if !prog_output.cambios.is_empty() {
                    match memory::apply_code_changes(&workspace_path, prog_output.cambios.clone()).await {
                        Ok(msg) => {
                            operacion_fisica_log.push_str(&format!("[PROGRAMADOR] {}\n", msg));
                            
                            eventos_validacion.push("[VALIDACIÓN] Validando workspace...".to_string());
                            
                            match validate_workspace(&workspace_path).await {
                                Ok(_) => {
                                    let _ = memory::update_last_memory_status(&workspace_path, "COMPILACIÓN_EXITOSA").await;
                                    eventos_validacion.push("[ÉXITO] El código valida correctamente. Memoria actualizada.".to_string());
                                    loop_activo = false;
                                },
                                Err(error_log) => {
                                    eventos_validacion.push(format!("[ERROR DETECTADO] Fallo de validación. Iniciando protocolo de Auto-Reparación..."));
                                    
                                    max_intentos -= 1;
                                    if max_intentos > 0 {
                                        let repair_prompt = format!(
                                            "El código que modificaste causó el siguiente error de compilación/validación: \n{}\nAnaliza el error y genera un nuevo JSON de cambios (con buscar y reemplazar) para solucionarlo de inmediato.",
                                            error_log
                                        );
                                        
                                        let content_releido = memory::read_files_safely(&workspace_path, orch_output.archivos_a_analizar.clone()).await;
                                        
                                        match delegate_to_programmer(&repair_prompt, &content_releido, programmer_model).await {
                                            Ok(new_json) => {
                                                if let Ok(new_prog) = serde_json::from_str::<ProgrammerOutput>(&new_json) {
                                                    prog_output = new_prog;
                                                } else {
                                                    loop_activo = false;
                                                    eventos_validacion.push("[FATAL] El modelo no devolvió JSON válido en la auto-reparación.".to_string());
                                                }
                                            },
                                            Err(e) => {
                                                loop_activo = false;
                                                eventos_validacion.push(format!("[FATAL] Error conectando con el modelo en reparación: {}", e));
                                            }
                                        }
                                    } else {
                                        let _ = memory::update_last_memory_status(&workspace_path, "COMPILACIÓN_FALLIDA").await;
                                        eventos_validacion.push("[FATAL] Máximo de intentos de auto-reparación alcanzado.".to_string());
                                    }
                                }
                            }
                        },
                        Err(e) => {
                            operacion_fisica_log.push_str(&format!("Error en aplicación física: {}\n", e));
                            loop_activo = false;
                        }
                    }
                } else {
                    loop_activo = false;
                }
            }
            
            prog_output_final = serde_json::to_value(prog_output).unwrap_or_else(|_| serde_json::json!({}));
        } else {
            prog_output_final = serde_json::from_str(&final_json_str).unwrap_or_else(|_| serde_json::json!({ "raw": final_json_str }));
        }
    }

    // Remover bloque obsoleto COMANDO al final

    let pipeline_res = PipelineResponse {
        orquestador: orch_output,
        programador: prog_output_final,
        operacion_fisica: operacion_fisica_log,
        eventos_validacion,
    };

    serde_json::to_string(&pipeline_res).map_err(|e| format!("Error serializando pipeline: {}", e))
}
