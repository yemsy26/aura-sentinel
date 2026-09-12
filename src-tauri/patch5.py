import sys

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\memory\mod.rs', 'r', encoding='utf-8') as f:
    lines = f.readlines()

start_idx = -1
end_idx = -1
for i, line in enumerate(lines):
    if 'let path_obj = Path::new(&cambio.archivo);' in line:
        start_idx = i
    if start_idx != -1 and 'let mut contenido_original = String::new();' in line:
        end_idx = i
        break

if start_idx == -1 or end_idx == -1:
    print("NOT FOUND")
    sys.exit(1)

replacement = '''        let full_path = match crate::core::workspace_resolver::WorkspaceResolver::resolve_exact(Path::new(workspace_path), &cambio.archivo) {
            Ok(p) => p,
            Err(_) => {
                patch_not_found_count += 1;
                patch_not_found_files.push(cambio.archivo.clone());
                continue; // Cannot proceed if path resolution fails
            }
        };

        if !crate::core::security::is_path_allowed(Path::new(workspace_path), &full_path) {
            patch_not_found_count += 1;
            patch_not_found_files.push(cambio.archivo.clone());
            continue;
        }

'''

lines = lines[:start_idx] + [replacement] + lines[end_idx:]

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\memory\mod.rs', 'w', encoding='utf-8') as f:
    f.writelines(lines)

print("SUCCESS")
