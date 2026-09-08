#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuraConfig {
    pub ollama: OllamaConfig,
    pub context: ContextConfig,
    pub security: SecurityConfig,
    pub performance: PerformanceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaConfig {
    pub host: String,
    pub port: u16,
    pub timeout_seconds: u64,
    pub embedding_timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    pub max_chars: usize,
    pub compaction_threshold: f32,
    pub preserve_task_charter: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    pub path_jail_enabled: bool,
    pub allowed_domains: Vec<String>,
    pub max_file_size_mb: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceConfig {
    pub background_task_max_logs: usize,
    pub max_concurrent_tasks: usize,
}

impl Default for AuraConfig {
    fn default() -> Self {
        Self {
            ollama: OllamaConfig {
                host: "localhost".to_string(),
                port: 11434,
                timeout_seconds: 600,
                embedding_timeout_seconds: 30,
            },
            context: ContextConfig {
                max_chars: 6000,
                compaction_threshold: 0.8,
                preserve_task_charter: true,
            },
            security: SecurityConfig {
                path_jail_enabled: true,
                allowed_domains: vec![],
                max_file_size_mb: 50,
            },
            performance: PerformanceConfig {
                background_task_max_logs: 500,
                max_concurrent_tasks: 10,
            },
        }
    }
}

pub fn load_config(workspace: &str) -> AuraConfig {
    let config_path = Path::new(workspace).join("aura_config.json");
    if let Ok(content) = std::fs::read_to_string(&config_path) {
        if let Ok(config) = serde_json::from_str::<AuraConfig>(&content) {
            return config;
        }
    }
    AuraConfig::default()
}

pub fn save_config(workspace: &str, config: &AuraConfig) -> Result<(), String> {
    let config_path = Path::new(workspace).join("aura_config.json");
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Error serializando config: {}", e))?;
    std::fs::write(&config_path, json)
        .map_err(|e| format!("Error guardando config: {}", e))?;
    Ok(())
}
