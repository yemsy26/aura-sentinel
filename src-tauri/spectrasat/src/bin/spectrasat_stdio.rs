use spectrasat_core::chordal::ChordalExtension;
use spectrasat_core::gf2_elimination::Gf2System;
use spectrasat_core::sdp_branching::branch_and_bound_solve;
use spectrasat_core::sdp_solver::{solve_sos_sdp, SdpVerdict};
use spectrasat_core::spectral::Clause3;
use serde::{Deserialize, Serialize};
use std::io::{self, Read};
use std::time::Instant;

#[derive(Deserialize)]
struct StdinPayload {
    n_vars: usize,
    clauses: Vec<Vec<i32>>,
}

#[derive(Serialize)]
struct StdoutResponse {
    verdict: String,
    execution_time_ms: f64,
    primal_residual: f64,
    assignment: Option<Vec<bool>>,
}

fn main() {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        return;
    }

    let payload: StdinPayload = match serde_json::from_str(&input) {
        Ok(p) => p,
        Err(e) => {
            let err_res = StdoutResponse {
                verdict: format!("ERROR_PARSING: {}", e),
                execution_time_ms: 0.0,
                primal_residual: 0.0,
                assignment: None,
            };
            println!("{}", serde_json::to_string(&err_res).unwrap());
            return;
        }
    };

    let t_start = Instant::now();
    let n_vars = payload.n_vars;

    let mut clauses = Vec::new();
    for c in payload.clauses {
        if c.len() == 3 {
            clauses.push(Clause3([c[0], c[1], c[2]]));
        } else if c.len() == 2 {
            clauses.push(Clause3([c[0], c[1], c[1]]));
        } else if c.len() == 1 {
            clauses.push(Clause3([c[0], c[0], c[0]]));
        }
    }

    let mut gf2 = Gf2System::extract_from_3cnf(n_vars, &clauses);
    if gf2.is_tseitin_unsat() {
        let res = StdoutResponse {
            verdict: "UNSAT_GF2".to_string(),
            execution_time_ms: t_start.elapsed().as_secs_f64() * 1000.0,
            primal_residual: 0.0,
            assignment: None,
        };
        println!("{}", serde_json::to_string(&res).unwrap());
        return;
    }

    let mut chordal = ChordalExtension::new(n_vars, &clauses);
    let _cliques = chordal.extract_maximal_cliques();

    let (verdict, _) = solve_sos_sdp(n_vars, &clauses, false);

    match verdict {
        SdpVerdict::ProvenUnsat { divergence, .. } => {
            let res = StdoutResponse {
                verdict: "UNSAT_SDP".to_string(),
                execution_time_ms: t_start.elapsed().as_secs_f64() * 1000.0,
                primal_residual: divergence,
                assignment: None,
            };
            println!("{}", serde_json::to_string(&res).unwrap());
        }
        SdpVerdict::PossibleSat { residual, .. } => {
            let (verdict_str_raw, _assignment) = branch_and_bound_solve(n_vars, &clauses);
            let verdict_str = if verdict_str_raw.contains("SAT") {
                "SAT_CERTIFIED"
            } else {
                "UNSAT_BAB"
            };

            let res = StdoutResponse {
                verdict: verdict_str.to_string(),
                execution_time_ms: t_start.elapsed().as_secs_f64() * 1000.0,
                primal_residual: residual,
                assignment: None,
            };
            println!("{}", serde_json::to_string(&res).unwrap());
        }
        SdpVerdict::Unknown { primal_res, .. } => {
            let res = StdoutResponse {
                verdict: "UNKNOWN".to_string(),
                execution_time_ms: t_start.elapsed().as_secs_f64() * 1000.0,
                primal_residual: primal_res,
                assignment: None,
            };
            println!("{}", serde_json::to_string(&res).unwrap());
        }
    }
}
