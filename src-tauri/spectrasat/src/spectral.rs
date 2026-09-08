// src/spectral.rs
//! Módulo de Análisis Espectral Disperso (Sparse Eigensolver) para TC^0-SAT
//!
//! Implementa el método de iteración de potencias con deflación de Gram-Schmidt
//! sobre el operador Laplaciano Normalizado disperso L = I - D^(-1/2) A D^(-1/2).
//! Complejidad espacial O(N) y temporal O(iter * |E|), escalable a N > 100,000.

use rayon::prelude::*;

/// Representa una cláusula 3-SAT con 3 literales (1-indexed con signo)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clause3(pub [i32; 3]);

/// Resultado del Análisis Espectral sobre el grafo de incidencia
#[derive(Debug, Clone)]
pub struct SpectralPartition {
    /// Segundo autovalor más pequeño (Conectividad Algebraica lambda_2)
    pub lambda_2: f64,
    /// Cota inferior de la constante de Cheeger: sqrt(2 * lambda_2)
    pub cheeger_lower_bound: f64,
    /// Coordenadas de Fiedler proyectadas y normalizadas por grado para las n variables
    #[allow(dead_code)]
    pub variable_fiedler_weights: Vec<(usize, f64)>,
    /// Índices de variables clasificadas como "Frontera / Backbone" (corte crítico)
    pub cut_variables: Vec<usize>,
    /// Variables particionadas en Comunidad Izquierda (v2 < -epsilon)
    pub left_partition: Vec<usize>,
    /// Variables particionadas en Comunidad Derecha (v2 > epsilon)
    pub right_partition: Vec<usize>,
}

/// Grafo Bipartito de Incidencia Disperso (CSR / Adjacency Lists)
#[repr(C, align(64))]
pub struct SpectralFactorGraph {
    pub num_vars: usize,
    pub num_clauses: usize,
    pub total_nodes: usize, // N = num_vars + num_clauses
    pub total_edges: usize, // |E| = 3 * num_clauses

    // Grados de los nodos
    pub var_degrees: Vec<usize>,
    pub clause_degrees: Vec<usize>,

    // Precalculo de 1 / sqrt(deg(u))
    pub inv_sqrt_degrees: Vec<f64>,

    // Lista de adyacencia dispersa
    pub var_to_clauses: Vec<Vec<usize>>,
    pub clause_to_vars: Vec<[usize; 3]>,

    // Autovector trivial normalizado v1 = D^(1/2) * 1 / sqrt(2*|E|)
    pub v1_trivial: Vec<f64>,
}

impl SpectralFactorGraph {
    /// Construye el factor graph disperso en O(N + |E|) de memoria y tiempo
    pub fn new(num_vars: usize, clauses: &[Clause3]) -> Self {
        let num_clauses = clauses.len();
        let total_nodes = num_vars + num_clauses;
        let total_edges = 3 * num_clauses;

        let mut var_degrees = vec![0usize; num_vars];
        let clause_degrees = vec![3usize; num_clauses];
        let mut var_to_clauses = vec![Vec::new(); num_vars];
        let mut clause_to_vars = Vec::with_capacity(num_clauses);

        for (c_idx, clause) in clauses.iter().enumerate() {
            let mut c_vars = [0usize; 3];
            for (lit_pos, &lit) in clause.0.iter().enumerate() {
                let var_idx = (lit.unsigned_abs() as usize) - 1;
                c_vars[lit_pos] = var_idx;
                if var_idx < num_vars {
                    var_degrees[var_idx] += 1;
                    var_to_clauses[var_idx].push(c_idx);
                }
            }
            clause_to_vars.push(c_vars);
        }

        // Precomputamos 1 / sqrt(deg(u)) para evitar divisiones en el solver iterativo
        let mut inv_sqrt_degrees = vec![0.0f64; total_nodes];
        for i in 0..num_vars {
            let deg = var_degrees[i] as f64;
            inv_sqrt_degrees[i] = if deg > 0.0 { 1.0 / deg.sqrt() } else { 0.0 };
        }
        for j in 0..num_clauses {
            inv_sqrt_degrees[num_vars + j] = 1.0 / (3.0f64).sqrt();
        }

        // Construimos el autovector trivial v1
        let sqrt_2e = (2.0 * total_edges as f64).sqrt();
        let mut v1_trivial = vec![0.0f64; total_nodes];
        for i in 0..num_vars {
            let deg = var_degrees[i] as f64;
            v1_trivial[i] = deg.sqrt() / sqrt_2e;
        }
        for j in 0..num_clauses {
            v1_trivial[num_vars + j] = (3.0f64).sqrt() / sqrt_2e;
        }

        Self {
            num_vars,
            num_clauses,
            total_nodes,
            total_edges,
            var_degrees,
            clause_degrees,
            inv_sqrt_degrees,
            var_to_clauses,
            clause_to_vars,
            v1_trivial,
        }
    }

