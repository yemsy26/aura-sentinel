pub mod architect;
pub mod tester;
pub mod env_check;
pub mod memory;
pub mod security;
pub mod languages;
pub mod env_manager;
pub mod session_journal;
pub mod intent_router;
pub mod stub_enforcer;
pub mod dependency_mapper;
pub mod vision;
pub mod error_classifier;
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

    let stdout = child.stdout.take()
        .ok_or_else(|| format!("Error: stdout no está disponible para el task {}", task_id))?;
    let stderr = child.stderr.take()
        .ok_or_else(|| format!("Error: stderr no está disponible para el task {}", task_id))?;

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
        // BUG-4 FIX: Dynamically resolve Python path from USERPROFILE instead of hardcoding username
        let python_cmd = {
            let profile = std::env::var("USERPROFILE").unwrap_or_default();
            let scoop_python = std::path::PathBuf::from(&profile)
                .join("scoop").join("apps").join("python").join("current").join("python.exe");
            if scoop_python.exists() {
                scoop_python.to_string_lossy().into_owned()
            } else {
                "python".to_string()
            }
        };
        let output = Command::new(&python_cmd)
            .arg("-m")
            .arg("compileall")
            .arg("-q")
            .arg("-x")
            .arg("node_modules|\\.git|__pycache__|venv|\\.venv")
            .arg(".")
            .env("PYTHONUTF8", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .current_dir(workspace_path)
            .stdin(Stdio::null())
            .output()
            .await;

        match output {
            Ok(out) if out.status.success() => return Ok(()),
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                return Err(format!("{} {}", stdout, stderr).trim().to_string());
            },
            Err(_) => {
                // python not found in PATH — treat as a no-op (python may not be installed yet)
                return Ok(());
            }
        }
    }

    // 3. Node.js — smoke-test: verify npm scripts are actually defined
    if path.join("package.json").exists() {
        // Read the package.json to check if a "dev" or "start" script exists.
        // If neither exists, surface a meaningful error so the agent knows to fix it.
        let pkg_content = std::fs::read_to_string(path.join("package.json")).unwrap_or_default();
        let pkg_json_res = serde_json::from_str::<serde_json::Value>(&pkg_content);
        match pkg_json_res {
            Ok(pkg_json) => {
                let has_dev = pkg_json.pointer("/scripts/dev").is_some();
                let has_start = pkg_json.pointer("/scripts/start").is_some();
                let has_build = pkg_json.pointer("/scripts/build").is_some();

                if !has_dev && !has_start && !has_build {
                    return Err(
                        "[NODE_SCRIPTS_MISSING] El package.json existe pero NO tiene scripts 'dev', 'start' ni 'build' definidos. \
                        Esto significa que 'npm run dev' fallará con 'Missing script: dev'. \
                        Debes usar TOOL_PROGRAMMER para agregar los scripts correctos al package.json antes de continuar. \
                        Un archivo .bat que llame a 'npm run dev' sin que el script exista es un trabajo INCOMPLETO.".to_string()
                    );
                }
            },
            Err(e) => {
                return Err(format!(
                    "[JSON_SYNTAX_ERROR] El package.json tiene errores de sintaxis (Línea {}): {}. \
                    RECUERDA: JSON estricto no permite comentarios (ni // ni /* */) ni comas sueltas. \
                    Usa TOOL_PROGRAMMER para limpiar el package.json y dejarlo como JSON válido.",
                    e.line(), e
                ));
            }
        }
        return Ok(());
    }

    // 4. Generic Fallback
    Ok(())
}

