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
            let depth = entry.depth();
            if depth == 0 {
                continue; // Saltar la raíz
            }

            let is_dir = entry.file_type().map_or(false, |ft| ft.is_dir());
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
    
    if map_output.lines().count() <= 2 {
        map_output.push_str("  (directorio vacío)\n");
    }

    map_output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_repo_map_finds_files() {
        let temp_dir = std::env::temp_dir().join("aura_test_repo_map");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::write(temp_dir.join("cyber_sentinel.html"), "<html></html>").unwrap();
        std::fs::write(temp_dir.join("style.css"), "body{}").unwrap();

        let map = generate_repo_map(&temp_dir);
        assert!(map.contains("cyber_sentinel.html"));
        assert!(map.contains("style.css"));
        assert!(!map.contains("(directorio vacío)"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
