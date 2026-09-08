use rand::Rng;
use std::time::Instant;

use spectrasat_core::chordal::ChordalExtension;
use spectrasat_core::gf2_elimination::Gf2System;
use spectrasat_core::sdp_solver::{solve_sos_sdp, SdpVerdict};
use spectrasat_core::spectral::Clause3;

fn main() {
    println!("\n🚀 INICIANDO SPECTRASAT BENCHMARK SUITE (MODO ESTRÉS) 🚀\n");

    
    let (n1, clauses1) = gen_3sat_pt(20, 85); 
    run_benchmark(
        "Transición de Fase 3-SAT (n=20, m=85) [GF2 ACTIVO]",
        n1,
        &clauses1,
        false,
    );
    run_benchmark(
        "Transición de Fase 3-SAT (n=20, m=85) [BYPASS GF2]",
        n1,
        &clauses1,
        true,
    );

    
    let (n2, clauses2) = gen_php_4_3();
    run_benchmark(
        "Principio del Palomar PHP(4,3) [GF2 ACTIVO]",
        n2,
        &clauses2,
        false,
    );
    run_benchmark(
        "Principio del Palomar PHP(4,3) [BYPASS GF2]",
        n2,
        &clauses2,
        true,
    );

    
    let (n3, clauses3) = gen_tseitin_noise(25, 100);
    run_benchmark(
        "Tseitin con Ruido No Lineal (n=25) [GF2 ACTIVO]",
        n3,
        &clauses3,
        false,
    );
    run_benchmark(
        "Tseitin con Ruido No Lineal (n=25) [BYPASS GF2]",
        n3,
        &clauses3,
        true,
    );
}




fn run_benchmark(name: &str, n_vars: usize, clauses: &[Clause3], bypass_gf2: bool) {
    println!("==================================================");
    println!("🧪 TARGET: {}", name);
    println!("📊 TOPO: {} variables, {} cláusulas", n_vars, clauses.len());

    
    if !bypass_gf2 {
        let t_gf2 = Instant::now();
        let mut gf2 = Gf2System::extract_from_3cnf(n_vars, clauses);
        let is_tseitin = gf2.is_tseitin_unsat();
        let d_gf2 = t_gf2.elapsed();

        println!("🛡️  GF(2) FIREWALL: Ejecutado en {:?}", d_gf2);
        if is_tseitin {
            println!("🔥 VEREDICTO: UNSAT (Demostración Algebraica GF(2))");
            println!("==================================================\n");
            return;
        } else {
            println!("⚠️  GF(2) FIREWALL: Superado sin contradicción lineal.");
        }
    } else {
        println!("🚧 BYPASS: GF(2) desactivado. Forzando motor estructural AMD+SDP.");
    }

    
    let t_amd = Instant::now();
    let mut chordal = ChordalExtension::new(n_vars, clauses);
    let cliques = chordal.extract_maximal_cliques();
    let d_amd = t_amd.elapsed();
    let max_clique = cliques.iter().map(|c| c.len()).max().unwrap_or(0);

    println!(
        "🕸️  AMD CORDAL: {} cliques extraídos. Max tamaño: {} (Tiempo: {:?})",
        cliques.len(),
        max_clique,
        d_amd
    );

    
    let t_admm = Instant::now();
    let (verdict, _) = solve_sos_sdp(n_vars, clauses, false);
    let d_admm = t_admm.elapsed();

    let (v_str, iters, p_res, d_res) = match verdict {
        SdpVerdict::PossibleSat { residual, iters } => {
            println!(
                "⚙️  MOTOR SDP: Convergencia en {} iteraciones (Tiempo: {:?})",
                iters, d_admm
            );
            println!("📉 RESIDUOS: Primal = {:.4e}, Dual = {:.4e}", residual, 0.0);
            println!("⚠️ VEREDICTO SDP: INCONCLUSIVO (Atrapado en Relajación/Pseudo-Expectativas)");

            
            println!("🌲 INICIANDO BRANCH-AND-BOUND (Poda Espectral)...");
            let t_bab = Instant::now();
            let (final_verdict_str, _assignment) =
                spectrasat_core::sdp_branching::branch_and_bound_solve(n_vars, clauses);
            let d_bab = t_bab.elapsed();

            println!("🚀 BÚSQUEDA COMPLETADA en {:?}", d_bab);
            println!("🏁 VEREDICTO FINAL: {}", final_verdict_str);
            println!("==================================================\n");
            return;
        }
        SdpVerdict::ProvenUnsat { divergence, iters } => (
            "UNSAT (Certificado de Positividad SDP)",
            iters,
            divergence,
            0.0,
        ),
        SdpVerdict::Unknown {
            primal_res,
            dual_res,
            iters,
        } => ("DESCONOCIDO / NO CONVERGE", iters, primal_res, dual_res),
    };

    println!(
        "⚙️  MOTOR SDP: Convergencia en {} iteraciones (Tiempo: {:?})",
        iters, d_admm
    );
    println!("📉 RESIDUOS: Primal = {:.4e}, Dual = {:.4e}", p_res, d_res);
    println!("🏁 VEREDICTO FINAL: {}", v_str);
    println!("==================================================\n");
}





fn gen_3sat_pt(n: usize, m: usize) -> (usize, Vec<Clause3>) {
    let mut rng = rand::thread_rng();
    let mut clauses = Vec::new();
    for _ in 0..m {
        let mut c = [0i32; 3];
        for i in 0..3 {
            let var = rng.gen_range(1..=n) as i32;
            let sign = if rng.gen_bool(0.5) { 1 } else { -1 };
            c[i] = var * sign;
        }
        clauses.push(Clause3(c));
    }
    (n, clauses)
}

fn gen_php_4_3() -> (usize, Vec<Clause3>) {
    let p = 4;
    let h = 3;
    let n_vars = p * h;
    let mut clauses = Vec::new();
    let var = |p: usize, h: usize| -> i32 { (p * 3 + h + 1) as i32 };

    for pi in 0..p {
        clauses.push(Clause3([var(pi, 0), var(pi, 1), var(pi, 2)]));
    }
    for hi in 0..h {
        for p1 in 0..p {
            for p2 in (p1 + 1)..p {
                clauses.push(Clause3([-var(p1, hi), -var(p2, hi), -var(p2, hi)]));
            }
        }
    }
    (n_vars, clauses)
}

fn gen_tseitin_noise(n: usize, m: usize) -> (usize, Vec<Clause3>) {
    let mut rng = rand::thread_rng();
    let mut clauses = Vec::new();

    
    clauses.push(Clause3([1, 3, 3]));
    clauses.push(Clause3([1, 2, 2]));
    clauses.push(Clause3([2, 3, 3]));

    for _ in 0..m {
        let mut c = [0i32; 3];
        for i in 0..3 {
            let v = rng.gen_range(4..=n) as i32;
            let sign = if rng.gen_bool(0.5) { 1 } else { -1 };
            c[i] = v * sign;
        }
        clauses.push(Clause3(c));
    }
    (n, clauses)
}
