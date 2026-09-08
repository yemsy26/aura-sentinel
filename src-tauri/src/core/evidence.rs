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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EvidenceGraph {
    pub entries: Vec<Evidence>,
}

impl EvidenceGraph {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub fn record(
        &mut self,
        kind: EvidenceKind,
        source: &str,
        claim: &str,
        value: &str,
        reliability: f32,
        step: u32,
    ) -> String {
        let id = format!("ev_{:x}", self.entries.len() + 1);
        self.entries.push(Evidence {
            id: id.clone(),
            kind,
            source: source.to_string(),
            claim: claim.to_string(),
            value: value.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            reliability,
            mission_step: step,
        });
        id
    }

    pub fn has_evidence_for(&self, claim: &str) -> bool {
        self.entries.iter().any(|e| e.claim.contains(claim) || claim.contains(&e.claim))
    }

    pub fn latest_tests_passed(&self) -> bool {
        self.entries
            .iter()
            .rev()
            .find(|e| e.kind == EvidenceKind::Test)
            .map(|e| e.value == "PASSED" || e.value == "0")
            .unwrap_or(false)
    }
}
