// src/dimacs.rs
//! Parser de alto rendimiento para archivos en formato DIMACS CNF (.cnf)
//!
//! Lee la descripción estándar de fórmulas booleanas ignorando comentarios ('c'),
//! extrayendo el encabezado 'p cnf <vars> <clauses>' y empaquetando cada cláusula
//! en la estructura `Clause3`.

use crate::spectral::Clause3;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Estructura contenedora del resultado de parseo DIMACS
#[derive(Debug, Clone)]
pub struct DimacsFormula {
    pub num_vars: usize,
    pub num_clauses: usize,
    pub clauses: Vec<Clause3>,
}

/// Parsea un archivo DIMACS CNF desde una ruta del sistema de archivos
pub fn parse_file<P: AsRef<Path>>(path: P) -> Result<DimacsFormula, String> {
    let file = File::open(path.as_ref())
        .map_err(|e| format!("Error al abrir archivo DIMACS {:?}: {}", path.as_ref(), e))?;
    let reader = BufReader::new(file);
    parse_reader(reader)
}

/// Parsea el contenido DIMACS CNF desde una cadena en memoria
#[allow(dead_code)]
pub fn parse_str(content: &str) -> Result<DimacsFormula, String> {
    let reader = BufReader::new(content.as_bytes());
    parse_reader(reader)
}

/// Lógica de lectura compartida basada en BufRead
fn parse_reader<R: BufRead>(reader: R) -> Result<DimacsFormula, String> {
    let mut num_vars: Option<usize> = None;
    let mut expected_clauses: Option<usize> = None;
    let mut clauses = Vec::new();
    let mut current_literals = Vec::with_capacity(3);

    for (line_num, line_result) in reader.lines().enumerate() {
        let line = line_result.map_err(|e| format!("Error en línea {}: {}", line_num + 1, e))?;
        let trimmed = line.trim();

        // 1. Ignorar líneas vacías y comentarios que inician con 'c'
        if trimmed.is_empty() || trimmed.starts_with('c') {
            continue;
        }

        // 2. Parsear línea de encabezado 'p cnf <vars> <clauses>'
        if trimmed.starts_with('p') {
            let tokens: Vec<&str> = trimmed.split_whitespace().collect();
            if tokens.len() < 4 || tokens[1] != "cnf" {
                return Err(format!(
                    "Encabezado DIMACS inválido en línea {}: '{}'",
                    line_num + 1,
                    trimmed
                ));
            }

            let vars: usize = tokens[2]
                .parse()
                .map_err(|_| format!("Número de variables inválido en línea {}", line_num + 1))?;
            let cls: usize = tokens[3]
                .parse()
                .map_err(|_| format!("Número de cláusulas inválido en línea {}", line_num + 1))?;

            num_vars = Some(vars);
            expected_clauses = Some(cls);
            clauses.reserve(cls);
            continue;
        }

        // 3. Parsear literales de cláusulas (enteros separados por espacios que terminan en '0')
        for token in trimmed.split_whitespace() {
            let lit: i32 = token
                .parse()
                .map_err(|_| format!("Token no numérico '{}' en línea {}", token, line_num + 1))?;

            if lit == 0 {
                // Fin de cláusula encontrado
                if current_literals.is_empty() {
                    continue;
                }

                // Normalización a 3-SAT
                match current_literals.len() {
                    3 => {
                        clauses.push(Clause3([
                            current_literals[0],
                            current_literals[1],
                            current_literals[2],
                        ]));
                    }
                    2 => {
                        clauses.push(Clause3([
                            current_literals[0],
                            current_literals[1],
                            current_literals[1],
                        ]));
                    }
                    1 => {
                        clauses.push(Clause3([
                            current_literals[0],
                            current_literals[0],
                            current_literals[0],
                        ]));
                    }
                    len => {
                        return Err(format!(
                            "Cláusula con {} literales detectada. Este motor está optimizado para 3-SAT.",
                            len
                        ));
                    }
                }
                current_literals.clear();
            } else {
                current_literals.push(lit);
            }
        }
    }

    let actual_vars = num_vars.unwrap_or_else(|| {
        clauses
            .iter()
            .flat_map(|c| c.0.iter())
            .map(|l| l.unsigned_abs() as usize)
            .max()
            .unwrap_or(0)
    });

    let final_num_clauses = clauses.len();
    if let Some(expected) = expected_clauses {
        if expected != final_num_clauses {
            eprintln!(
                "Advertencia DIMACS: Se esperaban {} cláusulas, pero se leyeron {}.",
                expected, final_num_clauses
            );
        }
    }

    Ok(DimacsFormula {
        num_vars: actual_vars,
        num_clauses: final_num_clauses,
        clauses,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_dimacs() {
        let raw = r#"
        c Ejemplo de archivo 3-SAT de prueba
        p cnf 4 3
        1 2 -3 0
        -1 -2 4 0
        2 3 -4 0
        "#;

        let formula = parse_str(raw).expect("Debe parsear correctamente");
        assert_eq!(formula.num_vars, 4);
        assert_eq!(formula.num_clauses, 3);
        assert_eq!(formula.clauses[0], Clause3([1, 2, -3]));
        assert_eq!(formula.clauses[1], Clause3([-1, -2, 4]));
        assert_eq!(formula.clauses[2], Clause3([2, 3, -4]));
    }
}
