#![allow(dead_code)]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StallType {
    NoStateChange,
    RepeatedTool,
    RepeatedCommand,
    SameError,
    NoCriteriaProgress,
    ModelLoop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressSignature {
    pub step: u32,
    pub state_hash: u64,
    pub files_changed: u32,
    pub criteria_satisfied: u32,
    pub evidence_count: u32,
}

pub struct StallDetector {
    signatures: Vec<ProgressSignature>,
    max_history: usize,
}

impl StallDetector {
    pub fn new(max_history: usize) -> Self {
        Self {
            signatures: Vec::new(),
            max_history,
        }
    }

    pub fn record_signature(&mut self, sig: ProgressSignature) {
        if self.signatures.len() >= self.max_history {
            self.signatures.remove(0);
        }
        self.signatures.push(sig);
    }

    /// Comprueba si en las últimas N firmas no ha habido ningún cambio en archivos ni criterios
    pub fn detect_stall(&self, window_size: usize) -> Option<StallType> {
        if self.signatures.len() < window_size || window_size < 2 {
            return None;
        }

        let slice = &self.signatures[self.signatures.len() - window_size..];
        let first = &slice[0];

        let has_progress = slice.iter().skip(1).any(|s| {
            s.files_changed != first.files_changed
                || s.criteria_satisfied != first.criteria_satisfied
                || s.evidence_count != first.evidence_count
                || s.state_hash != first.state_hash
        });

        if !has_progress {
            Some(StallType::NoStateChange)
        } else {
            None
        }
    }
}
