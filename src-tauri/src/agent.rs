//! Motor de las misiones: Agente Local (agente para Bionic/LM Studio). Aura ejecuta
//! `python -m agente <carpeta> --json` y traduce su protocolo a los eventos que escucha la interfaz:
//! `agent-event` (el evento tal cual), `agent-step`, `file-updated` y `agent-ask-user`. Una sesión por carpeta y modelo conserva la
//! conversación entre mensajes.
//!
//! Protocolo (una línea JSON por mensaje): Aura envía `{"tipo":"tarea","texto":...}` y, tras un evento
//! `confirmar`, `{"tipo":"respuesta","si":bool}`. El agente emite `listo`, `paso`, `archivos`, `nota`,
//! `aviso`, `progreso`, `error`, `confirmar` y, al acabar cada orden, `fin`. Las órdenes rápidas
//! (verificar, revisar, procesos, restaurar...) usan la misma sesión con `agent_order`.

use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, Mutex};

const SERVERS: [&str; 2] = ["http://127.0.0.1:1235", "http://127.0.0.1:1234"]; // Bionic, LM Studio
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ── Estado de la misión ─────────────────────────────────────────────────────

static RUNNING: AtomicBool = AtomicBool::new(false);
static CANCELLED: AtomicBool = AtomicBool::new(false);

/// Una sola misión a la vez; se libera al soltarse.
struct MissionLock;

impl MissionLock {
    fn try_lock() -> Option<Self> {
        (!RUNNING.swap(true, Ordering::SeqCst)).then_some(MissionLock)
    }
}

impl Drop for MissionLock {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

pub fn is_agent_cancelled() -> bool {
    CANCELLED.load(Ordering::SeqCst)
}

#[tauri::command]
pub fn cancel_agent_mission() -> bool {
    CANCELLED.store(true, Ordering::SeqCst);
    true
}

#[derive(Serialize, Clone)]
struct AgentEvent {
    step: u32,
    message: String,
    status: String,
}

pub fn emit_event(app: &AppHandle, step: u32, message: &str, status: &str) {
    let event = AgentEvent { step, message: message.to_string(), status: status.to_string() };
    let _ = app.emit("agent-step", event);
}

fn finish(status: &str, text: &str) -> String {
    json!({"status": status, "respuesta_conversacional": text}).to_string()
}

// ── Instalación de Agente Local ─────────────────────────────────────────────

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

/// Carpeta de Agente Local: `AGENTE_LOCAL_DIR`, la guardada en motor.json o la ubicación por defecto.
pub fn installation_dir(app: &AppHandle) -> String {
    if let Ok(dir) = std::env::var("AGENTE_LOCAL_DIR") {
        return dir;
    }
    settings_path(app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| value["carpeta"].as_str().map(str::to_string))
        .filter(|dir| !dir.is_empty())
        .or_else(|| default_installation().map(|dir| dir.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

fn installation(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = installation_dir(app);
    if is_installation(Path::new(&dir)) {
        Ok(PathBuf::from(dir))
    } else {
        Err(format!(
            "No encuentro Agente Local en '{dir}'. Debe contener .venv\\Scripts\\python.exe y agente\\__main__.py \
             (ejecuta instalar.bat allí) o indica su carpeta en la configuración."
        ))
    }
}

#[tauri::command]
pub fn get_agent_settings(app: AppHandle) -> Value {
    let dir = installation_dir(&app);
    json!({"carpeta": dir, "instalado": is_installation(Path::new(&dir))})
}

#[tauri::command]
pub async fn save_agent_settings(app: AppHandle, carpeta: String) -> Result<Value, String> {
    if !is_installation(Path::new(&carpeta)) {
        return Err(format!("'{carpeta}' no es una instalación de Agente Local."));
    }
    let path = settings_path(&app).ok_or("No hay carpeta de configuración de la app.")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, json!({"carpeta": carpeta}).to_string()).map_err(|e| e.to_string())?;
    // El próximo mensaje arranca con la nueva instalación; con una misión en curso no se espera.
    if let Ok(mut slot) = session().try_lock() {
        if let Some(old) = slot.take() {
            stop(old).await;
        }
    }
    Ok(get_agent_settings(app))
}

// ── Modelos ─────────────────────────────────────────────────────────────────

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

#[tauri::command]
pub async fn get_models() -> Result<Vec<String>, String> {
    list_models().await
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

// ── Misiones pendientes (plan de Agente Local con pasos sin marcar) ─────────

fn is_resume(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    ["contin", "sigue", "retoma", "resume"].iter().any(|w| lower.starts_with(w))
}

/// Misión a medio hacer en la carpeta: el plan tiene pasos sin marcar. El objetivo es la última
/// petición del usuario en el registro del agente (no un "continúa").
pub fn pending_mission(workspace: &Path) -> Option<Value> {
    let plan = std::fs::read_to_string(workspace.join(".agente").join("plan.md")).ok()?;
    let steps: Vec<&str> = plan.lines().filter(|line| line.starts_with("- [")).collect();
    let done = steps.iter().filter(|line| line.starts_with("- [x]")).count();
    if steps.is_empty() || done == steps.len() {
        return None;
    }
    let log = std::fs::read_to_string(workspace.join(".agente").join("registro.log")).unwrap_or_default();
    let objective = log
        .lines()
        .rev()
        .filter_map(|line| line.split_once("] USUARIO: ").map(|(_, text)| text.trim()))
        .find(|text| !is_resume(text))
        .map_or_else(|| plan_step_text(steps[done]).to_string(), str::to_string);
    Some(json!({"objective": objective, "workspace": workspace.to_string_lossy(), "step": done,
                "role": format!("{done}/{} pasos", steps.len())}))
}

#[tauri::command]
pub fn get_pending_mission(workspace_path: Option<String>) -> Option<Value> {
    pending_mission(Path::new(&workspace_path.filter(|w| !w.is_empty())?))
}

#[tauri::command]
pub fn dismiss_pending_mission(workspace_path: String) -> Result<(), String> {
    let dir = Path::new(&workspace_path).join(".agente");
    std::fs::rename(dir.join("plan.md"), dir.join("plan_descartado.md")).map_err(|e| e.to_string())
}

// ── Sesión con el proceso del agente ────────────────────────────────────────

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
                if is_agent_cancelled() {
                    return Next::Cancelled;
                }
            }
        }
    }
}

