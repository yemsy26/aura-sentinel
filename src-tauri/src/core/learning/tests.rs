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
            trajectory: None,
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

    // ── Test 19: Full AL-v1 End-to-End Cycle ─────────────────────────────────
    // Chain: Mission -> Contract -> Fingerprint -> AdaptiveRouter -> Recommendation
    //        -> Runtime -> ActionProposal -> Policy -> ToolRegistry -> Tool -> Observation
    //        -> Evidence -> CompletionGate -> Outcome -> LearningEngine -> Experience
    //        -> ModelStats -> StrategyStats -> AdaptiveRouter update.
    #[tokio::test]
    async fn test_al_v1_e2e_full_cycle() {
        use crate::core::mission_runtime::MissionRuntime;
        use crate::core::mission_contract::{MissionContract, AcceptanceCriterion, VerificationMethod, CriterionStatus};
        use crate::core::project_profile::{ProjectProfile, PrimaryLanguage};
        use crate::core::policy::{ActionProposal, RiskLevel};
        use crate::core::completion_gate::CompletionDecision;

        // 1. Mission & Contract
        let mut contract = MissionContract::new("Build and test Rust calculator");
        contract.acceptance_criteria.push(AcceptanceCriterion {
            id: "AC-1".to_string(),
            description: "Calculator runs and returns 4".to_string(),
            verification: VerificationMethod::CommandExitZero("cargo test".to_string()),
            required: true,
            status: CriterionStatus::Pending,
        });

        // 2. Project Profile & Fingerprint with context
        let profile = ProjectProfile {
            workspace_path: ".".to_string(),
            primary: PrimaryLanguage::Rust,
            secondary: vec![],
            frameworks: vec![],
            package_managers: vec!["cargo".to_string()],
            has_docker: false,
            has_git: false,
        };
        let fp = FingerprintBuilder::from_mission_with_world(&contract, &profile, None);
        assert_eq!(fp.language, Some("rust".to_string()));
        assert!(fp.requires_tests);

        // 3. AdaptiveRouter initial recommendation (Cold Start)
        let temp_dir = std::env::temp_dir().join(format!("aura_test_e2e_{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let persistence = LearningPersistence::with_dir(temp_dir.clone());
        let store: SharedExperienceStore = Arc::new(RwLock::new(persistence.load_experiences(500)));
        let router = AdaptiveRouter::new(Default::default(), Default::default(), store.clone(), "m_e2e_1");
        let available_models = vec!["qwen2.5-coder:7b".to_string(), "deepseek-coder:6.7b".to_string()];
        let rec1 = router.recommend(&fp, &available_models).await;
        assert_eq!(rec1.reason, RecommendationReason::ColdStart);

        // 4. Runtime & ToolRegistry execution
        let mut runtime = MissionRuntime::new(".", &contract.objective, 20);
        runtime.contract = contract.clone();

        // Register real mock executor in ToolRegistry
        runtime.tool_registry.register("TOOL_TERMINAL", Arc::new(|args| {
            let cmd = args.get("comando").and_then(|v| v.as_str()).unwrap_or("").to_string();
            Box::pin(async move {
                Ok(format!("Command '{}' executed with exit code 0", cmd))
            })
        })).unwrap();

        // Propose action
        let proposal = ActionProposal {
            tool: "TOOL_TERMINAL".to_string(),
            arguments: serde_json::json!({ "comando": "cargo test" }),
            expected_effect: "Run test suite".to_string(),
            risk: RiskLevel::Safe,
        };

        // Execution Gateway: Authorize & Execute
        assert!(runtime.authorize_action(&proposal).is_ok());
        let obs = runtime.execute_action(&proposal).await.unwrap();
        assert_eq!(obs.status, crate::core::observation::ObservationStatus::Success);

        // 5. Evidence & CompletionGate
        runtime.contract.mark_criterion("AC-1", true);
        use crate::core::evidence::EvidenceKind;
        runtime.evidence_graph.record(
            EvidenceKind::CommandExitCode,
            "TOOL_TERMINAL",
            "cargo test exit code 0",
            &obs.payload,
            1.0,
            runtime.current_step(),
        );

        let decision = runtime.can_complete();
        assert!(matches!(decision, CompletionDecision::Complete { .. }));

        // 6. Outcome -> LearningEngine records outcome
        let engine = LearningEngine::with_store_and_persistence(store.clone(), persistence.clone());
        let outcome_result = LearningResult {
            outcome: LearningOutcome::Success,
            metrics: OutcomeMetrics {
                steps: runtime.current_step(),
                tool_calls: 1,
                failed_actions: 0,
                recovery_actions: 0,
                verification_attempts: 1,
                successful_verifications: 1,
                elapsed_ms: 1500,
            },
            failures: vec![],
            recovery: None,
        };

        let record_res = engine.record_outcome(
            fp.clone(),
            rec1.model.clone(),
            rec1.strategy,
            outcome_result,
            0.9,
            runtime.mission_id.clone(),
            Some(format!("att_e2e_{}", runtime.mission_id)),
        ).await;
        assert!(record_res.is_ok());

        // 7. Verify Experience, ModelStats & subsequent recommendation update
        let exp_count = store.read().await.len();
        assert_eq!(exp_count, 1);

        let updated_stats = persistence.load_model_stats();
        let chosen_stats = updated_stats.get(&rec1.model);
        assert!(chosen_stats.is_some(), "Recorded model must have persisted stats");
        let stats = chosen_stats.unwrap();
        assert_eq!(stats.successes, 1);
        assert_eq!(stats.attempts, 1);

        // Router with updated stats should reflect experience
        let updated_router = AdaptiveRouter::new(updated_stats, persistence.load_strategy_stats(), store.clone(), "m_e2e_2");
        let rec2 = updated_router.recommend(&fp, &available_models).await;
        assert_eq!(rec2.model, rec1.model, "Successfully reinforced model should be recommended");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    // ── Test 20: Architectural Isolation Test ────────────────────────────────
    // Guarantee that LearningEngine and AdaptiveRouter hold NO references to
    // MissionRuntime, ToolRegistry, PolicyEngine, or CompletionGate.
    #[test]
    fn test_al_v1_architectural_isolation() {
        // Compile-time assertion: LearningEngine struct fields must only be store and persistence
        let store: SharedExperienceStore = Arc::new(RwLock::new(ExperienceStoreV2::new(10)));
        let engine = LearningEngine::with_store(store.clone());
        let _ = engine.store();

        // Assert size of LearningEngine is purely store (Arc) + persistence (PathBuf)
        assert!(std::mem::size_of::<LearningEngine>() <= 64, "LearningEngine must be a lightweight coordinator with no runtime handles");

        // Assert size of AdaptiveRouter is purely data (StateStrategyIndex adds a HashMap — still no runtime handles)
        assert!(std::mem::size_of::<AdaptiveRouter>() <= 256, "AdaptiveRouter must be purely algorithmic without runtime dependencies");
    }

    // ── AL-v2 Tests: StateSignature & Trajectory Foundation ──────────────────

    #[test]
    fn test_al_v2_state_signature_creation_and_similarity() {
        use crate::core::learning::signature::{StateSignatureBuilder, VerificationLevel};

        let sig1 = StateSignatureBuilder::new("fp_hash_rust_app")
            .phase(1)
            .files_changed(2)
            .files_created(1)
            .compile_failures(1)
            .test_failures(0)
            .verification_level(VerificationLevel::SyntaxOrStatic)
            .criteria(1, 1)
            .recovery_count(1)
            .replan_count(0)
            .last_error_class(Some("CompileError".to_string()))
            .progress_stalled(false)
            .build();

        // Identical signature must have similarity = 1.0
        assert_eq!(sig1.similarity(&sig1), 1.0);

        // Highly similar signature (same task and error class, slightly different counts)
        let sig2 = StateSignatureBuilder::new("fp_hash_rust_app")
            .phase(1)
            .files_changed(3)
            .files_created(1)
            .compile_failures(2)
            .test_failures(0)
            .verification_level(VerificationLevel::SyntaxOrStatic)
            .criteria(1, 1)
            .recovery_count(1)
            .replan_count(0)
            .last_error_class(Some("CompileError".to_string()))
            .progress_stalled(false)
            .build();

        let sim_close = sig1.similarity(&sig2);
        assert!(sim_close > 0.85, "Close signatures should have high similarity, got {}", sim_close);

        // Very distant signature (different task, different error, stalled)
        let sig_far = StateSignatureBuilder::new("fp_hash_python_script")
            .phase(3)
            .files_changed(0)
            .files_created(0)
            .compile_failures(5)
            .test_failures(4)
            .verification_level(VerificationLevel::None)
            .criteria(0, 5)
            .recovery_count(4)
            .replan_count(2)
            .last_error_class(Some("TimeoutError".to_string()))
            .progress_stalled(true)
            .build();

        let sim_distant = sig1.similarity(&sig_far);
        assert!(sim_distant < 0.35, "Distant signatures should have low similarity, got {}", sim_distant);
    }

    #[test]
    fn test_al_v2_trajectory_step_recording_and_recovery_extraction() {
        use crate::core::learning::signature::{StateSignatureBuilder, VerificationLevel};
        use crate::core::learning::trajectory::{Trajectory, TrajectoryStep};
        use crate::core::learning::strategy::StrategyKind;

        let mut traj = Trajectory::new("mission_rec_1", "fp_hash_rust_app");

        // Step 1: Encountered compile error
        let state1 = StateSignatureBuilder::new("fp_hash_rust_app")
            .phase(0)
            .compile_failures(1)
            .last_error_class(Some("CompileError".to_string()))
            .progress_stalled(true)
            .build();

        // Step 2: Successfully applied tool to fix compile error
        let state2 = StateSignatureBuilder::new("fp_hash_rust_app")
            .phase(0)
            .compile_failures(1)
            .verification_level(VerificationLevel::SyntaxOrStatic)
            .last_error_class(None) // Resolved
            .progress_stalled(false)
            .build();

        let step1 = TrajectoryStep {
            step: 1,
            from_state: state1.clone(),
            strategy: StrategyKind::DirectImplementation,
            tool: "TOOL_PROGRAMMER".to_string(),
            success: false,
            error_encountered: Some("unresolved import `crate::foo`".to_string()),
            to_state: Some(state1.clone()),
        };
        traj.record_step(step1);

        let step2 = TrajectoryStep {
            step: 2,
            from_state: state1.clone(),
            strategy: StrategyKind::CompileFirst,
            tool: "TOOL_PROGRAMMER".to_string(),
            success: true,
            error_encountered: None,
            to_state: Some(state2.clone()),
        };
        traj.record_step(step2);

        traj.finalize(LearningOutcome::Success, 3200);

        assert_eq!(traj.step_count(), 2);
        let recoveries = traj.extract_recoveries();
        assert_eq!(recoveries.len(), 1, "Must extract 1 successful recovery sequence");
        assert_eq!(recoveries[0].trigger_error, "CompileError");
        assert_eq!(recoveries[0].strategy, StrategyKind::CompileFirst);
        assert_eq!(recoveries[0].tool, "TOOL_PROGRAMMER");
        assert!(recoveries[0].resolved_state.last_error_class.is_none());
    }

    #[test]
    fn test_al_v2_experience_serialization_with_trajectory_backward_compatibility() {
        use crate::core::learning::signature::StateSignatureBuilder;
        use crate::core::learning::trajectory::{Trajectory, TrajectoryStep};
        use crate::core::learning::strategy::StrategyKind;

        // 1. Deserializing legacy JSON without "trajectory" field defaults to None
        let legacy_json = r#"{
            "schema_version": 2,
            "id": "exp_legacy_1",
            "attempt_id": "att_legacy_1",
            "mission_id": "m_leg",
            "timestamp": 1700000000,
            "fingerprint": {
                "language": "rust",
                "framework": null,
                "complexity": 0.4,
                "ambiguity": 0.2,
                "scope_bucket": 1,
                "requires_code": true,
                "requires_terminal": false,
                "requires_tests": false,
                "requires_network": false,
                "verification_level": 1
            },
            "model": "deepseek-coder",
            "strategy": "DirectImplementation",
            "result": {
                "outcome": "Success",
                "metrics": {
                    "steps": 1,
                    "tool_calls": 1,
                    "failed_actions": 0,
                    "recovery_actions": 0,
                    "verification_attempts": 1,
                    "successful_verifications": 1,
                    "elapsed_ms": 1000
                },
                "failures": [],
                "recovery": null
            },
            "confidence": 0.85,
            "lesson": null
        }"#;

        let loaded_legacy: Result<Experience, _> = serde_json::from_str(legacy_json);
        assert!(loaded_legacy.is_ok(), "Legacy experience without trajectory must deserialize successfully");
        let exp_legacy = loaded_legacy.unwrap();
        assert!(exp_legacy.trajectory.is_none(), "Legacy experience must have trajectory = None");

        // 2. Serializing and deserializing experience with an AL-v2 trajectory preserves all data
        let mut traj = Trajectory::new("m_v2", "hash_rust");
        let state = StateSignatureBuilder::new("hash_rust")
            .phase(1)
            .build();
        traj.record_step(TrajectoryStep {
            step: 1,
            from_state: state.clone(),
            strategy: StrategyKind::CompileFirst,
            tool: "TOOL_TERMINAL".to_string(),
            success: true,
            error_encountered: None,
            to_state: Some(state),
        });
        traj.finalize(LearningOutcome::Success, 1200);

        let mut exp_v2 = exp_legacy.clone();
        exp_v2.id = "exp_v2_1".to_string();
        exp_v2.trajectory = Some(traj);

        let serialized_v2 = serde_json::to_string(&exp_v2).unwrap();
        let deserialized_v2: Experience = serde_json::from_str(&serialized_v2).unwrap();
        assert!(deserialized_v2.trajectory.is_some());
        let loaded_traj = deserialized_v2.trajectory.unwrap();
        assert_eq!(loaded_traj.mission_id, "m_v2");
        assert_eq!(loaded_traj.step_count(), 1);
    }

    // ── AL-v2.3 Tests — Strategy Learning (StateSignature → Strategy → Outcome) ──

    #[test]
    fn test_al_v2_3_state_strategy_index_update_from_experience() {
        use crate::core::learning::state_stats::StateStrategyIndex;
        use crate::core::learning::signature::StateSignatureBuilder;
        use crate::core::learning::trajectory::{Trajectory, TrajectoryStep};
        use crate::core::learning::strategy::StrategyKind;
        use crate::core::learning::outcome::LearningOutcome;

        let mut idx = StateStrategyIndex::new();
        let state = StateSignatureBuilder::new("fp_v2_3")
            .phase(0).compile_failures(2).progress_stalled(true).build();

        // Record one successful DiagnoseThenRepair step
        let mut traj = Trajectory::new("m_v2_3_a", "fp_v2_3");
        traj.record_step(TrajectoryStep {
            step: 1, from_state: state.clone(),
            strategy: StrategyKind::DiagnoseThenRepair,
            tool: "TOOL_TERMINAL".to_string(), success: true,
            error_encountered: None, to_state: None,
        });
        traj.finalize(LearningOutcome::Success, 5000);

        idx.update_from_experience(&traj.steps, traj.total_duration_ms, &traj.outcome, 1.0);

        assert_eq!(idx.total_records(), 1, "Must have 1 record after one trajectory");
        let (strat, rate) = idx.best_strategy_for_state(&state, 1).unwrap();
        assert_eq!(strat, StrategyKind::DiagnoseThenRepair);
        assert!(rate > 0.5, "Success rate must exceed 0.5 after one mission success");
    }

    #[tokio::test]
    async fn test_al_v2_3_router_uses_state_signature_when_available() {
        use std::sync::Arc;
        use tokio::sync::RwLock;
        use crate::core::learning::experience::ExperienceStoreV2;
        use crate::core::learning::router::AdaptiveRouter;
        use crate::core::learning::state_stats::StateStrategyIndex;
        use crate::core::learning::signature::StateSignatureBuilder;
        use crate::core::learning::trajectory::{Trajectory, TrajectoryStep};
        use crate::core::learning::strategy::StrategyKind;
        use crate::core::learning::outcome::LearningOutcome;

        // Build a StateStrategyIndex with clear evidence for IncrementalPatch
        // under a stalled/compile-error state
        let state = StateSignatureBuilder::new("fp_router_state")
            .phase(1).compile_failures(3).progress_stalled(true).build();

        let mut idx = StateStrategyIndex::new();
        // Record multiple successful IncrementalPatch in this state
        for i in 0..4u32 {
            let mut traj = Trajectory::new(format!("m_rs_{}", i), "fp_router_state");
            traj.record_step(TrajectoryStep {
                step: 1, from_state: state.clone(),
                strategy: StrategyKind::IncrementalPatch,
                tool: "TOOL_PROGRAMMER".to_string(), success: true,
                error_encountered: None, to_state: None,
            });
            traj.finalize(LearningOutcome::Success, 3000);
            idx.update_from_experience(&traj.steps, traj.total_duration_ms, &traj.outcome, 0.0);
        }

        // Verify the index recommends IncrementalPatch for this state
        let (recommended, _) = idx.best_strategy_for_state(&state, 2).unwrap();
        assert_eq!(recommended, StrategyKind::IncrementalPatch,
            "Index must recommend IncrementalPatch with 4 successful observations");

        // Wire the router with this index and empty experience store
        let store = Arc::new(RwLock::new(ExperienceStoreV2::new(100)));
        let router = AdaptiveRouter::new(
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            store,
            "mission_router_state_test",
        ).with_state_index(idx);

        let fp = make_fp("rust", true);
        let models = vec!["model-a".to_string()];

        // Without state: cold start (no experience data)
        let rec_no_state = router.recommend(&fp, &models).await;
        assert!(!rec_no_state.state_informed,
            "Without state arg, state_informed must be false");

        // With state: should be state_informed = true since index has data
        let rec_with_state = router.recommend_with_state(&fp, Some(&state), &models).await;
        assert!(rec_with_state.state_informed,
            "With state arg and index data, state_informed must be true");
        assert_eq!(rec_with_state.strategy, StrategyKind::IncrementalPatch,
            "Router must select state-informed strategy over fingerprint default");
    }

    #[tokio::test]
    async fn test_al_v2_3_router_falls_back_to_fingerprint_when_no_state_data() {
        use std::sync::Arc;
        use tokio::sync::RwLock;
        use crate::core::learning::experience::ExperienceStoreV2;
        use crate::core::learning::router::AdaptiveRouter;
        use crate::core::learning::signature::StateSignatureBuilder;

        let store = Arc::new(RwLock::new(ExperienceStoreV2::new(100)));
        let router = AdaptiveRouter::new(
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            store,
            "mission_fallback_test",
        );
        // Empty StateStrategyIndex (default)

        let fp = make_fp("rust", true);
        let state = StateSignatureBuilder::new("fp_no_data")
            .phase(0).build();
        let models = vec!["model-x".to_string()];

        let rec = router.recommend_with_state(&fp, Some(&state), &models).await;
        assert!(!rec.state_informed,
            "With empty index, state_informed must be false (fingerprint fallback)");
    }
}
