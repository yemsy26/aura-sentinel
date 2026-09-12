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

    pub fn sanitize_tail(tail: &str) -> String {
        let stale_phrases = [
            "workspace está vacío",
            "workspace esta vacio",
            "workspace no tiene archivos",
            "no hay archivos en el workspace",
            "el workspace se encuentra vacío",
            "el workspace se encuentra vacio",
            "proyecto está completamente vacío",
            "proyecto esta completamente vacio",
        ];

        tail.lines()
            .map(|line| {
                let lower = line.to_lowercase();
                let contains_stale = stale_phrases.iter().any(|phrase| lower.contains(phrase));
                if contains_stale {
                    "[OBSOLETO: Afirmación de workspace vacío purgada por Runtime - Ver Mission State Anchor]"
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Compacting logic that strictly preserves the Immutable Task Charter,
    /// the Authoritative Mission State Anchor, and purges stale hallucinated statements from the tail.
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

        // Anchor 2: Mission operational state / Mission State Anchor
        let mission_state_block = format!("\n[ESTADO AUTORITATIVO DEL RUNTIME — MISSION STATE ANCHOR]\n{}\n\n", mission_state);

        let notice_marker = "\n[... ✂️ HISTORIAL ANTIGUO ELIMINADO Y SANITIZADO PARA PREVENIR ECHOLALIA ...]\n\n";

        let fixed_overhead = charter_block.len() + mission_state_block.len() + notice_marker.len();
        let tail_budget = self.max_chars.saturating_sub(fixed_overhead);

        let raw_tail: String = context.chars().rev().take(tail_budget).collect::<Vec<_>>().into_iter().rev().collect();

        // Clean turn boundary search in tail
        let clean_tail = if let Some(pos) = raw_tail.find("[PASO") {
            &raw_tail[pos..]
        } else if let Some(pos) = raw_tail.find("\n[") {
            &raw_tail[pos + 1..]
        } else {
            &raw_tail
        };

        // Sanitize tail to purge stale hallucinations ("workspace está vacío")
        let sanitized_tail = Self::sanitize_tail(clean_tail.trim_start());

        let mut result = format!(
            "{}{}{}{}",
            charter_block,
            mission_state_block,
            notice_marker,
            sanitized_tail
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

    #[test]
    fn test_context_monitor_preserves_mission_state() {
        let monitor = ContextMonitor::new(800, "Construir Cyber Sentinel");
        let huge_context = "Pasos anteriores... ".repeat(100);
        let mission_state = "Archivos físicos confirmados en disco: cyber_sentinel.html, style.css";

        let compacted = monitor.compact_context(&huge_context, mission_state);
        assert!(compacted.contains("cyber_sentinel.html, style.css"), "MissionState with physical files must be preserved");
    }

    #[test]
    fn test_compaction_with_state_anchor_purges_stale_echos() {
        let monitor = ContextMonitor::new(600, "Construir Cyber Sentinel");
        let huge_context = format!(
            "{}\n[PASO 1]: El workspace está vacío, por lo que necesito crear los archivos necesarios.\n[PASO 2]: Ejecutando...",
            "Contexto largo para forzar compactacion. ".repeat(40)
        );
        let anchor_block = "cyber_sentinel.html, style.css existen en disco.";

        let compacted = monitor.compact_context(&huge_context, anchor_block);
        assert!(compacted.contains("cyber_sentinel.html, style.css existen en disco."));
        assert!(!compacted.contains("El workspace está vacío"));
        assert!(compacted.contains("Afirmación de workspace vacío purgada"));
    }
}
