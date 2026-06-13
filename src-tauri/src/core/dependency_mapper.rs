//! dependency_mapper.rs
//! Aura-Sentinel TOOL_MAPPER — Static dependency analysis engine.
//!
//! Scans a workspace for source files and extracts import/dependency relationships
//! using pattern matching (no LLM needed). Produces:
//!   1. A topologically-sorted build order for the LLM to follow when writing code.
//!   2. A `.aura_graph.json` file persisted on disk for inter-session reuse.
//!   3. A human-readable text report injected into the LLM context.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

// ─── Data Structures ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub file_type: String,    // "python" | "javascript" | "typescript" | "rust" | "go" | "other"
    pub source_file: String,
    pub imports: Vec<String>, // raw import strings from the file
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub relation: String, // "imports" | "depends_on"
    pub confidence: String, // "EXTRACTED" | "INFERRED"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyGraph {
    pub workspace: String,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub build_order: Vec<String>, // topologically sorted file list
    pub god_nodes: Vec<String>,   // files imported by 3+ others
    pub isolated_nodes: Vec<String>, // files with no connections
    pub cycles: Vec<Vec<String>>,    // circular dependency groups
    pub generated_at: String,
}

// ─── Language Detectors ───────────────────────────────────────────────────────

fn detect_language(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "py"           => "python",
        "js" | "mjs"   => "javascript",
        "ts" | "tsx"   => "typescript",
        "jsx"          => "javascript",
        "rs"           => "rust",
        "go"           => "go",
        "c" | "cpp" | "h" | "hpp" => "c_cpp",
        _              => "other",
    }
}

fn is_supported_source_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).unwrap_or(""),
        "py" | "js" | "mjs" | "ts" | "tsx" | "jsx" | "rs" | "go" | "c" | "cpp" | "h" | "hpp"
    )
}

// ─── Import Extractors ────────────────────────────────────────────────────────

/// Extract Python imports. Handles:
///   import os, sys
///   from pathlib import Path
///   from . import utils  (relative)
fn extract_python_imports(content: &str) -> Vec<String> {
    let mut imports = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() { continue; }

        if let Some(rest) = line.strip_prefix("from ") {
            // "from X import Y" → extract X
            if let Some(module) = rest.split_whitespace().next() {
                let module = module.trim_start_matches('.');
                if !module.is_empty() {
                    imports.push(module.to_string());
                }
            }
        } else if let Some(rest) = line.strip_prefix("import ") {
            // "import os, sys" → split by comma
            for part in rest.split(',') {
                let module = part.split_whitespace().next().unwrap_or("").to_string();
                if !module.is_empty() {
                    imports.push(module);
                }
            }
        }
    }
    imports
}

/// Extract JS/TS imports. Handles:
///   import X from './module'
///   const X = require('./module')
///   import('./module')
fn extract_js_imports(content: &str) -> Vec<String> {
    let mut imports = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("//") || line.is_empty() { continue; }

        // ESM: import ... from 'path'
        if line.starts_with("import ") {
            if let Some(from_idx) = line.rfind("from ") {
                let after = &line[from_idx + 5..];
                let path = after.trim().trim_matches(|c| c == '\'' || c == '"' || c == ';');
                if !path.is_empty() {
                    imports.push(path.to_string());
                }
            }
        }

        // CJS: require('path')
        if line.contains("require(") {
            let after_require = line.split("require(").nth(1).unwrap_or("");
            // naive regex-less approach: path is usually surrounded by quotes
            let path = after_require.split(['\'', '"', ')']).next().unwrap_or("").trim();
            if !path.is_empty() {
                imports.push(path.to_string());
            }
        }
    }
    imports
}

/// Extract Rust `use` and `mod` declarations.
fn extract_rust_imports(content: &str) -> Vec<String> {
    let mut imports = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("//") || line.is_empty() { continue; }

        if line.starts_with("use ") || line.starts_with("mod ") || line.starts_with("pub mod ") {
            // Take everything up to { or ; or ::
            let path = line
                .trim_start_matches("pub ")
                .trim_start_matches("use ")
                .trim_start_matches("mod ")
                .split(&['{', ';', ':'][..])
                .next()
                .unwrap_or("")
                .trim();
            if !path.is_empty() {
                imports.push(path.to_string());
            }
        }
    }
    imports
}

