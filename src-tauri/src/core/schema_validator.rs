use serde_json::Value;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaValidationResult {
    Valid,
    Invalid(String),
}

#[allow(dead_code)]
pub struct SchemaValidator;

impl SchemaValidator {
    #[allow(dead_code)]
    pub fn validate_tool_payload(tool: &str, payload: &Value) -> SchemaValidationResult {
        match tool {
            "TOOL_PROGRAMMER" => {
                // Should have archivos_a_editar array
                if let Some(arr) = payload.get("archivos_a_editar").and_then(|v| v.as_array()) {
                    if arr.is_empty() {
                        return SchemaValidationResult::Invalid(
                            "TOOL_PROGRAMMER requiere al menos un archivo en 'archivos_a_editar'"
                                .to_string(),
                        );
                    }
                } else {
                    return SchemaValidationResult::Invalid(
                        "TOOL_PROGRAMMER debe incluir 'archivos_a_editar' como un array de strings"
                            .to_string(),
                    );
                }
                SchemaValidationResult::Valid
            }
            "TOOL_TERMINAL" => {
                let cmd = payload
                    .get("comando")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if cmd.is_empty() {
                    return SchemaValidationResult::Invalid(
                        "TOOL_TERMINAL requiere un 'comando' no vacío".to_string(),
                    );
                }
                if let Err(err) = validate_interpreter_target(cmd) {
                    return SchemaValidationResult::Invalid(err);
                }
                SchemaValidationResult::Valid
            }
            "TOOL_ENV_MANAGER" => {
                let pkg = payload
                    .get("comando")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if pkg.is_empty() {
                    return SchemaValidationResult::Invalid(
                        "TOOL_ENV_MANAGER requiere el nombre del paquete en 'comando'".to_string(),
                    );
                }
                SchemaValidationResult::Valid
            }
            "TOOL_LOGIC_SOLVER" => validate_logic_solver_payload(payload),
            "TOOL_FINISH" => SchemaValidationResult::Valid,
            "TOOL_THINK" => SchemaValidationResult::Valid,
            _ => SchemaValidationResult::Valid,
        }
    }

    /// P1-1: Valida que el archivo objetivo sea compatible con el intérprete invocado.
    /// Rechaza disparates como 'python cyber_sentinel.html' o 'node index.html'.
    pub fn validate_interpreter_target(cmd: &str) -> Option<String> {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }

        let interp = parts[0].to_lowercase();
        // Buscar el primer argumento que sea un archivo (no una flag que empiece con '-')
        let target_file = parts.iter().skip(1).find(|arg| !arg.starts_with('-'))?;
        let target_lower = target_file.to_lowercase();
        let target_clean = target_lower.trim_matches(|c: char| c == '"' || c == '\'' || c == '`');

        // Intérprete Python
        if interp == "python" || interp == "python3" || interp == "py" {
            // Ignorar llamadas a módulos con -m (ej: python -m unittest ...)
            if parts.iter().any(|&p| p == "-m") {
                return None;
            }
            if target_clean.ends_with(".html") || target_clean.ends_with(".htm") {
                return Some(format!(
                    "DENY_INVALID_INTERPRETER_TARGET: 'python' no puede ejecutar un archivo HTML ('{}'). Los archivos HTML se prueban mediante scripts verify_*.py o abriéndolos con 'start {}'.",
                    target_file, target_file
                ));
            }
            if target_clean.ends_with(".css")
                || target_clean.ends_with(".js")
                || target_clean.ends_with(".json")
                || target_clean.ends_with(".rs")
            {
                return Some(format!(
                    "DENY_INVALID_INTERPRETER_TARGET: 'python' no puede ejecutar un archivo '{}'. Usa python únicamente con scripts .py.",
                    target_file
                ));
            }
        }

        // Intérprete Node
        if interp == "node" || interp == "nodejs" {
            if target_clean.ends_with(".html")
                || target_clean.ends_with(".htm")
                || target_clean.ends_with(".css")
                || target_clean.ends_with(".py")
            {
                return Some(format!(
                    "DENY_INVALID_INTERPRETER_TARGET: 'node' no puede ejecutar un archivo '{}'. Usa node únicamente con archivos de JavaScript/TypeScript (.js, .mjs, .ts).",
                    target_file
                ));
            }
        }

        None
    }
}