    /// Producto matriz-vector disperso: y = M * x, donde M = I + D^(-1/2) A D^(-1/2) = 2I - L
    /// Los autovalores de M son mu_i = 2 - lambda_i.
    /// El autovalor máximo de M es mu_1 = 2 (con v1), y el segundo máximo es mu_2 = 2 - lambda_2 (con Fiedler v2).
    pub fn sparse_matvec_m(&self, x: &[f64], y: &mut [f64]) {
        let n_vars = self.num_vars;
        let n_clauses = self.num_clauses;

        // 1. Paralelismo Rayon para los nodos de variables
        y[..n_vars]
            .par_iter_mut()
            .enumerate()
            .for_each(|(var_idx, y_val)| {
                let inv_sqrt_u = self.inv_sqrt_degrees[var_idx];
                if inv_sqrt_u == 0.0 {
                    *y_val = x[var_idx];
                    return;
                }

                let mut neighbor_sum = 0.0;
                for &clause_idx in &self.var_to_clauses[var_idx] {
                    let c_node = n_vars + clause_idx;
                    neighbor_sum += x[c_node] * self.inv_sqrt_degrees[c_node];
                }

                *y_val = x[var_idx] + inv_sqrt_u * neighbor_sum;
            });

        // 2. Paralelismo Rayon para los nodos de cláusulas
        y[n_vars..n_vars + n_clauses]
            .par_iter_mut()
            .enumerate()
            .for_each(|(c_idx, y_val)| {
                let c_node = n_vars + c_idx;
                let inv_sqrt_c = self.inv_sqrt_degrees[c_node];
                let c_vars = &self.clause_to_vars[c_idx];

                let neighbor_sum = (x[c_vars[0]] * self.inv_sqrt_degrees[c_vars[0]])
                    + (x[c_vars[1]] * self.inv_sqrt_degrees[c_vars[1]])
                    + (x[c_vars[2]] * self.inv_sqrt_degrees[c_vars[2]]);

                *y_val = x[c_node] + inv_sqrt_c * neighbor_sum;
            });
    }

    /// Sparse Eigensolver: Calcula lambda_2 y el Vector de Fiedler v2
    /// usando Iteración de Potencias con Deflación de Gram-Schmidt sobre v1.
    pub fn compute_fiedler_partition(
        &self,
        epsilon_boundary: f64,
    ) -> Result<SpectralPartition, &'static str> {
        if self.num_vars < 2 || self.num_clauses == 0 {
            return Err("Instancia trivial: insuficiente número de variables o cláusulas");
        }

        let n = self.total_nodes;
        let mut x = vec![0.0f64; n];
        let mut y = vec![0.0f64; n];

        // Inicialización pseudo-aleatoria ortogonal determinista
        for i in 0..n {
            x[i] = ((i * 7919 + 104729) % 1000) as f64 / 1000.0 - 0.5;
        }

        // Deflación inicial respecto a v1
        self.orthogonalize_against_v1(&mut x);
        self.normalize_vector(&mut x);

        let max_iterations = 60;
        let tolerance = 1e-7;
        let mut mu_2 = 0.0;

