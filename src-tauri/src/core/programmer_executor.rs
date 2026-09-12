use crate::core::tool_registry::ExecutionResult;
use crate::core::workspace_resolver::WorkspaceResolver;
use crate::memory::Cambio;

pub struct ProgrammerExecutor;

fn strip_think_tags(mut text: String) -> String {
    while let (Some(start), Some(end)) = (text.find("<think>"), text.find("</think>")) {
        if end + 8 <= text.len() {
            text.replace_range(start..end + 8, "");
        } else {
            break;
        }
    }
    
    let mut clean_text = text.trim().to_string();
    if let Some(start) = clean_text.find('{') {
        if let Some(end) = clean_text.rfind('}') {
            clean_text = clean_text[start..end + 1].to_string();
        }
    }
    
    clean_text
}

fn try_salvage_programmer_output(raw: &str, requested_files: &[String]) -> Option<crate::llm::ProgrammerOutput> {
    if let (Some(first_brace), Some(last_brace)) = (raw.find('{'), raw.rfind('}')) {
        if last_brace > first_brace {
            let candidate = &raw[first_brace..=last_brace];
            if let Ok(po) = serde_json::from_str::<crate::llm::ProgrammerOutput>(candidate) {
                return Some(po);
            }
        }
    }

    let lang_tags = [
        ("html", "html"), ("htm", "html"), ("python", "py"), ("py", "py"),
        ("javascript", "js"), ("js", "js"), ("css", "css"), ("rust", "rs"),
        ("rs", "rs"), ("json", "json"),
    ];

    for (tag, ext_match) in &lang_tags {
        let block_prefix = format!("```{}", tag);
        if let Some(start) = raw.find(&block_prefix) {
            let after_start = &raw[start + block_prefix.len()..];
            let code_content = if let Some(end) = after_start.find("```") {
                &after_start[..end]
            } else {
                after_start
            };
            let code_trimmed = code_content.trim();
            if !code_trimmed.is_empty() {
                let target_file = requested_files.iter()
                    .find(|f| f.ends_with(&format!(".{}", ext_match)))
                    .cloned()
                    .or_else(|| requested_files.first().cloned())
                    .unwrap_or_else(|| format!("main.{}", ext_match));

                return Some(crate::llm::ProgrammerOutput {
                    pensamiento: Some("Recuperado desde bloque de código Markdown".to_string()),
                    explicacion_tecnica: "Código extraído de Markdown formateado".to_string(),
                    cambios: vec![Cambio {
                        archivo: target_file,
                        buscar: String::new(),
                        reemplazar: code_trimmed.to_string(),
                    }],
                });
            }
        }
    }
    None
}

