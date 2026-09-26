#![allow(dead_code)]
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

fn validate_package_scripts(source: &str) -> Result<(), String> {
    let package: serde_json::Value = serde_json::from_str(source)
        .map_err(|error| format!("[JSON_SYNTAX_ERROR] package.json: {}", error))?;
    let script = |name: &str| {
        package
            .pointer(&format!("/scripts/{}", name))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|command| !command.is_empty())
    };
    let has_run_script = ["dev", "start", "build", "test"]
        .iter()
        .any(|name| script(name).is_some());
    if !has_run_script {
        return Err("[NODE_SCRIPTS_MISSING] package.json debe declarar al menos un script no vacío: dev, start, build o test.".to_string());
    }
    if script("test").is_some_and(|command| {
        let command = command.to_ascii_lowercase();
        command == "node:test" || command.starts_with("node:test ")
    }) {
        return Err("[NODE_TEST_SCRIPT_INVALID] 'node:test' es el módulo de pruebas, no el comando del runner. Para Node.js usa 'node --test tests/app.test.js'.".to_string());
    }
    Ok(())
}

pub async fn validate_javascript(workspace_path: &str) -> Result<(), String> {
    let path = Path::new(workspace_path);

    // 1. package.json script validation
    let pkg_path = path.join("package.json");
    if pkg_path.exists() {
        let pkg_content = std::fs::read_to_string(&pkg_path).unwrap_or_default();
        validate_package_scripts(&pkg_content)?;
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
        std::fs::read_to_string(html_file)
            .map(|text| text.to_lowercase().contains("chart.js"))
            .unwrap_or(false)
    });
    for js_file in js_files {
        let content = std::fs::read_to_string(&js_file)
            .map_err(|e| format!("[JS_READ_ERROR] {}: {}", js_file, e))?;
        if content.contains("new Chart(")
            && !chart_library_linked
            && !content.contains("class Chart")
            && !content.contains("function Chart")
        {
            return Err(format!("[JS_UNRESOLVED_GLOBAL] {} usa 'new Chart(...)' pero no define Chart ni enlaza Chart.js. Implementa el gráfico con Canvas/JavaScript puro o añade explícitamente la dependencia.", js_file));
        }
        validate_undeclared_dom_listeners(&js_file, &content)?;
        validate_dom_element_ids(workspace_path, &js_file, &content)?;
        check_source(
            workspace_path,
            &js_file,
            &content,
            if js_file.ends_with(".cjs") {
                "commonjs"
            } else {
                "module"
            },
        )
        .await?;
    }
    let selector = scraper::Selector::parse("script").unwrap();
    for html_file in html_files {
        let content = std::fs::read_to_string(&html_file).map_err(|e| e.to_string())?;
        validate_canvas_bindings(&html_file, &content)?;
        let scripts: Vec<(String, String)> = {
            let document = scraper::Html::parse_document(&content);
            document
                .select(&selector)
                .filter_map(|script| {
                    if script.value().attr("src").is_some() {
                        return None;
                    }
                    let kind = script.value().attr("type").unwrap_or("");
                    if !["", "module", "text/javascript", "application/javascript"].contains(&kind)
                    {
                        return None;
                    }
                    // inner_html() serializes JavaScript arrow operators to HTML
                    // entities, creating a false syntax error in Node. Script elements
                    // are raw-text elements, so validate their text content directly.
                    let source = script.text().collect::<String>();
                    Some((
                        source,
                        if kind == "module" {
                            "module"
                        } else {
                            "commonjs"
                        }
                        .to_string(),
                    ))
                })
                .collect()
        };
        for (source, mode) in scripts {
            check_source(workspace_path, &html_file, &source, &mode).await?;
        }
    }
    Ok(())
}

