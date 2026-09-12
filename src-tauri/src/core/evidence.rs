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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StructuredFact {
    CommandResult {
        command: String,
        cwd: String,
        exit_code: i32,
        stdout_hash: String,
        stderr_hash: String,
    },
    TestResult {
        command: String,
        cwd: String,
        exit_code: i32,
        passed: u32,
        failed: u32,
        ignored: u32,
    },
    Generic {
        claim: String,
        value: String,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub id: String,
    pub kind: EvidenceKind,
    pub source: String,
    pub fact: StructuredFact,
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
        self.record_generic_with_hash(kind, source, claim, value, reliability, step, None)
    }

    pub fn record_with_hash(
        &mut self, kind: EvidenceKind, source: &str, claim: &str,
        value: &str, reliability: f32, step: u32, state_hash: Option<u64>
    ) -> Result<String, String> {
        self.record_generic_with_hash(kind, source, claim, value, reliability, step, state_hash)
    }

    pub fn record_generic_with_hash(
        &mut self, kind: EvidenceKind, source: &str, claim: &str,
        value: &str, reliability: f32, step: u32, state_hash: Option<u64>
    ) -> Result<String, String> {
        self.record_structured(
            kind,
            source,
            StructuredFact::Generic { claim: claim.to_string(), value: value.to_string() },
            reliability,
            step,
            state_hash
        )
    }

    pub fn record_structured(
        &mut self, kind: EvidenceKind, source: &str, fact: StructuredFact,
        reliability: f32, step: u32, state_hash: Option<u64>
    ) -> Result<String, String> {
        if Self::kind_requires_workspace_hash(&kind) && state_hash.is_none() {
            return Err(format!("Security Violation: EvidenceKind {:?} requires a state_hash but None was provided.", kind));
        }

        let id = format!("ev_{:x}", self.entries.len() + 1);
        self.entries.push(Evidence {
            id: id.clone(), kind, source: source.to_string(),
            fact,
            timestamp: chrono::Utc::now().to_rfc3339(), reliability, mission_step: step,
            state_hash,
        });
        Ok(id)
    }

    pub fn has_valid_structured_evidence<F>(&self, min_reliability: f32, current_state_hash: u64, predicate: F) -> bool
    where
        F: Fn(&StructuredFact) -> bool,
    {
        self.entries.iter().any(|e| {
            if e.reliability < min_reliability { return false; }
            if Self::kind_requires_workspace_hash(&e.kind) {
                if e.state_hash != Some(current_state_hash) { return false; }
            } else {
                if let Some(hash) = e.state_hash {
                    if hash != current_state_hash { return false; }
                }
            }
            predicate(&e.fact)
        })
    }

    pub fn has_valid_evidence_for_state(&self, claim: &str, min_reliability: f32, current_state_hash: u64) -> bool {
        self.has_valid_structured_evidence(min_reliability, current_state_hash, |fact| {
            if let StructuredFact::Generic { claim: c, .. } = fact {
                c == claim
            } else {
                false
            }
        })
    }

    pub fn has_valid_structured_evidence_for_state(
        &self,
        kind: EvidenceKind,
        claim: &str,
        min_reliability: f32,
        current_state_hash: u64
    ) -> bool {
        self.entries.iter().any(|e| {
            if e.kind != kind || e.reliability < min_reliability { return false; }
            if Self::kind_requires_workspace_hash(&e.kind) {
                if e.state_hash != Some(current_state_hash) { return false; }
            } else {
                if let Some(hash) = e.state_hash {
                    if hash != current_state_hash { return false; }
                }
            }
            if let StructuredFact::Generic { claim: c, .. } = &e.fact {
                c == claim
            } else {
                false
            }
        })
    }

    pub fn has_valid_evidence_for(&self, claim: &str, min_reliability: f32) -> bool {
        self.entries.iter().any(|e| {
            if e.reliability < min_reliability { return false; }
            if let StructuredFact::Generic { claim: c, .. } = &e.fact {
                c == claim
            } else {
                false
            }
        })
    }

    pub fn has_evidence_for(&self, claim: &str) -> bool {
        self.has_valid_evidence_for(claim, 0.0)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
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
