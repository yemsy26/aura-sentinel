use super::call_ollama_with_schema_options;
use crate::llm::agent::emit_event;
use tauri::AppHandle;

fn technical_intent_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "intent_type": {
                "type": "string",
                "enum": ["CONVERSATION", "FAST_TRACK_OS", "AGENTIC_TASK", "NEEDS_CLARIFICATION"]
            },
            "turn_relation": {
                "type": "string",
                "enum": ["NEW_TASK", "FOLLOW_UP", "PLAN_CHANGE", "ANSWER_TO_QUESTION", "NEEDS_CLARIFICATION"]
            },
            "phase_updates": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "numero": { "type": "integer" },
                        "descripcion": { "type": "string" },
                        "archivos": { "type": "array", "items": { "type": "string" } },
                        "criterio_de_exito": { "type": "string" }
                    },
                    "required": ["numero", "descripcion", "archivos", "criterio_de_exito"],
                    "additionalProperties": false
                }
            },
            "technical_translation": { "type": "string" },
            "os_command": { "type": ["string", "null"] },
            "direct_response": { "type": ["string", "null"] },
            "clarification_question": { "type": ["string", "null"] }
        },
        "required": [
            "intent_type", "turn_relation", "phase_updates", "technical_translation",
            "os_command", "direct_response", "clarification_question"
        ],
        "additionalProperties": false
    })
}

fn fallback_intent(user_input: &str, mission_context: Option<&str>) -> String {
    serde_json::json!({
        "intent_type": "AGENTIC_TASK",
        "turn_relation": if mission_context.is_some() { "FOLLOW_UP" } else { "NEW_TASK" },
        "phase_updates": [],
        "technical_translation": user_input,
        "os_command": null,
        "direct_response": null,
        "clarification_question": null
    })
    .to_string()
}

/// NLU unificado: una sola llamada LLM que corrige ortografía Y clasifica intención.
/// Usa el modelo seleccionado por el usuario para coherencia global.
pub async fn translate_to_technical_intent(
    user_input: &str,
    app_handle: &AppHandle,
    chat_history: &[String],
    mission_context: Option<&str>,
    requested_model: &str,
) -> String {
    // Resolver modelos disponibles
    let mut available_models = Vec::new();
    if let Ok(res) = reqwest::Client::new()
        .get("http://127.0.0.1:11434/api/tags")
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

    let model = crate::llm::agent::resolve_model_or_fallback(requested_model, &available_models);

    emit_event(
        app_handle,
        0,
        &format!("🧠 [NLU] Analizando con {}...", model),
        "PLANNING",
    );

    let context_str = if chat_history.is_empty() {
        "".to_string()
    } else {
        format!(
            "CONTEXTO RECIENTE DE LA CONVERSACIÓN:\n{}\n\n",
            chat_history.join("\n")
        )
    };
    let mission_state = mission_context
        .map(|context| format!("CONTEXTO ESTRUCTURADO DE MISIÓN ACTIVA:\n{}\n\n", context))
        .unwrap_or_default();
    let context_str = format!("{context_str}{mission_state}");

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
        resolver bugs, verificar que algo funciona, o resolver problemas de lógica/matemática/SAT. SIEMPRE AGENTIC_TASK si dice: 'investiga', 'busca', 'crea', 'construir', 'desarrollar', 'verifica', 'demuestra', o presenta un problema de restricciones lógicas.\n\
        4. \"NEEDS_CLARIFICATION\": Úsalo solo si no hay un objetivo accionable. Una idea amplia marcada como 'plan inicial' sí es accionable: clasifícala como AGENTIC_TASK para que el sistema proponga fases, supuestos y decisiones pendientes. No la rechaces con una pregunta genérica; reserva las preguntas para requisitos que bloquean una fase concreta.\n\n\
        RELACIÓN CON LA MISIÓN: Si recibes contexto de misión activa, clasifica como NEW_TASK, FOLLOW_UP, PLAN_CHANGE, ANSWER_TO_QUESTION o NEEDS_CLARIFICATION. Interpreta frases cortas, pronombres como 'eso' y errores de escritura usando el objetivo, la fase actual y la conversación reciente. Incorpora respuestas claras a preguntas abiertas. Si modifica el trabajo pendiente, conserva objetivo y plan y tradúcelo como AGENTIC_TASK relacionado con la misión. Si pide cambiar el plan, genera phase_updates solo para fases existentes que puedas identificar con confianza; devuelve el número y la descripción, archivos y criterio completos actualizados. No alteres otras fases ni inventes requisitos. Si no puedes identificar la fase, devuelve una lista vacía y explica la ambigüedad. Pide aclaración solo si hay interpretaciones materialmente distintas o falta un dato que bloquea la acción actual. Si inicia claramente otro objetivo, usa NEW_TASK. Sin misión activa, usa NEW_TASK. Una respuesta breve a una pregunta de Aura no es charla general.\n\
        ESTRUCTURA JSON OBLIGATORIA:\n\
        {{\"intent_type\": \"...\", \"turn_relation\": \"NEW_TASK|FOLLOW_UP|PLAN_CHANGE|ANSWER_TO_QUESTION|NEEDS_CLARIFICATION\", \"phase_updates\": [{{\"numero\": 1, \"descripcion\": \"...\", \"archivos\": [\"...\"], \"criterio_de_exito\": \"...\"}}], \"technical_translation\": \"<Si es AGENTIC_TASK, escribe la instrucción clara. IMPORTANTE: Si el usuario provee cláusulas matemáticas o arrays (ej. [[1,2,3]...]), CÓPIALOS EXACTAMENTE sin alterarlos>\", \
        \"os_command\": \"<comando Windows exacto si es FAST_TRACK_OS, si no null>\", \
        \"direct_response\": \"<respuesta natural si es CONVERSATION, si no null>\", \
        \"clarification_question\": \"<pregunta si es NEEDS_CLARIFICATION, si no null>\"}}\n\n\
        {}Usuario: {}\n",
        context_str,
        user_input
    );

    match call_ollama_with_schema_options(
        &model,
        &system_prompt,
        technical_intent_schema(),
        1024,
        0.0,
    )
    .await
    {
        Ok(res) => {
            if crate::core::structured_json::parse_json_object(&res).is_err() {
                emit_event(
                    app_handle,
                    0,
                    "[NLU_JSON_INVALID] Ollama devolvió una salida que no cumple el esquema; se conserva el mandato original.",
                    "WARNING",
                );
                return fallback_intent(user_input, mission_context);
            }
            emit_event(
                app_handle,
                0,
                "✅ Intención clasificada con esquema JSON.",
                "SUCCESS",
            );
            res
        }
        Err(e) => {
            emit_event(
                app_handle,
                0,
                &format!("[NLU_FALLBACK] No se pudo consultar al NLU: {}. Se usará literalmente el mandato original.", e),
                "WARNING",
            );
            fallback_intent(user_input, mission_context)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{fallback_intent, technical_intent_schema};

    #[test]
    fn fallback_serializes_arbitrary_user_text_without_corrupting_json() {
        let input = "revisa la pagina\ncon título \"Aura\" y ruta C:\\web";
        let value: serde_json::Value = serde_json::from_str(&fallback_intent(input, None)).unwrap();
        assert_eq!(value["technical_translation"], input);
        assert_eq!(value["turn_relation"], "NEW_TASK");
    }

    #[test]
    fn intent_schema_requires_a_closed_and_typed_envelope() {
        let schema = technical_intent_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("technical_translation")));
    }
}
