#![allow(dead_code)]
pub mod javascript;
pub mod python;
pub mod rust;
pub mod typescript;

use crate::core::auto_validator::{AutoValidator, Severity};

fn validate_firebase_config(file: &str, value: &serde_json::Value) -> Result<(), String> {
    if !file.eq_ignore_ascii_case("firebase.json") {
        return Ok(());
    }

    let Some(firestore) = value.get("firestore") else {
        return Ok(());
    };
    let databases: Vec<&serde_json::Value> = match firestore {
        serde_json::Value::Array(entries) => entries.iter().collect(),
        entry => vec![entry],
    };
    for database in databases {
        let Some(rules_value) = database.get("rules") else {
            return Err("[FIREBASE_CONFIG_INVALID] firebase.json debe referenciar el archivo de reglas de Firestore (por ejemplo, \"firestore.rules\"); el contenido de las reglas va en ese archivo separado.".to_string());
        };
        let Some(rules_path) = rules_value.as_str() else {
            return Err("[FIREBASE_CONFIG_INVALID] firebase.json debe guardar la ruta a firestore.rules como cadena, no las reglas inline.".to_string());
        };
        let extension = std::path::Path::new(rules_path)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("");
        if rules_path.contains('\n')
            || rules_path.contains('\r')
            || !extension.eq_ignore_ascii_case("rules")
            || rules_path.contains("service cloud.firestore")
        {
            return Err("[FIREBASE_CONFIG_INVALID] firebase.json contiene reglas de Firestore inline o una ruta inválida. Usa \"rules\": \"firestore.rules\" y coloca el código de reglas en firestore.rules.".to_string());
        }
        if let Some(indexes_value) = database.get("indexes") {
            let Some(indexes_path) = indexes_value.as_str() else {
                return Err("[FIREBASE_CONFIG_INVALID] firebase.json debe referenciar firestore.indexes.json mediante una ruta, no incluir los índices como un objeto inline.".to_string());
            };
            let indexes_extension = std::path::Path::new(indexes_path)
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("");
            if indexes_path.contains('\n')
                || indexes_path.contains('\r')
                || !indexes_extension.eq_ignore_ascii_case("json")
            {
                return Err("[FIREBASE_CONFIG_INVALID] firebase.json debe referenciar un archivo .json válido para los índices de Firestore.".to_string());
            }
        }
    }
    Ok(())
}

/// Central entrypoint for workspace validation across all supported languages and assets.
pub async fn validate_workspace(workspace_path: &str) -> Result<(), String> {
    // 1. Rust validation
    rust::validate_rust(workspace_path).await?;

    // 2. Python validation
    python::validate_python(workspace_path).await?;

    // 3. JavaScript & package.json validation
    javascript::validate_javascript(workspace_path).await?;

    // 4. TypeScript validation
    typescript::validate_typescript(workspace_path).await?;

    // 5. AutoValidator: verify local script and asset integrity (pure read-only)
    let auto_val = AutoValidator::new(workspace_path);
    let auto_res = auto_val.validate().await;
    let errors: Vec<_> = auto_res
        .issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .collect();
    if !errors.is_empty() {
        let mut err_msg = String::from("[ASSET_OR_SCRIPT_MISSING] Se detectaron referencias a archivos faltantes en el workspace:\n");
        for err in errors {
            err_msg.push_str(&format!("- [{}] {}\n", err.file, err.message));
        }
        err_msg.push_str("\nDebes crear los archivos o corregir las referencias relativas usando TOOL_PROGRAMMER.");
        return Err(err_msg);
    }

    Ok(())
}

/// Validates only the source languages touched by one programmer transaction.
/// Full workspace and asset validation remains available through
/// `validate_workspace` at phase/test checkpoints.
pub async fn validate_changed_files(workspace_path: &str, files: &[String]) -> Result<(), String> {
    if files.is_empty() {
        return Err("[VALIDATION_SCOPE_EMPTY] No se indicaron archivos modificados.".to_string());
    }

    let extension = |file: &str| {
        std::path::Path::new(file)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    let python_files: Vec<String> = files
        .iter()
        .filter(|file| extension(file) == "py")
        .cloned()
        .collect();
    if !python_files.is_empty() {
        python::validate_python_files(workspace_path, &python_files).await?;
    }

    if files.iter().any(|file| {
        matches!(extension(file).as_str(), "rs") || file.eq_ignore_ascii_case("Cargo.toml")
    }) {
        rust::validate_rust(workspace_path).await?;
    }

    if files.iter().any(|file| {
        matches!(
            extension(file).as_str(),
            "html" | "htm" | "js" | "mjs" | "cjs"
        ) || file.eq_ignore_ascii_case("package.json")
    }) {
        javascript::validate_javascript_files(workspace_path, files).await?;
    }

    if files
        .iter()
        .any(|file| matches!(extension(file).as_str(), "ts" | "tsx"))
    {
        typescript::validate_typescript(workspace_path).await?;
    }

    for file in files.iter().filter(|file| extension(file) == "json") {
        let path = std::path::Path::new(workspace_path).join(file);
        let contents = tokio::fs::read_to_string(&path)
            .await
            .map_err(|error| format!("[JSON_READ_ERROR] {}: {}", file, error))?;
        let value = serde_json::from_str::<serde_json::Value>(&contents)
            .map_err(|error| format!("[JSON_SYNTAX_ERROR] {}: {}", file, error))?;
        validate_firebase_config(file, &value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_firebase_config;

    #[test]
    fn firebase_config_requires_rule_file_paths_instead_of_inline_rules() {
        let valid = serde_json::json!({
            "firestore": {
                "rules": "firestore.rules",
                "indexes": "firestore.indexes.json"
            }
        });
        assert!(validate_firebase_config("firebase.json", &valid).is_ok());

        let inline_rules = serde_json::json!({
            "firestore": {
                "rules": "service cloud.firestore {\\n match /databases/{database}/documents { allow read, write: if request.auth != null; } }"
            }
        });
        let error = validate_firebase_config("firebase.json", &inline_rules).unwrap_err();
        assert!(error.contains("FIREBASE_CONFIG_INVALID"));
        assert!(validate_firebase_config("other.json", &inline_rules).is_ok());

        let inline_indexes = serde_json::json!({
            "firestore": {
                "rules": "firestore.rules",
                "indexes": {"indexes": []}
            }
        });
        let error = validate_firebase_config("firebase.json", &inline_indexes).unwrap_err();
        assert!(error.contains("FIREBASE_CONFIG_INVALID"));
    }
}
