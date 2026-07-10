//! command_trail.rs — Registro estructurado de pasos del agente
//!
//! Proporciona un registro inmutable y serializable de cada paso del agente,
//! permitiendo auditoría, depuración y análisis post-mortem.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::Path;

/// Máximo de pasos guardados en memoria (rotating buffer)
const MAX_TRAIL_STEPS: usize = 200;

/// Archivo de persistencia del trail
const TRAIL_FILE: &str = ".aura_command_trail.json";

/// Un paso individual en el trail del agente
#[derive(Serialize, Deserialize, Debug, Clone)]
#[allow(dead_code)]
pub struct TrailStep {
    pub step: u32,
    pub timestamp: String,
    pub role: String,                    // "Planner" | "Executor" | "Critic"
    pub tool: String,                    // "TOOL_PROGRAMMER", "TOOL_TERMINAL", etc.
    pub comando: String,                 // comando ejecutado (si aplica)
    pub archivos: Vec<String>,           // archivos afectados
    pub resultado: StepResult,           // "Success" | "Error" | "Blocked"
    pub error: Option<String>,           // mensaje de error si falló
    pub duracion_ms: u64,                // tiempo de ejecución
    pub contexto_hash: String,           // hash del contexto relevante
}

/// Resultado de un paso
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
pub enum StepResult {
    Success,
    Error,
    Blocked,
    Skipped,
}

impl std::fmt::Display for StepResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StepResult::Success => write!(f, "Success"),
            StepResult::Error => write!(f, "Error"),
            StepResult::Blocked => write!(f, "Blocked"),
            StepResult::Skipped => write!(f, "Skipped"),
        }
    }
}

/// Trail completo del agente (persistido a disco)
#[derive(Serialize, Deserialize, Debug, Default)]
#[allow(dead_code)]
pub struct CommandTrail {
    pub session_id: String,
    pub objetivo: String,
    pub steps: VecDeque<TrailStep>,
    pub created_at: String,
    pub updated_at: String,
}

/// Genera un ID de sesión único
fn new_session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("{:08x}", nanos)
}

/// Timestamp ISO 8601 UTC
fn now_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// Hash simple del contexto (para detectar cambios)
fn hash_contexto(ctx: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    ctx.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

impl CommandTrail {
    /// Crea un nuevo trail para una misión
    pub fn new(objetivo: &str) -> Self {
        let now = now_iso();
        Self {
            session_id: new_session_id(),
            objetivo: objetivo.to_string(),
            steps: VecDeque::with_capacity(MAX_TRAIL_STEPS),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// Carga trail existente o crea nuevo
    pub fn load_or_new(workspace_path: &str, objetivo: &str) -> Self {
        let path = Path::new(workspace_path).join(TRAIL_FILE);
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(mut trail) = serde_json::from_str::<CommandTrail>(&content) {
                // Asegurar capacidad
                if trail.steps.capacity() < MAX_TRAIL_STEPS {
                    let mut new_steps = VecDeque::with_capacity(MAX_TRAIL_STEPS);
                    new_steps.append(&mut trail.steps);
                    trail.steps = new_steps;
                }
                trail.objetivo = objetivo.to_string();
                trail.updated_at = now_iso();
                return trail;
            }
        }
        Self::new(objetivo)
    }

    /// Guarda el trail a disco
    pub fn save(&self, workspace_path: &str) {
        let path = Path::new(workspace_path).join(TRAIL_FILE);
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, json);
        }
    }

    /// Añade un paso al trail
    pub fn add_step(
        &mut self,
        step: u32,
        role: &str,
        tool: &str,
        comando: &str,
        archivos: Vec<String>,
        resultado: StepResult,
        error: Option<String>,
        duracion_ms: u64,
        contexto: &str,
    ) {
        let step = TrailStep {
            step,
            timestamp: now_iso(),
            role: role.to_string(),
            tool: tool.to_string(),
            comando: comando.to_string(),
            archivos,
            resultado,
            error,
            duracion_ms,
            contexto_hash: hash_contexto(contexto),
        };

        self.steps.push_back(step);
        
        // Mantener tamaño máximo (rotating buffer)
        if self.steps.len() > MAX_TRAIL_STEPS {
            self.steps.pop_front();
        }
        
        self.updated_at = now_iso();
    }

    /// Obtiene los últimos N pasos
    pub fn recent(&self, n: usize) -> Vec<&TrailStep> {
        self.steps.iter().rev().take(n).collect()
    }

    /// Obtiene pasos fallidos
    pub fn failed_steps(&self) -> Vec<&TrailStep> {
        self.steps.iter()
            .filter(|s| s.resultado == StepResult::Error || s.resultado == StepResult::Blocked)
            .collect()
    }

    /// Genera reporte legible
    pub fn report(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("📋 Command Trail — Sesión: {}\n", &self.session_id[..8]));
        out.push_str(&format!("🎯 Objetivo: {}\n", self.objetivo));
        out.push_str(&format!("📅 Creado: {} | Actualizado: {}\n", self.created_at, self.updated_at));
        out.push_str(&format!("📊 Pasos totales: {}\n", self.steps.len()));
        
        let failed = self.failed_steps().len();
        if failed > 0 {
            out.push_str(&format!("❌ Pasos fallidos: {}\n", failed));
        }
        out.push_str("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\n");

        for step in &self.steps {
            let icon = match step.resultado {
                StepResult::Success => "✅",
                StepResult::Error => "❌",
                StepResult::Blocked => "🚫",
                StepResult::Skipped => "⏭️",
            };
            let archivos_str = if step.archivos.is_empty() { "—".to_string() } else { step.archivos.join(", ") };
            let comando_str = if step.comando.is_empty() { "—".to_string() } else { step.comando.clone() };
            out.push_str(&format!(
                "{} Paso {} [{}] {} → {}\n   🛠 {} | 📁 {} | ⏱ {}ms\n",
                icon, step.step, step.role, step.tool, step.resultado,
                comando_str, archivos_str, step.duracion_ms
            ));
            if let Some(err) = &step.error {
                out.push_str(&format!("   ⚠️  {}\n", err));
            }
            out.push('\n');
        }
        out
    }

    /// Exporta a JSON para análisis externo
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trail_creation() {
        let trail = CommandTrail::new("Test objetivo");
        assert_eq!(trail.objetivo, "Test objetivo");
        assert_eq!(trail.steps.len(), 0);
    }

    #[test]
    fn test_add_step() {
        let mut trail = CommandTrail::new("Test");
        trail.add_step(
            1, "Executor", "TOOL_PROGRAMMER", "cargo build",
            vec!["src/main.rs".to_string()],
            StepResult::Success, None, 1500, "contexto test"
        );
        assert_eq!(trail.steps.len(), 1);
        assert_eq!(trail.steps[0].tool, "TOOL_PROGRAMMER");
    }

    #[test]
    fn test_max_steps() {
        let mut trail = CommandTrail::new("Test");
        for i in 0..MAX_TRAIL_STEPS + 10 {
            trail.add_step(
                i as u32, "Executor", "TOOL_TEST", "test",
                vec![], StepResult::Success, None, 100, "ctx"
            );
        }
        assert_eq!(trail.steps.len(), MAX_TRAIL_STEPS);
    }
}