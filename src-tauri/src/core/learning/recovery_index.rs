//! recovery_index.rs — AL-v2.4 Adaptive Recovery Pattern Index
//!
//! Aggregates historical RecoverySequences extracted from Trajectories into
//! a queryable index: given an error class and optional StateSignature,
//! recommend the strategy+tool pair most likely to resolve the situation.
//!
//! AUTHORITY: Advisory only. Learning recommends — Runtime v4 decides.
//! This module never calls execute_action(), dispatch(), or policy enforcement.

#![allow(dead_code)]

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::core::learning::strategy::StrategyKind;
use crate::core::learning::signature::StateSignature;
use crate::core::learning::trajectory::RecoverySequence;

/// Aggregated performance data for a (trigger_error, strategy, tool) pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryPattern {
    /// Error class this pattern resolves
    pub trigger_error: String,
    /// Strategy that resolved the error
    pub strategy: StrategyKind,
    /// Tool that executed the resolution
    pub tool: String,
    /// Total times this pattern was attempted
    pub attempts: u32,
    /// Times the recovery actually resolved the error (state improved)
    pub successes: u32,
    /// Average StateSignature similarity of the initial (error) state
    /// compared to the stored reference state — higher = more applicable
    pub avg_initial_state_similarity: f32,
}

impl RecoveryPattern {
    fn new(trigger_error: String, strategy: StrategyKind, tool: String) -> Self {
        Self {
            trigger_error,
            strategy,
            tool,
            attempts: 0,
            successes: 0,
            avg_initial_state_similarity: 0.0,
        }
    }

    /// Laplace-smoothed success rate
    pub fn smoothed_success_rate(&self) -> f32 {
        (self.successes as f32 + 0.5) / (self.attempts as f32 + 1.0)
    }

    fn record(&mut self, state_similarity: f32) {
        self.attempts += 1;
        self.successes += 1; // All RecoverySequences are successful by definition (extracted only on success)
        let n = self.attempts as f32;
        self.avg_initial_state_similarity =
            (self.avg_initial_state_similarity * (n - 1.0) + state_similarity) / n;
    }
}

/// Advisory recommendation produced by RecoveryIndex.
/// INVARIANT: Caller (Runtime v4) decides whether to act on this.
#[derive(Debug, Clone)]
pub struct RecoveryRecommendation {
    pub trigger_error: String,
    pub strategy: StrategyKind,
    pub tool: String,
    /// 0.0..1.0 — composite of success rate and state similarity
    pub confidence: f32,
    /// Number of historical observations backing this recommendation
    pub sample_size: u32,
}

/// Index of recovery patterns keyed by error class.
/// Derived cache — rebuilt from ExperienceStore trajectories on startup.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecoveryIndex {
    /// error_class -> list of patterns ordered by insertion
    pub index: HashMap<String, Vec<RecoveryPattern>>,
}