fn validate_logic_solver_payload(payload: &Value) -> SchemaValidationResult {
    let has_n_vars = payload.get("n_vars").is_some();
    let has_clauses = payload.get("clauses").is_some();
    if !has_n_vars && !has_clauses {
        // No SAT instance means the agent may be requesting a source-code logic
        // review. That mode is handled separately and must never be reported as
        // a SAT calculation by the registered native executor.
        return SchemaValidationResult::Valid;
    }
    if !has_n_vars || !has_clauses {
        return SchemaValidationResult::Invalid(
            "TOOL_LOGIC_SOLVER requiere 'n_vars' y 'clauses' juntos para resolver SAT".into(),
        );
    }

    let Some(n_vars) = payload.get("n_vars").and_then(Value::as_u64) else {
        return SchemaValidationResult::Invalid(
            "TOOL_LOGIC_SOLVER requiere 'n_vars' como entero no negativo".into(),
        );
    };
    let Some(clauses) = payload.get("clauses").and_then(Value::as_array) else {
        return SchemaValidationResult::Invalid(
            "TOOL_LOGIC_SOLVER requiere 'clauses' como una matriz de cláusulas".into(),
        );
    };

    for (clause_index, clause) in clauses.iter().enumerate() {
        let Some(literals) = clause.as_array() else {
            return SchemaValidationResult::Invalid(format!(
                "La cláusula {} debe ser una matriz de literales enteros",
                clause_index + 1
            ));
        };
        for (literal_index, literal) in literals.iter().enumerate() {
            let Some(value) = literal.as_i64() else {
                return SchemaValidationResult::Invalid(format!(
                    "El literal {} de la cláusula {} debe ser un entero",
                    literal_index + 1,
                    clause_index + 1
                ));
            };
            let Ok(value) = i32::try_from(value) else {
                return SchemaValidationResult::Invalid(format!(
                    "El literal {} de la cláusula {} excede el rango admitido",
                    literal_index + 1,
                    clause_index + 1
                ));
            };
            if value == 0 {
                return SchemaValidationResult::Invalid(
                    "El literal 0 no es válido en una fórmula SAT".into(),
                );
            }
            if value.unsigned_abs() as u64 > n_vars {
                return SchemaValidationResult::Invalid(format!(
                    "El literal {value} referencia una variable fuera de 1..={n_vars}"
                ));
            }
        }
    }
    SchemaValidationResult::Valid
}

pub fn validate_interpreter_target(cmd: &str) -> Result<(), String> {
    if let Some(err) = SchemaValidator::validate_interpreter_target(cmd) {
        Err(err)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_validate_terminal_empty_command() {
        let payload = json!({ "comando": "" });
        assert!(matches!(
            SchemaValidator::validate_tool_payload("TOOL_TERMINAL", &payload),
            SchemaValidationResult::Invalid(_)
        ));
    }

    #[test]
    fn test_validate_terminal_valid_command() {
        let payload = json!({ "comando": "cargo test" });
        assert_eq!(
            SchemaValidator::validate_tool_payload("TOOL_TERMINAL", &payload),
            SchemaValidationResult::Valid
        );
    }

    #[test]
    fn test_python_html_rejected() {
        let payload = json!({ "comando": "python cyber_sentinel.html" });
        assert!(matches!(
            SchemaValidator::validate_tool_payload("TOOL_TERMINAL", &payload),
            SchemaValidationResult::Invalid(msg) if msg.contains("DENY_INVALID_INTERPRETER_TARGET")
        ));
    }

    #[test]
    fn test_python_py_accepted() {
        let payload = json!({ "comando": "python verify_dashboard.py" });
        assert_eq!(
            SchemaValidator::validate_tool_payload("TOOL_TERMINAL", &payload),
            SchemaValidationResult::Valid
        );
    }

    #[test]
    fn test_node_html_rejected() {
        let payload = json!({ "comando": "node index.html" });
        assert!(matches!(
            SchemaValidator::validate_tool_payload("TOOL_TERMINAL", &payload),
            SchemaValidationResult::Invalid(msg) if msg.contains("DENY_INVALID_INTERPRETER_TARGET")
        ));
    }

    #[test]
    fn test_node_js_accepted() {
        let payload = json!({ "comando": "node script.js" });
        assert_eq!(
            SchemaValidator::validate_tool_payload("TOOL_TERMINAL", &payload),
            SchemaValidationResult::Valid
        );
    }

    #[test]
    fn test_logic_solver_rejects_partial_or_malformed_sat_payloads() {
        for payload in [
            json!({ "n_vars": 2 }),
            json!({ "clauses": [[1]] }),
            json!({ "n_vars": 1, "clauses": [[0]] }),
            json!({ "n_vars": 1, "clauses": [[2]] }),
            json!({ "n_vars": 1, "clauses": [[1.5]] }),
        ] {
            assert!(matches!(
                SchemaValidator::validate_tool_payload("TOOL_LOGIC_SOLVER", &payload),
                SchemaValidationResult::Invalid(_)
            ));
        }
    }

    #[test]
    fn test_logic_solver_accepts_valid_sat_and_code_review_payloads() {
        assert_eq!(
            SchemaValidator::validate_tool_payload(
                "TOOL_LOGIC_SOLVER",
                &json!({ "n_vars": 2, "clauses": [[1, -2], []] })
            ),
            SchemaValidationResult::Valid
        );
        assert_eq!(
            SchemaValidator::validate_tool_payload(
                "TOOL_LOGIC_SOLVER",
                &json!({ "archivos_a_editar": ["src/lib.rs"] })
            ),
            SchemaValidationResult::Valid
        );
    }
}
