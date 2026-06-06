import os

agent_path = "src-tauri/src/llm/agent.rs"
content = open(agent_path, "r", encoding="utf-8").read()

# 1. Add `architect_used` flag to run_agent_loop
if "let mut architect_used = false;" not in content:
    content = content.replace(
        "let mut step_count = 1;",
        "let mut architect_used = false;\n    let mut step_count = 1;"
    )

# 2. Update System Prompt for TOOL_ARCHITECT
old_architect_prompt = "9. 'TOOL_ARCHITECT': Analiza la estructura y dependencias de sistemas grandes. Úsala para generar mapas de arquitectura, detectar riesgos de modularidad o realizar refactorizaciones de alto nivel. No rellena ningún argumento adicional."
new_architect_prompt = "9. 'TOOL_ARCHITECT': Analiza la estructura y dependencias. No rellena argumentos. REGLA: Después de usarla, DEBES usar TOOL_FINISH obligatoriamente para resumirle los hallazgos al usuario."
content = content.replace(old_architect_prompt, new_architect_prompt)

# 3. Add Cooldown inside the TOOL_ARCHITECT match arm
# We need to find the "TOOL_ARCHITECT" => { block
if "architect_used = true;" not in content:
    # Let's find the TOOL_ARCHITECT arm
    architect_arm = """"TOOL_ARCHITECT" => {
                    let result = crate::core::architect::generate_dependency_map(&workspace_path);"""
    
    architect_arm_new = """"TOOL_ARCHITECT" => {
                    if architect_used {
                        emit_event(&app_handle, step_count, "Bucle interceptado por Cooldown (Architect)", "WARNING");
                        current_context.push_str(&format!("PASO {}:\\nAcción: TOOL_ARCHITECT\\nResultado: [SISTEMA INTERCEPTO] Error: Ya ejecutaste TOOL_ARCHITECT. Ahora DEBES usar TOOL_FINISH para resumir el mapa al usuario.\\n\\n", step_count));
                        step_count += 1;
                        continue;
                    }
                    architect_used = true;
                    let result = crate::core::architect::generate_dependency_map(&workspace_path);"""
    
    content = content.replace(architect_arm, architect_arm_new)

open(agent_path, "w", encoding="utf-8").write(content)
