//! budget_stats.rs — AL-v2.5 Budget-Aware Strategy Learning
//!
//! Tracks historical cost (steps taken) and success rate per (strategy, language)
//! so the AdaptiveRouter can prefer cheaper strategies when budget is tight.
//!
//! AUTHORITY: Advisory only. Produces cost-adjusted recommendations.
//! Never calls execute_action() or modifies Runtime v4 state.
//!
//! Key insight (auditor):
//!   Strategy A: success=91%, cost=high
//!   Strategy B: success=86%, cost=low
//!   → Prefer B when remaining_budget is scarce.

#![allow(dead_code)]

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::core::learning::strategy::StrategyKind;
use crate::core::learning::experience::Experience;
use crate::core::learning::outcome::LearningOutcome;

/// Historical cost + success profile for a (strategy, language_bucket) pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetProfile {
    pub strategy: StrategyKind,
    /// Language bucket — None means "any language"
    pub language: Option<String>,
    /// Number of missions observed
    pub attempts: u32,
    /// Laplace-smoothed success rate
    pub successes: f32,
    /// Average steps consumed per mission (cost proxy)
    pub avg_steps: f32,
    /// 75th-percentile steps — better worst-case estimate than mean
    pub p75_steps: u32,
    /// Step samples used for percentile calculation (capped at 50)
    step_samples: Vec<u32>,
}

impl BudgetProfile {
    pub fn new(strategy: StrategyKind, language: Option<String>) -> Self {
        Self {
            strategy, language,
            attempts: 0,
            successes: 0.0,
            avg_steps: 0.0,
            p75_steps: 0,
            step_samples: Vec::new(),
        }
    }

    /// Record one mission observation
    pub fn record(&mut self, steps: u32, outcome: &LearningOutcome) {
        self.attempts += 1;
        self.successes += outcome.success_weight();

        let n = self.attempts as f32;
        self.avg_steps = (self.avg_steps * (n - 1.0) + steps as f32) / n;

        // Maintain rolling window of up to 50 step samples for percentile
        if self.step_samples.len() >= 50 { self.step_samples.remove(0); }
        self.step_samples.push(steps);
        self.p75_steps = percentile_75(&self.step_samples);
    }

    /// Laplace-smoothed success rate
    pub fn smoothed_success_rate(&self) -> f32 {
        (self.successes + 0.5) / (self.attempts as f32 + 1.0)
    }

    /// Expected cost in steps (use p75 for safety margin)
    pub fn expected_cost_steps(&self) -> u32 {
        self.p75_steps.max(1)
    }

    /// Efficiency score: success per expected step consumed.
    /// Higher = more value per budget unit.
    pub fn efficiency(&self) -> f32 {
        self.smoothed_success_rate() / (self.expected_cost_steps() as f32)
    }
}

fn percentile_75(samples: &[u32]) -> u32 {
    if samples.is_empty() { return 0; }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let idx = ((sorted.len() as f32 * 0.75) as usize).min(sorted.len() - 1);
    sorted[idx]
}

/// Cache of BudgetProfiles indexed by (strategy, language).
/// Derived from ExperienceStore — rebuilt on each mission end.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BudgetAwareIndex {
    /// key: "<strategy>|<language_or_any>"
    pub profiles: HashMap<String, BudgetProfile>,
}

impl BudgetAwareIndex {
    pub fn new() -> Self { Self::default() }

    fn key(strategy: &StrategyKind, language: Option<&str>) -> String {
        format!("{}|{}", strategy.as_str(), language.unwrap_or("any"))
    }

    /// Ingest one experience into the budget index
    pub fn update_from_experience(&mut self, exp: &Experience) {
        let lang = exp.fingerprint.language.as_deref();
        let steps = exp.result.metrics.steps;

        // Update language-specific profile
        if let Some(l) = lang {
            let k = Self::key(&exp.strategy, Some(l));
            let profile = self.profiles.entry(k).or_insert_with(|| {
                BudgetProfile::new(exp.strategy.clone(), Some(l.to_string()))
            });
            profile.record(steps, &exp.result.outcome);
        }

        // Also update language-agnostic profile
        let k_any = Self::key(&exp.strategy, None);
        let profile_any = self.profiles.entry(k_any).or_insert_with(|| {
            BudgetProfile::new(exp.strategy.clone(), None)
        });
        profile_any.record(steps, &exp.result.outcome);
    }

