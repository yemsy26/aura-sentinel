#![allow(dead_code)]
use crate::core::content_hash::compute_content_hash;
use std::collections::HashMap;
use std::path::Path;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileSnapshot {
    pub relative_path: String,
    pub size_bytes: u64,
    pub modified_secs: u64,
    pub metadata_fingerprint: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvSnapshot {
    pub has_git: bool,
    pub has_node: bool,
    pub has_python: bool,
    pub has_cargo: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GitSnapshot {
    pub branch: String,
    pub clean: bool,
    pub modified_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldState {
    pub timestamp_secs: u64,
    pub workspace_root: String,
    pub files: HashMap<String, FileSnapshot>,
    pub environment: EnvSnapshot,
    pub git: Option<GitSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct WorldStateDiff {
    pub added_files: Vec<String>,
    pub modified_files: Vec<String>,
    pub deleted_files: Vec<String>,
}

impl WorldStateDiff {
    pub fn has_changes(&self) -> bool {
        !self.added_files.is_empty() || !self.modified_files.is_empty() || !self.deleted_files.is_empty()
    }
}

impl WorldState {
    pub fn capture(workspace_path: &str) -> Result<Self, String> {
        Self::capture_incremental(workspace_path, None)
    }
    
    pub fn capture_full(workspace_path: &str) -> Result<Self, String> {
        Self::capture_incremental(workspace_path, None)
    }

    pub fn capture_incremental(workspace_path: &str, previous: Option<&WorldState>) -> Result<Self, String> {
        let ws = Path::new(workspace_path);
        let mut files = HashMap::new();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        if !ws.exists() {
            return Err(format!("WORKSPACE_NOT_FOUND: {}", workspace_path));
        }
        if !ws.is_dir() {
            return Err(format!("WORKSPACE_NOT_DIRECTORY: {}", workspace_path));
        }

        Self::scan_dir_recursive(ws, ws, &mut files, previous)?;

        let environment = previous.map(|p| p.environment.clone()).unwrap_or_else(|| Self::detect_environment());
        let git = previous.map(|p| p.git.clone()).unwrap_or_else(|| Self::detect_git(workspace_path));

        Ok(Self { timestamp_secs: now, workspace_root: workspace_path.to_string(), files, environment, git })
    }

    fn scan_dir_recursive(root: &Path, current: &Path, out: &mut HashMap<String, FileSnapshot>, previous: Option<&WorldState>) -> Result<(), String> {
        let entries = match std::fs::read_dir(current) {
            Ok(e) => e,
            Err(e) => return Err(format!("WORKSPACE_SCAN_FAILED: {}: {}", current.display(), e)),
        };

        for entry_res in entries {
            let entry = entry_res.map_err(|e| format!("WORKSPACE_SCAN_FAILED: {}: {}", current.display(), e))?;
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

            if path.is_dir() {
                if name == "node_modules" || name == ".git" || name == "target" || name == "__pycache__" || name == ".venv" || name == ".aura" {
                    continue;
                }
                Self::scan_dir_recursive(root, &path, out, previous)?;
            } else if path.is_file() {
                if name == ".aura_session.json" || name == ".aura_session.json.tmp" || name == ".aura_graph.json" || name == ".fenix_index.json" || name.starts_with(".aura") {
                    continue;
                }
                if let Ok(rel) = path.strip_prefix(root) {
                    let rel_str = rel.to_string_lossy().replace('\\', "/");
                    let meta = path.metadata().map_err(|e| format!("WORKSPACE_SCAN_FAILED: {}: {}", path.display(), e))?;
                    let size = meta.len();
                    let mtime_nanos = meta.modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_nanos())
                        .unwrap_or(0);
                    let mtime_secs = (mtime_nanos / 1_000_000_000) as u64;
                    let fingerprint = format!("{}_{}", size, mtime_nanos);
                    let hash = if let Some(prev) = previous {
                        if let Some(prev_file) = prev.files.get(&rel_str) {
                            if prev_file.metadata_fingerprint == fingerprint {
                                prev_file.content_hash.clone()
                            } else {
                                compute_content_hash(&path).unwrap_or_else(|| "hash_err".to_string())
                            }
                        } else {
                            compute_content_hash(&path).unwrap_or_else(|| "hash_err".to_string())
                        }
                    } else {
                        compute_content_hash(&path).unwrap_or_else(|| "hash_err".to_string())
                    };
                    out.insert(rel_str.clone(), FileSnapshot {
                        relative_path: rel_str,
                        size_bytes: size,
                        modified_secs: mtime_secs,
                        metadata_fingerprint: fingerprint,
                        content_hash: hash,
                    });
                }
            }
        }
        Ok(())
    }

    fn detect_environment() -> EnvSnapshot {
        let has_git = std::process::Command::new("git").arg("--version").output().is_ok();
        let has_node = std::process::Command::new("node").arg("--version").output().is_ok();
        let has_python = std::process::Command::new("python").arg("--version").output().is_ok();
        let has_cargo = std::process::Command::new("cargo").arg("--version").output().is_ok();
        EnvSnapshot { has_git, has_node, has_python, has_cargo }
    }

    fn detect_git(workspace_path: &str) -> Option<GitSnapshot> {
        let ws = Path::new(workspace_path);
        if !ws.join(".git").exists() { return None; }

        let branch_out = std::process::Command::new("git")
            .arg("rev-parse").arg("--abbrev-ref").arg("HEAD")
            .current_dir(workspace_path).output().ok()?;
        let branch = String::from_utf8_lossy(&branch_out.stdout).trim().to_string();

        let status_out = std::process::Command::new("git")
            .arg("status").arg("--porcelain")
            .current_dir(workspace_path).output().ok()?;
        let lines: Vec<&str> = std::str::from_utf8(&status_out.stdout)
            .unwrap_or("").lines().filter(|l| !l.trim().is_empty()).collect();

        Some(GitSnapshot { branch, clean: lines.is_empty(), modified_count: lines.len() })
    }

    pub fn diff(&self, current: &WorldState) -> WorldStateDiff {
        let mut added = Vec::new();
        let mut modified = Vec::new();
        let mut deleted = Vec::new();

        for (path, curr_snap) in &current.files {
            match self.files.get(path) {
                None => added.push(path.clone()),
                Some(prev_snap) => {
                    if prev_snap.content_hash != curr_snap.content_hash || prev_snap.size_bytes != curr_snap.size_bytes {
                        modified.push(path.clone());
                    }
                }
            }
        }
        for path in self.files.keys() {
            if !current.files.contains_key(path) { deleted.push(path.clone()); }
        }
        added.sort(); modified.sort(); deleted.sort();
        WorldStateDiff { added_files: added, modified_files: modified, deleted_files: deleted }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_world_state_diff() {
        let mut prev_files = HashMap::new();
        prev_files.insert("unchanged.txt".to_string(), FileSnapshot {
            relative_path: "unchanged.txt".to_string(), size_bytes: 10,
            modified_secs: 100, metadata_fingerprint: "10_100".to_string(), content_hash: "aaa".to_string(),
        });
        prev_files.insert("deleted.txt".to_string(), FileSnapshot {
            relative_path: "deleted.txt".to_string(), size_bytes: 20,
            modified_secs: 100, metadata_fingerprint: "20_100".to_string(), content_hash: "bbb".to_string(),
        });

        let prev = WorldState {
            timestamp_secs: 100, workspace_root: "/test".to_string(),
            files: prev_files, environment: EnvSnapshot::default(), git: None,
        };

        let mut curr_files = HashMap::new();
        curr_files.insert("unchanged.txt".to_string(), FileSnapshot {
            relative_path: "unchanged.txt".to_string(), size_bytes: 10,
            modified_secs: 100, metadata_fingerprint: "10_100".to_string(), content_hash: "aaa".to_string(),
        });
        curr_files.insert("added.txt".to_string(), FileSnapshot {
            relative_path: "added.txt".to_string(), size_bytes: 30,
            modified_secs: 200, metadata_fingerprint: "30_200".to_string(), content_hash: "ccc".to_string(),
        });

        let curr = WorldState {
            timestamp_secs: 200, workspace_root: "/test".to_string(),
            files: curr_files, environment: EnvSnapshot::default(), git: None,
        };

        let diff = prev.diff(&curr);
        assert_eq!(diff.added_files, vec!["added.txt"]);
        assert_eq!(diff.deleted_files, vec!["deleted.txt"]);
        assert!(diff.has_changes());
    }
}
