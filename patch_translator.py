import os

# 1. Update mod.rs to make call_ollama_text public and add pub mod translator;
mod_rs_path = "src-tauri/src/llm/mod.rs"
mod_rs = open(mod_rs_path, "r", encoding="utf-8").read()

if "pub mod translator;" not in mod_rs:
    mod_rs = mod_rs.replace("pub mod agent;", "pub mod agent;\npub mod translator;")

if "async fn call_ollama_text(" in mod_rs and "pub async fn call_ollama_text(" not in mod_rs:
    mod_rs = mod_rs.replace("async fn call_ollama_text(", "pub async fn call_ollama_text(")

# Also we need to intercept user_message inside process_user_prompt.
# Instead of replacing inside process_user_prompt, we will patch it.
process_prompt_signature = "pub async fn process_user_prompt(user_message: String, workspace_path: String, app_handle: tauri::AppHandle) -> Result<String, String> {"
if "let technical_intent =" not in mod_rs:
    new_logic = """pub async fn process_user_prompt(user_message: String, workspace_path: String, app_handle: tauri::AppHandle) -> Result<String, String> {
    let technical_intent = translator::translate_to_technical_intent(&user_message).await;
    println!("[TRADUCTOR] Input: '{}' -> Intención: '{}'", user_message, technical_intent);
    let user_message = technical_intent;
"""
    mod_rs = mod_rs.replace(process_prompt_signature, new_logic)

open(mod_rs_path, "w", encoding="utf-8").write(mod_rs)

# 2. Create translator.rs
translator_rs_content = """use super::call_ollama_text;

pub async fn translate_to_technical_intent(user_input: &str) -> String {
    let model = "qwen2.5:0.5b";
    let system_prompt = format!(
        "Eres un experto en traducción técnica. Tu única función es tomar el mensaje coloquial del usuario (que puede contener errores, jerga o ser vago) y convertirlo en una directiva de ingeniería clara, técnica y orientada a acciones para un Agente DevSecOps. Devuelve ÚNICAMENTE la directiva traducida.\\n\\nMensaje del usuario: {}",
        user_input
    );
    
    match call_ollama_text(model, &system_prompt).await {
        Ok(mut res) => {
            res = res.trim().to_string();
            if res.is_empty() {
                format!("[INTENT_FALLBACK] {}", user_input)
            } else {
                res
            }
        },
        Err(_) => format!("[INTENT_FALLBACK] {}", user_input),
    }
}
"""
with open("src-tauri/src/llm/translator.rs", "w", encoding="utf-8") as f:
    f.write(translator_rs_content)

