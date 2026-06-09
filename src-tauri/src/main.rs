// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod core;
mod memory;
mod llm;
mod net;

#[tauri::command]
fn get_system_stats() -> String {
    use sysinfo::System;
    let mut sys = System::new_all();
    sys.refresh_all();
    if let Ok(pid) = sysinfo::get_current_pid() {
        if let Some(process) = sys.process(pid) {
            let used_memory = process.memory() as f64 / 1048576.0; // MB
            return format!("{:.1} MB", used_memory);
        }
    }
    "0.0 MB".to_string()
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
            llm::process_user_prompt,
            get_system_stats
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}