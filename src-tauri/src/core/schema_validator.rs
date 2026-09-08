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
                            "TOOL_PROGRAMMER requiere al menos un archivo en 'archivos_a_editar'".to_string(),
                        );
                    }
                } else {
                    return SchemaValidationResult::Invalid(
                        "TOOL_PROGRAMMER debe incluir 'archivos_a_editar' como un array de strings".to_string(),
                    );
                }
                SchemaValidationResult::Valid
            },
            "TOOL_TERMINAL" => {
                let cmd = payload.get("comando").and_then(|v| v.as_str()).unwrap_or("").trim();
                if cmd.is_empty() {
                    return SchemaValidationResult::Invalid(
                        "TOOL_TERMINAL requiere un 'comando' no vacío".to_string(),
                    );
                }
                SchemaValidationResult::Valid
            },
            "TOOL_ENV_MANAGER" => {
                let pkg = payload.get("comando").and_then(|v| v.as_str()).unwrap_or("").trim();
                if pkg.is_empty() {
                    return SchemaValidationResult::Invalid(
                        "TOOL_ENV_MANAGER requiere el nombre del paquete en 'comando'".to_string(),
                    );
                }
                SchemaValidationResult::Valid
            },
            "TOOL_FINISH" => SchemaValidationResult::Valid,
            "TOOL_THINK" => SchemaValidationResult::Valid,
            _ => SchemaValidationResult::Valid,
        }
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
}
