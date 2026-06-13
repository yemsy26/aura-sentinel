//! stub_enforcer.rs — Anti-Stub Code Quality Shield
//! 
//! Inspects code proposed by the LLM BEFORE it is written to disk.
//! Rejects any file that contains stub patterns (empty implementations,
//! TODO markers, or placeholder bodies) and returns detailed feedback
//! so the orchestrator can demand a complete rewrite.

pub struct StubReport {
    pub has_stubs: bool,
    #[allow(dead_code)]
    pub warnings: Vec<String>,
    pub rejection_message: String,
}

/// Detects stub patterns in source code based on file extension.
/// Returns a `StubReport` with details about what was found.
pub fn detect_stubs(content: &str, file_path: &str) -> StubReport {
    let ext = std::path::Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let mut warnings: Vec<String> = Vec::new();

    match ext.as_str() {
        "py" => {
            check_python_stubs(content, &mut warnings);
        }
        "rs" => {
            check_rust_stubs(content, &mut warnings);
        }
        "js" | "ts" => {
            check_js_stubs(content, &mut warnings);
        }
        "go" => {
            check_go_stubs(content, &mut warnings);
        }
        "java" | "kt" => {
            check_java_stubs(content, &mut warnings);
        }
        _ => {
            check_generic_stubs(content, &mut warnings);
        }
    }

    let has_stubs = !warnings.is_empty();
    let rejection_message = if has_stubs {
        format!(
            "[ANTI-STUB ENFORCER] ❌ Código RECHAZADO en '{}'. \
            Se detectaron {} implementaciones vacías o incompletas:\n{}\n\n\
            REGLA ABSOLUTA: DEBES reescribir este archivo con implementaciones REALES y COMPLETAS. \
            PROHIBIDO usar 'pass', 'TODO', funciones vacías, o placeholders. \
            Cada función debe tener lógica funcional real.",
            file_path,
            warnings.len(),
            warnings.iter().enumerate()
                .map(|(i, w)| format!("  {}. {}", i + 1, w))
                .collect::<Vec<_>>()
                .join("\n")
        )
    } else {
        String::new()
    };

    StubReport { has_stubs, warnings, rejection_message }
}

fn check_python_stubs(content: &str, warnings: &mut Vec<String>) {
    let lines: Vec<&str> = content.lines().collect();
    let mut in_function = false;
    let mut function_name = String::new();
    let mut function_line = 0usize;
    let mut body_lines = 0usize;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        // Detect function/class/method definitions
        if trimmed.starts_with("def ") || trimmed.starts_with("async def ") {
            // Save state of previous function
            if in_function && body_lines == 0 {
                warnings.push(format!(
                    "Línea {}: función '{}' no tiene cuerpo implementado",
                    function_line + 1, function_name
                ));
            }
            // Track new function
            in_function = true;
            function_line = i;
            body_lines = 0;
            function_name = trimmed
                .trim_start_matches("async ")
                .trim_start_matches("def ")
                .split('(')
                .next()
                .unwrap_or("desconocida")
                .to_string();
            continue;
        }

        if in_function {
            // Count actual meaningful body lines
            if !trimmed.is_empty() && trimmed != "pass" {
                body_lines += 1;
            }
        }

        // Detect explicit stub patterns regardless of context
        if trimmed == "pass" {
            // Only flag standalone pass (not in control flow like if/try/except)
            let prev_non_empty = lines[..i].iter().rev()
                .find(|l| !l.trim().is_empty())
                .map(|l| l.trim())
                .unwrap_or("");
            if prev_non_empty.starts_with("def ") 
                || prev_non_empty.starts_with("async def ")
                || prev_non_empty.ends_with(':') && (
                    prev_non_empty.starts_with("def ") || 
                    prev_non_empty.starts_with("class ")
                )
            {
                warnings.push(format!(
                    "Línea {}: 'pass' detectado como cuerpo vacío de función o clase",
                    i + 1
                ));
            }
        }

        if trimmed.starts_with("# TODO") || trimmed.starts_with("# todo") {
            warnings.push(format!("Línea {}: marcador TODO no implementado: '{}'", i + 1, trimmed));
        }
        if trimmed.starts_with("# FIXME") || trimmed.starts_with("# fixme") {
            warnings.push(format!("Línea {}: marcador FIXME encontrado: '{}'", i + 1, trimmed));
        }
        if trimmed.starts_with("raise NotImplementedError") {
            warnings.push(format!("Línea {}: NotImplementedError — función no implementada", i + 1));
        }
        if trimmed.starts_with("# implement") || trimmed.starts_with("# Implement") {
            warnings.push(format!("Línea {}: placeholder de implementación: '{}'", i + 1, trimmed));
        }
        if trimmed.starts_with("# draw") || trimmed.starts_with("# Dibujar") {
            // Common chess/game stub: "# Dibujar el tablero y las piezas"
            if i + 1 < lines.len() {
                let next = lines[i + 1].trim();
                if next == "pass" || next.is_empty() {
                    warnings.push(format!(
                        "Línea {}: comentario de placeholder sin implementación real: '{}'",
                        i + 1, trimmed
                    ));
                }
            }
        }
    }
}

