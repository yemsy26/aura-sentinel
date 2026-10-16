//! Motor "Agente Local": ejecuta `python -m agente <carpeta> --json` (agente para Bionic/LM Studio) y
//! traduce su protocolo de eventos a los que ya escucha la interfaz: `agent-step`, `file-updated` y
//! `agent-ask-user`. Una sesión por espacio de trabajo conserva la conversación entre mensajes.
//!
//! Protocolo (una línea JSON por mensaje): Aura envía `{"tipo":"tarea","texto":...}` y, tras un evento
//! `confirmar`, `{"tipo":"respuesta","si":bool}`. El agente emite `listo`, `paso`, `archivos`, `nota`,
//! `aviso`, `progreso`, `error` y, al acabar cada orden, `fin`.

use super::agent::emit_event;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, Mutex};

const SERVERS: [&str; 2] = ["http://127.0.0.1:1235", "http://127.0.0.1:1234"]; // Bionic, LM Studio
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct EngineSettings {
    /// "agente-local" o "aura" (motor original con Ollama).
    #[serde(default)]
    pub motor: String,
    /// Carpeta de instalación de Agente Local (contiene `.venv` y `agente/`).
    #[serde(default)]
    pub carpeta: String,
}

fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|dir| dir.join("motor.json"))
}

pub fn is_installation(dir: &Path) -> bool {
    dir.join(".venv").join("Scripts").join("python.exe").is_file()
        && dir.join("agente").join("__main__.py").is_file()
}

fn default_installation() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("USERPROFILE")?);
    [
        home.join(".gemini").join("antigravity").join("scratch").join("lm studio"),
        home.join("AgenteLocal"),
    ]
    .into_iter()
    .find(|dir| is_installation(dir))
}

/// motor.json de la configuración de la app; `AURA_MOTOR` y `AGENTE_LOCAL_DIR` lo sustituyen.
pub fn load_settings(app: &AppHandle) -> EngineSettings {
    let mut settings: EngineSettings = settings_path(app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    if let Ok(dir) = std::env::var("AGENTE_LOCAL_DIR") {
        settings.carpeta = dir;
    }
    if let Ok(motor) = std::env::var("AURA_MOTOR") {
        settings.motor = motor;
    }
    if settings.carpeta.is_empty() {
        if let Some(dir) = default_installation() {
            settings.carpeta = dir.to_string_lossy().into_owned();
        }
    }
    if settings.motor.is_empty() {
        let installed = is_installation(Path::new(&settings.carpeta));
        settings.motor = if installed { "agente-local" } else { "aura" }.into();
    }
    settings
}

/// `None` con el motor de Aura; con Agente Local, su carpeta o el motivo por el que no se puede usar.
pub fn selected(app: &AppHandle) -> Option<Result<PathBuf, String>> {
    let settings = load_settings(app);
    if settings.motor != "agente-local" {
        return None;
    }
    let dir = PathBuf::from(&settings.carpeta);
    Some(if is_installation(&dir) {
        Ok(dir)
    } else {
        Err(format!(
            "No encuentro Agente Local en '{}'. Debe contener .venv\\Scripts\\python.exe y agente\\__main__.py \
             (ejecuta instalar.bat allí) o elige el motor Aura.",
            settings.carpeta
        ))
    })
}

#[tauri::command]
pub fn get_engine_settings(app: AppHandle) -> Value {
    let settings = load_settings(&app);
    let installed = is_installation(Path::new(&settings.carpeta));
    json!({"motor": settings.motor, "carpeta": settings.carpeta, "instalado": installed})
}

#[tauri::command]
pub async fn save_engine_settings(
    app: AppHandle,
    motor: String,
    carpeta: Option<String>,
) -> Result<Value, String> {
    if motor != "agente-local" && motor != "aura" {
        return Err(format!("Motor desconocido: {motor}"));
    }
    let mut settings = load_settings(&app);
    settings.motor = motor;
    if let Some(dir) = carpeta.filter(|dir| !dir.trim().is_empty()) {
        settings.carpeta = dir;
    }
    if settings.motor == "agente-local" && !is_installation(Path::new(&settings.carpeta)) {
        return Err(format!("'{}' no es una instalación de Agente Local.", settings.carpeta));
    }
    let path = settings_path(&app).ok_or("No hay carpeta de configuración de la app.")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    // El próximo mensaje arranca con el motor elegido; con una misión en curso no se espera a que acabe.
    if let Ok(mut slot) = session().try_lock() {
        if let Some(old) = slot.take() {
            stop(old).await;
        }
    }
    Ok(get_engine_settings(app))
}

/// Modelos LLM de Bionic o LM Studio; los cargados primero.
pub async fn list_models() -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .map_err(|e| e.to_string())?;
    for base in SERVERS {
        let Ok(response) = client.get(format!("{base}/api/v1/models")).send().await else {
            continue;
        };
        let Ok(body) = response.json::<Value>().await else {
            continue;
        };
        return Ok(model_keys(&body));
    }
    Err("No responde Bionic (:1235) ni LM Studio (:1234). Abre Bionic para usar Agente Local.".into())
}

