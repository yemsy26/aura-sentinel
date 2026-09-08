#![allow(dead_code)]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CriterionStatus {
    Pending,
    Satisfied,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerificationMethod {
    FileExistence(String),
    CommandExitZero(String),
    TestPassed,
    ContentMatches { file: String, regex: String },
    ManualReview,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptanceCriterion {
    pub id: String,
    pub description: String,
    pub verification: VerificationMethod,
    pub required: bool,
    pub status: CriterionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Constraint {
    pub id: String,
    pub rule: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRequirement {
    pub claim: String,
    pub min_reliability: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionContract {
    pub objective: String,
    pub constraints: Vec<Constraint>,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub forbidden_actions: Vec<String>,
    pub required_evidence: Vec<EvidenceRequirement>,
}

impl MissionContract {
    pub fn new(objective: &str) -> Self {
        Self {
            objective: objective.to_string(),
            constraints: vec![
                Constraint {
                    id: "C-01".to_string(),
                    rule: "No modificar archivos fuera del workspace".to_string(),
                    active: true,
                },
                Constraint {
                    id: "C-02".to_string(),
                    rule: "Escribir código completo sin stubs ni placeholders".to_string(),
                    active: true,
                },
            ],
            acceptance_criteria: Vec::new(),
            forbidden_actions: vec![
                "rm -rf /".to_string(),
                "format".to_string(),
            ],
            required_evidence: Vec::new(),
        }
    }

    pub fn add_criterion(&mut self, id: &str, description: &str, verification: VerificationMethod, required: bool) {
        self.acceptance_criteria.push(AcceptanceCriterion {
            id: id.to_string(),
            description: description.to_string(),
            verification,
            required,
            status: CriterionStatus::Pending,
        });
    }

    pub fn mark_criterion(&mut self, id: &str, satisfied: bool) -> bool {
        if let Some(ac) = self.acceptance_criteria.iter_mut().find(|c| c.id == id) {
            ac.status = if satisfied {
                CriterionStatus::Satisfied
            } else {
                CriterionStatus::Failed
            };
            true
        } else {
            false
        }
    }

    pub fn are_required_criteria_satisfied(&self) -> bool {
        self.acceptance_criteria
            .iter()
            .filter(|c| c.required)
            .all(|c| c.status == CriterionStatus::Satisfied)
    }

    pub fn pending_criteria(&self) -> Vec<String> {
        self.acceptance_criteria
            .iter()
            .filter(|c| c.status != CriterionStatus::Satisfied)
            .map(|c| format!("[{}] {}", c.id, c.description))
            .collect()
    }
}
