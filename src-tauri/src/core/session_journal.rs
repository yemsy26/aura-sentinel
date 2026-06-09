use serde::{Deserialize, Serialize};
use std::path::Path;

/// Persistent session state saved to `{workspace}/.aura_session.json` after each agent step.
/// Survives laptop sleep/hibernate/restart cycles.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SessionJournal {
    /// The original user objective for this mission
    pub objetivo: String,
    /// Last step number completed
    pub ultimo_paso: u32,
    /// Short description of what was done last
    pub ultimo_estado: String,
    /// Status tag: "EN_PROGRESO", "COMPLETADO", "FALLIDO", "ESPERANDO"
    pub status: String,
    /// Tools used so far (for quick summary)
    pub herramientas_usadas: Vec<String>,
    /// Files touched/created this session
    pub archivos_tocados: Vec<String>,
    /// ISO 8601 timestamp of last update
    pub ultima_actualizacion: String,
    /// Workspace path (so we can reconstruct on boot)
    pub workspace_path: String,
    /// Short-term conversational memory to maintain context
    #[serde(default)]
    pub chat_history: Vec<String>,
}

/// File name for the session journal (hidden by convention via leading dot)
const JOURNAL_FILE: &str = ".aura_session.json";

/// Loads the session journal from disk, or returns a default empty one.
pub fn load_journal(workspace_path: &str) -> SessionJournal {
    let journal_path = Path::new(workspace_path).join(JOURNAL_FILE);
    match std::fs::read_to_string(&journal_path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => SessionJournal::default(),
    }
}

/// Saves the session journal to disk (best-effort — failures are non-fatal).
pub fn save_journal(workspace_path: &str, journal: &SessionJournal) {
    let journal_path = Path::new(workspace_path).join(JOURNAL_FILE);
    if let Ok(json) = serde_json::to_string_pretty(journal) {
        let _ = std::fs::write(&journal_path, json);
    }
}

/// Builds a human-readable status report from the journal.
/// Called by the meta-command handler — no LLM needed.
pub fn build_status_report(journal: &SessionJournal) -> String {
    if journal.objetivo.is_empty() {
        return "📋 No hay ninguna tarea activa registrada en este workspace.\n\
                El sistema no tiene memoria de misiones anteriores aquí.\n\
                Puedes darme una nueva instrucción cuando quieras."
            .to_string();
    }

    let status_icon = match journal.status.as_str() {
        "COMPLETADO" => "✅",
        "FALLIDO"    => "❌",
        "ESPERANDO"  => "⏸️",
        _            => "🔄", // EN_PROGRESO
    };

    let herramientas = if journal.herramientas_usadas.is_empty() {
        "ninguna aún".to_string()
    } else {
        // De-duplicate and join
        let mut seen = std::collections::HashSet::new();
        journal
            .herramientas_usadas
            .iter()
            .filter(|t| seen.insert(t.to_lowercase()))
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    };

    let archivos = if journal.archivos_tocados.is_empty() {
        "ninguno".to_string()
    } else {
        journal
            .archivos_tocados
            .iter()
            .map(|p| {
                // Show just the relative filename for brevity
                Path::new(p)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.clone())
            })
            .collect::<Vec<_>>()
            .join(", ")
    };

    format!(
        "{status_icon} **Estado de la Misión**\n\
         ─────────────────────────────────\n\
         🎯 **Objetivo:** {objetivo}\n\
         📊 **Estado:** {status}\n\
         🔢 **Último paso completado:** {paso}\n\
         🕐 **Última actualización:** {ts}\n\
         🛠️  **Herramientas usadas:** {herramientas}\n\
         📁 **Archivos tocados:** {archivos}\n\
         ─────────────────────────────────\n\
         💡 *Para continuar desde donde lo dejé, dime: \"continúa\" o \"retoma la tarea\".*",
        status_icon = status_icon,
        objetivo = journal.objetivo,
        status = journal.status,
        paso = journal.ultimo_paso,
        ts = journal.ultima_actualizacion,
        herramientas = herramientas,
        archivos = archivos,
    )
}

/// Updates an existing journal with new step data. Call from `run_agent_loop` each step.
pub fn update_journal(
    journal: &mut SessionJournal,
    paso: u32,
    estado: &str,
    herramienta: &str,
    archivos: &[String],
    workspace_path: &str,
) {
    journal.ultimo_paso = paso;
    journal.ultimo_estado = estado.to_string();
    journal.status = "EN_PROGRESO".to_string();
    journal.workspace_path = workspace_path.to_string();
    journal.ultima_actualizacion = current_timestamp();

    if !herramienta.is_empty() && !journal.herramientas_usadas.contains(&herramienta.to_string()) {
        journal.herramientas_usadas.push(herramienta.to_string());
    }
    for archivo in archivos {
        if !journal.archivos_tocados.contains(archivo) {
            journal.archivos_tocados.push(archivo.clone());
        }
    }
}

/// Marks the mission as completed or failed.
pub fn close_journal(journal: &mut SessionJournal, status: &str, workspace_path: &str) {
    journal.status = status.to_string();
    journal.ultima_actualizacion = current_timestamp();
    save_journal(workspace_path, journal);
}

fn current_timestamp() -> String {
    // No chrono dependency — use std::time for a simple Unix timestamp string
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Format as simple readable: "YYYY-MM-DD HH:MM:SS UTC" would need chrono.
    // Without chrono we store unix seconds and a human note.
    format!("unix:{} (usa 'date -d @{}' para convertir)", secs, secs)
}
