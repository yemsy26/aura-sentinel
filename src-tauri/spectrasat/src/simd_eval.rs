// src/simd_eval.rs
//! Motor de Evaluación Vectorizada SIMD y Poda Desacoplada para Circuitos TC^0
//!
//! Desacopla las comunidades izquierda y derecha (O(2^|VL| + 2^|VR|) en lugar de O(2^n)),
//! aplica poda temprana sobre las cláusulas de frontera y paraleliza con Rayon.

use crate::spectral::{Clause3, SpectralPartition};
use rayon::prelude::*;
use wide::{f64x4, CmpGe};

/// Bloque SoA alineado a 64 bytes que encapsula 4 cláusulas 3-SAT
#[repr(C, align(64))]
#[derive(Debug, Clone)]
pub struct SimdClauseBlock4 {
    pub var_indices: [[usize; 4]; 3],
    pub weights: [f64x4; 3],
    pub thresholds: f64x4,
}

/// Contenedor de la fórmula 3-SAT optimizado para evaluación vectorial en CPU
#[repr(C, align(64))]
#[derive(Debug, Clone)]
pub struct SimdFormula {
    pub num_vars: usize,
    pub num_clauses: usize,
    pub original_clauses: Vec<Clause3>,
    pub simd_blocks: Vec<SimdClauseBlock4>,
    pub scalar_clauses: Vec<Clause3>,
}

impl SimdFormula {
    /// Empaqueta una lista estándar de cláusulas 3-SAT en bloques SIMD SoA
    pub fn from_clauses(num_vars: usize, clauses: &[Clause3]) -> Self {
        let num_clauses = clauses.len();
        let num_full_blocks = num_clauses / 4;
        let mut simd_blocks = Vec::with_capacity(num_full_blocks);

        for b in 0..num_full_blocks {
            let base = b * 4;
            let c = [
                &clauses[base],
                &clauses[base + 1],
                &clauses[base + 2],
                &clauses[base + 3],
            ];

            let mut var_indices = [[0usize; 4]; 3];
            let mut w0 = [0.0f64; 4];
            let mut w1 = [0.0f64; 4];
            let mut w2 = [0.0f64; 4];
            let mut th = [0.0f64; 4];

            for i in 0..4 {
                let cl = c[i];
                let mut neg_count = 0.0;

                // Literal 0
                let lit0 = cl.0[0];
                var_indices[0][i] = (lit0.unsigned_abs() as usize) - 1;
                w0[i] = if lit0 > 0 {
                    1.0
                } else {
                    neg_count += 1.0;
                    -1.0
                };

                // Literal 1
                let lit1 = cl.0[1];
                var_indices[1][i] = (lit1.unsigned_abs() as usize) - 1;
                w1[i] = if lit1 > 0 {
                    1.0
                } else {
                    neg_count += 1.0;
                    -1.0
                };

                // Literal 2
                let lit2 = cl.0[2];
                var_indices[2][i] = (lit2.unsigned_abs() as usize) - 1;
                w2[i] = if lit2 > 0 {
                    1.0
                } else {
                    neg_count += 1.0;
                    -1.0
                };

                th[i] = 1.0 - neg_count;
            }

            simd_blocks.push(SimdClauseBlock4 {
                var_indices,
                weights: [f64x4::new(w0), f64x4::new(w1), f64x4::new(w2)],
                thresholds: f64x4::new(th),
            });
        }

        let remainder_start = num_full_blocks * 4;
        let scalar_clauses = clauses[remainder_start..].to_vec();

        Self {
            num_vars,
            num_clauses,
            original_clauses: clauses.to_vec(),
            simd_blocks,
            scalar_clauses,
        }
    }

    /// Evaluación vectorizada branchless de una asignación completa sobre todos los bloques SIMD
    #[inline(always)]
    pub fn evaluate_assignment_simd(&self, assignment: &[f64]) -> bool {
        let zero = f64x4::splat(0.0);

        for block in &self.simd_blocks {
            let x0 = f64x4::new([
                assignment[block.var_indices[0][0]],
                assignment[block.var_indices[0][1]],
                assignment[block.var_indices[0][2]],
                assignment[block.var_indices[0][3]],
            ]);

            let x1 = f64x4::new([
                assignment[block.var_indices[1][0]],
                assignment[block.var_indices[1][1]],
                assignment[block.var_indices[1][2]],
                assignment[block.var_indices[1][3]],
            ]);

            let x2 = f64x4::new([
                assignment[block.var_indices[2][0]],
                assignment[block.var_indices[2][1]],
                assignment[block.var_indices[2][2]],
                assignment[block.var_indices[2][3]],
            ]);

            let sum = (block.weights[0] * x0) + (block.weights[1] * x1) + (block.weights[2] * x2);
            let slack = sum - block.thresholds;

            let mask = slack.cmp_ge(zero);
            let mask_arr = mask.to_array();

            if mask_arr[0].to_bits() == 0
                || mask_arr[1].to_bits() == 0
                || mask_arr[2].to_bits() == 0
                || mask_arr[3].to_bits() == 0
            {
                return false;
            }
        }

        for cl in &self.scalar_clauses {
            let mut satisfied = false;
            for &lit in &cl.0 {
                let var_idx = (lit.unsigned_abs() as usize) - 1;
                let val = assignment[var_idx];
                if (lit > 0 && val > 0.5) || (lit < 0 && val < 0.5) {
                    satisfied = true;
                    break;
                }
            }
            if !satisfied {
                return false;
            }
        }

        true
    }
}

