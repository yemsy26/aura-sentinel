use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskFingerprint {
    pub language: String,
    pub frameworks: Vec<String>,
    pub file_count: usize,
    pub tools_required: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceRecord {
    pub id: String,
    pub timestamp: String,
    pub workspace: String,
    pub objective: String,
    pub fingerprint: TaskFingerprint,
    pub steps_taken: u32,
    pub outcome: ExperienceOutcome,
    pub strategy_summary: String,
    pub key_lessons: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExperienceOutcome {
    Success,
    PartialSuccess,
    Failed(String),
}

const EXPERIENCES_FILE: &str = ".aura_experiences.jsonl";

fn experiences_path() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(EXPERIENCES_FILE)
}

#[allow(dead_code)]
pub struct ExperienceStore;

impl ExperienceStore {
    /// Guarda un nuevo registro de experiencia en el almacén global JSONL
    #[allow(dead_code)]
    pub fn record_experience(exp: &ExperienceRecord) -> Result<(), String> {
        let line = serde_json::to_string(exp).map_err(|e| e.to_string())?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(experiences_path())
            .map_err(|e| e.to_string())?;
        writeln!(file, "{}", line).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Carga las últimas N experiencias
    #[allow(dead_code)]
    pub fn load_recent(limit: usize) -> Vec<ExperienceRecord> {
        let p = experiences_path();
        if !p.exists() {
            return Vec::new();
        }

        let file = match std::fs::File::open(&p) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };

        let reader = std::io::BufReader::new(file);
        let mut records: Vec<ExperienceRecord> = Vec::new();

        for line in reader.lines().flatten() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(rec) = serde_json::from_str::<ExperienceRecord>(trimmed) {
                records.push(rec);
            }
        }

        if records.len() > limit {
            records.split_off(records.len() - limit)
        } else {
            records
        }
    }

    /// Recupera ejemplos solo del proyecto activo. Las experiencias globales
    /// parecidas por palabras pueden pertenecer a una aplicación distinta y no
    /// deben contaminar el contexto de una misión.
    pub fn find_analogous_in_workspace(
        objective: &str,
        lang: &str,
        workspace: &str,
        limit: usize,
    ) -> Vec<ExperienceRecord> {
        let scoped = records_for_workspace(Self::load_recent(50), workspace);
        analogous_records(scoped, objective, lang, limit)
    }

    fn build_context_from_records(relevant: Vec<ExperienceRecord>) -> String {
        let relevant: Vec<_> = relevant
            .into_iter()
            .filter(|record| !matches!(record.outcome, ExperienceOutcome::Failed(_)))
            .collect();
        if relevant.is_empty() {
            return String::new();
        }

        let mut block = String::from("🧠 [EXPERIENCIA COGNITIVA PREVIA DEL WORKSPACE ACTUAL]:\n");
        for exp in relevant {
            let status = match &exp.outcome {
                ExperienceOutcome::Success => "ÉXITO",
                ExperienceOutcome::PartialSuccess => "PARCIAL",
                ExperienceOutcome::Failed(_) => continue,
            };
            block.push_str(&format!(
                "• Tarea: {} | Resultado: {}\n",
                exp.objective, status
            ));
            if !exp.key_lessons.is_empty() {
                block.push_str(&format!("  Lección: {}\n", exp.key_lessons.join("; ")));
            }
        }
        block.push('\n');
        block
    }

    /// Construye contexto de experiencias análogas únicamente del workspace activo.
    pub fn build_experience_context(objective: &str, lang: &str, workspace: &str) -> String {
        let relevant = Self::find_analogous_in_workspace(objective, lang, workspace, 3);
        Self::build_context_from_records(relevant)
    }

    /// Generates a quick record at mission completion.
    #[allow(dead_code)]
    pub fn create_record(
        workspace: &str,
        objective: &str,
        language: &str,
        tools: Vec<String>,
        steps: u32,
        outcome: ExperienceOutcome,
        lessons: Vec<String>,
    ) -> ExperienceRecord {
        ExperienceRecord {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now().to_rfc3339(),
            workspace: workspace.to_string(),
            objective: objective.to_string(),
            fingerprint: TaskFingerprint {
                language: language.to_string(),
                frameworks: Vec::new(),
                file_count: 0,
                tools_required: tools,
            },
            steps_taken: steps,
            outcome,
            strategy_summary: format!("Ejecución completada en {} pasos", steps),
            key_lessons: lessons,
        }
    }
}

fn analogous_records(
    all: Vec<ExperienceRecord>,
    objective: &str,
    lang: &str,
    limit: usize,
) -> Vec<ExperienceRecord> {
    let obj_tokens: Vec<String> = objective
        .to_lowercase()
        .split_whitespace()
        .filter(|w| w.len() > 3)
        .map(|w| w.to_string())
        .collect();

    let mut scored: Vec<(usize, ExperienceRecord)> = all
        .into_iter()
        .filter_map(|rec| {
            let mut score = 0usize;
            if !lang.is_empty() && rec.fingerprint.language.eq_ignore_ascii_case(lang) {
                score += 3;
            }
            let rec_obj = rec.objective.to_lowercase();
            for token in &obj_tokens {
                if rec_obj.contains(token) {
                    score += 1;
                }
            }
            if score > 0 {
                Some((score, rec))
            } else {
                None
            }
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().take(limit).map(|(_, r)| r).collect()
}

fn same_workspace(left: &str, right: &str) -> bool {
    let normalize = |value: &str| {
        let path = Path::new(value)
            .canonicalize()
            .unwrap_or_else(|_| Path::new(value).to_path_buf());
        let normalized = path.to_string_lossy().replace('\\', "/");
        #[cfg(windows)]
        {
            normalized.to_lowercase()
        }
        #[cfg(not(windows))]
        {
            normalized
        }
    };
    normalize(left) == normalize(right)
}

fn records_for_workspace(records: Vec<ExperienceRecord>, workspace: &str) -> Vec<ExperienceRecord> {
    records
        .into_iter()
        .filter(|record| same_workspace(&record.workspace, workspace))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_find_analogous() {
        let rec = ExperienceStore::create_record(
            "test_ws",
            "Crear un servidor web con Rust y Actix",
            "rust",
            vec!["TOOL_PROGRAMMER".to_string(), "TOOL_TERMINAL".to_string()],
            12,
            ExperienceOutcome::Success,
            vec!["Configurar Cargo.toml antes de compilar".to_string()],
        );

        assert_eq!(rec.fingerprint.language, "rust");
        assert_eq!(rec.steps_taken, 12);
        assert_eq!(rec.outcome, ExperienceOutcome::Success);
    }

    #[test]
    fn workspace_experience_filter_excludes_similar_tasks_from_other_projects() {
        let mut current = ExperienceStore::create_record(
            "C:/projects/consulta-clara",
            "Construir una aplicación web de consultas",
            "javascript",
            vec!["TOOL_PROGRAMMER".to_string()],
            8,
            ExperienceOutcome::Success,
            vec!["Validar los flujos de citas".to_string()],
        );
        let mut other = current.clone();
        other.id = "otro-proyecto".into();
        other.workspace = "C:/projects/cotizador".into();
        let filtered =
            records_for_workspace(vec![other, current.clone()], "c:\\projects\\consulta-clara");

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].workspace, current.workspace);
        current.outcome = ExperienceOutcome::Failed("syntax error viejo".into());
        assert!(ExperienceStore::build_context_from_records(vec![current]).is_empty());
    }
}
