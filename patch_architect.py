import os

# 1. Update core/mod.rs
mod_rs_path = "src-tauri/src/core/mod.rs"
mod_rs = open(mod_rs_path, "r", encoding="utf-8").read()
if "pub mod architect;" not in mod_rs:
    mod_rs = "pub mod architect;\n" + mod_rs
    open(mod_rs_path, "w", encoding="utf-8").write(mod_rs)

# 2. Update llm/agent.rs
agent_rs_path = "src-tauri/src/llm/agent.rs"
agent_rs = open(agent_rs_path, "r", encoding="utf-8").read()

# Prompt update
prompt_old = "8. 'TOOL_FINISH': Cuando el objetivo principal se haya completado con éxito, o si es imposible continuar. Rellena 'respuesta_conversacional' con la respuesta final para el usuario. ¡USALA SIEMPRE QUE HAYAS TERMINADO!\\n\\n\\"
prompt_new = "8. 'TOOL_FINISH': Cuando el objetivo principal se haya completado con éxito, o si es imposible continuar. Rellena 'respuesta_conversacional' con la respuesta final para el usuario. ¡USALA SIEMPRE QUE HAYAS TERMINADO!\\n            9. 'TOOL_ARCHITECT': Analiza la estructura y dependencias de sistemas grandes. Úsala para generar mapas de arquitectura, detectar riesgos de modularidad o realizar refactorizaciones de alto nivel. No rellena ningún argumento adicional.\\n\\n\\"

if "9. 'TOOL_ARCHITECT'" not in agent_rs:
    agent_rs = agent_rs.replace(prompt_old, prompt_new)

# Auto-debugger instructions for confidence
auto_debugger_old = "- Estrictud JSON (Auto-Debugger): Si recibes una alerta [AUTO-DEBUGGER], tu ÚNICA tarea es corregir la sintaxis o el ID que falló. No intentes ejecutar código nuevo hasta que la herramienta devuelva [SUCCESS].\\n\\n\\"
auto_debugger_new = "- Estrictud JSON (Auto-Debugger): Si recibes una alerta [AUTO-DEBUGGER], tu ÚNICA tarea es corregir la sintaxis o el ID que falló. No intentes ejecutar código nuevo hasta que la herramienta devuelva [SUCCESS].\\n            - Modo Arquitecto: Si al usar TOOL_ARCHITECT el campo de confianza es BAJA, no tomes decisiones de refactorización automáticas. Reporta los hallazgos al usuario y solicita confirmación manual.\\n\\n\\"

if "- Modo Arquitecto:" not in agent_rs:
    agent_rs = agent_rs.replace(auto_debugger_old, auto_debugger_new)

# Add the TOOL_ARCHITECT block in the match statement
if '"TOOL_ARCHITECT" => {' not in agent_rs:
    tool_architect_code = """
            "TOOL_ARCHITECT" => {
                emit_event(&app_handle, step_count, "Generando mapa arquitectónico del sistema...", "ACTION");
                match crate::core::architect::generate_dependency_map(&workspace_path) {
                    Ok(report) => {
                        current_context.push_str(&format!("Reporte Arquitectónico:\\n{}\\n\\n", report));
                        emit_event(&app_handle, step_count, "Mapa arquitectónico generado.", "SUCCESS");
                    },
                    Err(e) => {
                        current_context.push_str(&format!("Error en Arquitecto: {}\\n\\n", e));
                        emit_event(&app_handle, step_count, &e, "ERROR");
                    }
                }
            },
"""
    # Insert it right before "TOOL_FINISH" => {
    agent_rs = agent_rs.replace('            "TOOL_FINISH" => {', tool_architect_code + '            "TOOL_FINISH" => {')

open(agent_rs_path, "w", encoding="utf-8").write(agent_rs)

