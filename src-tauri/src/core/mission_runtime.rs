#![allow(dead_code)]
use crate::core::mission_contract::MissionContract;
use crate::core::cognitive_state::CognitiveState;
use crate::core::evidence::EvidenceGraph;
use crate::core::stall_detector::{StallDetector, StallType, ProgressSignature};
use crate::core::policy::{PolicyEngine, ActionProposal, PolicyDecision};
use crate::core::completion_gate::{CompletionGate, CompletionDecision};
use crate::core::step_budget::StepBudget;
use crate::core::recovery::{RecoveryEngine, RecoveryDecision, classify_error};
use crate::core::world_state::WorldState;
use crate::core::observation::Observation;

/// Runtime governance controller — the cognitive brain of agent.rs.
pub struct MissionRuntime {
    pub mission_id: String,
    pub workspace_path: String,
    pub contract: MissionContract,
    pub cognitive_state: CognitiveState,
    pub evidence_graph: EvidenceGraph,
    pub stall_detector: StallDetector,
    pub budget: StepBudget,
    pub recovery: RecoveryEngine,
    pub world: Option<WorldState>,
}

impl MissionRuntime {
    pub fn new(workspace_path: &str, objective: &str, max_steps: u32) -> Self {
        let mission_id = format!(
            "m_{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        MissionRuntime {
            contract: MissionContract::new(objective),
            cognitive_state: CognitiveState::new(&mission_id, objective),
            evidence_graph: EvidenceGraph::new(),
            stall_detector: StallDetector::new(6),
            budget: StepBudget::new(max_steps),
            recovery: RecoveryEngine::new(),
            world: None,
            workspace_path: workspace_path.to_string(),
            mission_id,
        }
    }

    // ─── Budget ────────────────────────────────────────────────────────────────

    pub fn budget_remaining(&self) -> u32 {
        self.budget.remaining_steps()
    }

    pub fn is_budget_exhausted(&self) -> bool {
        self.budget.is_exhausted()
    }

    /// Counts one cognitive cycle (LLM decision). Does NOT count a tool execution.
    pub fn record_step(&mut self) -> u32 {
        self.cognitive_state.update_step();
        self.budget.record_step()
    }

    /// Counts an actual tool execution (called only when a tool really ran).
    pub fn record_tool_call(&mut self) {
        self.cognitive_state.metrics.tool_calls += 1;
    }

    /// Returns the current step number from the cognitive state.
    pub fn current_step(&self) -> u32 {
        self.cognitive_state.mission.current_step
    }

    // ─── World Observation ─────────────────────────────────────────────────────

    /// Takes a fresh workspace snapshot and stores it in the runtime.
    pub fn observe_world(&mut self) {
        match WorldState::capture(&self.workspace_path) {
            Ok(ws) => {
                self.cognitive_state.set_world(ws.clone());
                self.world = Some(ws);
            }
            Err(e) => {
                eprintln!("[MissionRuntime] observe_world error: {}", e);
            }
        }
    }

    /// Returns the current world state hash using content hashes, not just file sizes.
    /// This correctly detects when a file changes content but keeps the same size.
    pub fn current_world_hash(&self) -> u64 {
        self.world.as_ref().map(|w| {
            use std::hash::{Hash, Hasher};
            use std::collections::hash_map::DefaultHasher;
            let mut hasher = DefaultHasher::new();
            // Sort for determinism, then hash path + content_hash (not size_bytes)
            let mut entries: Vec<(&String, &crate::core::world_state::FileSnapshot)> =
                w.files.iter().collect();
            entries.sort_by_key(|(p, _)| p.as_str());
            for (path, snapshot) in entries {
                path.hash(&mut hasher);
                snapshot.content_hash.hash(&mut hasher);
            }
            hasher.finish()
        }).unwrap_or(0)
    }

    // ─── Observation Recording ─────────────────────────────────────────────────

