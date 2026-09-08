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

        // 1. Verify required acceptance criteria
        let pending = contract.pending_criteria();
        if !pending.is_empty() {
            missing.extend(pending);
        }

        // 2. Verify required evidence requirements
        for req in &contract.required_evidence {
            if !evidence.has_evidence_for(&req.claim) {
                missing.push(format!("Evidencia faltante para: '{}'", req.claim));
            }
        }

        if missing.is_empty() {
            CompletionDecision::Complete
        } else {
            CompletionDecision::Incomplete(missing)
        }
    }
}
