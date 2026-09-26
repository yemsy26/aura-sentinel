use serde::{Deserialize, Serialize};

pub mod agent;
pub mod phase_planner;
pub mod router;
pub mod translator;

#[derive(Serialize)]
struct OllamaRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
    format: serde_json::Value,
    options: serde_json::Value,
}

#[derive(Deserialize)]
struct OllamaResponse {
    response: String,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub(crate) struct ProgrammerOutput {
    pub pensamiento: Option<String>,
    pub explicacion_tecnica: String,
    pub cambios: Vec<crate::memory::Cambio>,
}

fn context_from_available_memory(
    free_vram_mib: u64,
    model_size_mib: Option<u64>,
    is_loaded: bool,
    high_ram_pressure: bool,
) -> u32 {
    let estimated_free = if is_loaded {
        free_vram_mib
    } else {
        // Unknown model sizes get a conservative 6 GiB reservation. This avoids
        // granting a large context if Ollama metadata is temporarily unavailable.
        free_vram_mib.saturating_sub(model_size_mib.unwrap_or(6144))
    };
    let gpu_context = if estimated_free >= 1024 {
        8192
    } else if estimated_free >= 512 {
        4096
    } else {
        2048
    };
    if high_ram_pressure {
        gpu_context.min(4096)
    } else {
        gpu_context
    }
}

async fn get_safe_num_ctx(model: &str) -> u32 {
    use sysinfo::System;
    let mut sys = System::new_all();
    sys.refresh_all();
    let total_mem = sys.total_memory() as f64;
    let used_mem = sys.used_memory() as f64;
    let high_ram_pressure = total_mem > 0.0 && (used_mem / total_mem) > 0.80;

    // Read only local runtime metadata. Missing tools/API must never block inference
    // or trigger an implicit model install.
    let free_vram_mib = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::process::Command::new("nvidia-smi")
            .args(["--query-gpu=memory.free", "--format=csv,noheader,nounits"])
            .stdin(std::process::Stdio::null())
            .output(),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .filter(|output| output.status.success())
    .and_then(|output| String::from_utf8(output.stdout).ok())
    .and_then(|output| output.lines().next()?.trim().parse::<u64>().ok());

    let Some(free_vram_mib) = free_vram_mib else {
        return if high_ram_pressure { 2048 } else { 4096 };
    };

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(client) => client,
        Err(_) => {
            return context_from_available_memory(free_vram_mib, None, false, high_ram_pressure)
        }
    };

    let (tags, running) = tokio::join!(
        client.get("http://127.0.0.1:11434/api/tags").send(),
        client.get("http://127.0.0.1:11434/api/ps").send(),
    );
    let tags = match tags {
        Ok(response) if response.status().is_success() => {
            response.json::<serde_json::Value>().await.ok()
        }
        _ => None,
    };
    let running = match running {
        Ok(response) if response.status().is_success() => {
            response.json::<serde_json::Value>().await.ok()
        }
        _ => None,
    };

    let matches_model = |candidate: &str| {
        candidate == model
            || candidate.strip_suffix(":latest") == Some(model)
            || model.strip_suffix(":latest") == Some(candidate)
    };
    let is_loaded = running
        .as_ref()
        .and_then(|value| value.get("models"))
        .and_then(|value| value.as_array())
        .is_some_and(|models| {
            models.iter().any(|entry| {
                entry
                    .get("name")
                    .and_then(|name| name.as_str())
                    .is_some_and(matches_model)
            })
        });
    let model_size_mib = tags
        .as_ref()
        .and_then(|value| value.get("models"))
        .and_then(|value| value.as_array())
        .and_then(|models| {
            models.iter().find_map(|entry| {
                let name = entry.get("name")?.as_str()?;
                if !matches_model(name) {
                    return None;
                }
                entry
                    .get("size")?
                    .as_u64()
                    .map(|bytes| bytes / (1024 * 1024))
            })
        });

    context_from_available_memory(free_vram_mib, model_size_mib, is_loaded, high_ram_pressure)
}

#[cfg(test)]
mod context_budget_tests {
    use super::context_from_available_memory;

    #[test]
    fn context_budget_reserves_model_memory_before_loading() {
        assert_eq!(
            context_from_available_memory(8192, Some(4600), false, false),
            8192
        );
        assert_eq!(
            context_from_available_memory(8192, Some(7000), false, false),
            8192
        );
        assert_eq!(
            context_from_available_memory(8192, Some(7400), false, false),
            4096
        );
        assert_eq!(
            context_from_available_memory(8192, Some(7800), false, false),
            2048
        );
        assert_eq!(
            context_from_available_memory(8192, None, false, false),
            8192
        );
    }

    #[test]
    fn loaded_model_uses_current_free_vram_and_ram_pressure_caps_context() {
        assert_eq!(
            context_from_available_memory(3000, Some(7000), true, false),
            8192
        );
        assert_eq!(
            context_from_available_memory(3000, Some(7000), true, true),
            4096
        );
    }
}