// ── Traducción de eventos ───────────────────────────────────────────────────

pub(crate) fn step_status(tool: &str, error: bool) -> &'static str {
    match tool {
        _ if error => "ERROR",
        "verificar" | "revisar" | "ver_pantalla" => "VALIDATING",
        "plan" | "paso_hecho" => "DECISION",
        "terminar" | "escribir" | "editar" | "crear_bat" | "restaurar" | "ejecutar" | "git" => "SUCCESS",
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
    let error = event["error"].as_bool().unwrap_or(false);
    if tool == "paso_hecho" && !error {
        let done = plan_items.iter().filter(|line| line.starts_with("- [x]")).count();
        if done > 0 {
            return format!("[FASE {done} COMPLETADA] {}", text("resultado"));
        }
    }
    let detail = short_path(text("detalle"), workspace);
    let result = text("resultado");
    let arrow = if result.is_empty() { String::new() } else { format!(" → {result}") };
    // En un error, la primera línea concreta del informe dice qué falla ("- texto cortado ...").
    let reason = if error {
        text("salida")
            .lines()
            .skip(1)
            .map(str::trim)
            .find(|line| line.starts_with("- "))
            .map(|line| format!(" · {}", line.trim_start_matches("- ")))
            .unwrap_or_default()
    } else {
        String::new()
    };
    format!("[{}] {detail}{arrow}{reason}", tool.to_uppercase()).replace("]  →", "] →")
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
    let _ = app.emit("agent-event", event); // la interfaz pinta tarjetas con el evento completo
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
    let answer = crate::ask_user::ask_user_async(
        app,
        format!("El agente quiere ejecutar un comando potencialmente destructivo:\n{command}"),
        vec!["Permitir".into(), "Bloquear".into()],
        "Agente Local".into(),
    )
    .await
    .unwrap_or_default();
    send(session, json!({"tipo": "respuesta", "si": answer == "Permitir"})).await
}

/// Preferencias del perfil local, para que el agente las conozca desde el primer mensaje.
fn with_profile(app: &AppHandle, workspace: &str, message: String) -> String {
    let snapshot = crate::user_profile::load(app, Some(workspace)).unwrap_or_default();
    let context = crate::user_profile::prompt_context(&snapshot);
    if context.trim().is_empty() {
        message
    } else {
        format!("{message}\n\n(Perfil del usuario, tenlo en cuenta: {})", context.trim())
    }
}

// ── Misión y órdenes ────────────────────────────────────────────────────────

