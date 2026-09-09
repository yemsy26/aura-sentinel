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

        // 1. Only REQUIRED acceptance criteria block completion.
        // Optional criteria (required=false) are tracked but do NOT block the gate.
        let pending = contract.pending_required_criteria();
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