pub async fn call_ollama(model: &str, prompt: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| format!("Error construyendo cliente HTTP: {}", e))?;
    let url = "http://127.0.0.1:11434/api/generate";

    // Forzamos JSON para el Orquestador (necesita respuesta estructurada)
    let payload = OllamaRequest {
        model,
        prompt,
        stream: false,
        format: serde_json::json!("json"),
        // Tool routing is a small JSON decision. A 4K generation budget made 7B
        // models slower and more likely to ramble without improving the choice.
        options: serde_json::json!({ "num_ctx": get_safe_num_ctx(model).await, "num_predict": 1024, "repeat_penalty": 1.1, "temperature": 0.1 }),
    };

    let res = client
        .post(url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Error conectando a Ollama: {}", e))?;

    if res.status().is_success() {
        let ollama_res: OllamaResponse = res
            .json()
            .await
            .map_err(|e| format!("Error parseando la respuesta JSON: {}", e))?;
        Ok(ollama_res.response)
    } else {
        Err(format!("Error de Ollama. Status: {}", res.status()))
    }
}

#[allow(dead_code)]
pub async fn call_ollama_with_schema(
    model: &str,
    prompt: &str,
    schema: serde_json::Value,
) -> Result<String, String> {
    call_ollama_with_schema_options(model, prompt, schema, 8192, 0.2).await
}