/// Por qué terminó una orden sin su evento `fin`: estado para la interfaz y texto.
type Halt = (&'static str, String);

/// Modelo pedido si Bionic lo conoce; vacío = el de agente.json.
async fn known_model(model: Option<String>) -> String {
    let model = model.unwrap_or_default();
    let known = list_models().await.unwrap_or_default();
    if known.contains(&model) { model } else { String::new() }
}

fn reusable(session: &mut Session, workspace: &str, model: Option<&str>) -> bool {
    session.workspace == workspace
        && model.is_none_or(|m| session.model == m)
        && matches!(session.child.try_wait(), Ok(None))
}

/// Sesión para la carpeta: la abierta si sirve (`model` None = con cualquier modelo) o una nueva
/// esperando su `listo`. Devuelve también si es nueva.
async fn ready_session(
    app: &AppHandle,
    workspace: &str,
    model: Option<&str>,
    previous: Option<Session>,
) -> Result<(Session, bool), Halt> {
    let mut previous = previous;
    if previous.as_mut().is_some_and(|s| reusable(s, workspace, model)) {
        return Ok((previous.unwrap(), false));
    }
    if let Some(old) = previous {
        stop(old).await;
    }
    let dir = installation(app).map_err(|reason| ("ERROR", reason))?;
    emit_event(app, 0, "[AGENTE LOCAL] Iniciando el motor (si el modelo no está cargado, tarda un minuto)...", "PLANNING");
    let mut fresh = spawn(&dir, workspace, model.unwrap_or_default()).await.map_err(|e| ("ERROR", e))?;
    let mut step = 0;
    loop {
        match next(&mut fresh).await {
            Next::Event(event) if event["tipo"] == "listo" => {
                let _ = app.emit("agent-event", &event);
                let info = format!(
                    "[AGENTE LOCAL] Listo · modelo {} · contexto {} tokens",
                    event["modelo"].as_str().unwrap_or("?"),
                    event["contexto"]
                );
                emit_event(app, 0, &info, "PLANNING");
                return Ok((fresh, true));
            }
            Next::Event(event) if event["tipo"] == "error" => {
                let _ = app.emit("agent-event", &event);
                stop(fresh).await;
                return Err(("ERROR", event["texto"].as_str().unwrap_or("Error al iniciar Agente Local.").to_string()));
            }
            Next::Event(event) => forward(app, workspace, &event, &mut step),
            Next::Cancelled => {
                stop(fresh).await;
                return Err(("CANCELLED", "Inicio de Agente Local cancelado.".into()));
            }
            Next::Closed => return Err(("ERROR", format!("Agente Local se cerró al iniciar.\n{}", log_tail()))),
        }
    }
}

/// Envía una orden y reenvía sus eventos hasta el `fin`, que devuelve; la sesión vuelve a `slot`.
async fn run_order(
    app: &AppHandle,
    workspace: &str,
    mut current: Session,
    order: Value,
    slot: &mut Option<Session>,
) -> Result<Value, Halt> {
    send(&mut current, order).await.map_err(|e| ("ERROR", e))?;
    let mut step = 0;
    loop {
        match next(&mut current).await {
            Next::Event(event) => match event["tipo"].as_str().unwrap_or_default() {
                "fin" => {
                    let _ = app.emit("agent-event", &event);
                    *slot = Some(current); // la conversación sigue en el próximo mensaje
                    return Ok(event);
                }
                "confirmar" => {
                    let _ = app.emit("agent-event", &event);
                    let command = event["comando"].as_str().unwrap_or_default().to_string();
                    confirm(app, &mut current, &command).await.map_err(|e| ("ERROR", e))?;
                }
                _ => forward(app, workspace, &event, &mut step),
            },
            Next::Cancelled => {
                stop(current).await;
                return Err((
                    "CANCELLED",
                    "Misión cancelada: detuve el agente y sus procesos. Escribe 'continúa' para retomarla.".into(),
                ));
            }
            Next::Closed => return Err(("ERROR", format!("Agente Local se cerró inesperadamente.\n{}", log_tail()))),
        }
    }
}

/// Ejecuta un mensaje del chat con Agente Local y devuelve la respuesta final en el formato de la interfaz.
#[tauri::command]
pub async fn process_user_prompt(
    user_message: String,
    workspace_path: String,
    model: Option<String>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let Some(_lock) = MissionLock::try_lock() else {
        return Ok(finish("FINISH", "⚠️ Ya hay una misión en curso. Espera a que termine o cancélala."));
    };
    CANCELLED.store(false, Ordering::SeqCst);
    let mut message = user_message;
    if message.to_lowercase().starts_with("[user]") {
        message = message[6..].trim().to_string(); // historiales pegados por error
    }
    if let Some(idx) = message.find("[SYSTEM]") {
        message.truncate(idx);
    }
    let message = message.trim().to_string();
    if !is_resume(&message) {
        let ws = workspace_path.clone(); // base de "Ver cambios" para esta misión (lee disco: fuera del runtime)
        let _ = tokio::task::spawn_blocking(move || crate::ide::snapshot_workspace(&ws)).await;
    }
    let model = known_model(model).await;
    let mut slot = session().lock().await;
    let result = match ready_session(&app_handle, &workspace_path, Some(&model), slot.take()).await {
        Ok((current, new_session)) => {
            let text = if new_session && !is_resume(&message) {
                with_profile(&app_handle, &workspace_path, message)
            } else {
                message
            };
            run_order(&app_handle, &workspace_path, current, json!({"tipo": "tarea", "texto": text}), &mut slot).await
        }
        Err(halt) => Err(halt),
    };
    Ok(match result {
        Ok(fin) => {
            let status = if fin["estado"] == "error" { "ERROR" } else { "FINISH" };
            finish(status, fin["respuesta"].as_str().unwrap_or_default())
        }
        Err((status, text)) => finish(status, &text),
    })
}

const ORDERS: [&str; 8] = ["verificar", "revisar", "plan", "procesos", "detener", "restaurar", "regla", "nuevo"];

/// Órdenes rápidas de la interfaz. Devuelve el evento `fin` (respuesta, uso del contexto y, en
/// `procesos`, la lista estructurada en `datos`).
#[tauri::command]
pub async fn agent_order(order: Value, workspace_path: String, app_handle: AppHandle) -> Result<Value, String> {
    let kind = order["tipo"].as_str().unwrap_or_default().to_string();
    if !ORDERS.contains(&kind.as_str()) {
        return Err(format!("Orden no permitida: {kind}"));
    }
    let Some(_lock) = MissionLock::try_lock() else {
        return Err("El agente está trabajando en una misión; espera a que termine.".into());
    };
    CANCELLED.store(false, Ordering::SeqCst);
    let mut slot = session().lock().await;
    let live = slot.as_mut().is_some_and(|s| reusable(s, &workspace_path, None));
    if !live && matches!(kind.as_str(), "procesos" | "detener" | "nuevo") {
        // Sin sesión no hay procesos ni historial: no se arranca el agente (ni se carga el modelo) para nada.
        let text = if kind == "nuevo" { "Conversación nueva." } else { "No hay procesos del agente en marcha." };
        return Ok(json!({"tipo": "fin", "orden": kind, "estado": "ok", "datos": [], "respuesta": text}));
    }
    let (current, _) = ready_session(&app_handle, &workspace_path, None, slot.take()).await.map_err(|(_, text)| text)?;
    run_order(&app_handle, &workspace_path, current, order, &mut slot).await.map_err(|(_, text)| text)
}

/// Reglas del usuario y lecciones aprendidas que el agente aplica en cada tarea.
#[tauri::command]
pub fn get_agent_rules(app: AppHandle) -> Value {
    let dir = PathBuf::from(installation_dir(&app));
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap_or_default();
    json!({"reglas": read("reglas.md"), "lecciones": read("lecciones.md")})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_match_the_interface_palette() {
        assert_eq!(step_status("escribir", false), "SUCCESS");
        assert_eq!(step_status("escribir", true), "ERROR");
        assert_eq!(step_status("verificar", false), "VALIDATING");
        assert_eq!(step_status("ver_pantalla", false), "VALIDATING");
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
        let rejected = json!({"herramienta": "paso_hecho", "plan": plan, "error": true, "resultado": "RECHAZADO: falta"});
        assert_eq!(step_message(&rejected, "C:\\ws"), "[PASO_HECHO] → RECHAZADO: falta");
    }

    #[test]
    fn errors_show_the_first_concrete_reason() {
        let event = json!({"herramienta": "ver_pantalla", "detalle": "index.html", "error": true,
            "resultado": "VISUAL: PROBLEMAS en index.html",
            "salida": "VISUAL: PROBLEMAS en index.html\nComprobación automática:\n- texto cortado: 'Puntos'\n- otro"});
        assert_eq!(
            step_message(&event, "C:\\ws"),
            "[VER_PANTALLA] index.html → VISUAL: PROBLEMAS en index.html · texto cortado: 'Puntos'"
        );
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

    #[test]
    fn pending_mission_comes_from_the_agent_plan() {
        let ws = std::env::temp_dir().join(format!("aura_pend_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(ws.join(".agente")).unwrap();
        assert!(pending_mission(&ws).is_none());
        std::fs::write(ws.join(".agente").join("plan.md"), "# Plan\n- [x] 1. Crear index.html\n- [ ] 2. Probar\n").unwrap();
        std::fs::write(ws.join(".agente").join("registro.log"),
                       "[10:00:00] USUARIO: crea un catálogo\n[10:05:00] USUARIO: continúa\n").unwrap();
        let pending = pending_mission(&ws).unwrap();
        assert_eq!(pending["objective"], "crea un catálogo");
        assert_eq!(pending["step"], 1);
        dismiss_pending_mission(ws.to_string_lossy().into_owned()).unwrap();
        assert!(pending_mission(&ws).is_none());
        let _ = std::fs::remove_dir_all(&ws);
    }
}
