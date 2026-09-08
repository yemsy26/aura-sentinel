#![allow(dead_code)]
use crate::core::content_hash::compute_content_hash;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDelta {
    pub file_path: String,
    pub changed: bool,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub bytes_before: u64,
    pub bytes_after: u64,
}

pub struct StateDelta;

impl StateDelta {
    /// Computes a deterministic content hash for a file using shared content_hash module.
    pub fn compute_hash(path: &Path) -> Option<String> {
        compute_content_hash(path)
    }

    pub fn capture_before(workspace: &str, relative_path: &str) -> (Option<String>, u64) {
        let full_path = Path::new(workspace).join(relative_path);
        let hash = Self::compute_hash(&full_path);
        let bytes = full_path.metadata().map(|m| m.len()).unwrap_or(0);
        (hash, bytes)
    }

    pub fn capture_delta(
        workspace: &str,
        relative_path: &str,
        before_hash: Option<String>,
        before_bytes: u64,
    ) -> FileDelta {
        let full_path = Path::new(workspace).join(relative_path);
        let after_hash = Self::compute_hash(&full_path);
        let after_bytes = full_path.metadata().map(|m| m.len()).unwrap_or(0);
        let changed = before_hash != after_hash;
        FileDelta { file_path: relative_path.to_string(), changed, before_hash, after_hash, bytes_before: before_bytes, bytes_after: after_bytes }
    }
}