/// Schema-constrained generation with caller-selected latency and determinism.
/// Routing and NLU use small outputs; code generation keeps the larger default.
pub async fn call_ollama_with_schema_options(
    model: &str,
    prompt: &str,
    schema: serde_json::Value,
    max_output_tokens: u32,
    temperature: f32,
) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| format!("Error construyendo cliente HTTP: {}", e))?;
    let url = "http://127.0.0.1:11434/api/generate";

    let payload = OllamaRequest {
        model,
        prompt,
        stream: false,
        format: schema,
        options: serde_json::json!({
            "num_ctx": get_safe_num_ctx(model).await,
            "num_predict": max_output_tokens.max(64),
            "repeat_penalty": 1.05,
            "temperature": temperature.clamp(0.0, 1.0)
        }),
    };

    let res = client
        .post(url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Error conectando a Ollama: {}", e))?;

    if res.status().is_success() {
        let ollama_res: OllamaResponse = res
            .json()
            .await
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
        options: serde_json::Value,
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| format!("Error construyendo cliente HTTP (texto): {}", e))?;
    let url = "http://127.0.0.1:11434/api/generate";

    let payload = TextRequest {
        model,
        prompt,
        stream: false,
        options: serde_json::json!({ "num_ctx": get_safe_num_ctx(model).await, "num_predict": 4096, "repeat_penalty": 1.1, "temperature": 0.2 }),
    };

    let res = client
        .post(url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Error conectando a Ollama (texto): {}", e))?;

    if res.status().is_success() {
        let ollama_res: OllamaResponse = res
            .json()
            .await
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

pub const DEFAULT_EMBEDDING_MODEL: &str = "nomic-embed-text";

pub async fn get_embedding(text: &str) -> Result<Vec<f32>, String> {
    // Timeout de 30s para resiliencia en sistemas con CPU/Ollama bajo carga
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Error construyendo cliente de embeddings: {}", e))?;
    let url = "http://127.0.0.1:11434/api/embeddings";

    let payload = EmbeddingRequest {
        model: DEFAULT_EMBEDDING_MODEL,
        prompt: text,
    };

    let res = client
        .post(url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Timeout/Error en embeddings: {}", e))?;

    if res.status().is_success() {
        let ollama_res: EmbeddingResponse = res
            .json()
            .await
            .map_err(|e| format!("Error parseando respuesta de embeddings: {}", e))?;
        Ok(ollama_res.embedding)
    } else {
        Err(format!(
            "Error de Ollama Embeddings. Status: {}",
            res.status()
        ))
    }
}

pub(crate) async fn delegate_to_programmer(
    task: &str,
    file_contents: &str,
    requested_files: &[String],
    require_patch: bool,
    require_empty_search: bool,
    max_changes: usize,
    model: &str,
) -> Result<String, String> {
    let patch_rule = if require_patch {
        "MODO REPARACIÓN INCREMENTAL OBLIGATORIO: 'buscar' no puede estar vacío. Copia en 'buscar' un fragmento breve y literal del archivo actual o del borrador rechazado incluido en el diagnóstico; en 'reemplazar' devuelve solo ese fragmento corregido. Conserva automáticamente el resto. No pegues el archivo completo, no anexes una segunda copia y no dupliques funciones existentes."
    } else if require_empty_search {
        "MODO REEMPLAZO COMPLETO OBLIGATORIO: 'buscar' debe ser exactamente una cadena vacía. Devuelve en 'reemplazar' el contenido completo y corregido de cada archivo objetivo, tomando como base su contenido actual incluido arriba. No devuelvas parches parciales."
    } else {
        "Para crear un archivo nuevo usa 'buscar' vacío. Si la instrucción pide reemplazar un archivo completo, usa 'buscar' vacío y devuelve el archivo entero corregido en 'reemplazar'. En los demás cambios de archivos existentes, prefiere un fragmento literal breve en 'buscar'."
    };
    // JS DOM guideline kept as concat! to avoid rustc misinterpreting backticks/quotes inside format!
    let js_dom_guideline = concat!(
        "Para JavaScript de navegador: usa manipulacion directa del DOM ",
        "(document.createElement, textContent, addEventListener) o delegacion de eventos. ",
        "NUNCA uses atributos de manejadores inline (onclick, onsubmit, onchange, etc.) dentro de ",
        "cadenas HTML o template literals en JavaScript; ",
        "las comillas anidadas rompen la serializacion JSON del codigo y causan errores de sintaxis. ",
        "Cierra siempre todas las funciones, bloques y llaves antes del final del archivo."
    );
    let system_prompt = format!(
        "Eres el programador de Aura Sentinel. Implementa por completo la accion solicitada, respetando el codigo existente y el objetivo global.\n\nTAREA:\n{}\n\nARCHIVOS ACTUALES Y DIAGNOSTICO:\n{}\n\nDevuelve un objeto JSON con explicacion_tecnica y cambios. Cada cambio contiene archivo (ruta relativa), buscar (texto exacto; vacio solo al crear o reemplazar por completo) y reemplazar (codigo que sustituye ese fragmento).\n{}\nNo uses placeholders ni funciones vacias. No inventes archivos fuera de los solicitados. Devuelve como maximo {} cambios, con un solo cambio por archivo y nunca repitas el mismo valor de archivo. Usa sintaxis valida del lenguaje; en Python son validas comillas simples y dobles. {} Escapa las cadenas del JSON sin alterar el contenido del codigo. Manten la respuesta compacta: no anadas filas repetitivas, datos de relleno ni codigo dentro de comentarios; para interfaces, renderiza los registros desde datos reales del programa. No crees archivos de prueba, documentacion o configuracion auxiliar si el usuario no los solicito ni son necesarios para un comando real de validacion. Para una interfaz, las pruebas interactivas se hacen con el navegador; no generes pruebas Node que dependan de document o alert.\nPara verify_py: usa solo json, pathlib y re de la biblioteca estandar; lee los archivos reales, comprueba CADA requisito numerado del objetivo con al menos un check independiente y ejecuta los checks bajo if __name__ == __main__. Busca tokens simples por separado; NUNCA incrustes una linea HTML o JavaScript completa con comillas anidadas dentro de una cadena Python. Emite exactamente una linea JSON: passed debe ser el NUMERO ENTERO de checks aprobados (nunca booleano), total debe ser el NUMERO ENTERO de checks ejecutados y ser al menos 5 en esta tarea, percentage debe ser 100*passed/total, y failed_criteria debe ser una lista de textos con longitud total-passed. Termina con codigo 1 si hay fallos. Definir funciones sin llamarlas NO es una verificacion. Evita nombres de IDs inventados: usa el HTML real que figura arriba.\nLos scripts automaticos no deben pedir input ni usar pause. No cambies codigo correcto por causa de un error del entorno.",
        task, file_contents, patch_rule, max_changes.max(1), js_dom_guideline
    );
    let buscar_schema = if require_patch {
        serde_json::json!({"type":"string", "minLength":1})
    } else if require_empty_search {
        serde_json::json!({"type":"string", "enum":[""]})
    } else {
        serde_json::json!({"type":"string"})
    };
    let schema = serde_json::json!({
        "type":"object", "required":["explicacion_tecnica","cambios"], "additionalProperties":false,
        "properties":{
            "explicacion_tecnica":{"type":"string"},
            "cambios":{"type":"array","minItems":1,"maxItems":max_changes.max(1).min(requested_files.len().max(1)),"items":{
                "type":"object","required":["archivo","buscar","reemplazar"],"additionalProperties":false,
                "properties":{"archivo":{"type":"string","enum":requested_files},"buscar":buscar_schema,"reemplazar":{"type":"string"}}
            }}
        }
    });
    call_ollama_with_schema_options(model, &system_prompt, schema, 8192, 0.2).await
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
    call_ollama_text(model, &audit_prompt)
        .await
        .unwrap_or_else(|e| format!("Error en auditoría: {}", e))
}

/// Invoca SpectraSAT para resolver una fórmula booleana CNF en memoria.
/// La revisión semántica de código se realiza por separado en `delegate_to_logic_solver`.
pub(crate) fn solve_with_spectrasat(n_vars: usize, clauses: Vec<Vec<i32>>) -> String {
    spectrasat_core::solve_native_rust(n_vars, clauses)
}

pub(crate) async fn delegate_to_logic_solver(file_contents: &str, model: &str) -> String {
    // Modo análisis de código: el LLM detecta problemas lógicos en el código fuente
    let solver_prompt = format!(
        "Eres un Motor de Verificación Formal (Logic Solver). Analiza si el código adjunto contiene \
        fallos lógicos, condiciones inalcanzables, bucles infinitos o dependencias rotas.\n\n\
        CÓDIGO A ANALIZAR:\n{}\n\n\
        INSTRUCCIONES:\n\
        1. Analiza el flujo de control rigurosamente.\n\
        2. Identifica variables no inicializadas.\n\
        3. Detecta Dead Code (condiciones imposibles de cumplir).\n\
        4. Comprueba límites de memoria o recursión.\n\
        5. Si detectas un problema de satisfacibilidad booleana (SAT/UNSAT), exprésalo en formato CNF.\n\n\
        REPORTE LÓGICO:",
        file_contents
    );
    call_ollama_text(model, &solver_prompt)
        .await
        .unwrap_or_else(|e| format!("Error en verificación lógica: {}", e))
}

struct AgentLockGuard;
impl AgentLockGuard {
    fn try_lock() -> Option<Self> {
        if !AGENT_RUNNING.swap(true, std::sync::atomic::Ordering::SeqCst) {
            Some(AgentLockGuard)
        } else {
            None
        }
    }
}
impl Drop for AgentLockGuard {
    fn drop(&mut self) {
        AGENT_RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}
static AGENT_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static AGENT_CANCELLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn request_agent_cancel() {
    AGENT_CANCELLED.store(true, std::sync::atomic::Ordering::SeqCst);
}

pub fn is_agent_cancelled() -> bool {
    AGENT_CANCELLED.load(std::sync::atomic::Ordering::SeqCst)
}

pub fn reset_agent_cancel() {
    AGENT_CANCELLED.store(false, std::sync::atomic::Ordering::SeqCst);
}

#[tauri::command]
pub async fn process_user_prompt(
    mut user_message: String,
    workspace_path: String,
    orchestrator_model: String,
    programmer_model: String,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    let _guard = match AgentLockGuard::try_lock() {
        Some(g) => {
            reset_agent_cancel();
            g
        }
        None => {
            return Ok(serde_json::json!({
                "status": "FINISH",
                "respuesta_conversacional": "⚠️ Sistema Ocupado: AuraSentinel ya está ejecutando una misión. Por favor, espera a que termine antes de enviar otra instrucción."
            }).to_string());
        }
    };

    // Sanitizer Backend: Eliminar inyecciones accidentales de historiales pegados.
    if user_message.to_lowercase().starts_with("[user]") {
        user_message = user_message[6..].trim().to_string();
    }
    if let Some(idx) = user_message.find("[SYSTEM]") {
        user_message = user_message[..idx].trim().to_string();
    }

    let mut profile_snapshot =
        crate::core::user_profile::load(&app_handle, Some(&workspace_path)).unwrap_or_default();
    if crate::core::user_profile::remember_explicit_facts(
        &mut profile_snapshot.profile,
        &user_message,
    ) {
        match crate::core::user_profile::save(&app_handle, profile_snapshot.profile.clone()) {
            Ok(saved) => {
                profile_snapshot.profile = saved;
                agent::emit_event(
                    &app_handle,
                    0,
                    "[PERFIL] Se guardó una preferencia personal explícita en el perfil local.",
                    "INFO",
                );
            }
            Err(error) => agent::emit_event(
                &app_handle,
                0,
                &format!("[PERFIL] No se pudo guardar la preferencia: {error}"),
                "WARNING",
            ),
        }
    }
    let profile_context = crate::core::user_profile::prompt_context(&profile_snapshot);

    let mut enriched_message = String::new();
    // The selected workspace remains authoritative, including continuation requests.
    let mut journal = crate::core::session_journal::load_journal(&workspace_path);
    let mut active_mission_context = crate::core::intent_router::active_mission_context(&journal);
    let recovered_resume_objective = if crate::core::intent_router::is_resume_command(&user_message)
    {
        let chat_json = crate::memory::load_chat_history(workspace_path.clone())
            .await
            .unwrap_or_else(|_| "[]".to_string());
        crate::core::intent_router::recover_pending_approved_plan_request(&chat_json, &user_message)
    } else {
        None
    };

    // ── Zero-latency meta-command intercept ──────────────────────────────────
    if let Some(action) = crate::core::intent_router::try_handle_meta_command_with_recovery(
        &user_message,
        &workspace_path,
        recovered_resume_objective.as_deref(),
    ) {
        match action {
            crate::core::intent_router::IntentAction::Finish(msg) => {
                agent::emit_event(&app_handle, 0, "[META-CMD] Consulta de estado detectada. Respondiendo desde la memoria local...", "PLANNING");
                agent::emit_event(&app_handle, 1, "Respuesta generada sin IA.", "SUCCESS");
                let response = serde_json::json!({
                    "status": "FINISH",
                    "respuesta_conversacional": msg
                });
                return Ok(response.to_string());
            }
            crate::core::intent_router::IntentAction::Resume {
                objetivo,
                resume_msg,
            } => {
                agent::emit_event(&app_handle, 0, &resume_msg, "INFO");
                if objetivo != journal.objetivo {
                    match crate::core::session_journal::restore_mission_for_resume(
                        &workspace_path,
                        &objetivo,
                        recovered_resume_objective.is_some(),
                    ) {
                        Ok(restored) => {
                            journal = restored;
                            active_mission_context =
                                crate::core::intent_router::active_mission_context(&journal);
                            agent::emit_event(
                                &app_handle,
                                0,
                                "[REANUDACIÓN] Objetivo recuperado; el agente validará y reconstruirá el plan aprobado conservando el trabajo existente.",
                                "SUCCESS",
                            );
                        }
                        Err(error) => {
                            let response = serde_json::json!({
                                "status": "ERROR",
                                "respuesta_conversacional": format!("No pude restaurar el diario de la misión: {}", error)
                            });
                            return Ok(response.to_string());
                        }
                    }
                }
                // Preserve the continuation signal so the agent restores the checkpoint.
                enriched_message = "continua".to_string();
            }
        }
    }

    if enriched_message.is_empty() {
        let lower_msg = user_message.to_lowercase();

        // ── Context-aware follow-up for folder creation ──────────────────────────
        let waiting_for_folder_name = journal
            .chat_history
            .last()
            .map(|msg| {
                msg.contains("¿Con qué nombre quieres que cree la carpeta? Dime el nombre exacto")
            })
            .unwrap_or(false);

        if waiting_for_folder_name && !user_message.trim().is_empty() {
            let folder_name: String = user_message
                .split_whitespace()
                .next()
                .unwrap_or("")
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .collect();

            if !folder_name.is_empty() {
                agent::emit_event(
                    &app_handle,
                    0,
                    &format!(
                        "[FAST-TRACK] Creando carpeta (seguimiento): {}",
                        folder_name
                    ),
                    "ACTION",
                );
                let mkdir_cmd = format!("mkdir \"{}\"", folder_name);
                match crate::core::execute_terminal_command(&workspace_path, &mkdir_cmd).await {
                    Ok(_) => {
                        agent::emit_event(
                            &app_handle,
                            1,
                            &format!("Carpeta '{}' creada exitosamente.", folder_name),
                            "SUCCESS",
                        );
                        let resp_msg = format!(
                            "✅ Listo. Carpeta `{}` creada en tu workspace.",
                            folder_name
                        );
                        journal
                            .chat_history
                            .push(format!("Usuario: {}", user_message));
                        journal.chat_history.push(format!("Aura: {}", resp_msg));
                        if journal.chat_history.len() > 6 {
                            journal
                                .chat_history
                                .drain(0..journal.chat_history.len() - 6);
                        }
                        let _ =
                            crate::core::session_journal::save_journal(&workspace_path, &journal);
                        let response = serde_json::json!({"status": "FINISH", "respuesta_conversacional": resp_msg});
                        return Ok(response.to_string());
                    }
                    Err(e) => {
                        let resp_msg = if e.contains("ya existe")
                            || e.contains("already exists")
                            || e.contains("MKDIR")
                        {
                            format!("ℹ️ La carpeta `{}` ya existe en tu workspace.", folder_name)
                        } else {
                            format!(
                                "⚠️ No pude crear la carpeta `{}`. Error: {}",
                                folder_name, e
                            )
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
            "crea una carpeta con nombre",
            "crear una carpeta con nombre",
            "crea la carpeta con nombre",
            "crear la carpeta con nombre",
            "crea una carpeta llamada",
            "crear una carpeta llamada",
            "crea la carpeta llamada",
            "crear la carpeta llamada",
            "crea una carpeta",
            "crea la carpeta",
            "crea carpeta",
            "crear carpeta",
            "crea el directorio",
            "crear directorio",
            "crea directorio",
            "make a folder",
            "make folder",
            "create folder",
            "create directory",
        ];
        // Sort by length descending so longer prefixes match first
        folder_prefixes.sort_by_key(|b| std::cmp::Reverse(b.len()));
        let folder_name_opt: Option<String> = folder_prefixes.iter().find_map(|prefix| {
            if lower_msg.contains(prefix) {
                // Extract the word(s) after the prefix
                let rest = lower_msg[lower_msg.find(prefix).unwrap() + prefix.len()..]
                    .trim()
                    .to_string();
                // take first token as the folder name (stop at space or special char)
                let filler_words = [
                    "que", "el", "la", "lo", "un", "una", "te", "pedi", "me", "mi", "tu", "a",
                    "de", "en", "con", "por", "para", "como", "al", "del", "se", "le", "les",
                    "nombre", "llamada", "llamado", "llame", "y",
                ];
                let mut name = String::new();
                for word in rest.split_whitespace() {
                    let clean_word: String = word
                        .chars()
                        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                        .collect();
                    if !clean_word.is_empty()
                        && !filler_words.contains(&clean_word.to_lowercase().as_str())
                    {
                        name = clean_word;
                        break;
                    }
                }
                if !name.is_empty() {
                    Some(name)
                } else {
                    None
                }
            } else {
                None
            }
        });

        let folder_prefix_found = folder_prefixes.iter().any(|p| lower_msg.contains(p));
        if let Some(folder_name) = folder_name_opt {
            agent::emit_event(
                &app_handle,
                0,
                &format!("[FAST-TRACK] Creando carpeta: {}", folder_name),
                "ACTION",
            );
            let mkdir_cmd = format!("mkdir \"{}\"", folder_name);
            match crate::core::execute_terminal_command(&workspace_path, &mkdir_cmd).await {
                Ok(_) => {
                    agent::emit_event(
                        &app_handle,
                        1,
                        &format!("Carpeta '{}' creada exitosamente.", folder_name),
                        "SUCCESS",
                    );
                    let resp_msg = format!(
                        "✅ Listo. Carpeta `{}` creada en tu workspace.",
                        folder_name
                    );
                    journal
                        .chat_history
                        .push(format!("Usuario: {}", user_message));
                    journal.chat_history.push(format!("Aura: {}", resp_msg));
                    if journal.chat_history.len() > 6 {
                        journal
                            .chat_history
                            .drain(0..journal.chat_history.len() - 6);
                    }
                    let _ = crate::core::session_journal::save_journal(&workspace_path, &journal);
                    let response = serde_json::json!({"status": "FINISH", "respuesta_conversacional": resp_msg});
                    return Ok(response.to_string());
                }
                Err(e) => {
                    let resp_msg = if e.contains("ya existe")
                        || e.contains("already exists")
                        || e.contains("MKDIR")
                    {
                        format!("ℹ️ La carpeta `{}` ya existe en tu workspace.", folder_name)
                    } else {
                        format!(
                            "⚠️ No pude crear la carpeta `{}`. Error: {}",
                            folder_name, e
                        )
                    };
                    let response = serde_json::json!({"status": "FINISH", "respuesta_conversacional": resp_msg});
                    return Ok(response.to_string());
                }
            }
        } else if folder_prefix_found {
            // Prefix was detected but name was a filler word (e.g. "crea la carpeta que te pedí")
            let resp_msg = "¿Con qué nombre quieres que cree la carpeta? Dime el nombre exacto y la creo al instante.";
            let response =
                serde_json::json!({"status": "FINISH", "respuesta_conversacional": resp_msg});
            return Ok(response.to_string());
        }

        // ── Hardcoded keyword intercept (faster than NLU for known search verbs) ──
        let search_keywords = [
            "investiga",
            "investigue",
            "investig",
            "busca",
            "buscar",
            "busque",
            "consulta",
            "consulte",
        ];
        let first_token = lower_msg
            .split(|c: char| !c.is_alphanumeric())
            .find(|token| !token.is_empty())
            .unwrap_or("");
        let forced_search = search_keywords.iter().any(|kw| first_token.starts_with(kw));

        if forced_search && active_mission_context.is_none() {
            agent::emit_event(
                &app_handle,
                0,
                "[INTERCEPT] Verbo de búsqueda detectado. Forzando AGENTIC_TASK.",
                "INFO",
            );
            enriched_message = format!("Petición Original del Usuario: {}\n\nGuía de Traducción Técnica: El usuario usó un verbo de búsqueda explícito. DEBES usar TOOL_WEB_SEARCH para investigar en internet y luego usar TOOL_FINISH para responder en el chat.", user_message);

            journal
                .chat_history
                .push(format!("Usuario: {}", user_message));
            let _ = crate::core::session_journal::save_journal(&workspace_path, &journal);
        } else {
            let chat_json = crate::memory::load_chat_history(workspace_path.clone())
                .await
                .unwrap_or_else(|_| "[]".to_string());
            let mut visual_history = Vec::new();
            if let Ok(messages) = serde_json::from_str::<Vec<serde_json::Value>>(&chat_json) {
                for msg in messages.iter().rev().take(8).rev() {
                    let sender = msg
                        .get("sender")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
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
            // FIX #2: Synchronize the journal's chat history before NLU so the Agent Loop inherits it
            journal.chat_history = combined_history.clone();
            let _ = crate::core::session_journal::save_journal(&workspace_path, &journal);

            let approved_plan_request = crate::core::intent_router::recover_approved_plan_request(
                &chat_json,
                &user_message,
            );
            let pending_clarification_request =
                crate::core::intent_router::recover_pending_clarification_request(
                    &chat_json,
                    &user_message,
                );
            let scoped_visual_review =
                crate::core::intent_router::is_scoped_visual_review(&user_message);
            let mut nlu_response = if let Some(request) = approved_plan_request.as_deref() {
                agent::emit_event(
                    &app_handle,
                    0,
                    "[PLAN APROBADO] Recuperé el mandato y la hoja de ruta de esta conversación; continuaré con la implementación local.",
                    "INFO",
                );
                serde_json::json!({
                    "intent_type": "AGENTIC_TASK",
                    "technical_translation": request
                })
                .to_string()
            } else if let Some(request) = pending_clarification_request.as_deref() {
                agent::emit_event(
                    &app_handle,
                    0,
                    "[NLU_RECOVERY] Respuesta vinculada a la aclaración pendiente; se conserva el mandato original completo.",
                    "INFO",
                );
                serde_json::json!({
                    "intent_type": "AGENTIC_TASK",
                    "turn_relation": "FOLLOW_UP",
                    "phase_updates": [],
                    "technical_translation": request,
                    "os_command": null,
                    "direct_response": null,
                    "clarification_question": null
                })
                .to_string()
            } else if scoped_visual_review {
                agent::emit_event(
                    &app_handle,
                    0,
                    "[INTENT DETERMINISTIC] Revisión de interfaz detectada; se conserva el alcance de solo lectura sin pedir aclaraciones.",
                    "INFO",
                );
                serde_json::json!({
                    "intent_type": "AGENTIC_TASK",
                    "turn_relation": if active_mission_context.is_some() { "FOLLOW_UP" } else { "NEW_TASK" },
                    "phase_updates": [],
                    "technical_translation": user_message,
                    "os_command": null,
                    "direct_response": null,
                    "clarification_question": null
                })
                .to_string()
            } else {
                translator::translate_to_technical_intent(
                    &user_message,
                    &app_handle,
                    &combined_history,
                    active_mission_context.as_deref(),
                    &orchestrator_model,
                )
                .await
            };
            nlu_response = nlu_response.trim().to_string();
            println!("[NLU] Input: '{}' -> RAW: '{}'", user_message, nlu_response);

            let nlu_json: serde_json::Value = match crate::core::structured_json::parse_json_object(
                &nlu_response,
            ) {
                Ok(value) => value,
                Err(error) => {
                    agent::emit_event(
                            &app_handle,
                            0,
                            &format!(
                                "[NLU_JSON_INVALID] Respuesta estructurada inválida: {}. Se conserva literalmente la solicitud del usuario; no se usan argumentos generados por el NLU.",
                                error
                            ),
                            "WARNING",
                        );
                    serde_json::json!({
                        "intent_type": "AGENTIC_TASK",
                        "turn_relation": crate::core::intent_router::fallback_turn_relation(
                            &user_message,
                            active_mission_context.as_deref(),
                        ),
                        "phase_updates": [],
                        "technical_translation": user_message,
                        "os_command": null,
                        "direct_response": null,
                        "clarification_question": null
                    })
                }
            };

            let mut intent_type = nlu_json
                .get("intent_type")
                .and_then(|v| v.as_str())
                .unwrap_or("AGENTIC_TASK");
            let turn_relation = nlu_json
                .get("turn_relation")
                .and_then(|value| value.as_str())
                .unwrap_or_else(|| {
                    crate::core::intent_router::fallback_turn_relation(
                        &user_message,
                        active_mission_context.as_deref(),
                    )
                });
            let is_contextual_follow_up = pending_clarification_request.is_none()
                && active_mission_context.is_some()
                && matches!(
                    turn_relation,
                    "FOLLOW_UP" | "PLAN_CHANGE" | "ANSWER_TO_QUESTION"
                );
            if turn_relation == "PLAN_CHANGE" && active_mission_context.is_some() {
                let updated_phases =
                    crate::core::intent_router::apply_phase_plan_updates(&mut journal, &nlu_json);
                if updated_phases > 0 {
                    if let Err(error) =
                        crate::core::session_journal::save_journal(&workspace_path, &journal)
                    {
                        agent::emit_event(
                            &app_handle,
                            0,
                            &format!("[PLAN CHANGE] No se pudo guardar el cambio: {}", error),
                            "FATAL",
                        );
                        return Ok(serde_json::json!({
                            "status": "ERROR",
                            "respuesta_conversacional": format!("No pude guardar el cambio solicitado al plan: {}", error)
                        }).to_string());
                    }
                    active_mission_context =
                        crate::core::intent_router::active_mission_context(&journal);
                    agent::emit_event(
                        &app_handle,
                        0,
                        &format!(
                            "[PLAN CHANGE] {} fase(s) actualizadas; se conserva el diario y el resto del plan.",
                            updated_phases
                        ),
                        "SUCCESS",
                    );
                } else {
                    agent::emit_event(
                        &app_handle,
                        0,
                        "[PLAN CHANGE] No se pudo identificar con confianza una fase concreta; se conserva el plan y se continúa con la instrucción para evitar cambios accidentales.",
                        "WARNING",
                    );
                }
            }
            if is_contextual_follow_up {
                intent_type = "AGENTIC_TASK";
            }

            let effective_user_request = approved_plan_request
                .as_deref()
                .or(pending_clarification_request.as_deref())
                .unwrap_or(&user_message);
            let lower_msg = effective_user_request.to_lowercase();
            let initial_plan_request = agent::is_initial_plan_request(effective_user_request);
            if initial_plan_request {
                if intent_type != "AGENTIC_TASK" {
                    agent::emit_event(
                        &app_handle,
                        0,
                        "[NLU] Plan inicial reconocido; se preparará una hoja de ruta antes de programar.",
                        "INFO",
                    );
                }
                intent_type = "AGENTIC_TASK";
            } else if lower_msg.contains("tool_")
                || lower_msg.contains("script")
                || lower_msg.contains("reto")
                || lower_msg.contains("algoritmo")
                || lower_msg.contains("crea")
                || lower_msg.contains("procede")
                || lower_msg.contains("ejecuta")
                || lower_msg.contains("proyecto")
                || lower_msg.contains("backend")
                || lower_msg.contains("frontend")
                || lower_msg.contains("programa")
                || lower_msg.contains("haz")
                || lower_msg.contains("prueba")
                || lower_msg.contains("continua")
            {
                intent_type = "AGENTIC_TASK";
            }
            if turn_relation == "NEEDS_CLARIFICATION" {
                intent_type = "NEEDS_CLARIFICATION";
            }

            if intent_type == "CONVERSATION" {
                let direct_response = nlu_json
                    .get("direct_response")
                    .and_then(|v| v.as_str())
                    .unwrap_or("¡Hola! ¿En qué puedo ayudarte?");
                agent::emit_event(&app_handle, 0, "Conversación fluida detectada.", "SUCCESS");

                // Save to memory
                journal
                    .chat_history
                    .push(format!("Usuario: {}", user_message));
                journal
                    .chat_history
                    .push(format!("Aura: {}", direct_response));
                if journal.chat_history.len() > 6 {
                    journal
                        .chat_history
                        .drain(0..journal.chat_history.len() - 6);
                }
                let _ = crate::core::session_journal::save_journal(&workspace_path, &journal);

                let response = serde_json::json!({
                    "status": "FINISH",
                    "respuesta_conversacional": direct_response
                });
                return Ok(response.to_string());
            }

            // ── NEEDS_CLARIFICATION: El agente pregunta antes de actuar ──────────
            if intent_type == "NEEDS_CLARIFICATION" {
                let question = nlu_json.get("clarification_question")
                .and_then(|v| v.as_str())
                .unwrap_or("¿Puedes darme más detalles sobre lo que necesitas? Quiero asegurarme de entenderte bien antes de empezar.");
                agent::emit_event(
                    &app_handle,
                    0,
                    "[NLU] Mandato ambiguo — solicitando clarificación al usuario.",
                    "WARNING",
                );

                journal
                    .chat_history
                    .push(format!("Usuario: {}", user_message));
                journal.chat_history.push(format!(
                    "Aura: {} {}",
                    crate::core::intent_router::pending_clarification_marker(),
                    question
                ));
                if journal.chat_history.len() > 6 {
                    journal
                        .chat_history
                        .drain(0..journal.chat_history.len() - 6);
                }
                let _ = crate::core::session_journal::save_journal(&workspace_path, &journal);

                let response = serde_json::json!({
                    "status": "FINISH",
                    "respuesta_conversacional": question
                });
                return Ok(response.to_string());
            }

            if intent_type == "FAST_TRACK_OS" {
                if let Some(cmd) = nlu_json.get("os_command").and_then(|v| v.as_str()) {
                    if !cmd.is_empty() && cmd != "null" {
                        agent::emit_event(
                            &app_handle,
                            0,
                            &format!("[FAST-TRACK] Ejecutando: {}", cmd),
                            "ACTION",
                        );
                        match crate::core::execute_terminal_command(&workspace_path, cmd).await {
                            Ok(_) => {
                                agent::emit_event(
                                    &app_handle,
                                    0,
                                    "[FAST-TRACK] Comando ejecutado con éxito.",
                                    "SUCCESS",
                                );
                                let resp_msg = format!("✅ Listo. Ejecuté: `{}`", cmd);

                                journal
                                    .chat_history
                                    .push(format!("Usuario: {}", user_message));
                                journal.chat_history.push(format!("Aura: {}", resp_msg));
                                if journal.chat_history.len() > 6 {
                                    journal
                                        .chat_history
                                        .drain(0..journal.chat_history.len() - 6);
                                }
                                let _ = crate::core::session_journal::save_journal(
                                    &workspace_path,
                                    &journal,
                                );

                                let response = serde_json::json!({
                                    "status": "FINISH",
                                    "respuesta_conversacional": resp_msg
                                });
                                return Ok(response.to_string());
                            }
                            Err(e) => {
                                agent::emit_event(
                                    &app_handle,
                                    0,
                                    &format!("[FAST-TRACK] Error: {}. Derivando al Agente...", e),
                                    "WARNING",
                                );
                                // Fall through to AGENTIC_TASK
                            }
                        }
                    }
                }
            }

            let technical_intent = nlu_json
                .get("technical_translation")
                .and_then(|v| v.as_str())
                .unwrap_or(effective_user_request);
            // Include both the original (for user reference) and the cleaned technical intent
            enriched_message = if is_contextual_follow_up {
                crate::core::intent_router::contextual_follow_up_request(
                    active_mission_context.as_deref().unwrap_or_default(),
                    effective_user_request,
                    technical_intent,
                )
                .unwrap_or_else(|| {
                    format!(
                        "Petición Original del Usuario: {}\n\nGuía de Traducción Técnica (generada por NLU): {}",
                        effective_user_request, technical_intent
                    )
                })
            } else {
                format!(
                    "Petición Original del Usuario: {}\n\nGuía de Traducción Técnica (generada por NLU): {}",
                    effective_user_request, technical_intent
                )
            };
            if initial_plan_request {
                enriched_message.push_str(
                    "\n\n[MODO PLAN INICIAL]: Presenta una hoja de ruta MVP con fases, objetivos, entregables verificables y condiciones de salida; cubre cada módulo concreto que pidió el usuario. Respeta el orden pedido: primero pruebas locales/emuladores y después despliegue Firebase si así se solicitó. No escribas código. No bloquees la fase local por una decisión fiscal futura; confirma el país objetivo solo antes de implementar requisitos fiscales. El país de residencia del usuario no define automáticamente el mercado del proyecto. Usa documentación oficial para los datos actuales de Firebase y termina con solo preguntas que realmente bloqueen la primera fase.",
                );
            }
        } // end else (no keyword intercept)
    }

    if !profile_context.is_empty()
        && !crate::core::intent_router::is_resume_command(&enriched_message)
    {
        enriched_message.push_str("\n\n");
        enriched_message.push_str(&profile_context);
    }

    let workspace_tree_nodes =
        crate::memory::get_workspace_tree_internal(workspace_path.clone()).await?;
    // Filter out noise directories — node_modules alone can be 4000+ nodes and pollutes
    // the LLM context and embedding index with irrelevant framework internals.
    let ignored_dirs = ["node_modules", ".git", "__pycache__", "target"];
    let files_only: Vec<_> = workspace_tree_nodes
        .iter()
        .filter(|n| !n.is_dir && !ignored_dirs.iter().any(|d| n.path.contains(d)))
        .collect();
    let mut index = crate::memory::read_vector_index(&workspace_path).await;
    const MAX_NUEVOS_A_VECTORIZAR: usize = 50;
    if index.len() != files_only.len() {
        let nuevos: Vec<_> = files_only
            .iter()
            .filter(|f| !index.iter().any(|n| n.path == f.path))
            .collect();
        if nuevos.len() <= MAX_NUEVOS_A_VECTORIZAR {
            let mut new_index = Vec::new();
            for file in &files_only {
                if let Some(existing) = index.iter().find(|n| n.path == file.path) {
                    new_index.push(existing.clone());
                } else {
                    if let Ok(emb) = get_embedding(&file.path).await {
                        new_index.push(crate::memory::VectorNode {
                            path: file.path.clone(),
                            embedding: emb,
                        });
                    }
                }
            }
            index = new_index;
            let _ = crate::memory::write_vector_index(&workspace_path, &index).await;
        } else {
            index.clear();
        }
    }
    // No truncar el árbol de archivos con búsqueda semántica. El agente necesita ver el mapa real.
    let tree_json = serde_json::to_string(
        &files_only
            .iter()
            .take(500)
            .map(|n| n.path.clone())
            .collect::<Vec<String>>(),
    )
    .unwrap_or_default();

    agent::run_agent_loop(
        enriched_message,
        workspace_path,
        tree_json,
        orchestrator_model,
        programmer_model,
        app_handle,
    )
    .await
}
