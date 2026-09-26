use crate::core::tool_registry::ExecutionResult;
use crate::core::workspace_resolver::WorkspaceResolver;
use crate::memory::Cambio;

pub struct ProgrammerExecutor;

fn normalized_workspace_target(file: &str) -> String {
    file.replace('\\', "/")
        .trim_start_matches("./")
        .to_ascii_lowercase()
}

fn same_workspace_target(left: &str, right: &str) -> bool {
    normalized_workspace_target(left) == normalized_workspace_target(right)
}

fn is_test_artifact_target(file: &str) -> bool {
    let normalized = file.replace('\\', "/").to_ascii_lowercase();
    let name = normalized.rsplit('/').next().unwrap_or(&normalized);
    name.starts_with("verify_")
        || name.starts_with("test_")
        || [
            ".test.js",
            ".spec.js",
            ".test.ts",
            ".spec.ts",
            ".test.mjs",
            ".spec.mjs",
            ".test.cjs",
            ".spec.cjs",
        ]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

fn deduplicate_targets(files: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    files
        .into_iter()
        .filter(|file| seen.insert(normalized_workspace_target(file)))
        .collect()
}

fn missing_proposed_targets(requested: &[String], changes: &[Cambio]) -> Vec<String> {
    requested
        .iter()
        .filter(|target| {
            !changes
                .iter()
                .any(|change| same_workspace_target(&change.archivo, target))
        })
        .cloned()
        .collect()
}

fn map_known_target_aliases(changes: &mut [Cambio], requested: &[String]) {
    let requested_test = requested
        .iter()
        .any(|target| same_workspace_target(target, "tests/app.test.js"));
    if requested_test {
        for change in changes {
            if same_workspace_target(&change.archivo, "app.test.js") {
                change.archivo = "tests/app.test.js".into();
            }
        }
    }
}

fn filter_changes_to_targets(changes: Vec<Cambio>, targets: &[String]) -> Vec<Cambio> {
    if targets.is_empty() {
        return changes;
    }
    changes
        .into_iter()
        .filter(|change| {
            targets
                .iter()
                .any(|target| same_workspace_target(&change.archivo, target))
        })
        .collect()
}

fn error_mentions_target(error: &str, target: &str) -> bool {
    let target = normalized_workspace_target(target);
    let normalized_error = error.replace('\\', "/").to_ascii_lowercase();
    normalized_error.contains(&target)
}

fn repair_error_matches_scope(error: &str, targets: &[String]) -> bool {
    targets.is_empty()
        || targets
            .iter()
            .any(|target| error_mentions_target(error, target))
}

fn matching_rejected_changes(
    rejected: &[Cambio],
    repair_error: &str,
    targets: &[String],
) -> Vec<Cambio> {
    rejected
        .iter()
        .filter(|change| {
            targets
                .iter()
                .any(|target| same_workspace_target(&change.archivo, target))
                && error_mentions_target(repair_error, &change.archivo)
        })
        .cloned()
        .collect()
}

fn requires_full_file_rewrite(repair_error: &str) -> bool {
    [
        "DUPLICATE_TARGET_CONFLICT",
        "declarada varias veces",
        "Identifier has already been declared",
        "AUDIT_REQUIRED_TARGETS_MISSING",
        "PATCH_NOT_FOUND",
        "FULL_REWRITE_REQUIRED",
        "JSON_SYNTAX_ERROR",
        "PROGRAMMER_OUTPUT_INVALID_JSON",
        "FIREBASE_CONFIG_INVALID",
        "PYTHON_SYNTAX_ERROR",
        "JAVASCRIPT_SYNTAX_ERROR",
        "NODE_SYNTAX_ERROR",
    ]
    .iter()
    .any(|marker| repair_error.contains(marker))
}

fn invalid_programmer_output_diagnostic(raw: &str, requested_files: &[String]) -> String {
    format!(
        "PROGRAMMER_OUTPUT_INVALID_JSON: el modelo devolvió una respuesta estructurada inválida o truncada ({} caracteres) para {:?}. No se modificó ningún archivo y se descartó el cuerpo para evitar inflar el historial. Reintenta generando un único cambio JSON compacto a partir del contenido real del workspace; elimina datos y comentarios repetitivos.",
        raw.chars().count(),
        requested_files
    )
}

fn requires_incremental_repair(
    semantic_repair: bool,
    has_matching_rejected_draft: bool,
    repair_error: &str,
) -> bool {
    if requires_full_file_rewrite(repair_error) {
        // A patch-on-patch repair caused the failed duplicate declarations in
        // the consultation MVP. Syntax errors have the same failure mode: the
        // rejected draft is not a trustworthy base for another incremental patch.
        // Rebuild each affected file from its actual workspace contents.
        return false;
    }
    semantic_repair
        || (has_matching_rejected_draft
            && [
                "ANTI-STUB ENFORCER",
                "NODE_SCRIPTS_MISSING",
                "NODE_TEST_SCRIPT_INVALID",
            ]
            .iter()
            .any(|marker| repair_error.contains(marker)))
}

pub(crate) fn is_internal_workspace_file(file: &str) -> bool {
    let normalized = file.replace('\\', "/").to_lowercase();
    let trimmed = normalized.trim_start_matches("./");
    let basename = trimmed.rsplit('/').next().unwrap_or(trimmed);
    let basename_without_dot = basename.trim_start_matches('.');
    matches!(
        basename_without_dot,
        "aura_session.json"
            | "aura_command_trail.json"
            | "aura_graph.json"
            | "aura_logs.jsonl"
            | "fenix_memory.json"
            | "fenix_chat.json"
            | "fenix_index.json"
    ) || trimmed == ".aura"
        || trimmed == "aura"
        || trimmed.starts_with(".aura/")
        || trimmed.starts_with("aura/")
        || trimmed == ".git"
        || trimmed == "git"
        || trimmed.starts_with(".git/")
        || trimmed.starts_with("git/")
}

fn coalesce_duplicate_changes(
    workspace_path: &str,
    changes: Vec<Cambio>,
    rejected: &[Cambio],
) -> Result<Vec<Cambio>, String> {
    let mut merged: Vec<Cambio> = Vec::new();
    for change in changes {
        if let Some(existing) = merged
            .iter_mut()
            .find(|item| item.archivo.eq_ignore_ascii_case(&change.archivo))
        {
            let base = if existing.buscar.is_empty() {
                existing.reemplazar.clone()
            } else {
                let resolver = WorkspaceResolver::new(std::path::Path::new(workspace_path))
                    .map_err(|error| error.to_string())?;
                let path = resolver
                    .resolve_for_create(&existing.archivo)
                    .map_err(|error| error.to_string())?;
                std::fs::read_to_string(path)
                    .ok()
                    .or_else(|| {
                        rejected
                            .iter()
                            .find(|draft| draft.archivo.eq_ignore_ascii_case(&existing.archivo))
                            .map(|draft| draft.reemplazar.clone())
                    })
                    .unwrap_or_default()
            };
            let (current, first_matched, _) = crate::memory::apply_patch_to_string(
                &base,
                true,
                &existing.buscar,
                &existing.reemplazar,
                &existing.archivo,
            );
            if !first_matched {
                return Err(format!(
                    "DUPLICATE_TARGET_CONFLICT: no se pudo aplicar el primer cambio de {}.",
                    existing.archivo
                ));
            }
            let (updated, matched, _) = crate::memory::apply_patch_to_string(
                &current,
                true,
                &change.buscar,
                &change.reemplazar,
                &change.archivo,
            );
            if !matched {
                return Err(format!(
                    "DUPLICATE_TARGET_CONFLICT: {} tiene cambios repetidos que no se pueden combinar de forma segura.",
                    change.archivo
                ));
            }
            existing.buscar.clear();
            existing.reemplazar = updated;
        } else {
            merged.push(change);
        }
    }
    Ok(merged)
}

async fn persist_rejected_proposal(
    workspace_path: &str,
    error: &str,
    changes: &[Cambio],
) -> Result<(), String> {
    let resolver = WorkspaceResolver::new(std::path::Path::new(workspace_path))?;
    let failure = resolver.resolve_for_create(".aura/programmer_failure.json")?;
    if let Some(parent) = failure.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|write_error| write_error.to_string())?;
    }
    let record = serde_json::to_vec_pretty(&serde_json::json!({
        "error": error,
        "cambios": changes,
    }))
    .map_err(|write_error| write_error.to_string())?;
    tokio::fs::write(failure, record)
        .await
        .map_err(|write_error| write_error.to_string())
}

