use super::call_ollama_text;
use tauri::AppHandle;
use crate::llm::agent::emit_event;

/// NLU unificado: una sola llamada LLM que corrige ortografía Y clasifica intención.
/// Antes eran 2 llamadas (corrector + clasificador) -> ahora es 1 sola llamada.
pub async fn translate_to_technical_intent(user_input: &str, app_handle: &AppHandle, chat_history: &[String]) -> String {
    // Resolver modelos disponibles
    let mut available_models = Vec::new();
    if let Ok(res) = reqwest::Client::new()
        .get("http://localhost:11434/api/tags")
        .send()
        .await
    {
        if let Ok(json) = res.json::<serde_json::Value>().await {
            if let Some(models) = json.get("models").and_then(|m| m.as_array()) {
                for model in models {
                    if let Some(name) = model.get("name").and_then(|n| n.as_str()) {
                        available_models.push(name.to_string());
                    }
                }
            }
        }
    }

    // Usar FastTrack para NLU - bonsai27b está en brains.json como fast_parser
    // esto evita el swap de modelo entre NLU y la fase de orquestación
    let task_ctx = crate::llm::router::TaskContext {
        task_type: crate::llm::router::TaskType::FastTrack,
        language: None,
    };

    let model = crate::llm::router::get_best_model(&task_ctx, &available_models, app_handle, 0).await
        .unwrap_or_else(|_| crate::llm::agent::DEFAULT_ORCHESTRATOR_MODEL.to_string());

    emit_event(app_handle, 0, &format!("🧠 [NLU] Analizando con {}...", model), "PLANNING");

    let context_str = if chat_history.is_empty() {
        "".to_string()
    } else {
        format!("CONTEXTO RECIENTE DE LA CONVERSACIÓN:\n{}\n\n", chat_history.join("\n"))
    };

    // Una sola llamada: el modelo corrige ortografía Y clasifica en el mismo prompt
    let system_prompt = format!(
        "Eres el Analista de Intenciones (NLU) de Aura-Sentinel. \
        PRIMERO corrige errores ortográficos/tipográficos del texto del usuario, \
        LUEGO clasifica la intención en uno de estos tipos, devolviendo UN ÚNICO OBJETO JSON VÁLIDO (sin texto extra):\n\n\
        TIPOS DE INTENCIÓN:\n\
        1. \"CONVERSATION\": El usuario solo está saludando, haciendo charla general o preguntas básicas de conocimiento.\n\
        2. \"FAST_TRACK_OS\": El usuario quiere ejecutar un comando nativo sencillo en terminal (crear carpeta, listar, ping). \
        ⚠️ WINDOWS. Usa: 'mkdir', 'del', 'dir', 'copy', 'move', 'rd /s /q'. PROHIBIDO: rm, touch, ls, cat, sudo.\n\
        3. \"AGENTIC_TASK\": El usuario quiere crear código, modificar archivos, buscar info en tiempo real, analizar sistemas, \
        resolver bugs, verificar que algo funciona, o resolver problemas de lógica/matemática/SAT. SIEMPRE AGENTIC_TASK si dice: 'investiga', 'busca', 'crea', 'verifica', 'demuestra', o presenta un problema de restricciones lógicas.\n\
        4. \"NEEDS_CLARIFICATION\": La petición es demasiado vaga para actuar.\n\n\
        ESTRUCTURA JSON OBLIGATORIA:\n\
        {{\"intent_type\": \"...\", \"technical_translation\": \"<Si es AGENTIC_TASK, escribe la instrucción clara. IMPORTANTE: Si el usuario provee cláusulas matemáticas o arrays (ej. [[1,2,3]...]), CÓPIALOS EXACTAMENTE sin alterarlos>\", \
        \"os_command\": \"<comando Windows exacto si es FAST_TRACK_OS, si no null>\", \
        \"direct_response\": \"<respuesta natural si es CONVERSATION, si no null>\", \
        \"clarification_question\": \"<pregunta si es NEEDS_CLARIFICATION, si no null>\"}}\n\n\
        {}Usuario: {}\n",
        context_str,
        user_input
    );

    match call_ollama_text(&model, &system_prompt).await {
        Ok(mut res) => {
            res = res.trim().to_string();
            // Extraer JSON entre { y }
            let mut clean_text = res.trim().to_string();
            if let Some(start) = clean_text.find('{') {
                if let Some(end) = clean_text.rfind('}') {
                    clean_text = clean_text[start..end + 1].to_string();
                }
            }

            if serde_json::from_str::<serde_json::Value>(&clean_text).is_err() {
                emit_event(app_handle, 0, "NLU JSON inválido. Fallback a AGENTIC_TASK.", "WARNING");
                return format!("{{\"intent_type\":\"AGENTIC_TASK\",\"technical_translation\":\"{}\",\"os_command\":null,\"direct_response\":null,\"clarification_question\":null}}", user_input.replace('"', "\\\""));
            }

            if clean_text.is_empty() {
                emit_event(app_handle, 0, "NLU vacío. Forzando tarea agente.", "WARNING");
                format!("{{\"intent_type\":\"AGENTIC_TASK\",\"technical_translation\":\"{}\",\"os_command\":null,\"direct_response\":null,\"clarification_question\":null}}", user_input.replace('"', "\\\""))
            } else {
                emit_event(app_handle, 0, "✅ Intención clasificada.", "SUCCESS");
                clean_text
            }
        },
        Err(e) => {
            emit_event(app_handle, 0, &format!("NLU Falló: {}. Forzando tarea agente.", e), "ERROR");
            format!("{{\"intent_type\":\"AGENTIC_TASK\",\"technical_translation\":\"{}\",\"os_command\":null,\"direct_response\":null,\"clarification_question\":null}}", user_input.replace('"', "\\\""))
        }
    }
}
