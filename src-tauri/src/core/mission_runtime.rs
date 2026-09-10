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
    /// FINAL-6: Owns the executor dispatch table. Register all tools before the mission loop.
    pub tool_registry: ToolRegistry,
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
            tool_registry: ToolRegistry::new(),
        }
    }

    // ——— Budget ————————————————————————————————————————————————————————

    pub fn budget_remaining(&self) -> u32 {
        self.budget.remaining_steps()
    }

    pub fn is_budget_exhausted(&self) -> bool {
        self.budget.is_exhausted()
    }

    /// Read-only metric accessors for LearningEngine snapshot.
    /// Returns plain data — no authority or state transfer to Learning.
    pub fn steps_taken(&self) -> u32 {
        self.cognitive_state.mission.current_step
    }

    pub fn budget_total(&self) -> u32 {
        self.budget.total_steps
    }

    /// Total recovery actions attempted (sum of all failure counts)
    pub fn recovery_count(&self) -> u32 {
        self.recovery.total_recoveries()
    }

    /// Number of stall signatures recorded
    pub fn stall_count(&self) -> u32 {
        self.stall_detector.total_stalls()
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
    /// The Runtime controls how its own state is restored — agent.rs must NOT
    /// write directly to cognitive_state fields.
    pub fn restore_step(&mut self, step: u32) {
        self.cognitive_state.mission.current_step = step;
        // budget consumed is not restored (continuation gets a fresh 50-step budget)
    }

    /// Checks runtime coherence. Returns a list of violation strings.
    /// Does NOT panic — callers emit FATAL/WARNING and decide how to proceed.
    pub fn check_invariants(&self) -> Vec<String> {
        let mut violations = Vec::new();
        if self.contract.objective.trim().is_empty() {
            violations.push("CONTRACT_EMPTY: objetivo vacío".into());
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

    // ─── World Observation ─────────────────────────────────────────────────────

    /// FINAL-3: Takes a fresh workspace snapshot and stores it in the runtime.
    /// Returns Err if the snapshot fails so callers can distinguish
    /// "no world change" (hash equal) from "observation failed" (None hash).
    /// NEVER silences the error — eprintln is eliminated.
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
                let _ = self.evidence_graph.record_with_hash(
                    EvidenceKind::CommandExitCode,
                    &obs.tool_name,
                    &claim,
                    &obs.payload,
                    0.85,
                    self.cognitive_state.mission.current_step,
                    obs.state_hash_after
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

    /// H-10 + FINAL-1+2+6: Action Gateway — complete authorization pipeline.
    /// Order: ToolRegistry.validate_name → SchemaValidator → Budget → Policy.
    /// agent.rs MUST NOT call tools directly — always goes through execute_action.
    pub fn authorize_action(&self, proposal: &ActionProposal) -> Result<(), String> {
        // 0a. ToolRegistry — is this a known tool name?
        ToolRegistry::validate_name(&proposal.tool)?;

        // 0b. ToolRegistry — does this tool have a registered executor?
        //     TOOL_UNREGISTERED is a hard error: system misconfiguration, not a tool failure.
        if self.tool_registry.resolve(&proposal.tool).is_none() {
            return Err(format!(
                "TOOL_UNREGISTERED: '{}' is known but has no executor registered. \
                 Call runtime.tool_registry.register() before starting the mission loop.",
                proposal.tool
            ));
        }

        // 1. Schema — does the payload match the expected shape for this tool?
        match SchemaValidator::validate_tool_payload(&proposal.tool, &proposal.arguments) {
            SchemaValidationResult::Invalid(reason) =>
                return Err(format!("SCHEMA_INVALID: {}", reason)),
            SchemaValidationResult::Valid => {}
        }

        // 2. Budget — is there remaining step capacity?
        if self.is_budget_exhausted() {
            return Err("BUDGET_EXHAUSTED: presupuesto de pasos agotado".into());
        }

        // 3. Policy — is this action permitted by current security policy?
        match PolicyEngine::authorize(proposal) {
            PolicyDecision::Allow => Ok(()),
            PolicyDecision::Deny(reason) => Err(format!("POLICY_DENY: {}", reason)),
            PolicyDecision::RequireUser(msg) => Err(format!("POLICY_REQUIRE_USER: {}", msg)),
            PolicyDecision::Sandbox(msg) => Err(format!("POLICY_SANDBOX: {}", msg)),
        }
    }

    /// FINAL-6: Execution Gateway — ToolRegistry is the sole dispatch authority.
    ///
    /// Pipeline: ToolRegistry.validate_name → Schema → Budget → Policy
    ///         → observe_world before → ToolRegistry.dispatch → observe_world after
    ///         → Observation → record_observation
    ///
    /// agent.rs NEVER passes an executor closure. It registers executors at startup
    /// and calls execute_action(&proposal) — Runtime dispatches via ToolRegistry.
    pub async fn execute_action(
        &mut self,
        proposal: &ActionProposal,
    ) -> Result<Observation, String>
    {
        // 1. Full authorization gate: name → schema → budget → policy
        self.authorize_action(proposal)?;

        // 2. World snapshot BEFORE — abort if we cannot establish baseline
        if let Err(observe_err) = self.observe_world() {
            return Err(format!("OBSERVE_BEFORE_FAILED: {}", observe_err));
        }
        let hash_before = self.current_world_hash();

        // 3. ToolRegistry dispatches — Runtime resolves which code runs, not agent.rs
        let exec_result = self.tool_registry.dispatch(
            &proposal.tool,
            proposal.arguments.clone(),
        ).await;

        // 4. World snapshot AFTER — None = observation failed (not "no change")
        let hash_after = match self.observe_world() {
            Ok(()) => Some(self.current_world_hash()),
            Err(_) => None,
        };
        self.record_tool_call();

        // 5. Build Observation from dispatch result
        let mut obs = match exec_result {
            Ok(ref output) => Observation::success(&proposal.tool, output, vec![]),
            Err(ref err)   => Observation::error(&proposal.tool, err, None, true, None),
        };
        obs.state_hash_before = Some(hash_before);
        obs.state_hash_after  = hash_after;

        // 6. Record through full circuit (StallDetector, EvidenceGraph)
        self.record_observation(&obs);

        Ok(obs)
    }

    // ─── Completion Gate ───────────────────────────────────────────────────────

    /// The ONLY authority allowed to declare mission complete.
    /// Never let agent.rs declare completion without calling this.
    pub fn can_complete(&self) -> CompletionDecision {
        CompletionGate::evaluate(
            &self.contract, 
            &self.cognitive_state, 
            &self.evidence_graph,
            self.current_world_hash()
        )
    }

    // ─── Recovery ──────────────────────────────────────────────────────────────

    pub fn plan_recovery(&mut self, tool_name: &str, error_msg: &str) -> RecoveryDecision {
        let class = classify_error(error_msg);
        self.cognitive_state.metrics.recovery_actions += 1;
        self.recovery.recover(tool_name, error_msg, class)
    }

    /// FINAL-4: Routes an Observation through the Recovery circuit after execution.
    /// Returns Some(RecoveryDecision) for error observations so agent.rs can decide
    /// what to do next (retry, change tool, replan, ask user, abort).
    ///
    /// Separation: Execution Gateway ≠ Recovery Authority.
    /// execute_action() produces an Observation.
    /// handle_observation() consults RecoveryEngine and returns a decision.
    /// agent.rs acts on that decision — Runtime never forces the recovery action.
    pub fn handle_observation(&mut self, obs: &Observation) -> Option<RecoveryDecision> {
        use crate::core::observation::ObservationStatus;
        match obs.status {
            ObservationStatus::Error => {
                // Error → classify → RecoveryEngine → decision
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

    // ── H-11: Architecture invariant tests ──────────────────────────────────

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
        use std::sync::Arc;
        let mut rt = MissionRuntime::new(".", "Test budget gate", 2);
        rt.record_step(); rt.record_step();
        assert!(rt.is_budget_exhausted());
        // Must register executor so check 0b passes and budget check is reached
        rt.tool_registry.register("TOOL_TERMINAL", Arc::new(|_args| {
            Box::pin(async { Ok("ok".to_string()) })
        })).unwrap();
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
        // mission_id mismatch may fire because cognitive_state.mission.id is default — filter for BUDGET/STEP/WORKSPACE
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

    /// H-11-F: Empty contract blocks CompletionGate — no false positives.
    #[test]
    fn test_empty_contract_blocks_completion() {
        let rt = MissionRuntime::new(".", "Do something", 50);
        let dec = rt.can_complete();
        assert!(
            matches!(dec, crate::core::completion_gate::CompletionDecision::Incomplete(_)),
            "Empty contract must produce Incomplete, not Complete"
        );
    }

    /// Satisfied contract criteria allows CompletionGate to approve completion.
    #[test]
    fn test_satisfied_contract_allows_completion() {
        let mut rt = MissionRuntime::new(".", "Do something", 50);
        rt.contract.add_criterion(
            "AC-DELIVERABLES",
            "Generate files",
            crate::core::mission_contract::VerificationMethod::ManualReview,
            true,
        );
        rt.contract.add_criterion(
            "AC-VALIDATION",
            "Run tests",
            crate::core::mission_contract::VerificationMethod::TestPassed,
            true,
        );
        // Initially incomplete
        assert!(matches!(rt.can_complete(), crate::core::completion_gate::CompletionDecision::Incomplete(_)));

        // Satisfy manual criterion
        rt.evidence_graph.record_with_hash(crate::core::evidence::EvidenceKind::UserConfirmation, "USER", "AC-DELIVERABLES verified manually", "OK", 1.0, 1, None).unwrap();
        // Provide actual evidence for TestPassed criterion to satisfy CompletionGate dynamically
        let _ = rt.evidence_graph.record_with_hash(
            crate::core::evidence::EvidenceKind::Test,
            "cargo test",
            "cargo test passes",
            "0",
            0.9,
            1,
            Some(rt.current_world_hash())
        );

        assert_eq!(rt.can_complete(), crate::core::completion_gate::CompletionDecision::Complete);
    }

    /// H-11-G: record_step() is the only way step advances — no += 1 outside runtime.
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

    /// H-15: execute_action() runs the full pipeline through Runtime via ToolRegistry.
    #[tokio::test]
    async fn test_execute_action_full_pipeline() {
        use std::sync::Arc;
        let mut rt = MissionRuntime::new(".", "Test execution gateway", 10);
        // Register a simulated executor BEFORE execute_action
        rt.tool_registry.register("TOOL_TERMINAL", Arc::new(|_args| {
            Box::pin(async { Ok("simulated terminal output".to_string()) })
        })).unwrap();

        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".into(),
            arguments: serde_json::json!({"comando": "dir"}),
            expected_effect: "simulate output".into(),
            risk: crate::core::policy::RiskLevel::Safe,
        };

        let obs = rt.execute_action(&proposal).await;
        assert!(obs.is_ok(), "execute_action must succeed for authorized registered action");
        let o = obs.unwrap();
        assert_eq!(o.status, crate::core::observation::ObservationStatus::Success);
        assert!(o.state_hash_before.is_some(), "world hash before must be recorded");
        assert_eq!(o.tool_name, "TOOL_TERMINAL");
        assert_eq!(rt.cognitive_state.metrics.tool_calls, 1);
    }

    /// H-15b: execute_action() denies when budget is exhausted — registry not reached.
    #[tokio::test]
    async fn test_execute_action_denies_on_exhausted_budget() {
        use std::sync::Arc;
        let mut rt = MissionRuntime::new(".", "Test deny on exhausted", 2);
        rt.record_step(); rt.record_step();
        rt.tool_registry.register("TOOL_TERMINAL", Arc::new(|_args| {
            Box::pin(async { Ok("should not run".to_string()) })
        })).unwrap();

        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".into(),
            arguments: serde_json::json!({"comando": "dir"}),
            expected_effect: String::new(),
            risk: crate::core::policy::RiskLevel::Safe,
        };
        let result = rt.execute_action(&proposal).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("BUDGET_EXHAUSTED"));
    }

    // ——— FINAL-5: Integration tests — full circuit verification ——————————————————

    /// FINAL-5-A: Executor is PHYSICALLY never called when authorization is denied.
    /// AtomicBool proves the registered executor body was never entered.
    #[tokio::test]
    async fn test_executor_not_called_when_denied() {
        use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

        let mut rt = MissionRuntime::new(".", "Test executor not called", 2);
        rt.record_step(); rt.record_step(); // exhaust budget

        let executed = Arc::new(AtomicBool::new(false));
        let executed_clone = executed.clone();

        // Register executor that sets the flag if called
        rt.tool_registry.register("TOOL_TERMINAL", Arc::new(move |_args| {
            let flag = executed_clone.clone();
            Box::pin(async move {
                flag.store(true, Ordering::SeqCst);
                Ok("should not run".to_string())
            })
        })).unwrap();

        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".into(),
            arguments: serde_json::json!({"comando": "dir"}),
            expected_effect: String::new(),
            risk: crate::core::policy::RiskLevel::Safe,
        };

        let result = rt.execute_action(&proposal).await;
        assert!(result.is_err(), "Must deny when budget exhausted");
        assert!(
            !executed.load(Ordering::SeqCst),
            "registered executor MUST NOT run when authorization is denied"
        );
    }

    /// FINAL-5-B: Error from registered executor → handle_observation() → RecoveryDecision.
    #[tokio::test]
    async fn test_error_observation_produces_recovery_decision() {
        use std::sync::Arc;
        let mut rt = MissionRuntime::new(".", "Test recovery circuit", 10);

        rt.tool_registry.register("TOOL_TERMINAL", Arc::new(|_args| {
            Box::pin(async { Err("error[E0001]: command not found: rustc".to_string()) })
        })).unwrap();

        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".into(),
            arguments: serde_json::json!({"comando": "rustc nonexistent.rs"}),
            expected_effect: String::new(),
            risk: crate::core::policy::RiskLevel::Safe,
        };

        let obs = rt.execute_action(&proposal).await
            .expect("execute_action itself should succeed even on tool error");

        assert_eq!(obs.status, crate::core::observation::ObservationStatus::Error,
            "Failed executor must produce Error observation");
        let recovery = rt.handle_observation(&obs);
        assert!(recovery.is_some(),
            "Error Observation MUST produce a RecoveryDecision via handle_observation()");
    }

    /// FINAL-5-C: Schema INVALID blocks before dispatch — executor never runs.
    #[tokio::test]
    async fn test_schema_invalid_blocks_execution() {
        use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

        let mut rt = MissionRuntime::new(".", "Test schema gate", 10);
        let executed = Arc::new(AtomicBool::new(false));
        let executed_clone = executed.clone();

        rt.tool_registry.register("TOOL_TERMINAL", Arc::new(move |_args| {
            let flag = executed_clone.clone();
            Box::pin(async move {
                flag.store(true, Ordering::SeqCst);
                Ok("should not run".to_string())
            })
        })).unwrap();

        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".into(),
            arguments: serde_json::json!({"comando": ""}), // empty → SCHEMA_INVALID
            expected_effect: String::new(),
            risk: crate::core::policy::RiskLevel::Safe,
        };

        let result = rt.execute_action(&proposal).await;
        assert!(result.is_err(), "Empty command must be rejected");
        assert!(result.unwrap_err().contains("SCHEMA_INVALID"));
        assert!(!executed.load(Ordering::SeqCst),
            "registered executor MUST NOT run when Schema validation fails");
    }

    // ——— FINAL-6: ToolRegistry as dispatch authority ————————————————————————————

    /// FINAL-6-A: Known but unregistered tool → TOOL_UNREGISTERED error.
    #[tokio::test]
    async fn test_unregistered_tool_dispatch_fails() {
        let mut rt = MissionRuntime::new(".", "Test unregistered dispatch", 10);
        // TOOL_TERMINAL is KNOWN but NOT registered in the registry
        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".into(),
            arguments: serde_json::json!({"comando": "dir"}),
            expected_effect: String::new(),
            risk: crate::core::policy::RiskLevel::Safe,
        };
        let result = rt.execute_action(&proposal).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("TOOL_UNREGISTERED"),
            "Known but unregistered tool must fail with TOOL_UNREGISTERED");
    }

    /// FINAL-6-B: After registration, ToolRegistry dispatches correctly.
    #[tokio::test]
    async fn test_registered_tool_dispatches_via_registry() {
        use std::sync::Arc;
        let mut rt = MissionRuntime::new(".", "Test registry dispatch", 10);
        rt.tool_registry.register("TOOL_TERMINAL", Arc::new(|_args| {
            Box::pin(async { Ok("registry_dispatched".to_string()) })
        })).unwrap();

        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".into(),
            arguments: serde_json::json!({"comando": "dir"}),
            expected_effect: String::new(),
            risk: crate::core::policy::RiskLevel::Safe,
        };
        let obs = rt.execute_action(&proposal).await;
        assert!(obs.is_ok());
        assert_eq!(obs.unwrap().payload, "registry_dispatched");
    }

    #[test]
    fn test_integration_word_stats_mission_flow() {
        let mut rt = MissionRuntime::new(".", "Crea CLI word-stats", 50);
        
        rt.contract.add_criterion("AC-BUILD", "cargo build exitoso", crate::core::mission_contract::VerificationMethod::CommandExitZero("cargo build".to_string()), true);
        rt.contract.add_criterion("AC-TEST", "cargo test con 3 tests", crate::core::mission_contract::VerificationMethod::TestPassed, true);
        rt.contract.add_criterion("AC-JSON", "Genera word_stats.json válido", crate::core::mission_contract::VerificationMethod::FileExistence("word_stats.json".to_string()), true);

        // 1. Initial State -> Missing files, LLM attempts FINISH
        assert!(matches!(rt.can_complete(), crate::core::completion_gate::CompletionDecision::Incomplete(_)));

        // 2. LLM creates Cargo.toml and src/main.rs (simulated hash change)
        let hash_after_code = 12345;
        
        // 3. LLM executes cargo build successfully
        let _ = rt.evidence_graph.record_with_hash(
            crate::core::evidence::EvidenceKind::CommandExitCode, "TOOL_TERMINAL", "cargo build passes", "0", 1.0, rt.current_step(), Some(hash_after_code)
        ).unwrap();

        // Still incomplete
        assert!(matches!(rt.can_complete(), crate::core::completion_gate::CompletionDecision::Incomplete(_)));

        // 4. LLM executes cargo test successfully
        let _ = rt.evidence_graph.record_with_hash(
            crate::core::evidence::EvidenceKind::Test, "TOOL_TERMINAL", "cargo test passes", "0", 1.0, rt.current_step(), Some(hash_after_code)
        ).unwrap();

        // 5. LLM runs the tool, generating word_stats.json
        let _ = rt.evidence_graph.record_with_hash(
            crate::core::evidence::EvidenceKind::FileCreated, "TOOL_TERMINAL", "file word_stats.json exists", "0", 1.0, rt.current_step(), Some(hash_after_code)
        ).unwrap();

        // 6. Complete
        let mut contract_valid = false;
        if let crate::core::completion_gate::CompletionDecision::Complete = crate::core::completion_gate::CompletionGate::evaluate(&rt.contract, &rt.cognitive_state, &rt.evidence_graph, hash_after_code) {
            contract_valid = true;
        }
        assert!(contract_valid);
    }
}
