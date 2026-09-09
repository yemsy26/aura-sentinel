#![allow(dead_code)]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StallType {
    NoStateChange,
    RepeatedTool,
    RepeatedCommand,
    SameError,
    NoCriteriaProgress,
    ModelLoop,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProgressSignature {
    pub step: u32,
    pub state_hash: u64,
    pub files_changed: u32,
    pub criteria_satisfied: u32,
    pub evidence_count: u32,
    pub last_tool_used: String,
    pub last_command: String,
    pub last_error_hash: u64,
}

pub struct StallDetector {
    signatures: Vec<ProgressSignature>,
    max_history: usize,
}

impl StallDetector {
    pub fn new(max_history: usize) -> Self {
        Self { signatures: Vec::new(), max_history }
    }

    /// Number of stall signatures recorded (for LearningEngine snapshot).
    pub fn total_stalls(&self) -> u32 {
        self.signatures.len() as u32
    }

    pub fn record_signature(&mut self, sig: ProgressSignature) {
        if self.signatures.len() >= self.max_history {
            self.signatures.remove(0);
        }
        self.signatures.push(sig);
    }

    /// Detect the most serious stall in the last window_size steps.
    /// Priority: RepeatedTool > RepeatedCommand > SameError > NoStateChange > NoCriteriaProgress
    pub fn detect_stall(&self, window_size: usize) -> Option<StallType> {
        if self.signatures.len() < window_size || window_size < 2 {
            return None;
        }
        let slice = &self.signatures[self.signatures.len() - window_size..];

        // RepeatedTool: same non-empty tool every step in the window
        let first_tool = &slice[0].last_tool_used;
        if !first_tool.is_empty() && slice.iter().all(|s| &s.last_tool_used == first_tool) {
            return Some(StallType::RepeatedTool);
        }

        // RepeatedCommand: same non-empty command every step
        let first_cmd = &slice[0].last_command;
        if !first_cmd.is_empty() && slice.iter().all(|s| &s.last_command == first_cmd) {
            return Some(StallType::RepeatedCommand);
        }

        // SameError: same non-zero error hash every step
        let first_err = slice[0].last_error_hash;
        if first_err != 0 && slice.iter().all(|s| s.last_error_hash == first_err) {
            return Some(StallType::SameError);
        }

        // NoStateChange: no file, criteria, evidence or state_hash change
        let first = &slice[0];
        let has_progress = slice.iter().skip(1).any(|s| {
            s.files_changed != first.files_changed
                || s.criteria_satisfied != first.criteria_satisfied
                || s.evidence_count != first.evidence_count
                || s.state_hash != first.state_hash
        });
        if !has_progress {
            return Some(StallType::NoStateChange);
        }

        // NoCriteriaProgress: criteria count frozen across window even though other things changed
        let criteria_frozen = slice.iter().all(|s| s.criteria_satisfied == first.criteria_satisfied);
        if criteria_frozen && slice.iter().any(|s| s.evidence_count != first.evidence_count) {
            return Some(StallType::NoCriteriaProgress);
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(step: u32, tool: &str, cmd: &str, err: u64, files: u32, crit: u32, ev: u32) -> ProgressSignature {
        ProgressSignature {
            step, state_hash: 0, files_changed: files, criteria_satisfied: crit, evidence_count: ev,
            last_tool_used: tool.to_string(), last_command: cmd.to_string(), last_error_hash: err,
        }
    }

    #[test]
    fn test_stall_repeated_tool() {
        let mut d = StallDetector::new(5);
        for i in 0..3 { d.record_signature(sig(i, "TOOL_TERMINAL", "cargo build", 0, i, 0, 0)); }
        assert_eq!(d.detect_stall(3), Some(StallType::RepeatedTool));
    }

    #[test]
    fn test_stall_same_error() {
        let mut d = StallDetector::new(5);
        // Use different tools each step so RepeatedTool does NOT fire, only SameError
        let tools = ["TOOL_PROGRAMMER", "TOOL_TERMINAL", "TOOL_VALIDATOR"];
        for i in 0..3 {
            d.record_signature(sig(i as u32, tools[i], "", 0xdeadbeef, i as u32, 0, 0));
        }
        assert_eq!(d.detect_stall(3), Some(StallType::SameError));
    }

    #[test]
    fn test_stall_no_state_change() {
        let mut d = StallDetector::new(5);
        for i in 0..3 { d.record_signature(sig(i, "", "", 0, 5, 2, 10)); }
        assert_eq!(d.detect_stall(3), Some(StallType::NoStateChange));
    }

    #[test]
    fn test_no_stall_when_progress() {
        let mut d = StallDetector::new(5);
        d.record_signature(sig(0, "TOOL_TERMINAL", "cargo build", 0, 2, 0, 5));
        d.record_signature(sig(1, "TOOL_PROGRAMMER", "cargo build", 0, 4, 1, 7));
        d.record_signature(sig(2, "TOOL_TERMINAL", "cargo test", 0, 4, 2, 9));
        assert_eq!(d.detect_stall(3), None);
    }
}
