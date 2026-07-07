/// Error classification engine for Aura-Sentinel's self-repair loop.
///
/// Professional agents (SWE-agent, AutoGen, Claude) classify errors into 3 types
/// and choose different recovery strategies for each:
///
///  - TRANSIENT → retry automatically (network blip, lock, timeout)
///  - LOGIC     → change approach (syntax error, missing file, wrong command)
///  - BLOCKED   → escalate to user (missing credential, not installed, permission)
///
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub enum ErrorType {
    /// Temporary condition — retry the same action up to 3 times.
    Transient,
    /// Logic/code error — force a THINK step to reason about the fix, then retry once.
    Logic,
    /// Hard blocker — the agent cannot resolve this alone; escalate to user.
    Blocked,
}

/// Classify a terminal error into one of the three error types.
pub fn classify_error(stderr: &str, stdout: &str, exit_code: i32) -> ErrorType {
    let combined = format!("{} {}", stderr, stdout).to_lowercase();

    // ── BLOCKED errors (cannot be fixed by the agent alone) ────────────────
    let blocked_signals = [
        "access denied", "access is denied",
        "permission denied",
        "acceso denegado",
        "no se reconoce como un comando interno",   // command not found on PATH
        "is not recognized as",
        "not found in path",
        "credential", "authentication", "unauthorized",
        "not installed", "no instalado",
        "requires administrator", "elevate",
        "cannot find the file specified",           // binary truly missing
    ];
    for sig in &blocked_signals {
        if combined.contains(sig) {
            return ErrorType::Blocked;
        }
    }

    // ── TRANSIENT errors (retry same action) ───────────────────────────────
    let transient_signals = [
        "timeout", "timed out",
        "connection refused", "connection reset",
        "temporary failure",
        "resource temporarily unavailable",
        "econnreset", "epipe",
        "locked", "file is locked", "being used by another process",
        "try again",
    ];
    for sig in &transient_signals {
        if combined.contains(sig) {
            return ErrorType::Transient;
        }
    }

    // ── LOGIC errors (change strategy) ────────────────────────────────────
    // These are the most common during code generation tasks.
    // A non-zero exit code with stderr content that doesn't match above = Logic.
    if exit_code != 0 && !stderr.trim().is_empty() {
        return ErrorType::Logic;
    }

    // Fallback: treat as Logic so the agent reasons about what happened
    ErrorType::Logic
}

/// Given an error classification, produce the self-repair feedback message
/// that will be injected into the LLM's context before the next turn.
pub fn repair_prompt(
    error_type: &ErrorType,
    tool_name: &str,
    command: &str,
    stderr: &str,
    attempt: u32,
) -> String {
    match error_type {
        ErrorType::Transient => format!(
            "[SELF-REPAIR] Error transitorio en '{}' (intento {}/3).\n\
            Comando: '{}'\n\
            Error: {}\n\
            ACCIÓN: Reintenta el mismo comando. Si falla 3 veces, cambia de estrategia.",
            tool_name, attempt, command, stderr.trim()
        ),

        ErrorType::Logic => format!(
            "[SELF-REPAIR] Error lógico detectado en '{}'.\n\
            Comando que falló: '{}'\n\
            Error recibido: {}\n\
            DIAGNÓSTICO REQUERIDO: Antes de reintentar, debes usar TOOL_THINK para:\n\
            1. Identificar la causa exacta del error\n\
            2. Proponer una corrección específica\n\
            3. Ejecutar el comando corregido (no repitas el mismo)\n\
            NUNCA repitas el comando fallido sin modificación.",
            tool_name, command, stderr.trim()
        ),

        ErrorType::Blocked => format!(
            "[SELF-REPAIR] Error bloqueante en '{}'. El agente no puede resolverlo solo.\n\
            Comando: '{}'\n\
            Error: {}\n\
            ACCIÓN OBLIGATORIA: Usa TOOL_ASK_USER para explicar al usuario:\n\
            - Qué intenta hacer el agente\n\
            - Qué está bloqueando el progreso\n\
            - Qué necesita el usuario proporcionar o instalar\n\
            NO reintentes este comando. Espera instrucción del usuario.",
            tool_name, command, stderr.trim()
        ),
    }
}

/// Track consecutive failures per tool to decide when to escalate.
#[derive(Debug, Default)]
pub struct RetryTracker {
    pub transient_retries: u32,
    pub logic_retries: u32,
    pub last_tool: String,
}

impl RetryTracker {
    pub fn new() -> Self { Self::default() }

    /// Record a failure and return whether the agent should escalate.
    pub fn record_failure(&mut self, tool: &str, error_type: &ErrorType) -> bool {
        if self.last_tool != tool {
            // New tool — reset counters
            self.transient_retries = 0;
            self.logic_retries = 0;
            self.last_tool = tool.to_string();
        }
        match error_type {
            ErrorType::Transient => {
                self.transient_retries += 1;
                self.transient_retries >= 3 // escalate after 3 transient retries
            }
            ErrorType::Logic => {
                self.logic_retries += 1;
                self.logic_retries >= 2 // escalate after 2 logic retries (1 think + 1 retry)
            }
            ErrorType::Blocked => true, // always escalate immediately
        }
    }

    pub fn reset(&mut self) {
        self.transient_retries = 0;
        self.logic_retries = 0;
    }
}
