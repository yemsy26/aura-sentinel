#![allow(dead_code)]
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
    /// Calcula el hash rápido de un archivo si existe
    pub fn compute_hash(path: &Path) -> Option<String> {
        if !path.exists() {
            return None;
        }
        if let Ok(bytes) = std::fs::read(path) {
            use std::hash::{Hash, Hasher};
            use std::collections::hash_map::DefaultHasher;
            let mut hasher = DefaultHasher::new();
            bytes.hash(&mut hasher);
            Some(format!("{:016x}", hasher.finish()))
        } else {
            None
        }
    }

    /// Captura el estado antes de una modificación
    pub fn capture_before(workspace: &str, relative_path: &str) -> (Option<String>, u64) {
        let full_path = Path::new(workspace).join(relative_path);
        let hash = Self::compute_hash(&full_path);
        let bytes = full_path.metadata().map(|m| m.len()).unwrap_or(0);
        (hash, bytes)
    }

    /// Compara contra el estado posterior y genera un FileDelta
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

        FileDelta {
            file_path: relative_path.to_string(),
            changed,
            before_hash,
            after_hash,
            bytes_before: before_bytes,
            bytes_after: after_bytes,
        }
    }
}
