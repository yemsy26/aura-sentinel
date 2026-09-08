#![allow(dead_code)]
use std::process::Stdio;
use tokio::process::Command;
use std::path::Path;

pub async fn validate_javascript(workspace_path: &str) -> Result<(), String> {
    let path = Path::new(workspace_path);

    // 1. package.json script validation
    let pkg_path = path.join("package.json");
    if pkg_path.exists() {
        let pkg_content = std::fs::read_to_string(&pkg_path).unwrap_or_default();
        let pkg_json_res = serde_json::from_str::<serde_json::Value>(&pkg_content);
        match pkg_json_res {
            Ok(pkg_json) => {
                let has_dev = pkg_json.pointer("/scripts/dev").is_some();
                let has_start = pkg_json.pointer("/scripts/start").is_some();
                let has_build = pkg_json.pointer("/scripts/build").is_some();

                if !has_dev && !has_start && !has_build {
                    return Err(
                        "[NODE_SCRIPTS_MISSING] El package.json existe pero NO tiene scripts 'dev', 'start' ni 'build' definidos. \
                        Esto significa que 'npm run dev' fallará con 'Missing script: dev'. \
                        Debes usar TOOL_PROGRAMMER para agregar los scripts correctos al package.json antes de continuar.".to_string()
                    );
                }
            },
            Err(e) => {
                return Err(format!(
                    "[JSON_SYNTAX_ERROR] El package.json tiene errores de sintaxis (Línea {}): {}. \
                    JSON estricto no permite comentarios ni comas sueltas.",
                    e.line(), e
                ));
            }
        }
    }

    // 2. Syntax check only for pure JS files (.js, .mjs, .cjs)
    let mut js_files = Vec::new();
    let mut html_files = Vec::new();

    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
                let ext_lower = ext.to_lowercase();
                if ext_lower == "js" || ext_lower == "mjs" || ext_lower == "cjs" {
                    js_files.push(entry.path().to_string_lossy().to_string());
                } else if ext_lower == "html" || ext_lower == "htm" {
                    html_files.push(entry.path().to_string_lossy().to_string());
                }
            }
        }
    }

    for js_file in js_files {
        let output = Command::new("node")
            .arg("--check")
            .arg(&js_file)
            .current_dir(workspace_path)
            .stdin(Stdio::null())
            .output()
            .await;

        if let Ok(out) = output {
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                return Err(format!("[NODE_SYNTAX_ERROR] Archivo {}: {}", js_file, stderr).trim().to_string());
            }
        }
    }

    // 3. Scan inline HTML scripts
    for html_file in html_files {
        if let Ok(content) = std::fs::read_to_string(&html_file) {
            let mut scripts = String::new();
            let mut in_script = false;
            for line in content.lines() {
                let l_lower = line.to_lowercase();
                if l_lower.contains("<script") && !l_lower.contains("src=") {
                    in_script = true;
                    if let Some(idx) = l_lower.find("script>") {
                        let extracted = &line[idx + 7..];
                        if !extracted.trim().is_empty() {
                            scripts.push_str(extracted);
                            scripts.push('\n');
                        }
                    }
                    if l_lower.contains("</script>") {
                        in_script = false;
                    }
                    continue;
                }
                if l_lower.contains("</script>") {
                    if in_script {
                        if let Some(idx) = l_lower.find("</script>") {
                            scripts.push_str(&line[..idx]);
                            scripts.push('\n');
                        }
                    }
                    in_script = false;
                    continue;
                }
                if in_script {
                    scripts.push_str(line);
                    scripts.push('\n');
                }
            }

            if !scripts.trim().is_empty() {
                let tmp_path = path.join(".tmp_validate.js");
                if std::fs::write(&tmp_path, &scripts).is_ok() {
                    let output = Command::new("node")
                        .arg("--check")
                        .arg(".tmp_validate.js")
                        .current_dir(workspace_path)
                        .stdin(Stdio::null())
                        .output()
                        .await;
                    let _ = std::fs::remove_file(&tmp_path);

                    if let Ok(out) = output {
                        if !out.status.success() {
                            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                            return Err(format!("[NODE_SYNTAX_ERROR] Script en {}: {}", html_file, stderr).trim().to_string());
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
