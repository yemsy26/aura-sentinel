//! trajectory.rs — AL-v2 State & Action Trajectory Representation
//!
//! Models the sequence of operational state transitions during a mission:
//! State_i -> Action(Strategy, Tool) -> State_{i+1} -> Outcome.
//! Enables the system to learn which recovery patterns actually work to escape
//! error states, rather than only matching static prompts to model IDs.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use crate::core::learning::signature::StateSignature;
use crate::core::learning::strategy::StrategyKind;
use crate::core::learning::outcome::LearningOutcome;

/// An individual step in an execution trajectory
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryStep {
    /// Step index within the mission
    pub step: u32,
    /// Operational state snapshot before the action
    pub from_state: StateSignature,
    /// Strategy applied in this step
    pub strategy: StrategyKind,
    /// Specific tool invoked
    pub tool: String,
    /// Whether the action succeeded at execution level
    pub success: bool,
    /// Error encountered, if any
    pub error_encountered: Option<String>,
    /// Operational state snapshot after the action (if evaluated)
    pub to_state: Option<StateSignature>,
}

/// A validated recovery sequence extracted from a trajectory
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecoverySequence {
    /// The error class that triggered the need for recovery
    pub trigger_error: String,
    /// Strategy that successfully resolved the error
    pub strategy: StrategyKind,
    /// Tool that produced the resolution
    pub tool: String,
    /// The signature of the state that was recovered from
    pub initial_state: StateSignature,
    /// The signature of the resolved state
    pub resolved_state: StateSignature,
}

/// Full operational trajectory of a mission
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trajectory {
    /// Unique mission identifier
    pub mission_id: String,
    /// Hash of the task fingerprint
    pub task_fingerprint_hash: String,
    /// Ordered sequence of steps taken
    pub steps: Vec<TrajectoryStep>,
    /// Final mission outcome
    pub outcome: LearningOutcome,
    /// Elapsed execution time in milliseconds
    pub total_duration_ms: u64,
}

impl Trajectory {
    /// Creates a new empty trajectory for a mission
    pub fn new(mission_id: impl Into<String>, task_fingerprint_hash: impl Into<String>) -> Self {
        Self {
            mission_id: mission_id.into(),
            task_fingerprint_hash: task_fingerprint_hash.into(),
            steps: Vec::new(),
            outcome: LearningOutcome::Success,
            total_duration_ms: 0,
        }
    }

    /// Records a step in the trajectory
    pub fn record_step(&mut self, step: TrajectoryStep) {
        self.steps.push(step);
    }

    /// Sets the final outcome and elapsed duration
    pub fn finalize(&mut self, outcome: LearningOutcome, duration_ms: u64) {
        self.outcome = outcome;
        self.total_duration_ms = duration_ms;
    }

    /// Returns the total number of steps in this trajectory
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }

    /// Extracts all successful recovery sequences from the trajectory where an
    /// error state was directly resolved into a clean state by an action.
    pub fn extract_recoveries(&self) -> Vec<RecoverySequence> {
        let mut recoveries = Vec::new();

        for step in &self.steps {
            if let (Some(err), true, Some(ref to_st)) = (
                &step.from_state.last_error_class,
                step.success,
                &step.to_state,
            ) {
                // Resolved if target state has no active error or has lower failure counts
                if to_st.last_error_class.is_none() || (!to_st.progress_stalled && step.from_state.progress_stalled) {
                    recoveries.push(RecoverySequence {
                        trigger_error: err.clone(),
                        strategy: step.strategy.clone(),
                        tool: step.tool.clone(),
                        initial_state: step.from_state.clone(),
                        resolved_state: to_st.clone(),
                    });
                }
            }
        }

        recoveries
    }
}
