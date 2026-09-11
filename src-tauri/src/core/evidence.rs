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

    pub fn kind_requires_workspace_hash(kind: &EvidenceKind) -> bool {
        match kind {
            EvidenceKind::FileHash | EvidenceKind::FileCreated | EvidenceKind::FileModified |
            EvidenceKind::CommandExitCode | EvidenceKind::CommandOutput | EvidenceKind::Compilation |
            EvidenceKind::Test | EvidenceKind::Lint | EvidenceKind::StaticAnalysis |
            EvidenceKind::RuntimeCheck => true,
            EvidenceKind::UserConfirmation => false,
        }
    }

    pub fn record(
        &mut self, kind: EvidenceKind, source: &str, claim: &str,
        value: &str, reliability: f32, step: u32,
    ) -> Result<String, String> {
        self.record_with_hash(kind, source, claim, value, reliability, step, None)
    }

    pub fn record_with_hash(
        &mut self, kind: EvidenceKind, source: &str, claim: &str,
        value: &str, reliability: f32, step: u32, state_hash: Option<u64>
    ) -> Result<String, String> {
        if Self::kind_requires_workspace_hash(&kind) && state_hash.is_none() {
            return Err(format!("Security Violation: EvidenceKind {:?} requires a state_hash but None was provided.", kind));
        }

        let id = format!("ev_{:x}", self.entries.len() + 1);
        self.entries.push(Evidence {
            id: id.clone(), kind, source: source.to_string(),
            claim: claim.to_string(), value: value.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(), reliability, mission_step: step,
            state_hash,
        });
        Ok(id)
    }

    /// Strict exact-match evidence lookup with minimum reliability threshold AND workspace state hash verification.
    /// This prevents using old evidence (e.g., tests passing) after the workspace has been modified.
    pub fn has_valid_evidence_for_state(&self, claim: &str, min_reliability: f32, current_state_hash: u64) -> bool {
        self.entries.iter().any(|e| {
            if e.claim != claim || e.reliability < min_reliability {
                return false;
            }
            if Self::kind_requires_workspace_hash(&e.kind) {
                e.state_hash == Some(current_state_hash)
            } else {
                e.state_hash.map_or(true, |hash| hash == current_state_hash)
            }
        })
    }

    /// Strict exact-match evidence lookup filtering by EvidenceKind, minimum reliability, AND workspace state_hash verification.
    /// This guarantees that technical evidence (CommandExitCode, Test) came from real execution and matches current code.
    pub fn has_valid_structured_evidence_for_state(
        &self,
        kind: EvidenceKind,
        claim: &str,
        min_reliability: f32,
        current_state_hash: u64
    ) -> bool {
        self.entries.iter().any(|e| {
            if e.kind != kind || e.claim != claim || e.reliability < min_reliability {
                return false;
            }
            if Self::kind_requires_workspace_hash(&kind) {
                e.state_hash == Some(current_state_hash)
            } else {
                e.state_hash.map_or(true, |hash| hash == current_state_hash)
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
        g.record_with_hash(EvidenceKind::Compilation, "cargo", "cargo check passes", "0", 0.9, 1, Some(123)).unwrap();

        // Exact match succeeds
        assert!(g.has_valid_evidence_for("cargo check passes", 0.5));
        // Substring/superset does NOT match
        assert!(!g.has_valid_evidence_for("cargo check", 0.5));
        assert!(!g.has_valid_evidence_for("cargo check passes and tests", 0.5));
        // Reliability threshold enforced
        assert!(!g.has_valid_evidence_for("cargo check passes", 0.95));
    }
}