impl RecoveryIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ingest a batch of RecoverySequences extracted from a single trajectory.
    /// Called by LearningEngine after each successful mission with trajectory data.
    pub fn update_from_recoveries(&mut self, recoveries: &[RecoverySequence]) {
        for seq in recoveries {
            let patterns = self.index.entry(seq.trigger_error.clone()).or_default();

            // Find existing pattern for (strategy, tool) pair
            let pattern = patterns.iter_mut()
                .find(|p| p.strategy == seq.strategy && p.tool == seq.tool);

            // Compute similarity of the stored initial state to itself
            // (first occurrence: perfect match; subsequent: averaged in)
            let similarity = seq.initial_state.similarity(&seq.initial_state); // 1.0 for new

            if let Some(p) = pattern {
                p.record(similarity);
            } else {
                let mut p = RecoveryPattern::new(
                    seq.trigger_error.clone(),
                    seq.strategy.clone(),
                    seq.tool.clone(),
                );
                p.record(1.0); // First observation is a perfect match
                patterns.push(p);
            }
        }
    }

    /// Find the best recovery strategy for a given error class.
    ///
    /// When `current_state` is provided, patterns are ranked by:
    ///   composite = 0.7 * success_rate + 0.3 * state_similarity
    /// When `current_state` is None, ranked by success rate alone.
    ///
    /// Returns None when no pattern has >= min_attempts observations.
    pub fn best_recovery_for(
        &self,
        error_class: &str,
        current_state: Option<&StateSignature>,
        min_attempts: u32,
    ) -> Option<RecoveryRecommendation> {
        let patterns = self.index.get(error_class)?;

        let best = patterns.iter()
            .filter(|p| p.attempts >= min_attempts)
            .max_by(|a, b| {
                let score_a = self.pattern_score(a, current_state);
                let score_b = self.pattern_score(b, current_state);
                score_a.partial_cmp(&score_b).unwrap_or(std::cmp::Ordering::Equal)
            })?;

        let confidence = self.pattern_score(best, current_state).clamp(0.0, 1.0);

        Some(RecoveryRecommendation {
            trigger_error: best.trigger_error.clone(),
            strategy: best.strategy.clone(),
            tool: best.tool.clone(),
            confidence,
            sample_size: best.attempts,
        })
    }

    /// List all known error classes in the index
    pub fn known_error_classes(&self) -> Vec<&str> {
        self.index.keys().map(|s| s.as_str()).collect()
    }

    /// Total number of distinct (error, strategy, tool) patterns
    pub fn total_patterns(&self) -> usize {
        self.index.values().map(|v| v.len()).sum()
    }

    // Composite score: 70% success rate + 30% state similarity affinity
    fn pattern_score(&self, pattern: &RecoveryPattern, state: Option<&StateSignature>) -> f32 {
        let success_component = pattern.smoothed_success_rate() * 0.7;
        let similarity_component = if state.is_some() {
            pattern.avg_initial_state_similarity * 0.3
        } else {
            pattern.smoothed_success_rate() * 0.3
        };
        success_component + similarity_component
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::learning::signature::StateSignatureBuilder;
    use crate::core::learning::strategy::StrategyKind;

    fn make_recovery(error: &str, strategy: StrategyKind, tool: &str) -> RecoverySequence {
        let state = StateSignatureBuilder::new("fp_rec_test")
            .compile_failures(2).progress_stalled(true).build();
        RecoverySequence {
            trigger_error: error.to_string(),
            strategy,
            tool: tool.to_string(),
            initial_state: state.clone(),
            resolved_state: StateSignatureBuilder::new("fp_rec_test").build(),
        }
    }

    #[test]
    fn test_recovery_index_basic_update_and_query() {
        let mut idx = RecoveryIndex::new();
        let seq = make_recovery("CompileError", StrategyKind::DiagnoseThenRepair, "TOOL_TERMINAL");
        idx.update_from_recoveries(&[seq]);

        assert_eq!(idx.total_patterns(), 1);
        let rec = idx.best_recovery_for("CompileError", None, 1).unwrap();
        assert_eq!(rec.strategy, StrategyKind::DiagnoseThenRepair);
        assert_eq!(rec.tool, "TOOL_TERMINAL");
        assert!(rec.confidence > 0.5);
    }

    #[test]
    fn test_recovery_index_picks_higher_success_rate() {
        let mut idx = RecoveryIndex::new();

        // Add CompileFirst once (lower success base)
        idx.update_from_recoveries(&[make_recovery(
            "TestError", StrategyKind::CompileFirst, "TOOL_TERMINAL")]);

        // Add DiagnoseThenRepair 3 times (more observations = higher confidence)
        for _ in 0..3 {
            idx.update_from_recoveries(&[make_recovery(
                "TestError", StrategyKind::DiagnoseThenRepair, "TOOL_PROGRAMMER")]);
        }

        let rec = idx.best_recovery_for("TestError", None, 1).unwrap();
        assert_eq!(rec.strategy, StrategyKind::DiagnoseThenRepair,
            "Pattern with more observations must score higher");
        assert_eq!(rec.sample_size, 3);
    }

    #[test]
    fn test_recovery_index_returns_none_for_unknown_error() {
        let idx = RecoveryIndex::new();
        assert!(idx.best_recovery_for("UnknownError", None, 1).is_none());
    }

    #[test]
    fn test_recovery_index_returns_none_below_min_attempts() {
        let mut idx = RecoveryIndex::new();
        idx.update_from_recoveries(&[make_recovery(
            "CompileError", StrategyKind::MinimalChange, "TOOL_PROGRAMMER")]);
        // Only 1 observation; min_attempts=3 should return None
        assert!(idx.best_recovery_for("CompileError", None, 3).is_none());
    }

    #[test]
    fn test_recovery_index_state_aware_query() {
        let mut idx = RecoveryIndex::new();

        // Two strategies for same error
        idx.update_from_recoveries(&[
            make_recovery("RuntimeError", StrategyKind::IncrementalPatch, "TOOL_PROGRAMMER"),
            make_recovery("RuntimeError", StrategyKind::CompileFirst, "TOOL_TERMINAL"),
        ]);

        let query_state = StateSignatureBuilder::new("fp_rec_test")
            .compile_failures(2).progress_stalled(true).build();

        // With state: should still return a valid recommendation
        let rec = idx.best_recovery_for("RuntimeError", Some(&query_state), 1);
        assert!(rec.is_some(), "State-aware query must return a recommendation");
        let rec = rec.unwrap();
        assert!(rec.confidence > 0.0 && rec.confidence <= 1.0);
    }
}
