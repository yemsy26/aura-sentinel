use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::{AppHandle, Emitter};
use tokio::time::{interval, Duration};

/// A single scheduled recurring task
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ScheduledTask {
    pub id: String,
    pub description: String,
    pub objective: String, // Instruction passed to the agent loop
    pub workspace: String,
    pub cron_expr: String, // e.g. "0 9 * * 1" = Monday 9am
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

fn load_tasks() -> Result<Vec<ScheduledTask>, String> {
    let path = scheduler_path();
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents)
            .map_err(|error| format!("No se pudo interpretar {}: {}", path.display(), error)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("No se pudo leer {}: {}", path.display(), error)),
    }
}

fn save_tasks(tasks: &[ScheduledTask]) -> Result<(), String> {
    let path = scheduler_path();
    let json = serde_json::to_string_pretty(tasks)
        .map_err(|error| format!("No se pudo serializar el programador: {}", error))?;
    std::fs::write(&path, json)
        .map_err(|error| format!("No se pudo guardar {}: {}", path.display(), error))
}

/// Register a new scheduled task. Returns the task ID.
pub fn register_task(
    objective: &str,
    workspace: &str,
    cron_expr: &str,
    description: &str,
) -> Result<String, String> {
    if objective.trim().is_empty() || workspace.trim().is_empty() || description.trim().is_empty() {
        return Err("La tarea requiere objetivo, workspace y descripción no vacíos.".into());
    }
    validate_cron_expr(cron_expr)?;
    let mut tasks = load_tasks()?;
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
    save_tasks(&tasks)?;
    Ok(id)
}

/// Remove a scheduled task by ID
pub fn remove_task(id: &str) -> Result<bool, String> {
    let mut tasks = load_tasks()?;
    let before = tasks.len();
    tasks.retain(|t| t.id != id);
    let removed = tasks.len() < before;
    if removed {
        save_tasks(&tasks)?;
    }
    Ok(removed)
}

/// List all scheduled tasks as JSON string
pub fn list_tasks_json() -> Result<String, String> {
    serde_json::to_string_pretty(&load_tasks()?)
        .map_err(|error| format!("No se pudo serializar el listado de tareas: {}", error))
}

fn cron_value(raw: &str, field_index: usize) -> Option<u32> {
    if field_index == 4 {
        let named = match raw.to_ascii_uppercase().as_str() {
            "MON" => Some(1),
            "TUE" => Some(2),
            "WED" => Some(3),
            "THU" => Some(4),
            "FRI" => Some(5),
            "SAT" => Some(6),
            "SUN" => Some(7),
            _ => None,
        };
        return named
            .or_else(|| raw.parse::<u32>().ok())
            .map(|value| if value == 0 { 7 } else { value });
    }
    raw.parse::<u32>().ok()
}

fn cron_bounds(field_index: usize) -> (u32, u32) {
    match field_index {
        0 => (0, 59),
        1 => (0, 23),
        2 => (1, 31),
        3 => (1, 12),
        _ => (1, 7),
    }
}

/// Rejects malformed schedules before they can be saved as enabled but inert tasks.
fn validate_cron_expr(cron: &str) -> Result<(), String> {
    let fields: Vec<&str> = cron.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(
            "La expresión cron debe contener cinco campos: minuto hora día mes día-semana.".into(),
        );
    }
    for (field_index, field) in fields.iter().enumerate() {
        let (minimum, maximum) = cron_bounds(field_index);
        for item in field.split(',') {
            if item.is_empty() {
                return Err(format!(
                    "Campo cron {} contiene una opción vacía.",
                    field_index + 1
                ));
            }
            let mut step_parts = item.split('/');
            let base = step_parts.next().unwrap_or_default();
            let step = match step_parts.next() {
                Some(raw) if step_parts.next().is_none() => {
                    let value = raw
                        .parse::<u32>()
                        .ok()
                        .filter(|value| *value > 0)
                        .ok_or_else(|| format!("Paso cron inválido en '{}'.", item))?;
                    if value > maximum - minimum + 1 {
                        return Err(format!(
                            "Paso cron fuera del rango permitido en '{}'.",
                            item
                        ));
                    }
                    Some(value)
                }
                Some(_) => return Err(format!("Opción cron inválida '{}'.", item)),
                None => None,
            };
            if base == "?" && step.is_some() {
                return Err(format!("El comodín '?' no admite pasos en '{}'.", item));
            }
            if base == "*" || base == "?" {
                continue;
            }
            if let Some((start_raw, end_raw)) = base.split_once('-') {
                let start = cron_value(start_raw, field_index)
                    .ok_or_else(|| format!("Inicio de rango cron inválido en '{}'.", item))?;
                let end = cron_value(end_raw, field_index)
                    .ok_or_else(|| format!("Fin de rango cron inválido en '{}'.", item))?;
                if start < minimum || end > maximum || start > end {
                    return Err(format!("Rango cron fuera de límites en '{}'.", item));
                }
                if step.is_some() && start == end {
                    return Err(format!(
                        "El rango con paso debe abarcar más de un valor: '{}'.",
                        item
                    ));
                }
            } else {
                let value = cron_value(base, field_index)
                    .ok_or_else(|| format!("Valor cron inválido en '{}'.", item))?;
                if value < minimum || value > maximum {
                    return Err(format!("Valor cron fuera de límites en '{}'.", item));
                }
            }
        }
    }
    Ok(())
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
    let mut tasks = match load_tasks() {
        Ok(tasks) => tasks,
        Err(error) => {
            eprintln!(
                "[SCHEDULER][ERROR] No se pudieron cargar las tareas: {}",
                error
            );
            return;
        }
    };
    let mut changed = false;

    for task in tasks.iter_mut() {
        if !task.enabled {
            continue;
        }
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

    if changed {
        if let Err(error) = save_tasks(&tasks) {
            eprintln!(
                "[SCHEDULER][ERROR] No se pudo guardar el estado de ejecución: {}",
                error
            );
        }
    }
}

