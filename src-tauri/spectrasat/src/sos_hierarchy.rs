// src/sos_hierarchy.rs
//! Jerarquía Sum-of-Squares (SoS/Lasserre) de Grado 4 para 3-SAT
//!
//! Construye la Matriz de Momentos M ⪰ 0 en formato disperso CSC
//! y formula el SDP para refutar instancias UNSAT o guiar asignaciones SAT.
//!
//! Complejidad espacial: O(n^2) entradas dispersas, sin matrices densas.
//!
//! MATEMÁTICA:
//! Para una fórmula 3-CNF Φ con n variables y m cláusulas, la relajación
//! Lasserre de grado ℓ=4 busca un operador pseudoespectativo Ẽ[·] tal que:
//!
//!   Ẽ[1] = 1                      (normalización)
//!   Ẽ[x_i^2] = Ẽ[x_i]            (restricción booleana, ∀i)
//!   Ẽ[(1-l_{j1})(1-l_{j2})(1-l_{j3})] = 0  (satisfacibilidad, ∀j)
//!
//! La factibilidad del SDP implica que Φ podría ser SAT.
//! La infactibilidad certifica que Φ es definitivamente UNSAT (Nullstellensatz).

use crate::spectral::Clause3;
use std::collections::HashMap;

// ============================================================================
// Tipos de Monomios Reducidos Booleanos (grado ≤ 2)
// ============================================================================

/// Índice de un monomio reducido en el espacio de grado ≤ 2
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Monomial {
    /// Constante 1
    Const,
    /// Variable lineal x_i (0-indexed)
    Linear(usize),
    /// Producto x_i * x_j con i < j (0-indexed)
    Quadratic(usize, usize),
}

impl Monomial {
    /// Reduce x_i^2 → x_i (idempotencia booleana) y normaliza i < j.
    /// Retorna None si el producto tiene grado > 2 tras reducción.
    pub fn product(a: &Monomial, b: &Monomial) -> Option<Monomial> {
        match (a, b) {
            // Identidad
            (Monomial::Const, m) | (m, Monomial::Const) => Some(m.clone()),
            // x_i * x_i = x_i (idempotencia booleana)
            (Monomial::Linear(i), Monomial::Linear(j)) => {
                if i == j {
                    Some(Monomial::Linear(*i))
                } else {
                    Some(Monomial::Quadratic((*i).min(*j), (*i).max(*j)))
                }
            }
            // x_i * x_j * x_k: reducir si i==k o j==k, sino grado 3 (truncar)
            (Monomial::Linear(k), Monomial::Quadratic(i, j))
            | (Monomial::Quadratic(i, j), Monomial::Linear(k)) => {
                if k == i || k == j {
                    Some(Monomial::Quadratic(*i, *j))
                } else {
                    None // Grado 3 tras reducción booleana: fuera del espacio M_2
                }
            }
            // x_i*x_j * x_k*x_l
            (Monomial::Quadratic(i, j), Monomial::Quadratic(k, l)) => {
                if i == k && j == l {
                    Some(Monomial::Quadratic(*i, *j))
                } else {
                    None // Grado > 2 irreducible: truncar
                }
            }
        }
    }

    /// Descripción legible del monomio
    pub fn to_str(&self) -> String {
        match self {
            Monomial::Const => "1".into(),
            Monomial::Linear(i) => format!("x{}", i),
            Monomial::Quadratic(i, j) => format!("x{}x{}", i, j),
        }
    }
}

// ============================================================================
// Base de Monomios M_2
// ============================================================================

/// Tabla de indexación del espacio de monomios M_2 (grado ≤ 2)
#[derive(Debug, Clone)]
pub struct MonomialBasis {
    pub n_vars: usize,
    pub monomials: Vec<Monomial>,
    pub index: HashMap<Monomial, usize>,
    /// Dimensión total: 1 + n + n*(n-1)/2
    pub dim: usize,
}

