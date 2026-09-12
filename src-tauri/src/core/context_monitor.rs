/// Context window monitor and intelligent compaction for Aura Sentinel.
/// Implements Devin 2.0 / OSS 2025-2026 Tiered Context Management:
///  - Healthy (< 70% of budget)
///  - ApproachingLimit (70-80%): proactive alert
///  - Degraded (80-90%): deterministic compaction
///  - CriticalHandoff (> 90%): forced compaction to prevent LLM saturation and hallucination
///
/// Guarantees that the Immutable Task Charter (original user prompt/objective)
/// is NEVER lost during sliding-window compaction.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ContextStatus {
    Healthy,
    ApproachingLimit,
    Degraded,
    CriticalHandoff,
}

#[derive(Debug, Clone)]
pub struct ContextMonitor {
    pub max_chars: usize,
    pub task_charter: String,
}

impl ContextMonitor {
    pub fn new(max_chars: usize, task_charter: &str) -> Self {
        Self {
            max_chars: if max_chars == 0 { 6000 } else { max_chars },
            task_charter: task_charter.trim().to_string(),
        }
    }

    /// Calculate context fill percentage and status tier
    pub fn status(&self, current_len: usize) -> (f32, ContextStatus) {
        let fill = current_len as f32 / self.max_chars as f32;
        let status = if fill < 0.70 {
            ContextStatus::Healthy
        } else if fill < 0.80 {
            ContextStatus::ApproachingLimit
        } else if fill < 0.90 {
            ContextStatus::Degraded
        } else {
            ContextStatus::CriticalHandoff
        };
        (fill, status)
    }

    /// Check if context should be compacted
    pub fn should_compact(&self, current_len: usize) -> bool {
        current_len > self.max_chars
    }

    /// Compacting logic that strictly preserves the Immutable Task Charter
    pub fn compact_context(&self, context: &str) -> String {
        if context.len() <= self.max_chars {
            return context.to_string();
        }

        // Anchor 1: Task Charter header
        let charter_block = if !self.task_charter.is_empty() {
            format!("[🎯 OBJETIVO INMUTABLE DE LA MISIÓN]
{}

", self.task_charter)
        } else {
            String::new()
        };

        // Budget allocation: 85% tail (latest actions & errors). We DROP the head because 
        // small LLMs (like 7B) suffer from Echolalia when they see their very first actions (e.g. "workspace is empty") permanently pinned to the top.
        let tail_budget = (self.max_chars as f32 * 0.85) as usize;

        let tail: String = context.chars().rev().take(tail_budget).collect::<Vec<_>>().into_iter().rev().collect();

        // Clean turn boundary search in tail
        let clean_tail = if let Some(pos) = tail.find("[PASO") {
            &tail[pos..]
        } else if let Some(pos) = tail.find("
[") {
            &tail[pos + 1..]
        } else {
            &tail
        };

        format!(
            "{}

[... ✂️ HISTORIAL ANTIGUO ELIMINADO PARA PREVENIR ECHOLALIA ({} caracteres eliminados) ...]

{}",
            charter_block,
            context.len().saturating_sub(tail_budget),
            clean_tail.trim_start()
        )
    }
}
