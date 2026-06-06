import os

# 1. Modify core/mod.rs
core_path = 'src-tauri/src/core/mod.rs'
core_content = open(core_path, 'r', encoding='utf-8').read()

format_error_fn = """
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
"""
if "pub async fn format_system_error" not in core_content:
    core_content += format_error_fn

open(core_path, 'w', encoding='utf-8').write(core_content)


# 2. Modify llm/agent.rs
agent_path = 'src-tauri/src/llm/agent.rs'
agent_content = open(agent_path, 'r', encoding='utf-8').read()

# Import format_system_error
agent_content = agent_content.replace(
    'use crate::core::{execute_terminal_command, start_background_task, read_task_logs, kill_task, validate_workspace};',
    'use crate::core::{execute_terminal_command, start_background_task, read_task_logs, kill_task, validate_workspace, format_system_error};'
)

# Modify Prompt
agent_content = agent_content.replace(
    "- Resolución de Errores: Si un comando falla, lee los logs o la consola, usa 'TOOL_PROGRAMMER' para arreglar el código, y vuelve a intentar.\\n\\n\\",
    "- Resolución de Errores: Si un comando falla, lee los logs o la consola, usa 'TOOL_PROGRAMMER' para arreglar el código, y vuelve a intentar.\\n\\            - Estrictud JSON (Auto-Debugger): Si recibes una alerta [AUTO-DEBUGGER], tu ÚNICA tarea es corregir la sintaxis o el ID que falló. No intentes ejecutar código nuevo hasta que la herramienta devuelva [SUCCESS].\\n\\n\\"
)

# Modify TOOL_BACKGROUND_READ
agent_content = agent_content.replace(
    """                        current_context.push_str(&format!("Error al leer logs: {}\\n\\n", err));
                        emit_event(&app_handle, step_count, &err, "ERROR");""",
    """                        let fmt_err = format_system_error(&err).await;
                        current_context.push_str(&format!("[AUTO-DEBUGGER] Error al leer logs: {}\\n\\n", fmt_err));
                        emit_event(&app_handle, step_count, &fmt_err, "ERROR");"""
)

# Modify TOOL_BACKGROUND_KILL
agent_content = agent_content.replace(
    """                        current_context.push_str(&format!("Error matando tarea: {}\\n\\n", err));
                        emit_event(&app_handle, step_count, &err, "ERROR");""",
    """                        let fmt_err = format_system_error(&err).await;
                        current_context.push_str(&format!("[AUTO-DEBUGGER] Error matando tarea: {}\\n\\n", fmt_err));
                        emit_event(&app_handle, step_count, &fmt_err, "ERROR");"""
)

open(agent_path, 'w', encoding='utf-8').write(agent_content)
