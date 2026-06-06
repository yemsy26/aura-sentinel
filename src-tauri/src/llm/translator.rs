use super::call_ollama_text;
use tauri::AppHandle;
use crate::llm::agent::emit_event;

pub async fn translate_to_technical_intent(user_input: &str, app_handle: &AppHandle) -> String {
    let model = "llama3.1:8b";
    let system_prompt = format!(
        "Traduce el TEXTO coloquial a una orden técnica estricta para el sistema DevSecOps.\n\n\
        TEXTO: Haz un mapa de la estructura o dime qué falta\n\
        TRADUCCION: Ejecutar TOOL_ARCHITECT para auditar la arquitectura.\n\n\
        TEXTO: Crea un servidor en python\n\
        TRADUCCION: Crear servidor en python usando TOOL_PROGRAMMER y ejecutar con TOOL_BACKGROUND_START.\n\n\
        TEXTO: ponle un hola mundo al main\n\
        TRADUCCION: Modificar main.js agregando 'Hola Mundo' usando TOOL_PROGRAMMER.\n\n\
        TEXTO: {}\n\
        TRADUCCION:",
        user_input
    );
    
    emit_event(app_handle, 0, &format!("Traduciendo intención con {}...", model), "PLANNING");
    
    match call_ollama_text(model, &system_prompt).await {
        Ok(mut res) => {
            res = res.trim().to_string();
            if res.is_empty() || res.contains("No puedo") || res.contains("I cannot") {
                emit_event(app_handle, 0, "Fallo cognitivo en Traductor. Usando Fallback.", "WARNING");
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