/// Extract Go import blocks.
fn extract_go_imports(content: &str) -> Vec<String> {
    let mut imports = Vec::new();
    let mut in_import_block = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("import (") { in_import_block = true; continue; }
        if in_import_block {
            if line == ")" { in_import_block = false; continue; }
            let path = line.trim_matches(|c| c == '"' || c == ' ' || c == '\t');
            if !path.is_empty() { imports.push(path.to_string()); }
        } else if line.starts_with("import \"") {
            let path = line.trim_start_matches("import ").trim_matches('"');
            if !path.is_empty() { imports.push(path.to_string()); }
        }
    }
    imports
}

fn extract_imports(path: &Path, content: &str) -> Vec<String> {
    match detect_language(path) {
        "python"                  => extract_python_imports(content),
        "javascript" | "typescript" => extract_js_imports(content),
        "rust"                    => extract_rust_imports(content),
        "go"                      => extract_go_imports(content),
        _                         => Vec::new(),
    }
}

// ─── File Scanner ─────────────────────────────────────────────────────────────

fn scan_workspace_files(workspace: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let skip_dirs = ["node_modules", ".git", "__pycache__", "target", ".venv", "venv", "dist", "build", ".next"];

    fn recurse(dir: &Path, files: &mut Vec<PathBuf>, skip: &[&str], depth: usize) {
        if depth > 6 { return; }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                if path.is_dir() {
                    if !skip.contains(&name.as_str()) && !name.starts_with('.') {
                        recurse(&path, files, skip, depth + 1);
                    }
                } else if is_supported_source_file(&path) {
                    files.push(path);
                }
            }
        }
    }

    recurse(workspace, &mut files, &skip_dirs, 0);
    files
}

// ─── Graph Builder ────────────────────────────────────────────────────────────

