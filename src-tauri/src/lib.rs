mod diagnostics;
pub mod agent;
pub mod ask_user;
pub mod ide;
pub mod user_profile;
pub mod workspace;

#[tauri::command]
fn get_user_profile(
    app: tauri::AppHandle,
    workspace_path: Option<String>,
) -> Result<user_profile::ProfileSnapshot, String> {
    user_profile::load(&app, workspace_path.as_deref())
}

#[tauri::command]
fn save_user_profile(app: tauri::AppHandle, profile: user_profile::UserProfile) -> Result<user_profile::UserProfile, String> {
    user_profile::save(&app, profile)
}

#[tauri::command]
fn clear_user_profile(app: tauri::AppHandle) -> Result<(), String> {
    user_profile::clear(&app)
}

#[tauri::command]
fn save_project_country(app: tauri::AppHandle, workspace_path: String, country: Option<String>) -> Result<(), String> {
    user_profile::save_project_country(&app, &workspace_path, country)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Aislamiento de hardware: sin GPU en WebView2, la VRAM queda para el modelo. Se añade a lo que ya
    // hubiera (p. ej. --remote-debugging-port=9333 para depurar la interfaz) en vez de pisarlo.
    const NO_GPU: &str = "--disable-gpu --disable-gpu-compositing";
    let extra = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").unwrap_or_default();
    std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", format!("{extra} {NO_GPU}").trim());

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle().clone();
            if let Some(job_path) = std::env::args().skip_while(|arg| arg != "--diagnose-mission").nth(1) {
                diagnostics::start(&handle, &job_path).map_err(std::io::Error::other)?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            workspace::get_workspace_tree,
            workspace::get_current_directory,
            workspace::load_chat_history,
            workspace::save_chat_message,
            workspace::clear_chat_history,
            workspace::read_file_content,
            workspace::save_file_content,
            agent::process_user_prompt,
            agent::cancel_agent_mission,
            agent::get_models,
            agent::get_agent_settings,
            agent::save_agent_settings,
            agent::get_pending_mission,
            agent::dismiss_pending_mission,
            agent::agent_order,
            agent::get_agent_rules,
            agent::install_engine,
            ask_user::submit_user_answer,
            ide::get_system_stats,
            ide::get_git_info,
            ide::list_checkpoints,
            ide::read_image_data_url,
            ide::open_external,
            ide::search_workspace,
            ide::run_command,
            ide::get_mission_baseline,
            get_user_profile,
            save_user_profile,
            clear_user_profile,
            save_project_country,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
