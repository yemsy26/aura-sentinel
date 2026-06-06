import os

path = 'src-tauri/src/llm/agent.rs'
content = open(path, 'r', encoding='utf-8').read()

# 1. Add HashSet to run_agent_loop
if "let mut archivos_editados_historico" not in content:
    content = content.replace(
        "let mut current_context = String::new();",
        "let mut current_context = String::new();\n    let mut archivos_editados_historico: std::collections::HashSet<String> = std::collections::HashSet::new();"
    )

# 2. Intercept in TOOL_PROGRAMMER
tool_prog_start = '            "TOOL_PROGRAMMER" => {\n'
interception_code = """                let mut ya_editados = true;
                if archivos_vec.is_empty() { ya_editados = false; }
                for f in &archivos_vec {
                    if !archivos_editados_historico.contains(f) {
                        ya_editados = false;
                        break;
                    }
                }
                
                if ya_editados {
                    let interception = "[SISTEMA INTERCEPTO] Error Lógico: Ya editaste estos archivos en un turno anterior con éxito. ASUME QUE EL CÓDIGO FUE ESCRITO CORRECTAMENTE. No repitas esta acción. Actualiza tu checklist mental y avanza al siguiente paso o usa TOOL_FINISH.";
                    current_context.push_str(&format!("{}\\n\\n", interception));
                    emit_event(&app_handle, step_count, "Bucle interceptado por Cooldown", "WARNING");
                } else {
"""

if "let mut ya_editados = true;" not in content:
    content = content.replace(
        '            "TOOL_PROGRAMMER" => {\n                emit_event(&app_handle, step_count, "Delegando a Qwen para modificar código físico...", "ACTION");',
        tool_prog_start + interception_code + '                emit_event(&app_handle, step_count, "Delegando a Qwen para modificar código físico...", "ACTION");'
    )
    
    # 3. Add to HashSet on success
    content = content.replace(
        'exito_bucle_programador = true;\n                                                },',
        '''exito_bucle_programador = true;
                                                    for f in &archivos_vec {
                                                        archivos_editados_historico.insert(f.clone());
                                                    }
                                                },'''
    )
    
    # Close the else block at the end of TOOL_PROGRAMMER
    content = content.replace(
        '                            }\n                        },\n                        Err(e) => {',
        '                            }\n                        },\n                        Err(e) => {'
    )
    # The TOOL_PROGRAMMER block ends right before "TOOL_FINISH" =>
    # Let's find exactly where to insert the closing brace.
    # It's better to just split by `"TOOL_FINISH" =>`
    parts = content.split('            "TOOL_FINISH" => {\n')
    parts[0] = parts[0] + "                }\n"
    content = '            "TOOL_FINISH" => {\n'.join(parts)

open(path, 'w', encoding='utf-8').write(content)
