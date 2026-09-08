use crate::spectral::Clause3;
use std::collections::HashSet;

pub struct ChordalExtension {
    adjacency: Vec<HashSet<usize>>,
}

impl ChordalExtension {
    pub fn new(n_vars: usize, clauses: &[Clause3]) -> Self {
        let mut adjacency = vec![HashSet::new(); n_vars];
        for clause in clauses {
            let v1 = clause.0[0].unsigned_abs() as usize - 1;
            let v2 = clause.0[1].unsigned_abs() as usize - 1;
            let v3 = clause.0[2].unsigned_abs() as usize - 1;

            adjacency[v1].insert(v2);
            adjacency[v1].insert(v3);
            adjacency[v2].insert(v1);
            adjacency[v2].insert(v3);
            adjacency[v3].insert(v1);
            adjacency[v3].insert(v2);
        }
        Self { adjacency }
    }

    /// Approximate Minimum Degree (AMD) Heurístico
    /// Retorna los Cliques Maximales de la extensión cordal
    pub fn extract_maximal_cliques(&mut self) -> Vec<Vec<usize>> {
        let n = self.adjacency.len();
        let mut active = vec![true; n];
        let mut cliques = Vec::new();

        for _ in 0..n {
            let mut min_deg = usize::MAX;
            let mut v_min = 0;
            let mut found_active = false;

            for v in 0..n {
                if active[v] {
                    let deg = self.adjacency[v].iter().filter(|&&u| active[u]).count();
                    if deg < min_deg {
                        min_deg = deg;
                        v_min = v;
                        found_active = true;
                    }
                }
            }

            if !found_active {
                break;
            }

            let mut clique = vec![v_min];
            for &u in &self.adjacency[v_min] {
                if active[u] {
                    clique.push(u);
                }
            }
            cliques.push(clique.clone());
            active[v_min] = false;

            // Fill-in para Chordalidad
            for i in 1..clique.len() {
                for j in (i + 1)..clique.len() {
                    let (u, w) = (clique[i], clique[j]);
                    self.adjacency[u].insert(w);
                    self.adjacency[w].insert(u);
                }
            }
        }

        let cliques_clone = cliques.clone();
        cliques.retain(|c1| {
            !cliques_clone
                .iter()
                .any(|c2| c1 != c2 && c1.iter().all(|x| c2.contains(x)))
        });
        cliques
    }
}