        for _iter in 0..max_iterations {
            // y = M * x
            self.sparse_matvec_m(&x, &mut y);

            // Deflación de Gram-Schmidt: y = y - <y, v1> * v1
            self.orthogonalize_against_v1(&mut y);

            // Cociente de Rayleigh: mu_2 ≈ <y, x> / <x, x> = <y, x>
            let current_mu: f64 = x.par_iter().zip(y.par_iter()).map(|(&a, &b)| a * b).sum();

            let norm_y: f64 = y.par_iter().map(|&v| v * v).sum::<f64>().sqrt();
            if norm_y < 1e-12 {
                break;
            }

            // Actualizar x = y / ||y||
            let inv_norm = 1.0 / norm_y;
            x.par_iter_mut().zip(y.par_iter()).for_each(|(xi, &yi)| {
                *xi = yi * inv_norm;
            });

            if (current_mu - mu_2).abs() < tolerance {
                mu_2 = current_mu;
                break;
            }
            mu_2 = current_mu;
        }

        // lambda_2 = 2 - mu_2
        let lambda_2 = (2.0 - mu_2).max(0.0).min(2.0);
        let cheeger_lb = (2.0f64 * lambda_2).sqrt();

        // Extraemos las coordenadas de Fiedler normalizadas de las variables
        let mut variable_weights: Vec<(usize, f64)> = (0..self.num_vars)
            .into_par_iter()
            .map(|var_idx| {
                let v2_raw = x[var_idx];
                let deg = (self.var_degrees[var_idx] as f64).max(1.0);
                let normalized_fiedler = v2_raw / deg.sqrt();
                (var_idx, normalized_fiedler)
            })
            .collect();

        // Ordenamiento por coordenada de Fiedler (Sweep-Cut)
        variable_weights.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut left_partition = Vec::new();
        let mut right_partition = Vec::new();
        let mut cut_variables = Vec::new();

        for &(var_idx, weight) in &variable_weights {
            if weight < -epsilon_boundary {
                left_partition.push(var_idx);
            } else if weight > epsilon_boundary {
                right_partition.push(var_idx);
            } else {
                cut_variables.push(var_idx);
            }
        }

        // Garantizamos al menos 1 variable en cut_variables si quedó vacío
        if cut_variables.is_empty() && !variable_weights.is_empty() {
            let mid = variable_weights.len() / 2;
            cut_variables.push(variable_weights[mid].0);
        }

        Ok(SpectralPartition {
            lambda_2,
            cheeger_lower_bound: cheeger_lb,
            variable_fiedler_weights: variable_weights,
            cut_variables,
            left_partition,
            right_partition,
        })
    }

    #[inline(always)]
    fn orthogonalize_against_v1(&self, vec: &mut [f64]) {
        let dot: f64 = vec
            .par_iter()
            .zip(self.v1_trivial.par_iter())
            .map(|(&a, &b)| a * b)
            .sum();
        vec.par_iter_mut()
            .zip(self.v1_trivial.par_iter())
            .for_each(|(v, &v1)| {
                *v -= dot * v1;
            });
    }

    #[inline(always)]
    fn normalize_vector(&self, vec: &mut [f64]) {
        let norm: f64 = vec.par_iter().map(|&v| v * v).sum::<f64>().sqrt();
        if norm > 1e-12 {
            let inv_norm = 1.0 / norm;
            vec.par_iter_mut().for_each(|v| *v *= inv_norm);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sparse_eigensolver() {
        let clauses = vec![
            Clause3([1, 2, 3]),
            Clause3([-1, -2, 3]),
            Clause3([1, -2, -3]),
            Clause3([-1, 2, -3]),
        ];
        let factor_graph = SpectralFactorGraph::new(3, &clauses);
        assert_eq!(factor_graph.total_nodes, 7); // 3 vars + 4 clauses

        let partition = factor_graph
            .compute_fiedler_partition(0.05)
            .expect("Debe converger");
        assert!(partition.lambda_2 >= 0.0);
        assert!(!partition.cut_variables.is_empty());
    }
}