impl ProgrammerExecutor {
    pub async fn execute(workspace_path: &str, args: serde_json::Value) -> Result<ExecutionResult, String> {
        // Direct 'cambios' array passed explicitly
        if let Some(cambios_val) = args.get("cambios").and_then(|v| serde_json::from_value::<Vec<Cambio>>(v.clone()).ok()) {
            if !cambios_val.is_empty() {
                return Self::apply_and_validate(workspace_path, cambios_val).await;
            }
        }

        let task = args.get("instruccion")
            .or_else(|| args.get("prompt"))
            .or_else(|| args.get("task"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mut files: Vec<String> = if let Some(arr) = args.get("archivos_a_editar").and_then(|v| v.as_array()) {
            arr.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect()
        } else if let Some(arr) = args.get("archivos").and_then(|v| v.as_array()) {
            arr.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect()
        } else if let Some(s) = args.get("archivo").and_then(|v| v.as_str()) {
            vec![s.to_string()]
        } else {
            vec![]
        };

        let context = args.get("context")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let model = args.get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("qwen2.5-coder:7b")
            .to_string();

        if files.is_empty() && task.is_empty() {
            return Ok(ExecutionResult::error(
                "TOOL_PROGRAMMER requiere al menos 'archivos_a_editar' o 'instruccion'.",
                1
            ));
        }

        // Auto-heal check: if workspace has compile/syntax errors, add failing files
        if let Err(compile_err) = crate::core::validate_workspace(workspace_path).await {
            let failing_files = crate::core::extract_workspace_files_from_error(workspace_path, &compile_err);
            for ff in failing_files {
                if !files.contains(&ff) {
                    files.push(ff);
                }
            }
        }

        let safe_files = crate::memory::read_files_safely(workspace_path, files.clone()).await;
        let file_contents = format!("{}\n\n{}", context, safe_files);

        let prompt_res = crate::llm::delegate_to_programmer(&task, &file_contents, &model).await;
        let json_res = match prompt_res {
            Ok(res) => res,
            Err(e) => return Ok(ExecutionResult::error(format!("Error delegando a programador: {}", e), 1)),
        };

        let clean_json = strip_think_tags(json_res.clone());
        let parsed = serde_json::from_str::<crate::llm::ProgrammerOutput>(&clean_json)
            .ok()
            .or_else(|| try_salvage_programmer_output(&json_res, &files));

        let prog_output = match parsed {
            Some(po) => po,
            None => return Ok(ExecutionResult::error(format!("No se pudo interpretar el JSON del programador: {}", json_res), 1)),
        };

        if prog_output.cambios.is_empty() {
            return Ok(ExecutionResult::error("El programador no propuso ningún cambio en 'cambios'.", 1));
        }

        Self::apply_and_validate(workspace_path, prog_output.cambios).await
    }

    pub async fn apply_and_validate(workspace_path: &str, cambios: Vec<Cambio>) -> Result<ExecutionResult, String> {
        let _ = crate::core::create_git_backup(workspace_path, "Aura-Sentinel: ProgrammerExecutor Backup").await;

        // Anti-Stub Check
        let mut stub_rejections = Vec::new();
        for cambio in &cambios {
            let report = crate::core::stub_enforcer::detect_stubs(&cambio.reemplazar, &cambio.archivo);
            if report.has_stubs {
                stub_rejections.push(report.rejection_message);
            }
        }
        if !stub_rejections.is_empty() {
            let msg = format!("[ANTI-STUB REJECTED]: {}", stub_rejections.join("\n"));
            let mut err_res = ExecutionResult::error(msg, 1);
            err_res.cwd = Some(workspace_path.to_string());
            return Ok(err_res);
        }

        // Apply file writes via WorkspaceResolver
        let mut written_files = Vec::new();
        for cambio in cambios {
            let target_path = match WorkspaceResolver::resolve_create_path(workspace_path, &cambio.archivo) {
                Ok(p) => p,
                Err(e) => {
                    let _ = crate::core::restore_git_backup(workspace_path).await;
                    let mut err_res = ExecutionResult::error(format!("[SECURITY_VIOLATION] Ruta rechazada para '{}': {}", cambio.archivo, e), 1);
                    err_res.cwd = Some(workspace_path.to_string());
                    return Ok(err_res);
                }
            };

            let file_exists = target_path.exists();
            let original_content = if file_exists {
                tokio::fs::read_to_string(&target_path).await.unwrap_or_default()
            } else {
                if let Some(parent) = target_path.parent() {
                    let _ = tokio::fs::create_dir_all(parent).await;
                }
                String::new()
            };

            let (new_content, _ok, _log) = crate::memory::apply_patch_to_string(
                &original_content,
                file_exists,
                &cambio.buscar,
                &cambio.reemplazar,
                &cambio.archivo,
            );
            let sanitized = crate::memory::sanitize_generated_code(&cambio.archivo, new_content);

            if let Err(e) = tokio::fs::write(&target_path, sanitized).await {
                let _ = crate::core::restore_git_backup(workspace_path).await;
                let mut err_res = ExecutionResult::error(format!("Error escribiendo '{}': {}", cambio.archivo, e), 1);
                err_res.cwd = Some(workspace_path.to_string());
                return Ok(err_res);
            }
            written_files.push(cambio.archivo);
        }

        // Validate workspace compilation/syntax
        if let Err(comp_err) = crate::core::validate_workspace(workspace_path).await {
            let _ = crate::core::restore_git_backup(workspace_path).await;
            let mut err_res = ExecutionResult::error(format!("[COMPILATION_ERROR]: {}", comp_err), 1);
            err_res.files_affected = written_files;
            err_res.cwd = Some(workspace_path.to_string());
            return Ok(err_res);
        }

        let stdout = format!("{} archivos escritos y validados exitosamente: {:?}", written_files.len(), written_files);
        let mut res = ExecutionResult::success(stdout);
        res.files_affected = written_files;
        res.cwd = Some(workspace_path.to_string());
        Ok(res)
    }
}
