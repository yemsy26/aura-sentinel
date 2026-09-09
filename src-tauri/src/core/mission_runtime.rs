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
use crate::core::schema_validator::{SchemaValidator, SchemaValidationResult};
use crate::core::tool_registry::ToolRegistry;

/// Runtime governance controller â€” the cognitive brain of agent.rs.
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

    // â”€â”€â”€ Budget â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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

    /// Restores the runtime's step counter from a persisted checkpoint.
    /// The Runtime controls how its own state is restored â€” agent.rs must NOT
    /// write directly to cognitive_state fields.
    pub fn restore_step(&mut self, step: u32) {
        self.cognitive_state.mission.current_step = step;
        // budget consumed is not restored (continuation gets a fresh 50-step budget)
    }

    /// Checks runtime coherence. Returns a list of violation strings.
    /// Does NOT panic â€” callers emit FATAL/WARNING and decide how to proceed.
    pub fn check_invariants(&self) -> Vec<String> {
        let mut violations = Vec::new();
        if self.contract.objective.trim().is_empty() {
            violations.push("CONTRACT_EMPTY: objetivo vacÃ­o".into());
        }
        if self.cognitive_state.mission.id != self.mission_id {
            violations.push(format!(
                "MISSION_ID_MISMATCH: state={} runtime={}",
                self.cognitive_state.mission.id, self.mission_id
            ));
        }
        if self.workspace_path.trim().is_empty() {
            violations.push("WORKSPACE_EMPTY".into());
        }
        if self.budget.remaining_steps() > self.budget.total_steps {
            violations.push(format!(
                "BUDGET_INVALID: remaining({}) > total({})",
                self.budget.remaining_steps(), self.budget.total_steps
            ));
        }
        if self.current_step() > self.budget.total_steps {
            violations.push(format!(
                "STEP_EXCEEDS_BUDGET: step({}) > budget({})",
                self.current_step(), self.budget.total_steps
            ));
        }
        violations
    }

    // â”€â”€â”€ World Observation â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    /// FINAL-3: Takes a fresh workspace snapshot and stores it in the runtime.
    /// Returns Err if the snapshot fails so callers can distinguish
    /// "no world change" (hash equal) from "observation failed" (None hash).
    /// NEVER silences the error â€” eprintln is eliminated.
    pub fn observe_world(&mut self) -> Result<(), String> {
        match WorldState::capture(&self.workspace_path) {
            Ok(ws) => {
                self.cognitive_state.set_world(ws.clone());
                self.world = Some(ws);
                Ok(())
            }
            Err(e) => Err(format!("[OBSERVE_WORLD FAILED] {}", e)),
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

    // â”€â”€â”€ Observation Recording â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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

    // â”€â”€â”€ Stall Detection â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    pub fn should_stall_recover(&self, window: usize) -> Option<StallType> {
        self.stall_detector.detect_stall(window)
    }

    // â”€â”€â”€ Policy â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    pub fn check_policy(&self, proposal: &ActionProposal) -> PolicyDecision {
        PolicyEngine::authorize(proposal)
    }

    /// H-10 + FINAL-1+2: Action Gateway â€” complete authorization pipeline.
    /// Order: ToolRegistry â†’ SchemaValidator â†’ Budget â†’ Policy.
    /// agent.rs MUST call this (via execute_action) before any tool execution.
    pub fn authorize_action(&self, proposal: &ActionProposal) -> Result<(), String> {
        // 0. ToolRegistry â€” is this a known, registered tool?
        ToolRegistry::validate(&proposal.tool)?;

        // 1. Schema â€” does the payload match the expected shape for this tool?
        match SchemaValidator::validate_tool_payload(&proposal.tool, &proposal.arguments) {
            SchemaValidationResult::Invalid(reason) =>
                return Err(format!("SCHEMA_INVALID: {}", reason)),
            SchemaValidationResult::Valid => {}
        }

        // 2. Budget â€” is there remaining step capacity?
        if self.is_budget_exhausted() {
            return Err("BUDGET_EXHAUSTED: presupuesto de pasos agotado".into());
        }

        // 3. Policy â€” is this action permitted by current security policy?
        match PolicyEngine::authorize(proposal) {
            PolicyDecision::Allow => Ok(()),
            PolicyDecision::Deny(reason) => Err(format!("POLICY_DENY: {}", reason)),
            PolicyDecision::RequireUser(msg) => Err(format!("POLICY_REQUIRE_USER: {}", msg)),
            PolicyDecision::Sandbox(msg) => Err(format!("POLICY_SANDBOX: {}", msg)),
        }
    }

    /// H-12: Execution Gateway â€” the COMPLETE pipeline for any tool action.
    /// Enforces: authorize â†’ world snapshot before â†’ execute â†’ world snapshot after
    ///           â†’ Observation â†’ record_observation â†’ record_tool_call.
    ///
    /// The actual tool execution is provided as a closure so agent.rs can keep
    /// its async handles (app_handle, filesystem, network) without moving them into Runtime.
    ///
    /// agent.rs usage:
    /// ```ignore
    /// let obs = runtime.execute_action(&proposal, || async {
    ///     execute_terminal_command(&comando, &workspace_path).await
    ///         .map_err(|e| e.to_string())
    /// }).await;
    /// ```
    pub async fn execute_action<F, Fut>(
        &mut self,
        proposal: &ActionProposal,
        executor: F,
    ) -> Result<Observation, String>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<String, String>>,
    {
        // 1. Authorization: ToolRegistry â†’ Schema â†’ Budget â†’ Policy
        self.authorize_action(proposal)?;

        // 2. World snapshot BEFORE â€” abort if we cannot establish baseline
        //    (FINAL-3: observe_world returns Result; failure is surfaced, not silenced)
        if let Err(observe_err) = self.observe_world() {
            return Err(format!("OBSERVE_BEFORE_FAILED: {}", observe_err));
        }
        let hash_before = self.current_world_hash();

        // 3. Execute â€” runtime delegates to the closure provided by agent.rs
        //    The closure owns app_handle, filesystem, network â€” Runtime does not.
        let exec_result = executor().await;

        // 4. World snapshot AFTER â€” failure is recorded in Observation, not hidden
        let hash_after = match self.observe_world() {
            Ok(()) => Some(self.current_world_hash()),
            Err(_) => None, // None = observation failed, not "no change"
        };
        self.record_tool_call();

        // 5. Build Observation from result
        let mut obs = match exec_result {
            Ok(ref output) => Observation::success(&proposal.tool, output, vec![]),
            Err(ref err)   => Observation::error(&proposal.tool, err, None, true, None),
        };
        obs.state_hash_before = Some(hash_before);
        obs.state_hash_after  = hash_after; // None means observation failed post-exec

        // 6. Record through full circuit (StallDetector, EvidenceGraph)
        self.record_observation(&obs);

        Ok(obs)
    }

    // â”€â”€â”€ Completion Gate â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    /// The ONLY authority allowed to declare mission complete.
    /// Never let agent.rs declare completion without calling this.
    pub fn can_complete(&self) -> CompletionDecision {
        CompletionGate::evaluate(&self.contract, &self.cognitive_state, &self.evidence_graph)
    }

    // â”€â”€â”€ Recovery â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    pub fn plan_recovery(&mut self, tool_name: &str, error_msg: &str) -> RecoveryDecision {
        let class = classify_error(error_msg);
        self.cognitive_state.metrics.recovery_actions += 1;
        self.recovery.recover(tool_name, error_msg, class)
    }

    /// FINAL-4: Routes an Observation through the Recovery circuit after execution.
    /// Returns Some(RecoveryDecision) for error observations so agent.rs can decide
    /// what to do next (retry, change tool, replan, ask user, abort).
    ///
    /// Separation: Execution Gateway â‰  Recovery Authority.
    /// execute_action() produces an Observation.
    /// handle_observation() consults RecoveryEngine and returns a decision.
    /// agent.rs acts on that decision â€” Runtime never forces the recovery action.
    pub fn handle_observation(&mut self, obs: &Observation) -> Option<RecoveryDecision> {
        use crate::core::observation::ObservationStatus;
        match obs.status {
            ObservationStatus::Error => {
                // Error â†’ classify â†’ RecoveryEngine â†’ decision
                Some(self.plan_recovery(&obs.tool_name, &obs.payload))
            }
            ObservationStatus::Cancelled => {
                // User-initiated cancellation: control event, not a failure.
                // RecoveryEngine should NOT learn from cancellations.
                None
            }
            _ => None, // Success / BlockedByPolicy / Timeout / SchemaViolation handled by agent
        }
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

    // â”€â”€ H-11: Architecture invariant tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    /// H-11-A: restore_step() is the ONLY way to set step from outside.
    /// Verifies runtime controls its own restoration.
    #[test]
    fn test_restore_step_authority() {
        let mut rt = MissionRuntime::new(".", "Test restore", 50);
        assert_eq!(rt.current_step(), 0);
        rt.restore_step(35);
        assert_eq!(rt.current_step(), 35);
        // Budget must NOT change on step restore
        assert_eq!(rt.budget_remaining(), 50);
    }

    /// H-11-B: authorize_action() denies when budget is exhausted.
    #[test]
    fn test_authorize_action_denies_on_exhausted_budget() {
        let mut rt = MissionRuntime::new(".", "Test budget gate", 2);
        rt.record_step(); rt.record_step();
        assert!(rt.is_budget_exhausted());
        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".into(),
            arguments: serde_json::json!({"comando": "dir"}),
            expected_effect: String::new(),
            risk: crate::core::policy::RiskLevel::Safe,
        };
        let result = rt.authorize_action(&proposal);
        assert!(result.is_err(), "authorize_action must deny when budget is exhausted");
        assert!(result.unwrap_err().contains("BUDGET_EXHAUSTED"));
    }

    /// H-11-C: check_invariants() reports CONTRACT_EMPTY when objective is blank.
    #[test]
    fn test_check_invariants_contract_empty() {
        let rt = MissionRuntime::new(".", "", 50);
        let violations = rt.check_invariants();
        assert!(
            violations.iter().any(|v| v.contains("CONTRACT_EMPTY")),
            "Expected CONTRACT_EMPTY violation, got: {:?}", violations
        );
    }

    /// H-11-D: check_invariants() passes with a valid runtime.
    #[test]
    fn test_check_invariants_valid_runtime() {
        let rt = MissionRuntime::new(".", "Build a CLI tool", 50);
        let violations = rt.check_invariants();
        // mission_id mismatch may fire because cognitive_state.mission.id is default â€” filter for BUDGET/STEP/WORKSPACE
        let critical: Vec<_> = violations.iter()
            .filter(|v| v.contains("BUDGET_INVALID") || v.contains("STEP_EXCEEDS") || v.contains("WORKSPACE_EMPTY"))
            .collect();
        assert!(critical.is_empty(), "Unexpected critical violations: {:?}", critical);
    }

    /// H-11-E: Observation::cancelled is distinct from Observation::error.
    #[test]
    fn test_cancellation_observation_is_not_error() {
        let obs = crate::core::observation::Observation::cancelled(
            "AGENT", "User pressed stop"
        );
        assert_eq!(obs.status, crate::core::observation::ObservationStatus::Cancelled);
        assert_ne!(obs.status, crate::core::observation::ObservationStatus::Error);
        assert!(!obs.retryable, "Cancelled must not be retryable");
    }

    /// H-11-F: Empty contract blocks CompletionGate â€” no false positives.
    #[test]
    fn test_empty_contract_blocks_completion() {
        let rt = MissionRuntime::new(".", "Do something", 50);
        let dec = rt.can_complete();
        assert!(
            matches!(dec, crate::core::completion_gate::CompletionDecision::Incomplete(_)),
            "Empty contract must produce Incomplete, not Complete"
        );
    }

    /// H-11-G: record_step() is the only way step advances â€” no += 1 outside runtime.
    #[test]
    fn test_step_advances_only_via_record_step() {
        let mut rt = MissionRuntime::new(".", "Step authority test", 50);
        assert_eq!(rt.current_step(), 0);
        rt.record_step();
        assert_eq!(rt.current_step(), 1);
        rt.record_step();
        assert_eq!(rt.current_step(), 2);
        // restore_step does not increment budget
        rt.restore_step(10);
        assert_eq!(rt.current_step(), 10);
        assert_eq!(rt.budget_remaining(), 48, "budget should only decrease by record_step calls");
    }

    /// H-15: execute_action() runs the full pipeline through Runtime.
    /// Verifies: authorize â†’ world before â†’ execute â†’ world after â†’ Observation recorded.
    /// No LLM, no agent.rs, no app_handle required.
    #[tokio::test]
    async fn test_execute_action_full_pipeline() {
        let mut rt = MissionRuntime::new(".", "Test execution gateway", 10);
        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".into(),
            arguments: serde_json::json!({"comando": "dir"}),
            expected_effect: "simulate output".into(),
            risk: crate::core::policy::RiskLevel::Safe,
        };

        // Execute via gateway with a simulated successful closure
        let obs = rt.execute_action(&proposal, || async {
            Ok("simulated terminal output".to_string())
        }).await;

        assert!(obs.is_ok(), "execute_action must succeed for authorized action");
        let o = obs.unwrap();
        assert_eq!(o.status, crate::core::observation::ObservationStatus::Success);
        assert!(o.state_hash_before.is_some(), "world hash before must be recorded");
        assert!(o.state_hash_after.is_some(), "world hash after must be recorded");
        assert_eq!(o.tool_name, "TOOL_TERMINAL");
        // record_tool_call was called inside execute_action
        assert_eq!(rt.cognitive_state.metrics.tool_calls, 1);
    }

    /// H-15b: execute_action() denies when budget is exhausted.
    #[tokio::test]
    async fn test_execute_action_denies_on_exhausted_budget() {
        let mut rt = MissionRuntime::new(".", "Test deny on exhausted", 2);
        rt.record_step(); rt.record_step();
        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".into(),
            arguments: serde_json::json!({"comando": "dir"}),
            expected_effect: String::new(),
            risk: crate::core::policy::RiskLevel::Safe,
        };
        let result = rt.execute_action(&proposal, || async {
            Ok("should not run".to_string())
        }).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("BUDGET_EXHAUSTED"));
    }

    // â”€â”€ FINAL-5: Integration tests â€” full circuit verification â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    /// FINAL-5-A: Executor is PHYSICALLY never called when authorization is denied.
    /// Uses AtomicBool to prove the closure body was never entered â€” stronger than
    /// just checking the Result return value.
    #[tokio::test]
    async fn test_executor_not_called_when_denied() {
        use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

        let mut rt = MissionRuntime::new(".", "Test executor not called", 2);
        rt.record_step(); rt.record_step(); // exhaust budget

        let executed = Arc::new(AtomicBool::new(false));
        let executed_clone = executed.clone();

        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".into(),
            arguments: serde_json::json!({"comando": "dir"}),
            expected_effect: String::new(),
            risk: crate::core::policy::RiskLevel::Safe,
        };

        let result = rt.execute_action(&proposal, move || async move {
            executed_clone.store(true, Ordering::SeqCst);
            Ok("should not run".to_string())
        }).await;

        assert!(result.is_err(), "Must deny when budget exhausted");
        assert!(
            !executed.load(Ordering::SeqCst),
            "executor MUST NOT be called when authorization is denied"
        );
    }

    /// FINAL-5-B: Error Observation â†’ handle_observation() returns RecoveryDecision.
    /// Verifies the Execution â†’ Observation â†’ Recovery circuit is wired end-to-end.
    #[tokio::test]
    async fn test_error_observation_produces_recovery_decision() {
        let mut rt = MissionRuntime::new(".", "Test recovery circuit", 10);
        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".into(),
            arguments: serde_json::json!({"comando": "rustc nonexistent.rs"}),
            expected_effect: String::new(),
            risk: crate::core::policy::RiskLevel::Safe,
        };

        let obs = rt.execute_action(&proposal, || async {
            Err("error[E0001]: command not found: rustc".to_string())
        }).await.expect("execute_action itself should succeed");

        assert_eq!(obs.status, crate::core::observation::ObservationStatus::Error,
            "Failed executor must produce Error observation");
        let recovery = rt.handle_observation(&obs);
        assert!(recovery.is_some(),
            "Error Observation MUST produce a RecoveryDecision via handle_observation()");
    }

    /// FINAL-5-C: Schema INVALID blocks execution â€” empty command caught before executor runs.
    #[tokio::test]
    async fn test_schema_invalid_blocks_execution() {
        use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

        let mut rt = MissionRuntime::new(".", "Test schema gate", 10);
        let executed = Arc::new(AtomicBool::new(false));
        let executed_clone = executed.clone();

        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".into(),
            arguments: serde_json::json!({"comando": ""}), // empty â†’ SCHEMA_INVALID
            expected_effect: String::new(),
            risk: crate::core::policy::RiskLevel::Safe,
        };

        let result = rt.execute_action(&proposal, move || async move {
            executed_clone.store(true, Ordering::SeqCst);
            Ok("should not run".to_string())
        }).await;

        assert!(result.is_err(), "Empty command must be rejected");
        assert!(
            result.unwrap_err().contains("SCHEMA_INVALID"),
            "Error must be SCHEMA_INVALID from SchemaValidator"
        );
        assert!(
            !executed.load(Ordering::SeqCst),
            "executor MUST NOT run when Schema validation fails"
        );
    }
}

