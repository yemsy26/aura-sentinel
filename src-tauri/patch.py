
import re

with open("src/llm/agent.rs", "r", encoding="utf-8") as f:
    content = f.read()

old_block = """                    emit_event(&app_handle, step_count,
                        &format!("[FINISH BLOQUEADO] Faltan herramientas obligatorias: {:?}", missing_list),
                        "WARNING");
                    step_count += 1;
                    continue;"""

new_block = """                    emit_event(&app_handle, step_count,
                        &format!("[FINISH BLOQUEADO] Faltan herramientas obligatorias: {:?}", missing_list),
                        "WARNING");
                        
                    // Fix FSM: Force the missing tool in the next iteration to prevent infinite loops
                    forced_next_tool = Some((missing_mandatory[0].to_string(), "Ejecucion forzada de herramienta faltante por el mandato del usuario".to_string()));
                    
                    step_count += 1;
                    continue;"""

content = content.replace(old_block, new_block)

with open("src/llm/agent.rs", "w", encoding="utf-8") as f:
    f.write(content)

