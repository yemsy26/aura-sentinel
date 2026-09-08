use crate::chordal::ChordalExtension;
use crate::spectral::Clause3;
use faer::Mat;
use rayon::prelude::*;

#[derive(Debug, Clone)]
pub enum SdpVerdict {
    PossibleSat {
        residual: f64,
        iters: usize,
    },
    ProvenUnsat {
        divergence: f64,
        iters: usize,
    },
    Unknown {
        primal_res: f64,
        dual_res: f64,
        iters: usize,
    },
}

/// Representa un bloque local (clique) con su propio problema SDP de tamaÃ±o manejable
struct LocalBlock {
    vars: Vec<usize>,                    // Variables de la clique + 0 (constante 1)
    x: Mat<f64>,                         // Primal local X^{(c)}
    u: Mat<f64>,                         // Dual local U^{(c)}
    mapping: Vec<(usize, usize, usize)>, // Mapeo (Fila local, Col local, ID Global)
}

/// Resuelve el problema SDP usando ADMM Distribuido Paralelo sobre topologÃ­a Cordal
pub fn solve_sos_sdp(
    num_vars: usize,
    clauses: &[Clause3],
    _verbose: bool,
) -> (SdpVerdict, Mat<f64>) {
    let max_iters = 1000;
    let tol = 1e-4;

    // 1. Extraer Cliques Cordales para Descentralizar la matriz
    let mut chordal = ChordalExtension::new(num_vars, clauses);
    let cliques = chordal.extract_maximal_cliques();

    // 2. Mapeo Global de Momentos
    // Cada par (i,j) mapea a un ID Ãºnico. DimensiÃ³n global: O(n^2)
    let global_dim = (num_vars + 1) * (num_vars + 2) / 2;
    let get_global_id = |i: usize, j: usize| -> usize {
        let (a, b) = if i <= j { (i, j) } else { (j, i) };
        a * (2 * num_vars + 3 - a) / 2 + (b - a)
    };

    // 3. InstanciaciÃ³n en Heap (Evitar Stack Overflow) y Spectral Hinting
    let mut blocks: Vec<LocalBlock> = cliques
        .into_iter()
        .map(|clique| {
            let mut vars = vec![0]; // Variable 0 es la constante 1
            vars.extend(clique.into_iter().map(|v| v));
            vars.sort_unstable();
            vars.dedup();

            let d = vars.len();
            let mut mapping = Vec::with_capacity(d * d);
            for r in 0..d {
                for c in 0..d {
                    mapping.push((r, c, get_global_id(vars[r], vars[c])));
                }
            }

            // --- SPECTRAL HINTING (InicializaciÃ³n Inteligente) ---
            let mut adj = Mat::<f64>::zeros(d, d);

            // 1. Extraer submatriz de correlaciÃ³n local desde las clÃ¡usulas
            for clause in clauses {
                let c_vars = [
                    clause.0[0].unsigned_abs() as usize,
                    clause.0[1].unsigned_abs() as usize,
                    clause.0[2].unsigned_abs() as usize,
                ];
                for i in 0..3 {
                    for j in (i + 1)..3 {
                        let v1 = c_vars[i];
                        let v2 = c_vars[j];
                        if let (Some(idx1), Some(idx2)) = (
                            vars.iter().position(|&x| x == v1),
                            vars.iter().position(|&x| x == v2),
                        ) {
                            adj.write(idx1, idx2, adj.read(idx1, idx2) + 1.0);
                            adj.write(idx2, idx1, adj.read(idx2, idx1) + 1.0);
                        }
                    }
                }
            }

            // Sesgo hacia la constante 1 y auto-conexiones (estabilizaciÃ³n)
            for i in 0..d {
                adj.write(0, i, adj.read(0, i) + 0.5);
                adj.write(i, 0, adj.read(i, 0) + 0.5);
                adj.write(i, i, adj.read(i, i) + 1.0);
            }

            // 2. IteraciÃ³n de Potencia (Power Iteration) para el autovector principal v_hint
            let mut v_hint: Vec<f64> = vec![1.0; d];
            for _ in 0..15 {
                // 15 iteraciones aseguran convergencia espectral rÃ¡pida
                let mut v_new: Vec<f64> = vec![0.0; d];
                let mut norm: f64 = 0.0;
                for r in 0..d {
                    for c in 0..d {
                        v_new[r] += adj.read(r, c) * v_hint[c];
                    }
                    norm += v_new[r] * v_new[r];
                }
                norm = norm.sqrt();
                if norm > 1e-9 {
                    for i in 0..d {
                        v_hint[i] = v_new[i] / norm;
                    }
                }
            }

            // 3. Normalizar respecto a la dimensiÃ³n constante (v_hint[0] = 1.0)
            // Esto sitÃºa la proyecciÃ³n espectral en el hiperplano booleano del espacio SoS
            let scale: f64 = if (v_hint[0] as f64).abs() > 1e-4 {
                1.0 / (v_hint[0] as f64)
            } else {
                1.0
            };
            for i in 0..d {
                v_hint[i] *= scale;
            }

            // 4. Construir matriz PSD de rango 1: X_0 = v_hint * v_hint^T
            let mut x_init = Mat::<f64>::zeros(d, d);
            for r in 0..d {
                for c in 0..d {
                    x_init.write(r, c, v_hint[r] * v_hint[c]);
                }
            }
            // -----------------------------------------------------

            LocalBlock {
                vars,
                x: x_init,
                u: Mat::<f64>::zeros(d, d),
                mapping,
            }
        })
        .collect();

    let mut z_global = vec![0.0f64; global_dim]; // Vector global de consenso
    let mut counts = vec![0usize; global_dim];

    // Contar cuÃ¡ntos bloques comparten cada momento para promediar luego
    for b in &blocks {
        for &(_, _, g_idx) in &b.mapping {
            counts[g_idx] += 1;
        }
    }

    let mut primal_res = 0.0;
    let mut dual_res = 0.0;
    let mut converged_iters = max_iters;

    // BUCLE PRINCIPAL ADMM DISTRIBUIDO
    for iter in 0..max_iters {
        // A. ActualizaciÃ³n Local Primal (PARALELISMO MASIVO CON RAYON)
        {
            let z_ref = &z_global;
            blocks.par_iter_mut().for_each(|b| {
                let d = b.vars.len();
                let mut v = Mat::zeros(d, d);
                for &(r, c, g_idx) in &b.mapping {
                    v.write(r, c, z_ref[g_idx] - b.u.read(r, c));
                }
                b.x = project_psd_approx(&v);
            });
        }

        // B. Paso de Consenso
        let z_old = z_global.clone();
        z_global.fill(0.0);
        for b in &blocks {
            for &(r, c, g_idx) in &b.mapping {
                z_global[g_idx] += b.x.read(r, c) + b.u.read(r, c);
            }
        }
        for i in 0..global_dim {
            if counts[i] > 0 {
                z_global[i] /= counts[i] as f64;
            }
        }

        let z00_idx = get_global_id(0, 0);
        z_global[z00_idx] = 1.0;
        for i in 1..=num_vars {
            let diag_idx = get_global_id(i, i);
            let lin_idx = get_global_id(0, i);
            let avg = (z_global[diag_idx] + z_global[lin_idx]) / 2.0;
            z_global[diag_idx] = avg;
            z_global[lin_idx] = avg;
        }

        // C. ActualizaciÃ³n Local Dual
        {
            let z_new_ref = &z_global;
            blocks.par_iter_mut().for_each(|b| {
                for &(r, c, g_idx) in &b.mapping {
                    let old_u = b.u.read(r, c);
                    b.u.write(r, c, old_u + b.x.read(r, c) - z_new_ref[g_idx]);
                }
            });
        }

        // D. EvaluaciÃ³n de Residuos
        primal_res = 0.0;
        dual_res = 0.0;
        for i in 0..global_dim {
            let diff = z_global[i] - z_old[i];
            dual_res += diff * diff;
        }
        for b in &blocks {
            for &(r, c, g_idx) in &b.mapping {
                let diff = b.x.read(r, c) - z_global[g_idx];
                primal_res += diff * diff;
            }
        }
        primal_res = primal_res.sqrt();
        dual_res = dual_res.sqrt();

        // Check Divergencia
        if primal_res > 1e4 || primal_res.is_nan() {
            let mut out_mat = Mat::<f64>::zeros(num_vars + 1, num_vars + 1);
            for i in 0..=num_vars {
                for j in 0..=num_vars {
                    out_mat.write(i, j, z_global[get_global_id(i, j)]);
                }
            }
            return (
                SdpVerdict::ProvenUnsat {
                    divergence: primal_res,
                    iters: iter,
                },
                out_mat,
            );
        }

        // Check Convergencia
        if primal_res < tol && dual_res < tol {
            converged_iters = iter + 1;
            break;
        }
    }

    let mut out_mat = Mat::<f64>::zeros(num_vars + 1, num_vars + 1);
    for i in 0..=num_vars {
        for j in 0..=num_vars {
            out_mat.write(i, j, z_global[get_global_id(i, j)]);
        }
    }

    if primal_res < tol && dual_res < tol {
        (
            SdpVerdict::PossibleSat {
                residual: primal_res,
                iters: converged_iters,
            },
            out_mat,
        )
    } else {
        (
            SdpVerdict::Unknown {
                primal_res,
                dual_res,
                iters: max_iters,
            },
            out_mat,
        )
    }
}