fn model_keys(body: &Value) -> Vec<String> {
    let mut models: Vec<(bool, String)> = body["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|model| model["type"] == "llm")
        .filter_map(|model| {
            let loaded = model["loaded_instances"].as_array().is_some_and(|i| !i.is_empty());
            Some((loaded, model["key"].as_str()?.to_string()))
        })
        .collect();
    models.sort_by_key(|(loaded, _)| !loaded);
    models.into_iter().map(|(_, key)| key).collect()
}

struct Session {
    workspace: String,
    model: String,
    child: Child,
    stdin: ChildStdin,
    events: mpsc::UnboundedReceiver<Value>,
}

fn session() -> &'static Mutex<Option<Session>> {
    static SESSION: OnceLock<Mutex<Option<Session>>> = OnceLock::new();
    SESSION.get_or_init(|| Mutex::new(None))
}

fn log_path() -> PathBuf {
    std::env::temp_dir().join("aura_agente_local.log")
}

fn log_tail() -> String {
    let text = std::fs::read_to_string(log_path()).unwrap_or_default();
    let start = text.char_indices().rev().nth(1500).map_or(0, |(i, _)| i);
    text[start..].trim().to_string()
}

async fn spawn(dir: &Path, workspace: &str, model: &str) -> Result<Session, String> {
    let python = dir.join(".venv").join("Scripts").join("python.exe");
    let log = std::fs::File::create(log_path()).map_err(|e| e.to_string())?;
    let mut command = Command::new(&python);
    command.args(["-m", "agente", workspace, "--json"]);
    if !model.is_empty() {
        command.args(["--modelo", model]);
    }
    command
        .current_dir(dir)
        .env("PYTHONUTF8", "1")
        .env("PYTHONUNBUFFERED", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(log))
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let mut child = command
        .spawn()
        .map_err(|e| format!("No pude iniciar Agente Local ({}): {e}", python.display()))?;
    let stdin = child.stdin.take().ok_or("Agente Local sin entrada estándar")?;
    let stdout = child.stdout.take().ok_or("Agente Local sin salida estándar")?;
    let (sender, events) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(event) = serde_json::from_str::<Value>(&line) {
                if sender.send(event).is_err() {
                    break;
                }
            }
        } // al cerrarse la salida, el canal se cierra y recv() devuelve None
    });
    Ok(Session { workspace: workspace.into(), model: model.into(), child, stdin, events })
}

/// Detiene el agente y todo lo que lanzó (servidores en segundo plano incluidos).
async fn stop(mut session: Session) {
    if let Some(pid) = session.child.id() {
        let mut taskkill = Command::new("taskkill");
        taskkill.args(["/PID", &pid.to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null());
        #[cfg(windows)]
        taskkill.creation_flags(CREATE_NO_WINDOW);
        let _ = taskkill.status().await;
    }
    let _ = session.child.kill().await;
}

async fn send(session: &mut Session, order: Value) -> Result<(), String> {
    let line = format!("{order}\n");
    session.stdin.write_all(line.as_bytes()).await.map_err(|e| e.to_string())?;
    session.stdin.flush().await.map_err(|e| e.to_string())
}

enum Next {
    Event(Value),
    Cancelled,
    Closed,
}

async fn next(session: &mut Session) -> Next {
    loop {
        tokio::select! {
            event = session.events.recv() => return event.map_or(Next::Closed, Next::Event),
            _ = tokio::time::sleep(Duration::from_millis(250)) => {
                if super::is_agent_cancelled() {
                    return Next::Cancelled;
                }
            }
        }
    }
}

fn finish(status: &str, text: &str) -> String {
    json!({"status": status, "respuesta_conversacional": text}).to_string()
}

pub(crate) fn step_status(tool: &str, error: bool) -> &'static str {
    match tool {
        _ if error => "ERROR",
        "verificar" | "revisar" => "VALIDATING",
        "plan" | "paso_hecho" => "DECISION",
        "terminar" | "escribir" | "editar" | "crear_bat" | "restaurar" | "ejecutar" => "SUCCESS",
        _ => "ACTION",
    }
}

