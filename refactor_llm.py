import os

agent_path = 'src-tauri/src/llm/agent.rs'
agent_content = open(agent_path, 'r', encoding='utf-8').read()

# Add constants to agent.rs
if "const ORCHESTRATOR_MODEL:" not in agent_content:
    agent_content = agent_content.replace(
        "pub async fn run_agent_loop(",
        "pub const ORCHESTRATOR_MODEL: &str = \"llama3.1:8b\";\npub const PROGRAMMER_MODEL: &str = \"qwen2.5-coder:7b\";\n\npub async fn run_agent_loop("
    )
    agent_content = agent_content.replace('let orchestrator_model = "llama3.1:8b";\n', '')
    agent_content = agent_content.replace('let programmer_model = "qwen2.5-coder:7b";\n', '')
    agent_content = agent_content.replace('orchestrator_model', 'ORCHESTRATOR_MODEL')
    agent_content = agent_content.replace('programmer_model', 'PROGRAMMER_MODEL')

open(agent_path, 'w', encoding='utf-8').write(agent_content)

mod_path = 'src-tauri/src/llm/mod.rs'
mod_content = open(mod_path, 'r', encoding='utf-8').read()

# Clean up llm/mod.rs unused structs and code
struct1 = """#[derive(Serialize)]
struct OrchestratorOutput {
    intencion: String,
    pensamiento: String,
    comando_a_ejecutar: Option<String>,
    url_a_investigar: Option<String>,
    archivos_a_analizar: Vec<String>,
    archivos_a_modificar: Vec<String>,
    modelo_sugerido: String,
    respuesta_conversacional: String,
}"""

struct2 = """#[derive(Serialize)]
struct PipelineResponse {
    orquestador: OrchestratorOutput,
    programador: serde_json::Value,
    operacion_fisica: String,
    eventos_validacion: Vec<String>,
}"""

if struct1 in mod_content:
    mod_content = mod_content.replace(struct1, "")
if struct2 in mod_content:
    mod_content = mod_content.replace(struct2, "")

# Remove unused imports
mod_content = mod_content.replace("use tokio::process::Command;\n", "")
mod_content = mod_content.replace("use crate::{memory::{self, Cambio}, core::validate_workspace};\n", "use crate::memory::{self, Cambio};\n")
mod_content = mod_content.replace("use tauri::{AppHandle, Emitter};\n", "use tauri::AppHandle;\n")

open(mod_path, 'w', encoding='utf-8').write(mod_content)