/// Syntax-checks only files in the current write transaction. Cross-file asset
/// checks run after the planned deliverables exist, so sequential phase writes
/// can create HTML before its stylesheet and module.
pub async fn validate_javascript_files(
    workspace_path: &str,
    files: &[String],
) -> Result<(), String> {
    let root = Path::new(workspace_path);
    let chart_library_linked = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    matches!(extension.to_ascii_lowercase().as_str(), "html" | "htm")
                })
                .then(|| std::fs::read_to_string(path).ok())
                .flatten()
        })
        .any(|source| source.to_lowercase().contains("chart.js"));

    for file in files {
        let path = root.join(file);
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match extension.as_str() {
            "js" | "mjs" | "cjs" => {
                let source = tokio::fs::read_to_string(&path)
                    .await
                    .map_err(|error| format!("[JS_READ_ERROR] {}: {}", file, error))?;
                if source.contains("new Chart(")
                    && !chart_library_linked
                    && !source.contains("class Chart")
                    && !source.contains("function Chart")
                {
                    return Err(format!("[JS_UNRESOLVED_GLOBAL] {} usa 'new Chart(...)' pero no define Chart ni enlaza Chart.js.", file));
                }
                validate_undeclared_dom_listeners(file, &source)?;
                validate_dom_element_ids(workspace_path, file, &source)?;
                check_source(
                    workspace_path,
                    file,
                    &source,
                    if extension == "cjs" {
                        "commonjs"
                    } else {
                        "module"
                    },
                )
                .await?;
            }
            "html" | "htm" => {
                let source = tokio::fs::read_to_string(&path)
                    .await
                    .map_err(|error| format!("[HTML_READ_ERROR] {}: {}", file, error))?;
                validate_canvas_bindings(file, &source)?;
                let js_entries: Vec<(String, String)> = std::fs::read_dir(root)
                    .ok()
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter_map(|entry| {
                        let p = entry.path();
                        if p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("js")) {
                            std::fs::read_to_string(&p).ok().map(|src| (p.file_name().unwrap().to_string_lossy().to_string(), src))
                        } else {
                            None
                        }
                    })
                    .collect();
                for (js_name, js_src) in js_entries {
                    validate_dom_element_ids(workspace_path, &js_name, &js_src)?;
                }
                let selector = scraper::Selector::parse("script").unwrap();
                let inline_scripts: Vec<(String, bool)> = {
                    let document = scraper::Html::parse_document(&source);
                    document
                        .select(&selector)
                        .filter_map(|script| {
                            if script.value().attr("src").is_some() {
                                return None;
                            }
                            let kind = script.value().attr("type").unwrap_or("");
                            if !["", "module", "text/javascript", "application/javascript"]
                                .contains(&kind)
                            {
                                return None;
                            }
                            Some((script.text().collect::<String>(), kind == "module"))
                        })
                        .collect()
                };
                for (inline_source, is_module) in inline_scripts {
                    check_source(
                        workspace_path,
                        file,
                        &inline_source,
                        if is_module { "module" } else { "commonjs" },
                    )
                    .await?;
                }
            }
            _ => {}
        }
    }

    if files
        .iter()
        .any(|file| file.eq_ignore_ascii_case("package.json"))
    {
        let package_path = root.join("package.json");
        let package_source = tokio::fs::read_to_string(&package_path)
            .await
            .map_err(|error| format!("[PACKAGE_JSON_READ_ERROR] {}", error))?;
        validate_package_scripts(&package_source)?;
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
    let mut child = Command::new("node")
        .arg("--check")
        .arg(format!("--input-type={}", mode))
        .current_dir(workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("[NODE_UNAVAILABLE] {}", e))?;
    let mut input = child.stdin.take().ok_or("NODE_STDIN_UNAVAILABLE")?;
    input
        .write_all(source.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    drop(input);
    let output = child.wait_with_output().await.map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "[NODE_SYNTAX_ERROR] {}: {}",
            name,
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

fn validate_undeclared_dom_listeners(file_name: &str, content: &str) -> Result<(), String> {
    let listener_re = match regex::Regex::new(r"\b([a-zA-Z_$][a-zA-Z0-9_$]*)\s*\.\s*addEventListener\s*\(") {
        Ok(re) => re,
        Err(_) => return Ok(()),
    };
    for cap in listener_re.captures_iter(content) {
        let var_name = &cap[1];
        if matches!(
            var_name,
            "window"
                | "document"
                | "globalThis"
                | "self"
                | "this"
                | "e"
                | "event"
                | "el"
                | "element"
                | "btn"
                | "button"
                | "form"
                | "input"
                | "item"
                | "node"
                | "target"
        ) {
            continue;
        }
        let is_declared = content.contains(&format!("const {}", var_name))
            || content.contains(&format!("let {}", var_name))
            || content.contains(&format!("var {}", var_name))
            || content.contains(&format!("function {}", var_name))
            || content.contains(&format!("{} =", var_name))
            || content.contains(&format!("({}", var_name))
            || content.contains(&format!(", {}", var_name));
        if !is_declared {
            return Err(format!(
                "[JS_UNDECLARED_ELEMENT] {}: '{}' se usa con addEventListener pero no está declarado en este archivo. Añade antes: const {} = document.getElementById('{}') o declara la variable.",
                file_name, var_name, var_name, var_name
            ));
        }
    }
    Ok(())
}

fn validate_dom_element_ids(workspace_path: &str, file_name: &str, content: &str) -> Result<(), String> {
    let root = Path::new(workspace_path);
    let mut html_contents = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let p = entry.path();
            if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                if ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm") {
                    if let Ok(c) = std::fs::read_to_string(&p) {
                        html_contents.push(c);
                    }
                }
            }
        }
    }

    if html_contents.is_empty() {
        return Ok(());
    }

    let id_attr_re = match regex::Regex::new(r#"(?i)\bid\s*=\s*["']([^"']+)["']"#) {
        Ok(re) => re,
        Err(_) => return Ok(()),
    };
    let mut html_ids = std::collections::HashSet::new();
    for html in &html_contents {
        for cap in id_attr_re.captures_iter(html) {
            html_ids.insert(cap[1].to_string());
        }
    }

    if html_ids.is_empty() {
        return Ok(());
    }

    // Direct: document.getElementById('xyz').addEventListener(...)
    let direct_re = match regex::Regex::new(r#"document\.getElementById\(\s*['"]([^'"]+)['"]\s*\)\s*\.\s*addEventListener\s*\("#) {
        Ok(re) => re,
        Err(_) => return Ok(()),
    };
    for cap in direct_re.captures_iter(content) {
        let el_id = &cap[1];
        if !html_ids.contains(el_id) {
            return Err(format!(
                "[JS_DOM_ID_NOT_FOUND] {}: 'document.getElementById(\"{}\")' intenta registrar un addEventListener, pero ningún archivo HTML del workspace define id=\"{}\". Define el elemento <... id=\"{}\"> en el HTML o corrige el id en JavaScript.",
                file_name, el_id, el_id, el_id
            ));
        }
    }

    // Bound: const form = document.getElementById('task-form'); ... form.addEventListener(...)
    let bound_re = match regex::Regex::new(r#"(?:const|let|var)\s+([a-zA-Z_$][a-zA-Z0-9_$]*)\s*=\s*document\.getElementById\(\s*['"]([^'"]+)['"]\s*\)"#) {
        Ok(re) => re,
        Err(_) => return Ok(()),
    };
    for cap in bound_re.captures_iter(content) {
        let var_name = &cap[1];
        let el_id = &cap[2];
        if !html_ids.contains(el_id) {
            let listener_usage = format!("{}.addEventListener", var_name);
            let listener_usage_ws = format!("{} .addEventListener", var_name);
            let has_listener = content.contains(&listener_usage) || content.contains(&listener_usage_ws);
            if has_listener {
                let optional_chain = format!("{}?.addEventListener", var_name);
                let guard1 = format!("if ({})", var_name);
                let guard2 = format!("if ({} ", var_name);
                let guard3 = format!("if(!{})", var_name);
                let guard4 = format!("if (!{})", var_name);
                if !content.contains(&optional_chain)
                    && !content.contains(&guard1)
                    && !content.contains(&guard2)
                    && !content.contains(&guard3)
                    && !content.contains(&guard4)
                {
                    return Err(format!(
                        "[JS_DOM_ID_NOT_FOUND] {}: '{}' se obtiene con document.getElementById(\"{}\") y se usa en addEventListener sin comprobar si existe, pero ningún archivo HTML del workspace define id=\"{}\". Añade el elemento <... id=\"{}\"> en el HTML o añade comprobación de existencia 'if ({})'.",
                        file_name, var_name, el_id, el_id, el_id, var_name
                    ));
                }
            }
        }
    }

    Ok(())
}


