use crate::core::session_journal::{load_journal, save_journal};
use tauri::AppHandle;
use tauri::Emitter;

/// Saves a checkpoint of the FSM mid-loop state so that it survives restarts.
/// Called from agent.rs every N steps.
pub fn save_checkpoint(
    workspace_path: &str,
    context: &str,
    role: &str,
    step: u32,
) {
    let mut journal = load_journal(workspace_path);
    journal.fsm_context = Some(context.chars().take(8000).collect()); // cap at 8KB
    journal.fsm_role = Some(role.to_string());
    journal.fsm_step = step;
    journal.interrupted = true; // mark as interrupted until TOOL_FINISH clears it
    save_journal(workspace_path, &journal);
}

/// Clears the interrupted flag when a mission completes normally.
pub fn clear_interrupt(workspace_path: &str) {
    let mut journal = load_journal(workspace_path);
    journal.interrupted = false;
    journal.fsm_context = None;
    journal.fsm_role = None;
    save_journal(workspace_path, &journal);
}

/// Represents a resumable mission state loaded from disk.
#[derive(Debug, Clone)]
pub struct ResumeState {
    pub objective: String,
    pub context: String,
    pub role: String,
    pub step: u32,
    pub workspace_path: String,
}

/// Checks all known workspace journals for an interrupted mission.
/// Returns the most recent one ready to resume, if any.
pub fn find_pending_mission() -> Option<ResumeState> {
    // Check the user's home dir for a global index of workspaces used recently
    let home = dirs_or_fallback();
    let index_path = std::path::Path::new(&home).join(".aura_workspaces.json");

    let workspaces: Vec<String> = if let Ok(content) = std::fs::read_to_string(&index_path) {
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        Vec::new()
    };

    for ws in workspaces {
        let journal = load_journal(&ws);
        if journal.interrupted && journal.status == "EN_PROGRESO" {
            if let (Some(ctx), Some(role)) = (journal.fsm_context.clone(), journal.fsm_role.clone()) {
                return Some(ResumeState {
                    objective: journal.objetivo.clone(),
                    context: ctx,
                    role,
                    step: journal.fsm_step,
                    workspace_path: ws,
                });
            }
        }
    }
    None
}

/// Registers a workspace path in the global index so it can be scanned on boot.
pub fn register_workspace(workspace_path: &str) {
    let home = dirs_or_fallback();
    let index_path = std::path::Path::new(&home).join(".aura_workspaces.json");
    let mut workspaces: Vec<String> = if let Ok(content) = std::fs::read_to_string(&index_path) {
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        Vec::new()
    };
    if !workspaces.contains(&workspace_path.to_string()) {
        workspaces.push(workspace_path.to_string());
        if let Ok(json) = serde_json::to_string_pretty(&workspaces) {
            let _ = std::fs::write(&index_path, json);
        }
    }
}

/// Emits a "mission-resumed" event to the frontend if there is an interrupted mission.
/// Called once during Tauri setup.
pub fn auto_resume_if_needed(app_handle: &AppHandle) {
    if let Some(resume) = find_pending_mission() {
        let payload = serde_json::json!({
            "objective": resume.objective,
            "workspace": resume.workspace_path,
            "step": resume.step,
            "role": resume.role,
        });
        let _ = app_handle.emit("mission-resumed", payload);
    }
}

fn dirs_or_fallback() -> String {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string())
}
