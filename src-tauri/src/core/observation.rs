#![allow(dead_code)]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ObservationStatus {
    Success,
    Error,
    BlockedByPolicy,
    Timeout,
    SchemaViolation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub tool_name: String,
    pub status: ObservationStatus,
    pub payload: String,
    pub exit_code: Option<i32>,
    pub files_affected: Vec<String>,
    /// The exact command string that was executed (for RepeatedCommand stall detection)
    pub command: Option<String>,
    /// World state hash before the tool ran (for NoStateChange stall detection)
    pub state_hash_before: Option<u64>,
    /// World state hash after the tool ran
    pub state_hash_after: Option<u64>,
    pub retryable: bool,
    pub requires_human: bool,
    pub suggested_fix: Option<String>,
}

impl Observation {
    pub fn success(tool_name: impl Into<String>, payload: impl Into<String>, files: Vec<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            status: ObservationStatus::Success,
            payload: payload.into(),
            exit_code: Some(0),
            files_affected: files,
            command: None,
            state_hash_before: None,
            state_hash_after: None,
            retryable: false,
            requires_human: false,
            suggested_fix: None,
        }
    }

    pub fn error(
        tool_name: impl Into<String>,
        error_msg: impl Into<String>,
        exit_code: Option<i32>,
        retryable: bool,
        suggested_fix: Option<String>,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            status: ObservationStatus::Error,
            payload: error_msg.into(),
            exit_code,
            files_affected: Vec::new(),
            command: None,
            state_hash_before: None,
            state_hash_after: None,
            retryable,
            requires_human: !retryable,
            suggested_fix,
        }
    }

    pub fn blocked_by_policy(tool_name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            status: ObservationStatus::BlockedByPolicy,
            payload: reason.into(),
            exit_code: None,
            files_affected: Vec::new(),
            command: None,
            state_hash_before: None,
            state_hash_after: None,
            retryable: false,
            requires_human: true,
            suggested_fix: Some("Solicitar aprobación explícita del usuario o redefinir la acción dentro de la política de seguridad.".to_string()),
        }
    }

    pub fn schema_violation(tool_name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            status: ObservationStatus::SchemaViolation,
            payload: reason.into(),
            exit_code: None,
            files_affected: Vec::new(),
            command: None,
            state_hash_before: None,
            state_hash_after: None,
            retryable: true,
            requires_human: false,
            suggested_fix: Some("Asegúrate de enviar los argumentos requeridos en JSON estricto con los tipos correctos.".to_string()),
        }
    }

    pub fn to_agent_context_entry(&self, step: usize) -> String {
        let status_str = match self.status {
            ObservationStatus::Success => "SUCCESS",
            ObservationStatus::Error => "ERROR",
            ObservationStatus::BlockedByPolicy => "BLOCKED_BY_POLICY",
            ObservationStatus::Timeout => "TIMEOUT",
            ObservationStatus::SchemaViolation => "SCHEMA_VIOLATION",
        };

        let mut out = format!(
            "[OBSERVATION - PASO {}]\nHerramienta: {}\nEstado: {}\nDetalle:\n{}\n",
            step, self.tool_name, status_str, self.payload
        );

        if let Some(fix) = &self.suggested_fix {
            out.push_str(&format!("Sugerencia de Recuperación: {}\n", fix));
        }

        if !self.files_affected.is_empty() {
            out.push_str(&format!("Archivos Afectados: {:?}\n", self.files_affected));
        }

        out.push('\n');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_observation_creation() {
        let obs = Observation::success("TOOL_TERMINAL", "build passed", vec!["src/main.rs".to_string()]);
        assert_eq!(obs.status, ObservationStatus::Success);
        assert_eq!(obs.exit_code, Some(0));
        assert!(!obs.retryable);

        let err_obs = Observation::error("TOOL_TERMINAL", "exit 1", Some(1), true, Some("fix flag".to_string()));
        assert_eq!(err_obs.status, ObservationStatus::Error);
        assert!(err_obs.retryable);

        let policy_obs = Observation::blocked_by_policy("TOOL_TERMINAL", "rm -rf blocked");
        assert_eq!(policy_obs.status, ObservationStatus::BlockedByPolicy);
        assert!(policy_obs.requires_human);
    }
}
