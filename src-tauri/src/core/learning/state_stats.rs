//! state_stats.rs — AL-v2.3 StateSignature-indexed Strategy Statistics
//!
//! Tracks (StateSignature -> Strategy -> Outcome) frequencies to enable
//! the AdaptiveRouter to recommend strategies based on operational state
//! rather than only task description.
//!
//! AUTHORITY: This module ONLY reads Experience history and exposes a
//! recommendation interface. It never executes tools, modifies runtime
//! state, or declares mission completion.

#![allow(dead_code)]

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use serde::{Deserialize, Serialize};
use crate::core::learning::strategy::StrategyKind;
use crate::core::learning::signature::StateSignature;
use crate::core::learning::outcome::LearningOutcome;

/// Deterministically hash a StateSignature into a u64 bucket key.
pub fn hash_state(sig: &StateSignature) -> u64 {
    let mut h = DefaultHasher::new();
    sig.task_fingerprint_hash.hash(&mut h);
    sig.phase.hash(&mut h);
    // Bucket compile_failures into quartiles to allow fuzzy matching
    (sig.compile_failures / 3).hash(&mut h);
    (sig.test_failures / 3).hash(&mut h);
    sig.progress_stalled.hash(&mut h);
    sig.last_error_class.as_deref().unwrap_or("").hash(&mut h);
    h.finish()
}

/// A single observation of a strategy outcome in a specific state bucket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateStrategyRecord {
    pub state_hash: u64,
    pub strategy: StrategyKind,
    pub attempts: u32,
    pub successes: u32,
    pub avg_duration_ms: u64,
    pub avg_recovery_count: f32,
}

impl StateStrategyRecord {
    fn new(state_hash: u64, strategy: StrategyKind) -> Self {
        Self {
            state_hash, strategy, attempts: 0, successes: 0,
            avg_duration_ms: 0, avg_recovery_count: 0.0,
        }
    }

    pub fn smoothed_success_rate(&self) -> f32 {
        (self.successes as f32 + 0.5) / (self.attempts as f32 + 1.0)
    }

    fn record(&mut self, success: bool, duration_ms: u64, recovery_count: f32) {
        self.attempts += 1;
        if success { self.successes += 1; }
        let n = self.attempts as f32;
        self.avg_duration_ms = ((self.avg_duration_ms as f32 * (n - 1.0) + duration_ms as f32) / n) as u64;
        self.avg_recovery_count = (self.avg_recovery_count * (n - 1.0) + recovery_count) / n;
    }
}

/// Cache of (state_hash -> Vec<StateStrategyRecord>) built from Experience history.
/// DERIVED CACHE — source of truth is always ExperienceStore.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StateStrategyIndex {
    pub index: HashMap<u64, Vec<StateStrategyRecord>>,
}

impl StateStrategyIndex {
    pub fn new() -> Self { Self::default() }

    /// Update the index from a single experience trajectory.
    pub fn update_from_experience(
        &mut self,
        trajectory_steps: &[crate::core::learning::trajectory::TrajectoryStep],
        mission_duration_ms: u64,
        mission_outcome: &LearningOutcome,
        recovery_count: f32,
    ) {
        let mission_success = matches!(mission_outcome,
            LearningOutcome::Success | LearningOutcome::PartialSuccess);

        for step in trajectory_steps {
            let state_hash = hash_state(&step.from_state);
            let records = self.index.entry(state_hash).or_default();

            let record = records.iter_mut()
                .find(|r| r.strategy == step.strategy);

            let effective_success = step.success && mission_success;

            if let Some(rec) = record {
                rec.record(effective_success, mission_duration_ms, recovery_count);
            } else {
                let mut rec = StateStrategyRecord::new(state_hash, step.strategy.clone());
                rec.record(effective_success, mission_duration_ms, recovery_count);
                records.push(rec);
            }
        }
    }

