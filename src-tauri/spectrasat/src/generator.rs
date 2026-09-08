// src/generator.rs
//! Generador de instancias 3-SAT difíciles en la frontera de fase crítica (m/n ≈ 4.26)

use crate::spectral::Clause3;
use rand::prelude::*;

/// Genera una fórmula 3-SAT aleatoria uniforme
pub fn generate_random_3sat(
    num_vars: usize,
    num_clauses: usize,
    seed: Option<u64>,
) -> Vec<Clause3> {
    let mut rng: Box<dyn RngCore> = match seed {
        Some(s) => Box::new(StdRng::seed_from_u64(s)),
        None => Box::new(thread_rng()),
    };

    let mut clauses = Vec::with_capacity(num_clauses);

    for _ in 0..num_clauses {
        let mut chosen_vars = Vec::with_capacity(3);
        while chosen_vars.len() < 3 {
            let v = rng.gen_range(1..=num_vars) as i32;
            if !chosen_vars.contains(&v) {
                chosen_vars.push(v);
            }
        }

        let l1 = if rng.gen_bool(0.5) {
            chosen_vars[0]
        } else {
            -chosen_vars[0]
        };
        let l2 = if rng.gen_bool(0.5) {
            chosen_vars[1]
        } else {
            -chosen_vars[1]
        };
        let l3 = if rng.gen_bool(0.5) {
            chosen_vars[2]
        } else {
            -chosen_vars[2]
        };

        clauses.push(Clause3([l1, l2, l3]));
    }

    clauses
}

/// Genera una fórmula 3-SAT garantizadamente SAT (con solución plantada)
pub fn generate_planted_3sat(
    num_vars: usize,
    num_clauses: usize,
    seed: Option<u64>,
) -> (Vec<Clause3>, Vec<bool>) {
    let mut rng: Box<dyn RngCore> = match seed {
        Some(s) => Box::new(StdRng::seed_from_u64(s)),
        None => Box::new(thread_rng()),
    };

    // Plantamos una asignación objetivo
    let planted_assignment: Vec<bool> = (0..num_vars).map(|_| rng.gen_bool(0.5)).collect();

    let mut clauses = Vec::with_capacity(num_clauses);

    for _ in 0..num_clauses {
        let mut chosen_vars = Vec::with_capacity(3);
        while chosen_vars.len() < 3 {
            let v = rng.gen_range(1..=num_vars) as i32;
            if !chosen_vars.contains(&v) {
                chosen_vars.push(v);
            }
        }

        // Aseguramos que al menos un literal evalúe a True bajo la asignación plantada
        let mut lits = [0i32; 3];
        let mut satisfied = false;

        for i in 0..3 {
            let var_idx = (chosen_vars[i] as usize) - 1;
            let val = planted_assignment[var_idx];
            let sign = rng.gen_bool(0.5);
            lits[i] = if sign {
                chosen_vars[i]
            } else {
                -chosen_vars[i]
            };

            if (lits[i] > 0 && val) || (lits[i] < 0 && !val) {
                satisfied = true;
            }
        }

        if !satisfied {
            // Forzamos que el primer literal satisfaga la cláusula
            let var_idx = (chosen_vars[0] as usize) - 1;
            lits[0] = if planted_assignment[var_idx] {
                chosen_vars[0]
            } else {
                -chosen_vars[0]
            };
        }

        clauses.push(Clause3(lits));
    }

    (clauses, planted_assignment)
}
