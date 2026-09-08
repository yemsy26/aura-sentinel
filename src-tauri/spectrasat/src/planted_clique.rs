use crate::spectral::Clause3;
use rand::Rng;

/// Generador de instancias Planted Clique
/// Genera un grafo ER(N, 0.5) y planta un clique de tamaño K.
/// Traduce el problema de decisión a un sistema 3-CNF compatible con SpectraSAT.
pub fn generate(n: usize, k: usize) -> (usize, Vec<Clause3>) {
    let mut rng = rand::thread_rng();
    let mut adj = vec![vec![false; n]; n];

    // 1. Grafo Aleatorio Erdős-Rényi (p = 0.5)
    for i in 0..n {
        for j in (i + 1)..n {
            if rng.gen_bool(0.5) {
                adj[i][j] = true;
                adj[j][i] = true;
            }
        }
    }

    // 2. Plantar el Clique de tamaño K
    let mut nodes: Vec<usize> = (0..n).collect();
    // Fisher-Yates para elegir K vértices aleatorios
    for i in 0..k {
        let swap_idx = rng.gen_range(i..n);
        nodes.swap(i, swap_idx);
    }
    let clique_nodes = &nodes[0..k];

    // Conectar todos los vértices del clique plantado
    for &u in clique_nodes {
        for &v in clique_nodes {
            if u != v {
                adj[u][v] = true;
            }
        }
    }

    // 3. Traducción SAT (Encoding de Clique -> 3-CNF)
    // Variables principales: x_{c, i} = el c-ésimo nodo del clique es el vértice i
    let var = |c: usize, i: usize| -> i32 { (c * n + i + 1) as i32 };
    let mut num_vars = k * n;
    let mut clauses = Vec::new();

    // Regla A: Cada posición 'c' del clique debe estar ocupada por al menos un vértice 'i'
    for c in 0..k {
        // Reducción Tseitin para cláusulas de tamaño N a 3-CNF
        let mut current_or = var(c, 0);
        for i in 1..(n - 1) {
            num_vars += 1;
            let aux = num_vars as i32;
            // aux = current_or | var(c, i)
            clauses.push(Clause3([-aux, current_or, var(c, i)]));
            clauses.push(Clause3([-current_or, aux, aux]));
            clauses.push(Clause3([-var(c, i), aux, aux]));
            current_or = aux;
        }
        // Último eslabón de la cadena
        clauses.push(Clause3([current_or, var(c, n - 1), var(c, n - 1)]));
    }

    // Regla B: Las no-aristas del grafo NO pueden coexistir en el clique
    for i in 0..n {
        for j in (i + 1)..n {
            if !adj[i][j] {
                // Si NO hay arista
                for c1 in 0..k {
                    for c2 in 0..k {
                        if c1 != c2 {
                            // Convertido a cláusula 3-SAT repitiendo un literal
                            clauses.push(Clause3([-var(c1, i), -var(c2, j), -var(c2, j)]));
                        }
                    }
                }
            }
        }
    }

    // Regla C: Un mismo vértice no puede ocupar múltiples posiciones en el clique
    for i in 0..n {
        for c1 in 0..k {
            for c2 in (c1 + 1)..k {
                clauses.push(Clause3([-var(c1, i), -var(c2, i), -var(c2, i)]));
            }
        }
    }

    (num_vars, clauses)
}
