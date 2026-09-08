use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::io::{BufRead, Write};
use chrono::Utc;

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

    /// Busca experiencias relevantes que coincidan con palabras clave o lenguaje
    #[allow(dead_code)]
    pub fn find_analogous(objective: &str, lang: &str, limit: usize) -> Vec<ExperienceRecord> {
        let all = Self::load_recent(50);
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

    /// Construye un bloque formateado de lecciones aprendidas para inyectar al LLM
    #[allow(dead_code)]
    pub fn build_experience_context(objective: &str, lang: &str) -> String {
        let relevant = Self::find_analogous(objective, lang, 3);
        if relevant.is_empty() {
            return String::new();
        }

        let mut block = String::from("🧠 [EXPERIENCIA COGNITIVA PREVIA]:\n");
        for exp in relevant {
            let status = match &exp.outcome {
                ExperienceOutcome::Success => "✅ ÉXITO",
                ExperienceOutcome::PartialSuccess => "⚠️ PARCIAL",
                ExperienceOutcome::Failed(reason) => &format!("❌ FALLO ({})", reason),
            };
            block.push_str(&format!("• Tarea: {} | Resultado: {}\n", exp.objective, status));
            if !exp.key_lessons.is_empty() {
                block.push_str(&format!("  Lección: {}\n", exp.key_lessons.join("; ")));
            }
        }
        block.push('\n');
        block
    }

    /// Genera un registro rápido al finalizar una misión
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
}
