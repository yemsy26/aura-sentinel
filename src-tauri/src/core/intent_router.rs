pub enum IntentAction {
    Finish(String),
    Resume { objetivo: String, resume_msg: String },
}

/// Zero-latency intent router that intercepts meta-queries BEFORE the LLM pipeline.
pub fn try_handle_meta_command(
    user_message: &str,
    workspace_path: &str,
) -> Option<IntentAction> {
    let msg = user_message.trim().to_lowercase();

    // ── Status / pending task queries ───────────────────────────────────────
    let is_status_query = contains_any(&msg, &[
        "que tenia pendiente",
        "qué tenia pendiente",
        "que tenía pendiente",
        "qué tenía pendiente",
        "que estaba haciendo",
        "qué estaba haciendo",
        "revisa que tarea",
        "revisa qué tarea",
        "cual era mi tarea",
        "cuál era mi tarea",
        "que tarea tenia",
        "qué tarea tenía",
        "en que estaba",
        "en qué estaba",
        "estado de la mision",
        "estado de la misión",
        "show status",
        "mission status",
        "que me falta",
        "qué me falta",
        "que habia hecho",
        "qué había hecho",
        "resumen de la tarea",
        "check task",
        "pending task",
    ]);

    if is_status_query {
        let journal = crate::core::session_journal::load_journal(workspace_path);
        let report = crate::core::session_journal::build_status_report(&journal);
        return Some(IntentAction::Finish(report));
    }

    // ── Resume / continue task ───────────────────────────────────────────────
    let is_resume = contains_any(&msg, &[
        "continua",
        "continúa",
        "retoma",
        "retoma la tarea",
        "sigue",
        "continue",
        "resume",
        "donde me quede",
        "donde me quedé",
    ]);

    if is_resume {
        let journal = crate::core::session_journal::load_journal(workspace_path);
        if !journal.objetivo.is_empty() && journal.status == "EN_PROGRESO" {
            // Return a message that tells the frontend to re-run with the saved objective
            let resume_msg = format!(
                "🔄 **Retomando misión desde el paso {}**\n\n\
                 🎯 Objetivo: {}\n\n\
                 Reiniciando el agente con el contexto guardado...",
                journal.ultimo_paso + 1,
                journal.objetivo
            );
            return Some(IntentAction::Resume {
                objetivo: journal.objetivo,
                resume_msg,
            });
        } else if journal.objetivo.is_empty() {
            return Some(IntentAction::Finish(
                "📋 No hay ninguna tarea activa que retomar en este workspace. Dame un nuevo objetivo y empezamos.".to_string()
            ));
        }
    }

    // ── Help / capabilities ──────────────────────────────────────────────────
    let is_help = contains_any(&msg, &[
        "que puedes hacer",
        "qué puedes hacer",
        "ayuda",
        "help",
        "comandos disponibles",
        "herramientas disponibles",
    ]);

    if is_help {
        let help_text = r#"🛡️ **Aura-Sentinel — Comandos Disponibles**
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
**Meta-Comandos (instantáneos, sin IA):**
• `"revisa qué tarea tenía pendiente"` — Muestra el estado de la última misión
• `"continúa"` / `"retoma"` — Reanuda la misión donde se quedó
• `"ayuda"` — Este menú

**Herramientas de Desarrollo:**
• TOOL_PROGRAMMER — Escribe/modifica código en disco
• TOOL_TERMINAL — Ejecuta comandos del sistema
• TOOL_TESTER — Corre suites de pruebas automáticamente
• TOOL_ENV_MANAGER — Instala dependencias del sistema (Scoop/winget)
• TOOL_AUDITOR — Audita el código existente
• TOOL_ARCHITECT — Genera mapa de dependencias
• TOOL_BACKGROUND_START/READ/KILL — Servidores en segundo plano
• TOOL_WEB_SCRAPER — Extrae contenido de URLs
• TOOL_LEARN / TOOL_SEARCH — Memoria vectorial persistente
• TOOL_FINISH — Cierra la misión

**Lenguajes Soportados:**
Python, Rust, Go, JavaScript/TypeScript, Solidity (Hardhat/Foundry),
Kotlin/Android, Dart/Flutter, PHP, Swift, C, C++, Java
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"#;
        return Some(IntentAction::Finish(help_text.to_string()));
    }

    // Not a meta-command — let the normal pipeline handle it
    None
}

/// Helper: returns true if `s` contains any of the given substrings.
fn contains_any(s: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|p| s.contains(p))
}