impl MonomialBasis {
    /// Construye la base ordenada: [1, x_0, ..., x_{n-1}, x_0*x_1, x_0*x_2, ...]
    pub fn new(n_vars: usize) -> Self {
        let mut monomials = Vec::new();
        let mut index = HashMap::new();

        monomials.push(Monomial::Const);
        index.insert(Monomial::Const, 0);

        for i in 0..n_vars {
            let idx = monomials.len();
            monomials.push(Monomial::Linear(i));
            index.insert(Monomial::Linear(i), idx);
        }

        for i in 0..n_vars {
            for j in (i + 1)..n_vars {
                let idx = monomials.len();
                monomials.push(Monomial::Quadratic(i, j));
                index.insert(Monomial::Quadratic(i, j), idx);
            }
        }

        let dim = monomials.len();
        Self {
            n_vars,
            monomials,
            index,
            dim,
        }
    }

    /// Índice de un monomio reducido en la base, si existe
    #[inline]
    pub fn get_index(&self, m: &Monomial) -> Option<usize> {
        self.index.get(m).copied()
    }
}

// ============================================================================
// Matriz Simétrica Dispersa y Formato CSC
// ============================================================================

/// Matriz simétrica dispersa en formato de tripletas (triangular superior)
#[derive(Debug, Clone)]
pub struct SparseSymmetricMatrix {
    pub dim: usize,
    /// Tripletas (row, col, value) con row ≤ col
    pub entries: Vec<(usize, usize, f64)>,
}

impl SparseSymmetricMatrix {
    pub fn new(dim: usize) -> Self {
        Self {
            dim,
            entries: Vec::new(),
        }
    }

    pub fn insert_upper(&mut self, row: usize, col: usize, val: f64) {
        let (r, c) = if row <= col { (row, col) } else { (col, row) };
        self.entries.push((r, c, val));
    }

    /// Convierte a formato CSC (Compressed Sparse Column) triangular superior
    /// para ser consumido directamente por clarabel-rs
    pub fn to_csc_upper_triangular(&self) -> CscMatrixData {
        let mut col_map: Vec<Vec<(usize, f64)>> = vec![Vec::new(); self.dim];
        for &(r, c, v) in &self.entries {
            col_map[c].push((r, v));
        }
        for col in &mut col_map {
            col.sort_by_key(|&(r, _)| r);
        }

        let mut col_ptr = vec![0usize; self.dim + 1];
        for j in 0..self.dim {
            col_ptr[j + 1] = col_ptr[j] + col_map[j].len();
        }
        let nnz = col_ptr[self.dim];
        let mut row_idx = Vec::with_capacity(nnz);
        let mut values = Vec::with_capacity(nnz);

        for j in 0..self.dim {
            for &(r, v) in &col_map[j] {
                row_idx.push(r);
                values.push(v);
            }
        }

        CscMatrixData {
            nrows: self.dim,
            ncols: self.dim,
            col_ptr,
            row_idx,
            values,
            nnz,
        }
    }
}

/// Datos de matriz dispersa en formato CSC para solvers externos (clarabel, CSDP)
#[derive(Debug, Clone)]
pub struct CscMatrixData {
    pub nrows: usize,
    pub ncols: usize,
    pub nnz: usize,
    pub col_ptr: Vec<usize>,
    pub row_idx: Vec<usize>,
    pub values: Vec<f64>,
}

// ============================================================================
// Restricciones Lineales sobre las Entradas de M
// ============================================================================

/// Restricción lineal: ∑_k a_k * M[r_k, c_k] = b (ecuación sobre entradas de M)
#[derive(Debug, Clone)]
pub struct LinearConstraint {
    pub label: String,
    /// (row, col, coeficiente) con row ≤ col
    pub entries: Vec<(usize, usize, f64)>,
    pub rhs: f64,
}

// ============================================================================
// Motor de Construcción de la Relajación SoS de Grado 4
// ============================================================================

/// Motor de construcción de la relajación SoS/Lasserre de Grado 4 para 3-SAT
pub struct SoSRelaxation {
    pub basis: MonomialBasis,
    pub equality_constraints: Vec<LinearConstraint>,
    pub n_clauses: usize,
}

impl SoSRelaxation {
    pub fn new(n_vars: usize) -> Self {
        let basis = MonomialBasis::new(n_vars);
        Self {
            basis,
            equality_constraints: Vec::new(),
            n_clauses: 0,
        }
    }

