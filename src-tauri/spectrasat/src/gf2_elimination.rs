use crate::spectral::Clause3;

#[derive(Clone)]
pub struct Gf2System {
    pub rows: Vec<Vec<u64>>, // Bit-packed: cada u64 guarda 64 variables
    pub b: Vec<bool>,        // Resultados de las paridades
    pub cols: usize,
}

impl Gf2System {
    /// Extrae un sistema GF(2) heurísticamente buscando cláusulas que codifiquen paridad
    /// (Para esta PoC asume que todas las cláusulas de 3 variables forman pares de XOR)
    pub fn extract_from_3cnf(n_vars: usize, clauses: &[Clause3]) -> Self {
        // Dummy heurístico: En una implementación real buscaríamos 4 cláusulas exactas
        // que codifican x XOR y XOR z = b. Aquí crearemos una ecuación por cada cláusula
        // ignorando los signos, solo para atrapar dependencias lineales puras.
        let mut rows = Vec::new();
        let mut b = Vec::new();
        let chunks = n_vars.div_ceil(64);

        for clause in clauses {
            let mut row = vec![0u64; chunks];
            for &lit in &clause.0 {
                let var = (lit.unsigned_abs() as usize) - 1;
                row[var / 64] |= 1 << (var % 64);
            }
            // Agregamos la fila simulando b=1 (dependerá de los signos de los literales)
            rows.push(row);
            b.push(true);
        }

        Self {
            rows,
            b,
            cols: n_vars,
        }
    }

    /// Eliminación Gaussiana mod 2 en tiempo O(n^3/64).
    /// Retorna `true` si detecta una contradicción estricta (0 = 1).
    pub fn is_tseitin_unsat(&mut self) -> bool {
        let n_rows = self.rows.len();
        let chunks = self.cols.div_ceil(64);
        let mut pivot_row = 0;

        for c in 0..self.cols {
            let chunk_idx = c / 64;
            let bit_idx = c % 64;

            let mut found = None;
            for r in pivot_row..n_rows {
                if (self.rows[r][chunk_idx] >> bit_idx) & 1 == 1 {
                    found = Some(r);
                    break;
                }
            }

            if let Some(r) = found {
                self.rows.swap(pivot_row, r);
                self.b.swap(pivot_row, r);

                for r2 in 0..n_rows {
                    if r2 != pivot_row && ((self.rows[r2][chunk_idx] >> bit_idx) & 1 == 1) {
                        for k in 0..chunks {
                            self.rows[r2][k] ^= self.rows[pivot_row][k];
                        }
                        self.b[r2] ^= self.b[pivot_row];
                    }
                }
                pivot_row += 1;
            }
        }

        for r in pivot_row..n_rows {
            let row_is_zero = self.rows[r].iter().all(|&chunk| chunk == 0);
            if row_is_zero && self.b[r] {
                return true; // Contradicción GF(2)
            }
        }
        false
    }
}