    /// Rebuild the entire index from a slice of experiences.
    pub fn rebuild_from_experiences(experiences: &[Experience]) -> Self {
        let mut idx = Self::new();
        for exp in experiences { idx.update_from_experience(exp); }
        idx
    }

    /// Given a language and remaining budget, find the best strategy.
    ///
    /// Scoring model:
    ///   budget_ratio = remaining_budget / expected_cost_p75
    ///
    ///   if budget_ratio >= 2.0 (budget is ample):
    ///     score = success_rate             (prioritize success)
    ///   elif budget_ratio >= 1.0 (budget is moderate):
    ///     score = 0.7*success_rate + 0.3*efficiency
    ///   else (budget is tight, < 1x expected cost):
    ///     score = 0.4*success_rate + 0.6*efficiency  (prioritize efficiency)
    ///
    /// Returns None when insufficient data (< min_attempts).
    pub fn best_strategy_for_budget(
        &self,
        language: Option<&str>,
        remaining_budget: u32,
        min_attempts: u32,
    ) -> Option<BudgetStrategyChoice> {
        let strategies = [
            StrategyKind::DirectImplementation,
            StrategyKind::InspectThenImplement,
            StrategyKind::TestFirst,
            StrategyKind::CompileFirst,
            StrategyKind::IncrementalPatch,
            StrategyKind::DiagnoseThenRepair,
            StrategyKind::MinimalChange,
        ];

        let mut best: Option<BudgetStrategyChoice> = None;
        let mut best_score = -1.0f32;

        for strat in &strategies {
            // Prefer language-specific profile, fall back to any-language
            let profile = language
                .and_then(|l| self.profiles.get(&Self::key(strat, Some(l))))
                .or_else(|| self.profiles.get(&Self::key(strat, None)));

            let Some(profile) = profile else { continue };
            if profile.attempts < min_attempts { continue; }

            let expected = profile.expected_cost_steps().max(1) as f32;
            let effective_budget_ratio = remaining_budget as f32 / expected;

            // Scoring model:
            //   ample  (ratio ≥ 2.0): pure success rate — cost doesn't matter
            //   moderate (1.0..2.0): blend success + budget fit
            //   tight   (< 1.0):    discount success_rate by budget_ratio
            //                        → cheap strategies that fit the budget score higher
            let score = if effective_budget_ratio >= 2.0 {
                profile.smoothed_success_rate()
            } else if effective_budget_ratio >= 1.0 {
                let fit = effective_budget_ratio / 2.0; // 0.5..1.0
                0.7 * profile.smoothed_success_rate() + 0.3 * fit
            } else {
                // Tight: penalize by how badly strategy exceeds budget.
                // budget_ratio in (0, 1) — the lower it is, the more expensive relative to budget.
                let br = effective_budget_ratio.max(0.05);
                profile.smoothed_success_rate() * br
            };

            if score > best_score {
                best_score = score;
                best = Some(BudgetStrategyChoice {
                    strategy: strat.clone(),
                    expected_cost_steps: profile.expected_cost_steps(),
                    smoothed_success_rate: profile.smoothed_success_rate(),
                    budget_ratio: effective_budget_ratio,
                    score,
                });
            }
        }

        best
    }

    /// Total number of (strategy, language) profiles in the index
    pub fn total_profiles(&self) -> usize { self.profiles.len() }
}

