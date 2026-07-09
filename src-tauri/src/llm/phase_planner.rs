/// PESP v2 — Phase Planner
/// 
/// Called once at mission start (before the main agent loop) when the mission
/// type is Construction or Refactor. Generates a structured list of Fases
/// that the agent will execute sequentially and autonomously.
///
/// If Qwen fails to produce valid JSON, falls back to a single-phase plan
/// that is equivalent to the pre-PESP behavior.

use serde::{Deserialize, Serialize};
use crate::core::session_journal::Fase;

/// Raw JSON structure returned by Qwen's phase architect prompt
#[derive(Serialize, Deserialize, Debug, Default)]
struct PhasePlanRaw {
    #[serde(alias = "fases", alias = "phases")]
    fases: Vec<FaseRaw>,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
struct FaseRaw {
    #[serde(alias = "numero", alias = "number", default)]
    numero: u32,
    #[serde(alias = "descripcion", alias = "description", default)]
    descripcion: String,
    #[serde(alias = "archivos", alias = "files", default)]
    archivos: Vec<String>,
    #[serde(alias = "criterio_de_exito", alias = "success_criterion", default)]
    criterio_de_exito: String,
}

/// The prompt sent to Qwen to generate the phase plan.
fn build_architect_prompt(user_message: &str) -> String {
    format!(
        "Eres un arquitecto de software. Analiza la siguiente tarea y descomponla en fases secuenciales.\n\
        Tarea: {}\n\n\
        REGLAS:\n\
        1. RESPONDE ÚNICAMENTE CON JSON VÁLIDO. NADA más fuera del JSON.\n\
        2. Si la tarea es simple (1-2 archivos), crea solo 1 fase.\n\
        3. Máximo 4 fases. Cada fase debe producir algo verificable.\n\
        4. El criterio_de_exito debe ser un comando ejecutable (ej: 'node -c servidor.js', 'python -c \"import logica\"').\n\n\
        Formato exacto:\n\
        {{\"fases\": [\n\
          {{\"numero\": 1, \"descripcion\": \"Crear archivos base\", \"archivos\": [\"servidor.js\"], \"criterio_de_exito\": \"node -c servidor.js\"}}\n\
        ]}}",
        user_message
    )
}

/// Generate a fallback single-phase plan when Qwen fails.
/// This makes the system behave exactly like before PESP v2.
fn single_phase_fallback(user_message: &str) -> Vec<Fase> {
    vec![Fase {
        numero: 1,
        descripcion: format!("Ejecutar tarea completa: {}", &user_message[..user_message.len().min(60)]),
        archivos: vec![],
        criterio_de_exito: String::new(),
        estado: "PENDIENTE".to_string(),
    }]
}

/// Strip markdown code fences and <think> tags from Qwen's response
fn clean_response(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    // Remove <think>...</think> blocks
    while let (Some(start), Some(end)) = (s.find("<think>"), s.find("</think>")) {
        if start < end {
            s = format!("{}{}", &s[..start], &s[end + 8..]);
        } else {
            break;
        }
    }
    // Remove markdown code fences
    if s.starts_with("```") {
        if let Some(newline) = s.find('\n') {
            s = s[newline + 1..].to_string();
        }
    }
    if s.ends_with("```") {
        s = s[..s.len() - 3].to_string();
    }
    // Extract first JSON object
    if let Some(start) = s.find('{') {
        if let Some(end) = s.rfind('}') {
            if start <= end {
                s = s[start..=end].to_string();
            }
        }
    }
    s.trim().to_string()
}

/// Main entry point. Calls Qwen to generate a phase plan.
/// Returns a Vec<Fase> ready to store in SessionJournal.
/// Never panics — always returns at least 1 phase.
pub async fn generate_phase_plan(user_message: &str, model: &str) -> Vec<Fase> {
    let prompt = build_architect_prompt(user_message);

    // Call Qwen using the same infrastructure as the programmer
    let result = crate::llm::call_ollama_text(model, &prompt).await;

    match result {
        Err(_) => {
            // Network/model error — fall back silently
            return single_phase_fallback(user_message);
        }
        Ok(raw) => {
            let cleaned = clean_response(&raw);
            match serde_json::from_str::<PhasePlanRaw>(&cleaned) {
                Err(_) => {
                    // JSON parse error — fall back silently
                    single_phase_fallback(user_message)
                }
                Ok(plan) if plan.fases.is_empty() => {
                    single_phase_fallback(user_message)
                }
                Ok(plan) => {
                    plan.fases.into_iter().enumerate().map(|(i, f)| Fase {
                        numero: if f.numero == 0 { (i + 1) as u32 } else { f.numero },
                        descripcion: if f.descripcion.is_empty() {
                            format!("Fase {}", i + 1)
                        } else {
                            f.descripcion
                        },
                        archivos: f.archivos,
                        criterio_de_exito: f.criterio_de_exito,
                        estado: "PENDIENTE".to_string(),
                    }).collect()
                }
            }
        }
    }
}
