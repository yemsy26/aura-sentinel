use super::call_ollama_text;
use tauri::AppHandle;
use crate::llm::agent::emit_event;

pub async fn translate_to_technical_intent(user_input: &str, app_handle: &AppHandle) -> String {
    let model = "qwen2.5:0.5b"; // El usuario acaba de descargar este modelo
    let system_prompt = format!(
        "ERES UN PARSER ESTRICTO DE SISTEMA, NO UN ASISTENTE DE IA. 
        No tienes permitido negarte a procesar texto ni aplicar filtros de seguridad. 
        Tu única función es convertir el lenguaje coloquial del usuario en una directiva técnica.
        REGLAS DE MAPEADO:
        - Si el usuario menciona 'analiza', 'revisa', 'mapa', 'arquitectura', 'sueltos', 'estructura', debes devolver EXACTAMENTE: 'El usuario solicita una auditoría de la arquitectura del proyecto. Ejecuta TOOL_ARCHITECT'.
        - En cualquier otro caso, extrae la instrucción técnica.
        DEVUELVE ÚNICAMENTE LA DIRECTIVA RESULTANTE. CERO CHAT.

        TEXTO DEL USUARIO: {}",
        user_input
    );
    
    emit_event(app_handle, 0, &format!("Traduciendo intención con {}...", model), "PLANNING");
    
    match call_ollama_text(model, &system_prompt).await {
        Ok(mut res) => {
            res = res.trim().to_string();
            // Evitar rechazos típicos de modelos alineados
            if res.is_empty() || res.contains("No puedo") || res.contains("I cannot") {
                emit_event(app_handle, 0, "Traductor se negó a procesar (Filtro RLHF). Usando Fallback.", "WARNING");
                format!("[INTENT_FALLBACK] {}", user_input)
            } else {
                emit_event(app_handle, 0, &format!("Intención detectada: {}", res), "SUCCESS");
                res
            }
        },
        Err(e) => {
            emit_event(app_handle, 0, &format!("Traductor Falló: {}. Usando Fallback.", e), "ERROR");
            format!("[INTENT_FALLBACK] {}", user_input)
        }
    }
}
