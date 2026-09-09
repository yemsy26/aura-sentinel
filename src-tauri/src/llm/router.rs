use serde::Deserialize;
use std::collections::HashMap;
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
#[allow(dead_code)]
pub struct TaskContext {
    pub task_type: TaskType,
    pub language: Option<String>,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
pub struct BrainConfig {
    pub orchestrator: Vec<String>,
    pub fast_parser: Vec<String>,
    pub languages: HashMap<String, Vec<String>>,
    pub debugger: Vec<String>,
}

// AL-v1: Per-model telemetry is now unified with core::learning::stats::ModelStats.
// Experience is the sole source of truth for learning and stats derivation.
#[allow(unused_imports)]
pub use crate::core::learning::stats::ModelStats;

/// Legacy helper redirected to AL-v1. Since Experience is the sole source of truth,
/// learning outcomes are recorded via LearningEngine::record_outcome() at mission close.
#[allow(dead_code)]
pub fn record_model_result(_model: &str, _task_type: &TaskType, _success: bool, _steps: u32) {
    // Deprecated: Experience is the sole writer. LearningEngine computes and persists
    // ModelStats deterministically from validated Experience records.
}

#[allow(dead_code)]
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

    let mut ranked: Vec<String> = preferred_models.clone();

    // AL-v1: AdaptiveRouter re-ranks candidates using learned Experience + Stats.
    // Cold-start → brains.json order unchanged (confidence = 0.5, no reorder).
    // AdaptiveRouter cannot execute tools — it only produces a Recommendation.
    {
        use crate::core::learning::{AdaptiveRouter, FingerprintBuilder};
        use crate::core::learning::persistence::LearningPersistence;
        use crate::core::mission_contract::MissionContract;
        use crate::core::project_profile::ProjectProfile;
        use std::sync::Arc;

        // Build a minimal fingerprint from context (no LLM, no I/O errors)
        let dummy_contract = MissionContract::new(
            &context.language.clone().unwrap_or_default()
        );
        let dummy_profile = ProjectProfile::detect(".");

        let fp = FingerprintBuilder::from_mission(&dummy_contract, &dummy_profile);

        let persistence = LearningPersistence::new();
        let store = Arc::new(tokio::sync::RwLock::new(persistence.load_experiences(500)));
        let model_stats = persistence.load_model_stats();
        let strategy_stats = persistence.load_strategy_stats();

        // mission_id not available here — use task_type as seed for determinism
        let seed = context.task_type.as_str();
        let router = AdaptiveRouter::new(model_stats, strategy_stats, store, seed);
        let rec = router.recommend(&fp, &ranked).await;

        // Only reorder if router has actual experience (not cold-start)
        // Cold-start leaves brains.json order intact
        use crate::core::learning::RecommendationReason;
        if !matches!(rec.reason, RecommendationReason::ColdStart) && rec.confidence > 0.6 {
            // Move recommended model to front if it's in our candidate list
            if let Some(pos) = ranked.iter().position(|m| *m == rec.model) {
                let chosen = ranked.remove(pos);
                ranked.insert(0, chosen);
            }
        }
    }

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
