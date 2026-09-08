#![allow(dead_code)]
use serde::{Deserialize, Serialize};
use crate::core::world_state::WorldState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionStatus {
    Planning,
    Executing,
    Verifying,
    Blocked,
    WaitingUser,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionState {
    pub id: String,
    pub objective: String,
    pub status: MissionStatus,
    pub current_phase: usize,
    pub current_step: u32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeMetrics {
    pub total_steps: u32,
    pub tool_calls: u32,
    pub verification_attempts: u32,
    pub successful_verifications: u32,
    pub failed_actions: u32,
    pub recovery_actions: u32,
    pub replans: u32,
    pub stall_events: u32,
    pub tokens_estimated: u64,
    pub elapsed_ms: u64,
}

/// Unified cognitive state. WorldState comes from world_state::WorldState (no duplicate).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitiveState {
    pub mission: MissionState,
    pub world: Option<WorldState>,
    pub beliefs: Vec<String>,
    pub metrics: RuntimeMetrics,
}

impl CognitiveState {
    pub fn new(mission_id: &str, objective: &str) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            mission: MissionState {
                id: mission_id.to_string(),
                objective: objective.to_string(),
                status: MissionStatus::Planning,
                current_phase: 0,
                current_step: 0,
                created_at: now.clone(),
                updated_at: now,
            },
            world: None,
            beliefs: Vec::new(),
            metrics: RuntimeMetrics::default(),
        }
    }

    pub fn update_step(&mut self) {
        self.mission.current_step += 1;
        self.metrics.total_steps += 1;
        self.mission.updated_at = chrono::Utc::now().to_rfc3339();
    }

    pub fn set_world(&mut self, world: WorldState) {
        self.world = Some(world);
    }
}
