import sys

with open('src/llm/agent.rs', 'r', encoding='utf-8') as f:
    lines = f.readlines()

new_lines = []
skip = False
for line in lines:
    if "let res_msg = \"[SISTEMA INTERNO]: Advertencia: Estǭs repitiendo un comando de background fallido." in line:
        new_lines.append('                    let res_msg = "[SISTEMA INTERNO]: Advertencia: Este servidor o proceso YA ESTÁ EN EJECUCIÓN en segundo plano. NO necesitas volver a iniciarlo. Usa TOOL_VISION_EVALUATOR o TOOL_FINISH.";\n')
        new_lines.append('                    current_context.push_str(&format!("{}\\n\\n", res_msg));\n')
        new_lines.append('                    emit_event(&app_handle, step_count, "Servidor ya en ejecución (bucle evitado).", "WARNING");\n')
        skip = True
        continue
    
    if skip:
        if "return Ok(serde_json::to_string(&final_res).unwrap());" in line:
            skip = False
        continue
    
    new_lines.append(line)

with open('src/llm/agent.rs', 'w', encoding='utf-8') as f:
    f.writelines(new_lines)