/// Texto del paso con las marcas que la telemetría de la interfaz ya reconoce.
pub(crate) fn step_message(event: &Value, workspace: &str) -> String {
    let text = |key: &str| event[key].as_str().unwrap_or_default();
    let tool = text("herramienta");
    let plan_items: Vec<&str> = text("plan").lines().filter(|line| line.starts_with("- [")).collect();
    if tool == "plan" && !plan_items.is_empty() {
        let phases: Vec<String> = plan_items
            .iter()
            .enumerate()
            .map(|(i, line)| format!("Fase {}: {}", i + 1, plan_step_text(line)))
            .collect();
        return format!("Plan generado: {}", phases.join(" | "));
    }
    if tool == "paso_hecho" {
        let done = plan_items.iter().filter(|line| line.starts_with("- [x]")).count();
        if done > 0 {
            return format!("[FASE {done} COMPLETADA] {}", text("resultado"));
        }
    }
    let detail = short_path(text("detalle"), workspace);
    let result = text("resultado");
    let arrow = if result.is_empty() { String::new() } else { format!(" → {result}") };
    format!("[{}] {detail}{arrow}", tool.to_uppercase()).replace("]  →", "] →")
}

fn plan_step_text(line: &str) -> &str {
    let rest = line.get(5..).unwrap_or_default().trim_start(); // tras "- [ ]" o "- [x]"
    let rest = rest.split_once(". ").filter(|(n, _)| n.chars().all(|c| c.is_ascii_digit())).map_or(rest, |(_, t)| t);
    rest.split(" — ").next().unwrap_or(rest)
}

fn short_path<'a>(text: &'a str, workspace: &str) -> &'a str {
    text.strip_prefix(workspace).map_or(text, |rest| rest.trim_start_matches(['\\', '/']))
}

/// Ruta absoluta con los separadores de Windows, igual que las pestañas abiertas del editor.
pub(crate) fn workspace_file(workspace: &str, file: &str) -> PathBuf {
    Path::new(workspace).join(file).components().collect()
}

fn forward(app: &AppHandle, workspace: &str, event: &Value, step: &mut u32) {
    let text = |key: &str| event[key].as_str().unwrap_or_default().to_string();
    match event["tipo"].as_str().unwrap_or_default() {
        "paso" => {
            *step = event["paso"].as_u64().unwrap_or(*step as u64) as u32;
            let status = step_status(&text("herramienta"), event["error"].as_bool().unwrap_or(false));
            emit_event(app, *step, &step_message(event, workspace), status);
        }
        "archivos" => {
            for file in event["rutas"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                let path = workspace_file(workspace, file);
                let _ = app.emit("file-updated", json!({"path": path.to_string_lossy()}));
            }
        }
        "nota" => emit_event(app, *step, text("texto").trim_start_matches("· "), "PLANNING"),
        "aviso" => emit_event(app, *step, &text("texto"), "WARNING"),
        "error" => emit_event(app, *step, &text("texto"), "ERROR"),
        _ => {} // progreso: la interfaz ya muestra el paso en curso
    }
}

async fn confirm(app: &AppHandle, session: &mut Session, command: &str) -> Result<(), String> {
    let answer = crate::core::ask_user::ask_user_async(
        app,
        format!("El agente quiere ejecutar un comando potencialmente destructivo:\n{command}"),
        vec!["Permitir".into(), "Bloquear".into()],
        "Agente Local".into(),
    )
    .await
    .unwrap_or_default();
    send(session, json!({"tipo": "respuesta", "si": answer == "Permitir"})).await
}

