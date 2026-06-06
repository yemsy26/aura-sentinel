use std::path::Path;
use tokio::process::Command;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;
use std::collections::HashMap;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};

/// Represents an asynchronous background task managed by the system.
pub struct BackgroundTask {
    pub child: tokio::process::Child,
    pub logs: Arc<Mutex<Vec<String>>>,
}

/// Global registry for active background tasks.
static BACKGROUND_TASKS: OnceLock<Arc<Mutex<HashMap<String, BackgroundTask>>>> = OnceLock::new();

/// Retrieves the active background task registry.
fn get_bg_tasks() -> Arc<Mutex<HashMap<String, BackgroundTask>>> {
    BACKGROUND_TASKS.get_or_init(|| Arc::new(Mutex::new(HashMap::new()))).clone()
}

/// Retrieves the appropriate shell command for the current OS.
#[cfg(target_os = "windows")]
fn get_shell() -> &'static str {
    "cmd"
}

#[cfg(not(target_os = "windows"))]
fn get_shell() -> &'static str {
    "sh"
}

/// Retrieves the appropriate shell argument flag for the current OS.
#[cfg(target_os = "windows")]
fn get_shell_args() -> &'static str {
    "/C"
}

#[cfg(not(target_os = "windows"))]
fn get_shell_args() -> &'static str {
    "-c"
}

/// Spawns an asynchronous command in the background, continuously buffering its logs.
pub async fn start_background_task(workspace_path: &str, task_id: &str, command: &str) -> Result<String, String> {
    let mut child = Command::new(get_shell())
        .args([get_shell_args(), command])
        .current_dir(workspace_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Error starting task {}: {}", task_id, e))?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let logs = Arc::new(Mutex::new(Vec::new()));
    let logs_clone1 = logs.clone();
    let logs_clone2 = logs.clone();

    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            logs_clone1.lock().await.push(format!("[STDOUT] {}", line));
        }
    });

    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            logs_clone2.lock().await.push(format!("[STDERR] {}", line));
        }
    });

    let task = BackgroundTask {
        child,
        logs,
    };

    let tasks = get_bg_tasks();
    tasks.lock().await.insert(task_id.to_string(), task);

    Ok(format!("Asynchronous task '{}' (Command: '{}') started successfully.", task_id, command))
}

/// Fetches the recent logs (max 50 lines) of a running background task.
pub async fn read_task_logs(task_id: &str) -> Result<String, String> {
    let tasks = get_bg_tasks();
    let mut tasks_guard = tasks.lock().await;
    if let Some(task) = tasks_guard.get_mut(task_id) {
        let logs_guard = task.logs.lock().await;
        let mut recent_logs = logs_guard.iter().rev().take(50).rev().cloned().collect::<Vec<_>>().join("\n");
        if recent_logs.is_empty() {
            recent_logs = "[No new logs]".to_string();
        }
        Ok(format!("Logs for task '{}':\n{}", task_id, recent_logs))
    } else {
        Err(format!("Task '{}' not found or already terminated.", task_id))
    }
}

/// Terminates an active background task.
pub async fn kill_task(task_id: &str) -> Result<String, String> {
    let tasks = get_bg_tasks();
    let mut tasks_guard = tasks.lock().await;
    if let Some(mut task) = tasks_guard.remove(task_id) {
        let _ = task.child.kill().await;
        Ok(format!("Task '{}' successfully destroyed.", task_id))
    } else {
        Err(format!("Task '{}' not found for termination.", task_id))
    }
}

/// Validates the physical workspace compilation and environment integrity.
pub async fn validate_workspace(workspace_path: &str) -> Result<(), String> {
    let path = Path::new(workspace_path);

    // 1. Rust (Cargo)
    if path.join("Cargo.toml").exists() {
        let output = Command::new("cargo")
            .arg("check")
            .current_dir(workspace_path)
        .stdin(Stdio::null())
            .output()
            .await
            .map_err(|e| format!("Error executing cargo check: {}", e))?;

        if output.status.success() {
            return Ok(());
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(stderr);
        }
    }

    // 2. Python
    let has_python_files = || -> bool {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                if let Some(ext) = entry.path().extension() {
                    if ext == "py" { return true; }
                }
            }
        }
        false
    };

    if path.join("requirements.txt").exists() || path.join("main.py").exists() || has_python_files() {
        let output = Command::new("python")
            .arg("-m")
            .arg("compileall")
            .arg("-q")
            .arg(".")
            .current_dir(workspace_path)
        .stdin(Stdio::null())
            .output()
            .await
            .map_err(|e| format!("Error executing python compileall: {}", e))?;

        if output.status.success() {
            return Ok(());
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            return Err(format!("{} {}", stdout, stderr).trim().to_string());
        }
    }

    // 3. Node.js
    if path.join("package.json").exists() {
        return Ok(());
    }

    // 4. Generic Fallback
    Ok(())
}

/// Creates an emergency Git rollback checkpoint before executing risky code generation.
pub async fn create_git_backup(workspace_path: &str, commit_message: &str) -> Result<(), String> {
    let path = Path::new(workspace_path);
    
    // Init if needed
    if !path.join(".git").exists() {
        let _ = Command::new(get_shell())
            .args([get_shell_args(), "git init"])
            .current_dir(workspace_path)
        .stdin(Stdio::null())
            .output()
            .await;
        hide_file_windows(&path.join(".git")).await;
    }
    
    // Add all
    let _ = Command::new(get_shell())
        .args([get_shell_args(), "git add ."])
        .current_dir(workspace_path)
        .stdin(Stdio::null())
        .output()
        .await;
        
    // Commit
    let _ = Command::new(get_shell())
        .args([get_shell_args(), &format!("git commit -m \"{}\"", commit_message)])
        .current_dir(workspace_path)
        .stdin(Stdio::null())
        .output()
        .await;
        
    Ok(())
}

/// Hides a file specifically on Windows systems.
pub async fn hide_file_windows(path: &Path) {
    if cfg!(target_os = "windows") {
        let _ = Command::new("attrib")
            .args(["+h", path.to_str().unwrap_or("")])
            .output()
            .await;
    }
}

/// Executes a synchronous terminal command inside the active workspace.
pub async fn execute_terminal_command(workspace_path: &str, command: &str) -> Result<String, String> {
    let output = Command::new(get_shell())
        .args([get_shell_args(), command])
        .current_dir(workspace_path)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("Process Error: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        Ok(stdout)
    } else {
        Err(format!("{} {}", stdout, stderr))
    }
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let mut dot_product = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;
    for (i, j) in a.iter().zip(b.iter()) {
        dot_product += i * j;
        norm_a += i * i;
        norm_b += j * j;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot_product / (norm_a.sqrt() * norm_b.sqrt())
    }
}
