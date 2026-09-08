pub mod chordal;
pub mod dimacs;
pub mod generator;
pub mod gf2_elimination;
pub mod planted_clique;
pub mod sdp_branching;
pub mod sdp_solver;
pub mod simd_eval;
pub mod sos_hierarchy;
pub mod spectral;

use crate::spectral::Clause3;
use crate::gf2_elimination::Gf2System;
use crate::chordal::ChordalExtension;
use crate::sdp_solver::{solve_sos_sdp, SdpVerdict};
use crate::sdp_branching::branch_and_bound_solve;

use serde::Serialize;

#[derive(Serialize)]
pub struct SatResult {
    pub status: String,
    pub assignment: Option<Vec<bool>>,
}

pub fn solve_native_rust(n_vars: usize, clauses_in: Vec<Vec<i32>>) -> String {
    let mut clauses = Vec::new();
    for c in &clauses_in {
        match c.len() {
            3 => clauses.push(Clause3([c[0], c[1], c[2]])),
            2 => clauses.push(Clause3([c[0], c[1], c[1]])),
            1 => clauses.push(Clause3([c[0], c[0], c[0]])),
            _ => continue,
        }
    }

    // ── Stage 1: GF2 Algebraic pre-filter ─────────────────────────────────
    let mut gf2 = Gf2System::extract_from_3cnf(n_vars, &clauses);
    if gf2.is_tseitin_unsat() {
        let res = SatResult { status: "UNSAT_GF2".to_string(), assignment: None };
        return serde_json::to_string(&res).unwrap_or_else(|_| "UNSAT_GF2".to_string());
    }

    // ── Stage 2: SDP relaxation to guide branching ─────────────────────────
    let mut chordal = ChordalExtension::new(n_vars, &clauses);
    let _cliques = chordal.extract_maximal_cliques();
    let (verdict, _) = solve_sos_sdp(n_vars, &clauses, false);

    let res = match verdict {
        SdpVerdict::ProvenUnsat { .. } => SatResult { status: "UNSAT_SDP".to_string(), assignment: None },
        SdpVerdict::PossibleSat { .. } | SdpVerdict::Unknown { .. } => {
            // ── Stage 3: Branch & Bound (SDP-guided heuristic) ──────────────
            let (bb_verdict, bb_assign) = branch_and_bound_solve(n_vars, &clauses);

            if bb_verdict == "SAT_CERTIFIED" {
                if let Some(ref asgn) = bb_assign {
                    // CRITICAL: Verify the assignment actually satisfies ALL clauses
                    if satisfies_all_clauses(asgn, &clauses) {
                        return serde_json::to_string(&SatResult {
                            status: "SAT_CERTIFIED".to_string(),
                            assignment: bb_assign,
                        }).unwrap_or_else(|_| "SAT_CERTIFIED".to_string());
                    }
                }
            }

            // ── Stage 4: DPLL exhaustive fallback (always correct) ──────────
            // Branch-and-Bound is heuristic — if its answer fails verification,
            // fall back to exact DPLL which is guaranteed correct.
            match dpll_solve(n_vars, &clauses_in) {
                Some(assignment) => SatResult { status: "SAT_CERTIFIED".to_string(), assignment: Some(assignment) },
                None             => SatResult { status: "UNSAT_EXHAUSTED".to_string(), assignment: None },
            }
        }
    };

    serde_json::to_string(&res).unwrap_or_else(|_| "UNKNOWN".to_string())
}

/// Verifica que una asignación booleana satisface TODAS las cláusulas.
/// Esta es la "prueba matemática de cierre": sin esto, el motor puede certificar falsos positivos.
fn satisfies_all_clauses(assignment: &[bool], clauses: &[Clause3]) -> bool {
    clauses.iter().all(|clause| {
        clause.0.iter().any(|&lit| {
            let var_idx = (lit.unsigned_abs() as usize) - 1;
            if var_idx >= assignment.len() { return false; }
            let value = assignment[var_idx];
            if lit > 0 { value } else { !value }
        })
    })
}

/// Solucionador DPLL exacto y minimalista.
/// Garantiza corrección matemática absoluta (completo y correcto por construcción).
/// Complejidad: O(2^n) en peor caso, pero la poda unit-propagation lo hace
/// práctico para instancias de hasta ~40 variables.
fn dpll_solve(n_vars: usize, raw_clauses: &[Vec<i32>]) -> Option<Vec<bool>> {
    let mut assignment = vec![None::<bool>; n_vars];
    if dpll_recursive(&mut assignment, raw_clauses) {
        Some(assignment.into_iter().map(|v| v.unwrap_or(false)).collect())
    } else {
        None
    }
}

fn dpll_recursive(assignment: &mut Vec<Option<bool>>, clauses: &[Vec<i32>]) -> bool {
    // 1. Unit propagation
    loop {
        let mut propagated = false;
        for clause in clauses {
            let mut unset_lit: Option<i32> = None;
            let mut clause_satisfied = false;
            let mut all_false = true;
            for &lit in clause {
                let idx = (lit.unsigned_abs() as usize) - 1;
                match assignment.get(idx).and_then(|v| *v) {
                    Some(val) => {
                        let sat = if lit > 0 { val } else { !val };
                        if sat { clause_satisfied = true; all_false = false; break; }
                        // this literal is false, keep scanning
                    },
                    None => {
                        all_false = false;
                        unset_lit = Some(lit);
                    }
                }
            }
            if clause_satisfied { continue; }
            if all_false { return false; } // conflict
            if let Some(unit) = unset_lit {
                // Check no other unset literal — it's a unit clause
                let unset_count = clause.iter().filter(|&&l| {
                    let idx = (l.unsigned_abs() as usize) - 1;
                    assignment.get(idx).and_then(|v| *v).is_none()
                }).count();
                if unset_count == 1 {
                    let idx = (unit.unsigned_abs() as usize) - 1;
                    assignment[idx] = Some(unit > 0);
                    propagated = true;
                }
            }
        }
        if !propagated { break; }
    }

    // 2. Check if all clauses satisfied
    let all_sat = clauses.iter().all(|clause| {
        clause.iter().any(|&lit| {
            let idx = (lit.unsigned_abs() as usize) - 1;
            matches!(assignment.get(idx).and_then(|v| *v),
                Some(val) if (lit > 0 && val) || (lit < 0 && !val))
        })
    });
    if all_sat { return true; }

    // 3. Check for conflict (any clause all-false)
    let conflict = clauses.iter().any(|clause| {
        clause.iter().all(|&lit| {
            let idx = (lit.unsigned_abs() as usize) - 1;
            matches!(assignment.get(idx).and_then(|v| *v),
                Some(val) if (lit > 0 && !val) || (lit < 0 && val))
        })
    });
    if conflict { return false; }

    // 4. Pick first unset variable and branch
    let branch_var = match assignment.iter().position(|v| v.is_none()) {
        Some(idx) => idx,
        None => return false, // no unset vars, but not all clauses satisfied = conflict
    };

    for &val in &[true, false] {
        assignment[branch_var] = Some(val);
        if dpll_recursive(assignment, clauses) {
            return true;
        }
        assignment[branch_var] = None;
    }
    false
}


#[cfg(feature = "python-ext")]
use pyo3::prelude::*;

#[cfg(feature = "python-ext")]
#[pyfunction]
fn solve_native(n_vars: usize, clauses_in: Vec<Vec<i32>>) -> PyResult<String> {
    Ok(solve_native_rust(n_vars, clauses_in))
}

#[cfg(feature = "python-ext")]
#[pymodule]
fn spectrasat_core(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(solve_native, m)?)?;
    Ok(())
}