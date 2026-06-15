import re

with open('src-tauri/src/llm/agent.rs', 'r', encoding='utf-8') as f:
    text = f.read()

# Fix TOOL_TERMINAL success
search_1 = """match execute_terminal_command(&workspace_path, &comando).await {
                        Ok(out) => {
                            let res_msg = format!("Éxito: {}", out);
                            current_context.push_str(&format!("Resultado: {}\\n\\n[SISTEMA: El comando tuvo éxito. Tu SIGUIENTE PASO OBLIGATORIO es usar TOOL_TESTER para validar si el entorno ya está correcto. NO VUELVAS A USAR TOOL_TERMINAL BAJO NINGUNA CIRCUNSTANCIA EN EL PRÓXIMO TURNO.]\\n\\n", res_msg));"""

replace_1 = """match execute_terminal_command(&workspace_path, &comando).await {
                        Ok(out) => {
                            comandos_ejecutados_historico.remove(&comando);
                            let res_msg = format!("Éxito: {}", out);
                            current_context.push_str(&format!("Resultado: {}\\n\\n[SISTEMA: El comando en terminal se ejecutó con éxito. Analiza este resultado. Si esto completa el objetivo final del usuario, tu SIGUIENTE PASO OBLIGATORIO es usar 'TOOL_FINISH'. Si aún faltan pasos, continúa. NO uses TOOL_TESTER a menos que el usuario haya pedido pruebas automatizadas.]\\n\\n", res_msg));"""

text = text.replace(search_1, replace_1)

# Fix TOOL_BACKGROUND_START success
search_2 = """match start_background_task(&workspace_path, &task_id, &comando).await {
                        Ok(out) => {
                            current_context.push_str(&format!("Resultado: {}\\n\\n", out));"""

replace_2 = """match start_background_task(&workspace_path, &task_id, &comando).await {
                        Ok(out) => {
                            comandos_ejecutados_historico.remove(&comando);
                            current_context.push_str(&format!("Resultado: {}\\n\\n", out));"""

text = text.replace(search_2, replace_2)

# Remove Capa 3
text = re.sub(
    r'// ── CAPA 3: INTEGRATION VERIFIER ──.*?if integration_ok \|\| py_modules\.is_empty\(\) \{',
    r'if true {',
    text,
    flags=re.DOTALL
)

with open('src-tauri/src/llm/agent.rs', 'w', encoding='utf-8') as f:
    f.write(text)
