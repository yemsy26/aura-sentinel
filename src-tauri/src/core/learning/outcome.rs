use serde::{Deserialize, Serialize};

/// Why a mission ended — with enough detail to drive negative learning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LearningOutcome {
    Success,
    PartialSuccess,
    Failed,
    Blocked,
    Cancelled,
}

impl LearningOutcome {
    /// Weight for success-rate calculations (0.0, 0.5, or 1.0)
    pub fn success_weight(&self) -> f32 {
        match self {
            LearningOutcome::Success        => 1.0,
            LearningOutcome::PartialSuccess => 0.5,
            _                               => 0.0,
        }
    }
}

/// Quantitative metrics extracted from a completed mission.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OutcomeMetrics {
    pub steps: u32,
    pub tool_calls: u32,
    pub failed_actions: u32,
    pub recovery_actions: u32,
    pub verification_attempts: u32,
    pub successful_verifications: u32,
    pub elapsed_ms: u64,
}

/// A single recorded tool failure — enables failure-class learning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureInfo {
    /// Class of failure: "Compile", "Test", "Timeout", "Permission", "Schema", etc.
    pub class: String,
    pub tool: Option<String>,
    pub step: u32,
}

/// The RecoveryEngine decision that was actually applied, and whether it worked.
/// Enables learning which recovery strategies work for which failure classes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryRecord {
    pub strategy: String,   // "Retry", "ChangeStrategy", "ChangeTool", "Replan", "Abort"
    pub succeeded: bool,
}

/// Complete outcome data captured at mission end.
/// Built by agent.rs and passed to LearningEngine — Runtime v4 is not modified.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningResult {
    pub outcome: LearningOutcome,
    pub metrics: OutcomeMetrics,
    pub failures: Vec<FailureInfo>,
    pub recovery: Option<RecoveryRecord>,
}

impl LearningResult {
    pub fn success(metrics: OutcomeMetrics) -> Self {
        Self {
            outcome: LearningOutcome::Success,
            metrics,
            failures: vec![],
            recovery: None,
        }
    }

    pub fn failed(metrics: OutcomeMetrics, failures: Vec<FailureInfo>) -> Self {
        Self {
            outcome: LearningOutcome::Failed,
            metrics,
            failures,
            recovery: None,
        }
    }
}