fn check_rust_stubs(content: &str, warnings: &mut Vec<String>) {
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.contains("todo!()") {
            warnings.push(format!("Línea {}: todo!() — función no implementada", i + 1));
        }
        if trimmed.contains("unimplemented!()") {
            warnings.push(format!("Línea {}: unimplemented!() detectado", i + 1));
        }
        if trimmed.starts_with("// TODO") || trimmed.starts_with("// todo") {
            warnings.push(format!("Línea {}: TODO sin implementar: '{}'", i + 1, trimmed));
        }
        if trimmed == "{ }" || trimmed == "{}" {
            warnings.push(format!("Línea {}: cuerpo de función vacío detectado", i + 1));
        }
    }
}

fn check_js_stubs(content: &str, warnings: &mut Vec<String>) {
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.contains("throw new Error('not implemented')") 
            || trimmed.contains("throw new Error(\"not implemented\")") {
            warnings.push(format!("Línea {}: función no implementada (throw Error)", i + 1));
        }
        if trimmed.starts_with("// TODO") || trimmed.starts_with("// todo") {
            warnings.push(format!("Línea {}: TODO sin implementar: '{}'", i + 1, trimmed));
        }
        if trimmed == "// implement here" || trimmed == "// implementar aquí" {
            warnings.push(format!("Línea {}: placeholder de implementación: '{}'", i + 1, trimmed));
        }
    }
}

fn check_go_stubs(content: &str, warnings: &mut Vec<String>) {
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("// TODO") || trimmed.starts_with("// todo") {
            warnings.push(format!("Línea {}: TODO sin implementar: '{}'", i + 1, trimmed));
        }
        if trimmed.contains("panic(\"not implemented\")") {
            warnings.push(format!("Línea {}: panic not implemented detectado", i + 1));
        }
    }
}

fn check_java_stubs(content: &str, warnings: &mut Vec<String>) {
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("// TODO") || trimmed.starts_with("// todo") {
            warnings.push(format!("Línea {}: TODO sin implementar: '{}'", i + 1, trimmed));
        }
        if trimmed.contains("throw new UnsupportedOperationException") {
            warnings.push(format!("Línea {}: UnsupportedOperationException — método no implementado", i + 1));
        }
    }
}

fn check_generic_stubs(content: &str, warnings: &mut Vec<String>) {
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("# TODO") || trimmed.starts_with("// TODO") {
            warnings.push(format!("Línea {}: TODO sin implementar: '{}'", i + 1, trimmed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_python_pass_stub_detected() {
        let code = "class Tablero:\n    def dibujar(self, screen):\n        # Dibujar el tablero\n        pass\n";
        let report = detect_stubs(code, "tablero.py");
        assert!(report.has_stubs, "Debería detectar el stub 'pass'");
    }

    #[test]
    fn test_python_todo_detected() {
        let code = "def calcular_iva(precio):\n    # TODO: implementar\n    return 0\n";
        let report = detect_stubs(code, "factura.py");
        assert!(report.has_stubs, "Debería detectar el TODO");
    }

    #[test]
    fn test_clean_python_passes() {
        let code = "class Tablero:\n    def __init__(self):\n        self.casillas = [[None]*8 for _ in range(8)]\n\n    def dibujar(self, screen):\n        colors = [(255,206,158),(209,139,71)]\n        for row in range(8):\n            for col in range(8):\n                color = colors[(row+col)%2]\n                pygame.draw.rect(screen, color, (col*80, row*80, 80, 80))\n";
        let report = detect_stubs(code, "tablero.py");
        assert!(!report.has_stubs, "Código real no debería ser rechazado");
    }

    #[test]
    fn test_rust_todo_detected() {
        let code = "fn calcular(&self) -> i32 {\n    todo!()\n}\n";
        let report = detect_stubs(code, "main.rs");
        assert!(report.has_stubs, "Debería detectar todo!()");
    }
}