/// Ejecuta un mensaje del chat con Agente Local y devuelve la respuesta final en el formato de Aura.
pub async fn run_mission(
    app: &AppHandle,
    dir: PathBuf,
    workspace: String,
    message: String,
    model: String,
) -> Result<String, String> {
    let known = list_models().await.unwrap_or_default();
    let model = if known.contains(&model) { model } else { String::new() }; // vacío = el de agente.json
    let mut slot = session().lock().await;
    let mut previous = slot.take();
    let reusable = previous.as_mut().is_some_and(|s| {
        s.workspace == workspace && s.model == model && matches!(s.child.try_wait(), Ok(None))
    });
    let mut current = match previous {
        Some(s) if reusable => s,
        previous => {
            if let Some(old) = previous {
                stop(old).await;
            }
            emit_event(app, 0, "[AGENTE LOCAL] Iniciando el motor (si el modelo no está cargado, tarda un minuto)...", "PLANNING");
            let mut fresh = spawn(&dir, &workspace, &model).await?;
            let mut step = 0;
            loop {
                match next(&mut fresh).await {
                    Next::Event(event) if event["tipo"] == "listo" => {
                        let info = format!(
                            "[AGENTE LOCAL] Listo · modelo {} · contexto {} tokens",
                            event["modelo"].as_str().unwrap_or("?"),
                            event["contexto"]
                        );
                        emit_event(app, 0, &info, "PLANNING");
                        break fresh;
                    }
                    Next::Event(event) if event["tipo"] == "error" => {
                        stop(fresh).await;
                        return Ok(finish("ERROR", event["texto"].as_str().unwrap_or("Error al iniciar Agente Local.")));
                    }
                    Next::Event(event) => forward(app, &workspace, &event, &mut step),
                    Next::Cancelled => {
                        stop(fresh).await;
                        return Ok(finish("CANCELLED", "Inicio de Agente Local cancelado."));
                    }
                    Next::Closed => {
                        return Ok(finish("ERROR", &format!("Agente Local se cerró al iniciar.\n{}", log_tail())));
                    }
                }
            }
        }
    };

    send(&mut current, json!({"tipo": "tarea", "texto": message})).await?;
    let mut step = 0;
    loop {
        match next(&mut current).await {
            Next::Event(event) => match event["tipo"].as_str().unwrap_or_default() {
                "fin" => {
                    let status = if event["estado"] == "error" { "ERROR" } else { "FINISH" };
                    let text = event["respuesta"].as_str().unwrap_or_default().to_string();
                    *slot = Some(current); // la conversación sigue en el próximo mensaje
                    return Ok(finish(status, &text));
                }
                "confirmar" => {
                    let command = event["comando"].as_str().unwrap_or_default().to_string();
                    confirm(app, &mut current, &command).await?;
                }
                _ => forward(app, &workspace, &event, &mut step),
            },
            Next::Cancelled => {
                stop(current).await;
                return Ok(finish(
                    "CANCELLED",
                    "Misión cancelada: detuve el agente y sus procesos. Escribe 'continúa' para retomarla.",
                ));
            }
            Next::Closed => {
                return Ok(finish("ERROR", &format!("Agente Local se cerró inesperadamente.\n{}", log_tail())));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_match_the_interface_palette() {
        assert_eq!(step_status("escribir", false), "SUCCESS");
        assert_eq!(step_status("escribir", true), "ERROR");
        assert_eq!(step_status("verificar", false), "VALIDATING");
        assert_eq!(step_status("plan", false), "DECISION");
        assert_eq!(step_status("leer", false), "ACTION");
    }

    #[test]
    fn plan_and_completed_steps_feed_the_phase_bar() {
        let plan = "- [x] 1. Crear juego.py — hecho\n- [ ] 2. Tests";
        let event = json!({"herramienta": "plan", "plan": plan, "resultado": "OK"});
        assert_eq!(step_message(&event, "C:\\ws"), "Plan generado: Fase 1: Crear juego.py | Fase 2: Tests");
        let done = json!({"herramienta": "paso_hecho", "plan": plan, "resultado": "Paso 1 hecho"});
        assert_eq!(step_message(&done, "C:\\ws"), "[FASE 1 COMPLETADA] Paso 1 hecho");
    }

    #[test]
    fn paths_are_shortened_and_normalized() {
        let event = json!({"herramienta": "escribir", "detalle": "C:\\ws\\src\\a.py", "resultado": "OK creado"});
        assert_eq!(step_message(&event, "C:\\ws"), "[ESCRIBIR] src\\a.py → OK creado");
        assert_eq!(workspace_file("C:\\ws", "src/a.py"), PathBuf::from("C:\\ws\\src\\a.py"));
        assert_eq!(workspace_file("C:\\ws", "C:\\ws\\b.py"), PathBuf::from("C:\\ws\\b.py"));
    }

    #[test]
    fn loaded_models_come_first() {
        let body = json!({"models": [
            {"type": "llm", "key": "a", "loaded_instances": []},
            {"type": "embedding", "key": "e", "loaded_instances": []},
            {"type": "llm", "key": "b", "loaded_instances": [{"id": "b"}]}
        ]});
        assert_eq!(model_keys(&body), vec!["b", "a"]);
    }
}