fn strip_think_tags(mut text: String) -> String {
    while let (Some(start), Some(end)) = (text.find("<think>"), text.find("</think>")) {
        if end + 8 <= text.len() {
            text.replace_range(start..end + 8, "");
        } else {
            break;
        }
    }

    text.trim().to_string()
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
    if let Ok(value) = crate::core::structured_json::parse_json_object(raw) {
        if let Ok(output) = serde_json::from_value::<crate::llm::ProgrammerOutput>(value) {
            return Some(output);
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
        let repair_attempt = args
            .get("repair_attempt")
            .and_then(|value| value.as_u64())
            .unwrap_or(0);
        let semantic_repair = args
            .get("semantic_repair")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let require_all_targets = args
            .get("require_all_targets")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

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
        files = deduplicate_targets(files);

        let protected_files: Vec<String> = files
            .iter()
            .filter(|file| is_internal_workspace_file(file))
            .cloned()
            .collect();
        files.retain(|file| !is_internal_workspace_file(file));
        if files.is_empty() && !protected_files.is_empty() {
            return Ok(ExecutionResult::error(
                format!("INTERNAL_FILE_PROTECTED: TOOL_PROGRAMMER no puede modificar archivos internos del agente: {:?}.", protected_files),
                1,
            ));
        }

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

        let is_no_op = {
            let t = task.to_lowercase();
            (t.contains("no necesita") || t.contains("no requiere") || t.contains("no se requiere") || t.contains("no hace falta"))
                && (t.contains("modifica") || t.contains("cambio") || t.contains("accion") || t.contains("acción"))
        };
        if is_no_op {
            return Ok(ExecutionResult::success(
                "No se requieren modificaciones en los archivos; el estado actual se conserva intacto.",
            ));
        }

        // Only discover workspace-wide failures when no explicit phase/file scope
        // was supplied. A phase must not inherit unrelated broken files.
        if files.is_empty() {
            if let Err(compile_err) = crate::core::validate_workspace(workspace_path).await {
                let failing_files =
                    crate::core::extract_workspace_files_from_error(workspace_path, &compile_err);
                for ff in failing_files {
                    if !files.contains(&ff) {
                        files.push(ff);
                    }
                }
            }
        }

        let repair_path = WorkspaceResolver::resolve_create_path(
            workspace_path,
            ".aura/programmer_failure.json",
        )?;
        let repair = tokio::fs::read_to_string(repair_path)
            .await
            .unwrap_or_default();
        let repair_value = serde_json::from_str::<serde_json::Value>(&repair).ok();
        let persisted_repair_error = repair_value
            .as_ref()
            .and_then(|value| value.get("error"))
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string();
        let repair_error = args
            .get("repair_error")
            .and_then(|value| value.as_str())
            .filter(|error| !error.trim().is_empty())
            .map(str::to_string)
            .or_else(|| {
                repair_error_matches_scope(&persisted_repair_error, &files)
                    .then_some(persisted_repair_error)
            })
            .unwrap_or_default();
        let rejected_changes: Vec<Cambio> = repair_value
            .as_ref()
            .and_then(|value| value.get("cambios").cloned())
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        // Reuse a rejected draft only when it belongs to a file explicitly in the
        // current scope. Old failure records can outlive a phase transition.
        let failing_rejected = if repair_error.contains("AUDIT_REQUIRED_TARGETS_MISSING") {
            Vec::new()
        } else {
            matching_rejected_changes(&rejected_changes, &repair_error, &files)
        };
        if !failing_rejected.is_empty() {
            let mut focused_files: Vec<String> = failing_rejected
                .iter()
                .map(|change| change.archivo.clone())
                .collect();
            for requested in &files {
                if error_mentions_target(&repair_error, requested)
                    && !focused_files
                        .iter()
                        .any(|file| same_workspace_target(file, requested))
                {
                    focused_files.push(requested.clone());
                }
            }
            files = deduplicate_targets(focused_files);
        }
        if !require_all_targets
            && repair_error.contains("PROGRAMMER_OUTPUT_INVALID_JSON")
            && files.len() > 1
        {
            // Recover a truncated multi-file response from one real source file
            // at a time. Prefer product code over a test artifact so the repair
            // cannot spend its small output budget rebuilding an unused test.
            let target = files
                .iter()
                .find(|file| !is_test_artifact_target(file))
                .or_else(|| files.first())
                .cloned();
            if let Some(target) = target {
                files = vec![target];
            }
        }
        let mut context_files = files.clone();
        let is_verifier_target = files
            .iter()
            .any(|f| f.contains("verify_") || f.contains("test_"));
        let is_frontend_target = files.iter().any(|f| {
            let lower = f.to_lowercase();
            lower.ends_with(".js")
                || lower.ends_with(".ts")
                || lower.ends_with(".css")
                || lower.ends_with(".html")
        });

        // Verification needs the implementation, and frontend files (like script.js)
        // need the existing HTML/CSS files to inspect IDs and structure.
        if is_verifier_target || is_frontend_target {
            if let Ok(entries) = std::fs::read_dir(workspace_path) {
                let mut sources: Vec<_> = entries
                    .flatten()
                    .filter_map(|entry| {
                        let path = entry.path();
                        if path.is_file()
                            && matches!(
                                path.extension().and_then(|v| v.to_str()),
                                Some("html" | "css" | "js" | "ts" | "py" | "rs")
                            )
                        {
                            let name = entry.file_name().to_string_lossy().to_string();
                            if !is_internal_workspace_file(&name) {
                                Some(name)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                    .collect();
                sources.sort();
                for source in sources {
                    if !context_files.contains(&source) {
                        context_files.push(source);
                    }
                }
            }
        }
        let safe_files = crate::memory::read_files_safely(workspace_path, context_files).await;
        let repair_context = if !failing_rejected.is_empty() {
            if requires_full_file_rewrite(&repair_error) {
                let failing_files: Vec<_> = failing_rejected.iter().map(|c| &c.archivo).collect();
                serde_json::json!({
                    "error": repair_error,
                    "archivos_con_error": failing_files,
                    "aviso": "El borrador anterior fue descartado por error de sintaxis o conflicto. Reescribe el archivo COMPLETO desde cero con buscar vacío y sintaxis válida y cerrada."
                })
                .to_string()
            } else {
                serde_json::json!({
                    "error": repair_error,
                    "cambios_a_corregir": failing_rejected,
                })
                .to_string()
            }
        } else {
            String::new()
        };
        let file_contents = format!(
            "{}\n\n{}\n[BORRADOR RECHAZADO Y DIAGNÓSTICO, NO ESTÁ EN DISCO]\n{}",
            context, safe_files, repair_context
        );
        let scope = if require_all_targets {
            format!(
                "Archivo de reparación obligatorio: {:?}. Devuelve un único cambio para este objetivo.",
                files
            )
        } else if semantic_repair {
            format!("Archivos relacionados permitidos: {:?}. Modifica únicamente los necesarios para corregir el fallo semántico.", files)
        } else {
            format!(
                "Entregables requeridos: {:?}. Incluye los necesarios para completar la operación.",
                files
            )
        };
        let repair_instruction = if repair_error.contains("PROGRAMMER_OUTPUT_INVALID_JSON") {
            "La respuesta anterior no era JSON válido y no llegó a escribirse. Ignora por completo ese borrador; vuelve a leer el archivo objetivo actual y genera exactamente un objeto JSON válido con un único cambio para un único archivo. Usa buscar vacío solo para crear o reemplazar el archivo completo. No incluyas tablas de ejemplo, filas repetitivas ni comentarios de relleno. Mantén el resultado compacto y respeta estrictamente el esquema JSON."
        } else if repair_error.contains("PATCH_NOT_FOUND")
            || repair_error.contains("FULL_REWRITE_REQUIRED")
        {
            "El fragmento buscar del intento anterior no existe exactamente en el archivo actual. Relee el contenido real incluido arriba y reemplaza COMPLETO el archivo objetivo con una versión corregida. Devuelve buscar como cadena vacía y reemplazar con el archivo entero; no inventes un fragmento ni intentes otro parche parcial. Conserva las funciones y pruebas válidas del archivo actual."
        } else if (repair_error.contains("JSON_SYNTAX_ERROR")
            || repair_error.contains("FIREBASE_CONFIG_INVALID"))
            && error_mentions_target(&repair_error, "firebase.json")
        {
            r#"firebase.json fue rechazado. Reemplaza COMPLETO ese archivo con JSON estricto válido: usa comillas dobles, sin comentarios ni comas finales, y nunca pongas saltos de línea literales dentro de una cadena. Para Firestore, firebase.json solo debe referenciar archivos, por ejemplo "firestore": {"rules": "firestore.rules", "indexes": "firestore.indexes.json"}; el código de reglas va en el archivo separado firestore.rules, jamás como texto dentro de firebase.json. Conserva Hosting local para este sitio estático y los puertos de emulador ya definidos si existen. Devuelve buscar vacío y un único cambio para firebase.json."#
        } else if repair_error.contains("JSON_SYNTAX_ERROR")
            || repair_error.contains("FIREBASE_CONFIG_INVALID")
        {
            "El archivo JSON fue rechazado. Reemplaza COMPLETO únicamente el archivo indicado con JSON estricto válido: usa comillas dobles, escapa saltos de línea dentro de cadenas, no uses comentarios ni comas finales y conserva los datos válidos del borrador. Devuelve buscar vacío; no edites otros archivos."
        } else if repair_error.contains("PYTHON_SYNTAX_ERROR") {
            "El verificador Python rechazado tiene un error de sintaxis. Reescribe el archivo objetivo completo como Python válido, conservando las comprobaciones útiles del borrador pero corrigiendo la estructura; no entregues un parche parcial ni reutilices como código el objeto JSON del plan. Ejecuta después el verificador para confirmar que compila."
        } else if repair_error.contains("JAVASCRIPT_SYNTAX_ERROR")
            || repair_error.contains("NODE_SYNTAX_ERROR")
        {
            "El archivo JavaScript rechazado tiene un error de sintaxis. Reescribe el archivo objetivo completo como JavaScript válido usando el contenido real del workspace como referencia; no añadas otro parche parcial ni dupliques declaraciones."
        } else if repair_error.contains("AUDIT_REQUIRED_TARGETS_MISSING") {
            "La propuesta anterior omitió el archivo obligatorio. Genera ahora exactamente un cambio completo para el objetivo enumerado, usando el contenido actual del workspace y el diagnóstico; no modifiques otros archivos."
        } else if repair_error.contains("DUPLICATE_TARGET_CONFLICT")
            || repair_error.contains("declarada varias veces")
            || repair_error.contains("Identifier has already been declared")
        {
            "Reescribe cada archivo afectado como un único reemplazo completo basado en su contenido actual. Devuelve exactamente un elemento 'cambio' por archivo, sin destinos repetidos, conserva las funciones válidas y elimina declaraciones duplicadas. No añadas parches parciales al mismo destino."
        } else if failing_rejected.is_empty() {
            "No reutilices borradores rechazados guardados de otros archivos o fases."
        } else if repair_error.contains("ANTI-STUB ENFORCER") {
            "El diagnóstico señala funciones placeholder. Reemplaza solo el bloque defectuoso mediante un cambio incremental: conserva el resto del borrador, implementa la lógica real en esas funciones y no dupliques declaraciones ni pegues el archivo completo."
        } else if repair_error.contains("NODE_SCRIPTS_MISSING")
            || repair_error.contains("NODE_TEST_SCRIPT_INVALID")
        {
            "Corrige únicamente los scripts de package.json. Declara comandos no vacíos; si usas el runner integrado de Node, el comando es 'node --test tests/app.test.js' y 'node:test' solo se importa dentro del código de prueba, no se ejecuta como comando. No cambies los demás archivos."
        } else {
            "Corrige el borrador rechazado incluido en el diagnóstico mediante un cambio incremental y no ejecutes archivos revertidos."
        };
        let task = format!(
            "{}\nIntento de reparación: {}. {} {}",
            task, repair_attempt, scope, repair_instruction
        );
        let require_incremental_patch = requires_incremental_repair(
            semantic_repair,
            !failing_rejected.is_empty(),
            &repair_error,
        );
        let require_empty_search = requires_full_file_rewrite(&repair_error);

        let prompt_res = crate::llm::delegate_to_programmer(
            &task,
            &file_contents,
            &files,
            require_incremental_patch,
            require_empty_search,
            files.len().max(1),
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
        let parsed = crate::core::structured_json::parse_json_object(&clean_json)
            .ok()
            .and_then(|value| serde_json::from_value::<crate::llm::ProgrammerOutput>(value).ok())
            .or_else(|| try_salvage_programmer_output(&json_res, &files));

        let mut prog_output = match parsed {
            Some(po) => po,
            None => {
                return Ok(ExecutionResult::error(
                    invalid_programmer_output_diagnostic(&json_res, &files),
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

        // The approved consultation plan stores tests under tests/. A small
        // model sometimes omits that directory despite the explicit target;
        // map only this known alias when that exact path was requested.
        map_known_target_aliases(&mut prog_output.cambios, &files);

        if let Some(internal) = prog_output
            .cambios
            .iter()
            .find(|change| is_internal_workspace_file(&change.archivo))
        {
            return Ok(ExecutionResult::error(
                format!("INTERNAL_FILE_PROTECTED: TOOL_PROGRAMMER no puede modificar el archivo interno '{}'.", internal.archivo),
                1,
            ));
        }

        let scoped_change_count = prog_output.cambios.len();
        prog_output.cambios = filter_changes_to_targets(prog_output.cambios, &files);
        if prog_output.cambios.is_empty() {
            return Ok(ExecutionResult::error(
                format!(
                    "PROGRAMMER_SCOPE_MISMATCH: la respuesta propuso {} archivo(s), pero ninguno pertenece a los objetivos actuales {:?}.",
                    scoped_change_count, files
                ),
                1,
            ));
        }

        // Small models often return a patch against the preserved rejected draft,
        // even though that draft was rolled back and is not yet on disk. Resolve
        // that patch locally and turn it into a complete replacement.
        if !failing_rejected.is_empty() {
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

        let proposed_changes = std::mem::take(&mut prog_output.cambios);
        prog_output.cambios = match coalesce_duplicate_changes(
            workspace_path,
            proposed_changes.clone(),
            &failing_rejected,
        ) {
            Ok(changes) => changes,
            Err(error) => {
                let persistence =
                    persist_rejected_proposal(workspace_path, &error, &proposed_changes).await;
                let message = match persistence {
                    Ok(()) => format!(
                        "{error} Se conservaron los cambios propuestos y el diagnóstico en .aura/programmer_failure.json; la reparación siguiente deberá reescribir una sola vez cada archivo afectado."
                    ),
                    Err(persist_error) => format!(
                        "{error} No se pudo guardar el borrador de reparación: {persist_error}"
                    ),
                };
                return Ok(ExecutionResult::error(message, 1));
            }
        };

        if require_all_targets {
            let missing = missing_proposed_targets(&files, &prog_output.cambios);
            if !missing.is_empty() {
                let error = format!(
                    "AUDIT_REQUIRED_TARGETS_MISSING: la reparación omitió archivos que el auditor marcó como defectuosos: {:?}.",
                    missing
                );
                let persistence =
                    persist_rejected_proposal(workspace_path, &error, &prog_output.cambios).await;
                let message = match persistence {
                    Ok(()) => format!(
                        "{error} No se escribió ningún archivo; el diagnóstico se guardó para un intento completo."
                    ),
                    Err(persist_error) => format!(
                        "{error} No se pudo guardar el diagnóstico: {persist_error}"
                    ),
                };
                return Ok(ExecutionResult::error(message, 1));
            }
        }

        // A focused repair must be monotonic: it may add a missing capability,
        // but it cannot erase capabilities that were already present.
        if semantic_repair {
            for change in &prog_output.cambios {
                let physical = std::path::Path::new(workspace_path).join(&change.archivo);
                let Ok(current) = std::fs::read_to_string(&physical) else {
                    continue;
                };
                let (updated, matched, _) = crate::memory::apply_patch_to_string(
                    &current,
                    true,
                    &change.buscar,
                    &change.reemplazar,
                    &change.archivo,
                );
                if !matched {
                    continue;
                }
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

        if require_empty_search
            && prog_output
                .cambios
                .iter()
                .any(|change| !change.buscar.trim().is_empty())
        {
            let error = "FULL_REWRITE_REQUIRED: el modelo devolvió un parche parcial en una reparación que exige reemplazar el archivo completo; no se modificó ningún archivo.";
            let persistence =
                persist_rejected_proposal(workspace_path, error, &prog_output.cambios).await;
            let message = match persistence {
                Ok(()) => format!(
                    "{} El diagnóstico se conservó para reintentar con buscar vacío.",
                    error
                ),
                Err(persist_error) => format!(
                    "{} No se pudo guardar el borrador: {}",
                    error, persist_error
                ),
            };
            return Ok(ExecutionResult::error(message, 1));
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
            if is_internal_workspace_file(&cambio.archivo) {
                return Ok(ExecutionResult::error(
                    format!("INTERNAL_FILE_PROTECTED: TOOL_PROGRAMMER no puede modificar el archivo interno '{}'.", cambio.archivo),
                    1,
                ));
            }
            let path = resolver.resolve_for_create(&cambio.archivo)?;
            if proposed.iter().any(|(p, _)| p == &path) {
                let error = format!("DUPLICATE_TARGET_CONFLICT: La respuesta JSON contiene varias entradas para {}. No se escribió ningún archivo. Conserva el nombre solicitado y devuelve una sola entrada con su contenido completo; no cambies de nombre el entregable.", cambio.archivo);
                let persistence = persist_rejected_proposal(workspace_path, &error, &cambios).await;
                let message = match persistence {
                    Ok(()) => format!("{error} El borrador y diagnóstico quedaron guardados para reconstruir el archivo en el siguiente intento."),
                    Err(persist_error) => format!("{error} No se pudo guardar el borrador: {persist_error}"),
                };
                return Ok(ExecutionResult::error(message, 1));
            }
            let original = if path.exists() {
                Some(tokio::fs::read(&path).await.map_err(|e| e.to_string())?)
            } else {
                None
            };
            let text = match &original {
                Some(bytes) => std::str::from_utf8(bytes).map_err(|e| e.to_string())?,
                None => "",
            };
            let (mut updated, matched, _) = crate::memory::apply_patch_to_string(
                text,
                original.is_some(),
                &cambio.buscar,
                &cambio.reemplazar,
                &cambio.archivo,
            );
            if !matched {
                let error = format!(
                    "PATCH_NOT_FOUND: {}. El fragmento buscar no coincide con el archivo actual; vuelve a leerlo y propón un reemplazo completo con buscar vacío.",
                    cambio.archivo
                );
                let message = match persist_rejected_proposal(workspace_path, &error, &cambios).await
                {
                    Ok(()) => format!(
                        "{} El borrador rechazado y el diagnóstico se conservaron en .aura/programmer_failure.json; la siguiente reparación debe reemplazar el archivo completo desde su contenido actual.",
                        error
                    ),
                    Err(persist_error) => format!(
                        "{} No se pudo conservar el borrador: {}",
                        error, persist_error
                    ),
                };
                return Ok(ExecutionResult::error(message, 1));
            }
            if cambio.archivo.to_lowercase().ends_with(".html") {
                updated = heal_inline_script_entities(&updated);
            }
            let report = crate::core::stub_enforcer::detect_stubs(&updated, &cambio.archivo);
            if report.has_stubs {
                let failure = resolver.resolve_for_create(".aura/programmer_failure.json")?;
                tokio::fs::create_dir_all(failure.parent().unwrap())
                    .await
                    .map_err(|e| e.to_string())?;
                let rejected = Cambio {
                    archivo: cambio.archivo.clone(),
                    buscar: String::new(),
                    reemplazar: updated,
                };
                tokio::fs::write(
                    &failure,
                    serde_json::to_vec_pretty(&serde_json::json!({
                        "error": report.rejection_message,
                        "cambios": [rejected],
                    }))
                    .map_err(|e| e.to_string())?,
                )
                .await
                .map_err(|e| e.to_string())?;
                return Ok(ExecutionResult::error(report.rejection_message, 1));
            }
            snapshots.push((path.clone(), original));
            proposed.push((path, updated));
        }
        let changed_file_names: Vec<String> = cambios
            .iter()
            .map(|change| change.archivo.clone())
            .collect();
        let operation: Result<(), String> = async {
            for (path, content) in &proposed {
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                tokio::fs::write(path, content)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            crate::core::validation::validate_changed_files(workspace_path, &changed_file_names)
                .await
        }
        .await;
        if let Err(error) = operation {
            for (path, original) in snapshots.iter().rev() {
                match original {
                    Some(bytes) => tokio::fs::write(path, bytes)
                        .await
                        .map_err(|e| format!("ROLLBACK_FAILED: {}: {}", path.display(), e))?,
                    None if path.is_file() => tokio::fs::remove_file(path)
                        .await
                        .map_err(|e| format!("ROLLBACK_FAILED: {}: {}", path.display(), e))?,
                    None => {}
                }
            }
            let failure = resolver.resolve_for_create(".aura/programmer_failure.json")?;
            tokio::fs::create_dir_all(failure.parent().unwrap())
                .await
                .map_err(|e| e.to_string())?;
            let rejected_full_changes: Vec<Cambio> = cambios
                .iter()
                .zip(proposed.iter())
                .map(|(change, (_, content))| Cambio {
                    archivo: change.archivo.clone(),
                    buscar: String::new(),
                    reemplazar: content.clone(),
                })
                .collect();
            tokio::fs::write(
                &failure,
                serde_json::to_vec_pretty(&serde_json::json!({
                    "error": error, "cambios": rejected_full_changes,
                }))
                .map_err(|e| e.to_string())?,
            )
            .await
            .map_err(|e| e.to_string())?;
            let mut result = ExecutionResult::error(format!("[VALIDATION_FAILED] {}\nSe revirtieron únicamente los archivos de esta propuesta. El borrador y diagnóstico están en .aura/programmer_failure.json para su reparación.", error), 1);
            result.cwd = Some(workspace_path.into());
            return Ok(result);
        }
        let failure = resolver.resolve_for_create(".aura/programmer_failure.json")?;
        if failure.is_file() {
            tokio::fs::remove_file(failure)
                .await
                .map_err(|e| e.to_string())?;
        }
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

    #[test]
    fn internal_agent_files_are_protected() {
        assert!(is_internal_workspace_file(".fenix_chat.json"));
        assert!(is_internal_workspace_file("fenix_chat.json"));
        assert!(is_internal_workspace_file(".aura/programmer_failure.json"));
        assert!(is_internal_workspace_file("aura/programmer_failure.json"));
        assert!(is_internal_workspace_file(".git/config"));
        assert!(!is_internal_workspace_file("src/app.js"));
    }

    #[test]
    fn current_file_scope_discards_stale_rejected_targets() {
        let proposal = vec![
            Cambio {
                archivo: "verify_solution.py".into(),
                buscar: String::new(),
                reemplazar: "broken".into(),
            },
            Cambio {
                archivo: "index.html".into(),
                buscar: String::new(),
                reemplazar: "<main>Consulta</main>".into(),
            },
        ];
        let filtered = filter_changes_to_targets(proposal, &["./index.html".into()]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].archivo, "index.html");
        assert!(filter_changes_to_targets(vec![], &[]).is_empty());
    }

    #[test]
    fn duplicate_file_targets_are_collapsed_case_insensitively() {
        let files = deduplicate_targets(vec![
            "app.js".into(),
            "./APP.JS".into(),
            "tests/app.test.js".into(),
        ]);
        assert_eq!(files, vec!["app.js", "tests/app.test.js"]);
    }

    #[test]
    fn stale_repair_diagnostics_are_ignored_outside_the_requested_scope() {
        assert!(!repair_error_matches_scope(
            "[NODE_SCRIPTS_MISSING] package.json needs a test script",
            &["index.html".into(), "script.js".into()]
        ));
        assert!(repair_error_matches_scope(
            "[NODE_SCRIPTS_MISSING] package.json needs a test script",
            &["package.json".into()]
        ));
        assert!(repair_error_matches_scope("workspace-wide failure", &[]));
    }

    #[test]
    fn strict_audit_proposal_identifies_an_omitted_file() {
        let requested = vec!["tests/app.test.js".into()];
        let proposed = vec![Cambio {
            archivo: "app.js".into(),
            buscar: String::new(),
            reemplazar: "export {};".into(),
        }];
        assert_eq!(
            missing_proposed_targets(&requested, &proposed),
            vec!["tests/app.test.js"]
        );
    }

    #[test]
    fn consultation_test_alias_is_mapped_only_when_nested_target_was_requested() {
        let requested = vec!["tests/app.test.js".into()];
        let mut changes = vec![Cambio {
            archivo: "app.test.js".into(),
            buscar: String::new(),
            reemplazar: "import { test } from 'node:test';".into(),
        }];
        map_known_target_aliases(&mut changes, &requested);
        assert_eq!(changes[0].archivo, "tests/app.test.js");

        let mut unrelated = vec![Cambio {
            archivo: "app.test.js".into(),
            buscar: String::new(),
            reemplazar: "export {};".into(),
        }];
        map_known_target_aliases(&mut unrelated, &["app.js".into()]);
        assert_eq!(unrelated[0].archivo, "app.test.js");
    }

    #[tokio::test]
    async fn duplicate_proposal_diagnostic_keeps_all_drafts_for_the_next_repair() {
        let root =
            std::env::temp_dir().join(format!("aura-duplicate-draft-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let drafts = vec![
            Cambio {
                archivo: "app.js".into(),
                buscar: "first".into(),
                reemplazar: "part one".into(),
            },
            Cambio {
                archivo: "app.js".into(),
                buscar: "missing".into(),
                reemplazar: "part two".into(),
            },
        ];
        let error = "DUPLICATE_TARGET_CONFLICT: app.js tiene cambios repetidos que no se pueden combinar de forma segura.";
        persist_rejected_proposal(root.to_str().unwrap(), error, &drafts)
            .await
            .unwrap();

        let record: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join(".aura/programmer_failure.json")).unwrap(),
        )
        .unwrap();
        assert!(record["error"]
            .as_str()
            .unwrap()
            .contains("DUPLICATE_TARGET_CONFLICT"));
        assert_eq!(record["cambios"].as_array().unwrap().len(), 2);
        let restored: Vec<Cambio> = serde_json::from_value(record["cambios"].clone()).unwrap();
        assert_eq!(
            matching_rejected_changes(
                &restored,
                record["error"].as_str().unwrap(),
                &["app.js".into()]
            )
            .len(),
            2
        );
        assert!(!requires_incremental_repair(
            false,
            true,
            record["error"].as_str().unwrap()
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resume_reuses_only_the_rejected_draft_for_the_current_target() {
        let rejected = vec![Cambio {
            archivo: "app.js".into(),
            buscar: String::new(),
            reemplazar: "draft".into(),
        }];
        let error = "[ANTI-STUB ENFORCER] Código RECHAZADO en 'app.js'.";
        assert_eq!(
            matching_rejected_changes(&rejected, error, &["app.js".into()]).len(),
            1
        );
        assert!(matching_rejected_changes(&rejected, error, &["index.html".into()]).is_empty());
    }

    #[test]
    fn repair_mode_preserves_incremental_repairs_and_uses_clean_rewrites_for_conflicts() {
        assert!(requires_incremental_repair(
            false,
            true,
            "[ANTI-STUB ENFORCER] placeholder"
        ));
        assert!(!requires_incremental_repair(
            false,
            false,
            "[ANTI-STUB ENFORCER] stale file"
        ));
        assert!(requires_incremental_repair(
            true,
            false,
            "semantic verifier failed"
        ));
        assert!(requires_incremental_repair(
            false,
            true,
            "[NODE_SCRIPTS_MISSING] package.json"
        ));
        assert!(!requires_incremental_repair(
            false,
            true,
            "[ANTI-STUB ENFORCER] la función 'testAccounting' está declarada varias veces"
        ));
        assert!(!requires_incremental_repair(
            false,
            true,
            "[VALIDATION_FAILED] [NODE_SYNTAX_ERROR] Identifier has already been declared"
        ));
        assert!(!requires_incremental_repair(
            false,
            true,
            "DUPLICATE_TARGET_CONFLICT: no se pudo combinar app.js"
        ));
        assert!(!requires_incremental_repair(
            true,
            true,
            "[VALIDATION_FAILED] [PYTHON_SYNTAX_ERROR] verify_index.py SyntaxError"
        ));
        assert!(!requires_incremental_repair(
            true,
            true,
            "[VALIDATION_FAILED] [NODE_SYNTAX_ERROR] app.js unexpected token"
        ));
        assert!(!requires_incremental_repair(
            false,
            true,
            "[VALIDATION_FAILED] [JSON_SYNTAX_ERROR] firebase.json invalid string"
        ));
        assert!(!requires_incremental_repair(
            false,
            true,
            "[FIREBASE_CONFIG_INVALID] firebase.json embeds Firestore rules"
        ));
    }

    #[test]
    fn patch_not_found_forces_full_file_rewrite_even_during_semantic_repair() {
        assert!(requires_full_file_rewrite(
            "PATCH_NOT_FOUND: tests/app.test.js"
        ));
        assert!(!requires_incremental_repair(
            true,
            true,
            "PATCH_NOT_FOUND: tests/app.test.js"
        ));
        assert!(requires_full_file_rewrite(
            "FULL_REWRITE_REQUIRED: tests/app.test.js"
        ));
        assert!(requires_incremental_repair(
            true,
            true,
            "[VALIDATION_FAILED] [SEMANTIC] another verifier failure"
        ));
    }

    #[test]
    fn invalid_model_json_gets_a_bounded_repair_diagnostic() {
        let raw = "repetitive model output".repeat(10_000);
        let diagnostic = invalid_programmer_output_diagnostic(&raw, &["index.html".into()]);
        assert!(diagnostic.contains("PROGRAMMER_OUTPUT_INVALID_JSON"));
        assert!(diagnostic.contains("230000 caracteres"));
        assert!(diagnostic.len() < 500);
        assert!(!diagnostic.contains("repetitive model output"));
        assert!(requires_full_file_rewrite(&diagnostic));
        assert!(!requires_incremental_repair(true, false, &diagnostic));
    }

    #[test]
    fn duplicate_full_file_proposals_collapse_to_one_complete_change() {
        let root = std::env::temp_dir().join(format!("aura-duplicate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let changes = vec![
            Cambio {
                archivo: "app.js".into(),
                buscar: String::new(),
                reemplazar: "const login = () => {};".into(),
            },
            Cambio {
                archivo: "APP.JS".into(),
                buscar: String::new(),
                reemplazar: "const login = (email, password) => Boolean(email && password);".into(),
            },
        ];
        let merged = coalesce_duplicate_changes(root.to_str().unwrap(), changes, &[]).unwrap();
        assert_eq!(merged.len(), 1);
        assert!(merged[0].reemplazar.contains("password"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn anti_stub_rejection_preserves_current_draft_for_repair() {
        let root = std::env::temp_dir().join(format!("aura-stub-draft-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let result = ProgrammerExecutor::apply_and_validate(
            root.to_str().unwrap(),
            vec![Cambio {
                archivo: "app.js".into(),
                buscar: String::new(),
                reemplazar: "function login() {\n  // Aquí iría la lógica para iniciar sesión\n}\n"
                    .into(),
            }],
        )
        .await
        .unwrap();
        assert_ne!(result.exit_code, 0);
        let failure: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join(".aura/programmer_failure.json")).unwrap(),
        )
        .unwrap();
        assert!(failure["error"].as_str().unwrap().contains("ANTI-STUB"));
        assert!(failure["cambios"][0]["reemplazar"]
            .as_str()
            .unwrap()
            .contains("login"));
        assert!(!root.join("app.js").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn programmer_cannot_overwrite_agent_chat_history() {
        let root = std::env::temp_dir().join(format!("aura-protected-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let original = r#"{"messages":[]}"#;
        std::fs::write(root.join(".fenix_chat.json"), original).unwrap();
        let result = ProgrammerExecutor::apply_and_validate(
            root.to_str().unwrap(),
            vec![Cambio {
                archivo: ".fenix_chat.json".into(),
                buscar: String::new(),
                reemplazar: "{}".into(),
            }],
        )
        .await
        .unwrap();
        assert_ne!(result.exit_code, 0);
        assert!(result.stderr.contains("INTERNAL_FILE_PROTECTED"));
        assert_eq!(
            std::fs::read_to_string(root.join(".fenix_chat.json")).unwrap(),
            original
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn html_phase_ignores_unrelated_broken_python_and_future_assets() {
        let root =
            std::env::temp_dir().join(format!("aura-scoped-validation-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("verify_solution.py"), r#"text = "unterminated"#).unwrap();
        let html = r#"<!doctype html>
<html lang="es">
  <head><meta charset="utf-8"><title>Consulta</title><link rel="stylesheet" href="styles.css"></head>
  <body><main><h1>Consulta de clientes</h1><p>Busca y revisa los registros de tu negocio.</p></main><script src="app.js"></script></body>
</html>
"#;
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
        assert!(root.join("index.html").is_file());
        assert!(!root.join("styles.css").exists());
        assert!(!root.join("app.js").exists());
        assert!(std::fs::read_to_string(root.join("verify_solution.py"))
            .unwrap()
            .contains("unterminated"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn failed_generation_preserves_user_files_and_draft_for_repair() {
        let root = std::env::temp_dir().join(format!("aura-transaction-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("user-notes.txt"), "uncommitted user notes").unwrap();
        std::fs::write(root.join("existing.py"), "value = 1\n").unwrap();
        let result = ProgrammerExecutor::apply_and_validate(
            root.to_str().unwrap(),
            vec![
                Cambio {
                    archivo: "existing.py".into(),
                    buscar: "".into(),
                    reemplazar: "value = 2\n".into(),
                },
                Cambio {
                    archivo: "verify_dashboard.py".into(),
                    buscar: "".into(),
                    reemplazar: "text = \"unterminated\n".into(),
                },
            ],
        )
        .await
        .unwrap();
        assert_ne!(result.exit_code, 0);
        assert_eq!(
            std::fs::read_to_string(root.join("existing.py")).unwrap(),
            "value = 1\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("user-notes.txt")).unwrap(),
            "uncommitted user notes"
        );
        assert!(!root.join("verify_dashboard.py").exists());
        assert!(root.join(".aura/programmer_failure.json").exists());
        assert!(
            result.files_affected.is_empty(),
            "Rolled-back changes cannot be reported as physical progress"
        );
        let repaired = ProgrammerExecutor::apply_and_validate(root.to_str().unwrap(), vec![Cambio {
            archivo: "verify_dashboard.py".into(), buscar: "".into(), reemplazar: "import json\nif __name__ == '__main__':\n    passed = 1\n    total = 1\n    percentage = 100 * passed / total\n    failed_criteria = []\n    print(json.dumps({'passed': passed, 'total': total, 'percentage': percentage, 'failed_criteria': failed_criteria}))\n    exit(0 if passed == total else 1)\n".into(),
        }]).await.unwrap();
        assert_eq!(repaired.exit_code, 0, "{}", repaired.stderr);
        assert!(std::fs::read_to_string(root.join("verify_dashboard.py"))
            .unwrap()
            .contains("json.dumps"));
        assert!(
            !root.join(".git").exists(),
            "The programmer must not initialize or modify Git"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn missing_patch_does_not_report_success_or_modify_file() {
        let root = std::env::temp_dir().join(format!("aura-patch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("file.txt"), "original").unwrap();
        let result = ProgrammerExecutor::apply_and_validate(
            root.to_str().unwrap(),
            vec![Cambio {
                archivo: "file.txt".into(),
                buscar: "missing".into(),
                reemplazar: "replacement".into(),
            }],
        )
        .await
        .unwrap();
        assert_ne!(result.exit_code, 0);
        assert_eq!(
            std::fs::read_to_string(root.join("file.txt")).unwrap(),
            "original"
        );
        let failure: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join(".aura/programmer_failure.json")).unwrap(),
        )
        .unwrap();
        assert!(failure["error"]
            .as_str()
            .unwrap()
            .contains("PATCH_NOT_FOUND"));
        assert_eq!(failure["cambios"][0]["archivo"], "file.txt");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn html_encoded_javascript_operators_are_healed_before_validation() {
        let root = std::env::temp_dir().join(format!("aura-html-entity-{}", uuid::Uuid::new_v4()));
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
