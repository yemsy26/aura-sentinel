#![allow(dead_code)]
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

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
            }
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

    let chart_library_linked = html_files.iter().any(|html_file| {
        std::fs::read_to_string(html_file).map(|text| text.to_lowercase().contains("chart.js")).unwrap_or(false)
    });
    for js_file in js_files {
        let content = std::fs::read_to_string(&js_file).map_err(|e| format!("[JS_READ_ERROR] {}: {}", js_file, e))?;
        if content.contains("new Chart(")
            && !chart_library_linked
            && !content.contains("class Chart")
            && !content.contains("function Chart")
        {
            return Err(format!("[JS_UNRESOLVED_GLOBAL] {} usa 'new Chart(...)' pero no define Chart ni enlaza Chart.js. Implementa el gráfico con Canvas/JavaScript puro o añade explícitamente la dependencia.", js_file));
        }
        check_source(workspace_path, &js_file, &content, if js_file.ends_with(".cjs") { "commonjs" } else { "module" }).await?;
    }
    let selector = scraper::Selector::parse("script").unwrap();
    for html_file in html_files {
        let content = std::fs::read_to_string(&html_file).map_err(|e| e.to_string())?;
        validate_canvas_bindings(&html_file, &content)?;
        let scripts: Vec<(String, String)> = {
        let document = scraper::Html::parse_document(&content);
        document.select(&selector).filter_map(|script| {
            if script.value().attr("src").is_some() { return None; }
            let kind = script.value().attr("type").unwrap_or("");
            if !["", "module", "text/javascript", "application/javascript"].contains(&kind) { return None; }
            // inner_html() serializes JavaScript arrow operators to HTML
            // entities, creating a false syntax error in Node. Script elements
            // are raw-text elements, so validate their text content directly.
            let source = script.text().collect::<String>();
            Some((source, if kind == "module" { "module" } else { "commonjs" }.to_string()))
        }).collect()
        };
        for (source, mode) in scripts {
            check_source(workspace_path, &html_file, &source, &mode).await?;
        }
    }
    Ok(())
}

fn validate_canvas_bindings(html_file: &str, content: &str) -> Result<(), String> {
    let document = scraper::Html::parse_document(content);
    let canvas_selector = scraper::Selector::parse("canvas[id]").unwrap();
    let canvas_ids: std::collections::HashSet<&str> = document
        .select(&canvas_selector)
        .filter_map(|element| element.value().attr("id"))
        .collect();
    let binding_pattern = regex::Regex::new(
        r#"(?m)\b(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*document\.getElementById\(\s*['\"]([^'\"]+)['\"]\s*\)"#,
    )
    .unwrap();
    for binding in binding_pattern.captures_iter(content) {
        let variable = &binding[1];
        let element_id = &binding[2];
        let context_pattern = regex::Regex::new(&format!(
            r"\b{}\s*\.\s*getContext\s*\(",
            regex::escape(variable)
        ))
        .unwrap();
        if context_pattern.is_match(content) && !canvas_ids.contains(element_id) {
            return Err(format!(
                "[CANVAS_BINDING_ERROR] {} usa getContext() sobre '#{}', pero ese elemento no es <canvas>. Cambia el elemento HTML a <canvas id=\"{}\">.",
                html_file, element_id, element_id
            ));
        }
    }
    Ok(())
}

async fn check_source(workspace: &str, name: &str, source: &str, mode: &str) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    // stdin avoids Windows verbatim-path handling and never overwrites a workspace temp file.
    let mut child = Command::new("node").arg("--check").arg(format!("--input-type={}", mode))
        .current_dir(workspace).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|e| format!("[NODE_UNAVAILABLE] {}", e))?;
    let mut input = child.stdin.take().ok_or("NODE_STDIN_UNAVAILABLE")?;
    input.write_all(source.as_bytes()).await.map_err(|e| e.to_string())?;
    drop(input);
    let output = child.wait_with_output().await.map_err(|e| e.to_string())?;
    if output.status.success() { Ok(()) } else {
        Err(format!("[NODE_SYNTAX_ERROR] {}: {}", name, String::from_utf8_lossy(&output.stderr)))
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn canonical_windows_paths_and_inline_scripts_are_validated_without_temp_overwrite() {
        let root = std::env::temp_dir().join(format!("aura js {}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".tmp_validate.js"), "const preserve = true;").unwrap();
        std::fs::write(root.join("main.js"), "const value = 1;").unwrap();
        std::fs::write(root.join("index.html"), "<html><script>const values = [1]; values.forEach(value => console.log(value));</script></html>").unwrap();
        let canonical = root.canonicalize().unwrap();
        assert!(super::validate_javascript(canonical.to_str().unwrap()).await.is_ok());
        assert_eq!(std::fs::read_to_string(root.join(".tmp_validate.js")).unwrap(), "const preserve = true;");
        std::fs::write(root.join("index.html"), "<script>const broken = ;</script>").unwrap();
        assert!(super::validate_javascript(canonical.to_str().unwrap()).await.is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn unresolved_chart_global_is_rejected() {
        let root = std::env::temp_dir().join(format!("aura-chart-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("dashboard.js"), "const chart = new Chart(ctx, {});").unwrap();
        let result = super::validate_javascript(root.to_str().unwrap()).await;
        let _ = std::fs::remove_dir_all(root);
        assert!(result.unwrap_err().contains("JS_UNRESOLVED_GLOBAL"));
    }

    #[tokio::test]
    async fn get_context_on_a_div_is_rejected_before_runtime() {
        let root = std::env::temp_dir().join(format!("aura-canvas-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("dashboard.html"),
            "<div id='trafficChart'></div><script>const chart = document.getElementById('trafficChart'); const ctx = chart.getContext('2d');</script>",
        )
        .unwrap();
        let result = super::validate_javascript(root.to_str().unwrap()).await;
        let _ = std::fs::remove_dir_all(root);
        assert!(result.unwrap_err().contains("CANVAS_BINDING_ERROR"));
    }
}
