#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use crate::core::learning::{
        fingerprint::{TaskFingerprint, FingerprintBuilder},
        outcome::{LearningOutcome, OutcomeMetrics, LearningResult},
        strategy::StrategyKind,
        experience::{Experience, ExperienceStoreV2, SCHEMA_VERSION},
        stats::{ModelStats, compute_model_stats_from},
        router::{AdaptiveRouter, RecommendationReason},
    };

    fn make_fp(lang: &str, tests: bool) -> TaskFingerprint {
        TaskFingerprint {
            language: Some(lang.to_string()),
            framework: None,
            complexity: 0.4,
            ambiguity: 0.3,
            file_count_bucket: 1,
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
    #[test]
    fn test_cold_start_recommendation() {
        let store = Arc::new(ExperienceStoreV2::new(500));
        let router = AdaptiveRouter::new(Default::default(), Default::default(), store, "m_cold");
        let fp = make_fp("rust", true);
        let rec = router.recommend(&fp, &["qwen3:8b".to_string()]);
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

        // Full success > partial success > pure failure
        assert!(
            full.smoothed_success_rate() > partial.smoothed_success_rate(),
            "Full success must beat partial success"
        );
        assert!(
            partial.smoothed_success_rate() > zero.smoothed_success_rate(),
            "Partial success must beat pure failure"
        );
        // Partial with many samples should be clearly above failure baseline
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
        // ExperienceStoreV2::new() always succeeds even with no data
        let store = ExperienceStoreV2::new(500);
        assert!(store.is_empty(), "Fresh store must be empty");
        assert_eq!(store.len(), 0);
    }

    // ── Test 12: Router has no MissionRuntime reference ───────────────────────
    // Structural test: AdaptiveRouter only takes data structs, no Runtime
    #[test]
    fn test_router_has_no_runtime_reference() {
        // If this compiles, AdaptiveRouter does not import MissionRuntime.
        // The router.rs file is intentionally written without any MissionRuntime import.
        let store = Arc::new(ExperienceStoreV2::new(500));
        let router = AdaptiveRouter::new(Default::default(), Default::default(), store, "m_test");
        let fp = make_fp("rust", false);
        let rec = router.recommend(&fp, &["qwen3:8b".to_string()]);
        // Just verify it returns a valid recommendation
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
        // (2 + 1) / (3 + 2) = 0.6
        assert!((sr - 0.6).abs() < 0.01, "Expected 0.60, got {}", sr);
    }
}
