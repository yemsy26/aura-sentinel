use tauri::{Listener, Manager};
use std::io::Write;

fn diagnostic_exit_code(result: &Result<String, String>) -> i32 {
    let finished = result
        .as_ref()
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .and_then(|value| value.get("status").and_then(|status| status.as_str()).map(str::to_string))
        .is_some_and(|status| status == "FINISH");
    if finished { 0 } else { 1 }
}

pub fn start(app: &tauri::AppHandle, job_path: &str) -> Result<(), String> {
    let job: serde_json::Value = serde_json::from_slice(&std::fs::read(job_path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let workspace = job["workspace"].as_str().ok_or("workspace required")?.to_string();
    let prompt = job["prompt"].as_str().ok_or("prompt required")?.to_string();
    let model = job["model"].as_str().unwrap_or("qwen2.5-coder:7b").to_string();
    let root = std::path::Path::new(&workspace).canonicalize().map_err(|e| e.to_string())?;
    let log_path = root.join(".aura/diagnostic-events.jsonl");
    std::fs::create_dir_all(log_path.parent().unwrap()).map_err(|e| e.to_string())?;
    let file = std::sync::Arc::new(std::sync::Mutex::new(std::fs::File::create(&log_path).map_err(|e| e.to_string())?));
    let started = std::time::Instant::now();
    for event_name in ["agent-step", "agent-ask-user", "file-updated"] {
        let file = file.clone();
        app.listen(event_name, move |event| {
            let record = serde_json::json!({"elapsed_ms": started.elapsed().as_millis(), "event": event_name,
                "payload": serde_json::from_str::<serde_json::Value>(event.payload()).unwrap_or_default()});
            if let Ok(mut writer) = file.lock() { let _ = writeln!(writer, "{}", record); let _ = writer.flush(); }
        });
    }
    if let Some(window) = app.get_webview_window("main") { let _ = window.hide(); }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = crate::llm::process_user_prompt(prompt, workspace, model.clone(), model, handle.clone()).await;
        let exit_code = diagnostic_exit_code(&result);
        let report = serde_json::json!({"elapsed_ms":started.elapsed().as_millis(), "result": result});
        let _ = std::fs::write(root.join(".aura/diagnostic-result.json"), report.to_string());
        // Tauri's graceful exit may normalize the process code on some Windows
        // runtimes. This diagnostic binary is non-interactive, so terminate with
        // the exact mission status after the report has reached disk.
        std::process::exit(exit_code);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::diagnostic_exit_code;

    #[test]
    fn only_finish_maps_to_zero() {
        assert_eq!(diagnostic_exit_code(&Ok(r#"{"status":"FINISH"}"#.into())), 0);
        assert_eq!(diagnostic_exit_code(&Ok(r#"{"status":"ERROR"}"#.into())), 1);
        assert_eq!(diagnostic_exit_code(&Err("fallo".into())), 1);
    }
}
