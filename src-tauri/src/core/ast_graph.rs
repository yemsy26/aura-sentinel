use ignore::WalkBuilder;
use std::path::Path;
use std::fs;
use std::collections::HashMap;

/// Generates a lightweight Dependency Graph (Knowledge Graph) for LLM context.
/// It scans source files and extracts import/use statements to build a topology.
pub fn generate_dependency_graph(workspace: &Path) -> String {
    let mut builder = WalkBuilder::new(workspace);
    builder.max_depth(Some(5))
           .hidden(true)
           .git_ignore(true)
           .ignore(true);
           
    builder.filter_entry(|entry| {
        let name = entry.file_name().to_string_lossy();
        if name == "node_modules" || name == "target" || name == "__pycache__" || name == ".git" {
            return false;
        }
        true
    });

    let mut graph: HashMap<String, Vec<String>> = HashMap::new();

    for result in builder.build() {
        if let Ok(entry) = result {
            let path = entry.path();
            if path.is_dir() { continue; }

            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy();
                if ext_str == "rs" || ext_str == "py" || ext_str == "js" || ext_str == "ts" {
                    if let Ok(content) = fs::read_to_string(path) {
                        let deps = extract_dependencies(&content, &ext_str);
                        if !deps.is_empty() {
                            if let Ok(rel_path) = path.strip_prefix(workspace) {
                                let key = rel_path.to_string_lossy().to_string().replace("\\", "/");
                                graph.insert(key, deps);
                            }
                        }
                    }
                }
            }
        }
    }

    if graph.is_empty() {
        return String::from("KNOWLEDGE GRAPH: (No internal dependencies detected)\n");
    }

    let mut output = String::from("KNOWLEDGE GRAPH (Topological Dependencies):\n");
    let mut keys: Vec<&String> = graph.keys().collect();
    keys.sort(); // Sort for deterministic output

    for k in keys {
        let deps = &graph[k];
        // Only show up to 5 dependencies per file to avoid context bloat
        let display_deps: Vec<String> = deps.iter().take(5).cloned().collect();
        let suffix = if deps.len() > 5 { ", ...]" } else { "]" };
        
        output.push_str(&format!("  {} -> [{}{}\n", k, display_deps.join(", "), suffix));
    }

    output
}

fn extract_dependencies(content: &str, ext: &str) -> Vec<String> {
    let mut deps = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        match ext {
            "rs" => {
                if trimmed.starts_with("use crate::") || trimmed.starts_with("use super::") {
                    let mut parts = trimmed.split("::");
                    parts.next(); // skip 'use'
                    if let Some(module) = parts.next() {
                        let clean = module.trim_end_matches(';').trim_end_matches(":{").trim();
                        if !clean.is_empty() && !deps.contains(&clean.to_string()) {
                            deps.push(clean.to_string());
                        }
                    }
                }
            },
            "py" => {
                if trimmed.starts_with("import ") || trimmed.starts_with("from ") {
                    let parts: Vec<&str> = trimmed.split_whitespace().collect();
                    if parts.len() >= 2 {
                        let clean = parts[1].trim_end_matches(';');
                        if !deps.contains(&clean.to_string()) && !clean.starts_with('.') {
                            deps.push(clean.to_string());
                        }
                    }
                }
            },
            "js" | "ts" => {
                if trimmed.starts_with("import ") && trimmed.contains(" from ") {
                    if let Some(idx) = trimmed.find(" from ") {
                        let module = trimmed[idx + 6..].trim().trim_matches('\'').trim_matches('"').trim_matches(';');
                        if !deps.contains(&module.to_string()) {
                            deps.push(module.to_string());
                        }
                    }
                } else if trimmed.contains("require(") {
                    if let Some(start) = trimmed.find("require(") {
                        if let Some(end) = trimmed[start..].find(')') {
                            let module = trimmed[start + 8..start + end].trim().trim_matches('\'').trim_matches('"');
                            if !deps.contains(&module.to_string()) {
                                deps.push(module.to_string());
                            }
                        }
                    }
                }
            },
            _ => {}
        }
    }
    deps
}
