use serde::{Deserialize, Serialize};

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BudgetPhase {
    Execution,
    Verification,
    Recovery,
    Handoff,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepBudget {
    pub total_steps: u32,
    pub used_steps: u32,
    pub execution_budget: u32,
    pub verification_budget: u32,
    pub recovery_budget: u32,
    pub handoff_budget: u32,
}

impl StepBudget {
    #[allow(dead_code)]
    pub fn new(total_steps: u32) -> Self {
        // Industry Standard Allocation:
        // 40% execution / 30% verification / 20% error recovery / 10% handoff
        let execution_budget = (total_steps as f32 * 0.40).round() as u32;
        let verification_budget = (total_steps as f32 * 0.30).round() as u32;
        let recovery_budget = (total_steps as f32 * 0.20).round() as u32;
        let handoff_budget = total_steps.saturating_sub(execution_budget + verification_budget + recovery_budget);

        Self {
            total_steps,
            used_steps: 0,
            execution_budget,
            verification_budget,
            recovery_budget,
            handoff_budget,
        }
    }

    #[allow(dead_code)]
    pub fn record_step(&mut self) -> u32 {
        self.used_steps += 1;
        self.used_steps
    }

    #[allow(dead_code)]
    pub fn current_phase(&self) -> BudgetPhase {
        if self.used_steps < self.execution_budget {
            BudgetPhase::Execution
        } else if self.used_steps < (self.execution_budget + self.verification_budget) {
            BudgetPhase::Verification
        } else if self.used_steps < (self.total_steps - self.handoff_budget) {
            BudgetPhase::Recovery
        } else {
            BudgetPhase::Handoff
        }
    }

    #[allow(dead_code)]
    pub fn is_exhausted(&self) -> bool {
        self.used_steps >= self.total_steps
    }

    #[allow(dead_code)]
    pub fn should_handoff(&self) -> bool {
        self.used_steps >= (self.total_steps.saturating_sub(self.handoff_budget))
    }

    #[allow(dead_code)]
    pub fn remaining_steps(&self) -> u32 {
        self.total_steps.saturating_sub(self.used_steps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_step_budget_allocation() {
        let mut budget = StepBudget::new(50);
        assert_eq!(budget.execution_budget, 20);
        assert_eq!(budget.verification_budget, 15);
        assert_eq!(budget.recovery_budget, 10);
        assert_eq!(budget.handoff_budget, 5);

        assert_eq!(budget.current_phase(), BudgetPhase::Execution);
        for _ in 0..20 {
            budget.record_step();
        }
        assert_eq!(budget.current_phase(), BudgetPhase::Verification);
        for _ in 0..15 {
            budget.record_step();
        }
        assert_eq!(budget.current_phase(), BudgetPhase::Recovery);
        for _ in 0..10 {
            budget.record_step();
        }
        assert_eq!(budget.current_phase(), BudgetPhase::Handoff);
        assert!(budget.should_handoff());
    }
}
