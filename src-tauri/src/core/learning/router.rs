use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use crate::core::learning::experience::{Experience, SharedExperienceStore};
use crate::core::learning::fingerprint::{TaskFingerprint, FingerprintBuilder};
use crate::core::learning::stats::{ModelStats, StrategyStats};
use crate::core::learning::strategy::StrategyKind;
use crate::core::learning::signature::StateSignature;
use crate::core::learning::state_stats::StateStrategyIndex;

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
    /// AL-v2.3 — true if StateSignature data influenced strategy selection
    pub state_informed: bool,
}

/// AdaptiveRouter — the ONLY interface between Learning and the LLM selection layer.
///
/// INVARIANT: AdaptiveRouter has NO access to MissionRuntime, ToolRegistry,
/// PolicyEngine, or CompletionGate. It cannot execute tools or declare completion.
pub struct AdaptiveRouter {
    model_stats: HashMap<String, ModelStats>,
    strategy_stats: HashMap<String, StrategyStats>,
    store: SharedExperienceStore,
    /// AL-v2.3 — StateSignature-indexed strategy performance cache
    state_index: StateStrategyIndex,
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
        Self {
            model_stats, strategy_stats, store,
            state_index: StateStrategyIndex::new(),
            exploration_seed: hasher.finish(),
        }
    }

    /// AL-v2.3 — construct with a pre-loaded StateStrategyIndex.
    #[allow(dead_code)]
    pub fn with_state_index(mut self, index: StateStrategyIndex) -> Self {
        self.state_index = index;
        self
    }

    /// Produce a recommendation for the given fingerprint and available models.
    /// NEVER executes. NEVER writes to ExperienceStore.
    /// Delegates to recommend_with_state with no state override (backward-compatible).
    pub async fn recommend(
        &self,
        fp: &TaskFingerprint,
        available_models: &[String],
    ) -> Recommendation {
        self.recommend_with_state(fp, None, available_models).await
    }

    /// AL-v2.3 — recommend with optional StateSignature context.
    /// When `state` is Some, blends state-indexed strategy score with fingerprint score.
    /// When `state` is None, behaves identically to the original recommend().
    pub async fn recommend_with_state(
        &self,
        fp: &TaskFingerprint,
        state: Option<&StateSignature>,
        available_models: &[String],
    ) -> Recommendation {
        if available_models.is_empty() {
            return self.cold_start(fp, None);
        }

        let store_guard = self.store.read().await;
        // Gather similar experiences from the shared store
        let similar = store_guard.find_similar(fp, 20);


        if similar.is_empty() && store_guard.len() < 3 {
            // AL-v2.3: even on cold-start, consult the state index if a StateSignature is provided
            if let Some(sig) = state {
                if let Some((state_strat, _rate)) = self.state_index.best_strategy_for_state(sig, 2) {
                    return Recommendation {
                        model: available_models.first().cloned()
                            .unwrap_or_else(|| "default".to_string()),
                        strategy: state_strat,
                        confidence: 0.5,
                        reason: RecommendationReason::ColdStart,
                        fallback_model: None,
                        fallback_strategy: None,
                        state_informed: true,
                    };
                }
            }
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
                state_informed: false,
            };
        }

        // Score each available model using contextual & global weighting
        let (best_model, model_confidence) = self.best_model(fp, available_models, &similar);
        let fp_strategy = self.best_strategy(fp, &similar);

        // AL-v2.3 — blend state-indexed strategy when state data is available
        let (final_strategy, state_informed) = if let Some(sig) = state {
            if let Some((state_strat, state_score)) = self.state_index.best_strategy_for_state(sig, 2) {
                // Compute fingerprint strategy score for comparison
                let fp_score = self.strategy_score_for(&fp_strategy, fp, &similar);
                // Blend: 60% state evidence + 40% fingerprint evidence
                let state_weighted = state_score * 0.6;
                let fp_weighted = fp_score * 0.4;
                if state_weighted + fp_weighted > fp_score {
                    (state_strat, true)
                } else {
                    (fp_strategy, false)
                }
            } else {
                (fp_strategy, false)
            }
        } else {
            (fp_strategy, false)
        };

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
            strategy: final_strategy,
            confidence,
            reason,
            fallback_model: available_models.iter()
                .find(|m| **m != best_model).cloned(),
            fallback_strategy: Some(StrategyKind::default_for(fp)),
            state_informed,
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
            state_informed: false,
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

    /// Returns the smoothed success rate for a given strategy+fingerprint combo.
    /// Used as baseline for state-vs-fingerprint blending in AL-v2.3.
    fn strategy_score_for(&self, strat: &StrategyKind, fp: &TaskFingerprint, similar: &[&Experience]) -> f32 {
        let lang = fp.language.as_deref();
        if !similar.is_empty() {
            use crate::core::learning::stats::compute_strategy_stats_from;
            let stats = compute_strategy_stats_from(strat, lang, similar);
            if stats.attempts > 0 {
                return stats.smoothed_success_rate();
            }
        }
        self.strategy_stats.get(strat.as_str())
            .map(|s| s.smoothed_success_rate())
            .unwrap_or(0.50)
    }
}
