#![allow(dead_code)]
use std::path::Path;

/// Computes a deterministic content hash for a file using std DefaultHasher.
/// Returns a hex string identical format used across WorldState, StateDelta, and Evidence.
pub fn compute_content_hash(path: &Path) -> Option<String> {
    use std::hash::{Hash, Hasher};
    use std::collections::hash_map::DefaultHasher;
    match std::fs::read(path) {
        Ok(bytes) => {
            let mut hasher = DefaultHasher::new();
            bytes.hash(&mut hasher);
            Some(format!("{:016x}", hasher.finish()))
        }
        Err(_) => None,
    }
}

/// Computes content hash from raw bytes (for in-memory use).
pub fn hash_bytes(bytes: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    use std::collections::hash_map::DefaultHasher;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_bytes_deterministic() {
        let h1 = hash_bytes(b"hello world");
        let h2 = hash_bytes(b"hello world");
        assert_eq!(h1, h2);
        assert_ne!(hash_bytes(b"hello world"), hash_bytes(b"different"));
        assert_eq!(h1.len(), 16);
    }
}