/// Result of a budget-aware strategy selection
#[derive(Debug, Clone)]
pub struct BudgetStrategyChoice {
    pub strategy: StrategyKind,
    /// P75 estimated steps this strategy will consume
    pub expected_cost_steps: u32,
    /// Laplace-smoothed success rate
    pub smoothed_success_rate: f32,
    /// remaining_budget / expected_cost — < 1.0 means tight budget
    pub budget_ratio: f32,
    /// Composite score used for ranking
    pub score: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::learning::outcome::{LearningOutcome, OutcomeMetrics, LearningResult};
    use crate::core::learning::fingerprint::TaskFingerprint;
    use crate::core::learning::strategy::StrategyKind;
    use crate::core::learning::experience::{Experience, SCHEMA_VERSION};

    fn make_exp(strategy: StrategyKind, lang: &str, steps: u32, success: bool) -> Experience {
        Experience {
            schema_version: SCHEMA_VERSION,
            id: format!("exp_{steps}"),
            attempt_id: "att".to_string(),
            mission_id: "m".to_string(),
            timestamp: 0,
            fingerprint: TaskFingerprint {
                language: Some(lang.to_string()),
                framework: None,
                complexity: 0.5,
                ambiguity: 0.2,
                scope_bucket: 1,
                requires_code: true,
                requires_terminal: false,
                requires_tests: false,
                requires_network: false,
                verification_level: 1,
            },
            model: "test-model".to_string(),
            strategy,
            result: if success {
                LearningResult::success(OutcomeMetrics { steps, ..Default::default() })
            } else {
                LearningResult::failed(
                    OutcomeMetrics { steps, ..Default::default() }, vec![])
            },
            confidence: 0.7,
            lesson: None,
            trajectory: None,
        }
    }

    #[test]
    fn test_budget_index_records_and_queries() {
        let mut idx = BudgetAwareIndex::new();
        // Strategy A: high success, high cost
        for _ in 0..3 {
            idx.update_from_experience(&make_exp(
                StrategyKind::InspectThenImplement, "rust", 80, true));
        }
        // Strategy B: slightly lower success, low cost
        for _ in 0..3 {
            idx.update_from_experience(&make_exp(
                StrategyKind::DirectImplementation, "rust", 20, true));
        }

        // Ample budget: should prefer InspectThenImplement (higher success)
        let choice_ample = idx.best_strategy_for_budget(Some("rust"), 300, 1);
        assert!(choice_ample.is_some());
        assert!(choice_ample.unwrap().budget_ratio >= 2.0,
            "With ample budget, budget_ratio must be >= 2.0");

        // Tight budget: efficiency should win — DirectImplementation (cheaper)
        let choice_tight = idx.best_strategy_for_budget(Some("rust"), 15, 1).unwrap();
        assert_eq!(choice_tight.strategy, StrategyKind::DirectImplementation,
            "With tight budget, cheaper strategy must win");
        assert!(choice_tight.budget_ratio < 1.0,
            "budget_ratio must be < 1.0 when budget < expected cost");
    }

    #[test]
    fn test_budget_index_fallback_to_any_language() {
        let mut idx = BudgetAwareIndex::new();
        idx.update_from_experience(&make_exp(
            StrategyKind::MinimalChange, "python", 15, true));

        // Query for "rust" — no rust data, but any-language profile exists
        let choice = idx.best_strategy_for_budget(Some("rust"), 100, 1);
        assert!(choice.is_some(), "Must fall back to any-language profile");
    }

    #[test]
    fn test_budget_index_returns_none_below_min_attempts() {
        let mut idx = BudgetAwareIndex::new();
        idx.update_from_experience(&make_exp(
            StrategyKind::CompileFirst, "rust", 30, true));

        let choice = idx.best_strategy_for_budget(Some("rust"), 100, 5);
        assert!(choice.is_none(), "Must return None when below min_attempts");
    }

    #[test]
    fn test_budget_profile_p75_estimation() {
        let mut profile = BudgetProfile::new(StrategyKind::IncrementalPatch, Some("rust".to_string()));
        for steps in [10, 20, 30, 40, 50, 60, 70, 80] {
            profile.record(steps, &LearningOutcome::Success);
        }
        let p75 = profile.expected_cost_steps();
        // 8 samples, sorted: [10,20,30,40,50,60,70,80], p75 idx=6 → 70
        assert_eq!(p75, 70, "P75 must be 70 for this distribution");
    }
}
