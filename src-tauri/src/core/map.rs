use ignore::WalkBuilder;
use std::path::Path;


/// Genera un mapa del repositorio (Repository Map) excluyendo carpetas pesadas
/// y ocultas para inyectar en el contexto del LLM.
pub fn generate_repo_map(workspace: &Path) -> String {
    let mut builder = WalkBuilder::new(workspace);
    builder.max_depth(Some(5))
           .hidden(true)
           .git_ignore(true)
           .ignore(true);
           
    // Filtro adicional manual de seguridad para optimizar tokens
    builder.filter_entry(|entry| {
        let name = entry.file_name().to_string_lossy();
        if name == "node_modules" || name == "target" || name == "__pycache__" || name == ".git" {
            return false;
        }
        true
    });

    let mut map_output = String::from("REPOSITORY MAP:\n/\n");
    

    for result in builder.build() {
        if let Ok(entry) = result {
            let path = entry.path();
            if path == workspace {
                continue; // Saltar la raíz
            }
            
            // Calcular ruta relativa
            if let Ok(rel_path) = path.strip_prefix(workspace) {
                let depth = rel_path.components().count();
                if depth == 0 { continue; }
                
                let is_dir = path.is_dir();
                let name = entry.file_name().to_string_lossy();
                
                let mut prefix = String::new();
                for _ in 1..depth {
                    prefix.push_str("│   ");
                }
                
                if is_dir {
                    map_output.push_str(&format!("{}├── {}/\n", prefix, name));
                } else {
                    map_output.push_str(&format!("{}├── {}\n", prefix, name));
                }
            }
        }
    }
    
    if map_output.lines().count() <= 2 {
        map_output.push_str("  (directorio vacío)\n");
    }

    map_output
}