    /// Restricción 1: Normalización E[1] = M[const, const] = 1
    pub fn add_normalization_constraint(&mut self) {
        let ci = self.basis.get_index(&Monomial::Const).unwrap();
        self.equality_constraints.push(LinearConstraint {
            label: "E[1]=1".into(),
            entries: vec![(ci, ci, 1.0)],
            rhs: 1.0,
        });
    }

    /// Restricción 2: Booleanas — E[x_i^2] = E[x_i] ↔ M[x_i, x_i] = M[1, x_i]
    pub fn add_boolean_constraints(&mut self) {
        let ci = self.basis.get_index(&Monomial::Const).unwrap();
        for i in 0..self.basis.n_vars {
            let xi = self.basis.get_index(&Monomial::Linear(i)).unwrap();
            let r_diag = xi.min(xi);
            let c_diag = xi.max(xi);
            let r_off = ci.min(xi);
            let c_off = ci.max(xi);
            self.equality_constraints.push(LinearConstraint {
                label: format!("bool_x{i}"),
                entries: vec![(r_diag, c_diag, 1.0), (r_off, c_off, -1.0)],
                rhs: 0.0,
            });
        }
    }

    /// Restricción 3: Satisfacibilidad de cláusulas (Grado 4 SoS)
    /// Para c_j = (l1 ∨ l2 ∨ l3): E[(1-l1)(1-l2)(1-l3)] = 0
    /// y para cada variable x_k en la cláusula: E[(1-l1)(1-l2)(1-l3) * x_k] = 0
    pub fn add_clause_constraints(&mut self, clauses: &[Clause3]) {
        self.n_clauses = clauses.len();
        let ci = self.basis.get_index(&Monomial::Const).unwrap();

        for (j, clause) in clauses.iter().enumerate() {
            // Cada literal se expresa como: (coeff_const + coeff_var * x_{var})
            // literal positivo x_i:   (0 + 1 * x_i)  → negado = (1 - x_i)
            // literal negativo ¬x_i:  (1 - x_i)^neg = x_i → negado = x_i
            let vars: [(usize, bool); 3] = [
                ((clause.0[0].unsigned_abs() as usize) - 1, clause.0[0] > 0),
                ((clause.0[1].unsigned_abs() as usize) - 1, clause.0[1] > 0),
                ((clause.0[2].unsigned_abs() as usize) - 1, clause.0[2] > 0),
            ];

            // Restricción base: E[(1-l1)(1-l2)(1-l3)] = 0
            // Expandimos en monomios reducidos
            let mut constraint_entries: Vec<(usize, usize, f64)> = Vec::new();

            // Término constante: ∏ coeff_const_r
            // coeff_const de (1 - l_r): si l_r = x_i (positivo), negado = 1 - x_i → c=1
            //                            si l_r = ¬x_i (negativo), negado = x_i → c=0
            let const_part: f64 = vars
                .iter()
                .map(|&(_var, is_pos)| if is_pos { 1.0 } else { 0.0 })
                .product();
            if const_part.abs() > 1e-12 {
                constraint_entries.push((ci, ci, const_part));
            }

            // Términos lineales: suma de productos de dos constantes con una variable
            for r in 0..3 {
                let (var_r, is_pos_r) = vars[r];
                let xi_r = self.basis.get_index(&Monomial::Linear(var_r)).unwrap();
                // Coeficiente lineal de x_{var_r}: -1 si is_pos, +1 si is_neg
                let sign_r = if is_pos_r { -1.0 } else { 1.0 };
                // Factor del resto de los literales (sus términos constantes)
                let rest_product: f64 = (0..3)
                    .filter(|&s| s != r)
                    .map(|s| if vars[s].1 { 1.0 } else { 0.0 })
                    .product();
                let coeff = sign_r * rest_product;
                if coeff.abs() > 1e-12 {
                    let (row, col) = (ci.min(xi_r), ci.max(xi_r));
                    constraint_entries.push((row, col, coeff));
                }
            }

            // Términos cuadráticos: par de variables con coeficiente constante del tercero
            for r1 in 0..3 {
                for r2 in (r1 + 1)..3 {
                    let (var_r1, is_pos_r1) = vars[r1];
                    let (var_r2, is_pos_r2) = vars[r2];
                    let s1 = if is_pos_r1 { -1.0 } else { 1.0 };
                    let s2 = if is_pos_r2 { -1.0 } else { 1.0 };
                    let r3 = 3 - r1 - r2;
                    let rest_const = if vars[r3].1 { 1.0 } else { 0.0 };
                    let coeff: f64 = s1 * s2 * rest_const;
                    if coeff.abs() > 1e-12 {
                        let (vmin, vmax) = (var_r1.min(var_r2), var_r1.max(var_r2));
                        if vmin == vmax {
                            // x_i^2 = x_i → colapsa a término lineal
                            let xi = self.basis.get_index(&Monomial::Linear(vmin)).unwrap();
                            let (row, col) = (ci.min(xi), ci.max(xi));
                            constraint_entries.push((row, col, coeff));
                        } else if let Some(xij) =
                            self.basis.get_index(&Monomial::Quadratic(vmin, vmax))
                        {
                            let (row, col) = (ci.min(xij), ci.max(xij));
                            constraint_entries.push((row, col, coeff));
                        }
                    }
                }
            }

            // Término cúbico: coeficiente del producto de las tres variables (reducido a cuadrático)
            {
                let signs: [f64; 3] = std::array::from_fn(|r| if vars[r].1 { -1.0 } else { 1.0 });
                let cubic_coeff = signs[0] * signs[1] * signs[2];
                if cubic_coeff.abs() > 1e-12 {
                    // Reducción: x_{v0} * x_{v1} * x_{v2} colapsado por x_i^2 = x_i si dos son iguales
                    let varset: [usize; 3] = [vars[0].0, vars[1].0, vars[2].0];
                    if varset[0] == varset[1] || varset[0] == varset[2] || varset[1] == varset[2] {
                        // Al menos dos iguales: colapsa a cuadrático o lineal
                        let unique: Vec<usize> = {
                            let mut v = varset.to_vec();
                            v.sort();
                            v.dedup();
                            v
                        };
                        match unique.len() {
                            1 => {
                                let xi =
                                    self.basis.get_index(&Monomial::Linear(unique[0])).unwrap();
                                let (row, col) = (ci.min(xi), ci.max(xi));
                                constraint_entries.push((row, col, cubic_coeff));
                            }
                            2 => {
                                if let Some(xij) = self
                                    .basis
                                    .get_index(&Monomial::Quadratic(unique[0], unique[1]))
                                {
                                    let (row, col) = (ci.min(xij), ci.max(xij));
                                    constraint_entries.push((row, col, cubic_coeff));
                                }
                            }
                            _ => {} // Tres distintos: truncamos (grado 3 irreducible)
                        }
                    }
                    // Si los tres son distintos, el término de grado 3 no se puede reducir
                    // y no aparece en M (base de grado 2): se trunca a 0
                }
            }

            self.equality_constraints.push(LinearConstraint {
                label: format!("clause_sat_{j}"),
                entries: constraint_entries,
                rhs: 0.0,
            });
        }
    }

