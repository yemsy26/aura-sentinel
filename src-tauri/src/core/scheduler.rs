use serde::{Deserialize, Serialize};
use std::path::Path;
use chrono::{Utc, DateTime};
use tauri::{AppHandle, Emitter};
use tokio::time::{interval, Duration};

/// A single scheduled recurring task
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ScheduledTask {
    pub id: String,
    pub description: String,
    pub objective: String,    // Instruction passed to the agent loop
    pub workspace: String,
    pub cron_expr: String,    // e.g. "0 9 * * 1" = Monday 9am
    pub enabled: bool,
    pub last_run: Option<String>,
    pub created_at: String,
}

const SCHEDULER_FILE: &str = ".aura_scheduler.json";

fn scheduler_path() -> std::path::PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(SCHEDULER_FILE)
}

fn load_tasks() -> Vec<ScheduledTask> {
    let path = scheduler_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_tasks(tasks: &[ScheduledTask]) {
    let path = scheduler_path();
    if let Ok(json) = serde_json::to_string_pretty(tasks) {
        let _ = std::fs::write(path, json);
    }
}

/// Register a new scheduled task. Returns the task ID.
pub fn register_task(objective: &str, workspace: &str, cron_expr: &str, description: &str) -> String {
    let mut tasks = load_tasks();
    let id = format!("sched_{:x}", uuid_lite());
    tasks.push(ScheduledTask {
        id: id.clone(),
        description: description.to_string(),
        objective: objective.to_string(),
        workspace: workspace.to_string(),
        cron_expr: cron_expr.to_string(),
        enabled: true,
        last_run: None,
        created_at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    });
    save_tasks(&tasks);
    id
}

/// Remove a scheduled task by ID
pub fn remove_task(id: &str) -> bool {
    let mut tasks = load_tasks();
    let before = tasks.len();
    tasks.retain(|t| t.id != id);
    save_tasks(&tasks);
    tasks.len() < before
}

/// List all scheduled tasks as JSON string
pub fn list_tasks_json() -> String {
    serde_json::to_string_pretty(&load_tasks()).unwrap_or_else(|_| "[]".to_string())
}

/// Start the background scheduler loop (tick every 60 seconds)
/// Emits "scheduled-task-fire" to the frontend when a task is due
pub fn start_scheduler(app_handle: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = interval(Duration::from_secs(60));
        loop {
            ticker.tick().await;
            check_and_fire(&app_handle);
        }
    });
}

fn check_and_fire(app: &AppHandle) {
    let now = Utc::now();
    let mut tasks = load_tasks();
    let mut changed = false;

    for task in tasks.iter_mut() {
        if !task.enabled { continue; }
        if should_fire(&task.cron_expr, &task.last_run, &now) {
            task.last_run = Some(now.format("%Y-%m-%dT%H:%M:%SZ").to_string());
            changed = true;

            // Emit event to frontend — the JS bridge will trigger process_user_prompt
            let payload = serde_json::json!({
                "task_id": task.id,
                "objective": task.objective,
                "workspace": task.workspace,
                "description": task.description,
            });
            let _ = app.emit("scheduled-task-fire", payload);
        }
    }

    if changed { save_tasks(&tasks); }
}

/// Minimal cron-like evaluation. Supports:
///   - "*/N" for "every N minutes/hours"
///   - exact values
///   - "*" for any
/// Fields: minute hour day-of-month month day-of-week
fn should_fire(cron: &str, last_run: &Option<String>, now: &DateTime<Utc>) -> bool {
    let parts: Vec<&str> = cron.split_whitespace().collect();
    if parts.len() != 5 { return false; }

    let match_part = |part: &str, value: u32, is_dow: bool| -> bool {
        if part == "*" || part == "?" { return true; }

        // Handle comma-separated lists e.g. "1,15,30" or "MON,WED,FRI"
        for sub in part.split(',') {
            let sub = sub.trim();
            if sub.is_empty() { continue; }

            // Handle steps e.g. "*/5" or "10-30/5"
            if sub.contains('/') {
                let step_parts: Vec<&str> = sub.split('/').collect();
                if step_parts.len() == 2 {
                    let step = step_parts[1].parse::<u32>().unwrap_or(0);
                    if step > 0 {
                        if step_parts[0] == "*" {
                            if value % step == 0 { return true; }
                        } else if let Ok(start) = step_parts[0].parse::<u32>() {
                            if value >= start && (value - start) % step == 0 { return true; }
                        }
                    }
                }
                continue;
            }

            // Handle ranges e.g. "1-5"
            if sub.contains('-') {
                let range_parts: Vec<&str> = sub.split('-').collect();
                if range_parts.len() == 2 {
                    if let (Ok(start), Ok(end)) = (range_parts[0].parse::<u32>(), range_parts[1].parse::<u32>()) {
                        if value >= start && value <= end { return true; }
                    }
                }
                continue;
            }

            // Day of week special handling (names and 0/7 Sunday)
            if is_dow {
                let sub_upper = sub.to_uppercase();
                let dow_val = match sub_upper.as_str() {
                    "MON" => 1, "TUE" => 2, "WED" => 3, "THU" => 4, "FRI" => 5, "SAT" => 6, "SUN" => 7,
                    _ => sub.parse::<u32>().map(|v| if v == 0 { 7 } else { v }).unwrap_or(99),
                };
                if dow_val == value { return true; }
                continue;
            }

            // Direct numeric match
            if let Ok(v) = sub.parse::<u32>() {
                if v == value { return true; }
            }
        }
        false
    };

    let minute = now.format("%M").to_string().parse::<u32>().unwrap_or(99);
    let hour   = now.format("%H").to_string().parse::<u32>().unwrap_or(99);
    let dom    = now.format("%d").to_string().parse::<u32>().unwrap_or(99);
    let month  = now.format("%m").to_string().parse::<u32>().unwrap_or(99);
    let dow    = now.format("%u").to_string().parse::<u32>().unwrap_or(99); // 1=Mon, 7=Sun

    if !match_part(parts[0], minute, false) { return false; }
    if !match_part(parts[1], hour, false)   { return false; }
    if !match_part(parts[2], dom, false)    { return false; }
    if !match_part(parts[3], month, false)  { return false; }
    if !match_part(parts[4], dow, true)     { return false; }

    // Don't re-fire if we already ran this minute
    if let Some(last) = last_run {
        if last.len() >= 16 {
            let now_min = now.format("%Y-%m-%dT%H:%M").to_string();
            if last.starts_with(&now_min) { return false; }
        }
    }

    true
}

fn uuid_lite() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}
