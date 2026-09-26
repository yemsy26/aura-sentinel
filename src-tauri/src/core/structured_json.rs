//! Conservative extraction for structured LLM responses.
//!
//! The model is asked for schema-constrained JSON, but providers can still add
//! fences or short text around the payload. This parser extracts one balanced
//! JSON object without rewriting its contents. Malformed or truncated output
//! remains an explicit error so callers can fail safely.

use serde_json::Value;

pub fn parse_json_object(raw: &str) -> Result<Value, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("respuesta vacía".to_string());
    }

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return value
            .is_object()
            .then_some(value)
            .ok_or_else(|| "la respuesta JSON no es un objeto".to_string());
    }

    let bytes = trimmed.as_bytes();
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, byte) in bytes.iter().copied().enumerate() {
        if start.is_none() {
            if byte == b'{' {
                start = Some(index);
                depth = 1;
            }
            continue;
        }

        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }

        match byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let Some(begin) = start else {
                        return Err("inicio de objeto JSON ausente".to_string());
                    };
                    let candidate = &trimmed[begin..=index];
                    if let Ok(value) = serde_json::from_str::<Value>(candidate) {
                        if value.is_object() {
                            return Ok(value);
                        }
                    }
                    start = None;
                }
            }
            _ => {}
        }
    }

    Err("no se encontró un objeto JSON completo y válido".to_string())
}

#[cfg(test)]
mod tests {
    use super::parse_json_object;

    #[test]
    fn parses_plain_object() {
        let value = parse_json_object(r#"{"herramienta":"TOOL_FINISH"}"#).unwrap();
        assert_eq!(value["herramienta"], "TOOL_FINISH");
    }

    #[test]
    fn extracts_fenced_nested_object_without_breaking_braces_in_strings() {
        let raw =
            "```json\n{\"text\":\"literal { brace } and \\\"quote\\\"\",\"args\":{\"x\":1}}\n```";
        let value = parse_json_object(raw).unwrap();
        assert_eq!(value["args"]["x"], 1);
        assert_eq!(value["text"], "literal { brace } and \"quote\"");
    }

    #[test]
    fn extracts_object_after_prose_and_preserves_newlines_and_quotes() {
        let value = parse_json_object("Resultado:\n{\"text\":\"a\\nb\\\"c\"}").unwrap();
        assert_eq!(value["text"], "a\nb\"c");
    }

    #[test]
    fn rejects_truncated_and_non_object_json() {
        assert!(parse_json_object("{\"x\": [1, 2]").is_err());
        assert!(parse_json_object("[1, 2]").is_err());
    }
}
