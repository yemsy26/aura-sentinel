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

fn main() {
    // Aislamiento de hardware: Desactivar GPU en WebView2 (Windows)
    std::env::set_var(
        "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
        "--disable-gpu --disable-gpu-compositing",
    );

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|_app| {
            // Inicialización de módulos asíncronos o configuración adicional
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
            get_background_tasks,
            ui_kill_task
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}