    /// Records a structured tool observation and feeds it to StallDetector.
    pub fn record_observation(&mut self, obs: &Observation) {
        let ok = obs.status == crate::core::observation::ObservationStatus::Success;
        if !ok {
            self.cognitive_state.metrics.failed_actions += 1;
        } else {
            // Record implicit evidence for successful terminal / validator tool calls
            let tool_upper = obs.tool_name.to_uppercase();
            if tool_upper.contains("TERMINAL") || tool_upper.contains("VALIDATOR") {
                use crate::core::evidence::EvidenceKind;
                let claim = format!(
                    "{} exitoso en paso {}",
                    obs.tool_name, self.cognitive_state.mission.current_step
                );
                self.evidence_graph.record(
                    EvidenceKind::CommandExitCode,
                    &obs.tool_name,
                    &claim,
                    &obs.payload,
                    0.85,
                    self.cognitive_state.mission.current_step,
                );
            }
        }

        // Feed stall detector with REAL command and state hash from observation
        use std::hash::{Hash, Hasher};
        use std::collections::hash_map::DefaultHasher;
        let mut hasher = DefaultHasher::new();
        if !ok { obs.payload.hash(&mut hasher); }
        let err_hash = if ok { 0 } else { hasher.finish() };

        // Use real state_hash from observation if available, else fall back to current world hash
        let state_hash = obs.state_hash_after
            .or(obs.state_hash_before)
            .unwrap_or_else(|| self.current_world_hash());

        let sig = ProgressSignature {
            step: self.cognitive_state.mission.current_step,
            state_hash,
            files_changed: obs.files_affected.len() as u32,
            criteria_satisfied: self.contract.acceptance_criteria
                .iter()
                .filter(|c| c.status == crate::core::mission_contract::CriterionStatus::Satisfied)
                .count() as u32,
            evidence_count: self.evidence_graph.entries.len() as u32,
            last_tool_used: obs.tool_name.clone(),
            last_command: obs.command.clone().unwrap_or_default(),
            last_error_hash: err_hash,
        };
        self.stall_detector.record_signature(sig);
    }

    // ─── Stall Detection ───────────────────────────────────────────────────────

    pub fn should_stall_recover(&self, window: usize) -> Option<StallType> {
        self.stall_detector.detect_stall(window)
    }

    // ─── Policy ────────────────────────────────────────────────────────────────

    pub fn check_policy(&self, proposal: &ActionProposal) -> PolicyDecision {
        PolicyEngine::authorize(proposal)
    }

    // ─── Completion Gate ───────────────────────────────────────────────────────

    /// The ONLY authority allowed to declare mission complete.
    /// Never let agent.rs declare completion without calling this.
    pub fn can_complete(&self) -> CompletionDecision {
        CompletionGate::evaluate(&self.contract, &self.cognitive_state, &self.evidence_graph)
    }

    // ─── Recovery ──────────────────────────────────────────────────────────────

    pub fn plan_recovery(&mut self, tool_name: &str, error_msg: &str) -> RecoveryDecision {
        let class = classify_error(error_msg);
        self.cognitive_state.metrics.recovery_actions += 1;
        self.recovery.recover(tool_name, error_msg, class)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mission_runtime_init() {
        let rt = MissionRuntime::new(".", "Test objective", 50);
        assert_eq!(rt.budget.total_steps, 50);
        assert!(!rt.is_budget_exhausted());
        assert_eq!(rt.budget_remaining(), 50);
    }

    #[test]
    fn test_runtime_delegates_to_completion_gate() {
        let rt = MissionRuntime::new(".", "Build something", 50);
        // With no criteria and no evidence, empty contract must NOT complete.
        // This guards against silent false-positive completions.
        let dec = rt.can_complete();
        assert!(matches!(dec, crate::core::completion_gate::CompletionDecision::Incomplete(_)));
    }

    #[test]
    fn test_runtime_budget_tracking() {
        let mut rt = MissionRuntime::new(".", "Test", 10);
        for _ in 0..10 { rt.record_step(); }
        assert!(rt.is_budget_exhausted());
        assert_eq!(rt.budget_remaining(), 0);
    }
}