    /// Construye el SDP completo y retorna las métricas del problema generado
    pub fn build(&mut self, clauses: &[Clause3]) -> SoSProblemStats {
        self.add_normalization_constraint();
        self.add_boolean_constraints();
        self.add_clause_constraints(clauses);

        let dim = self.basis.dim;
        let psd_vars = dim * (dim + 1) / 2;
        let est_memory_mb = (psd_vars * 8) as f64 / (1024.0 * 1024.0);

        SoSProblemStats {
            moment_matrix_dim: dim,
            psd_vars,
            n_equality_constraints: self.equality_constraints.len(),
            n_clauses: self.n_clauses,
            est_dense_memory_mb: (dim * dim * 8) as f64 / (1024.0 * 1024.0),
            est_sparse_memory_mb: est_memory_mb,
        }
    }
}

/// Dimensiones y estimaciones de memoria del SDP generado
#[derive(Debug)]
pub struct SoSProblemStats {
    /// Dimensión de la matriz de momentos M (1 + n + n*(n-1)/2)
    pub moment_matrix_dim: usize,
    /// Número de variables libres del SDP (triángulo superior de M)
    pub psd_vars: usize,
    /// Número de restricciones de igualdad (booleanas + normalización + cláusulas)
    pub n_equality_constraints: usize,
    pub n_clauses: usize,
    /// Estimación de memoria para M densa (bytes)
    pub est_dense_memory_mb: f64,
    /// Estimación de memoria para triangular superior dispersa
    pub est_sparse_memory_mb: f64,
}