    /// Find the best strategy for the given state.
    /// Returns None when insufficient data (below min_attempts).
    pub fn best_strategy_for_state(
        &self,
        sig: &StateSignature,
        min_attempts: u32,
    ) -> Option<(StrategyKind, f32)> {
        let state_hash = hash_state(sig);

        // Exact bucket match first
        if let Some(records) = self.index.get(&state_hash) {
            let best = records.iter()
                .filter(|r| r.attempts >= min_attempts)
                .max_by(|a, b| {
                    a.smoothed_success_rate()
                        .partial_cmp(&b.smoothed_success_rate())
                        .unwrap_or(std::cmp::Ordering::Equal)
                });

            if let Some(rec) = best {
                return Some((rec.strategy.clone(), rec.smoothed_success_rate()));
            }
        }

        // AL-v2.3 Fix: Do NOT scan all buckets globally. If the specific state bucket
        // has no data, return None and let the AdaptiveRouter fall back to AL-v1 behavior.
        None
    }

    pub fn total_records(&self) -> usize {
        self.index.values().map(|v| v.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::learning::signature::StateSignatureBuilder;
    use crate::core::learning::trajectory::{Trajectory, TrajectoryStep};

    fn make_state(phase: u32, compile_fail: u32, stalled: bool) -> StateSignature {
        StateSignatureBuilder::new("fp_hash_test")
            .phase(phase).compile_failures(compile_fail).progress_stalled(stalled).build()
    }

    #[test]
    fn test_state_strategy_index_basic_update() {
        let mut idx = StateStrategyIndex::new();
        let state = make_state(0, 2, true);
        let mut traj = Trajectory::new("m_test", "fp_hash_test");
        traj.record_step(TrajectoryStep {
            step: 1, from_state: state.clone(),
            strategy: StrategyKind::DiagnoseThenRepair,
            tool: "TOOL_TERMINAL".to_string(), success: true,
            error_encountered: None, to_state: None,
        });
        traj.finalize(LearningOutcome::Success, 4000);
        idx.update_from_experience(&traj.steps, traj.total_duration_ms, &traj.outcome, 1.0);
        assert_eq!(idx.total_records(), 1);
        let (strat, rate) = idx.best_strategy_for_state(&state, 1).unwrap();
        assert_eq!(strat, StrategyKind::DiagnoseThenRepair);
        assert!(rate > 0.5);
    }

    #[test]
    fn test_state_strategy_index_picks_best_strategy() {
        let mut idx = StateStrategyIndex::new();
        let state = make_state(1, 0, false);

        let mut traj1 = Trajectory::new("m1", "fp_hash_test");
        traj1.record_step(TrajectoryStep {
            step: 1, from_state: state.clone(),
            strategy: StrategyKind::DirectImplementation,
            tool: "TOOL_PROGRAMMER".to_string(), success: false,
            error_encountered: None, to_state: None,
        });
        traj1.finalize(LearningOutcome::Failed, 5000);

        let mut traj2 = Trajectory::new("m2", "fp_hash_test");
        traj2.record_step(TrajectoryStep {
            step: 1, from_state: state.clone(),
            strategy: StrategyKind::InspectThenImplement,
            tool: "TOOL_PROGRAMMER".to_string(), success: true,
            error_encountered: None, to_state: None,
        });
        traj2.finalize(LearningOutcome::Success, 3000);

        idx.update_from_experience(&traj1.steps, traj1.total_duration_ms, &traj1.outcome, 0.0);
        idx.update_from_experience(&traj2.steps, traj2.total_duration_ms, &traj2.outcome, 0.0);

        let (best, _) = idx.best_strategy_for_state(&state, 1).unwrap();
        assert_eq!(best, StrategyKind::InspectThenImplement);
    }

    #[test]
    fn test_state_strategy_index_returns_none_below_min_attempts() {
        let mut idx = StateStrategyIndex::new();
        let state = make_state(0, 0, false);
        let mut traj = Trajectory::new("m_single", "fp_hash_test");
        traj.record_step(TrajectoryStep {
            step: 1, from_state: state.clone(),
            strategy: StrategyKind::MinimalChange,
            tool: "TOOL_PROGRAMMER".to_string(), success: true,
            error_encountered: None, to_state: None,
        });
        traj.finalize(LearningOutcome::Success, 2000);
        idx.update_from_experience(&traj.steps, traj.total_duration_ms, &traj.outcome, 0.0);
        // min_attempts=5: no hay suficientes datos
        assert!(idx.best_strategy_for_state(&state, 5).is_none());
    }
}
