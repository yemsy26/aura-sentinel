use std::sync::Arc;
use tokio::sync::RwLock;
use crate::core::learning::experience::{Experience, SharedExperienceStore, SCHEMA_VERSION};
use crate::core::learning::fingerprint::TaskFingerprint;
use crate::core::learning::outcome::LearningResult;
use crate::core::learning::persistence::LearningPersistence;
use crate::core::learning::strategy::StrategyKind;

/// Read-only snapshot of MissionRuntime metrics for LearningEngine.
/// LearningEngine DOES NOT hold a reference to MissionRuntime — it receives a plain data copy.
/// This preserves the authority boundary: Runtime v4 remains the sole execution authority.
#[derive(Debug, Clone)]
pub struct RuntimeSnapshot {
    pub steps_taken: u32,
    pub budget_total: u32,
    pub recovery_count: u32,
    pub stall_count: u32,
    pub duration_ms: u64,
}

/// The ONLY component that writes to ExperienceStoreV2.
/// Called by agent.rs after Runtime v4 has determined the mission outcome.
/// Never called mid-mission. Never called from AdaptiveRouter.
pub struct LearningEngine {
    store: SharedExperienceStore,
    persistence: LearningPersistence,
}

impl LearningEngine {
    pub fn new() -> Self {
        let persistence = LearningPersistence::new();
        let store = persistence.load_experiences(500);
        Self {
            store: Arc::new(RwLock::new(store)),
            persistence,
        }
    }

    pub fn with_store(store: SharedExperienceStore) -> Self {
        let persistence = LearningPersistence::new();
        Self {
            store,
            persistence,
        }
    }

    pub fn store(&self) -> SharedExperienceStore {
        self.store.clone()
    }

    /// Record the outcome of a completed mission.
    /// Called ONCE per mission, after CompletionGate determines outcome.
    /// Runtime v4 is NOT modified. This appends to the experience log and updates derived stats.
    pub async fn record_outcome(
        &self,
        fingerprint: TaskFingerprint,
        model: String,
        strategy: StrategyKind,
        result: LearningResult,
        confidence: f32,
        mission_id: String,
        attempt_id: Option<String>,
    ) -> Result<(), String> {
        let final_attempt_id = attempt_id.unwrap_or_else(|| {
            Self::generate_attempt_id(&mission_id)
        });

        let exp = Experience {
            schema_version: SCHEMA_VERSION,
            id: Self::generate_id(),
            attempt_id: final_attempt_id,
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
        let (pushed, all_exps) = {
            let mut store = self.store.write().await;
            let pushed = store.push(exp.clone());
            let all_exps = if pushed { store.experiences.clone() } else { Vec::new() };
            (pushed, all_exps)
        };

        if pushed {
            // 1. Persist experience atomically
            if let Err(e) = self.persistence.append_experience(&exp).await {
                eprintln!("[LearningEngine] PERSIST_EXP_WARN: {}", e);
            }

            // 2. Rebuild stats from all experiences (Experience = single source of truth)
            use crate::core::learning::stats::{build_model_stats_map, build_strategy_stats_map};
            let model_stats = build_model_stats_map(&all_exps);
            let strategy_stats = build_strategy_stats_map(&all_exps);

            // 3. Persist stats as derived cache (not source of truth)
            if let Err(e) = self.persistence.save_model_stats(&model_stats).await {
                eprintln!("[LearningEngine] PERSIST_MODEL_STATS_WARN: {}", e);
            }
            if let Err(e) = self.persistence.save_strategy_stats(&strategy_stats).await {
                eprintln!("[LearningEngine] PERSIST_STRATEGY_STATS_WARN: {}", e);
            }
        }

        Ok(())
    }

    fn generate_attempt_id(mission_id: &str) -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("{}_{:x}", mission_id, t)
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
