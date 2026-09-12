#![allow(dead_code)]
use std::path::Path;
use sha2::{Sha256, Digest};

/// Computes a deterministic content hash for a file using SHA-256.
/// Returns a hex string identical format used across WorldState, StateDelta, and Evidence.
pub fn compute_content_hash(path: &Path) -> Option<String> {
    match std::fs::read(path) {
        Ok(bytes) => Some(hash_bytes(&bytes)),
        Err(_) => None,
    }
}

/// Computes content hash from raw bytes (for in-memory use) using SHA-256.
pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let result = hasher.finalize();
    let num = u64::from_be_bytes(result[0..8].try_into().unwrap());
    format!("{:016x}", num)
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
