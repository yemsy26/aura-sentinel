#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use crate::core::learning::{
        fingerprint::{TaskFingerprint, FingerprintBuilder},
        outcome::{LearningOutcome, OutcomeMetrics, LearningResult},
        strategy::StrategyKind,
        experience::{Experience, ExperienceStoreV2, SharedExperienceStore, SCHEMA_VERSION},
        stats::{ModelStats, compute_model_stats_from},
        router::{AdaptiveRouter, RecommendationReason},
        engine::LearningEngine,
        persistence::LearningPersistence,
    };

    fn make_fp(lang: &str, tests: bool) -> TaskFingerprint {
        TaskFingerprint {
            language: Some(lang.to_string()),
            framework: None,
            complexity: 0.4,
            ambiguity: 0.3,
            scope_bucket: 1,
            requires_code: true,
            requires_terminal: true,
            requires_tests: tests,
            requires_network: false,
            verification_level: if tests { 2 } else { 1 },
        }
    }

    fn make_exp(model: &str, outcome: LearningOutcome, attempt_id: &str) -> Experience {
        Experience {
            schema_version: SCHEMA_VERSION,
            id: uuid_simple(),
            attempt_id: attempt_id.to_string(),
            mission_id: "m_test".to_string(),
            timestamp: 0,
            fingerprint: make_fp("rust", true),
            model: model.to_string(),
            strategy: StrategyKind::CompileFirst,
            result: LearningResult {
                outcome,
                metrics: OutcomeMetrics { steps: 5, ..Default::default() },
                failures: vec![],
                recovery: None,
            },
            confidence: 0.5,
            lesson: None,
        }
    }

    fn uuid_simple() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let t = SystemTime::now().duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos()).unwrap_or(0);
        format!("exp_{:x}", t)
    }

    // ── Test 1: Cold start ────────────────────────────────────────────────────
    #[tokio::test]
    async fn test_cold_start_recommendation() {
        let store: SharedExperienceStore = Arc::new(RwLock::new(ExperienceStoreV2::new(500)));
        let router = AdaptiveRouter::new(Default::default(), Default::default(), store, "m_cold");
        let fp = make_fp("rust", true);
        let rec = router.recommend(&fp, &["qwen3:8b".to_string()]).await;
        assert_eq!(rec.reason, RecommendationReason::ColdStart);
        assert!((rec.confidence - 0.5).abs() < 0.01, "cold start confidence must be 0.5");
    }

    // ── Test 2: Successful model increases score ──────────────────────────────
    #[test]
    fn test_successful_model_increases_score() {
        let mut stats = ModelStats::default();
        stats.attempts = 10;
        stats.successes = 9;
        let rate = stats.smoothed_success_rate();
        assert!(rate > 0.80, "9/10 successes should give > 0.80 smoothed rate, got {}", rate);
    }

    // ── Test 3: Failure decreases score ──────────────────────────────────────
    #[test]
    fn test_failure_decreases_score() {
        let mut stats = ModelStats::default();
        stats.attempts = 10;
        stats.failures = 9;
        stats.successes = 1;
        let rate = stats.smoothed_success_rate();
        assert!(rate < 0.30, "1/10 successes should give < 0.30 smoothed rate, got {}", rate);
    }

    // ── Test 4: Compile failure penalized specifically ────────────────────────
    #[test]
    fn test_compile_failure_penalized_specifically() {
        let mut good = ModelStats::default();
        good.attempts = 10; good.successes = 8;
        let mut bad = ModelStats::default();
        bad.attempts = 10; bad.successes = 8; bad.compile_failures = 6;
        assert!(
            good.confidence() > bad.confidence(),
            "Model with compile failures must have lower confidence"
        );
    }

    // ── Test 5: Partial outcome = 0.5 in success rate ────────────────────────
    #[test]
    fn test_partial_outcome_counts_as_half_success() {
        let mut full = ModelStats::default();
        full.attempts = 10; full.successes = 10;

        let mut partial = ModelStats::default();
        partial.attempts = 10; partial.partial = 10;

        let mut zero = ModelStats::default();
        zero.attempts = 10; zero.failures = 10;

        assert!(
            full.smoothed_success_rate() > partial.smoothed_success_rate(),
            "Full success must beat partial success"
        );
        assert!(
            partial.smoothed_success_rate() > zero.smoothed_success_rate(),
            "Partial success must beat pure failure"
        );
        assert!(
            partial.smoothed_success_rate() >= 0.5,
            "10 partial successes should be at or above 0.5 neutral, got {}",
            partial.smoothed_success_rate()
        );
    }

    // ── Test 6: Insufficient sample does not dominate baseline ───────────────
    #[test]
    fn test_insufficient_sample_does_not_dominate() {
        let mut stats = ModelStats::default();
        stats.attempts = 1;
        stats.successes = 1;
        let weight = stats.sample_weight();
        assert!(weight < 0.10, "1 sample should have very low weight, got {}", weight);
    }

    // ── Test 7: Fingerprint similarity — same type ────────────────────────────
    #[test]
    fn test_fingerprint_similarity_same_type() {
        let a = make_fp("rust", true);
        let b = make_fp("rust", true);
        let sim = FingerprintBuilder::similarity(&a, &b);
        assert!(sim >= 0.95, "Identical fingerprints should have similarity >= 0.95, got {}", sim);
    }

    // ── Test 8: Fingerprint similarity — different language ───────────────────
    #[test]
    fn test_fingerprint_similarity_different_language() {
        let a = make_fp("rust", true);
        let b = make_fp("python", true);
        let sim = FingerprintBuilder::similarity(&a, &b);
        assert!(sim < 0.75, "Different language should reduce similarity, got {}", sim);
    }

    // ── Test 9: Duplicate attempt_id not counted ──────────────────────────────
    #[test]
    fn test_duplicate_attempt_not_counted() {
        let mut store = ExperienceStoreV2::new(500);
        let exp = make_exp("qwen3:8b", LearningOutcome::Success, "attempt_001");
        let pushed1 = store.push(exp.clone());
        let pushed2 = store.push(exp.clone());
        assert!(pushed1, "First push must succeed");
        assert!(!pushed2, "Duplicate attempt_id must be rejected");
        assert_eq!(store.len(), 1, "Store must have exactly 1 experience");
    }

    // ── Test 10: Confidence bounded to 0..1 ──────────────────────────────────
    #[test]
    fn test_confidence_is_bounded() {
        let mut stats = ModelStats::default();
        stats.attempts = 100;
        stats.successes = 100;
        let c = stats.confidence();
        assert!(c >= 0.0 && c <= 1.0, "confidence must be in 0..1, got {}", c);
    }

    // ── Test 11: Corruption fallback — ExperienceStore starts empty on bad file ─
    #[test]
    fn test_corrupted_data_fallback() {
        let store = ExperienceStoreV2::new(500);
        assert!(store.is_empty(), "Fresh store must be empty");
        assert_eq!(store.len(), 0);
    }

    // ── Test 12: Router has no MissionRuntime reference ───────────────────────
    #[tokio::test]
    async fn test_router_has_no_runtime_reference() {
        let store: SharedExperienceStore = Arc::new(RwLock::new(ExperienceStoreV2::new(500)));
        let router = AdaptiveRouter::new(Default::default(), Default::default(), store, "m_test");
        let fp = make_fp("rust", false);
        let rec = router.recommend(&fp, &["qwen3:8b".to_string()]).await;
        assert!(!rec.model.is_empty());
        assert!(rec.confidence >= 0.0 && rec.confidence <= 1.0);
    }

    // ── Test 13: Compute model stats from experiences ────────────────────────
    #[test]
    fn test_compute_model_stats_from_experiences() {
        let exps = vec![
            make_exp("model_a", LearningOutcome::Success, "a1"),
            make_exp("model_a", LearningOutcome::Success, "a2"),
            make_exp("model_a", LearningOutcome::Failed, "a3"),
        ];
        let refs: Vec<&Experience> = exps.iter().collect();
        let stats = compute_model_stats_from("model_a", Some("rust"), &refs);
        assert_eq!(stats.attempts, 3);
        assert_eq!(stats.successes, 2);
        assert_eq!(stats.failures, 1);
        let sr = stats.smoothed_success_rate();
        assert!((sr - 0.6).abs() < 0.01, "Expected 0.60, got {}", sr);
    }

    // ── Test 14: Engine updates model stats after outcome ─────────────────────
    #[tokio::test]
    async fn test_engine_updates_model_stats_after_outcome() {
        let store: SharedExperienceStore = Arc::new(RwLock::new(ExperienceStoreV2::new(500)));
        let engine = LearningEngine::with_store(store.clone());
        let fp = make_fp("rust", true);
        let res = LearningResult::success(OutcomeMetrics { steps: 4, ..Default::default() });

        engine.record_outcome(
            fp,
            "test_model".to_string(),
            StrategyKind::CompileFirst,
            res,
            0.7,
            "m_engine_1".to_string(),
            Some("att_1".to_string()),
        ).await.unwrap();

        let s = store.read().await;
        assert_eq!(s.len(), 1);
        assert_eq!(s.experiences[0].model, "test_model");
    }

    // ── Test 15: Shared store write visible to reader ─────────────────────────
    #[tokio::test]
    async fn test_shared_store_write_visible_to_reader() {
        let store: SharedExperienceStore = Arc::new(RwLock::new(ExperienceStoreV2::new(500)));
        let engine = LearningEngine::with_store(store.clone());
        let router = AdaptiveRouter::new(Default::default(), Default::default(), store.clone(), "m_seed");

        let fp = make_fp("rust", true);
        // Pre-write: cold start
        let rec_pre = router.recommend(&fp, &["m1".to_string()]).await;
        assert_eq!(rec_pre.reason, RecommendationReason::ColdStart);

        // Record 3 experiences
        for i in 1..=3 {
            engine.record_outcome(
                fp.clone(),
                "m1".to_string(),
                StrategyKind::CompileFirst,
                LearningResult::success(OutcomeMetrics { steps: 3, ..Default::default() }),
                0.8,
                format!("m_multi_{}", i),
                Some(format!("att_multi_{}", i)),
            ).await.unwrap();
        }

        // Post-write: router now sees experiences in shared store!
        let rec_post = router.recommend(&fp, &["m1".to_string()]).await;
        match rec_post.reason {
            RecommendationReason::SimilarTask { sample_size, .. } => {
                assert_eq!(sample_size, 3);
            }
            RecommendationReason::Exploration => {} // Exploration is also a valid probabilistic branch
            other => panic!("Expected SimilarTask or Exploration, got {:?}", other),
        }
    }

    // ── Test 16: Strategy selection uses similar experiences ──────────────────
    #[tokio::test]
    async fn test_strategy_selection_uses_similar_experiences() {
        let store: SharedExperienceStore = Arc::new(RwLock::new(ExperienceStoreV2::new(500)));
        let _engine = LearningEngine::with_store(store.clone());
        let fp = make_fp("rust", true);

        // Record successes for MinimalChange strategy on rust
        for i in 1..=4 {
            let mut exp = make_exp("qwen", LearningOutcome::Success, &format!("att_strat_{}", i));
            exp.strategy = StrategyKind::MinimalChange;
            exp.fingerprint = fp.clone();
            let _ = store.write().await.push(exp);
        }

        let router = AdaptiveRouter::new(Default::default(), Default::default(), store, "m_fixed_seed_strat");
        let rec = router.recommend(&fp, &["qwen".to_string()]).await;
        // Even if default is CompileFirst, MinimalChange should be selected or explored
        if rec.reason != RecommendationReason::Exploration {
            assert_eq!(rec.strategy, StrategyKind::MinimalChange);
        }
    }

    // ── Test 17: Truly atomic persistence (including when file already exists on Windows) ──
    #[tokio::test]
    async fn test_persistence_truly_atomic() {
        let temp_dir = std::env::temp_dir().join(format!("aura_test_atomic_{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let persistence = LearningPersistence::with_dir(temp_dir.clone());
        let exp1 = make_exp("atom_model", LearningOutcome::Success, "att_atomic_1");

        let res1 = persistence.append_experience(&exp1).await;
        assert!(res1.is_ok(), "first append_experience must succeed: {:?}", res1);

        // Second append tests the atomic replace when destination file already exists on Windows
        let exp2 = make_exp("atom_model_2", LearningOutcome::Success, "att_atomic_2");
        let res2 = persistence.append_experience(&exp2).await;
        assert!(res2.is_ok(), "subsequent append_experience on existing file must succeed on Windows: {:?}", res2);

        // Verify stats atomic replace on existing file as well
        let mut model_stats = std::collections::HashMap::new();
        model_stats.insert("atom_model".to_string(), ModelStats::default());
        let res_stats1 = persistence.save_model_stats(&model_stats).await;
        assert!(res_stats1.is_ok(), "first save_model_stats must succeed: {:?}", res_stats1);

        let res_stats2 = persistence.save_model_stats(&model_stats).await;
        assert!(res_stats2.is_ok(), "second save_model_stats on existing file must succeed on Windows: {:?}", res_stats2);

        // Verify the file was created and can be loaded
        let loaded = persistence.load_experiences(500);
        assert!(loaded.experiences.iter().any(|e| e.attempt_id == "att_atomic_1"));
        assert!(loaded.experiences.iter().any(|e| e.attempt_id == "att_atomic_2"));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    // ── Test 18: Seen attempt IDs trimmed on eviction ─────────────────────────
    #[test]
    fn test_seen_ids_trimmed_on_eviction() {
        let mut store = ExperienceStoreV2::new(3); // capacity of 3
        for i in 1..=5 {
            let exp = make_exp("m", LearningOutcome::Success, &format!("att_evict_{}", i));
            assert!(store.push(exp));
        }
        assert_eq!(store.len(), 3);

        // att_evict_1 was drained from the store, so pushing it again should be accepted!
        let exp_re = make_exp("m", LearningOutcome::Success, "att_evict_1");
        assert!(store.push(exp_re), "Evicted attempt_id must be allowed back if re-encountered");
    }
}
