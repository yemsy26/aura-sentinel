use serde::{Deserialize, Serialize};
use std::path::Path;
use std::io::{BufRead, Write};
use chrono::Utc;

/// A single episode = one completed (or failed) agent mission.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Episode {
    pub id: String,
    pub timestamp: String,
    pub workspace: String,
    pub objective: String,
    pub outcome: String,             // "COMPLETADO" | "FALLIDO" | "INTERRUMPIDO"
    pub tools_used: Vec<String>,
    pub files_touched: Vec<String>,
    pub summary: String,             // Short LLM-compressed summary (max 300 chars)
    pub tags: Vec<String>,           // Auto-extracted semantic tags
}

const EPISODES_FILE: &str = ".aura_episodes.jsonl";

/// Returns path to the global episodes store
fn episodes_path() -> std::path::PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(EPISODES_FILE)
}

/// Appends a new episode to the persistent JSONL store
pub fn save_episode(
    workspace: &str,
    objective: &str,
    outcome: &str,
    tools_used: &[String],
    files_touched: &[String],
) {
    let tags = auto_extract_tags(objective, tools_used);
    let summary = build_summary(objective, outcome, tools_used.len(), files_touched.len());

    let episode = Episode {
        id: format!("{:x}", uuid_lite()),
        timestamp: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        workspace: workspace.to_string(),
        objective: objective.chars().take(200).collect(),
        outcome: outcome.to_string(),
        tools_used: tools_used.to_vec(),
        files_touched: files_touched.iter()
            .map(|f| Path::new(f).file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| f.clone()))
            .collect(),
        summary,
        tags,
    };

    if let Ok(line) = serde_json::to_string(&episode) {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true).append(true).open(episodes_path())
        {
            let _ = writeln!(file, "{}", line);
        }
    }
}

/// Returns the last N episodes as a formatted context block for the Planner
pub fn get_episode_context(n: usize) -> String {
    let episodes = load_recent_episodes(n);
    if episodes.is_empty() {
        return String::new();
    }

    let mut block = String::from("[MEMORIA HISTÓRICA DE SESIONES ANTERIORES]\n");
    for ep in episodes {
        block.push_str(&format!(
            "• [{}] {} | Estado: {} | Herramientas: {} | Archivos: {}\n  Resumen: {}\n",
            ep.timestamp,
            ep.objective,
            ep.outcome,
            ep.tools_used.join(", "),
            ep.files_touched.join(", "),
            ep.summary,
        ));
    }
    block.push_str("[FIN MEMORIA HISTÓRICA]\n\n");
    block
}

/// Loads the last N episodes from the JSONL file (reads from end)
pub fn load_recent_episodes(n: usize) -> Vec<Episode> {
    let path = episodes_path();
    let Ok(file) = std::fs::File::open(&path) else { return Vec::new() };
    let reader = std::io::BufReader::new(file);
    let mut episodes: Vec<Episode> = reader.lines()
        .filter_map(|line| line.ok())
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect();
    // Return last N (most recent)
    let skip = episodes.len().saturating_sub(n);
    episodes.split_off(skip)
}

/// Searches episodes by keyword in objective or tags
pub fn search_episodes(query: &str) -> Vec<Episode> {
    let q = query.to_lowercase();
    load_recent_episodes(100).into_iter()
        .filter(|ep| {
            ep.objective.to_lowercase().contains(&q)
                || ep.tags.iter().any(|t| t.contains(&q))
                || ep.summary.to_lowercase().contains(&q)
        })
        .collect()
}

// ─── Helpers ───────────────────────────────────────────────────────────────

fn auto_extract_tags(objective: &str, tools: &[String]) -> Vec<String> {
    let mut tags = Vec::new();
    let lower = objective.to_lowercase();

    if lower.contains("sat") || lower.contains("cláusula") || lower.contains("booleano") || lower.contains("lógica") {
        tags.push("matematica".to_string());
    }
    if lower.contains("docker") || lower.contains("contenedor") || lower.contains("container") {
        tags.push("contenedor".to_string());
    }
    if lower.contains("contrato") || lower.contains("auditor") {
        tags.push("contrato".to_string());
    }
    if lower.contains("rust") || lower.contains("cargo") {
        tags.push("rust".to_string());
    }
    if lower.contains("seguridad") || lower.contains("encriptación") || lower.contains("cifrado") {
        tags.push("seguridad".to_string());
    }
    for tool in tools {
        tags.push(tool.to_lowercase().replace("tool_", ""));
    }
    tags.sort();
    tags.dedup();
    tags
}

fn build_summary(objective: &str, outcome: &str, tools: usize, files: usize) -> String {
    let obj = objective.chars().take(150).collect::<String>();
    format!("{} → {} ({} herramientas, {} archivos)", obj, outcome, tools, files)
}

fn uuid_lite() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}
