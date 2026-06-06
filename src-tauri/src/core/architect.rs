use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use regex::Regex;
use serde::Serialize;

#[derive(Serialize)]
pub struct ArchitectReport {
    pub total_modules: usize,
    pub dependencies: HashMap<String, Vec<String>>,
    pub orphans: Vec<String>,
    pub cycles: Vec<Vec<String>>,
    pub confianza: String,
    pub motivo: String,
}

pub fn generate_dependency_map(workspace_path: &str) -> Result<String, String> {
    let mut deps: HashMap<String, Vec<String>> = HashMap::new();
    let mut dynamic_imports = 0;
    
    // Configuración Regex
    let js_import_re = Regex::new(r#"import\s+.*?from\s+['"](.+?)['"]"#).unwrap();
    let js_require_re = Regex::new(r#"require\(['"](.+?)['"]\)"#).unwrap();
    let js_dynamic_re = Regex::new(r#"import\([^'"]"#).unwrap();
    
    let py_import_re = Regex::new(r"^\s*import\s+([a-zA-Z0-9_\.]+)").unwrap();
    let py_from_re = Regex::new(r"^\s*from\s+([a-zA-Z0-9_\.]+)\s+import").unwrap();
    let py_dynamic_re = Regex::new(r"importlib\.import_module|__import__\(").unwrap();
    
    let rs_use_re = Regex::new(r"use\s+([a-zA-Z0-9_:]+)").unwrap();
    let rs_mod_re = Regex::new(r"mod\s+([a-zA-Z0-9_]+)").unwrap();
    
    let root = Path::new(workspace_path);
    let mut to_visit = vec![root.to_path_buf()];
    let mut all_files = Vec::new();
    
    // File walker
    while let Some(dir) = to_visit.pop() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                
                if !crate::core::security::is_path_allowed(root, &path) {
                    continue;
                }
                
                if path.is_dir() {
                    if !file_name.starts_with('.') && file_name != "node_modules" && file_name != "target" && file_name != "__pycache__" && file_name != "venv" {
                        to_visit.push(path);
                    }
                } else if path.is_file() {
                    if let Some(ext) = path.extension() {
                        let ext_str = ext.to_string_lossy();
                        if ext_str == "js" || ext_str == "ts" || ext_str == "py" || ext_str == "rs" {
                            all_files.push(path);
                        }
                    }
                }
            }
        }
    }
    
    for file_path in &all_files {
        let rel_path = file_path.strip_prefix(root).unwrap_or(file_path).to_string_lossy().to_string();
        let rel_path = rel_path.replace("\\", "/");
        let content = fs::read_to_string(file_path).unwrap_or_default();
        let ext = file_path.extension().unwrap_or_default().to_string_lossy();
        
        let mut file_deps = Vec::new();
        
        if ext == "js" || ext == "ts" {
            for cap in js_import_re.captures_iter(&content) { file_deps.push(cap[1].to_string()); }
            for cap in js_require_re.captures_iter(&content) { file_deps.push(cap[1].to_string()); }
            if js_dynamic_re.is_match(&content) { dynamic_imports += 1; }
        } else if ext == "py" {
            for cap in py_import_re.captures_iter(&content) { file_deps.push(cap[1].to_string()); }
            for cap in py_from_re.captures_iter(&content) { file_deps.push(cap[1].to_string()); }
            if py_dynamic_re.is_match(&content) { dynamic_imports += 1; }
        } else if ext == "rs" {
            for cap in rs_use_re.captures_iter(&content) { file_deps.push(cap[1].to_string()); }
            for cap in rs_mod_re.captures_iter(&content) { file_deps.push(cap[1].to_string()); }
        }
        
        deps.insert(rel_path, file_deps);
    }
    
    // Detect orphans
    let mut in_degree: HashMap<String, usize> = deps.keys().map(|k| (k.clone(), 0)).collect();
    for targets in deps.values() {
        for target in targets {
            for (node, count) in in_degree.iter_mut() {
                if node.contains(target) || target.contains(node) {
                    *count += 1;
                }
            }
        }
    }
    
    let entry_points = ["main.rs", "lib.rs", "index.js", "main.js", "app.js", "main.py", "app.py", "backend.py"];
    let mut orphans = Vec::new();
    for (node, count) in &in_degree {
        if *count == 0 {
            let is_entry = entry_points.iter().any(|&ep| node.ends_with(ep));
            if !is_entry {
                orphans.push(node.clone());
            }
        }
    }
    
    // Cycle detection
    let mut cycles = Vec::new();
    let mut visited = HashSet::new();
    let mut rec_stack = Vec::new();
    
    for node in deps.keys() {
        if !visited.contains(node) {
            dfs_cycle(node, &deps, &mut visited, &mut rec_stack, &mut cycles);
        }
    }
    
    let confianza = if dynamic_imports > 0 || deps.is_empty() { "BAJA".to_string() } else { "ALTA".to_string() };
    let motivo = if confianza == "BAJA" { "Dependencias dinámicas detectadas o nulas".to_string() } else { "Dependencias estáticas claras".to_string() };
    
    let report = ArchitectReport {
        total_modules: deps.len(),
        dependencies: deps,
        orphans,
        cycles,
        confianza,
        motivo,
    };
    
    serde_json::to_string_pretty(&report).map_err(|e| e.to_string())
}

fn dfs_cycle(
    node: &String,
    deps: &HashMap<String, Vec<String>>,
    visited: &mut HashSet<String>,
    rec_stack: &mut Vec<String>,
    cycles: &mut Vec<Vec<String>>
) {
    visited.insert(node.clone());
    rec_stack.push(node.clone());
    
    if let Some(targets) = deps.get(node) {
        for target in targets {
            let mut resolved_target = None;
            for k in deps.keys() {
                if k.contains(target) || target.contains(k) {
                    resolved_target = Some(k.clone());
                    break;
                }
            }
            
            if let Some(res_target) = resolved_target {
                if !visited.contains(&res_target) {
                    dfs_cycle(&res_target, deps, visited, rec_stack, cycles);
                } else if rec_stack.contains(&res_target) {
                    let start_idx = rec_stack.iter().position(|r| r == &res_target).unwrap_or(0);
                    let mut cycle_path = rec_stack[start_idx..].to_vec();
                    cycle_path.push(res_target.clone());
                    cycles.push(cycle_path);
                }
            }
        }
    }
    rec_stack.pop();
}
