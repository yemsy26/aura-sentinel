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

use crate::chordal::ChordalExtension;
use crate::gf2_elimination::Gf2System;
use crate::sdp_branching::branch_and_bound_solve;
use crate::sdp_solver::{solve_sos_sdp, SdpVerdict};
use crate::spectral::Clause3;
use serde::Serialize;

const MAX_VARIABLES: usize = 128;
const MAX_CLAUSES: usize = 20_000;
const MAX_TOTAL_LITERALS: usize = 100_000;
const MAX_DPLL_NODES: usize = 50_000;

#[derive(Serialize)]
pub struct SatResult {
    pub status: String,
    pub assignment: Option<Vec<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

enum DpllOutcome {
    Satisfiable(Vec<bool>),
    Unsatisfiable,
    SearchLimit,
}

fn result_json(status: &str, assignment: Option<Vec<bool>>, error: Option<String>) -> String {
    serde_json::to_string(&SatResult {
        status: status.to_string(),
        assignment,
        error,
    })
    .unwrap_or_else(|_| status.to_string())
}

fn validate_instance(n_vars: usize, clauses: &[Vec<i32>]) -> Result<(), String> {
    if n_vars > MAX_VARIABLES {
        return Err(format!(
            "La instancia declara {n_vars} variables; el límite seguro actual es {MAX_VARIABLES}."
        ));
    }
    if clauses.len() > MAX_CLAUSES {
        return Err(format!(
            "La instancia contiene {} cláusulas; el límite seguro actual es {MAX_CLAUSES}.",
            clauses.len()
        ));
    }

    let mut total_literals = 0usize;
    for (clause_index, clause) in clauses.iter().enumerate() {
        total_literals = total_literals.saturating_add(clause.len());
        if total_literals > MAX_TOTAL_LITERALS {
            return Err(format!(
                "La instancia supera el límite de {MAX_TOTAL_LITERALS} literales."
            ));
        }
        for (literal_index, &literal) in clause.iter().enumerate() {
            if literal == 0 {
                return Err(format!(
                    "El literal 0 no es válido (cláusula {}, posición {}).",
                    clause_index + 1,
                    literal_index + 1
                ));
            }
            let variable = literal.unsigned_abs() as usize;
            if variable > n_vars {
                return Err(format!(
                    "El literal {literal} referencia una variable fuera de 1..={n_vars}."
                ));
            }
        }
    }
    Ok(())
}

/// Resuelve CNF booleana. SDP/GF(2) y Branch-and-Bound solo proponen candidatos;
/// únicamente una asignación comprobada o la búsqueda exacta puede dar veredicto.
pub fn solve_native_rust(n_vars: usize, clauses_in: Vec<Vec<i32>>) -> String {
    if let Err(error) = validate_instance(n_vars, &clauses_in) {
        return result_json("INVALID_INPUT", None, Some(error));
    }

    // Una cláusula vacía es falsa para cualquier asignación. La fórmula sin
    // cláusulas es verdadera y puede resolverse sin iniciar los motores pesados.
    if clauses_in.iter().any(Vec::is_empty) {
        return result_json("UNSAT_EXHAUSTED", None, None);
    }
    if clauses_in.is_empty() {
        return result_json("SAT_CERTIFIED", Some(vec![false; n_vars]), None);
    }

    // Las capas geométricas solo entienden 1..=3 literales por cláusula. Se
    // omiten para k-SAT general: nunca se debe verificar un modelo contra una
    // versión incompleta de la fórmula.
    let is_three_cnf = clauses_in
        .iter()
        .all(|clause| (1..=3).contains(&clause.len()));
    if is_three_cnf && n_vars <= 24 {
        let clauses_3: Vec<Clause3> = clauses_in
            .iter()
            .map(|clause| match clause.len() {
                1 => Clause3([clause[0], clause[0], clause[0]]),
                2 => Clause3([clause[0], clause[1], clause[1]]),
                _ => Clause3([clause[0], clause[1], clause[2]]),
            })
            .collect();

        let mut gf2 = Gf2System::extract_from_3cnf(n_vars, &clauses_3);
        let gf2_unsat_hint = gf2.is_tseitin_unsat();
        if !gf2_unsat_hint {
            let mut chordal = ChordalExtension::new(n_vars, &clauses_3);
            let _cliques = chordal.extract_maximal_cliques();
            let (sdp_verdict, _) = solve_sos_sdp(n_vars, &clauses_3, false);

            // A numerical relaxation is not accepted as an UNSAT certificate.
            // It may guide a witness search; UNSAT is settled below by exact DPLL.
            if matches!(sdp_verdict, SdpVerdict::PossibleSat { .. }) {
                let (_, candidate) = branch_and_bound_solve(n_vars, &clauses_3);
                if let Some(assignment) = candidate {
                    if satisfies_all_clauses(&assignment, &clauses_in) {
                        return result_json("SAT_CERTIFIED", Some(assignment), None);
                    }
                }
            }
        }
    }

    match dpll_solve(n_vars, &clauses_in) {
        DpllOutcome::Satisfiable(assignment) => {
            if satisfies_all_clauses(&assignment, &clauses_in) {
                result_json("SAT_CERTIFIED", Some(assignment), None)
            } else {
                result_json(
                    "INTERNAL_VERIFICATION_FAILED",
                    None,
                    Some("El modelo SAT no satisface la fórmula original.".to_string()),
                )
            }
        }
        DpllOutcome::Unsatisfiable => result_json("UNSAT_EXHAUSTED", None, None),
        DpllOutcome::SearchLimit => result_json(
            "UNKNOWN_SEARCH_LIMIT",
            None,
            Some(format!(
                "La búsqueda exacta alcanzó el límite de {MAX_DPLL_NODES} nodos; no se certifica SAT ni UNSAT."
            )),
        ),
    }
}

/// Verifica el modelo contra todas las cláusulas originales recibidas.
fn satisfies_all_clauses(assignment: &[bool], clauses: &[Vec<i32>]) -> bool {
    clauses.iter().all(|clause| {
        clause.iter().any(|&literal| {
            let index = (literal.unsigned_abs() as usize) - 1;
            assignment
                .get(index)
                .is_some_and(|&value| if literal > 0 { value } else { !value })
        })
    })
}

/// Búsqueda DPLL exacta con límite de trabajo explícito; si se alcanza, devuelve
/// UNKNOWN en vez de presentar una conclusión parcial como si fuera un teorema.
fn dpll_solve(n_vars: usize, clauses: &[Vec<i32>]) -> DpllOutcome {
    dpll_recursive(vec![None; n_vars], clauses, &mut 0)
}

fn dpll_recursive(
    mut assignment: Vec<Option<bool>>,
    clauses: &[Vec<i32>],
    visited_nodes: &mut usize,
) -> DpllOutcome {
    if *visited_nodes >= MAX_DPLL_NODES {
        return DpllOutcome::SearchLimit;
    }
    *visited_nodes += 1;

    // Unit propagation. Each recursive branch owns its assignment, so failed
    // branch deductions cannot leak into the sibling branch.
    loop {
        let mut propagated = false;
        for clause in clauses {
            let mut unassigned_literal = None;
            let mut unassigned_count = 0usize;
            let mut clause_satisfied = false;

            for &literal in clause {
                let index = (literal.unsigned_abs() as usize) - 1;
                match assignment[index] {
                    Some(value) if (literal > 0 && value) || (literal < 0 && !value) => {
                        clause_satisfied = true;
                        break;
                    }
                    Some(_) => {}
                    None => {
                        unassigned_literal = Some(literal);
                        unassigned_count += 1;
                    }
                }
            }

            if clause_satisfied {
                continue;
            }
            if unassigned_count == 0 {
                return DpllOutcome::Unsatisfiable;
            }
            if unassigned_count == 1 {
                let literal = unassigned_literal.expect("one unassigned literal was counted");
                let index = (literal.unsigned_abs() as usize) - 1;
                assignment[index] = Some(literal > 0);
                propagated = true;
            }
        }
        if !propagated {
            break;
        }
    }

    let all_satisfied = clauses.iter().all(|clause| {
        clause.iter().any(|&literal| {
            let index = (literal.unsigned_abs() as usize) - 1;
            matches!(assignment[index], Some(value) if (literal > 0 && value) || (literal < 0 && !value))
        })
    });
    if all_satisfied {
        return DpllOutcome::Satisfiable(
            assignment
                .into_iter()
                .map(|value| value.unwrap_or(false))
                .collect(),
        );
    }

    let Some(branch_index) = assignment.iter().position(Option::is_none) else {
        return DpllOutcome::Unsatisfiable;
    };

    for value in [true, false] {
        let mut child = assignment.clone();
        child[branch_index] = Some(value);
        match dpll_recursive(child, clauses, visited_nodes) {
            sat @ DpllOutcome::Satisfiable(_) => return sat,
            DpllOutcome::SearchLimit => return DpllOutcome::SearchLimit,
            DpllOutcome::Unsatisfiable => {}
        }
    }
    DpllOutcome::Unsatisfiable
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn parsed(n_vars: usize, clauses: Vec<Vec<i32>>) -> Value {
        serde_json::from_str(&solve_native_rust(n_vars, clauses)).unwrap()
    }

    #[test]
    fn k_sat_constraints_are_not_dropped_by_the_three_sat_fast_path() {
        let result = parsed(1, vec![vec![1, 1, 1, 1], vec![-1, -1, -1, -1]]);
        assert_eq!(result["status"], "UNSAT_EXHAUSTED");
        assert!(result["assignment"].is_null());
    }

    #[test]
    fn empty_clause_is_certified_unsat() {
        let result = parsed(2, vec![vec![]]);
        assert_eq!(result["status"], "UNSAT_EXHAUSTED");
    }

    #[test]
    fn valid_model_is_returned_for_a_satisfiable_formula() {
        let clauses = vec![vec![1, 2], vec![-1, 2]];
        let result = parsed(2, clauses.clone());
        assert_eq!(result["status"], "SAT_CERTIFIED");
        let assignment: Vec<bool> = serde_json::from_value(result["assignment"].clone()).unwrap();
        assert!(satisfies_all_clauses(&assignment, &clauses));
    }

    #[test]
    fn zero_and_out_of_range_literals_are_rejected_without_panicking() {
        assert_eq!(parsed(1, vec![vec![0]])["status"], "INVALID_INPUT");
        assert_eq!(parsed(1, vec![vec![2]])["status"], "INVALID_INPUT");
        assert_eq!(parsed(0, vec![vec![1]])["status"], "INVALID_INPUT");
    }

    #[test]
    fn empty_formula_is_satisfiable_even_without_variables() {
        let result = parsed(0, vec![]);
        assert_eq!(result["status"], "SAT_CERTIFIED");
        assert_eq!(result["assignment"], serde_json::json!([]));
    }
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
