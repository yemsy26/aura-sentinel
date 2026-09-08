// src/benchmark.rs
//! Módulo de Benchmarking y Stress Test de Escalabilidad Asintótica
//!
//! Monitorea la evolución de lambda_2, la cota de Cheeger y el ratio de corte k/n
//! en instancias crecientes (n = 20 hasta n = 100).

use std::io::{self, Write};
use std::time::Instant;
use crate::generator::generate_planted_3sat;
use crate::simd_eval::{ParallelSpectralSolver, SimdFormula};
use crate::spectral::SpectralFactorGraph;

/// Registro de métricas de una instancia en el benchmark de escalado
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ScalingMetric {
    pub n_vars: usize,
    pub m_clauses: usize,
    pub lambda_2: f64,
    pub cheeger_lb: f64,
    pub cut_k: usize,
    pub cut_ratio: f64,
    pub left_vars: usize,
    pub right_vars: usize,
    pub spectral_time_us: u128,
    pub solve_time_ms: f64,
    pub brute_time_ms: Option<f64>,
    pub speedup: Option<f64>,
    pub is_sat: bool,
}

/// Ejecuta el bucle de escalado asintótico y monitoreo del Spectral Gap
pub fn run_scaling_benchmark(vars_list: &[usize], ratio: f64, epsilon: f64) -> Vec<ScalingMetric> {
    println!("\n==========================================================================================");
    println!("        BENCHMARK DE ESCALADO ASINTÓTICO Y EVOLUCIÓN DEL SPECTRAL GAP (TC^0-SAT)         ");
    println!("==========================================================================================");
    println!(" Ratio Cláusula/Variable: {:.2} | Epsilon de Frontera: {:.3}", ratio, epsilon);
    println!("------------------------------------------------------------------------------------------");
    println!(
        "{:>4} | {:>5} | {:>8} | {:>8} | {:>4} | {:>6} | {:>10} | {:>10} | {:>8}",
        "n", "m", "lambda_2", "Cheeger", "k", "k/n %", "T_Eigensv", "T_Spectral", "Speedup"
    );
    println!("------------------------------------------------------------------------------------------");
    io::stdout().flush().unwrap();

    let mut metrics = Vec::new();

    for &n in vars_list {
        let m = ((n as f64) * ratio).round() as usize;

        // 1. Generamos instancia plantada
        let (clauses, _) = generate_planted_3sat(n, m, Some(42 + n as u64));

        // 2. Medición del Sparse Eigensolver
        let t_spec_start = Instant::now();
        let factor_graph = SpectralFactorGraph::new(n, &clauses);
        let partition = match factor_graph.compute_fiedler_partition(epsilon) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Error en n={}: {}", n, e);
                continue;
            }
        };
        let spectral_time_us = t_spec_start.elapsed().as_micros();

        // 3. Empaquetado SIMD
        let simd_formula = SimdFormula::from_clauses(n, &clauses);
        let solver = ParallelSpectralSolver::new(&simd_formula, &partition);

        // 4. Resolución Espectral
        let t_solve_start = Instant::now();
        let res_spectral = solver.solve();
        let solve_time_ms = t_solve_start.elapsed().as_secs_f64() * 1000.0;
        let is_sat = res_spectral.is_some();

        // 5. Comparativa con Fuerza Bruta (solo para n <= 24 para evitar congelamiento de la CPU)
        let (brute_time_ms, speedup) = if n <= 24 {
            let t_brute_start = Instant::now();
            let _ = solver.fallback_bruteforce();
            let b_ms = t_brute_start.elapsed().as_secs_f64() * 1000.0;
            let sp = b_ms / solve_time_ms.max(0.0001);
            (Some(b_ms), Some(sp))
        } else {
            (None, None)
        };

        let cut_k = partition.cut_variables.len();
        let cut_ratio = (cut_k as f64 / n as f64) * 100.0;

        let metric = ScalingMetric {
            n_vars: n,
            m_clauses: m,
            lambda_2: partition.lambda_2,
            cheeger_lb: partition.cheeger_lower_bound,
            cut_k,
            cut_ratio,
            left_vars: partition.left_partition.len(),
            right_vars: partition.right_partition.len(),
            spectral_time_us,
            solve_time_ms,
            brute_time_ms,
            speedup,
            is_sat,
        };

        let speedup_str = match speedup {
            Some(s) => format!("{:.2}x", s),
            None => "N/A (>2^26)".to_string(),
        };

        println!(
            "{:>4} | {:>5} | {:>8.4} | {:>8.4} | {:>4} | {:>5.1}% | {:>8} µs | {:>7.2} ms | {:>8}",
            n,
            m,
            partition.lambda_2,
            partition.cheeger_lower_bound,
            cut_k,
            cut_ratio,
            spectral_time_us,
            solve_time_ms,
            speedup_str
        );
        io::stdout().flush().unwrap();

        metrics.push(metric);
    }

    println!("==========================================================================================\n");
    metrics
}
