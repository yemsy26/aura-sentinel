//! signature.rs — AL-v2 StateSignature (Cognitive & Operational State Snapshot)
//!
//! Captures a compact, deterministic, structural representation of the agent's
//! situation at any decision step without relying on heavy neural embeddings or vector databases.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// Level of verification reached so far in the mission
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum VerificationLevel {
    None = 0,
    SyntaxOrStatic = 1,
    AutomatedTests = 2,
    FullVerification = 3,
}

impl Default for VerificationLevel {
    fn default() -> Self {
        VerificationLevel::None
    }
}

/// Compact, deterministic snapshot of the operational state during a mission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateSignature {
    /// Hash or ID of the governing TaskFingerprint
    pub task_fingerprint_hash: String,
    /// Active phase index
    pub phase: u32,
    /// Number of existing files modified so far
    pub files_changed: usize,
    /// Number of new files created so far
    pub files_created: usize,
    /// Number of compilation / syntax errors recorded
    pub compile_failures: u32,
    /// Number of test execution failures recorded
    pub test_failures: u32,
    /// Highest verification level reached
    pub verification_level: VerificationLevel,
    /// Number of acceptance criteria confirmed satisfied
    pub criteria_satisfied: usize,
    /// Number of acceptance criteria still pending
    pub criteria_remaining: usize,
    /// Number of error recovery activations triggered
    pub recovery_count: u32,
    /// Number of phase / plan replans triggered
    pub replan_count: u32,
    /// Classification of the most recent error (if any)
    pub last_error_class: Option<String>,
    /// Whether the agent is currently flagged as stalled by the detector
    pub progress_stalled: bool,
}

impl StateSignature {
    /// Computes structural similarity in [0.0, 1.0] between two StateSignatures
    /// using weighted component distance. Deterministic and fast.
    pub fn similarity(&self, other: &Self) -> f32 {
        let mut score = 0.0f32;
        let mut total_weight = 0.0f32;

        // 1. Task similarity component (weight 0.20)
        let w_task = 0.20f32;
        total_weight += w_task;
        if self.task_fingerprint_hash == other.task_fingerprint_hash {
            score += w_task;
        }

        // 2. Error class component (weight 0.25)
        let w_error = 0.25f32;
        total_weight += w_error;
        match (&self.last_error_class, &other.last_error_class) {
            (Some(a), Some(b)) if a == b => score += w_error,
            (None, None) => score += w_error,
            _ => {},
        }

        // 3. Stalled state component (weight 0.15)
        let w_stall = 0.15f32;
        total_weight += w_stall;
        if self.progress_stalled == other.progress_stalled {
            score += w_stall;
        }

        // 4. Failure profile component (weight 0.20)
        let w_failures = 0.20f32;
        total_weight += w_failures;
        let c_diff = (self.compile_failures as i32 - other.compile_failures as i32).abs();
        let t_diff = (self.test_failures as i32 - other.test_failures as i32).abs();
        let fail_penalty = ((c_diff + t_diff) as f32 * 0.2).min(1.0);
        score += w_failures * (1.0 - fail_penalty);

        // 5. Progress / verification alignment (weight 0.20)
        let w_progress = 0.20f32;
        total_weight += w_progress;
        let total_a = (self.criteria_satisfied + self.criteria_remaining).max(1) as f32;
        let ratio_a = self.criteria_satisfied as f32 / total_a;
        let total_b = (other.criteria_satisfied + other.criteria_remaining).max(1) as f32;
        let ratio_b = other.criteria_satisfied as f32 / total_b;
        let ratio_diff = (ratio_a - ratio_b).abs();
        let v_diff = ((self.verification_level as i32) - (other.verification_level as i32)).abs() as f32 / 3.0;
        let progress_sim = (1.0 - (ratio_diff * 0.5 + v_diff * 0.5)).max(0.0);
        score += w_progress * progress_sim;

        (score / total_weight).clamp(0.0, 1.0)
    }
}

/// Fluent builder for constructing StateSignature instances cleanly
#[derive(Default)]
pub struct StateSignatureBuilder {
    task_fingerprint_hash: String,
    phase: u32,
    files_changed: usize,
    files_created: usize,
    compile_failures: u32,
    test_failures: u32,
    verification_level: VerificationLevel,
    criteria_satisfied: usize,
    criteria_remaining: usize,
    recovery_count: u32,
    replan_count: u32,
    last_error_class: Option<String>,
    progress_stalled: bool,
}

impl StateSignatureBuilder {
    pub fn new(task_fingerprint_hash: impl Into<String>) -> Self {
        Self {
            task_fingerprint_hash: task_fingerprint_hash.into(),
            ..Default::default()
        }
    }

    pub fn phase(mut self, phase: u32) -> Self {
        self.phase = phase;
        self
    }

    pub fn files_changed(mut self, n: usize) -> Self {
        self.files_changed = n;
        self
    }

    pub fn files_created(mut self, n: usize) -> Self {
        self.files_created = n;
        self
    }

    pub fn compile_failures(mut self, n: u32) -> Self {
        self.compile_failures = n;
        self
    }

    pub fn test_failures(mut self, n: u32) -> Self {
        self.test_failures = n;
        self
    }

    pub fn verification_level(mut self, level: VerificationLevel) -> Self {
        self.verification_level = level;
        self
    }

    pub fn criteria(mut self, satisfied: usize, remaining: usize) -> Self {
        self.criteria_satisfied = satisfied;
        self.criteria_remaining = remaining;
        self
    }

    pub fn recovery_count(mut self, n: u32) -> Self {
        self.recovery_count = n;
        self
    }

    pub fn replan_count(mut self, n: u32) -> Self {
        self.replan_count = n;
        self
    }

    pub fn last_error_class(mut self, error_class: Option<String>) -> Self {
        self.last_error_class = error_class;
        self
    }

    pub fn progress_stalled(mut self, stalled: bool) -> Self {
        self.progress_stalled = stalled;
        self
    }

    pub fn build(self) -> StateSignature {
        StateSignature {
            task_fingerprint_hash: self.task_fingerprint_hash,
            phase: self.phase,
            files_changed: self.files_changed,
            files_created: self.files_created,
            compile_failures: self.compile_failures,
            test_failures: self.test_failures,
            verification_level: self.verification_level,
            criteria_satisfied: self.criteria_satisfied,
            criteria_remaining: self.criteria_remaining,
            recovery_count: self.recovery_count,
            replan_count: self.replan_count,
            last_error_class: self.last_error_class,
            progress_stalled: self.progress_stalled,
        }
    }
}
