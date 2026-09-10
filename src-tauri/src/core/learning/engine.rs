use std::sync::Arc;
use tokio::sync::RwLock;
use crate::core::learning::experience::{Experience, SharedExperienceStore, SCHEMA_VERSION};
use crate::core::learning::fingerprint::TaskFingerprint;
use crate::core::learning::outcome::LearningResult;
use crate::core::learning::persistence::LearningPersistence;
use crate::core::learning::strategy::StrategyKind;
use crate::core::learning::state_stats::StateStrategyIndex;

/// Read-only snapshot of MissionRuntime metrics for LearningEngine.
/// LearningEngine DOES NOT hold a reference to MissionRuntime — it receives a plain data copy.
/// This preserves the authority boundary: Runtime v4 remains the sole execution authority.
#[allow(dead_code)]
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
    #[allow(dead_code)]
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

    #[allow(dead_code)]
    pub fn with_store_and_persistence(store: SharedExperienceStore, persistence: LearningPersistence) -> Self {
        Self {
            store,
            persistence,
        }
    }

    #[allow(dead_code)]
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
        self.record_outcome_with_trajectory(
            fingerprint, model, strategy, result, confidence, mission_id, attempt_id, None,
        ).await
    }

    /// Record the outcome of a completed mission, optionally with an execution trajectory.
    pub async fn record_outcome_with_trajectory(
        &self,
        fingerprint: TaskFingerprint,
        model: String,
        strategy: StrategyKind,
        result: LearningResult,
        confidence: f32,
        mission_id: String,
        attempt_id: Option<String>,
        trajectory: Option<crate::core::learning::trajectory::Trajectory>,
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
            trajectory,
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

            // 4. Update StateStrategyIndex from trajectory (AL-v2.3)
            // Derived cache — rebuilt from experience trajectory on each call
            if let Some(ref traj) = exp.trajectory {
                let mut state_idx = StateStrategyIndex::new();
                for ex in &all_exps {
                    if let Some(ref t) = ex.trajectory {
                        let recovery = ex.result.metrics.recovery_actions as f32;
                        state_idx.update_from_experience(
                            &t.steps,
                            t.total_duration_ms,
                            &ex.result.outcome,
                            recovery,
                        );
                    }
                }
                // Also include the current experience's trajectory
                let recovery = exp.result.metrics.recovery_actions as f32;
                state_idx.update_from_experience(
                    &traj.steps,
                    traj.total_duration_ms,
                    &exp.result.outcome,
                    recovery,
                );
                if let Err(e) = self.persistence.save_state_strategy_index(&state_idx).await {
                    eprintln!("[LearningEngine] PERSIST_STATE_STATS_WARN: {}", e);
                }

                // 5. Update RecoveryIndex from trajectory recoveries (AL-v2.4)
                use crate::core::learning::recovery_index::RecoveryIndex;
                let mut rec_idx = RecoveryIndex::new();
                for ex in &all_exps {
                    if let Some(ref t) = ex.trajectory {
                        let recoveries = t.extract_recoveries();
                        rec_idx.update_from_recoveries(&recoveries);
                    }
                }
                // Include current trajectory
                let current_recoveries = traj.extract_recoveries();
                rec_idx.update_from_recoveries(&current_recoveries);
                if let Err(e) = self.persistence.save_recovery_index(&rec_idx).await {
                    eprintln!("[LearningEngine] PERSIST_RECOVERY_IDX_WARN: {}", e);
                }
            }

            // 6. Rebuild BudgetAwareIndex from all experiences (AL-v2.5)
            // Does NOT require trajectory — uses OutcomeMetrics.steps from every experience.
            {
                use crate::core::learning::budget_stats::BudgetAwareIndex;
                let budget_idx = BudgetAwareIndex::rebuild_from_experiences(&all_exps);
                if let Err(e) = self.persistence.save_budget_index(&budget_idx).await {
                    eprintln!("[LearningEngine] PERSIST_BUDGET_IDX_WARN: {}", e);
                }
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
