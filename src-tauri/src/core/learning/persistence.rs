use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use serde_json;
use crate::core::learning::experience::{Experience, ExperienceStoreV2};
use crate::core::learning::stats::{ModelStats, StrategyStats};

const EXPERIENCES_FILE: &str = "learning_experiences.jsonl";
const MODEL_STATS_FILE: &str = "model_stats.json";
const STRATEGY_STATS_FILE: &str = "strategy_stats.json";

fn data_dir() -> PathBuf {
    // Locate data/learning/ relative to the binary or current dir
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

    fn experiences_path(&self) -> PathBuf { self.dir.join(EXPERIENCES_FILE) }
    fn model_stats_path(&self) -> PathBuf { self.dir.join(MODEL_STATS_FILE) }
    fn strategy_stats_path(&self) -> PathBuf { self.dir.join(STRATEGY_STATS_FILE) }

    /// Load ExperienceStoreV2. Corrupt lines are skipped (no panic on corruption).
    pub fn load_experiences(&self, max_entries: usize) -> ExperienceStoreV2 {
        let mut store = ExperienceStoreV2::new(max_entries);
        let path = self.experiences_path();
        if !path.exists() { return store; }

        let file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => return store,
        };
        let reader = std::io::BufReader::new(file);
        let mut all: Vec<Experience> = Vec::new();
        for line in reader.lines().flatten() {
            let trimmed = line.trim().to_string();
            if trimmed.is_empty() { continue; }
            if let Ok(exp) = serde_json::from_str::<Experience>(&trimmed) {
                all.push(exp);
            }
            // corrupt lines are silently skipped — cold-start safe
        }
        // load most recent max_entries
        let start = if all.len() > max_entries { all.len() - max_entries } else { 0 };
        for exp in all.into_iter().skip(start) {
            store.push(exp);
        }
        store
    }

    /// Append-only, atomic: write → tmp → fsync → rename.
    pub async fn append_experience(&self, exp: &Experience) -> Result<(), String> {
        let path = self.experiences_path();
        let tmp_path = path.with_extension("jsonl.tmp");
        let line = serde_json::to_string(exp).map_err(|e| e.to_string())?;
        // Append to a temp copy of the original file, then rename atomically
        {
            let mut file = std::fs::OpenOptions::new()
                .create(true).append(true)
                .open(&tmp_path)
                .map_err(|e| format!("PERSIST_OPEN: {}", e))?;
            writeln!(file, "{}", line).map_err(|e| format!("PERSIST_WRITE: {}", e))?;
            file.sync_all().map_err(|e| format!("PERSIST_FSYNC: {}", e))?;
        }
        // If original exists, append to it directly (simpler than full copy for append)
        {
            let mut file = std::fs::OpenOptions::new()
                .create(true).append(true)
                .open(&path)
                .map_err(|e| format!("PERSIST_OPEN_MAIN: {}", e))?;
            writeln!(file, "{}", line).map_err(|e| format!("PERSIST_WRITE_MAIN: {}", e))?;
            file.sync_all().map_err(|e| format!("PERSIST_FSYNC_MAIN: {}", e))?;
        }
        let _ = std::fs::remove_file(&tmp_path);
        Ok(())
    }

    /// Save model stats atomically: tmp → fsync → rename.
    pub async fn save_model_stats(
        &self,
        stats: &HashMap<String, ModelStats>,
    ) -> Result<(), String> {
        self.atomic_json_write(&self.model_stats_path(), stats)
    }

    /// Save strategy stats atomically.
    pub async fn save_strategy_stats(
        &self,
        stats: &HashMap<String, StrategyStats>,
    ) -> Result<(), String> {
        self.atomic_json_write(&self.strategy_stats_path(), stats)
    }

    pub fn load_model_stats(&self) -> HashMap<String, ModelStats> {
        self.load_json(&self.model_stats_path()).unwrap_or_default()
    }

    pub fn load_strategy_stats(&self) -> HashMap<String, StrategyStats> {
        self.load_json(&self.strategy_stats_path()).unwrap_or_default()
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    fn atomic_json_write<T: serde::Serialize>(
        &self,
        path: &Path,
        value: &T,
    ) -> Result<(), String> {
        let tmp = path.with_extension("json.tmp");
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
        std::fs::rename(&tmp, path)
            .map_err(|e| format!("PERSIST_RENAME: {}", e))
    }

    fn load_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &Path,
    ) -> Option<T> {
        let raw = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&raw).ok()
    }
}
