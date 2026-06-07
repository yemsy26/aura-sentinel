#[derive(Debug, Clone, PartialEq)]
pub enum Complexity {
    Parser,
    GeneralCode,
    HighComplexityFix,
    Orchestrator,
}

/// Devuelve el mejor modelo disponible según la complejidad de la tarea y los modelos instalados.
/// Realiza un fallback al modelo inmediatamente inferior si el recomendado no está disponible.
pub fn get_best_model(task_complexity: &Complexity, available_models: &[String]) -> Result<String, String> {
    // Función auxiliar para buscar si el usuario tiene una variante del modelo (ignorando tags exactos si es necesario,
    // o asumiendo el nombre completo). Ollama lista los modelos con sus tags (ej. "llama3.1:8b").
    let has_model = |prefix: &str| -> Option<String> {
        available_models.iter().find(|m| m.starts_with(prefix)).cloned()
    };

    match task_complexity {
        Complexity::Parser => {
            // Preferencia: qwen2.5:0.5b -> qwen2.5-coder:7b -> llama3.1:8b
            if let Some(m) = has_model("qwen2.5:0.5b") { return Ok(m); }
            if let Some(m) = has_model("qwen2.5-coder:7b") { return Ok(m); }
            if let Some(m) = has_model("llama3.1:8b") { return Ok(m); }
        }
        Complexity::GeneralCode => {
            // Preferencia: deepseek-coder:6.7b -> qwen2.5-coder:7b -> llama3.1:8b
            if let Some(m) = has_model("deepseek-coder:6.7b") { return Ok(m); }
            if let Some(m) = has_model("qwen2.5-coder:7b") { return Ok(m); }
            if let Some(m) = has_model("llama3.1:8b") { return Ok(m); }
        }
        Complexity::HighComplexityFix => {
            // Preferencia: qwen3.5:cloud -> nemotron-3-super:cloud -> gemma4:31b-cloud -> qwen2.5-coder:14b -> deepseek-coder:6.7b -> qwen2.5-coder:7b
            if let Some(m) = has_model("qwen3.5:cloud") { return Ok(m); }
            if let Some(m) = has_model("nemotron-3-super:cloud") { return Ok(m); }
            if let Some(m) = has_model("gemma4:31b-cloud") { return Ok(m); }
            if let Some(m) = has_model("qwen2.5-coder:14b") { return Ok(m); }
            if let Some(m) = has_model("deepseek-coder:6.7b") { return Ok(m); }
            if let Some(m) = has_model("qwen2.5-coder:7b") { return Ok(m); }
        }
        Complexity::Orchestrator => {
            // Preferencia: llama3.1:8b -> qwen2.5-coder:7b
            if let Some(m) = has_model("llama3.1:8b") { return Ok(m); }
            if let Some(m) = has_model("qwen2.5-coder:7b") { return Ok(m); }
        }
    }

    Err("No hay modelos compatibles disponibles para ejecutar esta tarea.".to_string())
}
