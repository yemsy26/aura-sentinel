use super::call_ollama_text;
use tauri::AppHandle;
use crate::llm::agent::emit_event;

pub async fn translate_to_technical_intent(user_input: &str, app_handle: &AppHandle) -> String {
    let model = "llama3.1:8b";
    let system_prompt = format!(
        "Eres un experto en traducción técnica. Tu única función es tomar el mensaje coloquial del usuario (que puede contener errores, jerga o ser vago) y convertirlo en una directiva de ingeniería clara, técnica y orientada a acciones para un Agente DevSecOps. Devuelve ÚNICAMENTE la directiva traducida.\n\nMensaje del usuario: {}",
        user_input
    );
    
    emit_event(app_handle, 0, "Traduciendo intención del usuario...", "PLANNING");
    
    match call_ollama_text(model, &system_prompt).await {
        Ok(mut res) => {
            res = res.trim().to_string();
            if res.is_empty() {
                emit_event(app_handle, 0, "Traductor devolvió vacío. Usando Fallback.", "WARNING");
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
