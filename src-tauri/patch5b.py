import sys

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\memory\mod.rs', 'r', encoding='utf-8') as f:
    lines = f.readlines()

start_idx = -1
end_idx = -1
for i, line in enumerate(lines):
    if 'let path_obj = Path::new(&file_path);' in line:
        start_idx = i
    if start_idx != -1 and 'match fs::metadata(&full_path).await {' in line:
        end_idx = i
        break

if start_idx == -1 or end_idx == -1:
    print("NOT FOUND")
    sys.exit(1)

replacement = '''        let full_path = match crate::core::workspace_resolver::WorkspaceResolver::resolve_exact(Path::new(workspace_path), &file_path) {
            Ok(p) => p,
            Err(_) => {
                combined_content.push_str(&format!(
                    "--- ARCHIVO IGNORADO: {} ([SECURITY_VIOLATION] Ruta inválida) ---\\n\\n",
                    file_path
                ));
                continue;
            }
        };

        if !crate::core::security::is_path_allowed(Path::new(workspace_path), &full_path) {
            combined_content.push_str(&format!(
                "--- ARCHIVO IGNORADO: {} ([SECURITY_VIOLATION] Acceso no permitido fuera del entorno de trabajo) ---\\n\\n",
                file_path
            ));
            continue;
        }

'''

lines = lines[:start_idx] + [replacement] + lines[end_idx:]

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\memory\mod.rs', 'w', encoding='utf-8') as f:
    f.writelines(lines)

print("SUCCESS")
