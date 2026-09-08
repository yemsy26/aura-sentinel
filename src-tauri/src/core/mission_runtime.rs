use crate::core::mission_contract::MissionContract;
use crate::core::cognitive_state::CognitiveState;
use crate::core::evidence::EvidenceGraph;
use crate::core::stall_detector::StallDetector;
use crate::core::policy::PolicyEngine;

#[allow(dead_code)]
pub struct MissionRuntime {
    pub mission_id: String,
    pub workspace_path: String,
    pub contract: MissionContract,
    pub cognitive_state: CognitiveState,
    pub evidence_graph: EvidenceGraph,
    pub stall_detector: StallDetector,
    pub step_budget: u32,
    pub max_steps: u32,
}

impl MissionRuntime {
    #[allow(dead_code)]
    pub fn new(workspace_path: &str, objective: &str, max_steps: u32) -> Self {
        let mission_id = format!("m_{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        let contract = MissionContract::new(objective);
        let cognitive_state = CognitiveState::new(&mission_id, objective, workspace_path);
        let evidence_graph = EvidenceGraph::new();
        let stall_detector = StallDetector::new(5);

        MissionRuntime {
            mission_id,
            workspace_path: workspace_path.to_string(),
            contract,
            cognitive_state,
            evidence_graph,
            stall_detector,
            step_budget: max_steps,
            max_steps,
        }
    }

    #[allow(dead_code)]
    pub fn is_budget_exhausted(&self, current_step: u32) -> bool {
        current_step >= self.max_steps
    }

    #[allow(dead_code)]
    pub fn record_action_step(&mut self) -> u32 {
        self.cognitive_state.update_step();
        self.cognitive_state.metrics.tool_calls += 1;
        self.cognitive_state.mission.current_step
    }

    #[allow(dead_code)]
    pub fn can_complete(&self) -> crate::core::completion_gate::CompletionDecision {
        crate::core::completion_gate::CompletionGate::evaluate(
            &self.contract,
            &self.cognitive_state,
            &self.evidence_graph,
        )
    }

    #[allow(dead_code)]
    pub fn check_policy(&self, proposal: &crate::core::policy::ActionProposal) -> crate::core::policy::PolicyDecision {
        PolicyEngine::authorize(proposal)
    }

    #[allow(dead_code)]
    pub fn record_tool_result(&mut self, ok: bool) {
        if !ok {
            self.cognitive_state.metrics.failed_actions += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mission_runtime_init() {
        let rt = MissionRuntime::new(".", "Test objective", 50);
        assert_eq!(rt.max_steps, 50);
        assert!(!rt.is_budget_exhausted(10));
        assert!(rt.is_budget_exhausted(50));
    }
}
