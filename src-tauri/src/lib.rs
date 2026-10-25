pub mod agent;
pub mod ask_user;
mod diagnostics;
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
fn save_user_profile(
    app: tauri::AppHandle,
    profile: user_profile::UserProfile,
) -> Result<user_profile::UserProfile, String> {
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

/// Añade al PATH heredado las carpetas del PATH del sistema y del usuario guardadas en Windows que no traiga.
/// Abierta desde el escritorio, la app recibe el entorno del Explorador, que puede no llevar el PATH del usuario
/// (pasa si es muy largo): sin él, el motor, su instalador y los comandos del usuario no encuentran uv, Python, cargo...
#[cfg(windows)]
fn complete_path() {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    #[link(name = "advapi32")]
    extern "system" {
        fn RegGetValueW(
            key: isize,
            subkey: *const u16,
            value: *const u16,
            flags: u32,
            kind: *mut u32,
            data: *mut core::ffi::c_void,
            size: *mut u32,
        ) -> i32;
    }
    const HKEY_CURRENT_USER: isize = 0x8000_0001_u32 as i32 as isize;
    const HKEY_LOCAL_MACHINE: isize = 0x8000_0002_u32 as i32 as isize;
    const RRF_RT_REG_SZ: u32 = 0x0000_0002; // un REG_EXPAND_SZ llega ya expandido
    let wide = |text: &str| std::ffi::OsStr::new(text).encode_wide().chain(Some(0)).collect::<Vec<_>>();
    let saved = |key: isize, subkey: &str| {
        let mut data = vec![0u16; 32_768]; // el máximo de una variable de entorno
        let mut size = (data.len() * 2) as u32;
        let (subkey, value) = (wide(subkey), wide("Path"));
        let status = unsafe {
            let data = data.as_mut_ptr().cast();
            RegGetValueW(key, subkey.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), data, &mut size)
        };
        let length = if status == 0 { data.iter().position(|&c| c == 0).unwrap_or(0) } else { 0 };
        std::ffi::OsString::from_wide(&data[..length]).to_string_lossy().into_owned()
    };
    let saved = [
        saved(HKEY_LOCAL_MACHINE, r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment"),
        saved(HKEY_CURRENT_USER, "Environment"),
    ];
    std::env::set_var("PATH", merged_path(&std::env::var("PATH").unwrap_or_default(), &saved));
}

/// `current` más las carpetas de `saved` (valores de PATH) que no tenga, sin repetir ni reordenar.
#[cfg(windows)]
fn merged_path(current: &str, saved: &[String]) -> String {
    let key = |dir: &str| dir.trim().trim_end_matches(['\\', '/']).to_lowercase();
    let mut path = current.trim_end_matches(';').to_string();
    let mut known: std::collections::HashSet<String> = path.split(';').map(key).collect();
    known.insert(String::new()); // las vacías no se añaden
    for dir in saved.iter().flat_map(|value| value.split(';')) {
        if known.insert(key(dir)) {
            if !path.is_empty() {
                path.push(';');
            }
            path.push_str(dir);
        }
    }
    path
}

#[cfg(all(test, windows))]
mod tests {
    use super::merged_path;

    #[test]
    fn adds_the_missing_user_path() {
        // Abierta desde el escritorio, la app instalada recibía solo el PATH del sistema: sin uv, Python ni cargo.
        let saved = [
            r"C:\Windows;C:\Windows\System32\;;".to_string(),
            r"C:\u\.local\bin;C:\U\.cargo\bin;C:\u\.local\bin".to_string(),
        ];
        assert_eq!(
            merged_path(r"C:\Windows;C:\Windows\system32;", &saved),
            r"C:\Windows;C:\Windows\system32;C:\u\.local\bin;C:\U\.cargo\bin"
        );
        assert_eq!(merged_path("", &[r"C:\Windows".to_string()]), r"C:\Windows");
        assert_eq!(merged_path(r"C:\a", &["; ".to_string()]), r"C:\a");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(windows)]
    complete_path();

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
            agent::probe_model,
            agent::get_agent_settings,
            agent::save_agent_settings,
            agent::get_pending_mission,
            agent::dismiss_pending_mission,
            agent::agent_order,
            agent::get_agent_rules,
            agent::install_engine,
            agent::get_addons,
            agent::activate_addon,
            agent::deactivate_addon,
            agent::open_addon_repo,
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