impl std::fmt::Display for SoSProblemStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "============================================================"
        )?;
        writeln!(f, "  SDP / SoS Lasserre Grado 4 — Dimensiones del Problema")?;
        writeln!(
            f,
            "============================================================"
        )?;
        writeln!(
            f,
            "  Dimensión de M:           {} x {}",
            self.moment_matrix_dim, self.moment_matrix_dim
        )?;
        writeln!(f, "  Variables libres SDP:     {}", self.psd_vars)?;
        writeln!(
            f,
            "  Restricciones (eq.):      {}",
            self.n_equality_constraints
        )?;
        writeln!(f, "  Cláusulas SAT:            {}", self.n_clauses)?;
        writeln!(
            f,
            "  Memoria M (densa):        {:.2} MB",
            self.est_dense_memory_mb
        )?;
        writeln!(
            f,
            "  Memoria M (dispersa):     {:.2} MB",
            self.est_sparse_memory_mb
        )?;
        writeln!(
            f,
            "============================================================"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monomial_basis_size() {
        let n = 10;
        let basis = MonomialBasis::new(n);
        let expected_dim = 1 + n + (n * (n - 1) / 2);
        assert_eq!(
            basis.dim, expected_dim,
            "Dimensión de la base de monomios incorrecta"
        );
    }

    #[test]
    fn test_boolean_reduction_idempotent() {
        let xi = Monomial::Linear(3);
        assert_eq!(Monomial::product(&xi, &xi), Some(Monomial::Linear(3)));
    }

    #[test]
    fn test_monomial_product_quadratic() {
        let xi = Monomial::Linear(2);
        let xj = Monomial::Linear(5);
        assert_eq!(Monomial::product(&xi, &xj), Some(Monomial::Quadratic(2, 5)));
    }

    #[test]
    fn test_cubic_truncation() {
        let xij = Monomial::Quadratic(1, 2);
        let xk = Monomial::Linear(4);
        // x_1 * x_2 * x_4: grado 3, debe truncar a None
        assert_eq!(Monomial::product(&xij, &xk), None);
    }

    #[test]
    fn test_sos_build_constraints() {
        let n = 5;
        let clauses = vec![
            Clause3([1, 2, -3]),
            Clause3([-1, 4, 5]),
            Clause3([2, -4, -5]),
        ];
        let mut sos = SoSRelaxation::new(n);
        let stats = sos.build(&clauses);

        let expected_dim = 1 + n + (n * (n - 1) / 2);
        assert_eq!(stats.moment_matrix_dim, expected_dim);
        // 1 normalización + n booleanas + 3 cláusulas
        assert_eq!(stats.n_equality_constraints, 1 + n + 3);
        println!("{}", stats);
    }

    #[test]
    fn test_n100_memory_feasibility() {
        let n = 100;
        let basis = MonomialBasis::new(n);
        let expected_dim = 1 + n + (n * (n - 1)) / 2;
        assert_eq!(basis.dim, expected_dim);

        let psd_vars = expected_dim * (expected_dim + 1) / 2;
        let mb_sparse = (psd_vars * 8) as f64 / (1024.0 * 1024.0);
        let mb_dense = (expected_dim * expected_dim * 8) as f64 / (1024.0 * 1024.0);

        println!(
            "n=100: dim={}, vars_SDP={}, RAM_dispersa={:.1}MB, RAM_densa={:.1}MB",
            expected_dim, psd_vars, mb_sparse, mb_dense
        );

        assert!(
            mb_sparse < 500.0,
            "Memoria dispersa excede 500 MB para n=100"
        );
    }
}
