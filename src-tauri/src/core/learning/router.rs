use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use crate::core::learning::experience::{Experience, SharedExperienceStore};
use crate::core::learning::fingerprint::{TaskFingerprint, FingerprintBuilder};
use crate::core::learning::stats::{ModelStats, StrategyStats};
use crate::core::learning::strategy::StrategyKind;

/// Why this recommendation was produced.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum RecommendationReason {
    ColdStart,
    GlobalHistory { sample_size: u32 },
    SimilarTask { sample_size: u32, avg_similarity: f32 },
    Exploration,
    Fallback,
}

/// A model + strategy recommendation for the next mission.
/// Runtime v4 decides whether to accept it — this is advisory only.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Recommendation {
    pub model: String,
    pub strategy: StrategyKind,
    /// 0.0..1.0 — how confident the router is in this recommendation
    pub confidence: f32,
    pub reason: RecommendationReason,
    pub fallback_model: Option<String>,
    pub fallback_strategy: Option<StrategyKind>,
}

/// AdaptiveRouter — the ONLY interface between Learning and the LLM selection layer.
///
/// INVARIANT: AdaptiveRouter has NO access to MissionRuntime, ToolRegistry,
/// PolicyEngine, or CompletionGate. It cannot execute tools or declare completion.
pub struct AdaptiveRouter {
    model_stats: HashMap<String, ModelStats>,
    strategy_stats: HashMap<String, StrategyStats>,
    store: SharedExperienceStore,
    /// Deterministic exploration seed from hash(mission_id)
    exploration_seed: u64,
}

impl AdaptiveRouter {
    pub fn new(
        model_stats: HashMap<String, ModelStats>,
        strategy_stats: HashMap<String, StrategyStats>,
        store: SharedExperienceStore,
        mission_id: &str,
    ) -> Self {
        let mut hasher = DefaultHasher::new();
        mission_id.hash(&mut hasher);
        Self { model_stats, strategy_stats, store, exploration_seed: hasher.finish() }
    }

    /// Produce a recommendation for the given fingerprint and available models.
    /// NEVER executes. NEVER writes to ExperienceStore.
    pub async fn recommend(
        &self,
        fp: &TaskFingerprint,
        available_models: &[String],
    ) -> Recommendation {
        if available_models.is_empty() {
            return self.cold_start(fp, None);
        }

        let store_guard = self.store.read().await;
        // Gather similar experiences from the shared store
        let similar = store_guard.find_similar(fp, 20);

        if similar.is_empty() && store_guard.len() < 3 {
            return self.cold_start(fp, available_models.first().cloned());
        }

        // Decide: explore or exploit?
        let explore = self.should_explore();

        if explore {
            // Deterministic exploration: pick model by seed index
            let idx = (self.exploration_seed % available_models.len() as u64) as usize;
            let model = available_models[idx].clone();
            let strategy = StrategyKind::default_for(fp);
            return Recommendation {
                model,
                strategy,
                confidence: 0.4,
                reason: RecommendationReason::Exploration,
                fallback_model: available_models.first().cloned(),
                fallback_strategy: None,
            };
        }

        // Score each available model using contextual & global weighting
        let (best_model, model_confidence) = self.best_model(fp, available_models, &similar);
        let best_strategy = self.best_strategy(fp, &similar);

        // Determine reason and sample depth
        let reason = if similar.is_empty() {
            RecommendationReason::GlobalHistory { sample_size: store_guard.len() as u32 }
        } else {
            let avg_similarity = similar.iter()
                .map(|e| FingerprintBuilder::similarity(fp, &e.fingerprint))
                .sum::<f32>() / (similar.len() as f32);
            RecommendationReason::SimilarTask {
                sample_size: similar.len() as u32,
                avg_similarity,
            }
        };

        // Blend model confidence with sample weight
        let stats = self.model_stats.get(&best_model);
        let sample_weight = stats.map(|s| s.sample_weight()).unwrap_or(0.3);
        let confidence = (model_confidence * sample_weight
            + 0.5 * (1.0 - sample_weight)).clamp(0.0, 1.0);

        Recommendation {
            model: best_model.clone(),
            strategy: best_strategy,
            confidence,
            reason,
            fallback_model: available_models.iter()
                .find(|m| **m != best_model).cloned(),
            fallback_strategy: Some(StrategyKind::default_for(fp)),
        }
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    fn cold_start(&self, fp: &TaskFingerprint, model: Option<String>) -> Recommendation {
        Recommendation {
            model: model.unwrap_or_else(|| "default".to_string()),
            strategy: StrategyKind::default_for(fp),
            confidence: 0.5,
            reason: RecommendationReason::ColdStart,
            fallback_model: None,
            fallback_strategy: None,
        }
    }

    fn should_explore(&self) -> bool {
        let total: u64 = self.model_stats.values().map(|s| s.attempts).sum();
        let exploration_rate = if total > 50 { 5u64 }
            else if total > 10 { 10 }
            else { 15 };
        (self.exploration_seed % 100) < exploration_rate
    }

    fn best_model(&self, fp: &TaskFingerprint, available: &[String], similar: &[&Experience]) -> (String, f32) {
        let lang = fp.language.as_deref();
        let mut best_model = available[0].clone();
        let mut best_score = -1.0f32;

        for model in available {
            let global_score = if let Some(stats) = self.model_stats.get(model) {
                if stats.language.as_deref() == lang {
                    stats.confidence()
                } else {
                    stats.confidence() * 0.8
                }
            } else {
                0.5
            };

            let similar_refs: Vec<&Experience> = similar.iter()
                .filter(|e| e.model == *model).copied().collect();

            let contextual_score = if similar_refs.is_empty() {
                0.5
            } else {
                use crate::core::learning::stats::compute_model_stats_from;
                compute_model_stats_from(model, lang, &similar_refs).confidence()
            };

            // 45% global + 55% contextual task similarity
            let score = 0.45 * global_score + 0.55 * contextual_score;

            if score > best_score {
                best_score = score;
                best_model = model.clone();
            }
        }
        (best_model, best_score.max(0.5))
    }

    fn best_strategy(&self, fp: &TaskFingerprint, similar: &[&Experience]) -> StrategyKind {
        let lang = fp.language.as_deref();
        let strategies = [
            StrategyKind::DirectImplementation,
            StrategyKind::InspectThenImplement,
            StrategyKind::TestFirst,
            StrategyKind::CompileFirst,
            StrategyKind::IncrementalPatch,
            StrategyKind::DiagnoseThenRepair,
            StrategyKind::MinimalChange,
        ];

        let mut best_strat = StrategyKind::default_for(fp);
        let mut best_score = -1.0f32;

        for strat in &strategies {
            let score = if !similar.is_empty() {
                use crate::core::learning::stats::compute_strategy_stats_from;
                let stats = compute_strategy_stats_from(strat, lang, similar);
                if stats.attempts > 0 {
                    stats.smoothed_success_rate()
                } else {
                    self.strategy_stats.get(strat.as_str())
                        .map(|s| s.smoothed_success_rate())
                        .unwrap_or_else(|| if *strat == StrategyKind::default_for(fp) { 0.55 } else { 0.50 })
                }
            } else {
                self.strategy_stats.get(strat.as_str())
                    .map(|s| s.smoothed_success_rate())
                    .unwrap_or_else(|| if *strat == StrategyKind::default_for(fp) { 0.55 } else { 0.50 })
            };

            if score > best_score {
                best_score = score;
                best_strat = strat.clone();
            }
        }
        best_strat
    }
}
