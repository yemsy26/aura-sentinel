#![allow(dead_code)]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceKind {
    FileHash,
    FileCreated,
    FileModified,
    CommandExitCode,
    CommandOutput,
    Compilation,
    Test,
    Lint,
    StaticAnalysis,
    RuntimeCheck,
    UserConfirmation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub id: String,
    pub kind: EvidenceKind,
    pub source: String,
    pub claim: String,
    pub value: String,
    pub timestamp: String,
    pub reliability: f32,
    pub mission_step: u32,
    pub state_hash: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EvidenceGraph {
    pub entries: Vec<Evidence>,
}

impl EvidenceGraph {
    pub fn new() -> Self { Self { entries: Vec::new() } }

    pub fn record(
        &mut self, kind: EvidenceKind, source: &str, claim: &str,
        value: &str, reliability: f32, step: u32,
    ) -> String {
        self.record_with_hash(kind, source, claim, value, reliability, step, None)
    }

    pub fn record_with_hash(
        &mut self, kind: EvidenceKind, source: &str, claim: &str,
        value: &str, reliability: f32, step: u32, state_hash: Option<u64>
    ) -> String {
        let id = format!("ev_{:x}", self.entries.len() + 1);
        self.entries.push(Evidence {
            id: id.clone(), kind, source: source.to_string(),
            claim: claim.to_string(), value: value.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(), reliability, mission_step: step,
            state_hash,
        });
        id
    }

    /// Strict exact-match evidence lookup with minimum reliability threshold AND workspace state hash verification.
    /// This prevents using old evidence (e.g., tests passing) after the workspace has been modified.
    pub fn has_valid_evidence_for_state(&self, claim: &str, min_reliability: f32, current_state_hash: u64) -> bool {
        self.entries.iter().any(|e| {
            if e.claim != claim || e.reliability < min_reliability {
                return false;
            }
            // If evidence doesn't have a hash, it's considered permanently valid (e.g. system state).
            // But if it's tied to a workspace state, it MUST match the current state.
            if let Some(hash) = e.state_hash {
                hash == current_state_hash
            } else {
                true
            }
        })
    }

    /// Strict exact-match evidence lookup with minimum reliability threshold.
    pub fn has_valid_evidence_for(&self, claim: &str, min_reliability: f32) -> bool {
        self.entries.iter().any(|e| {
            e.claim == claim && e.reliability >= min_reliability
        })
    }

    /// Backward-compatible loose check (min_reliability = 0.0, exact match).
    pub fn has_evidence_for(&self, claim: &str) -> bool {
        self.has_valid_evidence_for(claim, 0.0)
    }

    pub fn latest_tests_passed(&self) -> bool {
        self.entries.iter().rev()
            .find(|e| e.kind == EvidenceKind::Test)
            .map(|e| e.value == "PASSED" || e.value == "0")
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match_required() {
        let mut g = EvidenceGraph::new();
        g.record(EvidenceKind::Compilation, "cargo", "cargo check passes", "0", 0.9, 1);

        // Exact match succeeds
        assert!(g.has_valid_evidence_for("cargo check passes", 0.5));
        // Substring/superset does NOT match
        assert!(!g.has_valid_evidence_for("cargo check", 0.5));
        assert!(!g.has_valid_evidence_for("cargo check passes and tests", 0.5));
        // Reliability threshold enforced
        assert!(!g.has_valid_evidence_for("cargo check passes", 0.95));
    }
}
