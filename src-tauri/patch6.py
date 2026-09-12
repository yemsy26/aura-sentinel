import sys

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\core\session_journal.rs', 'r', encoding='utf-8') as f:
    text = f.read()

replacement = '''
pub fn start_new_mission(workspace_path: &str, objective: &str) -> SessionJournal {
    let mut journal = SessionJournal::default();
    journal.session_id = new_session_id();
    journal.objetivo = objective.to_string();
    journal.workspace_path = workspace_path.to_string();
    journal.status = "EN_PROGRESO".to_string();
    journal.ultima_actualizacion = current_timestamp();
    // Start totally clean
    journal.fsm_context = None;
    journal.fsm_role = None;
    journal.fsm_step = 0;
    journal.interrupted = false;
    journal.fases.clear();
    journal.micro_metas.clear();
    
    // Attempt save, ignore error on brand new init
    let _ = save_journal(workspace_path, &journal);
    journal
}

pub fn resume_existing_mission(workspace_path: &str) -> SessionJournal {
    let mut journal = load_journal(workspace_path);
    if !journal.interrupted {
        // If not explicitly interrupted, we are forcing a resume of an old state,
        // so we start a new session ID but link the parent
        let old_id = journal.session_id.clone();
        journal.parent_session = Some(old_id);
        journal.session_id = new_session_id();
    }
    journal.status = "EN_PROGRESO".to_string();
    journal.interrupted = false; // We are actively resuming now
    journal.ultima_actualizacion = current_timestamp();
    let _ = save_journal(workspace_path, &journal);
    journal
}

'''

text = text + replacement

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\core\session_journal.rs', 'w', encoding='utf-8') as f:
    f.write(text)

print("SUCCESS")
