/// Error classification engine for Aura-Sentinel's self-repair loop.
///
/// Professional agents (SWE-agent, AutoGen, Claude) classify errors into 3 types
/// and choose different recovery strategies for each:
///
///  - TRANSIENT → retry automatically (network blip, lock, timeout)
///  - LOGIC     → change approach (syntax error, missing file, wrong command)
///  - BLOCKED   → escalate to user (missing credential, not installed, permission)
///

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

    // ── CODE & TEST ASSERTION signals (Always Logic errors, never Blocked) ─
    let code_or_test_signals = [
        "[fail]", "fail:", "failed", "assertionerror", "syntaxerror",
        "typeerror", "referenceerror", "nameerror", "indexerror",
        "valueerror", "missing html", "missing js", "missing css",
        "traceback", "test failed", "verification failed", "assert "
    ];
    for sig in &code_or_test_signals {
        if combined.contains(sig) {
            return ErrorType::Logic;
        }
    }

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

        ErrorType::Logic => {
            let cmd_lower = command.to_lowercase();
            let err_lower = stderr.to_lowercase();
            let is_test = cmd_lower.contains("verify") || cmd_lower.contains("test")
                || err_lower.contains("[fail]") || err_lower.contains("assert")
                || err_lower.contains("missing html") || err_lower.contains("missing js");

            if is_test {
                format!(
                    "[AUTO-DIAGNOSIS DE TEST FALLIDO] en '{}' (intento {}/6).\n\
                    Comando ejecutado: '{}'\n\
                    Salida del error: {}\n\
                    GUÍA DE AUTO-REPARACIÓN AUTÓNOMA:\n\
                    1. Revisa el archivo objetivo: ¿falta realmente el elemento/función o existe con atributos en orden distinto (ej. <div class=\"...\" id=\"...\">)?\n\
                    2. Revisa el SCRIPT DE PRUEBA: ¿tiene asunciones rígidas (búsqueda de cadenas fijas en vez de regex semántico) o un bug lógico en el reporte (ej. array sin 'PASS' o exit(1) incorrecto)?\n\
                    3. Usa TOOL_PROGRAMMER para corregir el archivo objetivo o flexibilizar el script de prueba.\n\
                    4. Vuelve a ejecutar el test. PROHIBIDO usar TOOL_ASK_USER para pedir al usuario que arregle código o tests.",
                    tool_name, attempt, command, stderr.trim()
                )
            } else {
                format!(
                    "[SELF-REPAIR] Error lógico detectado en '{}' (intento {}/6).\n\
                    Comando que falló: '{}'\n\
                    Error recibido: {}\n\
                    DIAGNÓSTICO REQUERIDO: Antes de reintentar, debes usar TOOL_THINK para:\n\
                    1. Identificar la causa exacta del error en el código o comando.\n\
                    2. Proponer una corrección específica y aplicarla con TOOL_PROGRAMMER.\n\
                    3. Ejecutar el comando corregido (no repitas el mismo sin cambios).\n\
                    PROHIBIDO usar TOOL_ASK_USER para pedir permiso de corregir sintaxis o código.",
                    tool_name, attempt, command, stderr.trim()
                )
            }
        },

        ErrorType::Blocked => format!(
            "[SELF-REPAIR] Error bloqueante del sistema en '{}'. El agente no puede resolverlo solo.\n\
            Comando: '{}'\n\
            Error: {}\n\
            ACCIÓN OBLIGATORIA: Usa TOOL_ASK_USER para explicar al usuario:\n\
            - Qué intenta hacer el agente\n\
            - Qué dependencia o permiso del sistema está bloqueando el progreso\n\
            - Qué necesita el usuario proporcionar o instalar fuera del workspace\n\
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
                self.logic_retries >= 6 // escalate only after 6 logic attempts (ample self-repair room)
            }
            ErrorType::Blocked => true, // always escalate immediately
        }
    }

    pub fn _reset(&mut self) {
        self.transient_retries = 0;
        self.logic_retries = 0;
    }
}
