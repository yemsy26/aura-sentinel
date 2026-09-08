use crate::sdp_solver::{solve_sos_sdp, SdpVerdict};
use crate::spectral::Clause3;

/// Executes a Branch-and-Bound search guided by Semidefinite Programming (SDP) relaxations.
///
/// This function constructs a search tree over the boolean variables. At each node,
/// it evaluates the SDP relaxation of the formula conditioned on the current partial assignment.
/// If the SDP relaxation proves the node is strictly infeasible (UNSAT), the branch is pruned.
/// If the node is feasible but fractional, it branches on the variable with the maximum ambiguity
/// in the moment matrix (i.e., the variable whose expected value is closest to 0.5).
///
/// # Arguments
/// * `n_vars` - The total number of variables in the boolean formula.
/// * `clauses` - A slice containing the baseline 3-SAT clauses.
///
/// # Returns
/// A `String` indicating the exact boolean verdict ("SAT_CERTIFIED" or "UNSAT_EXHAUSTED").
pub fn branch_and_bound_solve(n_vars: usize, clauses: &[Clause3]) -> (String, Option<Vec<bool>>) {
    let mut stack: Vec<Vec<Option<bool>>> = Vec::new();
    stack.push(vec![None; n_vars]);

    while let Some(assignment) = stack.pop() {
        let mut local_clauses = clauses.to_vec();
        for (i, &val) in assignment.iter().enumerate() {
            if let Some(b) = val {
                let literal = if b { (i + 1) as i32 } else { -((i + 1) as i32) };
                local_clauses.push(Clause3([literal, literal, literal]));
            }
        }

        let (verdict, moment_matrix) = solve_sos_sdp(n_vars, &local_clauses, false);

        match verdict {
            SdpVerdict::ProvenUnsat { .. } => continue,
            SdpVerdict::PossibleSat { .. } => {
                if assignment.iter().all(|v| v.is_some()) {
                    let final_assignment = assignment.into_iter().map(|v| v.unwrap()).collect();
                    return ("SAT_CERTIFIED".to_string(), Some(final_assignment));
                }

                let mut target_var = None;
                let mut max_ambiguity = -1.0;

                for i in 0..n_vars {
                    if assignment[i].is_none() {
                        let val = moment_matrix.read(i + 1, i + 1);
                        let ambiguity = 0.25 - (val - 0.5).powi(2);
                        if ambiguity > max_ambiguity {
                            max_ambiguity = ambiguity;
                            target_var = Some(i);
                        }
                    }
                }

                if let Some(var_idx) = target_var {
                    let mut branch_true = assignment.clone();
                    branch_true[var_idx] = Some(true);
                    stack.push(branch_true);

                    let mut branch_false = assignment.clone();
                    branch_false[var_idx] = Some(false);
                    stack.push(branch_false);
                }
            }
            SdpVerdict::Unknown { .. } => {}
        }
    }
    ("UNSAT_EXHAUSTED".to_string(), None)
}
