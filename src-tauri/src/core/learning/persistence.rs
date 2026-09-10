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

#[derive(Clone, Debug)]
pub struct LearningPersistence {
    dir: PathBuf,
}

impl LearningPersistence {
    pub fn new() -> Self {
        let dir = data_dir();
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    #[allow(dead_code)]
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

    /// Persist the StateStrategyIndex as a derived cache (AL-v2.3).
    /// Corruption-safe: if this file is missing, it is rebuilt from experiences.
    pub async fn save_state_strategy_index(
        &self,
        index: &crate::core::learning::state_stats::StateStrategyIndex,
    ) -> Result<(), String> {
        self.atomic_json_write(&self.dir.join("state_strategy_index.json"), index).await
    }

    #[allow(dead_code)]
    pub fn load_state_strategy_index(&self) -> crate::core::learning::state_stats::StateStrategyIndex {
        self.load_json(&self.dir.join("state_strategy_index.json"))
            .unwrap_or_default()
    }

    /// Persist the RecoveryIndex as a derived cache (AL-v2.4).
    pub async fn save_recovery_index(
        &self,
        index: &crate::core::learning::recovery_index::RecoveryIndex,
    ) -> Result<(), String> {
        self.atomic_json_write(&self.dir.join("recovery_index.json"), index).await
    }

    #[allow(dead_code)]
    pub fn load_recovery_index(&self) -> crate::core::learning::recovery_index::RecoveryIndex {
        self.load_json(&self.dir.join("recovery_index.json"))
            .unwrap_or_default()
    }

    /// Persist the BudgetAwareIndex as a derived cache (AL-v2.5).
    pub async fn save_budget_index(
        &self,
        index: &crate::core::learning::budget_stats::BudgetAwareIndex,
    ) -> Result<(), String> {
        self.atomic_json_write(&self.dir.join("budget_index.json"), index).await
    }

    #[allow(dead_code)]
    pub fn load_budget_index(&self) -> crate::core::learning::budget_stats::BudgetAwareIndex {
        self.load_json(&self.dir.join("budget_index.json"))
            .unwrap_or_default()
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

/// Atomically replaces target with src asynchronously.
/// On Unix, std::fs::rename is atomically replacing target if it already exists.
/// On Windows, std::fs::rename fails if target exists. We invoke Windows' native ReplaceFileW
/// API, which is atomic at the filesystem/NTFS level without exposing an intermediate missing-target window.
pub(crate) async fn atomic_replace(src: &Path, target: &Path) -> Result<(), String> {
    atomic_replace_sync(src, target)
}

/// Atomically replaces target with src synchronously.
pub(crate) fn atomic_replace_sync(src: &Path, target: &Path) -> Result<(), String> {
    let mut attempts = 0;
    loop {
        // Fast path: if target does not exist, rename is atomic on all platforms.
        if !target.exists() {
            match std::fs::rename(src, target) {
                Ok(_) => {
                    if target.file_name().and_then(|n| n.to_str()).map_or(false, |s| s.starts_with('.')) {
                        crate::core::hide_file_windows_sync(target);
                    }
                    return Ok(());
                }
                Err(_e) if attempts < 5 => {
                    attempts += 1;
                    std::thread::sleep(std::time::Duration::from_millis(15));
                    continue;
                }
                Err(e) => {
                    let _ = std::fs::remove_file(src);
                    return Err(format!("PERSIST_RENAME_NEW: {}", e));
                }
            }
        }

        // Target already exists:
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            let mut target_wide: Vec<u16> = target.as_os_str().encode_wide().collect();
            target_wide.push(0);
            let mut src_wide: Vec<u16> = src.as_os_str().encode_wide().collect();
            src_wide.push(0);

            extern "system" {
                fn ReplaceFileW(
                    lpReplacedFileName: *const u16,
                    lpReplacementFileName: *const u16,
                    lpBackupFileName: *const u16,
                    dwReplaceFlags: u32,
                    lpExclude: *const std::ffi::c_void,
                    lpReserved: *const std::ffi::c_void,
                ) -> i32;
            }

            // ReplaceFileW atomically replaces target with replacement file.
            let success = unsafe {
                ReplaceFileW(
                    target_wide.as_ptr(),
                    src_wide.as_ptr(),
                    std::ptr::null(),
                    0,
                    std::ptr::null(),
                    std::ptr::null(),
                )
            };

            if success != 0 {
                if target.file_name().and_then(|n| n.to_str()).map_or(false, |s| s.starts_with('.')) {
                    crate::core::hide_file_windows_sync(target);
                }
                return Ok(());
            }

            let win_err = std::io::Error::last_os_error();
            if attempts < 5 {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(15));
                continue;
            } else {
                let _ = std::fs::remove_file(src);
                return Err(format!("PERSIST_REPLACE_FILE_W: {}", win_err));
            }
        }

        #[cfg(not(windows))]
        {
            match std::fs::rename(src, target) {
                Ok(_) => return Ok(()),
                Err(_e) if attempts < 5 => {
                    attempts += 1;
                    std::thread::sleep(std::time::Duration::from_millis(15));
                }
                Err(e) => {
                    let _ = std::fs::remove_file(src);
                    return Err(format!("PERSIST_RENAME: {}", e));
                }
            }
        }
    }
}

