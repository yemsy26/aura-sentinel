use std::sync::{Arc, Mutex};
use crate::core::learning::experience::{Experience, ExperienceStoreV2, SCHEMA_VERSION};
use crate::core::learning::fingerprint::TaskFingerprint;
use crate::core::learning::outcome::LearningResult;
use crate::core::learning::persistence::LearningPersistence;
use crate::core::learning::strategy::StrategyKind;
use crate::core::mission_runtime::MissionRuntime;

/// Read-only snapshot of MissionRuntime metrics for LearningEngine.
/// LearningEngine DOES NOT hold a reference to MissionRuntime — it receives a snapshot copy.
/// This preserves the authority boundary: Runtime v4 remains the sole execution authority.
#[derive(Debug, Clone)]
pub struct RuntimeSnapshot {
    pub steps_taken: u32,
    pub budget_total: u32,
    pub recovery_count: u32,
    pub stall_count: u32,
    pub duration_ms: u64,
}

impl RuntimeSnapshot {
    /// Build from MissionRuntime. Called by agent.rs — not by LearningEngine itself.
    pub fn from_runtime(rt: &MissionRuntime, duration_ms: u64) -> Self {
        Self {
            steps_taken: rt.steps_taken(),
            budget_total: rt.budget_total(),
            recovery_count: rt.recovery_count(),
            stall_count: rt.stall_count(),
            duration_ms,
        }
    }
}

/// The ONLY component that writes to ExperienceStoreV2.
/// Called by agent.rs after Runtime v4 has determined the mission outcome.
/// Never called mid-mission. Never called from AdaptiveRouter.
pub struct LearningEngine {
    store: Arc<Mutex<ExperienceStoreV2>>,
    persistence: LearningPersistence,
}

impl LearningEngine {
    pub fn new() -> Self {
        let persistence = LearningPersistence::new();
        let store = persistence.load_experiences(500);
        Self {
            store: Arc::new(Mutex::new(store)),
            persistence,
        }
    }

    /// Record the outcome of a completed mission.
    /// Called ONCE per mission, after CompletionGate determines outcome.
    /// Runtime v4 is NOT modified. This only appends to the experience log.
    pub async fn record_outcome(
        &self,
        fingerprint: TaskFingerprint,
        model: String,
        strategy: StrategyKind,
        result: LearningResult,
        confidence: f32,
        mission_id: String,
        attempt_num: u32,
    ) -> Result<(), String> {
        let attempt_id = format!("{}_{}", mission_id, attempt_num);

        let exp = Experience {
            schema_version: SCHEMA_VERSION,
            id: Self::generate_id(),
            attempt_id: attempt_id.clone(),
            mission_id,
            timestamp: Self::unix_secs(),
            fingerprint,
            model,
            strategy,
            result,
            confidence,
            lesson: None,
        };

        // Write to in-memory store (idempotent)
        let pushed = {
            let mut store = self.store.lock().map_err(|e| e.to_string())?;
            store.push(exp.clone())
        };

        if pushed {
            // Persist atomically — non-blocking failure: learning never crashes mission
            if let Err(e) = self.persistence.append_experience(&exp).await {
                // Log but don't propagate — a persistence failure must not crash the agent
                eprintln!("[LearningEngine] PERSIST_WARN: {}", e);
            }
        }

        Ok(())
    }

    fn generate_id() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("exp_{:x}", t)
    }

    fn unix_secs() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}