/// Creates an emergency Git rollback checkpoint before executing risky code generation.
pub async fn create_git_backup(workspace_path: &str, commit_message: &str) -> Result<(), String> {
    let path = Path::new(workspace_path);

    // ── Protect Aura-internal files from being included in rollbacks ──────────
    // Write (or update) a .gitignore so session/memory files are never staged.
    let gitignore_path = path.join(".gitignore");
    let gitignore_content = "\
# === Aura-Sentinel internal files (never roll back) ===\n\
.aura_session.json\n\
.fenix_memory.json\n\
.fenix_chat.json\n\
\n\
# Node.js\n\
node_modules/\n\
";
    if !gitignore_path.exists() {
        if let Err(e) = std::fs::write(&gitignore_path, gitignore_content) {
            eprintln!("Aura-Sentinel Warning: No se pudo escribir .gitignore - {}", e);
        }
    } else {
        if let Ok(existing) = std::fs::read_to_string(&gitignore_path) {
            if !existing.contains(".fenix_memory.json") {
                if let Err(e) = std::fs::write(&gitignore_path, format!("{}\n{}", existing.trim_end(), gitignore_content)) {
                    eprintln!("Aura-Sentinel Warning: No se pudo actualizar .gitignore - {}", e);
                }
            }
        }
    }

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

    // Add all (gitignore protects the session files)
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

/// Restores the workspace to the last committed state using Git, reverting all uncommitted changes.
/// SAFETY: Uses `git restore` for tracked files and `git clean -fd` ONLY on files added
/// by Aura (i.e., files that appear in `git status --porcelain` as untracked '??' entries).
/// This prevents destroying files the user had before Aura started working.
pub async fn restore_git_backup(workspace_path: &str) -> Result<(), String> {
    let path = Path::new(workspace_path);
    if path.join(".git").exists() {
        // 1. Restore all tracked files to last commit state (safe — only touches versioned files)
        let _ = Command::new(get_shell())
            .args([get_shell_args(), "git restore ."])
            .current_dir(workspace_path)
            .stdin(Stdio::null())
            .output()
            .await;

        // 2. Only clean files that were explicitly staged/added by Aura (not pre-existing user files)
        // We use `git clean -fd --dry-run` first to inspect, then selectively clean only
        // files that appear in the git index (were added via `git add .` by Aura)
        let staged = Command::new(get_shell())
            .args([get_shell_args(), "git diff --name-only --cached"])
            .current_dir(workspace_path)
            .stdin(Stdio::null())
            .output()
            .await;

        if let Ok(out) = staged {
            let files = String::from_utf8_lossy(&out.stdout);
            for file in files.lines() {
                let file = file.trim();
                if !file.is_empty() {
                    let full_path = path.join(file);
                    // Only remove if it exists and was staged by Aura
                    let _ = std::fs::remove_file(&full_path);
                }
            }
        }

        // 3. Reset the index so the removed files are unstaged
        let _ = Command::new(get_shell())
            .args([get_shell_args(), "git reset HEAD ."])
            .current_dir(workspace_path)
            .stdin(Stdio::null())
            .output()
            .await;
    }
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
    // ── Windows Command Normalizer ────────────────────────────────────────────
    // Translates Unix-style commands and fixes common Windows path issues so
    // the LLM doesn't have to know the exact Windows syntax every time.
    let command = {
        let trimmed = command.trim();

        // 1. Unix → Windows command translations
        // ls → dir
        let trimmed = if trimmed == "ls" {
            "dir".to_string()
        } else if let Some(rest) = trimmed.strip_prefix("ls ") {
            format!("dir {}", rest)
        // cat → type
        } else if let Some(rest) = trimmed.strip_prefix("cat ") {
            let path = rest.trim().trim_matches('"').trim_matches('\'');
            format!("type \"{}\"", path)
        // touch → echo (create empty file)
        } else if let Some(rest) = trimmed.strip_prefix("touch ") {
            format!("echo. > \"{}\"", rest.trim())
        // rm → del
        } else if let Some(rest) = trimmed.strip_prefix("rm -rf ") {
            format!("rd /s /q \"{}\" 2>nul & del /f /q \"{}\" 2>nul", rest.trim(), rest.trim())
        } else if let Some(rest) = trimmed.strip_prefix("rm ") {
            format!("del /f \"{}\"", rest.trim())
        // cp → copy
        } else if let Some(rest) = trimmed.strip_prefix("cp ") {
            format!("copy {}", rest)
        // mv → move
        } else if let Some(rest) = trimmed.strip_prefix("mv ") {
            format!("move {}", rest)
        // pwd → cd (print working dir)
        } else if trimmed == "pwd" {
            "cd".to_string()
        // mkdir -p → mkdir
        } else if let Some(rest) = trimmed.strip_prefix("mkdir -p ") {
            format!("mkdir \"{}\"", rest.trim())
        // grep → findstr
        } else if let Some(rest) = trimmed.strip_prefix("grep ") {
            format!("findstr {}", rest)
        // clear → cls
        } else if trimmed == "clear" {
            "cls".to_string()
        } else {
            trimmed.to_string()
        };

        // 2. `open <path>` → `start "" "<path>"`  (macOS/Linux compat)
        let trimmed = if let Some(path) = trimmed.strip_prefix("open ") {
            let p = path.trim().trim_matches('"').trim_matches('\'');
            format!("start \"\" \"{}\"", p)
        } else {
            trimmed
        };

        // 3. `start file:///C:/path/with spaces/file.html`
        //    Windows `start` cannot handle file:// URLs with spaces.
        //    Convert: strip file://, unquote, re-quote properly.
        // Also handles: `start chrome/firefox/edge <path>` → `start "" "<path>"`
        let trimmed = if trimmed.to_lowercase().starts_with("start") {
            // 3a. Browser-prefixed open: `start chrome C:\path\file.html`
            let browsers = ["chrome", "firefox", "edge", "msedge", "iexplore"];
            let after_start = trimmed["start".len()..].trim();
            let browser_stripped = browsers.iter().find_map(|b| {
                let lower = after_start.to_lowercase();
                if lower.starts_with(b) && after_start.len() > b.len() && after_start.as_bytes().get(b.len()) == Some(&b' ') {
                    Some(after_start[b.len()..].trim())
                } else {
                    None
                }
            });

            if let Some(raw_path) = browser_stripped {
                // Strip file:// if present
                let raw_path = raw_path.trim_matches('"').trim_matches('\'');
                let raw_path = if raw_path.starts_with("file:///") { &raw_path[8..] }
                               else if raw_path.starts_with("file://") { &raw_path[7..] }
                               else { raw_path };
                let fixed = raw_path.replace('/', "\\");
                // Heal underscores→spaces if the exact path doesn't exist
                let healed = if !std::path::Path::new(&fixed).exists() {
                    let candidate = fixed.replace('_', " ");
                    if std::path::Path::new(&candidate).exists() { candidate } else { fixed }
                } else { fixed };
                format!("start \"\" \"{}\"", healed)
            } else if trimmed.contains("file:///") {
                // 3b. file:/// URL (no browser prefix)
                let after_start = after_start.trim_start_matches("\"\"").trim();
                let path_raw = after_start.trim_matches('"').trim_matches('\'');
                let path_raw = if path_raw.starts_with("file:///") { &path_raw[8..] }
                               else if path_raw.starts_with("file://") { &path_raw[7..] }
                               else { path_raw };
                let fixed = path_raw.replace('/', "\\");
                // Heal underscores→spaces
                let healed = if !std::path::Path::new(&fixed).exists() {
                    let candidate = fixed.replace('_', " ");
                    if std::path::Path::new(&candidate).exists() { candidate } else { fixed }
                } else { fixed };
                format!("start \"\" \"{}\"", healed)
            } else {
                // 3c. Relative path with spaces: `start index.html` or `start some file.html`
                if !after_start.starts_with('"') && after_start.contains(' ') {
                    format!("start \"\" \"{}\"", after_start)
                } else {
                    trimmed
                }
            }
        } else {
            trimmed
        };

        // 4. Detect blocking server commands and short-circuit them
        //    Commands like `python -m http.server`, `npx serve`, `npm start` etc.
        //    block the terminal forever. For static web projects just open the file.
        let trimmed_lower = trimmed.to_lowercase();
        let is_blocking_server = 
            trimmed_lower.contains("-m http.server") ||
            trimmed_lower.contains("-m httpserver") ||
            trimmed_lower.starts_with("npx serve") ||
            trimmed_lower.starts_with("npx http-server") ||
            trimmed_lower == "npm start" ||
            trimmed_lower.starts_with("serve ") || trimmed_lower == "serve" ||
            trimmed_lower.starts_with("live-server") ||
            trimmed_lower.starts_with("php -s");
        // Mark blocking commands so the caller can return early
        let trimmed = if is_blocking_server {
            // Return a synthetic echo so the terminal handler gets a fast OK
            format!("echo [SERVIDOR OMITIDO] '{}' es un servidor bloqueante. Para proyectos web estaticos usa 'start index.html' directamente.", trimmed)
        } else {
            trimmed
        };

        // 5. Prose/sentence detector — reject natural-language strings in the comando field.
        //    The LLM sometimes writes instructions like "Para verificar los archivos, ejecuta 'dir'"
        //    instead of a real command. Detect this and return an empty string so the caller
        //    triggers the empty-command handler (which has its own guidance loop).
        let trimmed = {
            let first_word = trimmed.split_whitespace().next().unwrap_or("").to_lowercase();
            // Spanish/English prose markers that are never valid shell commands
            let prose_starters = [
                "para", "por", "el", "la", "los", "las", "se", "si", "en",
                "ejecuta", "verifica", "abre", "asegurate", "navega",
                "hemos", "he", "ahora", "luego", "primero", "luego",
                "please", "run", "note", "make", "ensure", "check",
            ];
            // Also reject if the string contains `: ` (looks like "command: explanation")
            let has_colon_explanation = trimmed.contains(": ") && trimmed.len() > 40;
            if prose_starters.contains(&first_word.as_str()) || has_colon_explanation {
                // Try to extract a real command from inside quotes or after a colon
                let extracted = if let Some(after_colon) = trimmed.split("ejecuta:").nth(1)
                    .or_else(|| trimmed.split("command:").nth(1))
                    .or_else(|| trimmed.split("run:").nth(1))
                {
                    // Extract first quoted or unquoted token after the colon
                    let candidate = after_colon.trim().trim_matches(|c| c == '\'' || c == '"');
                    let first_cmd = candidate.split_whitespace().next().unwrap_or("");
                    if first_cmd.len() > 1 { first_cmd.to_string() } else { String::new() }
                } else {
                    String::new()
                };
                eprintln!("[PROSE_DETECTOR] Rejected prose comando: '{}' -> extracted: '{}'", &trimmed[..trimmed.len().min(60)], extracted);
                extracted
            } else {
                trimmed
            }
        };

        // 6. Path-splitter healer — detects when the LLM replaced spaces with backslashes
        //    in a Windows path, e.g. `explorer carpeta\de\prueba\10` instead of
        //    `explorer "carpeta de prueba 10"`. Heals by trying space-joined variants.
        let trimmed = {
            // Only applies to commands that open files/folders: explorer, start, code, notepad, etc.
            let openers = ["explorer", "start", "code", "notepad", "chrome", "firefox"];
            let first_word = trimmed.split_whitespace().next().unwrap_or("").to_lowercase();
            if openers.contains(&first_word.as_str()) {
                // Extract the path argument (everything after the first word)
                let rest = trimmed[first_word.len()..].trim().trim_matches('"').trim_matches('\'');
                // Check if the path doesn't exist but a space-joined version does
                // Pattern: detect sequences like `\de\`, `\preuba\`, `\de ` etc.
                // Strategy: try converting each `\word\` that's a short lowercase word (likely a space) back to ` word `
                let needs_healing = rest.contains('\\') && !std::path::Path::new(rest).exists() && !rest.starts_with('"');
                if needs_healing {
                    // Attempt 1: replace ALL backslashes with spaces and see if a variant exists
                    let space_path = rest.replace('\\', " ");
                    // Now try to find a valid prefix that is a real absolute Windows path
                    // e.g. "C: Users yemsy OneDrive..." → doesn't help
                    // Attempt 2: Split on `\` and for each token that is a short (<=4 char) lowercase word, 
                    // try merging with the previous token with a space.
                    let parts: Vec<&str> = rest.split('\\').collect();
                    let mut healed = String::new();
                    let mut i = 0;
                    while i < parts.len() {
                        let part = parts[i];
                        // If it looks like a drive root (C:), keep as path separator
                        if part.ends_with(':') || part.is_empty() {
                            if !healed.is_empty() { healed.push('\\'); }
                            healed.push_str(part);
                            i += 1;
                        } else if i + 1 < parts.len() {
                            // Check if concatenating current + space + next gives a real directory
                            let merged = format!("{} {}", part, parts[i + 1]);
                            let candidate_path = if healed.is_empty() {
                                merged.clone()
                            } else {
                                format!("{}\\{}", healed, merged)
                            };
                            if std::path::Path::new(&candidate_path).exists() {
                                if !healed.is_empty() { healed.push('\\'); }
                                healed.push_str(&merged);
                                i += 2; // consumed both parts
                            } else {
                                if !healed.is_empty() { healed.push('\\'); }
                                healed.push_str(part);
                                i += 1;
                            }
                        } else {
                            if !healed.is_empty() { healed.push('\\'); }
                            healed.push_str(part);
                            i += 1;
                        }
                    }
                    // Prefer healed path if the final candidate exists, otherwise use space_path
                    let final_path = if std::path::Path::new(&healed).exists() {
                        healed
                    } else if std::path::Path::new(&space_path).exists() {
                        space_path
                    } else {
                        rest.to_string()
                    };
                    if final_path != rest {
                        eprintln!("[PATH_SPLIT_HEAL] '{}' -> '{}'", rest, final_path);
                        format!("{} \"{}\"", first_word, final_path)
                    } else {
                        trimmed
                    }
                } else {
                    trimmed
                }
            } else {
                trimmed
            }
        };

        trimmed
    };
    let command = &command;

    // Resolve 'python' to the correct binary path so it works even when PATH is missing conda/miniconda
    let resolved_command = {
        let profile = std::env::var("USERPROFILE").unwrap_or_default();
        // Try Miniconda3 first, then Anaconda3, then scoop, then fallback to bare 'python'
        let candidates = vec![
            std::path::PathBuf::from(&profile).join("Miniconda3").join("python.exe"),
            std::path::PathBuf::from(&profile).join("Anaconda3").join("python.exe"),
            std::path::PathBuf::from(&profile).join("miniconda3").join("python.exe"),
            std::path::PathBuf::from(&profile).join("anaconda3").join("python.exe"),
            std::path::PathBuf::from(&profile).join("scoop").join("apps").join("python").join("current").join("python.exe"),
        ];
        let python_path = candidates.into_iter().find(|p| p.exists())
            .map(|p| p.to_string_lossy().into_owned());

        if let Some(py) = python_path {
            // Replace leading 'python ' or 'python\n' or exact 'python'
            if let Some(stripped) = command.strip_prefix("python ") {
                // Quote any absolute path argument that contains spaces
                let stripped_fixed = {
                    let args: Vec<&str> = stripped.splitn(2, ' ').collect();
                    if args.len() == 2 {
                        let script_arg = args[0].trim_matches('"').trim_matches('\'');
                        let rest = args[1];
                        let script_fixed = if std::path::Path::new(script_arg).is_absolute() && script_arg.contains(' ') {
                            format!("\"{}\" {}", script_arg, rest)
                        } else if script_arg.contains(' ') && !script_arg.starts_with('"') {
                            format!("\"{}\" {}", script_arg, rest)
                        } else {
                            stripped.to_string()
                        };
                        script_fixed
                    } else if stripped.contains(' ') && !stripped.starts_with('-') && !stripped.starts_with('"') {
                        // Single argument with spaces — likely a path without quotes
                        format!("\"{}\"", stripped)
                    } else {
                        stripped.to_string()
                    }
                };
                if py.contains(' ') {
                    format!("\"{}\" {}", py, stripped_fixed)
                } else {
                    format!("{} {}", py, stripped_fixed)
                }
            } else if *command == "python" {
                if py.contains(' ') {
                    format!("\"{}\" ", py)
                } else {
                    py.to_string()
                }
            } else {
                command.to_string()
            }
        } else {
            command.to_string()
        }
    };

    let output = Command::new(get_shell())
        .args([get_shell_args(), &resolved_command])
        .current_dir(workspace_path)
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
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

pub async fn format_system_error(error_msg: &str) -> String {
    if error_msg.to_lowercase().contains("not found") {
        let tasks = get_bg_tasks();
        let guard = tasks.lock().await;
        let active_ids: Vec<String> = guard.keys().cloned().collect();
        format!("{} Los IDs activos son {:?}. Corrige el nombre y reintenta.", error_msg, active_ids)
    } else {
        error_msg.to_string()
    }
}
