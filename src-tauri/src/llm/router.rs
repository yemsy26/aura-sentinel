use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tokio::process::Command;
use tauri::AppHandle;

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum TaskType {
    FastTrack,
    GeneralCode,
    HighComplexityFix,
    Orchestrator,
}

impl TaskType {
    fn as_str(&self) -> &'static str {
        match self {
            TaskType::FastTrack => "FastTrack",
            TaskType::GeneralCode => "GeneralCode",
            TaskType::HighComplexityFix => "HighComplexityFix",
            TaskType::Orchestrator => "Orchestrator",
        }
    }
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

// Sprint 2: Model Telemetry
// Tracks per-model performance so the router can make data-driven decisions.

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ModelStats {
    pub total_uses: u32,
    pub success_count: u32,
    pub total_steps: u32,
}

impl ModelStats {
    pub fn success_rate(&self) -> f32 {
        if self.total_uses == 0 { return 0.5; }
        self.success_count as f32 / self.total_uses as f32
    }
}

type StatsMap = HashMap<String, ModelStats>;

fn get_stats_path() -> PathBuf {
    let mut p = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if p.ends_with("src-tauri") {
        p = p.parent().unwrap_or(&p).to_path_buf();
    }
    p.join("data").join("model_stats.json")
}

fn load_stats() -> StatsMap {
    let path = get_stats_path();
    if let Ok(raw) = fs::read_to_string(&path) {
        if let Ok(map) = serde_json::from_str(&raw) {
            return map;
        }
    }
    HashMap::new()
}

fn save_stats(map: &StatsMap) {
    let path = get_stats_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(map) {
        let _ = fs::write(&path, json);
    }
}

/// Llamar al final de cada mision para registrar el rendimiento del modelo.
/// success=true si TOOL_FINISH fue alcanzado, false si ERROR/timeout.
pub fn record_model_result(model: &str, task_type: &TaskType, success: bool, steps: u32) {
    let mut stats = load_stats();
    let key = format!("{}::{}", model, task_type.as_str());
    let entry = stats.entry(key).or_default();
    entry.total_uses += 1;
    entry.total_steps += steps;
    if success { entry.success_count += 1; }
    save_stats(&stats);
}

pub async fn get_best_model(
    context: &TaskContext,
    available_models: &[String],
    app_handle: &AppHandle,
    step: u32,
) -> Result<String, String> {
    // 100% force embedded config to avoid workspace drift
    let mut config_data = include_str!("../../brains.json").to_string();
    if config_data.starts_with('\u{feff}') {
        config_data = config_data.trim_start_matches('\u{feff}').to_string();
    }

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

    // Sprint 2: ordenar candidatos por telemetria cuando hay suficientes muestras
    let stats = load_stats();
    let mut ranked: Vec<String> = preferred_models.clone();
    ranked.sort_by(|a, b| {
        let key_a = format!("{}::{}", a, context.task_type.as_str());
        let key_b = format!("{}::{}", b, context.task_type.as_str());
        let stats_a = stats.get(&key_a);
        let stats_b = stats.get(&key_b);
        let use_stats = stats_a.map(|s| s.total_uses >= 5).unwrap_or(false)
            && stats_b.map(|s| s.total_uses >= 5).unwrap_or(false);
        if use_stats {
            let rate_b = stats_b.map(|s| s.success_rate()).unwrap_or(0.5);
            let rate_a = stats_a.map(|s| s.success_rate()).unwrap_or(0.5);
            rate_b.partial_cmp(&rate_a).unwrap_or(std::cmp::Ordering::Equal)
        } else {
            std::cmp::Ordering::Equal
        }
    });

    if let Some(first_choice) = ranked.first() {
        if !available_models.iter().any(|m| m.starts_with(first_choice)) {
            crate::llm::agent::emit_event(app_handle, step,
                &format!("Modelo preferido '{}' no encontrado. Descargando automaticamente...", first_choice),
                "WARNING");
            let status = Command::new("ollama").args(["pull", first_choice]).status().await;
            match status {
                Ok(s) if s.success() => {
                    crate::llm::agent::emit_event(app_handle, step,
                        &format!("Modelo '{}' descargado exitosamente.", first_choice), "SUCCESS");
                    return Ok(first_choice.clone());
                }
                _ => {
                    crate::llm::agent::emit_event(app_handle, step,
                        &format!("Fallo al descargar '{}'. Haciendo fallback...", first_choice), "ERROR");
                }
            }
        }
    }

    let has_model = |prefix: &str| -> Option<String> {
        available_models.iter().find(|m| m.starts_with(prefix)).cloned()
    };
    for model in ranked {
        if let Some(m) = has_model(&model) { return Ok(m); }
    }
    Err("No hay modelos compatibles disponibles en brains.json para ejecutar esta tarea.".to_string())
}
