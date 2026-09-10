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

        // 1. Evaluate REQUIRED acceptance criteria DYNAMICALLY based on EvidenceGraph.
        // We do NOT trust 'status == Satisfied' set by the LLM.
        for ac in &contract.acceptance_criteria {
            if !ac.required {
                continue;
            }

            let is_satisfied = match &ac.verification {
                crate::core::mission_contract::VerificationMethod::TestPassed => {
                    evidence.has_valid_evidence_for_state("cargo test passes", 0.5, current_world_hash)
                    || evidence.has_valid_evidence_for_state("tests pass", 0.5, current_world_hash)
                },
                crate::core::mission_contract::VerificationMethod::CommandExitZero(cmd) => {
                    let claim = format!("{} passes", cmd);
                    evidence.has_valid_evidence_for_state(&claim, 0.5, current_world_hash)
                },
                crate::core::mission_contract::VerificationMethod::FileExistence(file) => {
                    let claim = format!("file {} exists", file);
                    // File existence evidence should ideally match the hash when it was checked
                    evidence.has_valid_evidence_for_state(&claim, 0.5, current_world_hash)
                },
                
                
                
                crate::core::mission_contract::VerificationMethod::ContentMatches { file, regex } => evidence.has_valid_evidence_for_state(&format!("file {} matches {}", file, regex), 0.5, current_world_hash), crate::core::mission_contract::VerificationMethod::ManualReview => evidence.has_valid_evidence_for_state(&format!("{} verified manually", ac.id), 1.0, current_world_hash),
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

        // No evidence at all -> Incomplete
        let dec = CompletionGate::evaluate(&contract, &state, &evidence, hash);
        assert!(matches!(dec, CompletionDecision::Incomplete(_)));

        // Evidence with insufficient reliability -> still Incomplete
        evidence.record_with_hash(EvidenceKind::Test, "cargo", "cargo test passes", "0", 0.6, 1, Some(hash)).unwrap();
        let dec2 = CompletionGate::evaluate(&contract, &state, &evidence, hash);
        assert!(matches!(dec2, CompletionDecision::Incomplete(_)));

        // Evidence with sufficient reliability but WRONG hash -> Incomplete
        evidence.record_with_hash(EvidenceKind::Test, "cargo", "cargo test passes", "0", 0.9, 2, Some(99999)).unwrap();
        let dec3 = CompletionGate::evaluate(&contract, &state, &evidence, hash);
        assert!(matches!(dec3, CompletionDecision::Incomplete(_)));

        // Evidence with sufficient reliability AND CORRECT hash -> Complete
        evidence.record_with_hash(EvidenceKind::Test, "cargo", "cargo test passes", "0", 0.9, 3, Some(hash)).unwrap();
        let dec4 = CompletionGate::evaluate(&contract, &state, &evidence, hash);
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

        let dec = CompletionGate::evaluate(&contract, &state, &evidence, hash);
        assert!(matches!(dec, CompletionDecision::Incomplete(_)));
    }

    #[test]
    fn test_regression_evidence_without_hash_rejected() {
        let mut evidence = EvidenceGraph::new();
        let res = evidence.record(EvidenceKind::Compilation, "tool", "claim", "value", 1.0, 1);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Security Violation"));
    }
}
