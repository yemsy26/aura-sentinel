#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use sysinfo::System;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    pub status: SystemStatus,
    pub components: ComponentHealth,
    pub metrics: SystemMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SystemStatus {
    Healthy,
    Degraded,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentHealth {
    pub ollama_available: bool,
    pub workspace_accessible: bool,
    pub memory_adequate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMetrics {
    pub memory_usage_mb: f64,
    pub memory_total_mb: f64,
    pub memory_usage_percent: f64,
}

pub async fn perform_health_check(workspace: &str) -> HealthCheck {
    let mut sys = System::new_all();
    sys.refresh_all();

    // Verificación asíncrona no bloqueante de Ollama vía reqwest (timeout 2s, sin invocar curl en shell)
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap_or_default();
    let ollama_available = client.get("http://localhost:11434/api/tags")
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);

    let workspace_accessible = std::path::Path::new(workspace).exists();

    let total_mem = sys.total_memory() as f64 / (1024.0 * 1024.0);
    let used_mem = sys.used_memory() as f64 / (1024.0 * 1024.0);
    let mem_percent = if total_mem > 0.0 { (used_mem / total_mem) * 100.0 } else { 0.0 };
    let memory_adequate = mem_percent < 90.0;

    let status = if !ollama_available || !workspace_accessible {
        SystemStatus::Critical
    } else if !memory_adequate {
        SystemStatus::Degraded
    } else {
        SystemStatus::Healthy
    };

    HealthCheck {
        status,
        components: ComponentHealth {
            ollama_available,
            workspace_accessible,
            memory_adequate,
        },
        metrics: SystemMetrics {
            memory_usage_mb: used_mem,
            memory_total_mb: total_mem,
            memory_usage_percent: mem_percent,
        },
    }
}
