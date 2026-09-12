import sys

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'r', encoding='utf-8') as f:
    text = f.read()

start_idx = text.find('if let Err(auth_err) = runtime.authorize_action(&intercept_proposal) {')
if start_idx == -1:
    print('NOT FOUND START')
    sys.exit(1)

# The block ends before `} else { \n // Forzando abandono`
end_idx = text.find('                    } else {\n                        current_context.push_str(&format!(\n                            "[INTERCEPTOR] Forzando abandono', start_idx)

if end_idx == -1:
    print('NOT FOUND END')
    sys.exit(1)

replacement = '''match runtime.execute_action(&intercept_proposal).await {
                            Ok(obs) => {
                                let out_len = obs.payload.len();
                                let digest = if out_len > 3000 { &obs.payload[..3000] } else { &obs.payload[..] };
                                let auto_msg = format!(
                                    "[INTERCEPTOR AUTO-EXEC] Ejecutó '{}' bajo autorización de runtime.\\nResultado:\\n{}\\n\\n",
                                    forced_cmd_to_run, digest
                                );
                                current_context.push_str(&auto_msg);
                                emit_event(&app_handle, runtime.current_step(), &format!("[INTERCEPTOR EXECUTED] {}", forced_cmd_to_run), "SUCCESS");
                            }
                            Err(e) => {
                                current_context.push_str(&format!(
                                    "[INTERCEPTOR AUTO-EXEC BLOQUEADO/FALLIDO]: {}\\n\\n",
                                    e
                                ));
                                emit_event(&app_handle, runtime.current_step(), &format!("[INTERCEPTOR ERROR] {}", e), "ERROR");
                            }
                        }\n'''

text = text[:start_idx] + replacement + text[end_idx:]

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'w', encoding='utf-8') as f:
    f.write(text)

print('SUCCESS')
