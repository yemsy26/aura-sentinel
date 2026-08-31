// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod core;
mod memory;
mod llm;
mod net;


#[tauri::command]
async fn get_background_tasks() -> Result<Vec<serde_json::Value>, String> {
    Ok(core::get_active_tasks_snapshot().await)
}

#[tauri::command]
async fn ui_kill_task(task_id: String) -> Result<String, String> {
    core::kill_task(&task_id).await
}

#[tauri::command]
fn get_system_stats() -> String {
    use sysinfo::System;
    let mut sys = System::new_all();
    sys.refresh_all();
    
    let total_mem = sys.total_memory() as f64 / 1073741824.0; // GB
    let used_mem = sys.used_memory() as f64 / 1073741824.0; // GB
    let mem_percent = (used_mem / total_mem) * 100.0;
    
    let cpus = sys.cpus();
    let cpu_usage = if !cpus.is_empty() {
        cpus.iter().map(|c| c.cpu_usage()).sum::<f32>() / cpus.len() as f32
    } else {
        0.0
    };
    
    let status = if mem_percent > 85.0 {
        " [OOM SAFE MODE]"
    } else {
        ""
    };
    
    let app_mem = if let Ok(pid) = sysinfo::get_current_pid() {
        if let Some(process) = sys.process(pid) {
            process.memory() as f64 / 1048576.0
        } else {
            0.0
        }
    } else {
        0.0
    };
    
    format!("CPU: {:.1}% | RAM Sys: {:.1}/{:.1} GB | App: {:.0} MB{}", cpu_usage, used_mem, total_mem, app_mem, status)
}


// ── Fase 4: Scheduler Tauri commands ─────────────────────────────────────────

#[tauri::command]
fn schedule_task(objective: String, workspace: String, cron_expr: String, description: String) -> String {
    core::scheduler::register_task(&objective, &workspace, &cron_expr, &description)
}

#[tauri::command]
fn list_scheduled_tasks() -> String {
    core::scheduler::list_tasks_json()
}

#[tauri::command]
fn remove_scheduled_task(id: String) -> bool {
    core::scheduler::remove_task(&id)
}

// ── Fase 3: Episodic Memory Tauri commands ────────────────────────────────────

#[tauri::command]
fn search_episodes_cmd(query: String) -> String {
    let results = core::episodic_memory::search_episodes(&query);
    serde_json::to_string(&results).unwrap_or_else(|_| "[]".to_string())
}

#[tauri::command]
fn get_recent_episodes_cmd(n: u32) -> String {
    let results = core::episodic_memory::load_recent_episodes(n as usize);
    serde_json::to_string(&results).unwrap_or_else(|_| "[]".to_string())
}

#[tauri::command]
async fn get_ollama_models() -> Result<Vec<String>, String> {
    let client = reqwest::Client::new();
    match client.get("http://localhost:11434/api/tags").send().await {
        Ok(res) => {
            if let Ok(json) = res.json::<serde_json::Value>().await {
                if let Some(models) = json.get("models").and_then(|m| m.as_array()) {
                    let mut model_names = Vec::new();
                    for m in models {
                        if let Some(name) = m.get("name").and_then(|n| n.as_str()) {
                            model_names.push(name.to_string());
                        }
                    }
                    return Ok(model_names);
                }
            }
            Err("No models found".to_string())
        },
        Err(e) => Err(e.to_string()),
    }
}

fn main() {

    // Aislamiento de hardware: Desactivar GPU en WebView2 (Windows)
    std::env::set_var(
        "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
        "--disable-gpu --disable-gpu-compositing",
    );

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle().clone();

            // ── Fase 4: Iniciar Scheduler autónomo (tick cada 60s) ──
            core::scheduler::start_scheduler(handle.clone());

            // ── Fase 1: Auto-resume de misión interrumpida ──
            core::mission_persist::auto_resume_if_needed(&handle);

            Ok(())
        })
.invoke_handler(tauri::generate_handler![
            memory::get_workspace_tree,
            memory::get_current_directory,
            memory::init_memory_log,
            memory::add_memory_entry,
            memory::read_memory_logs,
            memory::load_chat_history,
            memory::save_chat_message,
            memory::clear_chat_history,
            memory::read_file_content,
            memory::save_file_content,
            llm::process_user_prompt,
            get_system_stats,
            get_ollama_models,
            get_background_tasks,
            ui_kill_task,
            core::ask_user::submit_user_answer,
            core::auto_validator::run_auto_validation,
            // ── Fase 4: Scheduler commands ──
            schedule_task,
            list_scheduled_tasks,
            remove_scheduled_task,
            // ── Fase 3: Episode memory commands ──
            search_episodes_cmd,
            get_recent_episodes_cmd,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}