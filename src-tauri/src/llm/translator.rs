use super::call_ollama_text;

pub async fn translate_to_technical_intent(user_input: &str) -> String {
    let model = "qwen2.5:0.5b";
    let system_prompt = format!(
        "Eres un experto en traducción técnica. Tu única función es tomar el mensaje coloquial del usuario (que puede contener errores, jerga o ser vago) y convertirlo en una directiva de ingeniería clara, técnica y orientada a acciones para un Agente DevSecOps. Devuelve ÚNICAMENTE la directiva traducida.\n\nMensaje del usuario: {}",
        user_input
    );
    
    match call_ollama_text(model, &system_prompt).await {
        Ok(mut res) => {
            res = res.trim().to_string();
            if res.is_empty() {
                format!("[INTENT_FALLBACK] {}", user_input)
            } else {
                res
            }
        },
        Err(_) => format!("[INTENT_FALLBACK] {}", user_input),
    }
}
