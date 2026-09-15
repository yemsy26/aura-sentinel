use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tauri::Emitter;
use tokio::sync::oneshot;

type PromptsMap = HashMap<String, oneshot::Sender<String>>;

fn pending_prompts() -> &'static Mutex<PromptsMap> {
    static PENDING: OnceLock<Mutex<PromptsMap>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Serialize, Clone)]
pub struct AskEvent {
    pub id: String,
    pub question: String,
    pub options: Vec<String>,
    pub context: String,
}

/// Ask the user a question via a dialog in the Tauri frontend.
/// This function blocks the async task until the user responds.
pub async fn ask_user_async(
    app: &tauri::AppHandle,
    question: String,
    options: Vec<String>,
    context: String,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();

    {
        let mut map = pending_prompts().lock().unwrap();
        map.insert(id.clone(), tx);
    }

    let event = AskEvent {
        id: id.clone(),
        question,
        options,
        context,
    };

    if let Err(error) = app.emit("agent-ask-user", event) {
        pending_prompts().lock().unwrap().remove(&id);
        return Err(format!("No se pudo mostrar la pregunta: {}", error));
    }

    let mut rx = rx;
    let result = loop {
        tokio::select! {
            reply = &mut rx => break reply.map_err(|_| "Pregunta interrumpida".to_string()),
            _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {
                if crate::llm::is_agent_cancelled() {
                    break Err("Misión cancelada mientras se esperaba una respuesta".to_string());
                }
            }
        }
    };
    pending_prompts().lock().unwrap().remove(&id);
    let _ = app.emit("agent-ask-user-closed", serde_json::json!({ "id": id }));
    result
}

/// Comando Tauri para que el frontend envíe la respuesta.
#[tauri::command]
pub fn submit_user_answer(id: String, answer: String) -> Result<(), String> {
    let mut map = pending_prompts().lock().unwrap();
    if let Some(tx) = map.remove(&id) {
        let _ = tx.send(answer);
        Ok(())
    } else {
        Err("ID de pregunta no encontrado o ya respondido.".to_string())
    }
}