/// Minimal cron-like evaluation. Supports:
///   - "*/N" for "every N minutes/hours"
///   - exact values
///   - "*" for any
/// Fields: minute hour day-of-month month day-of-week
fn should_fire(cron: &str, last_run: &Option<String>, now: &DateTime<Utc>) -> bool {
    let parts: Vec<&str> = cron.split_whitespace().collect();
    if parts.len() != 5 {
        return false;
    }

    let match_part = |part: &str, value: u32, is_dow: bool| -> bool {
        let parse_value = |raw: &str| {
            if is_dow {
                cron_value(raw, 4)
            } else {
                raw.parse::<u32>().ok()
            }
        };
        if part == "*" || part == "?" {
            return true;
        }

        // Handle comma-separated lists e.g. "1,15,30" or "MON,WED,FRI"
        for sub in part.split(',') {
            let sub = sub.trim();
            if sub.is_empty() {
                continue;
            }
            if sub == "*" || sub == "?" {
                return true;
            }

            // Handle steps e.g. "*/5" or "10-30/5"
            if sub.contains('/') {
                let step_parts: Vec<&str> = sub.split('/').collect();
                if step_parts.len() == 2 {
                    let step = step_parts[1].parse::<u32>().unwrap_or(0);
                    if step > 0 {
                        if step_parts[0] == "*" {
                            if value % step == 0 {
                                return true;
                            }
                        } else if let Some((start, end)) = step_parts[0].split_once('-') {
                            if let (Some(start), Some(end)) = (parse_value(start), parse_value(end))
                            {
                                if value >= start && value <= end && (value - start) % step == 0 {
                                    return true;
                                }
                            }
                        } else if let Some(start) = parse_value(step_parts[0]) {
                            if value >= start && (value - start) % step == 0 {
                                return true;
                            }
                        }
                    }
                }
                continue;
            }

            // Handle ranges e.g. "1-5"
            if sub.contains('-') {
                let range_parts: Vec<&str> = sub.split('-').collect();
                if range_parts.len() == 2 {
                    if let (Some(start), Some(end)) =
                        (parse_value(range_parts[0]), parse_value(range_parts[1]))
                    {
                        if value >= start && value <= end {
                            return true;
                        }
                    }
                }
                continue;
            }

            // Day of week special handling (names and 0/7 Sunday)
            if is_dow {
                let sub_upper = sub.to_uppercase();
                let dow_val = match sub_upper.as_str() {
                    "MON" => 1,
                    "TUE" => 2,
                    "WED" => 3,
                    "THU" => 4,
                    "FRI" => 5,
                    "SAT" => 6,
                    "SUN" => 7,
                    _ => sub
                        .parse::<u32>()
                        .map(|v| if v == 0 { 7 } else { v })
                        .unwrap_or(99),
                };
                if dow_val == value {
                    return true;
                }
                continue;
            }

            // Direct numeric match
            if let Ok(v) = sub.parse::<u32>() {
                if v == value {
                    return true;
                }
            }
        }
        false
    };

    let minute = now.format("%M").to_string().parse::<u32>().unwrap_or(99);
    let hour = now.format("%H").to_string().parse::<u32>().unwrap_or(99);
    let dom = now.format("%d").to_string().parse::<u32>().unwrap_or(99);
    let month = now.format("%m").to_string().parse::<u32>().unwrap_or(99);
    let dow = now.format("%u").to_string().parse::<u32>().unwrap_or(99); // 1=Mon, 7=Sun

    if !match_part(parts[0], minute, false) {
        return false;
    }
    if !match_part(parts[1], hour, false) {
        return false;
    }
    if !match_part(parts[2], dom, false) {
        return false;
    }
    if !match_part(parts[3], month, false) {
        return false;
    }
    if !match_part(parts[4], dow, true) {
        return false;
    }

    // Don't re-fire if we already ran this minute
    if let Some(last) = last_run {
        if last.len() >= 16 {
            let now_min = now.format("%Y-%m-%dT%H:%M").to_string();
            if last.starts_with(&now_min) {
                return false;
            }
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

#[cfg(test)]
mod tests {
    use super::{should_fire, validate_cron_expr};
    use chrono::{TimeZone, Utc};

    #[test]
    fn accepts_supported_cron_syntax() {
        assert!(validate_cron_expr("*/5 8-18/2 * 1,6 MON-FRI").is_ok());
        assert!(validate_cron_expr("0 9 1 1 *").is_ok());
        assert!(validate_cron_expr("0 0 * * SUN").is_ok());
        assert!(validate_cron_expr("*,1 * * * *").is_ok());
    }

    #[test]
    fn rejects_invalid_cron_before_registration() {
        assert!(validate_cron_expr("*/0 * * * *").is_err());
        assert!(validate_cron_expr("60 * * * *").is_err());
        assert!(validate_cron_expr("0 24 * * *").is_err());
        assert!(validate_cron_expr("0 9 * *").is_err());
        assert!(validate_cron_expr("0, 9 * * *").is_err());
        assert!(validate_cron_expr("?/2 * * * *").is_err());
    }

    #[test]
    fn weekday_name_ranges_are_evaluated_by_the_scheduler() {
        let monday = Utc.with_ymd_and_hms(2024, 1, 1, 9, 0, 0).unwrap();
        let sunday = Utc.with_ymd_and_hms(2024, 1, 7, 9, 0, 0).unwrap();
        assert!(should_fire("0 9 * * MON-FRI", &None, &monday));
        assert!(!should_fire("0 9 * * MON-FRI", &None, &sunday));
    }
}
