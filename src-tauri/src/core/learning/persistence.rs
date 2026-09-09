use std::io::Write;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use crate::core::learning::experience::{Experience, ExperienceStoreV2};
use crate::core::learning::stats::{ModelStats, StrategyStats};

const EXPERIENCES_FILE: &str = "learning_experiences.jsonl";
const MODEL_STATS_FILE: &str = "model_stats.json";
const STRATEGY_STATS_FILE: &str = "strategy_stats.json";

fn data_dir() -> PathBuf {
    let base = std::env::var("AURA_DATA_DIR").unwrap_or_else(|_| {
        let mut p = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        if p.ends_with("src-tauri") {
            p = p.parent().unwrap_or(&p).to_path_buf();
        }
        p.to_string_lossy().to_string()
    });
    PathBuf::from(base).join("data").join("learning")
}

pub struct LearningPersistence {
    dir: PathBuf,
}

impl LearningPersistence {
    pub fn new() -> Self {
        let dir = data_dir();
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    pub fn with_dir(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    pub fn experiences_path(&self) -> PathBuf { self.dir.join(EXPERIENCES_FILE) }
    fn model_stats_path(&self) -> PathBuf { self.dir.join(MODEL_STATS_FILE) }
    fn strategy_stats_path(&self) -> PathBuf { self.dir.join(STRATEGY_STATS_FILE) }

    /// Load ExperienceStoreV2. Corrupt lines are silently skipped — cold-start safe.
    pub fn load_experiences(&self, max_entries: usize) -> ExperienceStoreV2 {
        let mut store = ExperienceStoreV2::new(max_entries);
        let path = self.experiences_path();
        if !path.exists() { return store; }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return store,
        };

        let mut all: Vec<Experience> = Vec::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() { continue; }
            if let Ok(exp) = serde_json::from_str::<Experience>(trimmed) {
                all.push(exp);
            }
            // corrupt lines skipped silently — fallback to whatever is valid
        }

        let start = if all.len() > max_entries { all.len() - max_entries } else { 0 };
        for exp in all.into_iter().skip(start) {
            store.push(exp);
        }
        store
    }

    /// Truly atomic JSONL append: read all + new → write to unique .tmp → fsync → rename.
    /// If process dies before rename, .tmp is orphaned and main file is intact.
    pub async fn append_experience(&self, exp: &Experience) -> Result<(), String> {
        let path = self.experiences_path();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp_path = self.dir.join(format!("exp_{}_{:x}.tmp", std::process::id(), nanos));

        // 1. Read existing content (empty string if file doesn't exist yet)
        let existing = if path.exists() {
            std::fs::read_to_string(&path)
                .map_err(|e| format!("PERSIST_READ: {}", e))?
        } else {
            String::new()
        };

        // 2. Serialize new experience
        let new_line = serde_json::to_string(exp)
            .map_err(|e| format!("PERSIST_SERIALIZE: {}", e))?;

        // 3. Write existing + new line to .tmp
        {
            let mut f = std::fs::File::create(&tmp_path)
                .map_err(|e| format!("PERSIST_TMP_CREATE: {}", e))?;
            if !existing.is_empty() {
                f.write_all(existing.as_bytes())
                    .map_err(|e| format!("PERSIST_TMP_WRITE_EXISTING: {}", e))?;
            }
            writeln!(f, "{}", new_line)
                .map_err(|e| format!("PERSIST_TMP_WRITELN: {}", e))?;
            f.sync_all()
                .map_err(|e| format!("PERSIST_TMP_FSYNC: {}", e))?;
        }

        // 4. Truly atomic replace (cross-platform with Windows atomic support)
        atomic_replace(&tmp_path, &path).await
    }

    pub async fn save_model_stats(
        &self, stats: &HashMap<String, ModelStats>,
    ) -> Result<(), String> {
        self.atomic_json_write(&self.model_stats_path(), stats).await
    }

    pub async fn save_strategy_stats(
        &self, stats: &HashMap<String, StrategyStats>,
    ) -> Result<(), String> {
        self.atomic_json_write(&self.strategy_stats_path(), stats).await
    }

    pub fn load_model_stats(&self) -> HashMap<String, ModelStats> {
        self.load_json(&self.model_stats_path()).unwrap_or_default()
    }

    pub fn load_strategy_stats(&self) -> HashMap<String, StrategyStats> {
        self.load_json(&self.strategy_stats_path()).unwrap_or_default()
    }

    async fn atomic_json_write<T: serde::Serialize>(
        &self, path: &Path, value: &T,
    ) -> Result<(), String> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp = self.dir.join(format!("stats_{}_{:x}.tmp", std::process::id(), nanos));
        let json = serde_json::to_string_pretty(value)
            .map_err(|e| format!("PERSIST_SERIALIZE: {}", e))?;
        {
            let mut f = std::fs::File::create(&tmp)
                .map_err(|e| format!("PERSIST_TMP_CREATE: {}", e))?;
            f.write_all(json.as_bytes())
                .map_err(|e| format!("PERSIST_TMP_WRITE: {}", e))?;
            f.sync_all()
                .map_err(|e| format!("PERSIST_TMP_FSYNC: {}", e))?;
        }
        atomic_replace(&tmp, path).await
    }

    fn load_json<T: serde::de::DeserializeOwned>(&self, path: &Path) -> Option<T> {
        let raw = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&raw).ok()
    }
}

/// Atomically replaces target with src.
/// On Windows, std::fs::rename fails if target already exists.
/// This helper tries rename first (atomic on Unix and when target does not exist).
/// On failure on Windows, it creates a backup, renames, and cleans up, with retries.
async fn atomic_replace(src: &Path, target: &Path) -> Result<(), String> {
    let mut attempts = 0;
    loop {
        // Fast path: rename succeeds if target doesn't exist or on Unix
        match std::fs::rename(src, target) {
            Ok(_) => return Ok(()),
            Err(e) => {
                #[cfg(windows)]
                {
                    // Windows target-exists workaround:
                    // If target exists, rename target to a transient backup, rename src to target, then remove backup.
                    if target.exists() {
                        let nanos = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_nanos())
                            .unwrap_or(0);
                        let backup = target.with_extension(format!("bak_{:x}", nanos));
                        if std::fs::rename(target, &backup).is_ok() {
                            match std::fs::rename(src, target) {
                                Ok(_) => {
                                    let _ = std::fs::remove_file(&backup);
                                    return Ok(());
                                }
                                Err(err) => {
                                    // Rollback target from backup
                                    let _ = std::fs::rename(&backup, target);
                                    let _ = std::fs::remove_file(src);
                                    return Err(format!("PERSIST_REPLACE_ROLLBACK: {}", err));
                                }
                            }
                        }
                    }
                }

                if attempts < 5 {
                    attempts += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(15)).await;
                } else {
                    let _ = std::fs::remove_file(src);
                    return Err(format!("PERSIST_RENAME: {}", e));
                }
            }
        }
    }
}

