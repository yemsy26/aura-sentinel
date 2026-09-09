use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::core::learning::experience::Experience;
use crate::core::learning::outcome::LearningOutcome;
use crate::core::learning::strategy::StrategyKind;

/// Per-model performance statistics for adaptive routing.
/// Migrates and supersedes llm/router.rs::ModelStats.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelStats {
    pub model: String,
    /// Fingerprint language dimension for segmentation
    pub language: Option<String>,
    pub attempts: u64,
    pub successes: u64,       // LearningOutcome::Success
    pub partial: u64,         // LearningOutcome::PartialSuccess
    pub failures: u64,
    pub total_steps: u64,
    pub total_latency_ms: u64,
    pub tool_failures: u64,
    pub compile_failures: u64,
    pub test_failures: u64,
    pub recoveries: u64,
}

impl ModelStats {
    /// Bayesian smoothed success rate: (s + 1) / (n + 2)
    /// Partial counts as 0.5. Returns 0.5 for zero attempts (neutral prior).
    pub fn smoothed_success_rate(&self) -> f32 {
        let effective = self.successes as f32 + self.partial as f32 * 0.5 + 1.0;
        (effective / (self.attempts as f32 + 2.0)).clamp(0.0, 1.0)
    }

    pub fn average_steps(&self) -> f32 {
        if self.attempts == 0 { return 10.0; }
        self.total_steps as f32 / self.attempts as f32
    }

    pub fn average_latency_ms(&self) -> f32 {
        if self.attempts == 0 { return 0.0; }
        self.total_latency_ms as f32 / self.attempts as f32
    }

    /// Efficiency: lower step count relative to baseline (20 steps) = better
    fn efficiency(&self) -> f32 {
        let avg = self.average_steps();
        (1.0 - (avg / 20.0).min(1.0)).clamp(0.0, 1.0)
    }

    /// Failure penalty: compile + test failures normalized to attempts
    fn failure_penalty(&self) -> f32 {
        if self.attempts == 0 { return 0.0; }
        let rate = (self.compile_failures + self.test_failures) as f32 / self.attempts as f32;
        rate.min(1.0)
    }

    /// Composite confidence score 0.0..1.0
    /// 0.45*sr + 0.20*efficiency + 0.20*(1-failure_penalty) + 0.15*recovery_ability
    pub fn confidence(&self) -> f32 {
        let sr = self.smoothed_success_rate();
        let eff = self.efficiency();
        let no_fail = 1.0 - self.failure_penalty();
        let recovery = if self.recoveries > 0 { 0.7f32 } else { 0.5 };
        (0.45 * sr + 0.20 * eff + 0.20 * no_fail + 0.15 * recovery).clamp(0.0, 1.0)
    }

    /// Sample weight for confidence intervals: 0.0 at 0 samples, 1.0 at 20+ samples
    pub fn sample_weight(&self) -> f32 {
        (self.attempts as f32 / 20.0).min(1.0)
    }
}

/// Per-strategy performance statistics.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StrategyStats {
    pub strategy: String,  // StrategyKind::as_str()
    pub language: Option<String>,
    pub attempts: u64,
    pub successes: u64,
    pub partial: u64,
    pub failures: u64,
    pub total_steps: u64,
    pub recoveries: u64,
    pub successful_recoveries: u64,
}

impl StrategyStats {
    pub fn smoothed_success_rate(&self) -> f32 {
        let effective = self.successes as f32 + self.partial as f32 * 0.5 + 1.0;
        (effective / (self.attempts as f32 + 2.0)).clamp(0.0, 1.0)
    }

    pub fn recovery_success_rate(&self) -> f32 {
        if self.recoveries == 0 { return 0.5; }
        self.successful_recoveries as f32 / self.recoveries as f32
    }
}

// ─── Compute from experience slices ──────────────────────────────────────────

pub fn compute_model_stats_from(
    model: &str,
    language: Option<&str>,
    exps: &[&Experience],
) -> ModelStats {
    let mut s = ModelStats {
        model: model.to_string(),
        language: language.map(|s| s.to_string()),
        ..Default::default()
    };
    for e in exps {
        if e.model != model { continue; }
        s.attempts += 1;
        match e.result.outcome {
            LearningOutcome::Success => s.successes += 1,
            LearningOutcome::PartialSuccess => s.partial += 1,
            _ => s.failures += 1,
        }
        s.total_steps += e.result.metrics.steps as u64;
        s.tool_failures += e.result.metrics.failed_actions as u64;
        s.compile_failures += e.result.failures.iter()
            .filter(|f| f.class == "Compile").count() as u64;
        s.test_failures += e.result.failures.iter()
            .filter(|f| f.class == "Test").count() as u64;
        if e.result.recovery.is_some() { s.recoveries += 1; }
    }
    s
}

pub fn compute_strategy_stats_from(
    strategy: &StrategyKind,
    language: Option<&str>,
    exps: &[&Experience],
) -> StrategyStats {
    let strat_str = strategy.as_str();
    let mut s = StrategyStats {
        strategy: strat_str.to_string(),
        language: language.map(|l| l.to_string()),
        ..Default::default()
    };
    for e in exps {
        if e.strategy.as_str() != strat_str { continue; }
        s.attempts += 1;
        match e.result.outcome {
            LearningOutcome::Success => s.successes += 1,
            LearningOutcome::PartialSuccess => s.partial += 1,
            _ => s.failures += 1,
        }
        s.total_steps += e.result.metrics.steps as u64;
        if let Some(rec) = &e.result.recovery {
            s.recoveries += 1;
            if rec.succeeded { s.successful_recoveries += 1; }
        }
    }
    s
}

/// Build maps of stats indexed by model name / strategy name from all experiences
pub fn build_model_stats_map(exps: &[Experience]) -> HashMap<String, ModelStats> {
    let mut map: HashMap<String, ModelStats> = HashMap::new();
    let refs: Vec<&Experience> = exps.iter().collect();
    let models: std::collections::HashSet<&str> = refs.iter().map(|e| e.model.as_str()).collect();
    for model in models {
        let lang = refs.iter().find(|e| e.model == model)
            .and_then(|e| e.fingerprint.language.as_deref());
        let stats = compute_model_stats_from(model, lang, &refs);
        map.insert(model.to_string(), stats);
    }
    map
}

pub fn build_strategy_stats_map(exps: &[Experience]) -> HashMap<String, StrategyStats> {
    let mut map: HashMap<String, StrategyStats> = HashMap::new();
    let refs: Vec<&Experience> = exps.iter().collect();
    let strategies: std::collections::HashSet<String> =
        refs.iter().map(|e| e.strategy.as_str().to_string()).collect();
    for strat_str in strategies {
        let strat = StrategyKind::from_str(&strat_str);
        let lang = refs.iter().find(|e| e.strategy.as_str() == strat_str)
            .and_then(|e| e.fingerprint.language.as_deref());
        let stats = compute_strategy_stats_from(&strat, lang, &refs);
        map.insert(strat_str, stats);
    }
    map
}
