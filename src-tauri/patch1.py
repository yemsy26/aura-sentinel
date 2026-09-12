import sys

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'r', encoding='utf-8') as f:
    text = f.read()

start_idx = text.find(' LIVE WORKSPACE SCAN (Always 100% fresh on every step) ')
if start_idx == -1:
    print('NOT FOUND START')
    sys.exit(1)
start_idx = text.rfind('        // ', 0, start_idx) # backtrack to start of line

end_idx = text.find('\n        };', start_idx) + 11
original_block = text[start_idx:end_idx]

replacement = '''        // 🛡️ LIVE WORKSPACE SCAN (Delegated to WorldState) 🛡️
        // Fix P0-2: Remove local recursive scan, use the single source of truth from runtime
        let (live_workspace_context, workspace_is_empty) = {
            let is_empty = runtime.world.as_ref().map_or(true, |w| w.files.is_empty());
            let ctx = if is_empty {
                "El proyecto está completamente vacío. Aún no has creado ningún archivo físico.".to_string()
            } else {
                let repo_map = crate::core::map::generate_repo_map(std::path::Path::new(&workspace_path));
                let mut relative_files = Vec::new();
                if let Some(world) = runtime.world.as_ref() {
                    for path in world.files.keys() {
                        relative_files.push(path.clone());
                    }
                }
                relative_files.sort();
                format!("{}\\n\\nARCHIVOS EXISTENTES EN EL WORKSPACE (rutas relativas):\\n{}", repo_map, relative_files.join("\\n"))
            };
            (ctx, is_empty)
        };\n'''

text = text[:start_idx] + replacement + text[end_idx:]

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'w', encoding='utf-8') as f:
    f.write(text)

print('SUCCESS')
