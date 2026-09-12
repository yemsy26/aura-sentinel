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
    /// and guarantees the output never exceeds `max_chars`.
    pub fn compact_context(&self, context: &str, mission_state: &str) -> String {
        if context.len() <= self.max_chars {
            return context.to_string();
        }

        // Anchor 1: Task Charter header
        let charter_block = if !self.task_charter.is_empty() {
            format!("[🎯 OBJETIVO INMUTABLE DE LA MISIÓN]\n{}\n\n", self.task_charter)
        } else {
            String::new()
        };

        // Anchor 2: Mission operational state
        let mission_state_block = format!("\n[ESTADO OPERACIONAL DE LA MISIÓN]\n{}\n\n", mission_state);

        let notice_marker = "\n[... ✂️ HISTORIAL ANTIGUO ELIMINADO PARA PREVENIR ECHOLALIA ...]\n\n";

        let fixed_overhead = charter_block.len() + mission_state_block.len() + notice_marker.len();
        let tail_budget = self.max_chars.saturating_sub(fixed_overhead);

        let tail: String = context.chars().rev().take(tail_budget).collect::<Vec<_>>().into_iter().rev().collect();

        // Clean turn boundary search in tail
        let clean_tail = if let Some(pos) = tail.find("[PASO") {
            &tail[pos..]
        } else if let Some(pos) = tail.find("\n[") {
            &tail[pos + 1..]
        } else {
            &tail
        };

        let mut result = format!(
            "{}{}{}{}",
            charter_block,
            mission_state_block,
            notice_marker,
            clean_tail.trim_start()
        );

        if result.len() > self.max_chars {
            result.truncate(self.max_chars);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_monitor_guarantees_max_chars_bound() {
        let monitor = ContextMonitor::new(500, "Construir API REST en Rust");
        let huge_context = "x".repeat(5000);
        let mission_state = "Paso 1 completado, Paso 2 en curso";

        let compacted = monitor.compact_context(&huge_context, mission_state);
        assert!(compacted.len() <= 500, "Compacted length {} exceeds max_chars 500", compacted.len());
        assert!(compacted.contains("Construir API REST en Rust"), "Immutable task charter must be preserved");
    }

    #[test]
    fn test_context_monitor_no_compact_when_within_budget() {
        let monitor = ContextMonitor::new(1000, "Mi objetivo");
        let short_context = "Contexto corto";
        assert_eq!(monitor.compact_context(short_context, "Estado"), short_context);
    }
}
