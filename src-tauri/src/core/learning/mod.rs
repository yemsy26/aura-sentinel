/// Adaptive Learning v1 — Authority boundary invariants
///
/// PERMITTED:
///   - Read ExperienceStoreV2
///   - Read ModelStats / StrategyStats
///   - Produce Recommendation
///   - Write Experience (only via LearningEngine, only at mission end)
///
/// NEVER PERMITTED:
///   - Access MissionRuntime, ToolRegistry, PolicyEngine, CompletionGate
///   - Modify CognitiveState or MissionContract
///   - Call execute_action() or tool_registry.dispatch()
///   - Declare CompletionDecision

pub mod fingerprint;
pub mod outcome;
pub mod strategy;
pub mod experience;
pub mod stats;
pub mod persistence;
pub mod router;
pub mod engine;
pub mod signature;
pub mod trajectory;
pub mod state_stats;
pub mod recovery_index;

#[cfg(test)]
mod tests;

// Public surface — used by agent.rs wiring and AL-v2
#[allow(unused_imports)]
pub use fingerprint::{TaskFingerprint, FingerprintBuilder};
#[allow(unused_imports)]
pub use outcome::{LearningOutcome, OutcomeMetrics, FailureInfo, RecoveryRecord, LearningResult};
#[allow(unused_imports)]
pub use strategy::StrategyKind;
#[allow(unused_imports)]
pub use experience::{Experience, ExperienceStoreV2, SharedExperienceStore};
#[allow(unused_imports)]
pub use stats::{ModelStats as LearningModelStats, StrategyStats};
#[allow(unused_imports)]
pub use router::{AdaptiveRouter, Recommendation, RecommendationReason};
#[allow(unused_imports)]
pub use persistence::LearningPersistence;
#[allow(unused_imports)]
pub use engine::{LearningEngine, RuntimeSnapshot};
#[allow(unused_imports)]
pub use signature::{StateSignature, StateSignatureBuilder, VerificationLevel};
#[allow(unused_imports)]
pub use trajectory::{Trajectory, TrajectoryStep, RecoverySequence};
#[allow(unused_imports)]
pub use state_stats::{StateStrategyIndex, StateStrategyRecord, hash_state};
#[allow(unused_imports)]
pub use recovery_index::{RecoveryIndex, RecoveryPattern, RecoveryRecommendation};
