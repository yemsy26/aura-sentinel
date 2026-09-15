#![allow(dead_code)]
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

fn validate_verifier_protocol(path: &Path) -> Result<(), String> {
    let entries = std::fs::read_dir(path).map_err(|error| error.to_string())?;
    for entry in entries.flatten() {
        let file_path = entry.path();
        let file_name = file_path.file_name().and_then(|name| name.to_str()).unwrap_or("");
        if !file_path.is_file() || !file_name.starts_with("verify_") || file_path.extension().and_then(|ext| ext.to_str()) != Some("py") {
            continue;
        }
        let source = std::fs::read_to_string(&file_path).map_err(|error| error.to_string())?;
        let required = ["passed", "total", "percentage", "failed_criteria", "__main__", "json.dumps"];
        let missing: Vec<_> = required.iter().filter(|token| !source.contains(**token)).copied().collect();
        let exits_explicitly = source.contains("sys.exit(") || source.contains("exit(");
        let main_position = source.find("if __name__");
        let output_position = source.find("print(json.dumps");
        let executes_from_main = matches!((main_position, output_position), (Some(main), Some(output)) if main < output);
        let tracks_failures = source.contains("failed_criteria =")
            && (source.contains("failed_criteria.append(") || source.contains("for ") || source.contains(" if "));
        if !missing.is_empty() || !exits_explicitly || !executes_from_main || !tracks_failures {
            return Err(format!(
                "[VERIFIER_PROTOCOL_INVALID] {} debe ejecutar sus checks dentro de __main__, imprimir exactamente una línea JSON mediante json.dumps con passed y total enteros, percentage calculado y failed_criteria calculado de los resultados, y terminar explícitamente con exit(0) al aprobar o exit(1) al fallar. Campos o mecanismos ausentes: {}{}{}{}",
                file_name,
                if missing.is_empty() { "ninguno".to_string() } else { missing.join(", ") },
                if exits_explicitly { "" } else { ", exit(0/1)" },
                if executes_from_main { "" } else { ", ejecución real dentro de __main__" },
                if tracks_failures { "" } else { ", failed_criteria calculado" }
            ));
        }
    }
    Ok(())
}

pub async fn validate_python(workspace_path: &str) -> Result<(), String> {
    let path = Path::new(workspace_path);

    let has_py = || -> bool {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                if let Some(ext) = entry.path().extension() {
                    if ext == "py" {
                        return true;
                    }
                }
            }
        }
        false
    };

    if !has_py() {
        return Ok(());
    }

    let output = Command::new("python")
        .arg("-m")
        .arg("compileall")
        .arg("-q")
        .arg("-x")
        .arg("node_modules|\\.git|__pycache__|venv|\\.venv")
        .arg(".")
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .current_dir(workspace_path)
        .stdin(Stdio::null())
        .output()
        .await;

    match output {
        Ok(out) if out.status.success() => validate_verifier_protocol(path),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            Err(format!("[PYTHON_SYNTAX_ERROR] {} {}", stdout, stderr)
                .trim()
                .to_string())
        }
        Err(error) => Err(format!("[PYTHON_UNAVAILABLE] No se pudo comprobar la sintaxis: {}", error)),
    }
}

#[cfg(test)]
mod tests {
    use super::validate_verifier_protocol;

    #[test]
    fn rejects_unstructured_verifier_before_execution() {
        let root = std::env::temp_dir().join(format!("aura-verifier-protocol-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("verify_dashboard.py"), "if __name__ == '__main__':\n    print('Check passed')\n").unwrap();
        let result = validate_verifier_protocol(&root);
        let _ = std::fs::remove_dir_all(&root);
        assert!(result.unwrap_err().contains("VERIFIER_PROTOCOL_INVALID"));
    }

    #[test]
    fn accepts_structured_verifier_contract() {
        let root = std::env::temp_dir().join(format!("aura-verifier-valid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("verify_dashboard.py"), "import json\nif __name__ == '__main__':\n    passed = 5\n    total = 5\n    percentage = 100 * passed / total\n    failed_criteria = []\n    print(json.dumps({'passed': passed, 'total': total, 'percentage': percentage, 'failed_criteria': failed_criteria}))\n    exit(0 if passed == total else 1)\n").unwrap();
        let result = validate_verifier_protocol(&root);
        let _ = std::fs::remove_dir_all(&root);
        assert!(result.is_ok());
    }
}
