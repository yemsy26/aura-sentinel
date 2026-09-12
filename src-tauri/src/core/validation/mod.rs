#![allow(dead_code)]
pub mod rust;
pub mod python;
pub mod javascript;
pub mod typescript;

use crate::core::auto_validator::{AutoValidator, Severity};

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
    let errors: Vec<_> = auto_res.issues.iter().filter(|i| i.severity == Severity::Error).collect();
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
