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

fn heal_inline_script_entities(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let mut cursor = 0;
    loop {
        let lower_tail = source[cursor..].to_lowercase();
        let Some(script_rel) = lower_tail.find("<script") else {
            result.push_str(&source[cursor..]);
            break;
        };
        let script_start = cursor + script_rel;
        let Some(open_end_rel) = source[script_start..].find('>') else {
            result.push_str(&source[cursor..]);
            break;
        };
        let content_start = script_start + open_end_rel + 1;
        let lower_content = source[content_start..].to_lowercase();
        let Some(close_rel) = lower_content.find("</script>") else {
            result.push_str(&source[cursor..]);
            break;
        };
        let content_end = content_start + close_rel;
        result.push_str(&source[cursor..content_start]);
        let healed = source[content_start..content_end]
            .replace("=&gt;", "=>")
            .replace("&amp;&amp;", "&&")
            .replace(" &lt; ", " < ")
            .replace(" &gt; ", " > ");
        result.push_str(&healed);
        cursor = content_end;
    }
    result
}

fn semantic_repair_regressions(before: &str, after: &str) -> Vec<&'static str> {
    let before = before.to_lowercase();
    let after = after.to_lowercase();
    let feature_groups: &[(&str, &[&str])] = &[
        ("estructura HTML5", &["<!doctype html", "<html"]),
        ("Canvas", &["<canvas"]),
        ("animación", &["requestanimationframe"]),
        ("coordenada coseno", &["math.cos"]),
        ("coordenada seno", &["math.sin"]),
        ("nodos de amenaza", &["threat", "amenaza", "node"]),
        ("telemetría", &["packet", "paquete", "telemetr"]),
        ("tráfico", &["traffic", "trafico", "tráfico"]),
        ("alertas", &["alert", "alerta"]),
        ("ataques", &["attack", "ataque"]),
        ("glassmorphism", &["backdrop-filter", "glass"]),
        ("tipografía monospace", &["monospace", "courier"]),
    ];
    feature_groups
        .iter()
        .filter_map(|(label, tokens)| {
            let existed = tokens.iter().any(|token| before.contains(token));
            let remains = tokens.iter().any(|token| after.contains(token));
            (existed && !remains).then_some(*label)
        })
        .collect()
}