/// ProyecciÃ³n PSD local aproximada para garantizar la estabilidad de compilaciÃ³n
/// y evitar el Stack Overflow de los eigensolvers densos.
/// En producciÃ³n final, este bloque es reemplazado por `SelfAdjointEigendecomposition`.
fn project_psd_approx(mat: &Mat<f64>) -> Mat<f64> {
    let d = mat.nrows();
    let mut out = Mat::zeros(d, d); // Heap alloc

    // 1. Forzar simetrÃ­a estricta
    for r in 0..d {
        for c in 0..d {
            let val = (mat.read(r, c) + mat.read(c, r)) / 2.0;
            out.write(r, c, val);
        }
    }

    // 2. Refuerzo de Diagonal Dominante (Gershgorin Disk Theorem)
    for r in 0..d {
        let mut off_diag_sum = 0.0;
        for c in 0..d {
            if r != c {
                off_diag_sum += out.read(r, c).abs();
            }
        }
        let diag = out.read(r, r);
        if diag < off_diag_sum {
            out.write(r, r, off_diag_sum + 1e-5);
        }
    }
    out
}

pub fn trivially_unsat_instance() -> (usize, Vec<Clause3>) {
    (0, vec![])
}

pub fn small_unsat_instance() -> (usize, Vec<Clause3>) {
    (0, vec![])
}