/// Converts a raw import string to the canonical node ID of the local file it refers to.
/// For example: "from utils import X" → looks for utils.py in workspace.
fn resolve_local_import(
    import_str: &str,
    source_file: &Path,
    workspace: &Path,
    all_stems: &HashMap<String, PathBuf>,
) -> Option<String> {
    // Only try to resolve local imports (relative paths or plain names without slashes that match a local file)
    let is_stdlib_or_external = import_str.contains("std::") 
        || import_str.starts_with("std")
        || import_str.starts_with('@')  // npm scoped packages
        || import_str.contains("node_modules");

    if is_stdlib_or_external { return None; }

    // Relative JS/TS import: "./utils" or "../lib/helper"
    if import_str.starts_with('.') {
        let base = source_file.parent().unwrap_or(workspace);
        let target = base.join(import_str);
        // Try with and without extension
        for ext in &["", ".py", ".js", ".ts", ".tsx", ".jsx", ".rs"] {
            let with_ext = if ext.is_empty() {
                target.clone()
            } else {
                target.with_extension(&ext[1..])
            };
            if with_ext.exists() {
                let rel = with_ext.strip_prefix(workspace).unwrap_or(&with_ext);
                return Some(rel.to_string_lossy().replace('\\', "/"));
            }
        }
        // Try index files
        for idx in &["index.js", "index.ts", "index.tsx", "mod.rs"] {
            let idx_path = target.join(idx);
            if idx_path.exists() {
                let rel = idx_path.strip_prefix(workspace).unwrap_or(&idx_path);
                return Some(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }

    // Plain module name (Python, Rust mod, Go pkg)
    // Check if a file with that stem exists in the workspace
    let stem = import_str.split("::").next().unwrap_or(import_str)
        .split('.').next().unwrap_or(import_str)
        .replace('-', "_");

    if let Some(path) = all_stems.get(&stem) {
        let rel = path.strip_prefix(workspace).unwrap_or(path);
        return Some(rel.to_string_lossy().replace('\\', "/"));
    }

    None
}

// ─── Topological Sort (Kahn's algorithm) ─────────────────────────────────────

fn topological_sort(nodes: &[String], edges: &[(String, String)]) -> (Vec<String>, Vec<Vec<String>>) {
    let mut in_degree: HashMap<&str, usize> = nodes.iter().map(|n| (n.as_str(), 0)).collect();
    let mut adj: HashMap<&str, Vec<&str>> = nodes.iter().map(|n| (n.as_str(), vec![])).collect();

    for (src, tgt) in edges {
        if adj.contains_key(src.as_str()) && in_degree.contains_key(tgt.as_str()) {
            adj.get_mut(src.as_str()).unwrap().push(tgt.as_str());
            *in_degree.get_mut(tgt.as_str()).unwrap() += 1;
        }
    }

    let mut queue: VecDeque<&str> = in_degree.iter()
        .filter(|(_, &d)| d == 0)
        .map(|(n, _)| *n)
        .collect();

    let mut order = Vec::new();
    let mut visited_count = 0;

    while let Some(node) = queue.pop_front() {
        order.push(node.to_string());
        visited_count += 1;
        if let Some(neighbors) = adj.get(node) {
            for &neighbor in neighbors {
                if let Some(d) = in_degree.get_mut(neighbor) {
                    *d -= 1;
                    if *d == 0 {
                        queue.push_back(neighbor);
                    }
                }
            }
        }
    }

    // Nodes not visited → part of a cycle
    let mut cycles = Vec::new();
    if visited_count < nodes.len() {
        let in_order: HashSet<&str> = order.iter().map(|s| s.as_str()).collect();
        let cycle_nodes: Vec<String> = nodes.iter()
            .filter(|n| !in_order.contains(n.as_str()))
            .cloned()
            .collect();
        if !cycle_nodes.is_empty() {
            cycles.push(cycle_nodes);
        }
    }

    // Reverse: files with NO dependents (leaf nodes) go first → they should be written first
    order.reverse();
    (order, cycles)
}

// ─── Main Entry Point ─────────────────────────────────────────────────────────

/// Scans the workspace, builds the dependency graph, persists it, and returns a report.
pub fn analyze_workspace(workspace_path: &str) -> DependencyGraph {
    let workspace = Path::new(workspace_path);
    let files = scan_workspace_files(workspace);

    // Build a stem→path lookup for local import resolution
    let mut all_stems: HashMap<String, PathBuf> = HashMap::new();
    for file in &files {
        if let Some(stem) = file.file_stem() {
            all_stems.insert(stem.to_string_lossy().to_string(), file.clone());
        }
    }

    let mut nodes: Vec<GraphNode> = Vec::new();
    let mut raw_edges: Vec<(String, String)> = Vec::new();
    let mut graph_edges: Vec<GraphEdge> = Vec::new();

    // Parse every file
    for file in &files {
        let rel_path = file.strip_prefix(workspace).unwrap_or(file);
        let id = rel_path.to_string_lossy().replace('\\', "/");
        let label = rel_path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let lang = detect_language(file).to_string();

        let content = std::fs::read_to_string(file).unwrap_or_default();
        let imports = extract_imports(file, &content);

        // Resolve which local files are actually imported
        let mut resolved_imports = Vec::new();
        for imp in &imports {
            if let Some(target_id) = resolve_local_import(imp, file, workspace, &all_stems) {
                if target_id != id {
                    resolved_imports.push(imp.clone());
                    raw_edges.push((id.clone(), target_id.clone()));
                    graph_edges.push(GraphEdge {
                        source: id.clone(),
                        target: target_id,
                        relation: "imports".to_string(),
                        confidence: "EXTRACTED".to_string(),
                    });
                }
            }
        }

        nodes.push(GraphNode {
            id: id.clone(),
            label,
            file_type: lang,
            source_file: id,
            imports: resolved_imports,
        });
    }

    // Topological sort
    let node_ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let (build_order, cycles) = topological_sort(&node_ids, &raw_edges);

    // Find god nodes (imported by 3+ files)
    let mut import_count: HashMap<String, usize> = HashMap::new();
    for (_, tgt) in &raw_edges {
        *import_count.entry(tgt.clone()).or_insert(0) += 1;
    }
    let god_nodes: Vec<String> = import_count.iter()
        .filter(|(_, &c)| c >= 3)
        .map(|(n, _)| n.clone())
        .collect();

    // Find isolated nodes
    let connected: HashSet<String> = raw_edges.iter()
        .flat_map(|(s, t)| [s.clone(), t.clone()])
        .collect();
    let isolated_nodes: Vec<String> = node_ids.iter()
        .filter(|n| !connected.contains(*n))
        .cloned()
        .collect();

    let graph = DependencyGraph {
        workspace: workspace_path.to_string(),
        nodes,
        edges: graph_edges,
        build_order,
        god_nodes,
        isolated_nodes,
        cycles,
        generated_at: chrono::Utc::now().to_rfc3339(),
    };

    // Persist to disk
    let graph_path = workspace.join(".aura_graph.json");
    if let Ok(json) = serde_json::to_string_pretty(&graph) {
        let _ = std::fs::write(&graph_path, json);
    }

    graph
}

// ─── Report Generator ─────────────────────────────────────────────────────────

/// Converts a DependencyGraph into a concise text report for the LLM context.
pub fn format_graph_report(graph: &DependencyGraph) -> String {
    let mut report = String::new();

    report.push_str("╔══════════════════════════════════════════════════════════════╗\n");
    report.push_str("║          AURA DEPENDENCY MAP — TOOL_MAPPER REPORT            ║\n");
    report.push_str("╚══════════════════════════════════════════════════════════════╝\n\n");

    report.push_str(&format!("📁 Workspace: {}\n", graph.workspace));
    report.push_str(&format!("📊 Total files: {} nodes, {} dependency edges\n\n",
        graph.nodes.len(), graph.edges.len()));

    // Build order — most important section
    if !graph.build_order.is_empty() {
        report.push_str("🔨 ORDEN DE ESCRITURA RECOMENDADO (escribe primero los que no tienen dependencias):\n");
        for (i, file) in graph.build_order.iter().enumerate() {
            report.push_str(&format!("  {}. {}\n", i + 1, file));
        }
        report.push('\n');
    }

    // God nodes
    if !graph.god_nodes.is_empty() {
        report.push_str("⭐ NODOS CRÍTICOS (importados por 3+ archivos — escríbelos PRIMERO):\n");
        for n in &graph.god_nodes {
            report.push_str(&format!("  • {}\n", n));
        }
        report.push('\n');
    }

    // Dependency edges
    if !graph.edges.is_empty() {
        report.push_str("🔗 GRAFO DE DEPENDENCIAS:\n");
        for edge in &graph.edges {
            report.push_str(&format!("  {} → {} [{}]\n", edge.source, edge.target, edge.relation));
        }
        report.push('\n');
    }

    // Isolated files
    if !graph.isolated_nodes.is_empty() {
        report.push_str("🔵 ARCHIVOS AISLADOS (sin dependencias entre sí — pueden escribirse en cualquier orden):\n");
        for n in &graph.isolated_nodes {
            report.push_str(&format!("  • {}\n", n));
        }
        report.push('\n');
    }

    // Cycles
    if !graph.cycles.is_empty() {
        report.push_str("⚠️  DEPENDENCIAS CIRCULARES DETECTADAS (¡cuidado!):\n");
        for cycle in &graph.cycles {
            report.push_str(&format!("  ↻ {}\n", cycle.join(" → ")));
        }
        report.push('\n');
    }

    report.push_str("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    report.push_str("REGLA: Sigue ESTRICTAMENTE el Orden de Escritura de arriba.\n");
    report.push_str("Escribe cada archivo con TOOL_PROGRAMMER antes de pasar al siguiente.\n");
    report.push_str("Los Nodos Críticos (⭐) deben implementarse COMPLETAMENTE antes\n");
    report.push_str("de empezar cualquier archivo que los importe.\n");

    report
}