fn try_salvage_programmer_output(
    raw: &str,
    requested_files: &[String],
) -> Option<crate::llm::ProgrammerOutput> {
    if let (Some(first_brace), Some(last_brace)) = (raw.find('{'), raw.rfind('}')) {
        if last_brace > first_brace {
            let candidate = &raw[first_brace..=last_brace];
            if let Ok(po) = serde_json::from_str::<crate::llm::ProgrammerOutput>(candidate) {
                return Some(po);
            }
        }
    }

    let lang_tags = [
        ("html", "html"),
        ("htm", "html"),
        ("python", "py"),
        ("py", "py"),
        ("javascript", "js"),
        ("js", "js"),
        ("css", "css"),
        ("rust", "rs"),
        ("rs", "rs"),
        ("json", "json"),
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
                let target_file = requested_files
                    .iter()
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
    pub async fn execute(
        workspace_path: &str,
        args: serde_json::Value,
    ) -> Result<ExecutionResult, String> {
        // Direct 'cambios' array passed explicitly
        if let Some(cambios_val) = args
            .get("cambios")
            .and_then(|v| serde_json::from_value::<Vec<Cambio>>(v.clone()).ok())
        {
            if !cambios_val.is_empty() {
                return Self::apply_and_validate(workspace_path, cambios_val).await;
            }
        }

        let task = args
            .get("instruccion")
            .or_else(|| args.get("prompt"))
            .or_else(|| args.get("task"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let repair_attempt = args.get("repair_attempt").and_then(|value| value.as_u64()).unwrap_or(0);
        let semantic_repair = args.get("semantic_repair").and_then(|value| value.as_bool()).unwrap_or(false);

        let mut files: Vec<String> =
            if let Some(arr) = args.get("archivos_a_editar").and_then(|v| v.as_array()) {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect()
            } else if let Some(arr) = args.get("archivos").and_then(|v| v.as_array()) {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect()
            } else if let Some(s) = args.get("archivo").and_then(|v| v.as_str()) {
                vec![s.to_string()]
            } else {
                vec![]
            };

        let context = args
            .get("context")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let model = args
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("qwen2.5-coder:7b")
            .to_string();

        if files.is_empty() && task.is_empty() {
            return Ok(ExecutionResult::error(
                "TOOL_PROGRAMMER requiere al menos 'archivos_a_editar' o 'instruccion'.",
                1,
            ));
        }

        // Auto-heal check: if workspace has compile/syntax errors, add failing files
        if let Err(compile_err) = crate::core::validate_workspace(workspace_path).await {
            let failing_files =
                crate::core::extract_workspace_files_from_error(workspace_path, &compile_err);
            for ff in failing_files {
                if !files.contains(&ff) {
                    files.push(ff);
                }
            }
        }

        let repair_path = WorkspaceResolver::resolve_create_path(workspace_path, ".aura/programmer_failure.json")?;
        let repair = tokio::fs::read_to_string(repair_path).await.unwrap_or_default();
        let repair_value = serde_json::from_str::<serde_json::Value>(&repair).ok();
        let repair_error = repair_value
            .as_ref()
            .and_then(|value| value.get("error"))
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string();
        let rejected_changes: Vec<Cambio> = repair_value
            .as_ref()
            .and_then(|value| value.get("cambios").cloned())
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        // A failed multi-file proposal is kept for recovery, but the next model call
        // should repair only the file named by the validator. The remaining rejected
        // files are merged back below and validated as one transaction.
        let failing_rejected: Vec<Cambio> = rejected_changes
            .iter()
            .filter(|change| repair_error.contains(&change.archivo))
            .cloned()
            .collect();
        if repair_attempt > 0 && !failing_rejected.is_empty() {
            let mut focused_files: Vec<String> = failing_rejected
                .iter()
                .map(|change| change.archivo.clone())
                .collect();
            // Keep newly requested files named by the validator. A rejected HTML
            // draft may reference a JS/CSS file that did not exist in the original
            // proposal; dropping it here made every repair repeat the same error.
            for requested in &files {
                if repair_error.contains(requested) && !focused_files.contains(requested) {
                    focused_files.push(requested.clone());
                }
            }
            files = focused_files;
        }
        let mut context_files = files.clone();
        // Verification needs the implementation, even when only the verifier is edited.
        if files.iter().any(|f| f.contains("verify_") || f.contains("test_")) {
            if let Ok(entries) = std::fs::read_dir(workspace_path) {
                let mut sources: Vec<_> = entries.flatten().filter_map(|entry| {
                    let path = entry.path();
                    if path.is_file() && matches!(path.extension().and_then(|v| v.to_str()), Some("html" | "css" | "js" | "ts" | "py" | "rs")) {
                        Some(entry.file_name().to_string_lossy().to_string())
                    } else { None }
                }).collect();
                sources.sort();
                for source in sources { if !context_files.contains(&source) { context_files.push(source); } }
            }
        }
        let safe_files = crate::memory::read_files_safely(workspace_path, context_files).await;
        let repair_context = if repair_attempt > 0 && !repair_error.is_empty() {
            serde_json::json!({
                "error": repair_error,
                "cambios_a_corregir": failing_rejected,
            })
            .to_string()
        } else {
            String::new()
        };
        let file_contents = format!("{}\n\n{}\n[BORRADOR RECHAZADO Y DIAGNÓSTICO, NO ESTÁ EN DISCO]\n{}", context, safe_files, repair_context);
        let scope = if semantic_repair {
            format!("Archivos relacionados permitidos: {:?}. Modifica únicamente los necesarios para corregir el fallo semántico.", files)
        } else {
            format!("Entregables requeridos: {:?}. Incluye los necesarios para completar la operación.", files)
        };
        let task = format!("{}\nIntento de reparación: {}. {} Si hay un borrador rechazado, corrígelo con el diagnóstico exacto y genera una solución distinta; no repitas el borrador rechazado ni ejecutes un archivo que fue revertido.", task, repair_attempt, scope);

        let prompt_res = crate::llm::delegate_to_programmer(
            &task,
            &file_contents,
            &files,
            semantic_repair,
            &model,
        )
        .await;
        let json_res = match prompt_res {
            Ok(res) => res,
            Err(e) => {
                return Ok(ExecutionResult::error(
                    format!("Error delegando a programador: {}", e),
                    1,
                ))
            }
        };

        let clean_json = strip_think_tags(json_res.clone());
        let parsed = serde_json::from_str::<crate::llm::ProgrammerOutput>(&clean_json)
            .ok()
            .or_else(|| try_salvage_programmer_output(&json_res, &files));

        let mut prog_output = match parsed {
            Some(po) => po,
            None => {
                return Ok(ExecutionResult::error(
                    format!(
                        "No se pudo interpretar el JSON del programador: {}",
                        json_res
                    ),
                    1,
                ))
            }
        };

        if prog_output.cambios.is_empty() {
            return Ok(ExecutionResult::error(
                "El programador no propuso ningún cambio en 'cambios'.",
                1,
            ));
        }

        // Small models often return a patch against the preserved rejected draft,
        // even though that draft was rolled back and is not yet on disk. Resolve
        // that patch locally and turn it into a complete replacement.
        if repair_attempt > 0 {
            for change in &mut prog_output.cambios {
                let physical = std::path::Path::new(workspace_path).join(&change.archivo);
                if change.buscar.is_empty() {
                    continue;
                }
                let patch_matches_current = std::fs::read_to_string(&physical)
                    .ok()
                    .map(|current| {
                        crate::memory::apply_patch_to_string(
                            &current,
                            true,
                            &change.buscar,
                            &change.reemplazar,
                            &change.archivo,
                        )
                        .1
                    })
                    .unwrap_or(false);
                if patch_matches_current {
                    continue;
                }
                if let Some(draft) = rejected_changes
                    .iter()
                    .find(|draft| draft.archivo == change.archivo)
                {
                    let (updated, matched, _) = crate::memory::apply_patch_to_string(
                        &draft.reemplazar,
                        true,
                        &change.buscar,
                        &change.reemplazar,
                        &change.archivo,
                    );
                    if matched {
                        change.buscar.clear();
                        change.reemplazar = updated;
                    }
                }
            }
        }

        // A syntax repair may return only the broken file. Preserve the other files
        // from the rejected all-or-nothing proposal and validate the merged proposal.
        for rejected in rejected_changes {
            if !prog_output.cambios.iter().any(|change| change.archivo == rejected.archivo) {
                prog_output.cambios.push(rejected);
            }
        }

        // A focused repair must be monotonic: it may add a missing capability,
        // but it cannot erase capabilities that were already present.
        if semantic_repair {
            for change in &prog_output.cambios {
                let physical = std::path::Path::new(workspace_path).join(&change.archivo);
                let Ok(current) = std::fs::read_to_string(&physical) else { continue };
                let (updated, matched, _) = crate::memory::apply_patch_to_string(
                    &current,
                    true,
                    &change.buscar,
                    &change.reemplazar,
                    &change.archivo,
                );
                if !matched { continue; }
                let regressions = semantic_repair_regressions(&current, &updated);
                let destructive_shrink = current.chars().count() > 1000
                    && updated.chars().count() * 2 < current.chars().count();
                if destructive_shrink || !regressions.is_empty() {
                    let detail = if regressions.is_empty() {
                        "el archivo perdería más de la mitad de su contenido".to_string()
                    } else {
                        format!(
                            "se eliminarían funciones ya válidas: {}",
                            regressions.join(", ")
                        )
                    };
                    return Ok(ExecutionResult::error(
                        format!("SEMANTIC_REPAIR_REGRESSION: {}: {}. Conserva el archivo actual y añade solo la corrección solicitada.", change.archivo, detail),
                        1,
                    ));
                }
            }
        }

        Self::apply_and_validate(workspace_path, prog_output.cambios).await
    }

    pub async fn apply_and_validate(
        workspace_path: &str,
        cambios: Vec<Cambio>,
    ) -> Result<ExecutionResult, String> {
        // Snapshot only the files in this transaction. Never commit/clean the user's repository.
        let resolver = WorkspaceResolver::new(std::path::Path::new(workspace_path))?;
        let mut snapshots: Vec<(std::path::PathBuf, Option<Vec<u8>>)> = Vec::new();
        let mut proposed: Vec<(std::path::PathBuf, String)> = Vec::new();
        for cambio in &cambios {
            let path = resolver.resolve_for_create(&cambio.archivo)?;
            if proposed.iter().any(|(p, _)| p == &path) {
                return Ok(ExecutionResult::error(format!("DUPLICATE_TARGET: La respuesta JSON contiene varias entradas para {}. No se escribió ningún archivo. Conserva el nombre solicitado y devuelve una sola entrada con su contenido completo; no cambies de nombre el entregable.", cambio.archivo), 1));
            }
            let original = if path.exists() { Some(tokio::fs::read(&path).await.map_err(|e| e.to_string())?) } else { None };
            let text = match &original {
                Some(bytes) => std::str::from_utf8(bytes).map_err(|e| e.to_string())?,
                None => "",
            };
            let (mut updated, matched, _) = crate::memory::apply_patch_to_string(
                text, original.is_some(), &cambio.buscar, &cambio.reemplazar, &cambio.archivo);
            if !matched {
                return Ok(ExecutionResult::error(format!("PATCH_NOT_FOUND: {}. Lee el archivo actual o proporciona un reemplazo completo.", cambio.archivo), 1));
            }
            if cambio.archivo.to_lowercase().ends_with(".html") {
                updated = heal_inline_script_entities(&updated);
            }
            let report = crate::core::stub_enforcer::detect_stubs(&updated, &cambio.archivo);
            if report.has_stubs { return Ok(ExecutionResult::error(report.rejection_message, 1)); }
            snapshots.push((path.clone(), original));
            proposed.push((path, updated));
        }
        let operation: Result<(), String> = async {
            for (path, content) in &proposed {
                if let Some(parent) = path.parent() { tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?; }
                tokio::fs::write(path, content).await.map_err(|e| e.to_string())?;
            }
            crate::core::validate_workspace(workspace_path).await
        }.await;
        if let Err(error) = operation {
            for (path, original) in snapshots.iter().rev() {
                match original {
                    Some(bytes) => tokio::fs::write(path, bytes).await.map_err(|e| format!("ROLLBACK_FAILED: {}: {}", path.display(), e))?,
                    None if path.is_file() => tokio::fs::remove_file(path).await.map_err(|e| format!("ROLLBACK_FAILED: {}: {}", path.display(), e))?,
                    None => {},
                }
            }
            let failure = resolver.resolve_for_create(".aura/programmer_failure.json")?;
            tokio::fs::create_dir_all(failure.parent().unwrap()).await.map_err(|e| e.to_string())?;
            let rejected_full_changes: Vec<Cambio> = cambios
                .iter()
                .zip(proposed.iter())
                .map(|(change, (_, content))| Cambio {
                    archivo: change.archivo.clone(),
                    buscar: String::new(),
                    reemplazar: content.clone(),
                })
                .collect();
            tokio::fs::write(&failure, serde_json::to_vec_pretty(&serde_json::json!({
                "error": error, "cambios": rejected_full_changes,
            })).map_err(|e| e.to_string())?).await.map_err(|e| e.to_string())?;
            let mut result = ExecutionResult::error(format!("[VALIDATION_FAILED] {}\nSe revirtieron únicamente los archivos de esta propuesta. El borrador y diagnóstico están en .aura/programmer_failure.json para su reparación.", error), 1);
            result.cwd = Some(workspace_path.into());
            return Ok(result);
        }
        let failure = resolver.resolve_for_create(".aura/programmer_failure.json")?;
        if failure.is_file() { tokio::fs::remove_file(failure).await.map_err(|e| e.to_string())?; }
        let files: Vec<String> = cambios.iter().map(|c| c.archivo.clone()).collect();
        let mut result = ExecutionResult::success(format!("Archivos escritos: {:?}. Validación sintáctica aprobada; las pruebas funcionales siguen pendientes.", files));
        result.files_affected = files;
        result.cwd = Some(workspace_path.into());
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn failed_generation_preserves_user_files_and_draft_for_repair() {
        let root = std::env::temp_dir().join(format!("aura-transaction-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("user-notes.txt"), "uncommitted user notes").unwrap();
        std::fs::write(root.join("existing.py"), "value = 1\n").unwrap();
        let result = ProgrammerExecutor::apply_and_validate(root.to_str().unwrap(), vec![
            Cambio { archivo: "existing.py".into(), buscar: "".into(), reemplazar: "value = 2\n".into() },
            Cambio { archivo: "verify_dashboard.py".into(), buscar: "".into(), reemplazar: "text = \"unterminated\n".into() },
        ]).await.unwrap();
        assert_ne!(result.exit_code, 0);
        assert_eq!(std::fs::read_to_string(root.join("existing.py")).unwrap(), "value = 1\n");
        assert_eq!(std::fs::read_to_string(root.join("user-notes.txt")).unwrap(), "uncommitted user notes");
        assert!(!root.join("verify_dashboard.py").exists());
        assert!(root.join(".aura/programmer_failure.json").exists());
        assert!(result.files_affected.is_empty(), "Rolled-back changes cannot be reported as physical progress");
        let repaired = ProgrammerExecutor::apply_and_validate(root.to_str().unwrap(), vec![Cambio {
            archivo: "verify_dashboard.py".into(), buscar: "".into(), reemplazar: "import json\nif __name__ == '__main__':\n    passed = 1\n    total = 1\n    percentage = 100 * passed / total\n    failed_criteria = []\n    print(json.dumps({'passed': passed, 'total': total, 'percentage': percentage, 'failed_criteria': failed_criteria}))\n    exit(0 if passed == total else 1)\n".into(),
        }]).await.unwrap();
        assert_eq!(repaired.exit_code, 0, "{}", repaired.stderr);
        assert!(std::fs::read_to_string(root.join("verify_dashboard.py")).unwrap().contains("json.dumps"));
        assert!(!root.join(".git").exists(), "The programmer must not initialize or modify Git");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn missing_patch_does_not_report_success_or_modify_file() {
        let root = std::env::temp_dir().join(format!("aura-patch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("file.txt"), "original").unwrap();
        let result = ProgrammerExecutor::apply_and_validate(root.to_str().unwrap(), vec![Cambio {
            archivo: "file.txt".into(), buscar: "missing".into(), reemplazar: "replacement".into(),
        }]).await.unwrap();
        assert_ne!(result.exit_code, 0);
        assert_eq!(std::fs::read_to_string(root.join("file.txt")).unwrap(), "original");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn html_encoded_javascript_operators_are_healed_before_validation() {
        let root =
            std::env::temp_dir().join(format!("aura-html-entity-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let html = "<!doctype html><script>const values = [1]; values.forEach(value =&gt; console.log(value));</script>";
        let result = ProgrammerExecutor::apply_and_validate(
            root.to_str().unwrap(),
            vec![Cambio {
                archivo: "index.html".into(),
                buscar: String::new(),
                reemplazar: html.into(),
            }],
        )
        .await
        .unwrap();
        assert_eq!(result.exit_code, 0, "{}", result.stderr);
        let saved = std::fs::read_to_string(root.join("index.html")).unwrap();
        assert!(saved.contains("value =>"));
        assert!(!saved.contains("=&gt;"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn semantic_repair_detects_loss_of_existing_capabilities() {
        let before = "<!doctype html><canvas></canvas><script>requestAnimationFrame(draw); Math.cos(1); Math.sin(1); const threatNodes=[]; const packets=[]; function traffic(){}; alert('ataque');</script><style>body{font-family:monospace;backdrop-filter:blur(8px)}</style>";
        let after = "<div id='telemetry'>Telemetría de paquetes</div>";
        let regressions = semantic_repair_regressions(before, after);
        assert!(regressions.contains(&"Canvas"));
        assert!(regressions.contains(&"animación"));
        assert!(regressions.contains(&"tráfico"));
        assert!(regressions.contains(&"glassmorphism"));
    }

    #[tokio::test]
    async fn failed_patch_persists_the_complete_rejected_draft() {
        let root = std::env::temp_dir().join(format!("aura-draft-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("app.js"), "function run() { return 1; }\n").unwrap();
        let result = ProgrammerExecutor::apply_and_validate(
            root.to_str().unwrap(),
            vec![Cambio {
                archivo: "app.js".into(),
                buscar: "return 1;".into(),
                reemplazar: "return 2; }}".into(),
            }],
        )
        .await
        .unwrap();
        assert_ne!(result.exit_code, 0);
        let failure: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join(".aura/programmer_failure.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(failure["cambios"][0]["buscar"], "");
        assert!(failure["cambios"][0]["reemplazar"]
            .as_str()
            .unwrap()
            .contains("function run()"));
        assert_eq!(
            std::fs::read_to_string(root.join("app.js")).unwrap(),
            "function run() { return 1; }\n"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
