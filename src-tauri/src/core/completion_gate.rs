#![allow(dead_code)]
use serde::{Deserialize, Serialize};
use super::mission_contract::MissionContract;
use super::evidence::EvidenceGraph;
use super::cognitive_state::CognitiveState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompletionDecision {
    Complete,
    Incomplete(Vec<String>),
    Blocked(Vec<String>),
}

pub struct CompletionGate;

impl CompletionGate {
    pub fn evaluate(
        contract: &MissionContract,
        _state: &CognitiveState,
        evidence: &EvidenceGraph,
        current_world_hash: u64,
        workspace_path: &std::path::Path,
    ) -> CompletionDecision {
        // Guard: empty contract must never silently complete a build/coding mission.
        // A contract with no required criteria AND no required evidence is invalid
        // for any non-trivial mission — block it explicitly.
        let has_any_requirements = !contract.acceptance_criteria.is_empty()
            || !contract.required_evidence.is_empty();
        if !has_any_requirements {
            return CompletionDecision::Incomplete(vec![
                "El contrato de misión no tiene criterios de aceptación ni evidencia requerida. \
                 Agrega criterios antes de declarar completitud.".to_string(),
            ]);
        }

        let mut missing = Vec::new();

        // 1. Evaluate REQUIRED acceptance criteria DYNAMICALLY based on physical workspace inspection & EvidenceGraph.
        // We do NOT trust 'status == Satisfied' set by the LLM.
        for ac in &contract.acceptance_criteria {
            if !ac.required {
                continue;
            }

            let is_satisfied = match &ac.verification {
                crate::core::mission_contract::VerificationMethod::TestPassed => {
                    evidence.has_valid_structured_evidence_for_state(
                        crate::core::evidence::EvidenceKind::Test,
                        "cargo test passes",
                        0.5,
                        current_world_hash
                    )
                    || evidence.has_valid_structured_evidence_for_state(
                        crate::core::evidence::EvidenceKind::Test,
                        "tests pass",
                        0.5,
                        current_world_hash
                    )
                },
                crate::core::mission_contract::VerificationMethod::CommandExitZero(cmd) => {
                    let claim = format!("{} passes", cmd.trim());
                    evidence.has_valid_structured_evidence_for_state(
                        crate::core::evidence::EvidenceKind::CommandExitCode,
                        &claim,
                        0.5,
                        current_world_hash
                    )
                },
                crate::core::mission_contract::VerificationMethod::FileExistence(file) => {
                    // P0: Physical disk check required! Historical evidence alone is not enough if file was deleted.
                    let target_path = workspace_path.join(file);
                    target_path.exists()
                },
                crate::core::mission_contract::VerificationMethod::ContentMatches { file, regex } => {
                    // P0: Read real file content from disk and match regex!
                    let target_path = workspace_path.join(file);
                    if let Ok(content) = std::fs::read_to_string(&target_path) {
                        if let Ok(re) = regex::Regex::new(regex) {
                            re.is_match(&content)
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                },
                crate::core::mission_contract::VerificationMethod::ManualReview => {
                    evidence.has_valid_evidence_for_state(&format!("{} verified manually", ac.id), 1.0, current_world_hash)
                },
            };

            if !is_satisfied {
                missing.push(format!("[{}] {}", ac.id, ac.description));
            }
        }

        // 2. Required evidence – uses STRICT exact match + state_hash validation
        for req in &contract.required_evidence {
            if !evidence.has_valid_evidence_for_state(&req.claim, req.min_reliability, current_world_hash) {
                missing.push(format!(
                    "Evidencia requerida ausente o inválida por cambio de código: '{}' (confiabilidad mínima: {:.0}%)",
                    req.claim, req.min_reliability * 100.0
                ));
            }
        }

        if missing.is_empty() {
            CompletionDecision::Complete
        } else {
            CompletionDecision::Incomplete(missing)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mission_contract::{MissionContract, EvidenceRequirement};
    use crate::core::evidence::{EvidenceGraph, EvidenceKind};
    use std::path::Path;

    #[test]
    fn test_completion_gate_blocks_on_min_reliability() {
        let mut contract = MissionContract::new("Build app");
        contract.required_evidence.push(EvidenceRequirement {
            claim: "cargo test passes".to_string(),
            min_reliability: 0.8,
        });

        let mut evidence = EvidenceGraph::new();
        let state = CognitiveState::new("m1", "Build app");
        let hash = 12345;
        let dummy_path = Path::new(".");

        // No evidence at all -> Incomplete
        let dec = CompletionGate::evaluate(&contract, &state, &evidence, hash, dummy_path);
        assert!(matches!(dec, CompletionDecision::Incomplete(_)));

        // Evidence with insufficient reliability -> still Incomplete
        evidence.record_with_hash(EvidenceKind::Test, "cargo", "cargo test passes", "0", 0.6, 1, Some(hash)).unwrap();
        let dec2 = CompletionGate::evaluate(&contract, &state, &evidence, hash, dummy_path);
        assert!(matches!(dec2, CompletionDecision::Incomplete(_)));

        // Evidence with sufficient reliability but WRONG hash -> Incomplete
        evidence.record_with_hash(EvidenceKind::Test, "cargo", "cargo test passes", "0", 0.9, 2, Some(99999)).unwrap();
        let dec3 = CompletionGate::evaluate(&contract, &state, &evidence, hash, dummy_path);
        assert!(matches!(dec3, CompletionDecision::Incomplete(_)));

        // Evidence with sufficient reliability AND CORRECT hash -> Complete
        evidence.record_with_hash(EvidenceKind::Test, "cargo", "cargo test passes", "0", 0.9, 3, Some(hash)).unwrap();
        let dec4 = CompletionGate::evaluate(&contract, &state, &evidence, hash, dummy_path);
        assert_eq!(dec4, CompletionDecision::Complete);
    }

    #[test]
    fn test_regression_false_satisfied() {
        let mut contract = MissionContract::new("Build");
        contract.add_criterion(
            "AC-1",
            "Tests pass",
            crate::core::mission_contract::VerificationMethod::TestPassed,
            true
        );
        contract.mark_criterion("AC-1", true);

        let evidence = EvidenceGraph::new();
        let state = CognitiveState::new("m1", "Build app");
        let hash = 111;

        let dec = CompletionGate::evaluate(&contract, &state, &evidence, hash, Path::new("."));
        assert!(matches!(dec, CompletionDecision::Incomplete(_)));
    }

    #[test]
    fn test_regression_evidence_without_hash_rejected() {
        let mut evidence = EvidenceGraph::new();
        let res = evidence.record(EvidenceKind::Compilation, "tool", "claim", "value", 1.0, 1);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Security Violation"));
    }

    #[test]
    fn test_physical_file_existence_verification() {
        let temp_dir = std::env::temp_dir().join(format!("aura_test_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let mut contract = MissionContract::new("Check file");
        contract.add_criterion(
            "AC-FILE",
            "Output file exists",
            crate::core::mission_contract::VerificationMethod::FileExistence("output.txt".to_string()),
            true
        );
        let evidence = EvidenceGraph::new();
        let state = CognitiveState::new("m1", "File check");

        // Test 1: File does not exist -> Incomplete
        let dec1 = CompletionGate::evaluate(&contract, &state, &evidence, 100, &temp_dir);
        assert!(matches!(dec1, CompletionDecision::Incomplete(_)));

        // Test 2: File created physically -> Complete
        let file_path = temp_dir.join("output.txt");
        std::fs::write(&file_path, "hello").unwrap();
        let dec2 = CompletionGate::evaluate(&contract, &state, &evidence, 100, &temp_dir);
        assert_eq!(dec2, CompletionDecision::Complete);

        // Test 3: File deleted physically -> Incomplete (despite previous existence)
        std::fs::remove_file(&file_path).unwrap();
        let dec3 = CompletionGate::evaluate(&contract, &state, &evidence, 100, &temp_dir);
        assert!(matches!(dec3, CompletionDecision::Incomplete(_)));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_physical_content_matches_verification() {
        let temp_dir = std::env::temp_dir().join(format!("aura_test_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let mut contract = MissionContract::new("Check content");
        contract.add_criterion(
            "AC-CONTENT",
            "JSON contains total_words",
            crate::core::mission_contract::VerificationMethod::ContentMatches {
                file: "stats.json".to_string(),
                regex: r#""total_words"\s*:\s*\d+"#.to_string(),
            },
            true
        );
        let evidence = EvidenceGraph::new();
        let state = CognitiveState::new("m1", "Content check");
        let file_path = temp_dir.join("stats.json");

        // Test 4: File contains matching JSON -> Complete
        std::fs::write(&file_path, r#"{"total_words": 42}"#).unwrap();
        let dec1 = CompletionGate::evaluate(&contract, &state, &evidence, 200, &temp_dir);
        assert_eq!(dec1, CompletionDecision::Complete);

        // Test 5: File modified to not match -> Incomplete
        std::fs::write(&file_path, r#"{"error": "failed"}"#).unwrap();
        let dec2 = CompletionGate::evaluate(&contract, &state, &evidence, 200, &temp_dir);
        assert!(matches!(dec2, CompletionDecision::Incomplete(_)));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_command_exit_zero_and_test_passed_verification() {
        let mut contract = MissionContract::new("Build and test");
        contract.add_criterion(
            "AC-BUILD",
            "cargo build passes",
            crate::core::mission_contract::VerificationMethod::CommandExitZero("cargo build".to_string()),
            true
        );
        contract.add_criterion(
            "AC-TEST",
            "cargo test passes",
            crate::core::mission_contract::VerificationMethod::TestPassed,
            true
        );

        let mut evidence = EvidenceGraph::new();
        let state = CognitiveState::new("m1", "Build & Test");
        let current_hash = 999;
        let old_hash = 111;
        let dummy_path = Path::new(".");

        // Test 6 & 7: CommandExitZero with old hash -> Incomplete
        evidence.record_with_hash(EvidenceKind::CommandExitCode, "TOOL_TERMINAL", "cargo build passes", "0", 1.0, 1, Some(old_hash)).unwrap();
        let dec1 = CompletionGate::evaluate(&contract, &state, &evidence, current_hash, dummy_path);
        assert!(matches!(dec1, CompletionDecision::Incomplete(_)));

        // Record cargo build with current hash
        evidence.record_with_hash(EvidenceKind::CommandExitCode, "TOOL_TERMINAL", "cargo build passes", "0", 1.0, 2, Some(current_hash)).unwrap();
        // Still missing test passed
        let dec2 = CompletionGate::evaluate(&contract, &state, &evidence, current_hash, dummy_path);
        assert!(matches!(dec2, CompletionDecision::Incomplete(_)));

        // Test 8: TestPassed with current hash -> Complete
        evidence.record_with_hash(EvidenceKind::Test, "TOOL_TERMINAL", "cargo test passes", "0", 1.0, 3, Some(current_hash)).unwrap();
        let dec3 = CompletionGate::evaluate(&contract, &state, &evidence, current_hash, dummy_path);
        assert_eq!(dec3, CompletionDecision::Complete);
    }

    #[test]
    fn test_unstructured_textual_claim_rejected_by_gate() {
        let mut contract = MissionContract::new("Build and test");
        contract.add_criterion(
            "AC-BUILD",
            "cargo build passes",
            crate::core::mission_contract::VerificationMethod::CommandExitZero("cargo build".to_string()),
            true
        );
        contract.add_criterion(
            "AC-TEST",
            "cargo test passes",
            crate::core::mission_contract::VerificationMethod::TestPassed,
            true
        );

        let mut evidence = EvidenceGraph::new();
        let state = CognitiveState::new("m1", "Build & Test");
        let hash = 888;
        let dummy_path = Path::new(".");

        // Fraudulent / textual evidence using StaticAnalysis instead of CommandExitCode/Test
        evidence.record_with_hash(EvidenceKind::StaticAnalysis, "TOOL_LLM", "cargo build passes", "0", 1.0, 1, Some(hash)).unwrap();
        evidence.record_with_hash(EvidenceKind::StaticAnalysis, "TOOL_LLM", "cargo test passes", "0", 1.0, 2, Some(hash)).unwrap();

        // CompletionGate MUST REJECT because the kind is StaticAnalysis, NOT CommandExitCode or Test!
        let dec = CompletionGate::evaluate(&contract, &state, &evidence, hash, dummy_path);
        assert!(matches!(dec, CompletionDecision::Incomplete(_)));
    }
}
