use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tokio::process::Command;
use tauri::AppHandle;

#[derive(Debug, Clone, PartialEq)]
pub enum TaskType {
    FastTrack,
    GeneralCode,
    HighComplexityFix,
    Orchestrator,
}

#[derive(Debug, Clone)]
pub struct TaskContext {
    pub task_type: TaskType,
    pub language: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct BrainConfig {
    pub orchestrator: Vec<String>,
    pub fast_parser: Vec<String>,
    pub languages: HashMap<String, Vec<String>>,
    pub debugger: Vec<String>,
}

pub async fn get_best_model(
    context: &TaskContext,
    available_models: &[String],
    app_handle: &AppHandle,
    step: u32,
) -> Result<String, String> {
    let mut config_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    config_path.push("brains.json");

    let config_data = fs::read_to_string(&config_path)
        .unwrap_or_else(|_| include_str!("../../brains.json").to_string());

    let config: BrainConfig = serde_json::from_str(&config_data)
        .map_err(|e| format!("Error parseando brains.json: {}", e))?;

    let preferred_models = match context.task_type {
        TaskType::FastTrack => config.fast_parser.clone(),
        TaskType::Orchestrator => config.orchestrator.clone(),
        TaskType::HighComplexityFix => config.debugger.clone(),
        TaskType::GeneralCode => {
            if let Some(lang) = &context.language {
                let lang_lower = lang.to_lowercase();
                if let Some(models) = config.languages.get(&lang_lower) {
                    models.clone()
                } else {
                    config.languages.get("default").cloned().unwrap_or_default()
                }
            } else {
                config.languages.get("default").cloned().unwrap_or_default()
            }
        }
    };

    // Auto-descargar el primer modelo de preferencia si no existe
    if let Some(first_choice) = preferred_models.first() {
        if !available_models.iter().any(|m| m.starts_with(first_choice)) {
            crate::llm::agent::emit_event(
                app_handle,
                step,
                &format!("Modelo preferido '{}' no encontrado. Descargando automáticamente (esto puede tomar varios minutos)...", first_choice),
                "WARNING"
            );
            
            let status = Command::new("ollama")
                .args(["pull", first_choice])
                .status()
                .await;

            match status {
                Ok(s) if s.success() => {
                    crate::llm::agent::emit_event(
                        app_handle,
                        step,
                        &format!("Modelo '{}' descargado exitosamente.", first_choice),
                        "SUCCESS"
                    );
                    return Ok(first_choice.clone());
                }
                _ => {
                    crate::llm::agent::emit_event(
                        app_handle,
                        step,
                        &format!("Fallo al descargar el modelo '{}'. Haciendo fallback...", first_choice),
                        "ERROR"
                    );
                }
            }
        }
    }

    // Si la descarga falló o el primer modelo ya existía, buscar en los modelos disponibles locales
    let has_model = |prefix: &str| -> Option<String> {
        available_models.iter().find(|m| m.starts_with(prefix)).cloned()
    };

    for model in preferred_models {
        if let Some(m) = has_model(&model) {
            return Ok(m);
        }
    }

    Err("No hay modelos compatibles disponibles en brains.json para ejecutar esta tarea.".to_string())
}
