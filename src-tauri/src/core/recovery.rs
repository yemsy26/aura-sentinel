#![allow(dead_code)]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RecoveryAction {
    RetryWithFix { advice: String },
    AlternativeTool { recommended_tool: String, rationale: String },
    RewriteFileDirectly { file_path: String },
    AskUserClarification { prompt: String },
    AbortMission { reason: String },
}

#[derive(Debug, Default)]
pub struct RecoveryEngine {
    tool_failure_counts: std::collections::HashMap<String, usize>,
}

impl RecoveryEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a failure and deterministically determines the optimal recovery action.
    pub fn plan_recovery(&mut self, tool_name: &str, error_msg: &str) -> RecoveryAction {
        let count = self.tool_failure_counts.entry(tool_name.to_string()).or_insert(0);
        *count += 1;
        let failures = *count;

        let err_lower = error_msg.to_lowercase();

        // 1. Patch mismatch in programmer
        if tool_name == "TOOL_PROGRAMMER" && (err_lower.contains("no coincide") || err_lower.contains("patch") || err_lower.contains("match")) {
            return RecoveryAction::RewriteFileDirectly {
                file_path: "archivo_afectado".to_string(),
            };
        }

        // 2. Missing binary or command not found in terminal
        if tool_name == "TOOL_TERMINAL" && (err_lower.contains("not recognized") || err_lower.contains("not found") || err_lower.contains("no se reconoce")) {
            return RecoveryAction::AlternativeTool {
                recommended_tool: "TOOL_ENV_CHECK".to_string(),
                rationale: "El comando solicitado no está instalado en el PATH. Comprueba las dependencias disponibles.".to_string(),
            };
        }

        // 3. Repeated failure escalation
        if failures >= 3 {
            return RecoveryAction::AskUserClarification {
                prompt: format!(
                    "La herramienta '{}' ha fallado {} veces consecutivas con el error: '{}'. ¿Cómo deseas proceder?",
                    tool_name, failures, error_msg
                ),
            };
        }

        if failures == 2 {
            if tool_name == "TOOL_PROGRAMMER" {
                return RecoveryAction::AlternativeTool {
                    recommended_tool: "TOOL_THINK".to_string(),
                    rationale: "Fallo repetido al escribir código. Reevalúa la estructura lógica del módulo antes de reintentar.".to_string(),
                };
            }
        }

        RecoveryAction::RetryWithFix {
            advice: format!("Corrige el parámetro causante del error antes de volver a intentar: {}", error_msg),
        }
    }

    pub fn record_success(&mut self, tool_name: &str) {
        self.tool_failure_counts.remove(tool_name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recovery_planning() {
        let mut engine = RecoveryEngine::new();

        // Patch mismatch
        let action = engine.plan_recovery("TOOL_PROGRAMMER", "El bloque a buscar no coincide");
        assert!(matches!(action, RecoveryAction::RewriteFileDirectly { .. }));

        // Missing command
        let action = engine.plan_recovery("TOOL_TERMINAL", "cargo: command not found");
        assert!(matches!(action, RecoveryAction::AlternativeTool { .. }));

        // Escalation after 3 failures
        let mut engine2 = RecoveryEngine::new();
        let _ = engine2.plan_recovery("TOOL_TERMINAL", "generic error");
        let _ = engine2.plan_recovery("TOOL_TERMINAL", "generic error");
        let action3 = engine2.plan_recovery("TOOL_TERMINAL", "generic error");
        assert!(matches!(action3, RecoveryAction::AskUserClarification { .. }));
    }
}
