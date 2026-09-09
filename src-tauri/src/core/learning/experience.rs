use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use crate::core::learning::fingerprint::TaskFingerprint;
use crate::core::learning::outcome::LearningResult;
use crate::core::learning::strategy::StrategyKind;

pub const SCHEMA_VERSION: u16 = 2;

/// A complete mission learning record — immutable after being written.
/// Only LearningEngine writes this. Runtime v4 is not modified.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Experience {
    pub schema_version: u16,
    pub id: String,
    /// Idempotence key: "<mission_id>_<attempt_num>"
    pub attempt_id: String,
    pub mission_id: String,
    pub timestamp: u64,
    pub fingerprint: TaskFingerprint,
    pub model: String,
    pub strategy: StrategyKind,
    pub result: LearningResult,
    /// Confidence score of the original Recommendation that was applied
    pub confidence: f32,
    pub lesson: Option<String>,
}

/// In-memory store. Loaded from JSONL at startup. Idempotent on insert.
pub struct ExperienceStoreV2 {
    pub experiences: Vec<Experience>,
    seen_attempt_ids: HashSet<String>,
    pub max_entries: usize,
}

impl ExperienceStoreV2 {
    pub fn new(max_entries: usize) -> Self {
        Self {
            experiences: Vec::new(),
            seen_attempt_ids: HashSet::new(),
            max_entries,
        }
    }

    /// Returns false if attempt_id was already seen (idempotent — no duplicate).
    pub fn push(&mut self, exp: Experience) -> bool {
        if self.seen_attempt_ids.contains(&exp.attempt_id) {
            return false; // idempotent
        }
        self.seen_attempt_ids.insert(exp.attempt_id.clone());
        self.experiences.push(exp);
        // Trim oldest if over capacity
        if self.experiences.len() > self.max_entries {
            let remove = self.experiences.len() - self.max_entries;
            self.experiences.drain(0..remove);
        }
        true
    }

    /// Experiences with structural similarity >= threshold, most recent first.
    pub fn find_similar<'a>(
        &'a self, fp: &TaskFingerprint, limit: usize,
    ) -> Vec<&'a Experience> {
        use crate::core::learning::fingerprint::FingerprintBuilder;
        let mut scored: Vec<(f32, &Experience)> = self.experiences.iter()
            .map(|e| (FingerprintBuilder::similarity(fp, &e.fingerprint), e))
            .filter(|(s, _)| *s >= 0.5)
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().take(limit).map(|(_, e)| e).collect()
    }

    pub fn by_model<'a>(&'a self, model: &str) -> Vec<&'a Experience> {
        self.experiences.iter().filter(|e| e.model == model).collect()
    }

    pub fn len(&self) -> usize { self.experiences.len() }
    pub fn is_empty(&self) -> bool { self.experiences.is_empty() }
}
