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
    ) -> CompletionDecision {
        let mut missing = Vec::new();

        // 1. Acceptance criteria that are not yet satisfied
        // Note: criteria can only be Satisfied via internal evidence verification,
        // NOT by LLM declaration (mark_criterion is pub(crate)).
        let pending = contract.pending_criteria();
        if !pending.is_empty() {
            missing.extend(pending);
        }

        // 2. Required evidence — uses STRICT exact match + min_reliability
        for req in &contract.required_evidence {
            if !evidence.has_valid_evidence_for(&req.claim, req.min_reliability) {
                missing.push(format!(
                    "Evidencia requerida ausente: '{}' (confiabilidad mínima: {:.0}%)",
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

        // No evidence at all -> Incomplete
        let dec = CompletionGate::evaluate(&contract, &state, &evidence);
        assert!(matches!(dec, CompletionDecision::Incomplete(_)));

        // Evidence with insufficient reliability -> still Incomplete
        evidence.record(EvidenceKind::Test, "cargo", "cargo test passes", "0", 0.6, 1);
        let dec2 = CompletionGate::evaluate(&contract, &state, &evidence);
        assert!(matches!(dec2, CompletionDecision::Incomplete(_)));

        // Evidence with sufficient reliability -> Complete
        evidence.record(EvidenceKind::Test, "cargo", "cargo test passes", "0", 0.9, 2);
        let dec3 = CompletionGate::evaluate(&contract, &state, &evidence);
        assert_eq!(dec3, CompletionDecision::Complete);
    }
}