#[cfg(test)]
mod tests {
    #[test]
    fn accepts_test_only_packages_for_static_apps_when_command_is_real() {
        assert!(super::validate_package_scripts(
            r#"{"scripts":{"test":"node --test tests/app.test.js"}}"#
        )
        .is_ok());
    }

    #[test]
    fn rejects_empty_scripts_and_node_test_module_as_a_command() {
        let missing = super::validate_package_scripts(r#"{"scripts":{"test":"  "}}"#).unwrap_err();
        assert!(missing.contains("NODE_SCRIPTS_MISSING"));

        let invalid = super::validate_package_scripts(
            r#"{"scripts":{"test":"node:test tests/app.test.js"}}"#,
        )
        .unwrap_err();
        assert!(invalid.contains("NODE_TEST_SCRIPT_INVALID"));
        assert!(invalid.contains("node --test"));
    }

    #[tokio::test]
    async fn canonical_windows_paths_and_inline_scripts_are_validated_without_temp_overwrite() {
        let root = std::env::temp_dir().join(format!("aura js {}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".tmp_validate.js"), "const preserve = true;").unwrap();
        std::fs::write(root.join("main.js"), "const value = 1;").unwrap();
        std::fs::write(root.join("index.html"), "<html><script>const values = [1]; values.forEach(value => console.log(value));</script></html>").unwrap();
        let canonical = root.canonicalize().unwrap();
        assert!(super::validate_javascript(canonical.to_str().unwrap())
            .await
            .is_ok());
        assert_eq!(
            std::fs::read_to_string(root.join(".tmp_validate.js")).unwrap(),
            "const preserve = true;"
        );
        std::fs::write(root.join("index.html"), "<script>const broken = ;</script>").unwrap();
        assert!(super::validate_javascript(canonical.to_str().unwrap())
            .await
            .is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn unresolved_chart_global_is_rejected() {
        let root = std::env::temp_dir().join(format!("aura-chart-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("dashboard.js"),
            "const chart = new Chart(ctx, {});",
        )
        .unwrap();
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

    #[tokio::test]
    async fn missing_dom_element_id_with_listener_is_rejected() {
        let root = std::env::temp_dir().join(format!("aura-dom-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("index.html"),
            "<html><body><form id='login-form'></form></body></html>",
        )
        .unwrap();
        std::fs::write(
            root.join("script.js"),
            "const taskForm = document.getElementById('task-form'); taskForm.addEventListener('submit', () => {});",
        )
        .unwrap();
        let result = super::validate_javascript(root.to_str().unwrap()).await;
        let _ = std::fs::remove_dir_all(root);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("JS_DOM_ID_NOT_FOUND"));
    }

    #[tokio::test]
    async fn guarded_dom_element_id_with_listener_is_accepted() {
        let root = std::env::temp_dir().join(format!("aura-dom-guarded-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("index.html"),
            "<html><body><form id='login-form'></form></body></html>",
        )
        .unwrap();
        std::fs::write(
            root.join("script.js"),
            "const taskForm = document.getElementById('task-form'); if (taskForm) { taskForm.addEventListener('submit', () => {}); }",
        )
        .unwrap();
        let result = super::validate_javascript(root.to_str().unwrap()).await;
        let _ = std::fs::remove_dir_all(root);
        assert!(result.is_ok());
    }
}

