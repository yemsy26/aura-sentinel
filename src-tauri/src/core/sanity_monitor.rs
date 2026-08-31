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
        }
    }
}

/// Checks the agent's cognitive health.
/// Called every 5 steps from run_agent_loop.
pub fn check(
    tool_history: &[String],     // Last 10 tools chosen
    context_size: usize,         // Bytes in current_context
    json_error_count: u32,
    step_count: u32,
    last_step_with_progress: u32,
) -> SanityReport {
    let mut score: f32 = 1.0;
    let mut issues: Vec<String> = Vec::new();
    let mut level = "GREEN";

    // ─── 1. Repetition / Loop Detection ──────────────────────────────────
    let consecutive_same = count_consecutive_same(tool_history);
    if consecutive_same >= 4 {
        score -= 0.4;
        issues.push(format!("🔁 Loop detectado: herramienta '{}' repetida {} veces.", 
            tool_history.last().cloned().unwrap_or_default(), consecutive_same));
        level = "RED";
    } else if consecutive_same >= 2 {
        score -= 0.15;
        issues.push(format!("⚠️ Posible loop: herramienta repetida {} veces.", consecutive_same));
        level = "YELLOW";
    }

    // ─── 2. JSON Parse Failures ───────────────────────────────────────────
    if json_error_count >= 3 {
        score -= 0.3;
        issues.push(format!("⚠️ {} errores de JSON consecutivos — LLM podría estar degradado.", json_error_count));
        level = if level == "RED" { "RED" } else { "YELLOW" };
    }

    // ─── 3. Mission Stall (no progress in last 10 steps) ─────────────────
    let stall = step_count > last_step_with_progress + 10;
    if stall {
        score -= 0.25;
        issues.push(format!("⏱️ Sin progreso detectable en {} pasos.", step_count - last_step_with_progress));
        level = "YELLOW";
    }

    // ─── 4. Context Bloat ─────────────────────────────────────────────────
    let context_kb = context_size / 1024;
    if context_kb > 32 {
        score -= 0.1;
        issues.push(format!("📦 Contexto muy grande: {}KB — considera comprimir historial.", context_kb));
    }

    // ─── 5. RAM Pressure ──────────────────────────────────────────────────
    let ram_pct = get_ram_usage_pct();
    if ram_pct > 90.0 {
        score -= 0.2;
        issues.push(format!("🔴 RAM al {:.0}% — riesgo de OOM.", ram_pct));
        level = "RED";
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
/// Injects guidance for the Planner to self-correct.
pub fn build_correction_hint(report: &SanityReport) -> Option<String> {
    if report.level == "GREEN" { return None; }

    Some(format!(
        "[⚕️ MONITOR DE CORDURA — NIVEL {}]\n{}\n\
         ACCIÓN REQUERIDA: Revisa tu checklist mental. Si estás en un loop, usa TOOL_THINK \
         para reflexionar sobre el problema desde cero antes de continuar.\n\n",
        report.level, report.recommendation
    ))
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
