import sys

with open('src/llm/agent.rs', 'r', encoding='utf-8', errors='ignore') as f:
    lines = f.readlines()

new_lines = []
in_target = False
found_bg = False
for i, line in enumerate(lines):
    if '"TOOL_BACKGROUND_START" => {' in line:
        found_bg = True
        
    if found_bg and '} else if comandos_ejecutados_historico.contains(&comando) {' in line:
        in_target = True
        new_lines.append(line)
        new_lines.append('                    let res_msg = "[SISTEMA INTERNO]: Advertencia: Este servidor o proceso YA ESTÁ EN EJECUCIÓN en segundo plano. NO necesitas volver a iniciarlo. Usa TOOL_VISION_EVALUATOR o TOOL_FINISH.";\n')
        new_lines.append('                    current_context.push_str(&format!("{}\\n\\n", res_msg));\n')
        new_lines.append('                    emit_event(&app_handle, step_count, "Servidor ya en ejecución (bucle evitado).", "WARNING");\n')
        continue
        
    if in_target:
        if '} else {' in line:
            in_target = False
            new_lines.append(line)
        continue
        
    new_lines.append(line)

with open('src/llm/agent.rs', 'w', encoding='utf-8') as f:
    f.writelines(new_lines)
