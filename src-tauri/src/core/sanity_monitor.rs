use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

/// Health report of the agent's cognitive state
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SanityReport {
    /// 0.0 = incoherent/stuck, 1.0 = fully healthy
    pub coherence_score: f32,
    pub stall_detected: bool,
    pub ram_pressure_pct: f32,
    pub consecutive_same_tool: u32,
    pub recommendation: String,
    pub level: String,  // "GREEN" | "YELLOW" | "RED"
    /// When RED, the harness should force this tool next turn (not just inject text)
    pub forced_tool_override: Option<String>,
}

impl Default for SanityReport {
    fn default() -> Self {
        Self {
            coherence_score: 1.0,
            stall_detected: false,
            ram_pressure_pct: 0.0,
            consecutive_same_tool: 0,
            recommendation: "Sistema operando con normalidad.".to_string(),
            level: "GREEN".to_string(),
            forced_tool_override: None,
        }
    }
}

/// Checks the agent's cognitive health.
/// Called every 5 steps from run_agent_loop.
///
/// `last_error_hashes`: ring buffer of recent terminal output hashes (last 5).
///   Caller builds this by hashing each new terminal-output string and pushing.
pub fn check(
    tool_history: &[String],     // Last 10 tools chosen
    context_size: usize,         // Bytes in current_context
    json_error_count: u32,
    step_count: u32,
    last_step_with_progress: u32,
    last_error_hashes: &[u64],   // Recent terminal output hashes (newest last)
) -> SanityReport {
    let mut score: f32 = 1.0;
    let mut issues: Vec<String> = Vec::new();
    let mut level = "GREEN";
    let mut forced_tool: Option<String> = None;

    // ─── 1. Repetition / Loop Detection ──────────────────────────────────
    let consecutive_same = count_consecutive_same(tool_history);
    if consecutive_same >= 4 {
        score -= 0.4;
        issues.push(format!("🔁 Loop detectado: herramienta '{}' repetida {} veces.", 
            tool_history.last().cloned().unwrap_or_default(), consecutive_same));
        level = "RED";
        forced_tool = Some("TOOL_THINK".to_string());
    } else if consecutive_same >= 2 {
        score -= 0.15;
        issues.push(format!("⚠️ Posible loop: herramienta repetida {} veces.", consecutive_same));
        if level != "RED" { level = "YELLOW"; }
    }

    // ─── 2. Semantic Error Loop Detection (same error output ≥3 times) ───
    //    If the last 3 hashes are identical, the agent is getting the same
    //    error output regardless of what it tries — escalate immediately.
    if last_error_hashes.len() >= 3 {
        let recent = &last_error_hashes[last_error_hashes.len().saturating_sub(3)..];
        if recent.iter().all(|&h| h == recent[0] && h != 0) {
            score -= 0.45;
            issues.push(format!(
                "🔴 ERROR SEMÁNTICO REPETIDO: La misma salida de error apareció {} veces consecutivas. \
                 El agente está atascado en un loop de corrección sin cambio real. \
                 ACCIÓN: Usa TOOL_THINK para reconsiderar la estrategia completamente. \
                 Si es un script de verificación, usa regex flexible en lugar de coincidencia exacta.",
                recent.len()
            ));
            level = "RED";
            forced_tool = Some("TOOL_THINK".to_string());
        }
    }

    // ─── 3. JSON Parse Failures ───────────────────────────────────────────
    if json_error_count >= 3 {
        score -= 0.3;
        issues.push(format!("⚠️ {} errores de JSON consecutivos — LLM podría estar degradado.", json_error_count));
        if level != "RED" { level = "YELLOW"; }
    }

    // ─── 4. Mission Stall (no progress in last 10 steps) ─────────────────
    //    FIX: Only update level to YELLOW if not already RED (stall can't downgrade RED)
    let stall = step_count > last_step_with_progress + 10;
    if stall {
        score -= 0.25;
        issues.push(format!("⏱️ Sin progreso detectable en {} pasos.", step_count - last_step_with_progress));
        if level != "RED" { level = "YELLOW"; }  // FIX: was unconditional, could downgrade RED
    }

    // ─── 5. Context Bloat ─────────────────────────────────────────────────
    let context_kb = context_size / 1024;
    if context_kb > 32 {
        score -= 0.1;
        issues.push(format!("📦 Contexto muy grande: {}KB — considera comprimir historial.", context_kb));
    }

    // ─── 6. RAM Pressure ──────────────────────────────────────────────────
    let ram_pct = get_ram_usage_pct();
    if ram_pct > 90.0 {
        score -= 0.2;
        issues.push(format!("🔴 RAM al {:.0}% — riesgo de OOM.", ram_pct));
        level = "RED";
        if forced_tool.is_none() { forced_tool = Some("TOOL_FINISH".to_string()); }
    } else if ram_pct > 75.0 {
        score -= 0.05;
        issues.push(format!("🟡 RAM al {:.0}% — presión moderada.", ram_pct));
    }

    score = score.max(0.0);

    let recommendation = if issues.is_empty() {
        "Sistema operando con normalidad.".to_string()
    } else {
        issues.join(" | ")
    };

    SanityReport {
        coherence_score: score,
        stall_detected: stall,
        ram_pressure_pct: ram_pct,
        consecutive_same_tool: consecutive_same,
        recommendation,
        level: level.to_string(),
        forced_tool_override: if level == "RED" { forced_tool } else { None },
    }
}

/// Emits the sanity report to the frontend UI
pub fn emit_report(app: &AppHandle, report: &SanityReport) {
    let _ = app.emit("sanity-report", serde_json::json!({
        "coherence": report.coherence_score,
        "stall": report.stall_detected,
        "ram": report.ram_pressure_pct,
        "level": report.level,
        "recommendation": report.recommendation,
    }));
}

/// Builds a context injection string when anomalies are detected.
/// Returns (hint_text, forced_tool_name_option).
/// When level is RED, the caller MUST set forced_next_tool — not just inject text.
pub fn build_correction_hint(report: &SanityReport) -> Option<(String, Option<String>)> {
    if report.level == "GREEN" { return None; }

    let hint = format!(
        "[⚕️ MONITOR DE CORDURA — NIVEL {}]\n{}\n\
         ACCIÓN REQUERIDA: Revisa tu checklist mental. Si estás en un loop, usa TOOL_THINK \
         para reflexionar sobre el problema desde cero antes de continuar.\n\n",
        report.level, report.recommendation
    );
    Some((hint, report.forced_tool_override.clone()))
}

// ─── Internal helpers ────────────────────────────────────────────────────────

fn count_consecutive_same(tools: &[String]) -> u32 {
    if tools.is_empty() { return 0; }
    let last = tools.last().unwrap();
    tools.iter().rev().take_while(|t| *t == last).count() as u32
}

fn get_ram_usage_pct() -> f32 {
    use sysinfo::System;
    let mut sys = System::new();
    sys.refresh_memory();
    let total = sys.total_memory() as f32;
    let used  = sys.used_memory() as f32;
    if total == 0.0 { return 0.0; }
    (used / total) * 100.0
}