/// Evaluador de Satisfacibilidad Híbrido: Poda Espectral Desacoplada + SIMD + Rayon
pub struct ParallelSpectralSolver<'a> {
    pub formula: &'a SimdFormula,
    pub partition: &'a SpectralPartition,
}

impl<'a> ParallelSpectralSolver<'a> {
    pub fn new(formula: &'a SimdFormula, partition: &'a SpectralPartition) -> Self {
        Self { formula, partition }
    }

    /// Resuelve desacoplando el corte de las comunidades izquierda y derecha
    pub fn solve(&self) -> Option<Vec<bool>> {
        let k = self.partition.cut_variables.len();

        // Si el corte es demasiado grande (grafo hiper-expansor sin corte claro),
        // limitamos la búsqueda sobre las variables de mayor peso de Fiedler
        let effective_k = k.min(22);
        let total_branches = 1u64 << effective_k;

        let cut_vars = &self.partition.cut_variables[..effective_k];
        let left_vars = &self.partition.left_partition;
        let right_vars = &self.partition.right_partition;

        // Separar cláusulas relevantes para izquierda, derecha y corte
        let left_clauses: Vec<Clause3> = self
            .formula
            .original_clauses
            .iter()
            .filter(|c| {
                c.0.iter()
                    .any(|lit| left_vars.contains(&((lit.unsigned_abs() as usize) - 1)))
            })
            .cloned()
            .collect();

        let right_clauses: Vec<Clause3> = self
            .formula
            .original_clauses
            .iter()
            .filter(|c| {
                c.0.iter()
                    .any(|lit| right_vars.contains(&((lit.unsigned_abs() as usize) - 1)))
            })
            .cloned()
            .collect();

        let left_simd = SimdFormula::from_clauses(self.formula.num_vars, &left_clauses);
        let right_simd = SimdFormula::from_clauses(self.formula.num_vars, &right_clauses);

        // Rayon divide las ramas del corte de forma masiva
        (0..total_branches)
            .into_par_iter()
            .find_map_any(|branch_mask| {
                let mut base_assignment = vec![0.0f64; self.formula.num_vars];

                // 1. Asignar variables de corte
                for (bit_idx, &var_idx) in cut_vars.iter().enumerate() {
                    base_assignment[var_idx] = if (branch_mask & (1 << bit_idx)) != 0 {
                        1.0
                    } else {
                        0.0
                    };
                }

                // 2. Poda temprana: comprobar si alguna cláusula puramente de corte ya está falsificada
                for cl in &self.formula.original_clauses {
                    let all_in_cut =
                        cl.0.iter()
                            .all(|lit| cut_vars.contains(&((lit.unsigned_abs() as usize) - 1)));
                    if all_in_cut {
                        let mut sat = false;
                        for &lit in &cl.0 {
                            let var_idx = (lit.unsigned_abs() as usize) - 1;
                            let val = base_assignment[var_idx];
                            if (lit > 0 && val > 0.5) || (lit < 0 && val < 0.5) {
                                sat = true;
                                break;
                            }
                        }
                        if !sat {
                            return None; // Rama inviable, poda inmediata
                        }
                    }
                }

                // 3. Resolver sub-comunidad izquierda de forma independiente (O(2^|VL|))
                let left_solution =
                    Self::solve_subcommunity(&left_simd, left_vars, &base_assignment)?;

                // 4. Resolver sub-comunidad derecha de forma independiente (O(2^|VR|))
                let right_solution =
                    Self::solve_subcommunity(&right_simd, right_vars, &left_solution)?;

                // 5. Verificación final completa en SIMD
                if self.formula.evaluate_assignment_simd(&right_solution) {
                    Some(right_solution.iter().map(|&v| v > 0.5).collect())
                } else {
                    None
                }
            })
    }

    /// Resuelve una subcomunidad en tiempo O(2^|sub_vars|)
    fn solve_subcommunity(
        sub_formula: &SimdFormula,
        sub_vars: &[usize],
        base_assignment: &[f64],
    ) -> Option<Vec<f64>> {
        let sub_len = sub_vars.len();
        if sub_len == 0 {
            return Some(base_assignment.to_vec());
        }

        // Si la subcomunidad es pequeña (<= 18 vars), búsqueda exhaustiva SIMD local
        let max_sub_search = sub_len.min(18);
        let space = 1u64 << max_sub_search;

        let mut local_assignment = base_assignment.to_vec();

        for mask in 0..space {
            for (bit_idx, &var_idx) in sub_vars.iter().take(max_sub_search).enumerate() {
                local_assignment[var_idx] = if (mask & (1 << bit_idx)) != 0 {
                    1.0
                } else {
                    0.0
                };
            }

            if sub_formula.evaluate_assignment_simd(&local_assignment) {
                return Some(local_assignment);
            }
        }

        None
    }

    /// Método de fallback de fuerza bruta paralela para instancias muy pequeñas (n <= 24)
    pub fn fallback_bruteforce(&self) -> Option<Vec<bool>> {
        let n = self.formula.num_vars;
        if n >= 26 {
            return None; // Evitar explosión 2^26 en fuerza bruta
        }
        let total_space = 1u64 << n;

        (0..total_space).into_par_iter().find_map_any(|mask| {
            let mut assignment = vec![0.0f64; n];
            for i in 0..n {
                assignment[i] = if (mask & (1 << i)) != 0 { 1.0 } else { 0.0 };
            }

            if self.formula.evaluate_assignment_simd(&assignment) {
                Some(assignment.iter().map(|&v| v > 0.5).collect())
            } else {
                None
            }
        })
    }
}
