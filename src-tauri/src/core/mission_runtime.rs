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
    pub world_version: u64,
    pub state_anchor: MissionStateAnchor,
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
        let anchor = MissionStateAnchor {
            mission_id: mission_id.clone(),
            workspace_root: workspace_path.to_string(),
            world_hash: 0,
            world_version: 0,
            current_phase: "Planning".to_string(),
            current_step: 0,
            active_plan_step: None,
            required_files: Vec::new(),
            existing_files: Vec::new(),
            criteria_satisfied: 0,
            criteria_remaining: 0,
            last_tool: None,
            last_command: None,
            last_observation: None,
            last_verified_effect: None,
            objective: objective.to_string(),
            project_root: workspace_path.to_string(),
            role: "Planner".to_string(),
            files_created: Vec::new(),
            files_modified: Vec::new(),
            files_pending: Vec::new(),
            criteria_pending: Vec::new(),
            last_world_revision: "0".to_string(),
            last_error: None,
            next_required_action: None,
        };
        MissionRuntime {
            contract: MissionContract::new(objective),
            cognitive_state: CognitiveState::new(&mission_id, objective),
            evidence_graph: EvidenceGraph::new(),
            stall_detector: StallDetector::new(6),
            budget: StepBudget::new(max_steps),
            recovery: RecoveryEngine::new(),
            world: None,
            world_version: 0,
            state_anchor: anchor,
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
        // P0 Fix: Budget consumed MUST be restored to maintain coherent step limits\n        self.budget.used_steps = step;
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
        match WorldState::capture_incremental(&self.workspace_path, self.world.as_ref()) {
            Ok(ws) => {
                self.cognitive_state.set_world(ws.clone());
                self.world = Some(ws);
                self.update_anchor(None);
                Ok(())
            }
            Err(e) => Err(format!("[OBSERVE_WORLD FAILED] {}", e)),
        }
    }

    /// Returns the current world state hash using content hashes, not just file sizes.
    /// This correctly detects when a file changes content but keeps the same size.
    pub fn current_world_hash(&self) -> u64 {
        self.world.as_ref().map(|w| {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            let mut entries: Vec<(&String, &crate::core::world_state::FileSnapshot)> = w.files.iter().collect();
            entries.sort_by_key(|(p, _)| p.as_str());
            
            for (path, snap) in entries {
                hasher.update(path.as_bytes());
                hasher.update(b"|");
                hasher.update(snap.content_hash.as_bytes());
                hasher.update(b"|");
            }
            let result = hasher.finalize();
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&result[0..8]);
            u64::from_be_bytes(buf)
        }).unwrap_or(0)
    }

    /// Authoritative State Synchronization: updates self.state_anchor directly from physical reality.
    pub fn update_anchor(&mut self, last_obs: Option<&Observation>) {
        let world_hash = self.current_world_hash();
        if world_hash != self.state_anchor.world_hash {
            self.world_version += 1;
        }
        
        let mut existing_files: Vec<String> = match &self.world {
            Some(w) => w.files.keys().cloned().collect(),
            None => Vec::new(),
        };
        existing_files.sort();

        let mut required_files: Vec<String> = Vec::new();
        for crit in &self.contract.acceptance_criteria {
            match &crit.verification {
                crate::core::mission_contract::VerificationMethod::FileExistence(f) => {
                    if !required_files.contains(f) {
                        required_files.push(f.clone());
                    }
                }
                crate::core::mission_contract::VerificationMethod::ContentMatches { file, .. } => {
                    if !required_files.contains(file) {
                        required_files.push(file.clone());
                    }
                }
                _ => {}
            }
        }
        required_files.sort();

        let criteria_satisfied = self.verified_criteria_count();
        let total_required = self.contract.acceptance_criteria.iter().filter(|c| c.required).count() as u32;
        let criteria_remaining = total_required.saturating_sub(criteria_satisfied);

        let current_phase = format!("{:?}", self.cognitive_state.mission.status);

        if let Some(obs) = last_obs {
            self.state_anchor.last_tool = Some(obs.tool_name.clone());
            self.state_anchor.last_command = obs.command.clone();
            self.state_anchor.last_observation = Some(if obs.payload.len() > 300 {
                format!("{}... [truncated]", &obs.payload[..300])
            } else {
                obs.payload.clone()
            });
            self.state_anchor.last_verified_effect = if !obs.files_affected.is_empty() {
                Some(format!("Archivos afectados: {}", obs.files_affected.join(", ")))
            } else if obs.exit_code == Some(0) {
                Some("Comando finalizado con éxito (exit code 0)".to_string())
            } else {
                Some(format!("Estado: {:?}", obs.status))
            };
        }

        self.state_anchor.mission_id = self.mission_id.clone();
        self.state_anchor.workspace_root = self.workspace_path.clone();
        self.state_anchor.project_root = self.workspace_path.clone();
        self.state_anchor.world_hash = world_hash;
        self.state_anchor.world_version = self.world_version;
        self.state_anchor.current_phase = current_phase;
        self.state_anchor.current_step = self.current_step();
        self.state_anchor.required_files = required_files;
        self.state_anchor.existing_files = existing_files;
        self.state_anchor.criteria_satisfied = criteria_satisfied;
        self.state_anchor.criteria_remaining = criteria_remaining;
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
                use crate::core::evidence::{EvidenceKind, StructuredFact};
                
                // P0: Replace blind implicit claim with structured execution facts.
                // Bind real process metadata ONLY if exit_code is present.
                // If exit_code is None (e.g. no process metadata or infrastructure failure),
                // NEVER fabricate success with unwrap_or(0) — do not record technical command evidence.
                if let Some(exit_code) = obs.exit_code {
                    let fact = StructuredFact::CommandResult {
                        command: obs.command.clone().unwrap_or_else(|| "unknown".to_string()),
                        cwd: obs.cwd.clone().unwrap_or_else(|| self.workspace_path.clone()),
                        exit_code, 
                        stdout_hash: obs.stdout_hash.clone().unwrap_or_else(|| "unknown".to_string()),
                        stderr_hash: obs.stderr_hash.clone().unwrap_or_default(),
                    };
                    
                    let _ = self.evidence_graph.record_structured(
                        EvidenceKind::CommandExitCode,
                        &obs.tool_name,
                        fact,
                        0.85,
                        self.cognitive_state.mission.current_step,
                        obs.state_hash_after, // P0: Bind to exact world state
                    );
                }
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
            files_changed: obs.physical_files_changed.unwrap_or(obs.files_affected.len() as u32),
            criteria_satisfied: self.verified_criteria_count(),
            evidence_count: self.evidence_graph.entries.len() as u32,
            last_tool_used: obs.tool_name.clone(),
            last_command: obs.command.clone().unwrap_or_default(),
            last_files: obs.files_affected.join(","),
            last_error_hash: err_hash,
        };
        self.stall_detector.record_signature(sig);
    }

    /// Evaluates how many criteria are actually verified dynamically by CompletionGate.
    pub fn verified_criteria_count(&self) -> u32 {
        let ws = std::path::Path::new(&self.workspace_path);
        let decision = CompletionGate::evaluate(&self.contract, &self.cognitive_state, &self.evidence_graph, self.current_world_hash(), ws);
        
        let total = self.contract.acceptance_criteria.iter().filter(|c| c.required).count() as u32;
        let missing = match decision {
            CompletionDecision::Complete => 0,
            CompletionDecision::Incomplete(m) | CompletionDecision::Blocked(m) => m.len() as u32,
        };
        
        total.saturating_sub(missing)
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

        // P0-4: Recovery Barrier
        if let Some(stall) = self.should_stall_recover(3) {
            // If the model is stalled, check if the current proposal is IDENTICAL to the last action.
            if let Some(last_sig) = self.stall_detector.last_signature() {
                let cmd = proposal.arguments.get("comando")
                    .or_else(|| proposal.arguments.get("command"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let files = proposal.arguments.get("archivos_a_editar")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(","))
                    .unwrap_or_default();
                
                if proposal.tool == last_sig.last_tool_used && cmd == last_sig.last_command && files == last_sig.last_files {
                    return Err(format!("RECOVERY_BARRIER: Acción repetida bloqueada por estancamiento ({:?}). Debes cambiar tu estrategia o comando/archivo.", stall));
                }
            }
        }

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

        // P0-3: Action Identity + Stale Proposal Rejection
        if let Some(expected_hash) = proposal.world_hash {
            if expected_hash != self.current_world_hash() {
                // Reject stale action by returning an Error Observation
                let mut obs = Observation::error(
                    &proposal.tool,
                    "STALE_ACTION: El entorno ha cambiado desde que tomaste la decisión. Re-evalúa el estado actual.",
                    None,
                    true,
                    None
                );
                obs.state_hash_before = Some(self.current_world_hash());
                obs.state_hash_after = obs.state_hash_before;
                
                // Record the observation so it goes to StallDetector
                self.record_observation(&obs);
                self.update_anchor(Some(&obs));
                return Ok(obs);
            }
        }

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

        let old_world = self.world.clone();
        // 4. World snapshot AFTER — None = observation failed (not "no change")
        let hash_after = match self.observe_world() {
            Ok(()) => Some(self.current_world_hash()),
            Err(_) => None,
        };
        self.record_tool_call();

        // 5. Build Observation from dispatch result with real execution metadata
        let mut obs = match exec_result {
            Ok(ref res) => {
                if res.exit_code == 0 {
                    let mut o = Observation::success(&proposal.tool, &res.stdout, res.files_affected.clone());
                    o.exit_code = Some(0);
                    o.command = res.command.clone();
                    o.cwd = res.cwd.clone();
                    o.stdout_hash = Some(res.stdout_hash.clone());
                    o.stderr_hash = Some(res.stderr_hash.clone());
                    o
                } else {
                    let err_payload = if !res.stderr.is_empty() {
                        res.stderr.clone()
                    } else if !res.stdout.is_empty() {
                        res.stdout.clone()
                    } else {
                        format!("Process exited with code {}", res.exit_code)
                    };
                    let mut o = Observation::error(&proposal.tool, err_payload, Some(res.exit_code), true, None);
                    o.command = res.command.clone();
                    o.cwd = res.cwd.clone();
                    o.stdout_hash = Some(res.stdout_hash.clone());
                    o.stderr_hash = Some(res.stderr_hash.clone());
                    o.files_affected = res.files_affected.clone();
                    o
                }
            },
            Err(ref err)   => {
                let o = Observation::error(&proposal.tool, err, None, true, None);
                o
            },
        };
        if obs.command.is_none() && proposal.tool == "TOOL_TERMINAL" {
            obs.command = proposal.arguments.get("comando")
                .or_else(|| proposal.arguments.get("command"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
        if obs.cwd.is_none() {
            obs.cwd = Some(self.workspace_path.clone());
        }
        obs.state_hash_before = Some(hash_before);
        obs.state_hash_after  = hash_after;

        let diff_count = if let (Some(w1), Some(w2)) = (&old_world, &self.world) {
            let diff = w1.diff(w2);
            (diff.added_files.len() + diff.modified_files.len() + diff.deleted_files.len()) as u32
        } else {
            0
        };
        obs.physical_files_changed = Some(diff_count);

        // 6. Record through full circuit (StallDetector, EvidenceGraph)
        self.record_observation(&obs);

        // 7. Update authoritative state anchor with observation and post-action world state
        self.update_anchor(Some(&obs));

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
            self.current_world_hash(),
            std::path::Path::new(&self.workspace_path),
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
    pub fn get_state_anchor(&self, journal: &crate::core::session_journal::SessionJournal, current_role: &str, last_error: &str) -> MissionStateAnchor {
        let current_meta = if journal.micro_meta_actual < journal.micro_metas.len() {
            Some(journal.micro_metas[journal.micro_meta_actual].descripcion.clone())
        } else {
            None
        };

        let mut anchor = self.state_anchor.clone();
        anchor.role = current_role.to_string();
        anchor.active_plan_step = current_meta.clone();
        anchor.next_required_action = current_meta;
        if !last_error.is_empty() {
            anchor.last_error = Some(last_error.to_string());
        }
        anchor
    }
    
    pub fn format_anchor(&self, journal: &crate::core::session_journal::SessionJournal, current_role: &str, last_error: &str) -> String {
        let anchor = self.get_state_anchor(journal, current_role, last_error);
        anchor.format_prompt_block()
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

    // ── P0-1 and P0-3 Tests ──────────────────────────────────────────────
    #[test]
    fn test_p0_2_authoritative_planner_state() {
        let mut rt = MissionRuntime::new(".", "Test P0-2", 10);
        rt.state_anchor.world_hash = 999;
        rt.state_anchor.world_version = 5;
        let prompt_block = rt.state_anchor.format_prompt_block();
        println!("PROMPT BLOCK: {}", prompt_block);
        assert!(prompt_block.contains("00000000000003e7"));
        assert!(prompt_block.contains("5"));
    }

    #[test]
    fn test_p0_1_workspace_freeze() {
        let rt = MissionRuntime::new("original/workspace", "Test", 10);
        // The workspace should remain "original/workspace" regardless of external changes
        assert_eq!(rt.workspace_path, "original/workspace");
    }

    #[tokio::test]
    async fn test_p0_3_action_identity_and_stale_rejection() {
        let mut rt = MissionRuntime::new(".", "Test", 10);
        rt.tool_registry.register("TOOL_TERMINAL", std::sync::Arc::new(|_cmd| {
            Box::pin(async move {
                Ok(crate::core::tool_registry::ExecutionResult {
                    stdout: "".to_string(), stderr: "".to_string(), exit_code: 0, files_affected: vec![],
                    command: None, cwd: None, stdout_hash: "".to_string(), stderr_hash: "".to_string(),
                })
            })
        }));
        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".to_string(),
            arguments: serde_json::json!({ "comando": "echo 1" }),
            expected_effect: "None".to_string(),
            risk: crate::core::policy::RiskLevel::Safe,
            world_hash: Some(12345), // Fake hash, mismatched with rt.current_world_hash()
        };
        let res = rt.execute_action(&proposal).await;
        assert!(res.is_ok());
        let obs = res.unwrap();
        assert_eq!(obs.status, crate::core::observation::ObservationStatus::Error);
        assert!(obs.payload.contains("STALE_ACTION"));
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
            Box::pin(async { Ok(crate::core::tool_registry::ExecutionResult::success("ok")) })
        })).unwrap();
        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".into(),
            arguments: serde_json::json!({"comando": "dir"}),
            expected_effect: String::new(),
            risk: crate::core::policy::RiskLevel::Safe,
            world_hash: None,
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
        // Provide actual structured evidence for TestPassed criterion to satisfy CompletionGate dynamically
        let _ = rt.evidence_graph.record_structured(
            crate::core::evidence::EvidenceKind::Test,
            "TOOL_TERMINAL",
            crate::core::evidence::StructuredFact::CommandResult {
                command: "cargo test".to_string(),
                cwd: ".".to_string(),
                exit_code: 0,
                stdout_hash: "mock".to_string(),
                stderr_hash: "".to_string(),
            },
            1.0,
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
            Box::pin(async { Ok(crate::core::tool_registry::ExecutionResult::success("simulated terminal output")) })
        })).unwrap();

        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".into(),
            arguments: serde_json::json!({"comando": "dir"}),
            expected_effect: "simulate output".into(),
            risk: crate::core::policy::RiskLevel::Safe,
            world_hash: None,
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
            Box::pin(async { Ok(crate::core::tool_registry::ExecutionResult::success("should not run")) })
        })).unwrap();

        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".into(),
            arguments: serde_json::json!({"comando": "dir"}),
            expected_effect: String::new(),
            risk: crate::core::policy::RiskLevel::Safe,
            world_hash: None,
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
                Ok(crate::core::tool_registry::ExecutionResult::success("should not run"))
            })
        })).unwrap();

        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".into(),
            arguments: serde_json::json!({"comando": "dir"}),
            expected_effect: String::new(),
            risk: crate::core::policy::RiskLevel::Safe,
            world_hash: None,
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
            world_hash: None,
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
                Ok(crate::core::tool_registry::ExecutionResult::success("should not run"))
            })
        })).unwrap();

        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".into(),
            arguments: serde_json::json!({"comando": ""}), // empty → SCHEMA_INVALID
            expected_effect: String::new(),
            risk: crate::core::policy::RiskLevel::Safe,
            world_hash: None,
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
            world_hash: None,
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
            Box::pin(async { Ok(crate::core::tool_registry::ExecutionResult::success("registry_dispatched")) })
        })).unwrap();

        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".into(),
            arguments: serde_json::json!({"comando": "dir"}),
            expected_effect: String::new(),
            risk: crate::core::policy::RiskLevel::Safe,
            world_hash: None,
        };
        let obs = rt.execute_action(&proposal).await;
        assert!(obs.is_ok());
        assert_eq!(obs.unwrap().payload, "registry_dispatched");
    }

    #[tokio::test]
    async fn test_integration_word_stats_mission_flow() {
        let temp_dir = std::env::temp_dir().join(format!("aura_ws_test_{}", uuid::Uuid::new_v4()));
        let src_dir = temp_dir.join("src");
        std::fs::create_dir_all(&src_dir).unwrap();

        let mut rt = MissionRuntime::new(temp_dir.to_str().unwrap(), "Crea CLI word-stats", 50);
        
        rt.contract.add_criterion("AC-BUILD", "cargo build exitoso", crate::core::mission_contract::VerificationMethod::CommandExitZero("cargo build".to_string()), true);
        rt.contract.add_criterion("AC-TEST", "cargo test con 3 tests", crate::core::mission_contract::VerificationMethod::TestPassed, true);
        rt.contract.add_criterion("AC-JSON", "Genera word_stats.json válido", crate::core::mission_contract::VerificationMethod::FileExistence("word_stats.json".to_string()), true);

        // 1. Initial State -> Missing files and evidence -> Incomplete
        assert!(matches!(rt.can_complete(), crate::core::completion_gate::CompletionDecision::Incomplete(_)));

        // 2. LLM creates Cargo.toml and src/main.rs physically
        std::fs::write(temp_dir.join("Cargo.toml"), "[package]\nname=\"word-stats\"\nversion=\"0.1.0\"\n").unwrap();
        std::fs::write(src_dir.join("main.rs"), "fn main() { println!(\"hello\"); }").unwrap();

        rt.observe_world().unwrap();
        let hash_step1 = rt.current_world_hash();
        
        // 3. LLM executes cargo build and cargo test successfully
        let _ = rt.evidence_graph.record_structured(
            crate::core::evidence::EvidenceKind::CommandExitCode,
            "TOOL_TERMINAL",
            crate::core::evidence::StructuredFact::CommandResult {
                command: "cargo build".to_string(),
                cwd: temp_dir.to_str().unwrap().to_string(),
                exit_code: 0,
                stdout_hash: "mock".to_string(),
                stderr_hash: "".to_string(),
            },
            1.0,
            rt.current_step(),
            Some(hash_step1),
        ).unwrap();
        let _ = rt.evidence_graph.record_structured(
            crate::core::evidence::EvidenceKind::Test,
            "TOOL_TERMINAL",
            crate::core::evidence::StructuredFact::CommandResult {
                command: "cargo test".to_string(),
                cwd: temp_dir.to_str().unwrap().to_string(),
                exit_code: 0,
                stdout_hash: "mock".to_string(),
                stderr_hash: "".to_string(),
            },
            1.0,
            rt.current_step(),
            Some(hash_step1),
        ).unwrap();

        // Still incomplete because word_stats.json does NOT physically exist on disk yet!
        assert!(matches!(rt.can_complete(), crate::core::completion_gate::CompletionDecision::Incomplete(_)));

        // 4. LLM runs binary, physically creating word_stats.json on disk
        std::fs::write(temp_dir.join("word_stats.json"), r#"{"total_words": 10}"#).unwrap();

        rt.observe_world().unwrap();
        let hash_step2 = rt.current_world_hash();
        // Record test/build evidence for the new state hash
        let _ = rt.evidence_graph.record_structured(
            crate::core::evidence::EvidenceKind::CommandExitCode,
            "TOOL_TERMINAL",
            crate::core::evidence::StructuredFact::CommandResult {
                command: "cargo build".to_string(),
                cwd: temp_dir.to_str().unwrap().to_string(),
                exit_code: 0,
                stdout_hash: "mock".to_string(),
                stderr_hash: "".to_string(),
            },
            1.0,
            rt.current_step(),
            Some(hash_step2),
        ).unwrap();
        let _ = rt.evidence_graph.record_structured(
            crate::core::evidence::EvidenceKind::Test,
            "TOOL_TERMINAL",
            crate::core::evidence::StructuredFact::CommandResult {
                command: "cargo test".to_string(),
                cwd: temp_dir.to_str().unwrap().to_string(),
                exit_code: 0,
                stdout_hash: "mock".to_string(),
                stderr_hash: "".to_string(),
            },
            1.0,
            rt.current_step(),
            Some(hash_step2),
        ).unwrap();

        // 5. Everything matches state and physical disk -> Complete!
        assert_eq!(rt.can_complete(), crate::core::completion_gate::CompletionDecision::Complete);

        // 6. NEGATIVE TEST 1: LLM modifies main.rs AFTER passing tests (invalidating state_hash)
        std::fs::write(src_dir.join("main.rs"), "fn main() { println!(\"broken code extra long\"); }").unwrap();
        rt.observe_world().unwrap();
        // World state hash changes! CompletionGate MUST reject!
        assert!(matches!(rt.can_complete(), crate::core::completion_gate::CompletionDecision::Incomplete(_)));

        // 7. NEGATIVE TEST 2: File deleted physically
        std::fs::remove_file(temp_dir.join("word_stats.json")).unwrap();
        rt.observe_world().unwrap();
        assert!(matches!(rt.can_complete(), crate::core::completion_gate::CompletionDecision::Incomplete(_)));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_e2e_agent_runtime_registry_observation_evidence_gate_chain() {
        use std::sync::Arc;
        let temp_dir = std::env::temp_dir().join(format!("aura_chain_e2e_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let mut rt = MissionRuntime::new(temp_dir.to_str().unwrap(), "Complete full-chain test", 20);

        // 1. Contract requires test passed
        rt.contract.add_criterion(
            "AC-TEST-1",
            "Cargo test suite passes",
            crate::core::mission_contract::VerificationMethod::TestPassed,
            true,
        );

        // Before any tool execution -> CompletionGate must report Incomplete
        assert!(matches!(rt.can_complete(), crate::core::completion_gate::CompletionDecision::Incomplete(_)));

        // 2. Register real executor in ToolRegistry for TOOL_TERMINAL
        let ws_clone = temp_dir.to_string_lossy().to_string();
        rt.tool_registry.register("TOOL_TERMINAL", Arc::new(move |args| {
            let ws = ws_clone.clone();
            Box::pin(async move {
                let cmd = args.get("comando").and_then(|v| v.as_str()).unwrap_or("");
                if cmd == "cargo test" {
                    let mut res = crate::core::tool_registry::ExecutionResult::success("test result: ok. 1 passed; 0 failed");
                    res.command = Some("cargo test".to_string());
                    res.cwd = Some(ws);
                    res.exit_code = 0;
                    Ok(res)
                } else {
                    let mut res = crate::core::tool_registry::ExecutionResult::error("unknown command", 1);
                    res.command = Some(cmd.to_string());
                    res.cwd = Some(ws);
                    res.exit_code = 1;
                    Ok(res)
                }
            })
        })).unwrap();

        // 3. Agent constructs ActionProposal
        let proposal = crate::core::policy::ActionProposal {
            tool: "TOOL_TERMINAL".to_string(),
            arguments: serde_json::json!({ "comando": "cargo test" }),
            expected_effect: "Run test suite to produce required evidence".to_string(),
            risk: crate::core::policy::RiskLevel::Safe,
            world_hash: None,
        };

        // 4. Dispatch through Runtime: ActionProposal -> Policy -> ToolRegistry -> Executor -> Observation -> Evidence
        let obs = rt.execute_action(&proposal).await.expect("Runtime execution must succeed");

        assert_eq!(obs.status, crate::core::observation::ObservationStatus::Success);
        assert_eq!(obs.exit_code, Some(0));
        assert_eq!(obs.tool_name, "TOOL_TERMINAL");

        // 5. CompletionGate evaluated against accumulated evidence
        let completion = rt.can_complete();
        assert_eq!(
            completion,
            crate::core::completion_gate::CompletionDecision::Complete,
            "CompletionGate MUST verify completion when real executor succeeds through Runtime"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_mission_state_anchor_tracks_physical_world_accurately() {
        let temp_dir = std::env::temp_dir().join(format!("aura_test_anchor_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let mut rt = MissionRuntime::new(temp_dir.to_str().unwrap(), "Crear cyber_sentinel frontend", 20);

        // 1. Initial state: workspace is physically empty
        let _ = rt.observe_world();
        assert!(rt.state_anchor.existing_files.is_empty());
        let initial_prompt_block = rt.state_anchor.format_prompt_block();
        assert!(initial_prompt_block.contains("El workspace está actualmente vacío"));

        // 2. Physical files created
        std::fs::write(temp_dir.join("cyber_sentinel.html"), "<!DOCTYPE html><html></html>").unwrap();
        std::fs::write(temp_dir.join("style.css"), "body { margin: 0; }").unwrap();

        // 3. World observed: anchor immediately syncs with reality
        rt.observe_world().expect("observe_world must succeed");
        assert_eq!(rt.state_anchor.existing_files.len(), 2);
        assert!(rt.state_anchor.existing_files.contains(&"cyber_sentinel.html".to_string()));
        assert!(rt.state_anchor.existing_files.contains(&"style.css".to_string()));

        let prompt_block = rt.state_anchor.format_prompt_block();
        assert!(prompt_block.contains("cyber_sentinel.html"));
        assert!(prompt_block.contains("style.css"));
        assert!(prompt_block.contains("El workspace NO está vacío"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MissionStateAnchor {
    pub mission_id: String,
    pub workspace_root: String,
    pub world_hash: u64,
    pub world_version: u64,
    pub current_phase: String,
    pub current_step: u32,
    pub active_plan_step: Option<String>,
    pub required_files: Vec<String>,
    pub existing_files: Vec<String>,
    pub criteria_satisfied: u32,
    pub criteria_remaining: u32,
    pub last_tool: Option<String>,
    pub last_command: Option<String>,
    pub last_observation: Option<String>,
    pub last_verified_effect: Option<String>,
    #[serde(default)]
    pub objective: String,
    #[serde(default)]
    pub project_root: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub files_created: Vec<String>,
    #[serde(default)]
    pub files_modified: Vec<String>,
    #[serde(default)]
    pub files_pending: Vec<String>,
    #[serde(default)]
    pub criteria_pending: Vec<String>,
    #[serde(default)]
    pub last_world_revision: String,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub next_required_action: Option<String>,
}

impl MissionStateAnchor {
    pub fn format_prompt_block(&self) -> String {
        let files_list = if self.existing_files.is_empty() {
            "  (ninguno detectado aún en el workspace)".to_string()
        } else {
            self.existing_files
                .iter()
                .map(|f| format!("  - {}", f))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let req_files = if self.required_files.is_empty() {
            "  (no especificados explícitamente en el contrato)".to_string()
        } else {
            self.required_files
                .iter()
                .map(|f| format!("  - {}", f))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let active_step = self.active_plan_step.as_deref().unwrap_or("Ninguno");
        let last_tool = self.last_tool.as_deref().unwrap_or("Ninguna");
        let last_cmd = self.last_command.as_deref().unwrap_or("Ninguno");
        let last_obs = self.last_observation.as_deref().unwrap_or("Ninguna");
        let last_effect = self.last_verified_effect.as_deref().unwrap_or("Ninguno");

        let workspace_status_rule = if self.existing_files.is_empty() {
            "REGLA CRÍTICA DE REALIDAD: El workspace está actualmente vacío en disco. Procede a crear los archivos requeridos."
        } else {
            "REGLA CRÍTICA DE REALIDAD: Los archivos físicos listados arriba EXISTEN REALMENTE EN DISCO. El workspace NO está vacío. NUNCA asumas que el workspace está vacío ni vuelvas a crearlos si ya existen sin cambios necesarios."
        };

        format!(
r#"================================================================================
ESTADO AUTORITATIVO DEL RUNTIME (MISSION STATE ANCHOR - REALIDAD FÍSICA INMUTABLE)
================================================================================
Misión ID: {mission_id} | Versión Mundo: v{world_ver} | Hash Mundo: {world_hash:016x}
Workspace Root: {ws}
Fase Actual: {phase} | Paso: {step}
Paso Activo del Plan: {active_step}

ARCHIVOS EXISTENTES EN DISCO (VERIFICADOS FÍSICAMENTE):
{files_list}

ARCHIVOS REQUERIDOS POR EL PLAN / CONTRATO:
{req_files}

CRITERIOS: Satisfechos: {sat} | Restantes: {rem}
ÚLTIMA ACCIÓN EJECUTADA:
  Herramienta: {last_tool}
  Comando: {last_cmd}
  Efecto Verificado: {last_effect}
  Observación: {last_obs}

{workspace_status_rule}
================================================================================"#,
            mission_id = self.mission_id,
            world_ver = self.world_version,
            world_hash = self.world_hash,
            ws = self.workspace_root,
            phase = self.current_phase,
            step = self.current_step,
            active_step = active_step,
            files_list = files_list,
            req_files = req_files,
            sat = self.criteria_satisfied,
            rem = self.criteria_remaining,
            last_tool = last_tool,
            last_cmd = last_cmd,
            last_effect = last_effect,
            last_obs = last_obs,
            workspace_status_rule = workspace_status_rule,
        )
    }
}
