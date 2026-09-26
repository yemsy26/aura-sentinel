use super::{call_ollama_with_schema_options, delegate_to_auditor, delegate_to_logic_solver};
use crate::core::{
    background_task_command,
    command_trail::StepResult, // CommandTrail used inline via full path in the trail block
    format_system_error,
    kill_task,
    read_task_logs,
    runner_generator::generate_standard_runners,
    start_background_task,
    validate_workspace,
};
use crate::memory;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

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

fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn normalized_visual_text(text: &str) -> String {
    text.to_lowercase()
        .replace('á', "a")
        .replace('é', "e")
        .replace('í', "i")
        .replace('ó', "o")
        .replace('ú', "u")
        .replace('ü', "u")
        .replace('ñ', "n")
}

fn phase_has_visual_deliverable(files: &[String]) -> bool {
    files.iter().any(|file| {
        matches!(
            std::path::Path::new(file)
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("html" | "htm" | "jsx" | "tsx" | "vue" | "svelte")
        )
    })
}

fn mission_requires_visual_verification(prompt: &str, _workspace: &str) -> bool {
    let normalized = normalized_visual_text(prompt);
    [
        "web app",
        "aplicacion web",
        "app web",
        "pagina web",
        "sitio web",
        "website",
        "frontend",
        "front-end",
        "interfaz",
        "dashboard",
        "interfaz grafica",
        "captura de la pagina",
        "revisa la interfaz",
        "valida la interfaz",
        "abrir en navegador",
        "index.html",
        "html/css",
        "construye una app",
        "construir una app",
        "aplicacion movil",
        "react",
        "vue",
        "angular",
        "vite",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn mission_requires_browser_interaction_verification(prompt: &str) -> bool {
    let normalized = normalized_visual_text(prompt);
    mission_requires_visual_verification(prompt, "")
        && [
            "probar en el navegador",
            "prueba en el navegador",
            "prueba en navegador",
            "probar los flujos",
            "comprobar los flujos",
            "prueba los flujos",
            "probar interacciones",
            "validar interacciones",
        ]
        .iter()
        .any(|marker| normalized.contains(marker))
}

fn same_local_server_origin(candidate: &str, confirmed: &str) -> bool {
    let Ok(candidate) = reqwest::Url::parse(candidate) else {
        return false;
    };
    let Ok(confirmed) = reqwest::Url::parse(confirmed) else {
        return false;
    };
    let is_local = |url: &reqwest::Url| {
        matches!(url.scheme(), "http" | "https")
            && url
                .host_str()
                .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"))
    };
    is_local(&candidate)
        && is_local(&confirmed)
        && candidate.scheme() == confirmed.scheme()
        && candidate.host_str() == confirmed.host_str()
        && candidate.port_or_known_default() == confirmed.port_or_known_default()
}

fn browser_interaction_test_guidance(url: &str, objective: &str) -> String {
    format!(
        "Ejecuta pruebas interactivas reales en el navegador mediante TOOL_TESTER. Usa el comando `BROWSER_TEST: {{\"url\":\"{url}\",\"steps\":[...]}}`; inspecciona index.html/script.js para obtener selectores reales. El runner admite click, fill, select, assert_visible, assert_hidden, assert_text, assert_value, assert_count, wait_for_visible, wait, reload, set_viewport y assert_no_console_errors. Diseña pasos que ejerciten cada flujo interactivo solicitado en el objetivo, valida mensajes y estado observable, comprueba persistencia tras reload y revisa consola en escritorio y móvil. No escribas pruebas DOM para ejecutar con Node, no instales dependencias ni pongas JavaScript arbitrario en el plan. Si el producto no implementa un flujo, registra la prueba que falla y corrige ese comportamiento antes de cerrar. Objetivo: {objective}"
    )
}

fn mission_requires_local_http_server(prompt: &str) -> bool {
    let normalized = normalized_visual_text(prompt);
    [
        "servidor http local",
        "servidor local",
        "local http server",
        "serve it over http",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn is_local_html_open_command(command: &str) -> bool {
    let lower = command.trim().to_ascii_lowercase();
    lower.starts_with("start ")
        && lower.contains(".html")
        && !lower.contains("http://")
        && !lower.contains("https://")
}

fn local_http_server_command_for_request(prompt: &str, command: &str) -> Option<&'static str> {
    (mission_requires_local_http_server(prompt)
        && (is_local_html_open_command(command)
            || command.trim().to_ascii_lowercase().starts_with("npx serve")))
    .then_some("python -m http.server 8000 --bind 127.0.0.1")
}

fn is_adaptive_text_model_candidate(model: &str) -> bool {
    let family = model
        .trim()
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    !family.contains("embed")
        && ![
            "moondream",
            "llava",
            "llava-phi3",
            "bakllava",
            "llama3.2-vision",
        ]
        .iter()
        .any(|vision_only| {
            family == *vision_only || family.starts_with(&format!("{}-", vision_only))
        })
}

fn adaptive_strategy_guidance(
    strategy: &crate::core::learning::strategy::StrategyKind,
) -> &'static str {
    use crate::core::learning::strategy::StrategyKind;
    match strategy {
        StrategyKind::DirectImplementation => {
            "Implementa el objetivo aprobado directamente y valida sus criterios concretos antes de cerrar."
        }
        StrategyKind::InspectThenImplement => {
            "Inspecciona primero los archivos y scripts reales del workspace; después implementa solo lo pendiente."
        }
        StrategyKind::TestFirst => {
            "Usa una prueba de comportamiento existente o un caso mínimo verificable; no crees pruebas vacías ni infraestructura que el proyecto no use."
        }
        StrategyKind::CompileFirst => {
            "Si ya existe un comando de compilación o sintaxis, ejecútalo una vez para conocer el estado inicial. Tras editar, valida el comportamiento solicitado: sintaxis correcta por sí sola no prueba la tarea."
        }
        StrategyKind::IncrementalPatch => {
            "Haz cambios pequeños en los archivos necesarios y valida el resultado después de cada grupo coherente; no repitas una escritura sin un diagnóstico nuevo."
        }
        StrategyKind::DiagnoseThenRepair => {
            "Antes de reparar, identifica el error actual y el archivo afectado; corrige ese defecto concreto y comprueba el resultado con evidencia nueva."
        }
        StrategyKind::MinimalChange => {
            "Limita la edición al menor conjunto de archivos que resuelve los criterios pendientes; conserva el trabajo válido y evita reescrituras amplias."
        }
    }
}

fn learned_strategy_has_reliable_evidence(
    confidence: f32,
    reason: &crate::core::learning::RecommendationReason,
    attempts: u64,
    failures: u64,
) -> bool {
    confidence >= 0.65
        && attempts >= 3
        && failures > 0
        && matches!(
            reason,
            crate::core::learning::RecommendationReason::GlobalHistory { .. }
                | crate::core::learning::RecommendationReason::SimilarTask { .. }
        )
}

fn is_node_syntax_check(command: &str) -> bool {
    let normalized = command.trim().to_ascii_lowercase();
    normalized == "node --check" || normalized.starts_with("node --check ")
}

fn should_force_web_validation_after_stall(
    prompt: &str,
    command: &str,
    stall_recoveries: u8,
    confirmed_url: Option<&str>,
) -> bool {
    stall_recoveries > 0
        && mission_requires_local_http_server(prompt)
        && is_node_syntax_check(command)
        && confirmed_url.is_none()
}

fn should_inject_prior_episodes(is_resume: bool, is_contextual_follow_up: bool) -> bool {
    is_resume || is_contextual_follow_up
}

fn is_long_running_server_command(command: &str) -> bool {
    let lower = command.trim().to_ascii_lowercase();
    lower.contains("http-server")
        || lower.contains("npm start")
        || lower.contains("npm run dev")
        || lower.contains("python -m http.server")
        || lower.contains("flask run")
        || lower.contains("uvicorn")
        || lower.contains("live-server")
        || lower.starts_with("serve ")
        || lower == "serve"
}

fn local_http_server_recovery_action(
    prompt: &str,
    workspace: &str,
    server_task_id: Option<&str>,
) -> Option<(String, String)> {
    if !mission_requires_local_http_server(prompt) {
        return None;
    }
    if let Some(task_id) = server_task_id {
        return Some(("TOOL_BACKGROUND_READ".into(), task_id.to_string()));
    }
    if std::path::Path::new(workspace).join("index.html").is_file() {
        return Some((
            "TOOL_BACKGROUND_START".into(),
            "python -m http.server 8000 --bind 127.0.0.1".into(),
        ));
    }
    Some((
        "TOOL_PROGRAMMER".into(),
        "El mandato exige una URL HTTP localhost. Primero implementa el index.html y los archivos web pendientes; después inicia el servidor con TOOL_BACKGROUND_START. No uses file:// como sustituto.".into(),
    ))
}

fn web_validation_recovery_action(
    prompt: &str,
    workspace: &str,
    confirmed_url: Option<&str>,
    server_task_id: Option<&str>,
) -> Option<(String, String)> {
    if let Some(url) = confirmed_url {
        return Some(
            if mission_requires_browser_interaction_verification(prompt) {
                (
                    "TOOL_TESTER".into(),
                    browser_interaction_test_guidance(url, prompt),
                )
            } else {
                ("TOOL_VISION_EVALUATOR".into(), url.to_string())
            },
        );
    }
    if let Some(action) = local_http_server_recovery_action(prompt, workspace, server_task_id) {
        return Some(action);
    }
    if mission_requires_browser_interaction_verification(prompt)
        && std::path::Path::new(workspace).join("index.html").is_file()
    {
        return Some((
            "TOOL_BACKGROUND_START".into(),
            "python -m http.server 8000 --bind 127.0.0.1".into(),
        ));
    }
    mission_requires_visual_verification(prompt, workspace)
        .then(|| ("TOOL_VISION_EVALUATOR".into(), String::new()))
}

fn web_validation_handoff_action(
    prompt: &str,
    command: &str,
    stall_recoveries: u8,
    confirmed_url: Option<&str>,
    workspace: &str,
    server_task_id: Option<&str>,
) -> Option<(String, String)> {
    if !should_force_web_validation_after_stall(prompt, command, stall_recoveries, confirmed_url) {
        return None;
    }
    local_http_server_recovery_action(prompt, workspace, server_task_id)
}

fn console_validation_profile(prompt: &str) -> Option<(&'static str, &'static str)> {
    let normalized = normalized_visual_text(prompt);
    let is_console_project = [
        "programa de consola",
        "aplicacion de consola",
        "consola interactiva",
        "terminal interactiva",
        "linea de comandos",
        "command line",
        "cli",
    ]
    .iter()
    .any(|marker| normalized.contains(marker));
    if !is_console_project {
        return None;
    }

    let detailed_markers = [
        "detallado",
        "completo",
        "casos limite",
        "edge case",
        "todos los flujos",
        "cada modulo",
        "multiples",
        "varios casos",
        "autenticacion",
        "permisos",
        "persistencia",
        "facturacion",
        "contabilidad",
        "reportes",
        "registro",
        "iniciar sesion",
        "crud",
        "validaciones",
    ];
    let requested_features = [
        "modulo",
        "flujo",
        "funcionalidad",
        "caracteristica",
        "entrada",
        "salida",
        "guardar",
        "buscar",
        "editar",
        "eliminar",
        "crear",
        "calcular",
        "exportar",
        "importar",
    ];
    let detailed = detailed_markers
        .iter()
        .any(|marker| normalized.contains(marker))
        || requested_features
            .iter()
            .filter(|marker| normalized.contains(**marker))
            .count()
            >= 3;

    if detailed {
        Some((
            "DETALLADA",
            "Ejecuta pruebas representativas para cada flujo, entrada inválida y caso límite explícito; registra entrada, stdout/stderr, código de salida y efectos. No consideres aprobado lo que solo compila.",
        ))
    } else {
        Some((
            "BÁSICA",
            "Ejecuta el caso principal con una entrada real; comprueba la salida observable y el código de salida. No afirmes funcionalidades que no ejecutaste.",
        ))
    }
}

fn adaptive_failure_details(reason: &str) -> (&'static str, Option<&'static str>) {
    if reason.contains("NO_PROGRESS_EXHAUSTED") {
        ("NoProgress", Some("TOOL_PROGRAMMER"))
    } else if reason.contains("BUDGET_EXHAUSTED") {
        ("BudgetExhausted", None)
    } else if reason.contains("LOCAL_SERVER_NOT_CONFIRMED") {
        ("LocalServer", Some("TOOL_BACKGROUND_READ"))
    } else if reason.contains("VERIFIER_REPAIR_EXHAUSTED")
        || reason.contains("VERIFIER_NO_PROGRESS")
    {
        ("Verification", Some("TOOL_TESTER"))
    } else if reason.contains("FIREBASE_INTEGRATION_REPAIR_EXHAUSTED") {
        ("FirebaseIntegration", Some("TOOL_PROGRAMMER"))
    } else if reason.contains("PROGRAMMER_REPAIR_EXHAUSTED")
        || reason.contains("FORCED_PROGRAMMER_FAILED")
        || reason.contains("FORCED_PROGRAMMER_BLOCKED")
    {
        ("ProgrammerRepair", Some("TOOL_PROGRAMMER"))
    } else if reason.contains("TOOL_UNREGISTERED") {
        ("ToolRegistry", None)
    } else if reason.contains("POLICY_DENY") {
        ("Policy", None)
    } else {
        ("MissionFailure", None)
    }
}

fn is_scoped_visual_review(prompt: &str) -> bool {
    crate::core::intent_router::is_scoped_visual_review(prompt)
}

fn resolve_scoped_visual_review_instruction(
    current_instruction: &str,
    saved_objective: &str,
    is_continuation: bool,
) -> Option<String> {
    if is_scoped_visual_review(current_instruction) {
        Some(current_instruction.to_string())
    } else if is_continuation && is_scoped_visual_review(saved_objective) {
        Some(saved_objective.to_string())
    } else {
        None
    }
}

fn scoped_review_forbids_action(tool: &str, command: &str) -> bool {
    if matches!(
        tool,
        "TOOL_PROGRAMMER"
            | "TOOL_ENV_MANAGER"
            | "TOOL_ASSET_MANAGER"
            | "TOOL_AST_INJECT"
            | "TOOL_CREATE_RUNNER"
            | "TOOL_WORKSPACE_MANAGER"
            | "TOOL_GIT"
            | "TOOL_CONTAINER"
            | "TOOL_SCHEDULER"
            | "TOOL_LEARN"
    ) {
        return true;
    }
    if tool != "TOOL_TERMINAL" {
        return false;
    }

    let command = command.trim().to_ascii_lowercase();
    [
        "del ",
        "erase ",
        "rd /s",
        "rmdir ",
        "remove-item",
        "rm ",
        "mv ",
        "move ",
        "copy ",
        "cp ",
        "ren ",
        "npm install",
        "npm ci",
        "pnpm install",
        "yarn install",
        "cargo clean",
        "git clean",
        "git reset",
        "git checkout",
        "set-content",
        "add-content",
    ]
    .iter()
    .any(|prefix| command.starts_with(prefix))
        || command.contains(" >")
        || command.contains("&& del ")
        || command.contains("&& rm ")
}

fn action_response_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "herramienta": {
                "type": "string",
                "enum": crate::core::tool_registry::KNOWN_TOOLS
            },
            "pensamiento": { "type": "string" },
            "comando": { "type": ["string", "null"] },
            "task_id": { "type": ["string", "null"] },
            "url_a_investigar": { "type": ["string", "null"] },
            "archivos_a_editar": { "type": "array", "items": { "type": "string" } },
            "ast_nodes": { "type": "array", "items": { "type": "object" } },
            "respuesta_conversacional": { "type": ["string", "null"] }
        },
        "required": [
            "herramienta", "pensamiento", "comando", "task_id", "url_a_investigar",
            "archivos_a_editar", "ast_nodes", "respuesta_conversacional"
        ],
        "additionalProperties": true
    })
}

fn is_console_runtime_command(command: &str) -> bool {
    let normalized = crate::core::evidence::normalize_command_str(command);
    let command = normalized
        .strip_prefix("cmd /c ")
        .or_else(|| normalized.strip_prefix("cmd.exe /c "))
        .unwrap_or(&normalized)
        .trim();

    if [
        "cargo test",
        "npm test",
        "npm run test",
        "pytest",
        "python -m pytest",
        "python -m unittest",
        "go test",
        "dotnet test",
        "mvn test",
    ]
    .iter()
    .any(|prefix| command == *prefix || command.starts_with(&format!("{prefix} ")))
    {
        return false;
    }

    [
        "python ",
        "python3 ",
        "py ",
        "node ",
        "npm start",
        "npm run start",
        "cargo run",
        "dotnet run",
        "go run",
        "java ",
        "ruby ",
        "php ",
        "perl ",
        "bash ",
        "sh ",
        "./",
        "target/debug/",
    ]
    .iter()
    .any(|prefix| command.starts_with(prefix))
        || command
            .split_whitespace()
            .next()
            .is_some_and(|first| first.ends_with(".exe") && !first.contains("/"))
}

fn has_current_console_runtime_evidence(
    evidence: &crate::core::evidence::EvidenceGraph,
    current_hash: u64,
    workspace: &str,
) -> bool {
    let expected_workspace = std::path::Path::new(workspace)
        .canonicalize()
        .map(|path| path.to_string_lossy().to_lowercase());

    evidence.entries.iter().any(|entry| {
        if !matches!(
            entry.kind,
            crate::core::evidence::EvidenceKind::CommandExitCode
                | crate::core::evidence::EvidenceKind::RuntimeCheck
        ) || entry.reliability < 0.5
            || entry.state_hash != Some(current_hash)
        {
            return false;
        }

        match &entry.fact {
            crate::core::evidence::StructuredFact::CommandResult {
                command,
                cwd,
                exit_code,
                ..
            } => {
                if *exit_code != 0 || !is_console_runtime_command(command) {
                    return false;
                }
                let actual_workspace = std::path::Path::new(cwd)
                    .canonicalize()
                    .map(|path| path.to_string_lossy().to_lowercase());
                match (&expected_workspace, &actual_workspace) {
                    (Ok(expected), Ok(actual)) => expected == actual,
                    _ => cwd
                        .trim_end_matches(['\\', '/'])
                        .eq_ignore_ascii_case(workspace.trim_end_matches(['\\', '/'])),
                }
            }
            _ => false,
        }
    })
}

fn extract_local_ui_url(text: &str) -> Option<String> {
    let pattern = regex::Regex::new(
        r#"(?i)https?://(?:localhost|127\.0\.0\.1|\[::1\])(?::\d+)?(?:/[^\s"'<>]*)?"#,
    )
    .ok()?;
    pattern.find(text).map(|matched| {
        matched
            .as_str()
            .trim_end_matches(|character: char| {
                matches!(character, '.' | ',' | ';' | ')' | ']' | '}' | '`')
            })
            .to_string()
    })
}

fn local_http_url_from_server_command(command: &str) -> Option<String> {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    let module_index = tokens
        .iter()
        .position(|token| token.eq_ignore_ascii_case("http.server"))?;
    if !tokens.iter().any(|token| token.eq_ignore_ascii_case("-m")) {
        return None;
    }

    let mut port = 8000u16;
    let mut host = "127.0.0.1";
    let mut index = module_index + 1;
    while index < tokens.len() {
        let token = tokens[index];
        if token == "--bind" {
            host = *tokens.get(index + 1)?;
            index += 2;
            continue;
        }
        if let Some(value) = token.strip_prefix("--bind=") {
            host = value;
        } else if !token.starts_with('-') {
            if let Ok(value) = token.parse::<u16>() {
                if value == 0 {
                    return None;
                }
                port = value;
            }
        }
        index += 1;
    }

    let local_host = match host.trim_matches(['\'', '"']) {
        "localhost" | "127.0.0.1" | "0.0.0.0" => "127.0.0.1",
        "::1" | "[::1]" => "[::1]",
        _ => return None,
    };
    Some(format!("http://{local_host}:{port}/"))
}

async fn probe_background_local_http_server(task_id: &str) -> Option<String> {
    let command = background_task_command(task_id).await?;
    let url = local_http_url_from_server_command(&command)?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(1))
        .build()
        .ok()?;

    // Process startup and log-reader scheduling are asynchronous. Probe the
    // exact local origin implied by the launched command instead of treating an
    // empty log buffer as proof that the server failed.
    for _ in 0..6 {
        if client
            .get(&url)
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return Some(url);
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    None
}

fn extract_explicit_http_url(text: &str) -> Option<String> {
    let pattern = regex::Regex::new(r#"(?i)https?://[^\s"'<>]+\b"#).ok()?;
    pattern.find(text).map(|matched| {
        matched
            .as_str()
            .trim_end_matches(|character: char| {
                matches!(character, '.' | ',' | ';' | ')' | ']' | '}' | '`')
            })
            .to_string()
    })
}

fn save_resumable_validation_block(
    journal: &mut crate::core::session_journal::SessionJournal,
    workspace: &str,
    step: u32,
    role: &str,
    context: &str,
    reason: &str,
) -> Result<(), String> {
    journal.interrupted = true;
    journal.status = "EN_PROGRESO".to_string();
    journal.ultimo_paso = step;
    journal.ultimo_estado = reason.to_string();
    journal.fsm_role = Some(role.to_string());
    journal.fsm_step = step;
    journal.fsm_context = Some(context.to_string());
    crate::core::session_journal::save_journal(workspace, journal)
}

fn visual_capture_from_error(workspace: &str, error: &str) -> Option<std::path::PathBuf> {
    let relative = error
        .split_once("Captura:")
        .map(|(_, tail)| tail.lines().next().unwrap_or_default().trim())
        .or_else(|| {
            error
                .split_once("captura se guardó en ")
                .map(|(_, tail)| tail.split([';', '\n']).next().unwrap_or_default().trim())
        })?;
    let relative = relative
        .trim_matches(|character| matches!(character, ' ' | '\t' | '\r' | '"' | '\'' | '`'));
    let directory = std::path::Path::new(workspace)
        .join(".aura")
        .join("evidence")
        .join("visual");
    let path = std::path::Path::new(workspace).join(relative);
    let canonical_path = path.canonicalize().ok()?;
    let canonical_directory = directory.canonicalize().ok()?;
    (canonical_path.starts_with(canonical_directory)
        && canonical_path.extension().and_then(|value| value.to_str()) == Some("png"))
    .then_some(canonical_path)
}

fn vision_result_needs_manual_review(error: &str) -> bool {
    [
        "VISUAL_QA_UNAVAILABLE",
        "VISUAL_QA_UNCERTAIN",
        "VISUAL_QA_INVALID_RESPONSE",
    ]
    .iter()
    .any(|marker| error.contains(marker))
}

fn is_explicit_approval(answer: &str) -> bool {
    let normalized = answer
        .trim()
        .to_lowercase()
        .replace('á', "a")
        .replace('é', "e")
        .replace('í', "i")
        .replace('ó', "o")
        .replace('ú', "u")
        .replace('ñ', "n");
    let words = normalized
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    matches!(
        words.as_str(),
        "si" | "yes"
            | "confirmo"
            | "confirmar"
            | "autorizo"
            | "autorizar"
            | "apruebo"
            | "aprobado"
            | "approve"
            | "si autorizo"
            | "si autorizo esta accion"
            | "si confirmo"
            | "si confirmo esta accion"
            | "autorizo esta accion"
            | "autorizar esta accion"
            | "confirmo esta accion"
            | "confirmar esta accion"
            | "apruebo esta accion"
            | "yes approve"
            | "yes i approve"
            | "i approve"
            | "yes authorize"
            | "yes i authorize"
    )
}

fn recent_context(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    text.chars()
        .skip(char_count.saturating_sub(max_chars))
        .collect()
}

fn parse_sat_clauses(value: &serde_json::Value) -> Result<Vec<Vec<i32>>, String> {
    let clauses = value
        .as_array()
        .ok_or_else(|| "'clauses' debe ser una matriz de cláusulas".to_string())?;
    clauses
        .iter()
        .enumerate()
        .map(|(clause_index, clause)| {
            clause
                .as_array()
                .ok_or_else(|| format!("La cláusula {} no es una matriz", clause_index + 1))?
                .iter()
                .enumerate()
                .map(|(literal_index, literal)| {
                    let number = literal.as_i64().ok_or_else(|| {
                        format!(
                            "El literal {} de la cláusula {} no es un entero",
                            literal_index + 1,
                            clause_index + 1
                        )
                    })?;
                    i32::try_from(number).map_err(|_| {
                        format!(
                            "El literal {} de la cláusula {} excede el rango admitido",
                            literal_index + 1,
                            clause_index + 1
                        )
                    })
                })
                .collect()
        })
        .collect()
}

fn parse_sat_payload(
    payload: &serde_json::Value,
) -> Result<Option<(usize, Vec<Vec<i32>>)>, String> {
    let has_n_vars = payload.get("n_vars").is_some();
    let has_clauses = payload.get("clauses").is_some();
    if !has_n_vars && !has_clauses {
        return Ok(None);
    }
    if !has_n_vars || !has_clauses {
        return Err("TOOL_LOGIC_SOLVER requiere 'n_vars' y 'clauses' juntos".into());
    }
    let n_vars = payload
        .get("n_vars")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| "'n_vars' debe ser un entero no negativo".to_string())?;
    let clauses = parse_sat_clauses(&payload["clauses"])?;
    Ok(Some((n_vars, clauses)))
}

fn extract_sat_payload(message: &str) -> Result<Option<(usize, Vec<Vec<i32>>)>, String> {
    let Some(start) = message.find("[[").or_else(|| message.find("[ [")) else {
        return Ok(None);
    };
    let text = &message[start..];
    let mut depth = 0i32;
    let mut end = None;
    for (index, character) in text.char_indices() {
        match character {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth < 0 {
                    return Err("La matriz SAT tiene corchetes desbalanceados".into());
                }
                if depth == 0 {
                    end = Some(index + character.len_utf8());
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end.ok_or_else(|| "La matriz SAT está incompleta".to_string())?;
    let value: serde_json::Value = serde_json::from_str(&text[..end])
        .map_err(|error| format!("La matriz SAT no es JSON válido: {error}"))?;
    let clauses = parse_sat_clauses(&value)?;
    let n_vars = clauses
        .iter()
        .flatten()
        .map(|literal| literal.unsigned_abs() as usize)
        .max()
        .unwrap_or(0);
    Ok(Some((n_vars, clauses)))
}

fn sanitize_runtime_context(context: &str, workspace_path: &str) -> String {
    let stale_markers = ["proxy-stack-windows", "proxy-stack", "\\proxy-", "/proxy-"];
    let ws_clean = workspace_path.trim().replace('/', "\\");
    context
        .lines()
        .filter(|line| !stale_markers.iter().any(|marker| line.contains(marker)))
        .filter(|line| {
            if let Some(pos) = line.find("scratch\\") {
                let after = &line[pos + 8..];
                let foreign_folder = after
                    .split(&['\\', '/', ' ', '"', '\'', '`'][..])
                    .next()
                    .unwrap_or("");
                return foreign_folder.is_empty()
                    || ws_clean.contains(foreign_folder)
                    || foreign_folder == "aura sentinel";
            }
            true
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn official_verifier_command(
    contract: &crate::core::mission_contract::MissionContract,
) -> Option<String> {
    contract.acceptance_criteria.iter().find_map(|criterion| {
        if let crate::core::mission_contract::VerificationMethod::SemanticVerification { command } =
            &criterion.verification
        {
            Some(command.clone())
        } else {
            None
        }
    })
}

fn is_managed_verifier(workspace_path: &str, file: &str) -> bool {
    if !(file.starts_with("verify_") || file.starts_with("test_")) {
        return false;
    }
    std::fs::read_to_string(std::path::Path::new(workspace_path).join(file))
        .map(|source| source.starts_with("# AURA_MANAGED_VERIFIER_"))
        .unwrap_or(false)
}

fn programmer_file_fallback(
    journal: &crate::core::session_journal::SessionJournal,
    workspace_path: &str,
    explicit_files: &[String],
) -> Vec<String> {
    phase_pending_programmer_file(journal, workspace_path)
        .or_else(|| {
            explicit_files.iter().find_map(|file| {
                (!crate::core::programmer_executor::is_internal_workspace_file(file)
                    && !std::path::Path::new(workspace_path).join(file).is_file())
                .then(|| file.clone())
            })
        })
        .into_iter()
        .collect()
}

fn phase_pending_programmer_file(
    journal: &crate::core::session_journal::SessionJournal,
    workspace_path: &str,
) -> Option<String> {
    journal
        .fases
        .get(journal.fase_actual)?
        .archivos
        .iter()
        .find(|file| {
            !crate::core::programmer_executor::is_internal_workspace_file(file)
                && !std::path::Path::new(workspace_path).join(file).is_file()
        })
        .cloned()
}

fn schema_recovery_programmer_target(
    journal: &crate::core::session_journal::SessionJournal,
    workspace_path: &str,
    explicit_files: &[String],
) -> Option<String> {
    phase_pending_programmer_file(journal, workspace_path)
        .or_else(|| {
            journal
                .fases
                .get(journal.fase_actual)?
                .archivos
                .iter()
                .find_map(|file| {
                    (!crate::core::programmer_executor::is_internal_workspace_file(file)
                        && std::path::Path::new(workspace_path).join(file).is_file())
                    .then(|| file.clone())
                })
        })
        .or_else(|| {
            explicit_files.iter().find_map(|file| {
                (!crate::core::programmer_executor::is_internal_workspace_file(file))
                    .then(|| file.clone())
            })
        })
}

fn needs_approved_plan_recovery(
    approved_consultation_task: bool,
    journal: &crate::core::session_journal::SessionJournal,
) -> bool {
    approved_consultation_task
        && journal.plan_generado
        && !journal
            .fases
            .iter()
            .all(|phase| phase.estado == "COMPLETADA")
        && (journal
            .fases
            .first()
            .map(|phase| {
                !phase
                    .archivos
                    .iter()
                    .any(|file| file.eq_ignore_ascii_case("index.html"))
                    || phase.criterio_de_exito != "npm test"
            })
            .unwrap_or(true)
            || journal
                .fases
                .get(1)
                .map(|phase| phase.criterio_de_exito != "npm run test:firebase")
                .unwrap_or(true)
            || journal
                .fases
                .get(2)
                .map(|phase| phase.criterio_de_exito != "firebase deploy --only hosting")
                .unwrap_or(true))
}

fn needs_local_consultation_agenda_plan_recovery(
    local_consultation_agenda_task: bool,
    journal: &crate::core::session_journal::SessionJournal,
) -> bool {
    local_consultation_agenda_task
        && journal.plan_generado
        && !journal
            .fases
            .iter()
            .all(|phase| phase.estado == "COMPLETADA")
        && !crate::llm::phase_planner::local_consultation_agenda_plan_is_complete(&journal.fases)
}

fn recover_programmer_file_args(
    payload: &mut serde_json::Value,
    journal: &crate::core::session_journal::SessionJournal,
    workspace_path: &str,
    explicit_files: &[String],
) -> Option<Vec<String>> {
    let original_count = payload
        .get("archivos_a_editar")
        .and_then(|value| value.as_array())
        .map(Vec::len)
        .unwrap_or_default();
    let safe_requested: Vec<String> = payload
        .get("archivos_a_editar")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .filter(|file| {
            !file.is_empty() && !crate::core::programmer_executor::is_internal_workspace_file(file)
        })
        .map(str::to_string)
        .collect();

    let recovered = if safe_requested.is_empty() {
        programmer_file_fallback(journal, workspace_path, explicit_files)
    } else if safe_requested.len() != original_count {
        safe_requested.clone()
    } else {
        return None;
    };
    if recovered.is_empty() {
        return None;
    }
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "archivos_a_editar".into(),
            serde_json::json!(recovered.clone()),
        );
        Some(recovered)
    } else {
        None
    }
}

fn file_is_already_known(existing_files: &[String], requested: &str) -> bool {
    let requested = requested.replace('\\', "/");
    existing_files.iter().any(|existing| {
        let existing = existing.replace('\\', "/");
        existing == requested || existing.ends_with(&format!("/{requested}"))
    })
}

fn is_test_artifact_path(file: &str) -> bool {
    let normalized = file.replace('\\', "/").to_lowercase();
    let name = normalized.rsplit('/').next().unwrap_or(&normalized);
    name.starts_with("verify_")
        || name.starts_with("test_")
        || name.ends_with(".test.js")
        || name.ends_with(".spec.js")
        || name.ends_with(".test.ts")
        || name.ends_with(".spec.ts")
        || name.ends_with(".test.mjs")
        || name.ends_with(".spec.mjs")
        || name.ends_with(".test.cjs")
        || name.ends_with(".spec.cjs")
}

fn test_command_for_path(file: &str) -> Option<String> {
    let normalized = file.replace('\\', "/");
    let lower = normalized.to_lowercase();
    let quoted = format!("\"{}\"", normalized);
    if lower.ends_with(".py") {
        Some(format!("python {}", quoted))
    } else if [
        ".test.js",
        ".spec.js",
        ".test.mjs",
        ".spec.mjs",
        ".test.cjs",
        ".spec.cjs",
    ]
    .iter()
    .any(|suffix| lower.ends_with(suffix))
    {
        Some(format!("node --test {}", quoted))
    } else if [".js", ".mjs", ".cjs"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
    {
        Some(format!("node {}", quoted))
    } else {
        None
    }
}

fn node_script_argument(command: &str) -> Option<String> {
    let mut args = split_command_arguments(command);
    let executable = args.first()?.to_ascii_lowercase();
    if !matches!(executable.as_str(), "node" | "node.exe" | "nodejs") {
        return None;
    }
    args.remove(0);

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-e" | "--eval" | "-p" | "--print" => return None,
            "--require"
            | "-r"
            | "--import"
            | "--loader"
            | "--experimental-loader"
            | "--conditions"
            | "-C"
            | "--test-name-pattern"
            | "--test-reporter"
            | "--test-reporter-destination" => index += 2,
            "--" => return args.get(index + 1).cloned(),
            option if option.starts_with('-') => index += 1,
            target => return Some(target.to_string()),
        }
    }
    None
}

fn split_command_arguments(command: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for character in command.chars() {
        match quote {
            Some(delimiter) if character == delimiter => quote = None,
            Some(_) => current.push(character),
            None if matches!(character, '"' | '\'') => quote = Some(character),
            None if character.is_whitespace() => {
                if !current.is_empty() {
                    arguments.push(std::mem::take(&mut current));
                }
            }
            None => current.push(character),
        }
    }
    if !current.is_empty() {
        arguments.push(current);
    }
    arguments
}

fn workspace_file_path(workspace_path: &str, file: &str) -> Option<std::path::PathBuf> {
    let workspace = std::path::Path::new(workspace_path);
    let workspace_root = workspace.canonicalize().ok()?;
    let target = std::path::Path::new(file);
    let candidate = if target.is_absolute() {
        target.to_path_buf()
    } else {
        workspace_root.join(target)
    };
    let target_path = candidate.canonicalize().ok()?;
    (target_path.starts_with(&workspace_root) && target_path.is_file()).then_some(target_path)
}

fn test_file_has_runnable_cases(workspace_path: &str, file: &str) -> bool {
    let Some(target_path) = workspace_file_path(workspace_path, file) else {
        return false;
    };
    let Ok(source) = std::fs::read_to_string(&target_path) else {
        return false;
    };
    let has_test_case = match target_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("py") => source.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with("def test_") || line.starts_with("async def test_")
        }),
        Some("js" | "mjs" | "cjs") => ["test(", "test (", "it(", "it (", "specify(", "specify ("]
            .iter()
            .any(|marker| source.contains(marker)),
        _ => false,
    };
    if !has_test_case {
        return false;
    }

    let lower_source = source.to_ascii_lowercase();
    let uses_node_test = lower_source.contains("node:test");
    if uses_node_test {
        let node_test_unsupported_apis = [
            "tohavebeencalled",
            "mockreturnvalue",
            "mockimplementation",
            "jest.fn(",
            "vi.fn(",
        ];
        let imports_expect = lower_source.contains("from 'expect'")
            || lower_source.contains("from \"expect\"")
            || lower_source.contains("require('expect')")
            || lower_source.contains("require(\"expect\")")
            || lower_source.contains("function expect(")
            || lower_source.contains("const expect =")
            || lower_source.contains("let expect =");
        if node_test_unsupported_apis
            .iter()
            .any(|marker| lower_source.contains(marker))
            || (lower_source.contains("expect(") && !imports_expect)
        {
            return false;
        }
    }

    // A test declaration with an empty callback exits successfully under most
    // runners. It is not evidence that the implementation behaves correctly.
    match target_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("py") => source.lines().any(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("assert ") || trimmed.starts_with("self.assert")
        }),
        Some("js" | "mjs" | "cjs") => [
            "assert.",
            "assert(",
            "expect(",
            ".to.equal(",
            ".to.eql(",
            ".to.be.",
            ".to.have.",
            ".toBe(",
            ".toEqual(",
            ".toStrictEqual(",
            ".toContain(",
        ]
        .iter()
        .any(|marker| source.contains(marker)),
        _ => false,
    }
}

fn successful_test_run_has_behavioral_cases(
    command: &str,
    workspace_path: &str,
    output: &str,
) -> bool {
    let lower_command = command.trim().to_ascii_lowercase();
    if lower_command.starts_with("node --test") {
        return node_script_argument(command)
            .is_some_and(|file| test_file_has_runnable_cases(workspace_path, &file));
    }

    let lower_output = output.to_ascii_lowercase();
    if [
        "no tests found",
        "no test files found",
        "0 tests",
        "tests 0",
        "0 passed",
        "passed: 0",
        "[no test files]",
    ]
    .iter()
    .any(|marker| lower_output.contains(marker))
    {
        return false;
    }

    if lower_command.starts_with("cargo test") {
        return lower_output.contains("test result: ok")
            && lower_output.contains(" passed;")
            && !lower_output.contains("0 passed;");
    }
    if lower_command.starts_with("go test") {
        return lower_output.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with("ok\t") || line.starts_with("ok  ")
        });
    }
    if lower_command.starts_with("pytest") || lower_command.starts_with("python -m pytest") {
        return lower_output.lines().any(|line| {
            line.split_whitespace().any(|token| token == "passed")
                && !line.trim_start().starts_with("0 passed")
        });
    }
    false
}

fn programmer_loop_validation_command(
    existing_files: &[String],
    contract: &crate::core::mission_contract::MissionContract,
    workspace_path: &str,
) -> Option<String> {
    if let Some(command) = existing_files
        .iter()
        .filter(|file| {
            is_test_artifact_path(file) && test_file_has_runnable_cases(workspace_path, file)
        })
        .find_map(|file| test_command_for_path(file))
    {
        return Some(command);
    }
    if let Some(command) = official_verifier_command(contract) {
        return Some(command);
    }
    if let Some(file) = existing_files.iter().find(|file| {
        let lower = file.to_ascii_lowercase().replace('\\', "/");
        !is_test_artifact_path(file)
            && [".js", ".mjs", ".cjs"]
                .iter()
                .any(|extension| lower.ends_with(extension))
            && workspace_file_path(workspace_path, file).is_some()
    }) {
        return Some(format!("node --check \"{}\"", file.replace('\\', "/")));
    }
    if existing_files.iter().any(|file| {
        file.eq_ignore_ascii_case("Cargo.toml")
            && workspace_file_path(workspace_path, file).is_some()
    }) {
        return Some("cargo check".to_string());
    }
    None
}

const MAX_PROGRAMMER_STALL_RECOVERIES: u8 = 2;
const MAX_PROGRAMMER_CALLS_WITHOUT_VALIDATION: u8 = 3;

/// Return the script path for direct Python execution. Module and inline-code
/// invocations (`python -m ...`, `python -c ...`) are not workspace files.
fn python_script_argument(command: &str) -> Option<String> {
    let mut parts = split_command_arguments(command).into_iter();
    let executable = parts.next()?.to_lowercase();
    if !matches!(
        executable.as_str(),
        "python" | "python3" | "py" | "python.exe" | "python3.exe" | "py.exe"
    ) {
        return None;
    }

    while let Some(argument) = parts.next() {
        match argument.as_str() {
            "-m" | "-c" => return None,
            "--" => return parts.next(),
            "-W" | "-X" => {
                parts.next();
            }
            value if value.starts_with('-') => {}
            value => return Some(value.to_string()),
        }
    }
    None
}

fn relevant_programmer_files(
    workspace_path: &str,
    contract: &crate::core::mission_contract::MissionContract,
    existing_files: &[String],
    reason: &str,
) -> Vec<String> {
    let mut files: Vec<String> = contract
        .acceptance_criteria
        .iter()
        .filter_map(|criterion| {
            if let crate::core::mission_contract::VerificationMethod::FileExistence(file) =
                &criterion.verification
            {
                let existing_managed_verifier = (file.starts_with("verify_")
                    || file.starts_with("test_"))
                    && file_is_already_known(existing_files, file);
                if existing_managed_verifier {
                    None
                } else {
                    Some(file.clone())
                }
            } else {
                None
            }
        })
        .collect();
    let reason_lower = reason.to_lowercase();
    let is_repair = [
        "corrig",
        "repara",
        "fall",
        "error",
        "criterio",
        "estancamiento",
    ]
    .iter()
    .any(|token| reason_lower.contains(token));
    if is_repair || files.is_empty() {
        files.extend(existing_files.iter().filter_map(|file| {
            let path = std::path::Path::new(file);
            match path.extension().and_then(|value| value.to_str()) {
                Some("html" | "css" | "js" | "ts" | "tsx" | "jsx" | "py" | "rs" | "json") => {
                    Some(file.clone())
                }
                _ => None,
            }
        }));
    }
    if let Ok(pattern) =
        regex::Regex::new(r"(?i)\b[a-z_][a-z0-9_./\\-]*\.(?:html|css|js|ts|tsx|jsx|py|rs|json)\b")
    {
        files.extend(pattern.find_iter(reason).filter_map(|matched| {
            if matched.as_str().contains(".aura") {
                return None;
            }
            std::path::Path::new(matched.as_str())
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .filter(|name| {
                    name != "programmer_failure.json" && !name.eq_ignore_ascii_case("node.js")
                })
        }));
    }
    files.sort();
    files.dedup();
    files.retain(|file| !is_managed_verifier(workspace_path, file));
    files
}

fn command_from_forced_reason(reason: &str, official_command: Option<&str>) -> Option<String> {
    let lower = reason.to_lowercase();
    if let Some(command) = official_command {
        if reason.contains(command)
            || ["verific", "criterio", "contrato", "prueba", "test", "repar"]
                .iter()
                .any(|token| lower.contains(token))
        {
            return Some(command.to_string());
        }
    }
    for prefix in ["ejecutar '", "comando='", "comando: '"] {
        if let Some(rest) = reason.split(prefix).nth(1) {
            if let Some(command) = rest.split('\'').next() {
                if !command.trim().is_empty() {
                    return Some(command.trim().to_string());
                }
            }
        }
    }
    let trimmed = reason.trim().trim_matches('\'').trim_matches('`');
    let direct_prefixes = [
        "dir",
        "python ",
        "python3 ",
        "node ",
        "cargo ",
        "npm ",
        "npx ",
        "git ",
        "powershell ",
        "pwsh ",
    ];
    if direct_prefixes.iter().any(|prefix| {
        trimmed.eq_ignore_ascii_case(prefix.trim()) || trimmed.to_lowercase().starts_with(prefix)
    }) {
        return Some(trimmed.to_string());
    }
    None
}

fn is_test_runner_command(command: &str) -> bool {
    let command = command.trim().to_ascii_lowercase();
    [
        "npm test",
        "npm run test",
        "pnpm test",
        "pnpm run test",
        "yarn test",
        "bun test",
        "node --test",
        "cargo test",
        "pytest",
        "python -m pytest",
        "python -m unittest",
        "go test",
        "dotnet test",
        "npx jest",
        "npx vitest",
        "npx playwright test",
        "playwright test",
    ]
    .iter()
    .any(|prefix| command == *prefix || command.starts_with(&format!("{} ", prefix)))
}

fn is_firebase_hosting_spark_executable_rejection(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    lower.contains("executable files are forbidden on the spark billing plan")
        && (lower.contains("firebasehosting.googleapis.com")
            || lower.contains("hosting")
            || lower.contains("populatefiles"))
}

/// Runtime-owned transitions do not need a second LLM call merely to select a tool.
/// The model is still used for the creative programming payload itself.
fn deterministic_forced_decision(
    workspace_path: &str,
    forced: &str,
    reason: &str,
    contract: &crate::core::mission_contract::MissionContract,
    existing_files: &[String],
) -> Option<serde_json::Value> {
    let base = |tool: &str| {
        serde_json::json!({
            "herramienta": tool,
            "pensamiento": reason,
            "comando": null,
            "task_id": null,
            "url_a_investigar": null,
            "archivos_a_editar": [],
            "ast_nodes": [],
            "respuesta_conversacional": null
        })
    };
    match forced {
        "TOOL_PROGRAMMER" => {
            let files = relevant_programmer_files(workspace_path, contract, existing_files, reason);
            if files.is_empty() {
                None
            } else {
                let mut value = base(forced);
                value["archivos_a_editar"] = serde_json::json!(files);
                Some(value)
            }
        }
        "TOOL_TERMINAL" => {
            let official = official_verifier_command(contract);
            command_from_forced_reason(reason, official.as_deref()).map(|command| {
                let mut value = base(forced);
                value["comando"] = serde_json::json!(command);
                value
            })
        }
        "TOOL_WEB_SEARCH" => {
            let mut value = base(forced);
            value["comando"] = serde_json::json!(
                "site:firebase.google.com/docs Firebase web app Authentication Cloud Firestore Hosting deployment"
            );
            Some(value)
        }
        "TOOL_FINISH" => {
            let mut value = base(forced);
            value["respuesta_conversacional"] = serde_json::json!(reason);
            Some(value)
        }
        // TOOL_THINK is a control transfer, not a request for creative output.
        // Reusing the already supplied diagnosis avoids three fragile JSON retries
        // when a small local model is explicitly told to think before repairing.
        "TOOL_THINK" if !reason.trim().is_empty() => {
            let mut value = base(forced);
            value["comando"] = serde_json::json!(reason.trim());
            Some(value)
        }
        "TOOL_BACKGROUND_START" if !reason.trim().is_empty() => {
            let mut value = base(forced);
            value["comando"] = serde_json::json!(reason.trim());
            Some(value)
        }
        "TOOL_BACKGROUND_READ" | "TOOL_BACKGROUND_QUERY" if !reason.trim().is_empty() => {
            let mut value = base(forced);
            value["task_id"] = serde_json::json!(reason.trim());
            Some(value)
        }
        "TOOL_VISION_EVALUATOR" if !reason.trim().is_empty() => {
            let mut value = base(forced);
            value["comando"] = serde_json::json!(format!(
                "Evalúa la aplicación web real en esta URL local y reporta evidencia visible y defectos: {}",
                reason.trim()
            ));
            Some(value)
        }
        _ => None,
    }
}

fn focused_repair_criteria(criteria: &[String], limit: usize) -> Vec<String> {
    let mut categories = std::collections::HashSet::new();
    let mut focused = Vec::new();
    for criterion in criteria {
        let lower = criterion.to_lowercase();
        let category = if lower.contains("radar") {
            "radar"
        } else if lower.contains("telemetr") || lower.contains("paquete") {
            "telemetry"
        } else if lower.contains("tráfico")
            || lower.contains("trafico")
            || lower.contains("gráfico")
            || lower.contains("grafico")
            || lower.contains("visualiza")
            || lower.contains("alerta")
        {
            "traffic"
        } else if lower.contains("diseño")
            || lower.contains("glass")
            || lower.contains("monospace")
            || lower.contains("neón")
            || lower.contains("neon")
        {
            "design"
        } else if lower.contains("html") || lower.contains("canvas") {
            "structure"
        } else {
            criterion.as_str()
        };
        if categories.insert(category.to_string()) {
            focused.push(criterion.clone());
            if focused.len() == limit {
                break;
            }
        }
    }
    focused
}

fn tactical_repair_blueprint(objective: &str) -> &'static str {
    let lower = objective.to_lowercase();
    if lower.contains("dashboard") && lower.contains("radar") && lower.contains("telemetr") {
        "Implementación mínima funcional esperada: conserva un único HTML autocontenido; usa un arreglo threatNodes con angle/radius y dibuja cada nodo con Math.cos y Math.sin dentro de requestAnimationFrame; crea generatePacket/updatePackets (o nombres equivalentes que contengan packet/paquete) con campos ip, protocol, latency y risk y actualiza el panel mediante textContent/innerHTML; usa un segundo <canvas id=\"trafficCanvas\"> y un contexto separado llamado trafficCtx para el historial de tráfico, sin borrar el Canvas del radar; muestra una barra alert con texto attack/ataque; aplica backdrop-filter, monospace y box-shadow/text-shadow. No añadas solo palabras: conecta cada dato a una función visible."
    } else if lower.contains("consulta")
        && lower.contains("factura")
        && lower.contains("contabilidad")
        && lower.contains("firebase")
    {
        "Implementa el MVP SOLAMENTE con HTML, CSS y JavaScript; no uses Python/Flask, requirements.txt ni verify_solution.py. Crea los entregables planificados index.html, styles.css, app.js, package.json y tests/app.test.js, progresivamente según la lista permitida. Separa la lógica de dominio comprobable de la interfaz y persiste el prototipo local en localStorage. En app.js exporta exactamente estas funciones: registerUser(data), loginUser(data), createClient(data), createAppointment(data), createInvoice(data) y addAccountingTransaction(data). Cada una debe validar sus datos, realizar la operación sobre el estado persistido necesario y devolver el resultado, no un booleano constante. Implementa la persistencia leyendo con localStorage.getItem y guardando con localStorage.setItem; las pruebas deben usar un almacenamiento en memoria compatible para comprobar que los datos realmente se guardan. Registro e inicio de sesión son demostrativos en local: avisa en la interfaz que no son autenticación segura de producción; Firebase Authentication/Firestore se integra en la fase posterior. En index.html enlaza styles.css y carga app.js con `<script type=\"module\" src=\"app.js\">`. En package.json declara `type: module` y un script test no vacío con el comando exacto `node --test tests/app.test.js`. En tests/app.test.js importa las seis funciones desde `../app.js` con módulos ES y registra al menos seis casos con `node:test`; cada caso debe invocar su función y usar `node:assert/strict` para comprobar resultado o rechazo. No mezcles require() con módulos ES ni dupliques declaraciones. No inventes país, moneda ni impuestos; calcula facturas a partir de importes explícitos. No cambies el alcance de la fase."
    } else {
        ""
    }
}

fn consultation_firebase_blueprint() -> &'static str {
    "Fase 2 solamente: conserva intactos index.html, styles.css y las funciones funcionales de Fase 1. Usa el SDK modular instalado: importa initializeApp desde firebase/app; getAuth, createUserWithEmailAndPassword, signInWithEmailAndPassword y connectAuthEmulator desde firebase/auth; getFirestore, connectFirestoreEmulator, addDoc/getDocs desde firebase/firestore. Configura un projectId de emulador fijo con prefijo demo- en firebase-config.js; nunca pongas ahí credenciales privadas. Conecta Auth en localhost:9099 y Firestore en localhost:8080 solo en ejecución local. Añade `test:firebase` a package.json usando `firebase emulators:exec --only auth,firestore --project <mismo-demo-id> \"node --test tests/firebase-emulator.test.js\"`; declara firebase como dependencia y firebase-tools como devDependency. Las pruebas deben importar operaciones reales de app.js y comprobar registro/inicio de sesión y escritura/lectura Firestore con node:test y node:assert/strict. Reglas deben exigir request.auth.uid y denegar acceso a otros usuarios. No declares completada esta fase con npm test: el único criterio es npm run test:firebase. La configuración y el projectId de producción se exigirán solo antes del deploy."
}

/// Detecta si el workspace contiene archivos HTML (entorno web/frontend)

/// Resume y compacta salidas verbose de terminal conservando solo lo crítico (errores, advertencias, confirmaciones).
/// Evita la saturación del contexto del LLM y acelera las inferencias.
fn digest_terminal_output(raw: &str, max_chars: usize) -> String {
    if raw.len() <= max_chars {
        return raw.to_string();
    }

    let lines: Vec<&str> = raw.lines().collect();
    let mut critical_lines = Vec::new();
    let mut generic_tail = Vec::new();

    for &line in &lines {
        let l_lower = line.to_lowercase();
        if l_lower.contains("error")
            || l_lower.contains("err:")
            || l_lower.contains("failed")
            || l_lower.contains("warning")
            || l_lower.contains("warn")
            || l_lower.contains("conflict")
            || l_lower.contains("panicked")
            || l_lower.contains("exception")
            || l_lower.contains("syntaxerror")
            || l_lower.contains("traceback")
            || l_lower.contains("assert")
        {
            critical_lines.push(line);
        }
    }

    // Conservar las últimas 15 líneas que suelen tener el resumen final (ej. test summary, exit status)
    let tail_count = 15.min(lines.len());
    for &line in &lines[lines.len() - tail_count..] {
        if !critical_lines.contains(&line) {
            generic_tail.push(line);
        }
    }

    let mut result = String::new();
    if !critical_lines.is_empty() {
        result.push_str("⚠️ [LÍNEAS CRÍTICAS / ERRORES]:\n");
        for line in critical_lines.iter().take(25) {
            result.push_str(line);
            result.push('\n');
        }
        result.push('\n');
    }

    result.push_str("📋 [ÚLTIMAS LÍNEAS DE SALIDA]:\n");
    for line in generic_tail {
        result.push_str(line);
        result.push('\n');
    }

    if result.len() > max_chars {
        format!(
            "{}...\n[Salida recortada para eficiencia]",
            truncate_chars(&result, max_chars)
        )
    } else {
        result
    }
}

#[derive(Clone, Serialize)]
pub struct AgentEvent {
    pub step: u32,
    pub message: String,
    pub status: String,
}

pub fn emit_event(app: &AppHandle, step: u32, message: &str, status: &str) {
    let event = AgentEvent {
        step,
        message: message.to_string(),
        status: status.to_string(),
    };
    let _ = app.emit("agent-step", event);
}

#[derive(Serialize)]
pub struct FinalResponse {
    pub status: String,
    pub respuesta_conversacional: String,
}

/// ── Multi-Agent Role FSM ──────────────────────────────────────────────────
/// Aura Sentinel operates in a three-phase cycle:
///   Planner  → designs the architecture in RAM (TOOL_AST_INJECT, TOOL_MAPPER, TOOL_THINK)
///   Executor → writes physical code to disk (TOOL_PROGRAMMER, TOOL_TERMINAL, TOOL_ENV_MANAGER)
///   Critic   → validates correctness (TOOL_TESTER, TOOL_TERMINAL, TOOL_VISION_EVALUATOR, TOOL_FINISH)
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
enum AgentRole {
    Planner,
    Executor,
    Critic,
}

fn apply_consultation_mvp_audit_override(
    tool: &mut String,
    role: &mut AgentRole,
    command: &mut String,
    files: &mut Vec<String>,
    payload: &mut serde_json::Value,
    verifier_diagnostic: &mut String,
    audit: &crate::llm::phase_planner::ConsultationMvpAudit,
) -> bool {
    if audit.issues.is_empty() {
        return false;
    }

    *tool = "TOOL_PROGRAMMER".to_string();
    *role = AgentRole::Executor;
    command.clear();
    // Keep each repair small enough for the local 7B coder. A four-file audit
    // request was causing it to emit partial changes (often the root-level
    // app.test.js alias) and then hit duplicate-patch conflicts. Repair tests
    // first, then the core, manifest, and UI, re-auditing after each write.
    let repair_order = ["tests/app.test.js", "app.js", "package.json", "index.html"];
    let next_file = repair_order
        .iter()
        .copied()
        .find(|file| {
            audit
                .repair_files
                .iter()
                .any(|target| target.as_str() == *file)
        })
        .or_else(|| audit.repair_files.first().map(String::as_str))
        .unwrap_or("app.js");
    *files = vec![next_file.to_string()];
    let focused_issues = audit
        .issues
        .iter()
        .filter(|issue| issue.contains(next_file))
        .cloned()
        .collect::<Vec<_>>();
    *verifier_diagnostic = if focused_issues.is_empty() {
        audit.issues.join("\n")
    } else {
        focused_issues.join("\n")
    };
    if let Some(object) = payload.as_object_mut() {
        object.insert("herramienta".into(), serde_json::json!("TOOL_PROGRAMMER"));
        object.insert("comando".into(), serde_json::Value::Null);
        object.insert("archivos_a_editar".into(), serde_json::json!(files));
        object.insert("require_all_targets".into(), serde_json::Value::Bool(true));
        object.insert(
            "instruccion".into(),
            serde_json::json!(format!(
                "Corrige el defecto comprobado del archivo objetivo en esta propuesta. Devuelve exactamente un cambio completo para ese archivo y no cambies otros entregables. Diagnóstico comprobado en disco:\n{}\nEdita solo este archivo: {:?}. El auditor volverá a comprobar los demás archivos en los siguientes pasos.",
                verifier_diagnostic, files
            )),
        );
        object.insert(
            "context".into(),
            serde_json::json!(format!(
                "Auditoría funcional de Fase 1:\n{}",
                verifier_diagnostic
            )),
        );
    }
    true
}

fn apply_consultation_firebase_audit_override(
    tool: &mut String,
    role: &mut AgentRole,
    command: &mut String,
    files: &mut Vec<String>,
    payload: &mut serde_json::Value,
    verifier_diagnostic: &mut String,
    audit: &crate::llm::phase_planner::ConsultationFirebaseAudit,
) -> bool {
    if audit.issues.is_empty() {
        return false;
    }

    *tool = "TOOL_PROGRAMMER".to_string();
    *role = AgentRole::Executor;
    command.clear();
    let repair_order = [
        "tests/firebase-emulator.test.js",
        "package.json",
        "app.js",
        "firebase-config.js",
        "firestore.rules",
        "firebase.json",
    ];
    let next_file = repair_order
        .iter()
        .copied()
        .find(|file| audit.repair_files.iter().any(|target| target == file))
        .or_else(|| audit.repair_files.first().map(String::as_str))
        .unwrap_or("app.js");
    *files = vec![next_file.to_string()];
    let focused = audit
        .issues
        .iter()
        .filter(|issue| issue.contains(next_file))
        .cloned()
        .collect::<Vec<_>>();
    *verifier_diagnostic = if focused.is_empty() {
        audit.issues.join("\n")
    } else {
        focused.join("\n")
    };
    if let Some(object) = payload.as_object_mut() {
        object.insert("herramienta".into(), serde_json::json!("TOOL_PROGRAMMER"));
        object.insert("comando".into(), serde_json::Value::Null);
        object.insert("archivos_a_editar".into(), serde_json::json!(files));
        object.insert("require_all_targets".into(), serde_json::Value::Bool(true));
        object.insert(
            "instruccion".into(),
            serde_json::json!(format!(
                "Corrige solo el defecto Firebase comprobado en el archivo objetivo. Mantén la Fase 1 funcional; conecta la operación real al SDK modular y al emulador cuando corresponda. No uses valores de proyecto de producción en las pruebas. Devuelve un cambio completo únicamente para {:?}. Diagnóstico en disco:\n{}",
                next_file, verifier_diagnostic
            )),
        );
        object.insert(
            "context".into(),
            serde_json::json!(format!(
                "Auditoría de integración Firebase, Fase 2:\n{}",
                verifier_diagnostic
            )),
        );
    }
    true
}

fn successful_command_for_world(
    history: &std::collections::HashSet<String>,
    command: &str,
    world_hash: u64,
) -> bool {
    history.contains(&format!(
        "{}|{}",
        command.trim().to_ascii_lowercase(),
        world_hash
    ))
}

fn is_forced_test_repair(
    forced_override: Option<&(String, String)>,
    has_failed_test_run: bool,
) -> bool {
    has_failed_test_run
        && forced_override
            .map(|(tool, _)| tool == "TOOL_PROGRAMMER")
            .unwrap_or(false)
}

fn only_deliverables_review_pending(missing: &[String]) -> bool {
    !missing.is_empty()
        && missing
            .iter()
            .all(|reason| reason.contains("AC-DELIVERABLES"))
}

fn consultation_audit_diagnostic_improved(previous: &str, current: &str) -> bool {
    if previous.trim().is_empty() || current.trim().is_empty() {
        return false;
    }
    let resolved = previous
        .lines()
        .filter(|issue| !current.lines().any(|current_issue| current_issue == *issue))
        .count();
    let introduced = current
        .lines()
        .filter(|issue| {
            !previous
                .lines()
                .any(|previous_issue| previous_issue == *issue)
        })
        .count();
    resolved > introduced
}

fn update_consultation_audit_stall_count(
    last_world: &mut Option<u64>,
    repeated_without_progress: &mut u8,
    last_diagnostic: &mut String,
    world: u64,
    diagnostic: &str,
) -> u8 {
    let world_changed = *last_world != Some(world);
    let diagnostic_improved = consultation_audit_diagnostic_improved(last_diagnostic, diagnostic);
    if world_changed || diagnostic_improved {
        *repeated_without_progress = 0;
    } else {
        *repeated_without_progress = repeated_without_progress.saturating_add(1);
    }
    *last_world = Some(world);
    *last_diagnostic = diagnostic.to_string();
    *repeated_without_progress
}

fn reset_consultation_audit_stall_tracking(
    last_world: &mut Option<u64>,
    repeated_without_progress: &mut u8,
    last_diagnostic: &mut String,
) {
    *last_world = None;
    *repeated_without_progress = 0;
    last_diagnostic.clear();
}

fn is_node_test_jest_mock_failure(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    let uses_node_test = lower.contains("node --test") || lower.contains("node:test");
    uses_node_test
        && [
            "jest is not defined",
            "jest.fn(",
            ".mockreturnvalue",
            ".mockimplementation",
            ".mock.calls",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
}

fn is_node_test_undefined_push_failure(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    let uses_node_test = lower.contains("node --test") || lower.contains("node:test");
    uses_node_test
        && (lower.contains("cannot read properties of undefined (reading 'push')")
            || lower.contains("cannot read property 'push' of undefined"))
}

fn is_node_test_uninitialized_storage_mock_failure(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    let uses_node_test = lower.contains("node --test") || lower.contains("node:test");
    let points_to_test_file = lower.contains(".test.")
        || lower.contains("/tests/")
        || lower.contains("\\tests\\")
        || lower.contains("test at tests\\");
    let reads_missing_map_method = lower
        .contains("cannot read properties of undefined (reading 'set')")
        || lower.contains("cannot read properties of undefined (reading 'get')")
        || lower.contains("cannot read property 'set' of undefined")
        || lower.contains("cannot read property 'get' of undefined");
    uses_node_test && points_to_test_file && reads_missing_map_method
}

fn is_node_test_browser_document_failure(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    let uses_node_test = lower.contains("node --test") || lower.contains("node:test");
    uses_node_test
        && (lower.contains("referenceerror: document is not defined")
            || lower.contains("document is not defined"))
}

fn reuse_or_record_recovery_decision(
    runtime: &mut crate::core::mission_runtime::MissionRuntime,
    existing: Option<crate::core::recovery::RecoveryDecision>,
    tool_name: &str,
    error: &str,
) -> crate::core::recovery::RecoveryDecision {
    existing.unwrap_or_else(|| runtime.plan_recovery(tool_name, error))
}

fn workspace_changed_after_programmer(
    affected_files: &[String],
    physical_files_changed: u32,
) -> bool {
    !affected_files.is_empty() || physical_files_changed > 0
}

fn reset_terminal_retry_circuits_after_success(
    runtime: &mut crate::core::mission_runtime::MissionRuntime,
    retry_tracker: &mut crate::core::error_classifier::RetryTracker,
) {
    // Only a successful terminal action proves that the terminal failure loop
    // has recovered. A source-file write alone may be unrelated or may repeat
    // the same invalid repair (for example, another malformed Cargo.toml).
    runtime.recovery.record_success("TOOL_TERMINAL");
    retry_tracker.reset_all();
}

fn apply_forced_test_repair_override(
    tool: &mut String,
    role: &mut AgentRole,
    command: &mut String,
    files: &mut Vec<String>,
    payload: &mut serde_json::Value,
    diagnostic: &mut String,
    failed_command: &str,
    failure_output: &str,
) {
    let is_jest_mock_mismatch = is_node_test_jest_mock_failure(failure_output);
    let is_undefined_push_failure = is_node_test_undefined_push_failure(failure_output);
    let is_uninitialized_storage_mock =
        is_node_test_uninitialized_storage_mock_failure(failure_output);
    let is_browser_document_failure = is_node_test_browser_document_failure(failure_output);
    if is_jest_mock_mismatch
        || is_undefined_push_failure
        || is_uninitialized_storage_mock
        || is_browser_document_failure
    {
        let failure_lower = failure_output.to_ascii_lowercase();
        let test_files: Vec<String> = files
            .iter()
            .filter(|file| {
                let lower = file.to_ascii_lowercase().replace('\\', "/");
                let is_test = lower.contains("/test/")
                    || lower.contains("/tests/")
                    || lower.contains(".test.")
                    || lower.contains("_test.")
                    || lower.starts_with("test/")
                    || lower.starts_with("tests/");
                if is_jest_mock_mismatch || is_uninitialized_storage_mock {
                    is_test
                } else if is_browser_document_failure {
                    let is_source =
                        lower.ends_with(".js") || lower.ends_with(".mjs") || lower.ends_with(".ts");
                    let basename = lower.rsplit('/').next().unwrap_or(&lower);
                    is_source && !is_test && failure_lower.contains(basename)
                } else {
                    let basename = lower.rsplit('/').next().unwrap_or(&lower);
                    is_test || failure_lower.contains(basename)
                }
            })
            .cloned()
            .collect();
        if !test_files.is_empty() {
            *files = test_files;
        }
    }

    let guidance = if is_jest_mock_mismatch {
        "El runner real es node --test. Reescribe completo el archivo de pruebas con import { beforeEach, test } from 'node:test' y assert de 'node:assert/strict'. Simula localStorage con un Map y métodos normales getItem(key), setItem(key, value), removeItem(key) y clear(); reinicia el Map en beforeEach. No uses jest, jest.fn(), mockReturnValue, mockImplementation, mock.calls ni APIs de Vitest; no instales dependencias. Conserva casos que ejerciten las funciones exportadas y comprueba el estado persistido real."
    } else if is_uninitialized_storage_mock {
        "La traza apunta al mock de localStorage en el archivo de pruebas: usa this.storage.get/set, pero storage no está inicializado. Reescribe completo únicamente el archivo de pruebas señalado con un Map creado antes de los tests (const storage = new Map()), métodos getItem/setItem/removeItem/clear que usen ese Map y beforeEach para limpiarlo. El runner es node --test: usa node:test y node:assert/strict, elimina Jest mock.calls y no cambies el runner ni app.js para encubrir un defecto del test. Mantén pruebas de cada operación y verifica el estado persistido."
    } else if is_browser_document_failure {
        "La prueba Node importa un módulo de navegador que accede a document durante la carga. Conserva las funciones de dominio exportadas para Node y mueve la inicialización de la interfaz detrás de `if (typeof document !== 'undefined')`, comprobando que cada elemento exista antes de registrar eventos. No simules document en los tests para ocultar el defecto ni cambies el runner; edita solo el módulo fuente que aparece en la traza."
    } else if is_undefined_push_failure {
        "El runner ya ejecuta las pruebas; corrige el fallo funcional indicado por la traza. Inspecciona la línea de app.js que intenta hacer push sobre undefined y la lectura localStorage que alimenta esa colección. Inicializa colecciones ausentes o JSON inválido como arreglos, conserva las claves y funciones exportadas reales, y ajusta el mock Map de la prueba para sembrar las claves correctas. Mantén los casos y sus aserciones: no elimines ni debilites pruebas, no ocultes el error y no cambies de runner. Si el parche no coincide, vuelve a leer el archivo actual y reemplázalo completo."
    } else {
        "Corrige el defecto indicado por la salida de la prueba. Conserva el runner, el stack y las dependencias existentes; no repitas la misma prueba antes de modificar el código."
    };
    *tool = "TOOL_PROGRAMMER".to_string();
    *role = AgentRole::Executor;
    command.clear();
    *diagnostic = format!(
        "Falló el verificador `{}`. Salida relevante: {}. {}",
        failed_command,
        truncate_chars(failure_output.trim(), 900),
        guidance
    );
    if let Some(object) = payload.as_object_mut() {
        object.insert("herramienta".into(), serde_json::json!("TOOL_PROGRAMMER"));
        object.insert("comando".into(), serde_json::Value::Null);
        object.insert("archivos_a_editar".into(), serde_json::json!(files));
        object.insert(
            "instruccion".into(),
            serde_json::json!(format!(
                "Repara el fallo real del verificador antes de volver a probar. Edita únicamente {:?}. Diagnóstico: {}",
                files, diagnostic
            )),
        );
        object.insert("context".into(), serde_json::json!(diagnostic));
    }
}

/// ── Mission Type Classifier ───────────────────────────────────────────────
/// Classifies the user intent BEFORE entering the LLM loop.
/// ANALYSIS tasks never enter the Executor — they resolve via TOOL_FINISH from the Planner.
#[derive(Debug, Clone, PartialEq)]
enum MissionType {
    Planning,     // Define an initial scope and phased plan before implementation
    Analysis,     // "analiza", "describe", "explica", "qué hay"
    Construction, // "crea", "implementa", "build"
    Refactor,     // "mejora", "optimiza", "refactoriza"
    Debug,        // "arregla", "bug", "error", "fix"
    Execution,    // "ejecuta", "corre", "prueba", "testea", "run", "verify"
}

pub(crate) fn is_initial_plan_request(msg: &str) -> bool {
    let m = msg.to_lowercase();
    if m.contains("[modo implementacion de plan aprobado]") {
        return false;
    }
    let has_plan_marker = [
        "plan inicial",
        "plan preliminar",
        "plan de inicio",
        "por ahora este es el plan",
        "por ahora es el plan",
    ]
    .iter()
    .any(|marker| m.contains(marker));
    let has_project_goal = [
        "aplicacion",
        "aplicación",
        "sistema",
        "proyecto",
        "construir",
        "construye",
        "contruir",
        "desarrollar",
        "implementar",
    ]
    .iter()
    .any(|cue| m.contains(cue));

    has_plan_marker && has_project_goal
}

fn planning_tool_allowed(tool: &str) -> bool {
    matches!(tool, "TOOL_WEB_SEARCH" | "TOOL_FINISH" | "TOOL_THINK")
}

fn initial_plan_response_complete(response: &str) -> bool {
    let text = response.to_lowercase();
    let has_two_phases = text.contains("fase 1") && text.contains("fase 2");
    let has_deliverables = text.contains("entregable");
    let has_assumptions = text.contains("supuesto") || text.contains("asuncion");
    let has_open_decisions = text.contains("decisiones pendientes") || text.contains("preguntas");

    !response.trim().is_empty()
        && has_two_phases
        && has_deliverables
        && has_assumptions
        && has_open_decisions
}

fn planning_response_complete(response: &str, research_failed: bool, request: &str) -> bool {
    if !initial_plan_response_complete(response) {
        return false;
    }
    if !missing_requested_plan_coverage(request, response).is_empty() {
        return false;
    }
    let request_lower = request.to_lowercase();
    let local_first = request_lower.contains("local")
        && (request_lower.contains("luego")
            || request_lower.contains("después")
            || request_lower.contains("despues"))
        && (request_lower.contains("firebase") || request_lower.contains("despleg"));
    if local_first && !plan_tests_locally_before_deploying(response) {
        return false;
    }
    if !research_failed {
        return true;
    }
    let text = response.to_lowercase();
    text.contains("no se pudo verificar")
        || text.contains("búsqueda web falló")
        || text.contains("busqueda web fallo")
        || text.contains("sin conexión")
        || text.contains("sin conexion")
}

fn missing_requested_plan_coverage(request: &str, response: &str) -> Vec<String> {
    let request = request.to_lowercase();
    let response = response.to_lowercase();
    let checks: [(&str, &[&str], &[&str]); 9] = [
        (
            "consultas o citas",
            &["consulta", "cita", "agenda"],
            &["consulta", "cita", "agenda", "reserv"],
        ),
        (
            "clientes o pacientes",
            &["cliente", "paciente"],
            &["cliente", "paciente"],
        ),
        (
            "facturación",
            &["factur", "facuta", "facut", "invoice"],
            &["factur", "facuta", "invoice", "comprobante"],
        ),
        (
            "contabilidad",
            &["contabilidad", "accounting"],
            &["contabilidad", "accounting", "libro mayor"],
        ),
        (
            "inicio de sesión",
            &["login", "inicio de sesión", "inicio de sesion", "acceso"],
            &[
                "login",
                "inicio de sesión",
                "inicio de sesion",
                "autentic",
                "acceso",
            ],
        ),
        (
            "registro de usuarios",
            &["registro", "crear cuenta"],
            &["registro", "crear cuenta", "alta de usuario"],
        ),
        (
            "pruebas locales",
            &["local", "emulador local"],
            &[
                "prueba local",
                "pruebas local",
                "probar local",
                "localmente",
                "emulador local",
            ],
        ),
        ("servicios Firebase", &["firebase"], &["firebase"]),
        (
            "despliegue solicitado",
            &["despleg", "deploy"],
            &["despleg", "hosting", "publicar", "deploy"],
        ),
    ];
    checks
        .iter()
        .filter_map(|(label, triggers, coverage)| {
            let requested = triggers.iter().any(|term| request.contains(term));
            let covered = coverage.iter().any(|term| response.contains(term));
            (requested && !covered).then(|| (*label).to_string())
        })
        .collect()
}

fn plan_tests_locally_before_deploying(response: &str) -> bool {
    let text = response.to_lowercase();
    let Some(phase_one) = text.find("fase 1") else {
        return false;
    };
    let Some(phase_two) = text[phase_one + 6..]
        .find("fase 2")
        .map(|index| phase_one + 6 + index)
    else {
        return false;
    };
    let first_phase = &text[phase_one..phase_two];
    let later_phases = &text[phase_two..];
    let local_validation = first_phase.contains("local")
        && [
            "prueba",
            "probar",
            "test",
            "emulador",
            "validación",
            "validacion",
        ]
        .iter()
        .any(|term| first_phase.contains(term));
    let later_deploy = ["desplieg", "hosting", "publicar", "deploy"]
        .iter()
        .any(|term| later_phases.contains(term));
    local_validation && later_deploy
}

/// Gives small local models a deterministic escape hatch when they repeatedly
/// produce a plan that fails the request-aware completion checks.
fn build_fallback_initial_plan(request: &str, research_failed: bool) -> String {
    let mut plan = format!(
        "## Plan inicial\n\n\
         **Objetivo:** construir una aplicación web de gestión de consultas, con acceso de usuarios, facturación y contabilidad.\n\n\
         ### Fase 1 — MVP y pruebas locales\n\
         **Objetivo:** acordar el alcance funcional mínimo e implementar el sistema para ejecutarlo y validarlo localmente antes de publicarlo.\n\
         **Alcance:** registro e inicio de sesión; perfiles y roles básicos; clientes; agenda y gestión de consultas; emisión y consulta de facturas; y módulo de contabilidad con movimientos y reportes básicos.\n\
         **Entregables:** aplicación web funcional; persistencia y autenticación probadas con el entorno local o emuladores de Firebase; pruebas de registro, acceso, consultas, facturación y contabilidad.\n\
         **Condición de salida:** los flujos principales pasan las pruebas locales y se revisan permisos, validación de datos y errores antes de publicar.\n\n\
         ### Fase 2 — Firebase y despliegue\n\
         **Objetivo:** configurar los servicios de Firebase para el proyecto y publicar el MVP aprobado.\
         **Entregables:** Firebase Authentication, Cloud Firestore con reglas de seguridad, configuración de Hosting y despliegue; pruebas de acceso y flujos críticos en el entorno publicado.\
         **Condición de salida:** el sitio está disponible en Firebase Hosting y las pruebas de humo posteriores al despliegue pasan.\n\n\
         **Supuestos:** este mandato solicita una hoja de ruta inicial, no comenzar todavía la implementación. “Sistema de consulta” se interpreta como gestión de clientes y consultas/agenda, sin asumir un expediente clínico. La facturación y contabilidad se planifican como módulos, sin inventar reglas fiscales.\n\
         **Decisiones pendientes para las fases correspondientes:** confirmar el país objetivo antes de definir impuestos, numeración o cumplimiento fiscal; validar los roles y datos específicos de la consulta antes de cerrar el esquema. Estas decisiones no bloquean el diseño ni las pruebas locales iniciales.\n"
    );

    // Preserve any additional explicit requirements recognized by the same
    // conservative keyword gate, so fallback cannot silently narrow scope.
    let additional = missing_requested_plan_coverage(request, &plan);
    if !additional.is_empty() {
        plan.push_str("\n**Requisitos explícitos que deben incluirse en el alcance:** ");
        plan.push_str(&additional.join(", "));
        plan.push_str(".\n");
    }
    if research_failed {
        plan.push_str("\n**Investigación:** no se pudo verificar información actual de Firebase; por eso las decisiones de configuración deben confirmarse con la documentación oficial antes del despliegue.\n");
    } else {
        plan.push_str("\n**Referencias oficiales para validar durante la fase técnica:** [Firebase Local Emulator Suite](https://firebase.google.com/docs/emulator-suite), [Firebase Authentication para web](https://firebase.google.com/docs/auth/web/start), [reglas de seguridad de Firestore](https://firebase.google.com/docs/firestore/security/get-started) y [Firebase Hosting](https://firebase.google.com/docs/hosting).\n");
    }
    plan
}

fn conversational_response(value: Option<&serde_json::Value>) -> String {
    match value {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Null) | None => String::new(),
        Some(other) => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

fn classify_mission(msg: &str) -> MissionType {
    let m = msg.to_lowercase();
    if is_initial_plan_request(&m) {
        return MissionType::Planning;
    }
    let execution = [
        "ejecuta",
        "ejecutar",
        "corre",
        "correr",
        "testea",
        "run",
        "execute",
        "verifica",
        "verificar",
    ];
    let construction = [
        "construye",
        "construir",
        "crea",
        "crear",
        "implementa",
        "implementar",
        "escribe",
        "escribir",
        "genera",
        "generar",
        "programa",
        "programar",
        "desarrolla",
        "desarrollar",
        "build",
        "create",
        "write",
        "microservicio",
    ];
    let debug = [
        "arregla",
        "corrige",
        "bug",
        "falla",
        "fallo",
        "fix",
        "debug",
        "broken",
        "no funciona",
        "no compila",
        "sale error",
        "hay un error",
    ];
    let refactor = [
        "refactoriza",
        "refactorizar",
        "mejora",
        "optimiza",
        "limpia el",
        "reorganiza",
        "simplifica",
    ];
    let analysis = [
        "analiza",
        "analisa",
        "analice",
        "analisis",
        "que hay",
        "qué hay",
        "que sistema",
        "qué sistema",
        "describe",
        "explica",
        "muéstrame",
        "muestrame",
        "que tiene",
        "qué tiene",
        "que contiene",
        "qué contiene",
        "inspect",
        "analyze",
        "show me",
        "que es",
        "qué es",
        "que tipo",
        "qué tipo",
        "que hace",
        "qué hace",
        "analisa este",
        "analiza este",
        "revisa este",
        "auditoría",
        "auditoria",
        "audita",
        "sat",
        "lógica",
        "logica",
        "satisfacibilidad",
        "donde quedaste",
        "dónde quedaste",
        "donde te quedaste",
        "dónde te quedaste",
        "en que quedamos",
        "en qué quedamos",
        "en que quedaste",
        "en qué quedaste",
        "que falta",
        "qué falta",
        "que queda",
        "qué queda",
        "como va",
        "cómo va",
        "estado actual",
        "estado del proyecto",
        "cual es el estado",
        "cuál es el estado",
        "resumen del estado",
        "informe",
        "reporte",
        "status",
    ];

    if construction.iter().any(|w| m.contains(w)) {
        return MissionType::Construction;
    }
    if execution.iter().any(|w| m.contains(w)) {
        return MissionType::Execution;
    }
    if debug.iter().any(|w| m.contains(w)) {
        return MissionType::Debug;
    }
    if refactor.iter().any(|w| m.contains(w)) {
        return MissionType::Refactor;
    }
    if analysis.iter().any(|w| m.contains(w)) {
        return MissionType::Analysis;
    }
    MissionType::Construction
}

/// Helper to verify if a required file from a phase is strictly satisfied on disk.
/// Requires exact file presence and non-empty content (size > 0).
/// Does not allow loose extension matching or inline script shortcuts.
fn is_phase_file_satisfied(workspace_path: &str, file_name: &str) -> bool {
    let ws = std::path::Path::new(workspace_path);
    crate::core::workspace_resolver::WorkspaceResolver::new(ws)
        .and_then(|r| r.file_exists(file_name))
        .unwrap_or(false)
}

/// Formats the acceptance contract from the Planner's TOOL_THINK 'comando' field.
fn formato_contrato(cmd: &str) -> String {
    let mut out = format!("CRITERIOS DE EXITO DEFINIDOS POR EL PLANIFICADOR:\n{}", cmd);
    let cmd_lower = cmd.to_lowercase();
    if cmd_lower.contains("dashboard") || cmd_lower.contains("panel") {
        out.push_str("\n\nCRITERIOS AUTOMÁTICOS PARA DASHBOARDS (P1-3):");
        out.push_str("\n- [ ] Layout de cuadrícula/flexbox responsivo.");
        out.push_str("\n- [ ] Contenedores para gráficos interactivos/datos.");
        out.push_str("\n- [ ] Separación clara de UI y lógica de actualización.");
    }
    out
}

/// Auto-genera runners (test, build, dev, lint) para el proyecto detectando el lenguaje
async fn generate_project_runners(workspace_path: &str, prompt: &str) -> Vec<std::path::PathBuf> {
    use std::path::Path;

    let _project_root = Path::new(workspace_path);
    let _all_generated: Vec<std::path::PathBuf> = Vec::new();

    let projects = detect_projects(workspace_path);
    let mut all_runners = Vec::new();

    for proj in projects {
        let language = proj.language;
        let root = proj.root;

        let mut lang = language.clone();
        if lang == "unknown" {
            let prompt_lower = prompt.to_lowercase();
            if prompt_lower.contains("rust") || prompt_lower.contains("cargo") {
                lang = "rust".to_string();
            } else if prompt_lower.contains("python")
                || prompt_lower.contains("django")
                || prompt_lower.contains("flask")
                || prompt_lower.contains("fastapi")
            {
                lang = "python".to_string();
            } else if prompt_lower.contains("javascript")
                || prompt_lower.contains("node")
                || prompt_lower.contains("react")
                || prompt_lower.contains("vue")
                || prompt_lower.contains("npm")
            {
                lang = "javascript".to_string();
            } else if prompt_lower.contains("typescript")
                || prompt_lower.contains("tsx")
                || prompt_lower.contains("ts ")
            {
                lang = "typescript".to_string();
            } else if prompt_lower.contains("go ") || prompt_lower.contains("golang") {
                lang = "go".to_string();
            } else if prompt_lower.contains("java")
                || prompt_lower.contains("spring")
                || prompt_lower.contains("maven")
                || prompt_lower.contains("gradle")
            {
                lang = "java".to_string();
            }
        }

        if lang != "unknown" {
            let mut runners = generate_for_language(&lang, &root).await;
            all_runners.append(&mut runners);
        }
    }
    all_runners
}

async fn generate_for_language(
    language: &str,
    project_root: &std::path::Path,
) -> Vec<std::path::PathBuf> {
    let (test_cmd, build_cmd, dev_cmd, lint_cmd) = match language {
        "rust" => (
            Some("cargo test".to_string()),
            Some("cargo build".to_string()),
            Some("cargo run".to_string()),
            Some("cargo clippy".to_string()),
        ),
        "python" => (
            Some("python -m pytest".to_string()),
            Some("python -m py_compile src/**/*.py".to_string()),
            Some("python main.py".to_string()),
            Some("ruff check .".to_string()),
        ),
        "javascript" | "typescript" => (
            Some("npm test".to_string()),
            Some("npm run build".to_string()),
            Some("npm run dev".to_string()),
            Some("npm run lint".to_string()),
        ),
        "go" => (
            Some("go test ./...".to_string()),
            Some("go build".to_string()),
            Some("go run main.go".to_string()),
            Some("golangci-lint run".to_string()),
        ),
        "java" => (
            Some("mvn test".to_string()),
            Some("mvn compile".to_string()),
            Some("mvn spring-boot:run".to_string()),
            Some("mvn checkstyle:check".to_string()),
        ),
        "kotlin" => (
            Some("./gradlew test".to_string()),
            Some("./gradlew build".to_string()),
            Some("./gradlew run".to_string()),
            Some("./gradlew detekt".to_string()),
        ),
        "php" => (
            Some("./vendor/bin/phpunit".to_string()),
            Some("composer install".to_string()),
            Some("php artisan serve".to_string()),
            Some("./vendor/bin/phpcs".to_string()),
        ),
        "dart" => (
            Some("flutter test".to_string()),
            Some("flutter build".to_string()),
            Some("flutter run".to_string()),
            Some("flutter analyze".to_string()),
        ),
        "swift" => (
            Some("swift test".to_string()),
            Some("swift build".to_string()),
            Some("swift run".to_string()),
            Some("swiftlint".to_string()),
        ),
        "csharp" => (
            Some("dotnet test".to_string()),
            Some("dotnet build".to_string()),
            Some("dotnet run".to_string()),
            Some("dotnet format".to_string()),
        ),
        "ruby" => (
            Some("rspec".to_string()),
            Some("bundle install".to_string()),
            Some("rails server".to_string()),
            Some("rubocop".to_string()),
        ),
        "solidity" => (
            Some("forge test".to_string()),
            Some("forge build".to_string()),
            Some("anvil".to_string()),
            Some("forge fmt".to_string()),
        ),
        _ => (None, None, None, None),
    };

    match generate_standard_runners(
        project_root,
        language,
        test_cmd,
        build_cmd,
        dev_cmd,
        lint_cmd,
    )
    .await
    {
        Ok(paths) => paths,
        Err(_) => Vec::new(),
    }
}

#[derive(Debug)]
pub struct ProjectDescriptor {
    pub root: std::path::PathBuf,
    pub language: String,
}

pub fn detect_projects(workspace_path: &str) -> Vec<ProjectDescriptor> {
    use std::path::Path;
    let ws = Path::new(workspace_path);
    let mut projects = Vec::new();

    let mut dirs_to_check = vec![ws.to_path_buf()];

    if let Ok(entries) = std::fs::read_dir(ws) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if name != "node_modules" && name != "target" && name != ".git" && name != ".venv" {
                    dirs_to_check.push(path);
                }
            }
        }
    }

    for path in dirs_to_check {
        let mut lang = "unknown".to_string();
        if path.join("Cargo.toml").exists() {
            lang = "rust".to_string();
        } else if path.join("package.json").exists() {
            if path.join("tsconfig.json").exists() {
                lang = "typescript".to_string();
            } else {
                lang = "javascript".to_string();
            }
        } else if path.join("requirements.txt").exists()
            || path.join("pyproject.toml").exists()
            || path.join("main.py").exists()
        {
            lang = "python".to_string();
        } else if path.join("go.mod").exists() {
            lang = "go".to_string();
        } else if path.join("pom.xml").exists() {
            lang = "java".to_string();
        } else if path.join("build.gradle").exists() || path.join("build.gradle.kts").exists() {
            lang = "kotlin".to_string();
        } else if path.join("composer.json").exists() {
            lang = "php".to_string();
        } else if path.join("pubspec.yaml").exists() {
            lang = "dart".to_string();
        } else if path.join("Package.swift").exists() {
            lang = "swift".to_string();
        } else if std::fs::read_dir(&path)
            .map(|entries| {
                entries.filter_map(|e| e.ok()).any(|e| {
                    e.path()
                        .extension()
                        .map(|ext| ext == "csproj")
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
        {
            lang = "csharp".to_string();
        } else if path.join("Gemfile").exists() {
            lang = "ruby".to_string();
        } else if path.join("foundry.toml").exists()
            || path.join("hardhat.config.js").exists()
            || path.join("hardhat.config.ts").exists()
        {
            lang = "solidity".to_string();
        }

        if lang != "unknown" {
            projects.push(ProjectDescriptor {
                root: path,
                language: lang,
            });
        }
    }

    if projects.is_empty() {
        projects.push(ProjectDescriptor {
            root: ws.to_path_buf(),
            language: "unknown".to_string(),
        });
    }

    projects
}

pub const DEFAULT_ORCHESTRATOR_MODEL: &str = "qwen2.5-coder:7b";

/// Resuelve el modelo solicitado contra la lista de modelos de Ollama disponibles.
/// Si el modelo solicitado coincide exactamente o por prefijo, lo usa directamente.
/// Solo recurre a un fallback si el modelo no está físicamente instalado en Ollama.
pub fn resolve_model_or_fallback(requested: &str, available_models: &[String]) -> String {
    let req = requested.trim();
    if req.is_empty() {
        return DEFAULT_ORCHESTRATOR_MODEL.to_string();
    }

    // 1. Coincidencia exacta
    if let Some(found) = available_models.iter().find(|m| m.as_str() == req) {
        return found.clone();
    }

    // 2. Coincidencia con o sin tag (ej: 'qwen2.5-coder:7b' vs 'qwen2.5-coder' o 'qwen2.5-coder:latest')
    let req_base = req.split(':').next().unwrap_or(req);
    if let Some(found) = available_models.iter().find(|m| {
        let m_base = m.split(':').next().unwrap_or(m.as_str());
        m.as_str() == req
            || m.starts_with(&format!("{}:", req))
            || (m_base == req_base && (m.ends_with(":latest") || !req.contains(':')))
    }) {
        return found.clone();
    }

    // 3. Si coincide prefijo completo
    if let Some(found) = available_models
        .iter()
        .find(|m| m.starts_with(req) || req.starts_with(m.as_str()))
    {
        return found.clone();
    }

    // 4. Fallback de seguridad si el modelo solicitado no existe en Ollama
    if let Some(valid) = available_models
        .iter()
        .find(|m| !m.contains("embed") && (m.contains("coder") || m.contains("qwen")))
    {
        valid.clone()
    } else if let Some(valid) = available_models.iter().find(|m| !m.contains("embed")) {
        valid.clone()
    } else {
        DEFAULT_ORCHESTRATOR_MODEL.to_string()
    }
}

pub async fn run_agent_loop(
    mut user_message: String,
    raw_workspace_path: String,
    _tree_json: String,
    orchestrator_model: String,
    programmer_model: String,
    app_handle: AppHandle,
) -> Result<String, String> {
    // P0-G Fix: Workspace Authority - always canonicalize to absolute paths to prevent '.' bypasses
    let workspace_path = std::path::Path::new(&raw_workspace_path)
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(&raw_workspace_path))
        .to_string_lossy()
        .to_string();

    // PRE-FLIGHT CHECK
    emit_event(
        &app_handle,
        0,
        "Ejecutando validación ambiental (Pre-Flight Check)...",
        "ACTION",
    );
    let available_models = match crate::core::env_check::validate_environment(&workspace_path).await
    {
        Ok(report) => {
            for warning in report.warnings {
                emit_event(&app_handle, 0, &format!("[ENTORNO] {}", warning), "WARNING");
            }
            report.models
        }
        Err(env_errors) => {
            let error_msg = env_errors.join("\n");
            emit_event(
                &app_handle,
                0,
                &format!("[ENV_FAILURE] Fallo pre-vuelo:\n{}", error_msg),
                "FATAL",
            );
            let final_res = FinalResponse {
                status: "ERROR".to_string(),
                respuesta_conversacional: format!("[ENV_FAILURE] No puedo continuar porque el entorno no cumple con los requisitos mínimos:\n{}\n\nPor favor, soluciona esto e intenta de nuevo.", error_msg),
            };
            return Ok(serde_json::to_string(&final_res).unwrap());
        }
    };
    emit_event(&app_handle, 0, "Pre-Flight Check superado.", "SUCCESS");

    // Resolver modelos de forma global respetando estrictamente la selección del usuario.
    let orchestrator_model = resolve_model_or_fallback(&orchestrator_model, &available_models);
    let programmer_model = resolve_model_or_fallback(&programmer_model, &available_models);
    emit_event(
        &app_handle,
        0,
        &format!("⚙️ [CEREBRO GLOBAL ACTIVO] Modelo: {}", orchestrator_model),
        "INFO",
    );
    emit_event(
        &app_handle,
        0,
        &format!(
            "[BUILD] Aura Sentinel {} | core={}",
            env!("CARGO_PKG_VERSION"),
            option_env!("AURA_CORE_SOURCE_FINGERPRINT").unwrap_or("unavailable")
        ),
        "INFO",
    );

    let mut current_context = String::new();

    // ── Inject current workspace state first ──────────────────────────────
    // Always show the LLM what ACTUALLY exists in the workspace right now.
    // This prevents hallucinating a blank-slate project when files already exist.
    {
        let mut existing_files = Vec::new();
        fn scan_workspace_files(dir: &std::path::Path, files: &mut Vec<String>, depth: usize) {
            if depth > 5 {
                return;
            }
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    // Skip node_modules, .git, __pycache__, hidden dirs
                    if name.starts_with('.')
                        || name == "node_modules"
                        || name == "__pycache__"
                        || name == "target"
                    {
                        continue;
                    }
                    if path.is_dir() {
                        scan_workspace_files(&path, files, depth + 1);
                    } else {
                        files.push(path.to_string_lossy().to_string());
                    }
                }
            }
        }
        scan_workspace_files(
            std::path::Path::new(&workspace_path),
            &mut existing_files,
            0,
        );
        if !existing_files.is_empty() {
            let relative_existing: Vec<String> = existing_files
                .iter()
                .map(|f| {
                    f.strip_prefix(&workspace_path)
                        .unwrap_or(f)
                        .trim_start_matches(['/', '\\'])
                        .to_string()
                })
                .collect();
            current_context.push_str(&format!(
                "[ESTADO ACTUAL DEL WORKSPACE] Los siguientes archivos YA EXISTEN en el proyecto. \
                Antes de crear nada, verifica si estos archivos ya cumplen el objetivo:\n{}\n\n",
                relative_existing.join("\n")
            ));
        }

        // ── Auto TOOL_MAPPER: inject dependency graph for multi-file projects ──
        // Count source files (py/js/ts/rs/go) to decide if mapping is worthwhile.
        let source_file_count = existing_files
            .iter()
            .filter(|f| {
                let fl = f.to_lowercase();
                fl.ends_with(".py")
                    || fl.ends_with(".js")
                    || fl.ends_with(".ts")
                    || fl.ends_with(".tsx")
                    || fl.ends_with(".jsx")
                    || fl.ends_with(".rs")
                    || fl.ends_with(".go")
            })
            .count();

        if source_file_count >= 3 {
            emit_event(&app_handle, 0, "🗺️ [AUTO-MAPPER] Proyecto multi-archivo detectado. Generando grafo de dependencias...", "ACTION");
            let graph = crate::core::dependency_mapper::analyze_workspace(&workspace_path);
            let report = crate::core::dependency_mapper::format_graph_report(&graph);
            current_context.push_str(&format!(
                "[AUTO-MAPPER] Grafo de dependencias generado automáticamente para este proyecto.\n\n{}\n\n",
                report
            ));
            emit_event(
                &app_handle,
                0,
                &format!(
                    "🗺️ Grafo listo: {} archivos | {} dependencias",
                    graph.nodes.len(),
                    graph.edges.len()
                ),
                "SUCCESS",
            );
        }
    }

    // Legacy vector RAG removed per architectural directive P1-1.
    // Working memory is maintained strictly via SessionJournal and Observation circuit.
    let mut archivos_editados_historico: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut comandos_ejecutados_historico: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut comandos_exitosos_historico: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut paquetes_instalados_historico: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut architect_used = false;
    let mut tester_attempts = 0;
    let mut tester_success_hits = 0;
    let mut programmer_cooldown_hits = 0;
    let mut programmer_failures = 0u32;
    let mut verifier_failures = 0u32;
    let mut last_failed_verifier_world: Option<u64> = None;
    let mut repeated_verifier_without_change = 0u32;
    let mut last_consultation_audit_world: Option<u64> = None;
    let mut repeated_consultation_audit_without_change = 0u8;
    let mut last_consultation_audit_diagnostic = String::new();
    let mut last_firebase_audit_world: Option<u64> = None;
    let mut repeated_firebase_audit_without_change = 0u8;
    let mut last_firebase_audit_diagnostic = String::new();
    let mut verifier_diagnostic = String::new();
    let mut last_failed_test_run: Option<(String, String)> = None;
    let mut scoped_review_blocked_actions = 0u8;
    let mut journal = crate::core::session_journal::load_journal(&workspace_path);
    let is_continuation_command = crate::core::intent_router::is_resume_command(&user_message);
    let is_contextual_follow_up =
        crate::core::intent_router::is_contextual_follow_up(&user_message);
    let current_turn_instruction =
        crate::core::intent_router::contextual_follow_up_instruction(&user_message)
            .unwrap_or(&user_message)
            .to_string();
    let mut original_prompt_parsed = if let Some(objective) =
        crate::core::intent_router::contextual_follow_up_objective(&user_message)
    {
        objective.trim().to_string()
    } else if let Some(idx) = user_message.find("\n\nGuía de Traducción Técnica") {
        let text = &user_message[..idx];
        text.replace("Petición Original del Usuario: ", "")
            .trim()
            .to_string()
    } else {
        user_message.clone()
    };
    // Resolve a resume request from the journal before classifying its scope;
    // the literal message "continua" does not describe the saved task.
    if is_continuation_command && !journal.objetivo.trim().is_empty() {
        original_prompt_parsed = journal.objetivo.clone();
    }
    let scoped_visual_review_instruction = resolve_scoped_visual_review_instruction(
        &current_turn_instruction,
        &journal.objetivo,
        is_continuation_command,
    );
    let scoped_visual_review = scoped_visual_review_instruction.is_some();
    let mut _no_tests_consecutive = 0u32;
    let mut think_consecutive = 0u32;
    let mut _programmer_consecutive = 0u32; // reserved for future per-tool cooldown
                                            // ── THINK↔PROGRAMMER alternation loop detector ─────────────────────────
                                            // Tracks consecutive steps that are ONLY THINK or PROGRAMMER with no
                                            // TERMINAL, TESTER, or FINISH in between. If this reaches >= 8 steps,
                                            // we force TOOL_TERMINAL to break the loop.
                                            // let mut think_programmer_alternation_count = 0u32; removed for Commit 9
    let mut auditor_consecutive = 0u32;
    let mut mapper_consecutive = 0u32;
    let mut workspace_manager_error_consecutive = 0u32;
    let mut learn_consecutive = 0u32;
    let mut unknown_tool_consecutive = 0u32;

    // ── Mandatory Tool Checklist (Bug 1 fix) ──────────────────────────────────
    // Parse user_message for required tools and enforce them before TOOL_FINISH.
    let browser_interaction_validation_required =
        mission_requires_browser_interaction_verification(&original_prompt_parsed);
    let mandatory_tools_required: std::collections::HashSet<String> = {
        let mut required = std::collections::HashSet::new();
        let msg_upper = original_prompt_parsed.to_uppercase();
        if msg_upper.contains("TOOL_TESTER") {
            required.insert("TOOL_TESTER".to_string());
        }
        if browser_interaction_validation_required {
            required.insert("TOOL_TESTER".to_string());
        }
        if msg_upper.contains("TOOL_VISION_EVALUATOR") {
            required.insert("TOOL_VISION_EVALUATOR".to_string());
        }
        if msg_upper.contains("TOOL_AUDITOR") {
            required.insert("TOOL_AUDITOR".to_string());
        }
        required
    };
    let mut mandatory_tools_executed: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut visual_validation_required =
        mission_requires_visual_verification(&original_prompt_parsed, &workspace_path)
            || scoped_visual_review;
    let mut visual_validation_world_hash: Option<u64> = None;
    let mut browser_interaction_validation_world_hash: Option<u64> = None;
    let mut visual_validation_error: Option<String> = None;
    let mut visual_validation_result: Option<String> = None;
    let mut visual_validation_error_world_hash: Option<u64> = None;
    let mut visual_validation_attempts = 0u32;
    let mut visual_finish_recovery_attempts = 0u32;
    let mut console_runtime_recovery_attempts = 0u32;
    let mut last_local_ui_url: Option<String> = None;
    let mut last_background_task_id: Option<String> = None;
    let mut last_http_server_task_id: Option<String> = None;
    let mut local_http_server_read_attempts = 0u8;
    let mut forced_next_tool: Option<(String, String)> = None;
    let mut pending_user_approval: Option<crate::core::policy::ActionProposal> = None;
    let mut last_stall_recovery_step: u32 = 0; // prevent recovering from the same stall on consecutive steps
    let mut intercept_consecutive: u32 = 0; // track consecutive LLM disobedience
    let mut ask_user_consecutive: u32 = 0; // track consecutive user questions to prevent stalling loops
                                           // FIX-B3: Per-file patch failure counter. When a file accumulates 3+ PATCH_FAILs in a row,
                                           // escalate to full-file overwrite mode instead of retrying failed patches forever.
    let mut _patch_fail_counts: std::collections::HashMap<String, u32> =
        std::collections::HashMap::new();
    // ── Semantic Error Loop Detector — ring buffer of last 7 terminal output hashes ──
    // Populated whenever a TOOL_TERMINAL command produces an error. If identical errors repeat,
    // sanity_monitor escalates to RED and forces TOOL_THINK.
    let mut last_error_hashes: std::collections::VecDeque<u64> =
        std::collections::VecDeque::with_capacity(7);

    let mut retry_tracker = crate::core::error_classifier::RetryTracker::new();
    let agent_workspace = std::sync::Arc::new(std::sync::Mutex::new(
        chronos_vfs::workspace::AgentWorkspace::<chronos_vfs::aura_bridge::AuraAstNode>::new(
            1_048_576,
        )
        .unwrap(),
    ));
    // ── Context Window Tiered Monitor (Devin 2.0 / OSS 2025 Pattern) ────────
    let context_monitor =
        crate::core::context_monitor::ContextMonitor::new(12000, &original_prompt_parsed);

    // ── Multi-Agent Role State Machine ─────────────────────────────────────
    let mut current_role = AgentRole::Planner;
    let mut critic_feedback: Option<String> = None;

    // ── Fase 5: Sanity Monitor state ─────────────────────────────────────────
    let mut tool_history: Vec<String> = Vec::new();
    let mut last_progress_step: u32 = 1;
    let mut programmer_stall_recoveries = 0u8;
    let mut programmer_calls_since_validation = 0u8;

    // ── FASE B: Cached Workspace Tree (RAM invalidation on file modification) ──
    let mut _no_verify_consecutive: u32 = 0;
    // ── Mission Type Classifier ─────────────────────────────────────────────
    let mission_type = if scoped_visual_review {
        MissionType::Execution
    } else {
        classify_mission(&original_prompt_parsed)
    };
    let mut planning_research_failed = false;
    let mission_label = match &mission_type {
        MissionType::Planning => "📐 PLAN INICIAL",
        MissionType::Analysis => "🔍 ANÁLISIS",
        MissionType::Construction => "🏗️ CONSTRUCCIÓN",
        MissionType::Refactor => "♻️ REFACTORING",
        MissionType::Debug => "🐛 DEBUG",
        MissionType::Execution => "⚡ EJECUCIÓN/VERIFICACIÓN",
    };

    if mission_type == MissionType::Execution {
        current_role = AgentRole::Executor;
    }

    // A broad plan that explicitly names Firebase depends on current platform
    // capabilities. Make one bounded research attempt instead of relying on a
    // small local model to remember that it should search.
    if mission_type == MissionType::Planning
        && original_prompt_parsed.to_lowercase().contains("firebase")
    {
        forced_next_tool = Some((
            "TOOL_WEB_SEARCH".to_string(),
            "Investiga una vez documentación oficial actual de Firebase sobre autenticación, Firestore y Hosting. Si la búsqueda falla, continúa indicando que no se pudo verificar.".to_string(),
        ));
    }

    // ── Acceptance Contract ─────────────────────────────────────────────────
    let mut acceptance_contract: Option<String> = None;

    let mut json_error_count = 0;
    let mut consecutive_schema_errors = 0u8;

    // ── Session Journal ────────────────────────────────────────
    // ── Fase 1: Register workspace in global index for auto-resume ────────────
    crate::core::mission_persist::register_workspace(&workspace_path);

    // ── Fase 3: Inject episodic memory context ────────────────────────────────
    let episode_context =
        if should_inject_prior_episodes(is_continuation_command, is_contextual_follow_up) {
            crate::core::episodic_memory::get_episode_context(&workspace_path, 3)
        } else {
            String::new()
        };
    if !episode_context.is_empty() {
        current_context.push_str(&episode_context);
    }

    // ── Arquitectura Cognitiva v4: Inyectar Contexto de Experiencias Previas ──
    let exp_context = crate::core::experience::ExperienceStore::build_experience_context(
        &original_prompt_parsed,
        "",
        &workspace_path,
    );
    if !exp_context.is_empty() {
        current_context.push_str(&exp_context);
    }

    // ── FASE C: Inyectar Lecciones Consolidadas de Proyectos Previos ────────────
    let proactive_lessons = crate::core::memory::get_proactive_lessons(&workspace_path, 3).await;
    if !proactive_lessons.is_empty() {
        current_context.push_str(&proactive_lessons);
    }

    if (is_continuation_command || is_contextual_follow_up) && !journal.objetivo.is_empty() {
        // Retain original mission objective and existing phases!
        journal = crate::core::session_journal::resume_existing_mission(&workspace_path)?;
        original_prompt_parsed = journal.objetivo.clone();
        if !is_contextual_follow_up {
            user_message = journal.objetivo.clone();
        }
        // A contextual edit gets a fresh planner turn with the new instruction.
        // Explicit resume alone restores the previous serialized FSM context.
        if !is_contextual_follow_up {
            journal.interrupted = true;
        }
    } else {
        // Any new user prompt (not an explicit continuation) starts a completely clean session
        journal = crate::core::session_journal::start_new_mission(&workspace_path, &user_message)?;
    }

    // Ensure internal files in workspace are hidden on Windows
    crate::core::hide_workspace_internal_files(&workspace_path);

    // ── MissionRuntime: cognitive governor built AFTER objective is resolved ──
    // This ensures the runtime contract has the correct objective for continuations.
    let runtime_objective = if scoped_visual_review {
        scoped_visual_review_instruction
            .clone()
            .unwrap_or_else(|| current_turn_instruction.clone())
    } else if is_contextual_follow_up {
        format!(
            "{}\n\nInstrucción incremental aprobada por el usuario:\n{}",
            original_prompt_parsed,
            crate::core::intent_router::contextual_follow_up_instruction(&user_message)
                .unwrap_or(&user_message)
        )
    } else {
        original_prompt_parsed.clone()
    };
    let scoped_budget = if scoped_visual_review { 18 } else { 50 };
    let mut runtime = crate::core::mission_runtime::MissionRuntime::new(
        &workspace_path,
        &runtime_objective,
        scoped_budget,
    );

    if scoped_visual_review {
        runtime.contract = crate::core::mission_contract::MissionContract::new(&runtime_objective);
        runtime.contract.add_criterion(
            "AC-REVIEW-TESTS",
            "Pruebas funcionales ya configuradas ejecutadas, si el proyecto dispone de ellas",
            crate::core::mission_contract::VerificationMethod::TestPassed,
            false,
        );
    }

    // Initial contract criteria: every mission must have explicit deliverables & validation
    if runtime.contract.acceptance_criteria.is_empty()
        && runtime.contract.required_evidence.is_empty()
    {
        runtime.contract =
            crate::core::mission_contract::MissionContract::from_objective(&runtime_objective);
    }
    if mission_type == MissionType::Planning {
        for criterion in &mut runtime.contract.acceptance_criteria {
            if matches!(
                criterion.verification,
                crate::core::mission_contract::VerificationMethod::ManualReview
            ) {
                criterion.description =
                    "La hoja de ruta inicial refleja el objetivo y separa supuestos de decisiones pendientes."
                        .to_string();
            }
        }
    }

    match crate::core::managed_verifier::ensure_for_mission(
        &workspace_path,
        &runtime_objective,
        &runtime.contract,
    ) {
        Ok(Some(file)) => emit_event(
            &app_handle,
            0,
            &format!(
                "[VERIFICADOR GESTIONADO] Se creó '{}' con criterios físicos y mensajes descriptivos.",
                file
            ),
            "SUCCESS",
        ),
        Ok(None) => {}
        Err(error) => {
            emit_event(&app_handle, 0, &error, "FATAL");
            return Err(error);
        }
    }

    // ── FINAL-6: Register all tool executors with ToolRegistry ────────────────
    // Register concrete adapters before the mission loop. Every known action passes
    // the runtime authorization gate; executor-backed actions dispatch through this table.
    register_default_tools(
        &mut runtime,
        &workspace_path,
        &original_prompt_parsed,
        Some(app_handle.clone()),
        orchestrator_model.clone(),
        agent_workspace.clone(),
    );
    emit_event(&app_handle, 0, &format!("Herramientas registradas: {}. La disponibilidad de sus dependencias se comprueba al ejecutarlas.", runtime.tool_registry.registered_count()), "INFO");

    // ── AL-v1: Adaptive Learning Setup ───────────────────────────────────────
    let al_project_profile = crate::core::project_profile::ProjectProfile::detect(&workspace_path);
    let _ = runtime.observe_world();
    let al_fingerprint = crate::core::learning::FingerprintBuilder::from_mission_with_world(
        &runtime.contract,
        &al_project_profile,
        runtime.world.as_ref(),
    );
    let al_persistence = crate::core::learning::LearningPersistence::new();
    let al_store: crate::core::learning::SharedExperienceStore = {
        let s = al_persistence.load_experiences(500);
        std::sync::Arc::new(tokio::sync::RwLock::new(s))
    };
    let al_model_stats = al_persistence.load_model_stats();
    let al_strategy_stats = al_persistence.load_strategy_stats();
    let al_router = crate::core::learning::AdaptiveRouter::new(
        al_model_stats,
        al_strategy_stats.clone(),
        al_store.clone(),
        &runtime.mission_id,
    );
    let mut adaptive_text_models = available_models
        .iter()
        .filter(|model| is_adaptive_text_model_candidate(model))
        .cloned()
        .collect::<Vec<_>>();
    if adaptive_text_models.is_empty() {
        adaptive_text_models.push(orchestrator_model.clone());
    }
    let al_recommendation = al_router
        .recommend_with_context(
            &al_fingerprint,
            None,                             // No StateSignature yet at mission start
            Some(runtime.budget_remaining()), // remaining budget from runtime
            &adaptive_text_models,
        )
        .await;
    let recommended_strategy_stats = al_strategy_stats
        .get(al_recommendation.strategy.as_str())
        .cloned()
        .unwrap_or_default();
    let use_learned_strategy = learned_strategy_has_reliable_evidence(
        al_recommendation.confidence,
        &al_recommendation.reason,
        recommended_strategy_stats.attempts,
        recommended_strategy_stats.failures,
    );
    let applied_strategy = if use_learned_strategy {
        al_recommendation.strategy.clone()
    } else {
        crate::core::learning::strategy::StrategyKind::default_for(&al_fingerprint)
    };
    let applied_strategy_confidence = if use_learned_strategy {
        al_recommendation.confidence
    } else {
        0.5
    };
    let strategy_source = if use_learned_strategy {
        "recomendación histórica con confianza suficiente"
    } else {
        "estrategia segura por tipo de tarea"
    };
    let al_engine = crate::core::learning::LearningEngine::with_store(al_store.clone());
    let al_start_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    macro_rules! persist_and_learn_failure {
        ($reason:expr) => {{
            let failure_reason: &str = $reason;
            persist_failed_journal(
                &mut journal,
                &workspace_path,
                &app_handle,
                runtime.current_step(),
                failure_reason,
            );
            record_adaptive_failure(
                &al_engine,
                &al_fingerprint,
                &orchestrator_model,
                &applied_strategy,
                applied_strategy_confidence,
                &runtime,
                al_start_ms,
                failure_reason,
            )
            .await;
        }};
    }
    emit_event(
        &app_handle,
        0,
        &format!(
            "[ADAPTIVE LEARNING] Modelo candidato: {} ({:.0}%); modelo activo: {}. Estrategia aplicada: {:?} ({}, {:.0}%).",
            al_recommendation.model,
            al_recommendation.confidence * 100.0,
            orchestrator_model,
            applied_strategy,
            strategy_source,
            applied_strategy_confidence * 100.0
        ),
        "INFO",
    );

    journal.workspace_path = workspace_path.clone();
    journal.status = "EN_PROGRESO".to_string();
    if !is_contextual_follow_up && !is_continuation_command {
        journal.herramientas_usadas.clear();
        journal.archivos_tocados.clear();
    }
    if let Err(e) = crate::core::session_journal::save_journal(&workspace_path, &journal) {
        emit_event(
            &app_handle,
            runtime.current_step(),
            &format!("[CHECKPOINT FAILED] {}", e),
            "FATAL",
        );
        let final_res = FinalResponse {
            status: "ERROR".to_string(),
            respuesta_conversacional: format!("[PERSISTENCE_FAILURE] Error crítico al persistir el diario de sesión: {}. Misión detenida.", e),
        };
        return Ok(serde_json::to_string(&final_res).unwrap());
    }
    emit_event(
        &app_handle,
        0,
        "[DIARIO] Misión registrada en diario de sesión.",
        "INFO",
    );
    emit_event(
        &app_handle,
        0,
        &format!(
            "[MISIÓN] Tipo clasificado: {} — El agente operará en modo apropiado.",
            mission_label
        ),
        "INFO",
    );

    // ─── COMMAND TRAIL (registro estructurado de pasos) ───
    use crate::core::command_trail::CommandTrail;
    let mut command_trail = CommandTrail::load_or_new(&workspace_path, &original_prompt_parsed);
    command_trail.save(&workspace_path);

    // ─── AUTO-GENERATE RUNNERS (test, build, dev, lint) ───
    // Detectar lenguaje y generar scripts de ejecución internamente en .aura/runtime/runners
    let runners_generated =
        generate_project_runners(&workspace_path, &original_prompt_parsed).await;
    if !runners_generated.is_empty() {
        emit_event(
            &app_handle,
            0,
            &format!(
                "Scripts de ejecución preparados en .aura/runtime/runners (aún no ejecutados): {}",
                runners_generated
                    .iter()
                    .map(|p| p.file_name().unwrap().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            "SUCCESS",
        );
    }

    // =======================================================
    // PESP v2 — Generación del Plan de Fases (Paso 0)
    // =======================================================
    let approved_consultation_task = !scoped_visual_review
        && crate::llm::phase_planner::is_approved_consultation_web_task(&original_prompt_parsed);
    let local_consultation_agenda_task = !scoped_visual_review
        && crate::llm::phase_planner::is_local_consultation_agenda_task(&original_prompt_parsed);
    let local_agenda_plan_needs_recovery =
        needs_local_consultation_agenda_plan_recovery(local_consultation_agenda_task, &journal);
    let prior_plan_needs_recovery =
        needs_approved_plan_recovery(approved_consultation_task, &journal)
            || local_agenda_plan_needs_recovery;
    if !scoped_visual_review
        && (!journal.plan_generado || prior_plan_needs_recovery)
        && (mission_type == MissionType::Construction || mission_type == MissionType::Refactor)
    {
        if prior_plan_needs_recovery {
            let recovery_message = if approved_consultation_task {
                "[PESP RECUPERACIÓN] Actualizando las fases Firebase; se conserva como completada la Fase 1 aprobada y se reabre la integración sin prueba propia."
            } else {
                "[PESP RECUPERACIÓN] El plan anterior omitió funciones del mandato local; se restaura una fase completa y se conservan los archivos existentes."
            };
            emit_event(&app_handle, 0, recovery_message, "WARNING");
        }
        emit_event(
            &app_handle,
            0,
            "🏗️ [ARQUITECTO DE FASES] Analizando tarea para dividirla en fases...",
            "PLANNING",
        );
        let model_for_planner = &orchestrator_model;
        let fases = crate::llm::phase_planner::generate_phase_plan(
            &original_prompt_parsed,
            model_for_planner,
        )
        .await;

        let previous_phases = journal.fases.clone();
        journal.fases = fases.clone();
        if prior_plan_needs_recovery {
            let phase_one_was_complete = previous_phases
                .iter()
                .find(|phase| phase.numero == 1)
                .is_some_and(|phase| phase.estado == "COMPLETADA");
            for phase in &mut journal.fases {
                phase.estado = "PENDIENTE".to_string();
            }
            journal.fase_actual = if phase_one_was_complete && journal.fases.len() > 1 {
                journal.fases[0].estado = "COMPLETADA".to_string();
                1
            } else {
                0
            };
            if let Some(current_phase) = journal.fases.get_mut(journal.fase_actual) {
                current_phase.estado = "EN_PROGRESO".to_string();
            }
        } else {
            journal.fase_actual = 0;
        }
        journal.plan_generado = true;
        if let Err(e) = crate::core::session_journal::save_journal(&workspace_path, &journal) {
            emit_event(
                &app_handle,
                runtime.current_step(),
                &format!("[CHECKPOINT FAILED] {}", e),
                "FATAL",
            );
            let final_res = FinalResponse {
                status: "ERROR".to_string(),
                respuesta_conversacional: format!("[PERSISTENCE_FAILURE] Error crítico al persistir el plan en el diario: {}. Misión detenida.", e),
            };
            return Ok(serde_json::to_string(&final_res).unwrap());
        }

        let plan_desc = fases
            .iter()
            .map(|f| format!("Fase {}: {}", f.numero, f.descripcion))
            .collect::<Vec<_>>()
            .join(" | ");
        emit_event(
            &app_handle,
            0,
            &format!("Plan generado: {}", plan_desc),
            "SUCCESS",
        );
    }
    visual_validation_required |= journal
        .fases
        .get(journal.fase_actual)
        .is_some_and(|phase| phase_has_visual_deliverable(&phase.archivos));
    let mut _task_complexity = crate::llm::router::TaskContext {
        task_type: crate::llm::router::TaskType::GeneralCode,
        language: None,
    };

    // ── SESSION PERSISTENCE (RESTORE IF INTERRUPTED) ──
    if journal.interrupted {
        if let Some(saved_role) = &journal.fsm_role {
            match saved_role.as_str() {
                "Executor" => current_role = AgentRole::Executor,
                "Critic" => current_role = AgentRole::Critic,
                _ => current_role = AgentRole::Planner,
            }
        }
        // H-2: Runtime controls its own state restoration
        if journal.fsm_step > 0 {
            runtime.restore_step(journal.fsm_step);
        }
        // Runtime budget is the SOLE authority — reset to 50 steps for continuation
        runtime.budget = crate::core::step_budget::StepBudget::new(50);
        journal.interrupted = false;
        if let Err(e) = crate::core::session_journal::save_journal(&workspace_path, &journal) {
            emit_event(
                &app_handle,
                runtime.current_step(),
                &format!("[CHECKPOINT FAILED] {}", e),
                "FATAL",
            );
            let final_res = FinalResponse {
                status: "ERROR".to_string(),
                respuesta_conversacional: format!("[PERSISTENCE_FAILURE] Error crítico al actualizar el diario tras restauración: {}. Misión detenida.", e),
            };
            return Ok(serde_json::to_string(&final_res).unwrap());
        }
        if let Some(ctx) = &journal.fsm_context {
            if !ctx.is_empty() {
                current_context = ctx.clone();
            }
        }
        emit_event(&app_handle, runtime.current_step(), &format!("[SESSION RESTORED] Misión retomada exitosamente desde el paso {}. Presupuesto activo extendido a {} pasos.", runtime.current_step(), runtime.budget_remaining()), "SUCCESS");
    }
    if prior_plan_needs_recovery {
        current_role = AgentRole::Planner;
        let phase_summary = journal
            .fases
            .iter()
            .map(|phase| {
                format!(
                    "Fase {} [{}]: {} | archivos: {:?} | criterio: {}",
                    phase.numero,
                    phase.estado,
                    phase.descripcion,
                    phase.archivos,
                    phase.criterio_de_exito
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        current_context = if approved_consultation_task {
            format!(
                "[CONTEXTO RECUPERADO DESDE LA HOJA DE RUTA APROBADA]\nObjetivo original: {}\n\nPlan de fases restaurado:\n{}\n\nLa ejecución anterior perdió el plan y probó herramientas/archivos ajenos al producto. Ignora intentos de Python, Flask, requirements.txt, README.md como entregable y búsquedas repetidas. Conserva los archivos existentes, completa progresivamente solo los archivos de la fase actual y ejecuta su criterio de aceptación. Para la fase MVP usa HTML, CSS, JavaScript, módulos ES y node:test sin dependencias adicionales. La autenticación local es una demostración; no declares seguridad de producción. No avances a Firebase Hosting antes de que npm test pase.\n",
                original_prompt_parsed, phase_summary
            )
        } else {
            format!(
                "[CONTEXTO RESTAURADO CON COBERTURA COMPLETA DEL MANDATO LOCAL]\nObjetivo original: {}\n\nFase restaurada:\n{}\n\nEl plan anterior omitía funciones solicitadas. Conserva los archivos actuales y amplíalos; no vuelvas a crear la estructura básica si ya existe. Implementa todos los flujos enumerados, inicia el servidor HTTP local sin instalar paquetes y valida cada flujo en navegador, escritorio y móvil; revisa la consola. No crees archivos de prueba o configuración vacíos. No uses Firebase ni despliegues. No declares éxito sin evidencia real.\n",
                original_prompt_parsed, phase_summary
            )
        };
        if !episode_context.is_empty() {
            current_context.push_str(&episode_context);
        }
        if !journal.ultimo_estado.trim().is_empty() {
            current_context.push_str(&format!(
                "[DIAGNÓSTICO REAL DEL ÚLTIMO INTENTO FALLIDO — NO REPETIR ESA RUTA]: {}\nConserva los archivos actuales, resuelve este bloqueo y avanza al siguiente criterio verificable.\n\n",
                journal.ultimo_estado.chars().take(1000).collect::<String>()
            ));
        }
        journal.fsm_role = Some("Planner".to_string());
        journal.fsm_context = Some(current_context.clone());
        if let Err(error) = crate::core::session_journal::save_journal(&workspace_path, &journal) {
            let final_res = FinalResponse {
                status: "ERROR".to_string(),
                respuesta_conversacional: format!("[PERSISTENCE_FAILURE] No se pudo guardar la recuperación del plan aprobado: {}", error),
            };
            return Ok(serde_json::to_string(&final_res).unwrap());
        }
        emit_event(
            &app_handle,
            runtime.current_step(),
            "[PESP RECUPERACIÓN] Se descartó el rol/contexto obsoleto y se restauró el plan aprobado sin borrar archivos.",
            "SUCCESS",
        );
    }
    if approved_consultation_task
        && mission_type == MissionType::Construction
        && current_role == AgentRole::Planner
        && forced_next_tool.is_none()
    {
        let phase = journal
            .fases
            .get(journal.fase_actual)
            .map(|phase| format!("Fase {}: {}", phase.numero, phase.descripcion))
            .unwrap_or_else(|| "la fase local aprobada".to_string());
        forced_next_tool = Some((
            "TOOL_THINK".to_string(),
            format!("La hoja de ruta de la aplicación web ya fue aprobada. No investigues de nuevo ni cambies de tecnología. Transfiere el control al Ejecutor para completar {} con los entregables y pruebas definidos.", phase),
        ));
    }
    if is_contextual_follow_up {
        current_context.push_str(&format!(
            "[INSTRUCCIÓN NUEVA SOBRE LA MISIÓN ACTIVA — conservar fases y requisitos no contradictorios]:\n{}\n\n",
            user_message
        ));
    }
    if scoped_visual_review {
        let phase = journal
            .fases
            .get(journal.fase_actual)
            .map(|phase| {
                format!(
                    "Fase {} [{}]: {}",
                    phase.numero, phase.estado, phase.descripcion
                )
            })
            .unwrap_or_else(|| "sin fase activa".to_string());
        current_role = AgentRole::Critic;
        // Start with a bounded, read-only inventory so a weak model does not
        // spend its first turns asking what to inspect or inventing modules.
        forced_next_tool = Some(("TOOL_TERMINAL".to_string(), "dir /b".to_string()));
        current_context = format!(
            "[REVISIÓN VISUAL AISLADA — NO REABRIR OTRAS FASES]\nPetición de esta intervención: {}\nObjetivo persistido: {}\nFase persistida que debe conservarse: {phase}.\nRevisa solo la interfaz ya creada: inspecciona los comandos existentes, ejecuta las pruebas disponibles y levanta la app localmente para que el Crítico capture y evalúe la página real. No edites archivos en una solicitud de revisión; informa los defectos y conserva pendiente la fase original. No ejecutes Firebase ni intentes Hosting salvo que la petición actual lo solicite expresamente.\n",
            scoped_visual_review_instruction
                .as_deref()
                .unwrap_or(&current_turn_instruction),
            journal.objetivo
        );
        emit_event(
            &app_handle,
            runtime.current_step(),
            "[MODO REVISION VISUAL AISLADA] La petición actual inspeccionará la página; el avance Firebase guardado se conserva y no se ejecutará en esta revisión.",
            "INFO",
        );
    }

    while !runtime.is_budget_exhausted() {
        if !runtime.can_continue() {
            let reason = match &runtime.terminal_state {
                Some(crate::core::mission_runtime::RuntimeTerminalState::Failed(msg)) => {
                    msg.clone()
                }
                Some(crate::core::mission_runtime::RuntimeTerminalState::Completed) => {
                    "Misión completada con éxito.".to_string()
                }
                Some(crate::core::mission_runtime::RuntimeTerminalState::WaitingUser(prompt)) => {
                    prompt.clone()
                }
                _ => "El runtime ha reportado un estado final inesperado.".to_string(),
            };
            emit_event(
                &app_handle,
                runtime.current_step(),
                &format!("[RUNTIME TERMINAL] {}", reason),
                "WARNING",
            );
            let final_res = FinalResponse {
                status: "FINISH".to_string(),
                respuesta_conversacional: format!("La misión ha concluido. Razón: {}", reason),
            };
            crate::llm::router::record_model_result(
                &orchestrator_model,
                &crate::llm::router::TaskType::Orchestrator,
                true,
                runtime.current_step(),
            );
            return Ok(serde_json::to_string(&final_res).unwrap());
        }

        // ── H-3: Runtime Integrity Guard — check at each cognitive cycle ──
        let invariant_violations = runtime.check_invariants();
        if !invariant_violations.is_empty() {
            emit_event(
                &app_handle,
                runtime.current_step(),
                &format!(
                    "[INVARIANT] Violaciones detectadas: {:?}",
                    invariant_violations
                ),
                "WARNING",
            );
        }

        // ── Cancellation Check: Interrupción inmediata solicitada por el usuario ──
        if crate::llm::is_agent_cancelled() {
            // H-5: Cancellation is a control event, NOT a tool failure
            let cancel_obs = crate::core::observation::Observation::cancelled(
                "AGENT",
                "Misión detenida por el usuario",
            );
            runtime.record_observation(&cancel_obs);
            emit_event(
                &app_handle,
                runtime.current_step(),
                "🛑 [CANCELADO] Misión detenida por el usuario.",
                "WARNING",
            );
            journal.interrupted = true;
            journal.status = "INTERRUMPIDO".to_string();
            journal.fsm_step = runtime.current_step(); // sync step before saving
            if let Err(e) = crate::core::session_journal::save_journal(&workspace_path, &journal) {
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    &format!("[CHECKPOINT FAILED] {}", e),
                    "FATAL",
                );
            }
            let final_res = FinalResponse {
                status: "FINISH".to_string(),
                respuesta_conversacional: "🛑 **Misión Cancelada**: Has detenido la ejecución del agente. Puedes continuar en cualquier momento con `continua` o enviarle una nueva instrucción.".to_string(),
            };
            return Ok(serde_json::to_string(&final_res).unwrap());
        }

        // ── FASE 1: Mission Checkpoint for Auto-Resume cross-restart ──
        let role_str = match current_role {
            AgentRole::Planner => "Planner",
            AgentRole::Executor => "Executor",
            AgentRole::Critic => "Critic",
        };
        if let Err(e) = crate::core::mission_persist::save_checkpoint(
            &workspace_path,
            &current_context,
            role_str,
            runtime.current_step(),
            Some(&original_prompt_parsed),
        ) {
            emit_event(&app_handle, runtime.current_step(), &e, "FATAL");
        }

        // ── FASE 5: Monitor de Cordura y Salud Cognitiva del Agente ──
        if runtime.current_step() % 3 == 0 || runtime.current_step() == 1 {
            let error_hashes_slice: Vec<u64> = last_error_hashes.iter().copied().collect();
            let report = crate::core::sanity_monitor::check(
                &tool_history,
                current_context.len(),
                json_error_count,
                runtime.current_step(),
                last_progress_step,
                &error_hashes_slice,
            );
            crate::core::sanity_monitor::emit_report(&app_handle, &report);
            if let Some((hint, forced_tool_opt)) =
                crate::core::sanity_monitor::build_correction_hint(&report)
            {
                current_context.push_str(&hint);
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    &format!("[CORDURA] {}", report.recommendation),
                    "WARNING",
                );
                // RED level: mechanically force the tool — don't just inject text
                if let Some(forced_tool_name) = forced_tool_opt {
                    if forced_next_tool.is_none() {
                        forced_next_tool = Some((
                            forced_tool_name.clone(),
                            format!(
                                "[CORDURA-RED] Forzando {} — loop o error semántico detectado.",
                                forced_tool_name
                            ),
                        ));
                    }
                }
            }
        }

        // Count writes across intervening reflections and failed commands too.
        // Otherwise a think/dir/error cycle can evade the consecutive-tool guard.
        let repeated_programmer_calls = programmer_calls_since_validation as usize;
        let can_replace_forced_action = matches!(
            forced_next_tool.as_ref().map(|(tool, _)| tool.as_str()),
            None | Some("TOOL_THINK") | Some("TOOL_PROGRAMMER")
        );
        if repeated_programmer_calls >= MAX_PROGRAMMER_CALLS_WITHOUT_VALIDATION as usize
            && can_replace_forced_action
        {
            if programmer_stall_recoveries >= MAX_PROGRAMMER_STALL_RECOVERIES {
                let message = format!(
                    "NO_PROGRESS_EXHAUSTED: TOOL_PROGRAMMER volvió al ciclo de escritura en {} ocasiones sin cerrar el contrato de aceptación. La misión se detuvo antes de consumir los 50 pasos; los archivos existentes se conservaron para continuar.",
                    programmer_stall_recoveries
                );
                emit_event(&app_handle, runtime.current_step(), &message, "ERROR");
                persist_and_learn_failure!(&message);
                return Ok(serde_json::json!({
                    "status": "INCOMPLETE",
                    "respuesta_conversacional": message
                })
                .to_string());
            }

            let validation_command = programmer_loop_validation_command(
                &runtime.state_anchor.existing_files,
                &runtime.contract,
                &workspace_path,
            );
            let Some(command) = validation_command else {
                let message = "NO_PROGRESS_EXHAUSTED: El agente repitió escrituras sin un comando de verificación disponible. La misión se detuvo antes de consumir los 50 pasos y conservó los archivos; no se declararon pruebas ni resultados.".to_string();
                emit_event(&app_handle, runtime.current_step(), &message, "ERROR");
                persist_and_learn_failure!(&message);
                return Ok(serde_json::json!({
                    "status": "INCOMPLETE",
                    "respuesta_conversacional": message
                })
                .to_string());
            };

            programmer_stall_recoveries = programmer_stall_recoveries.saturating_add(1);
            programmer_calls_since_validation = 0;
            current_role = AgentRole::Critic;
            forced_next_tool = Some((
                "TOOL_TERMINAL".to_string(),
                format!(
                    "Ejecuta ahora la verificación disponible con comando='{}'; no vuelvas a escribir archivos antes de conocer el resultado.",
                    command
                ),
            ));
            current_context.push_str(&format!(
                "[NO-PROGRESS GUARD] Se interrumpió el ciclo de escrituras repetidas. Ejecuta `{command}` y usa su resultado real para decidir el siguiente paso.\n\n"
            ));
            emit_event(
                &app_handle,
                runtime.current_step(),
                &format!(
                    "[NO-PROGRESS GUARD] {} llamadas seguidas a TOOL_PROGRAMMER; forzando validación {} de {}.",
                    repeated_programmer_calls,
                    programmer_stall_recoveries,
                    MAX_PROGRAMMER_STALL_RECOVERIES
                ),
                "WARNING",
            );
        }

        // ── EMERGENCY EXIT: step budget exhausted ────────────────────────
        if runtime.is_budget_exhausted() {
            // ── Evaluate CompletionGate before claiming success at budget limit ──
            let deliverables_ok = validate_workspace(&workspace_path).await.is_ok();
            let completion_ok = match runtime.can_complete() {
                crate::core::completion_gate::CompletionDecision::Complete => true,
                _ => false,
            };

            if deliverables_ok && completion_ok {
                let mut created_files: Vec<String> = Vec::new();
                if let Ok(entries) = std::fs::read_dir(&workspace_path) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_file() {
                            let name = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            if !name.starts_with('.') {
                                created_files.push(name);
                            }
                        }
                    }
                }

                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    "✅ Misión completada. Todos los entregables validados.",
                    "SUCCESS",
                );

                let al_elapsed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0)
                    .saturating_sub(al_start_ms);
                let al_result = crate::core::learning::LearningResult {
                    outcome: crate::core::learning::LearningOutcome::Success,
                    metrics: crate::core::learning::OutcomeMetrics {
                        steps: runtime.steps_taken(),
                        tool_calls: runtime.cognitive_state.metrics.tool_calls,
                        failed_actions: 0,
                        recovery_actions: runtime.recovery_count(),
                        verification_attempts: 1,
                        successful_verifications: 1,
                        elapsed_ms: al_elapsed,
                    },
                    failures: vec![],
                    recovery: None,
                };
                let _ = al_engine
                    .record_outcome(
                        al_fingerprint.clone(),
                        orchestrator_model.clone(),
                        applied_strategy.clone(),
                        al_result,
                        applied_strategy_confidence,
                        runtime.mission_id.clone(),
                        None,
                    )
                    .await;

                let final_res = FinalResponse {
                        status: "FINISH".to_string(),
                        respuesta_conversacional: format!(
                            "### 🛡︠ Misión Completada con Éxito\n\n\
                            Se han implementado y validado todos los componentes del proyecto:\n\
                            {}\n\n\
                            La puerta de finalización confirmó evidencia vigente para todos los criterios requeridos.",
                            created_files.iter().map(|f| format!("- `{}`", f)).collect::<Vec<_>>().join("\n")
                        ),
                    };
                return Ok(serde_json::to_string(&final_res).unwrap());
            } else if deliverables_ok && !completion_ok {
                // Files exist but CompletionGate not satisfied   honest incomplete status
                emit_event(&app_handle, runtime.current_step(), "⠸︠ Presupuesto agotado   entregables presentes pero criterios de misión incompletos.", "WARNING");

                let al_elapsed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0)
                    .saturating_sub(al_start_ms);
                let al_result = crate::core::learning::LearningResult {
                    outcome: crate::core::learning::LearningOutcome::PartialSuccess,
                    metrics: crate::core::learning::OutcomeMetrics {
                        steps: runtime.steps_taken(),
                        tool_calls: runtime.cognitive_state.metrics.tool_calls,
                        failed_actions: runtime.recovery_count(),
                        recovery_actions: runtime.recovery_count(),
                        verification_attempts: 1,
                        successful_verifications: 0,
                        elapsed_ms: al_elapsed,
                    },
                    failures: vec![],
                    recovery: None,
                };
                let _ = al_engine
                    .record_outcome(
                        al_fingerprint.clone(),
                        orchestrator_model.clone(),
                        applied_strategy.clone(),
                        al_result,
                        applied_strategy_confidence,
                        runtime.mission_id.clone(),
                        None,
                    )
                    .await;

                let final_res = FinalResponse {
                        status: "INCOMPLETE".to_string(),
                        respuesta_conversacional: "⚠︠ **Presupuesto agotado**: Los archivos fueron creados pero el agente no pudo verificar el 100% de los criterios de aceptación. Escribe **'continua'** para otorgar otro bloque de pasos.".to_string(),
                    };
                return Ok(serde_json::to_string(&final_res).unwrap());
            }

            // Save state for continuation
            journal.interrupted = true;
            journal.fsm_step = runtime.current_step();
            journal.fsm_role = Some(role_str.to_string());
            journal.fsm_context = Some(current_context.clone());
            if let Err(e) = crate::core::session_journal::save_journal(&workspace_path, &journal) {
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    &format!("[CHECKPOINT FAILED] {}", e),
                    "FATAL",
                );
                let final_res = FinalResponse {
                    status: "ERROR".to_string(),
                    respuesta_conversacional: format!("[PERSISTENCE_FAILURE] Error crítico al guardar checkpoint de pausa: {}. Misión detenida.", e),
                };
                return Ok(serde_json::to_string(&final_res).unwrap());
            }

            let mut workspace_files: Vec<String> = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&workspace_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let name = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        if !name.starts_with('.') && name != "target" {
                            workspace_files.push(name);
                        }
                    }
                }
            }

            let pause_msg = format!(
                "⏸️ **Pausa de Presupuesto Agéntico (Paso {})**\n\n\
                Se ha completado el bloque de {} pasos asignado a este turno. El proyecto sigue en desarrollo activo en el workspace:\n\n\
                **📋 Estado de Fases:**\n{}\n\n\
                **📁 Archivos en Workspace:**\n{}\n\n\
                💡 Escribe **'continua'** para otorgarme otro bloque de 50 pasos y continuar exactamente donde me quedé sin perder progreso.",
                runtime.current_step(), runtime.budget.total_steps,
                if journal.fases.is_empty() {
                    "- Fases en desarrollo activo".to_string()
                } else {
                    journal.fases.iter().map(|f| format!("- Fase {}: {} [{}]", f.numero, f.descripcion, f.estado)).collect::<Vec<_>>().join("\n")
                },
                if workspace_files.is_empty() {
                    "- (Preparando archivos)".to_string()
                } else {
                    workspace_files.iter().map(|f| format!("- `{}`", f)).collect::<Vec<_>>().join("\n")
                }
            );
            emit_event(&app_handle, runtime.current_step(), &pause_msg, "WARNING");

            let final_res = FinalResponse {
                status: "PAUSED".to_string(),
                respuesta_conversacional: pause_msg,
            };
            return Ok(serde_json::to_string(&final_res).unwrap());
        }

        // ── PESP: Inject micro-meta progress status into context ─────────────
        // Tells the LLM exactly where it is in the global project plan every turn.

        // ── PESP: Global Project Plan Banner (Dynamic Turn Context) ─────────
        // Injected into the prompt for the current turn WITHOUT polluting current_context.
        let pesp_banner = if !journal.fases.is_empty() {
            let total = journal.fases.len();
            let progress: String = journal
                .fases
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    let icon = match f.estado.as_str() {
                        "COMPLETADA" => "✅",
                        "EN_PROGRESO" => "🔄",
                        "FALLIDA" => "❌",
                        _ => "⏳",
                    };
                    format!(
                        "  {} [{}/{}] {} → {}",
                        icon,
                        i + 1,
                        total,
                        f.descripcion,
                        f.estado
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let current_f = journal
                .fases
                .get(journal.fase_actual)
                .map(|f| {
                    format!(
                        "(Fase {}/{}) {}\n    Criterio de Éxito: {}",
                        f.numero, total, f.descripcion, f.criterio_de_exito
                    )
                })
                .unwrap_or_else(|| "(todas completadas)".to_string());
            format!(
                "[ESTADO DE FASES DEL PROYECTO — PESP PROTOCOL]\n{}\n\n📍 FASE ACTUAL EN EJECUCIÓN:\n{}\n\n",
                progress,
                current_f
            )
        } else if !journal.micro_metas.is_empty() {
            let total = journal.micro_metas.len();
            let progress: String = journal
                .micro_metas
                .iter()
                .enumerate()
                .map(|(i, mm)| {
                    let icon = match mm.estado.as_str() {
                        "VERIFICADA" => "✅",
                        "COMPLETADA" => "✅",
                        "EN_PROGRESO" => "🔄",
                        _ => "⏳",
                    };
                    format!(
                        "  {} [{}/{}] {} → {}",
                        icon,
                        i + 1,
                        total,
                        mm.descripcion,
                        mm.estado
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let current_mm = journal
                .micro_metas
                .get(journal.micro_meta_actual)
                .map(|mm| mm.descripcion.clone())
                .unwrap_or_else(|| "(todas completadas)".to_string());
            format!(
                "[ESTADO DE MICRO-METAS DEL PROYECTO — PESP PROTOCOL]\n{}\nMICRO-META ACTUAL: [{}/{}] {}\n\n",
                progress,
                journal.micro_meta_actual + 1,
                total,
                current_mm
            )
        } else {
            String::new()
        };

        // ── Observe physical world at turn start to guarantee absolute reality anchor ──
        let _ = runtime.observe_world();

        // ── Context Window Tiered Monitor & Intelligent Compaction (Devin 2.0 / OSS 2025 Pattern) ──
        let (fill_pct, ctx_status) = context_monitor.status(current_context.len());
        if context_monitor.should_compact(current_context.len()) {
            emit_event(&app_handle, runtime.current_step(), &format!("[MEMORIA] Compactando historial activo ({:.0}% del límite interno por tamaño de texto, no por tokens); preservando objetivo y estado...", fill_pct.min(1.0) * 100.0), "INFO");
            let anchor_block = runtime.state_anchor.format_prompt_block();
            current_context = context_monitor.compact_context(&current_context, &anchor_block);
            emit_event(
                &app_handle,
                runtime.current_step(),
                "[MEMORIA] Contexto compactado exitosamente sin pérdida del objetivo.",
                "SUCCESS",
            );
        } else if ctx_status == crate::core::context_monitor::ContextStatus::ApproachingLimit {
            emit_event(
                &app_handle,
                runtime.current_step(),
                &format!(
                    "[MEMORIA] Historial activo al {:.0}% del límite interno por tamaño de texto; no representa tokens del modelo.",
                    fill_pct.min(1.0) * 100.0
                ),
                "INFO",
            );
        }

        // Audit the approved first-phase MVP before spending an orchestrator
        // turn on validation. Existing filenames can belong to a prior failed
        // run and are not proof that the behavior or tests were implemented.
        let consultation_mvp_audit = if approved_consultation_task
            && journal.fase_actual == 0
            && phase_pending_programmer_file(&journal, &workspace_path).is_none()
        {
            Some(crate::llm::phase_planner::audit_consultation_mvp(
                std::path::Path::new(&workspace_path),
            ))
        } else {
            None
        };
        if let Some(audit) = consultation_mvp_audit
            .as_ref()
            .filter(|audit| !audit.issues.is_empty())
        {
            let audit_world = runtime.current_world_hash();
            let diagnostic = audit.issues.join("\n");
            update_consultation_audit_stall_count(
                &mut last_consultation_audit_world,
                &mut repeated_consultation_audit_without_change,
                &mut last_consultation_audit_diagnostic,
                audit_world,
                &diagnostic,
            );
            if repeated_consultation_audit_without_change >= 2 {
                let message = format!(
                    "CONSULTATION_MVP_REPAIR_EXHAUSTED: dos reparaciones no cambiaron el workspace; se conservan los archivos y el diagnóstico. {}",
                    diagnostic
                );
                emit_event(&app_handle, runtime.current_step(), &message, "ERROR");
                persist_and_learn_failure!(&message);
                return Ok(
                    serde_json::json!({"status":"ERROR", "respuesta_conversacional":message})
                        .to_string(),
                );
            }
            verifier_diagnostic = diagnostic.clone();
            current_role = AgentRole::Executor;
            let reason = format!(
                "La auditoría funcional detectó que la Fase 1 sigue incompleta. Corrige estos defectos comprobados sin repetir pruebas: {}. Edita solo {:?}.",
                diagnostic, audit.repair_files
            );
            forced_next_tool = Some(("TOOL_PROGRAMMER".to_string(), reason.clone()));
            current_context.push_str(&format!("[PESP AUDITORÍA FUNCIONAL]: {}\n\n", reason));
            emit_event(
                &app_handle,
                runtime.current_step(),
                &format!(
                    "[PESP AUDITORÍA FUNCIONAL] Fase 1 incompleta; reparación obligatoria de {:?}.",
                    audit.repair_files
                ),
                "WARNING",
            );
        } else if consultation_mvp_audit.is_some() {
            last_consultation_audit_world = None;
            repeated_consultation_audit_without_change = 0;
            if !last_consultation_audit_diagnostic.is_empty()
                && verifier_diagnostic == last_consultation_audit_diagnostic
            {
                verifier_diagnostic.clear();
            }
            last_consultation_audit_diagnostic.clear();
        }

        // Phase 2 has its own deterministic audit and emulator acceptance run.
        // Never let the Phase 1 `npm test` command stand in for Firebase Auth /
        // Firestore integration evidence.
        let consultation_firebase_audit = if approved_consultation_task && journal.fase_actual == 1
        {
            Some(crate::llm::phase_planner::audit_consultation_firebase(
                std::path::Path::new(&workspace_path),
            ))
        } else {
            None
        };
        if let Some(audit) = consultation_firebase_audit.as_ref() {
            if !audit.issues.is_empty() {
                let audit_world = runtime.current_world_hash();
                let diagnostic = audit.issues.join("\n");
                update_consultation_audit_stall_count(
                    &mut last_firebase_audit_world,
                    &mut repeated_firebase_audit_without_change,
                    &mut last_firebase_audit_diagnostic,
                    audit_world,
                    &diagnostic,
                );
                if repeated_firebase_audit_without_change >= 2 {
                    let message = format!(
                        "FIREBASE_INTEGRATION_REPAIR_EXHAUSTED: dos reparaciones no cambiaron los archivos; se conserva la Fase 1 y se detiene antes de Hosting. {}",
                        diagnostic
                    );
                    emit_event(&app_handle, runtime.current_step(), &message, "ERROR");
                    persist_and_learn_failure!(&message);
                    return Ok(
                        serde_json::json!({"status":"ERROR", "respuesta_conversacional":message})
                            .to_string(),
                    );
                }
                verifier_diagnostic = diagnostic.clone();
                current_role = AgentRole::Executor;
                let reason = format!(
                    "La auditoría de Fase 2 detectó una integración Firebase incompleta. Corrige solo los defectos comprobados; usa la configuración demo local y conserva sin cambios la Fase 1: {}. Archivos a reparar: {:?}.",
                    diagnostic, audit.repair_files
                );
                forced_next_tool = Some(("TOOL_PROGRAMMER".to_string(), reason.clone()));
                current_context.push_str(&format!("[PESP AUDITORÍA FIREBASE]: {}\n\n", reason));
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    &format!(
                        "[PESP AUDITORÍA FIREBASE] Fase 2 incompleta; corrección obligatoria de {:?}.",
                        audit.repair_files
                    ),
                    "WARNING",
                );
            } else {
                last_firebase_audit_world = None;
                repeated_firebase_audit_without_change = 0;
                last_firebase_audit_diagnostic.clear();
                let world = runtime.current_world_hash();
                if !successful_command_for_world(
                    &comandos_exitosos_historico,
                    "npm run test:firebase",
                    world,
                ) && forced_next_tool.is_none()
                {
                    current_role = AgentRole::Executor;
                    forced_next_tool = Some((
                        "TOOL_TERMINAL".to_string(),
                        "Ejecuta ahora el criterio obligatorio de la Fase 2: npm run test:firebase. Este comando debe iniciar Auth y Firestore Emulator y ejecutar las pruebas de integración; npm test solo valida la Fase 1.".to_string(),
                    ));
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "[PESP AUDITORÍA FIREBASE] Código conectado; falta evidencia de npm run test:firebase.",
                        "WARNING",
                    );
                } else if forced_next_tool.is_none() {
                    current_role = AgentRole::Critic;
                    forced_next_tool = Some((
                        "TOOL_FINISH".to_string(),
                        "La auditoría Firebase pasó y npm run test:firebase tuvo éxito en este estado del workspace. Evalúa el cierre de Fase 2 y solicita la aprobación normal antes de pasar a Hosting.".to_string(),
                    ));
                }
            }
        }

        let mut forced_override: Option<(String, String)> = None;
        if let Some((forced, override_msg)) = forced_next_tool.take() {
            let intercept_log = format!("[SISTEMA INTERCEPTO] En el turno anterior se decidió forzarte a usar: {}. Razón: {}", forced, override_msg);
            current_context.push_str(&format!("{}\n\n", intercept_log));
            forced_override = Some((forced, override_msg));
        }
        // A failed verifier must be repaired before PESP can select another
        // acceptance run merely because the planned files already exist.
        let forced_test_repair =
            is_forced_test_repair(forced_override.as_ref(), last_failed_test_run.is_some());

        let mut extra_prompt = String::new();
        if let Some((forced, reason)) = &forced_override {
            extra_prompt = format!("\n\nACCIÓN REQUERIDA: {}. Genera los argumentos válidos para esta herramienta. Diagnóstico vigente: {}", forced, reason);
        }
        extra_prompt.push_str(&format!(
            "\n\n[ESTRATEGIA DE EJECUCIÓN APLICADA: {:?}] {}",
            applied_strategy,
            adaptive_strategy_guidance(&applied_strategy)
        ));

        // 🛡️ LIVE WORKSPACE SCAN (Delegated to WorldState & MissionStateAnchor) 🛡️
        // Fix P0: WorldState -> MissionStateAnchor -> Prompt -> LLM single source of truth
        let (live_workspace_context, workspace_is_empty) = {
            let is_empty = runtime.state_anchor.existing_files.is_empty();
            let anchor_block = runtime.state_anchor.format_prompt_block();
            (anchor_block, is_empty)
        };

        // ── Critic → Executor feedback block ──────────────────────────────────
        let critic_feedback_block = if let Some(ref fb) = critic_feedback {
            format!("\n\n[REPORTE DEL CRÍTICO — DEBES CORREGIR ESTOS PROBLEMAS ANTES DE CONTINUAR]:\n{}\n", fb)
        } else {
            String::new()
        };

        // ── Analysis Fast Path: inject into Planner context ─────────────────────────────
        let analysis_fast_path = if mission_type == MissionType::Analysis {
            if workspace_is_empty {
                "

⚡ [MODO ANÁLISIS]: EL WORKSPACE ESTÁ COMPLETAMENTE VACÍO. \
                Si el usuario pide analizar el código local, NO INVENTES UN REPORTE ni crees archivos simulados. \
                Usa TOOL_FINISH inmediatamente indicando: 'El proyecto está vacío, no hay archivos para analizar.'".to_string()
            } else {
                "

⚡ [MODO ANÁLISIS PURO ACTIVADO]: El usuario pidió un análisis. \
                REGLA ESTRICTA: El resultado de tu análisis DEBE guardarse físicamente en un archivo (ej. 'informe_analisis.md') usando TOOL_PROGRAMMER. NO pongas el reporte gigante en la respuesta conversacional. \
                Al terminar, usa TOOL_FINISH e indica la ruta exacta del archivo generado para que el usuario pueda abrirlo.".to_string()
            }
        } else {
            String::new()
        };

        let planning_fast_path = if mission_type == MissionType::Planning {
            "\n\n📐 [MODO PLAN INICIAL]: El usuario pidió una hoja de ruta, no una misión de programación. Cubre explícitamente todos los módulos y restricciones del mandato; no omitas requisitos porque el pedido sea amplio. Divide el MVP en hasta cuatro fases con objetivo, entregables verificables y condición de salida. Respeta el orden solicitado; si se pidió probar localmente antes de desplegar, empieza por estructura y pruebas locales/emuladores y deja Firebase Hosting para una fase posterior. Separa supuestos y decisiones que solo bloquean fases futuras. No pidas el país para comenzar el trabajo local; confirma el país objetivo solo antes de implementar reglas fiscales. El país del usuario no determina automáticamente el mercado del proyecto. Usa TOOL_WEB_SEARCH una vez para documentación oficial actual de Firebase y TOOL_FINISH cuando el plan cubra el encargo."
                .to_string()
        } else {
            String::new()
        };

        let empty_workspace_rule = if mission_type == MissionType::Planning {
            "El workspace vacío es esperado en modo plan inicial. No crees archivos de aplicación; presenta la hoja de ruta."
        } else {
            "Si el workspace no tiene archivos, usa TOOL_PROGRAMMER para crear los entregables del plan. Si existen, ejecuta el criterio de aceptación de la fase o del contrato. Solo añade pruebas si el proyecto las necesita y usa su lenguaje y runner existentes; nunca elijas Python por defecto ni crees un verificador ajeno al stack. No consultes localhost salvo que un servidor haya sido iniciado y confirmado activo. En Windows la terminal usa cmd.exe: no propongas grep ni sintaxis de PowerShell. No reescribas archivos existentes sin un fallo concreto."
        };

        // ── Acceptance Contract injection ────────────────────────────────────────────────
        let mut contract_block = acceptance_contract
            .as_deref()
            .map(|c| format!("[CONTRATO DE ACEPTACION DEL PLANIFICADOR]\n{}\n", c))
            .unwrap_or_default();
        if let Some((depth, guidance)) = console_validation_profile(&original_prompt_parsed) {
            contract_block.push_str(&format!(
                "[PROFUNDIDAD DE VALIDACIÓN CONSOLA: {depth}] {guidance}\n"
            ));
        }

        // Apply the contamination filter before constructing this turn's prompt.
        // Filtering after prompt creation protected only the following turn.
        current_context = sanitize_runtime_context(&current_context, &workspace_path);

        let json_schema = format!("Tu respuesta DEBE ser ÚNICAMENTE un objeto JSON (sin markdown, sin texto extra):\n\
            {{\n\
              \"herramienta\": \"<NOMBRE_HERRAMIENTA>\",\n\
              \"pensamiento\": \"Razonamiento lógico y fáctico de tu decisión\",\n\
              \"comando\": \"<COMANDO_REAL o null>\",\n\
              \"task_id\": null,\n\
              \"url_a_investigar\": null,\n\
              \"archivos_a_editar\": [\"archivo.ext\", \"otro_archivo.ext\"],\n\
              \"ast_nodes\": [{{\"intent\": \"<código>\", \"parent_id\": 0, \"opcode\": 2}}],\n\
              \"respuesta_conversacional\": \"<respuesta o null>\"\n\
            }}\n\
            REGLAS CRITICAS DEL JSON:
            1. 'comando' = UN SOLO comando de shell real. NUNCA prosa/descripción. Ejemplos: 'dir', 'node --check script.js', 'node app.js'. Para un servidor HTTP persistente usa TOOL_BACKGROUND_START; abrir index.html no sustituye el servidor.
            2. 'archivos_a_editar' = Si eliges TOOL_PROGRAMMER, DEBES incluir al menos un nombre de archivo relativo a crear o editar. NUNCA lo dejes vacío [].
            3. {empty_workspace_rule}
            4. El workspace actual es: {ws}. NUNCA uses rutas absolutas de proyectos anteriores ni de otros directorios.",
            empty_workspace_rule = empty_workspace_rule,
            ws = workspace_path);
        let json_schema = if scoped_visual_review {
            format!(
                "{}\n[REGLA PRIORITARIA DE REVISIÓN SOLO LECTURA — invalida cualquier instrucción anterior de reparación]: No uses TOOL_PROGRAMMER ni ninguna herramienta que cree o modifique archivos. Si una prueba falla o detectas un defecto, documéntalo y continúa la inspección segura; no repares automáticamente. Ejecuta solo pruebas ya configuradas. Inicia la aplicación una sola vez con el comando existente, confirma una URL real en los logs, evalúa la página con TOOL_VISION_EVALUATOR y finaliza con hallazgos y límites comprobados. Si falta una dependencia o configuración, reporta el bloqueo sin instalar, crear archivos ni desplegar.\n",
                json_schema
            )
        } else {
            json_schema
        };

        let agent_prompt = match current_role {
            // Planner - Enrutamiento Semántico Avanzado (Zero-Hint Routing)
            AgentRole::Planner => format!(
                "{}[PLANIFICADOR / ZERO-HINT ROUTER]\nObjetivo del Usuario: {}\nWorkspace Actual: {}\n{}\nHistorial de Conversación:\n{}\n\n\
                ERES EL ENRUTADOR PRINCIPAL. Tu misión es analizar la intención del usuario de forma 100% implícita y autónoma, SIN depender de que el usuario mencione herramientas por su nombre.\n\
                HERRAMIENTAS PERMITIDAS (SELECCIONA POR SEMÁNTICA):\n\
                - TOOL_LOGIC_SOLVER: Úsala automáticamente para fórmulas booleanas CNF/SAT y restricciones expresadas como cláusulas. SpectraSAT no resuelve aritmética general, paridad arbitraria ni optimización de grafos; para revisar lógica de código, usa el modo de auditoría semántica y no afirmes que es una prueba SAT.\n\
                - TOOL_SCHEDULER: Usa esto si el usuario pide ejecutar tareas repetitivas o programadas (ej. 'haz esto cada lunes', 'audita cada semana'). Comando: 'cron_expr|descripción'.\n\
                - TOOL_THINK: Si el objetivo requiere crear, escribir, modificar o debugear código fuente, usa esto para transferir el control al EJECUTOR de forma transparente.\n\
                - TOOL_MAPPER: Solo para analizar dependencias en proyectos con múltiples archivos existentes.\n\
                - TOOL_AUDITOR: Para revisiones de seguridad en código fuente existente.\n\
                - TOOL_SEARCH / TOOL_WEB_SEARCH: Si necesitas buscar documentación o investigar información externa.\n\
                - TOOL_ASK_USER: Para pedir clarificaciones si la intención del usuario es completamente ambigua (PROHIBIDO para pedir ayuda con código, sintaxis o tests).\n\
                - TOOL_FINISH: Usa esto ÚNICAMENTE si la tarea ya está 100% completada y verificada (o si reportaste los resultados finales). NUNCA uses esto si aún hay pasos pendientes.\n\
                \nPROHIBIDO: TOOL_WORKSPACE_MANAGER, TOOL_PROGRAMMER, TOOL_TERMINAL, TOOL_CONTAINER (Estas son exclusivas del Ejecutor).\n\
                REGLA DE ZERO-HINT: Nunca le pidas al usuario que especifique la herramienta. Deduce la necesidad matemática o de código y actúa en consecuencia.\n\
                {}{}{}{}",
                pesp_banner, user_message, live_workspace_context, extra_prompt, current_context,
                critic_feedback_block, analysis_fast_path, planning_fast_path, json_schema
            ),
            // Executor - compressed to <200 tokens
            AgentRole::Executor => format!(
                "{}[EJECUTOR] Objetivo: {}\nWorkspace: {}\n{}\nHistorial:\n{}\n\nTOOLS PERMITIDOS: TOOL_PROGRAMMER, TOOL_TERMINAL, TOOL_CONTAINER, TOOL_ASSET_MANAGER, TOOL_BACKGROUND_START, TOOL_BACKGROUND_READ.\n- TOOL_CONTAINER: Comando = 'run/exec/stop/activate_env image/id'. Úsalo para sandbox, testing en Docker/Podman o para activar entornos virtuales (venv, nvm, cargo).\n- TOOL_ENV_MANAGER: *SOLO* para instalar binarios scoop.\nREGLAS: ANTI-STUB (no pass/TODO/funciones vacias). Respeta los entregables de la fase y limita cada escritura a los archivos pendientes o a los que tengan un fallo comprobado. Incluye al menos un archivo en 'archivos_a_editar' cuando uses TOOL_PROGRAMMER. Si ya existe un criterio de aceptación, ejecútalo en lugar de inventar otro. Si faltan pruebas, elige un runner que ya use el proyecto y comprueba comportamiento real; no cambies de lenguaje solo para probar. En un frontend HTML/CSS/JavaScript, usa pruebas JavaScript con Node cuando sean necesarias, no Python. No uses TOOL_TESTER ni TOOL_FINISH. PROHIBIDO usar TOOL_ASK_USER.\nEJEMPLOS TOOL_TERMINAL: Para 'npm install' usa TOOL_TERMINAL con comando='npm install'. NUNCA inventes herramientas como 'NPM INSTALL'. En Windows, TOOL_TERMINAL usa cmd.exe: no uses grep ni sintaxis de PowerShell. No llames a localhost hasta iniciar el servidor con TOOL_BACKGROUND_START y confirmar que quedó activo.\n\n{}{}",
                pesp_banner, user_message, live_workspace_context, extra_prompt, current_context,
                critic_feedback_block, json_schema
            ),
            // Critic - compressed to <200 tokens
            AgentRole::Critic => format!(
                "{}[CRITICO] Objetivo: {}\nWorkspace: {}\n{}\nHistorial:\n{}\n\nTOOLS PERMITIDOS: TOOL_TESTER, TOOL_TERMINAL, TOOL_BACKGROUND_START, TOOL_BACKGROUND_READ, TOOL_VISION_EVALUATOR, TOOL_FINISH, TOOL_ASK_USER.\nREGLAS: Usa TOOL_TESTER/TOOL_TERMINAL para validar. Si hay errores descríbelos con precisión. Solo TOOL_FINISH si todo pasa y los criterios requeridos tienen evidencia. PROHIBIDO usar TOOL_ASK_USER para fallos de tests o código (transfiere a TOOL_PROGRAMMER para corregir el código o el test).\n[REGLA FRONTEND]: Nunca ejecutes HTML con Node ni crees app.test.js para probar document, alert u otros globals del navegador desde Node. En tareas con flujos interactivos solicitados, usa TOOL_TESTER con `comando` que empiece por `BROWSER_TEST:` y un plan JSON de pasos reales (click/fill/select/assert_text/assert_visible/reload/set_viewport/assert_no_console_errors); el runner abre Chrome/Edge local vía DevTools sin instalar paquetes y solo acepta la URL localhost confirmada. Usa Node para pruebas de lógica pura únicamente si el proyecto ya expone esa lógica separada o tiene un entorno DOM configurado. Inicia/carga el frontend con TOOL_BACKGROUND_START, consulta TOOL_BACKGROUND_READ hasta confirmar la URL, prueba los flujos interactivos y evalúa luego la apariencia con TOOL_VISION_EVALUATOR. Una captura no sustituye las pruebas funcionales.\n[REGLA CONSOLA]: Para programas de consola/CLI, compila y ejecuta el artefacto o sus pruebas con entradas representativas. Compara código de salida, stdout/stderr y efectos observables con cada requisito solicitado; compilar o pasar pruebas genéricas no demuestra por sí solo que el comportamiento pedido exista. Si el mandato pide una demostración sencilla, valida el caso básico; si pide varios flujos o casos límite, valida cada uno. No exijas visión para una tarea de consola.\nEn Windows la terminal usa cmd.exe: no propongas grep ni comandos de PowerShell. No llames a localhost salvo que un servidor iniciado mediante TOOL_BACKGROUND_START haya confirmado que está activo.\n\n{}{}",
                pesp_banner, user_message, live_workspace_context, extra_prompt, current_context,
                contract_block, json_schema
            ),
        };

        // El modelo del orquestador respeta la selección global del usuario (ya resuelto en resolve_model_or_fallback)
        let role_label = match current_role {
            AgentRole::Planner => "🧠 PLANIFICADOR",
            AgentRole::Executor => "⚙️ EJECUTOR",
            AgentRole::Critic => "🔬 CRÍTICO",
        };
        let deterministic_decision = forced_override.as_ref().and_then(|(forced, reason)| {
            if mission_type == MissionType::Planning && forced == "TOOL_FINISH" {
                // Let the selected model produce the actual plan instead of treating
                // the router's instruction text as the user's final answer.
                None
            } else {
                deterministic_forced_decision(
                    &workspace_path,
                    forced,
                    reason,
                    &runtime.contract,
                    &runtime.state_anchor.existing_files,
                )
            }
        });
        if deterministic_decision.is_some() {
            emit_event(
                &app_handle,
                runtime.current_step(),
                "[RUNTIME ROUTER] Transición conocida resuelta por reglas locales.",
                "PLANNING",
            );
        } else {
            emit_event(
                &app_handle,
                runtime.current_step(),
                &format!("[{}] Pensando con {}...", role_label, orchestrator_model),
                "PLANNING",
            );
        }

        // ── Fase 5: Sanity Monitor (cada 5 pasos) ──────────────────────────────
        if runtime.current_step() % 5 == 0 {
            let error_hashes_slice2: Vec<u64> = last_error_hashes.iter().copied().collect();
            let sanity = crate::core::sanity_monitor::check(
                &tool_history,
                current_context.len(),
                json_error_count,
                runtime.current_step(),
                last_progress_step,
                &error_hashes_slice2,
            );
            crate::core::sanity_monitor::emit_report(&app_handle, &sanity);
            if let Some((hint2, forced_tool_opt2)) =
                crate::core::sanity_monitor::build_correction_hint(&sanity)
            {
                current_context.push_str(&hint2);
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    &format!(
                        "[⚕️ CORDURA {}] {}",
                        sanity.level,
                        &sanity.recommendation.chars().take(80).collect::<String>()
                    ),
                    "WARNING",
                );
                if let Some(forced_tool_name2) = forced_tool_opt2 {
                    if forced_next_tool.is_none() {
                        forced_next_tool = Some((
                            forced_tool_name2.clone(),
                            format!(
                                "[CORDURA-RED] Forzando {} — loop o error semántico detectado.",
                                forced_tool_name2
                            ),
                        ));
                    }
                }
            }
        }

        // ── Fase 1: Mission Checkpoint (cada 3 pasos) ───────────────────────────
        if runtime.current_step() % 3 == 0 {
            let role_str = format!("{:?}", current_role);
            if let Err(e) = crate::core::mission_persist::save_checkpoint(
                &workspace_path,
                &current_context.chars().take(6000).collect::<String>(),
                &role_str,
                runtime.current_step(),
                Some(&original_prompt_parsed),
            ) {
                emit_event(&app_handle, runtime.current_step(), &e, "FATAL");
            }
        }

        let decision_is_deterministic = deterministic_decision.is_some();
        let mut agent_res = if let Some(value) = deterministic_decision {
            value.to_string()
        } else {
            match call_ollama_with_schema_options(
                &orchestrator_model,
                &agent_prompt,
                action_response_schema(),
                1024,
                0.05,
            )
            .await
            {
                Ok(res) => res,
                Err(e) => {
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!("Error de conexión: {}", e),
                        "ERROR",
                    );
                    return Err(e);
                }
            }
        };

        // Limpiar JSON
        agent_res = agent_res.trim().to_string();
        if agent_res.starts_with("```json") {
            agent_res = agent_res.trim_start_matches("```json").to_string();
        } else if agent_res.starts_with("```") {
            agent_res = agent_res.trim_start_matches("```").to_string();
        }
        if agent_res.ends_with("```") {
            agent_res = agent_res.trim_end_matches("```").to_string();
        }
        agent_res = agent_res.trim().to_string();

        let clean_agent_res = strip_think_tags(agent_res.clone());
        let mut raw_value: serde_json::Value = match crate::core::structured_json::parse_json_object(
            &clean_agent_res,
        ) {
            Ok(v) => {
                if decision_is_deterministic {
                    println!("RUNTIME ROUTER DECISION: {}", agent_res);
                } else {
                    println!("LLM RAW RESPONSE: {}", agent_res);
                }
                json_error_count = 0; // Reset error count on success
                v
            }
            Err(e) => {
                println!("LLM RAW RESPONSE ERROR: {}", agent_res);
                json_error_count += 1;
                if json_error_count >= 2 {
                    emit_event(&app_handle, runtime.current_step(), &format!("[JSON_ACTION_INVALID] La respuesta del modelo siguió fuera del esquema después de reintentar una vez: {}. No se ejecutó ninguna herramienta.", e), "ERROR");
                    let final_res = FinalResponse {
                        status: "ERROR".to_string(),
                        respuesta_conversacional:
                            "No pude leer una decisión segura del modelo. No ejecuté acciones ni cambié archivos; el objetivo y el estado guardado se conservaron para reanudarlo."
                                .to_string(),
                    };
                    crate::llm::router::record_model_result(
                        &orchestrator_model,
                        &crate::llm::router::TaskType::Orchestrator,
                        final_res.status == "FINISH",
                        runtime.current_step(),
                    );
                    return Ok(serde_json::to_string(&final_res).unwrap());
                } else {
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!(
                            "[JSON] La salida no coincide con el objeto esperado (intento {}/2); reintentaré una vez con el esquema estricto.",
                            json_error_count
                        ),
                        "WARNING",
                    );
                    current_context.push_str(&format!("[SISTEMA INTERNO] La respuesta anterior no se pudo interpretar como un objeto JSON completo. Error: {}. No ejecutes ninguna acción hasta devolver los campos requeridos por el esquema.\n\n", e));
                    continue;
                }
            }
        };
        let checklist = raw_value
            .get("checklist_mental")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let mut tool = raw_value
            .get("herramienta")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN")
            .to_uppercase();
        let pensamiento = raw_value
            .get("pensamiento")
            .and_then(|v| v.as_str())
            .unwrap_or("Sin pensamiento")
            .to_string();
        let mut comando = raw_value
            .get("comando")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let requested_task_id = raw_value
            .get("task_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let task_id = match tool.as_str() {
            "TOOL_BACKGROUND_START" => requested_task_id
                .filter(|value| value != "default_task")
                .unwrap_or_else(|| {
                    format!(
                        "aura-{}-{}",
                        runtime.current_step(),
                        runtime.mission_id.chars().take(8).collect::<String>()
                    )
                }),
            "TOOL_BACKGROUND_READ" | "TOOL_BACKGROUND_QUERY" => requested_task_id
                .filter(|value| value != "default_task")
                .or_else(|| last_http_server_task_id.clone())
                .or_else(|| last_background_task_id.clone())
                .unwrap_or_default(),
            _ => requested_task_id.unwrap_or_default(),
        };
        let mut respuesta_conv = raw_value
            .get("respuesta_conversacional")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if scoped_visual_review && scoped_review_forbids_action(&tool, &comando) {
            scoped_review_blocked_actions = scoped_review_blocked_actions.saturating_add(1);
            runtime.record_step();
            let message = format!(
                "[REVIEW_SCOPE_BLOCKED] La revisión es de solo lectura; se bloqueó {} y no se ejecutó. El defecto debe informarse, no repararse automáticamente.",
                tool
            );
            emit_event(&app_handle, runtime.current_step(), &message, "WARNING");
            current_context.push_str(&format!("{}\n\n", message));
            if scoped_review_blocked_actions >= 2 {
                let stopped = "La revisión se detuvo porque el modelo volvió a proponer cambios fuera del alcance de solo lectura. No se ejecutaron esas acciones; la fase original se conserva.";
                emit_event(&app_handle, runtime.current_step(), stopped, "ERROR");
                return Ok(serde_json::json!({
                    "status": "ERROR",
                    "respuesta_conversacional": stopped
                })
                .to_string());
            }
            current_context.push_str(
                "REGLA DE ALCANCE: continúa como revisor. Ejecuta pruebas ya existentes y evalúa la página. No corrijas ni crees archivos.\n\n",
            );
            continue;
        }

        if tool == "TOOL_PROGRAMMER" {
            let explicit_files: Vec<String> = runtime
                .contract
                .acceptance_criteria
                .iter()
                .filter_map(|criterion| {
                    if let crate::core::mission_contract::VerificationMethod::FileExistence(file) =
                        &criterion.verification
                    {
                        Some(file.clone())
                    } else {
                        None
                    }
                })
                .collect();
            if let Some(recovered) = recover_programmer_file_args(
                &mut raw_value,
                &journal,
                &workspace_path,
                &explicit_files,
            ) {
                current_context.push_str(&format!(
                    "[RECUPERACIÓN DE ARGUMENTOS] TOOL_PROGRAMMER omitió archivos o solo nombró archivos internos. Se recuperó el siguiente entregable pendiente: {:?}. Modifica únicamente ese archivo.\n\n",
                    recovered
                ));
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    &format!(
                        "[RECUPERACIÓN] Archivo de programación recuperado desde la fase: {:?}.",
                        recovered
                    ),
                    "INFO",
                );
            }
        }

        // Si el modelo solo quiere responder conversacionalmente (ej. "herramienta": null o "null"),
        // y adjuntó una respuesta para el usuario, enrutar limpiamente como TOOL_FINISH para no entrar en bucle de error.
        if (tool == "UNKNOWN" || tool == "NULL" || tool == "NONE" || tool.is_empty())
            && !respuesta_conv.trim().is_empty()
        {
            tool = "TOOL_FINISH".to_string();
        }

        if mission_type == MissionType::Planning && !planning_tool_allowed(&tool) {
            runtime.record_step();
            emit_event(
                &app_handle,
                runtime.current_step(),
                &format!(
                    "[PLAN INICIAL] Se bloqueó {} para mantener la misión en planificación.",
                    tool
                ),
                "WARNING",
            );
            current_context.push_str(
                "[PLAN INICIAL]: No ejecutes código ni comandos. Devuelve ahora la hoja de ruta con fases, entregables, supuestos y decisiones pendientes mediante TOOL_FINISH.\n\n",
            );
            forced_next_tool = Some((
                "TOOL_FINISH".to_string(),
                "Entrega la hoja de ruta inicial en la respuesta; no crees archivos ni ejecutes comandos."
                    .to_string(),
            ));
            continue;
        }

        if mission_type == MissionType::Planning
            && tool == "TOOL_WEB_SEARCH"
            && tool_history.iter().any(|used| used == "TOOL_WEB_SEARCH")
        {
            runtime.record_step();
            emit_event(
                &app_handle,
                runtime.current_step(),
                "[PLAN INICIAL] Búsqueda web única completada; se bloqueó otra búsqueda para evitar un ciclo.",
                "INFO",
            );
            forced_next_tool = Some((
                "TOOL_FINISH".to_string(),
                "La investigación puntual terminó. Resume el plan inicial y las fuentes consultadas."
                    .to_string(),
            ));
            continue;
        }

        // Strict Schema Requirement: TOOL_TERMINAL requires non-empty 'comando'
        if tool == "TOOL_TERMINAL" && comando.trim().is_empty() {
            consecutive_schema_errors = consecutive_schema_errors.saturating_add(1);
            emit_event(
                &app_handle,
                runtime.current_step(),
                &format!(
                    "[SCHEMA ERROR {}/3] TOOL_TERMINAL requiere un campo 'comando' no vacío.",
                    consecutive_schema_errors
                ),
                "WARNING",
            );
            current_context.push_str("[VALIDACIÓN DE ESQUEMA FALLIDA]: TOOL_TERMINAL requiere un campo 'comando' explícito y no vacío en el JSON. Prohibido omitir el comando.\n\n");
            if consecutive_schema_errors >= 3 {
                let final_res = FinalResponse {
                    status: "ERROR".to_string(),
                    respuesta_conversacional: "Misión detenida: TOOL_TERMINAL omitió el comando requerido en tres decisiones consecutivas. No se ejecutó ningún comando; el estado guardado se conserva.".to_string(),
                };
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    "[SCHEMA ERROR] Misión detenida tras tres decisiones inválidas consecutivas.",
                    "ERROR",
                );
                crate::llm::router::record_model_result(
                    &orchestrator_model,
                    &crate::llm::router::TaskType::Orchestrator,
                    false,
                    runtime.current_step(),
                );
                return Ok(serde_json::to_string(&final_res).unwrap());
            }
            continue;
        }

        // Bloquear TOOL_THINK consecutivo
        if tool == "TOOL_THINK" && runtime.state_anchor.last_tool.as_deref() == Some("TOOL_THINK") {
            emit_event(
                &app_handle,
                runtime.current_step(),
                if mission_type == MissionType::Planning {
                    "[PLAN INICIAL] Se bloqueó una reflexión repetida; se solicita terminar la hoja de ruta."
                } else {
                    "[ANTI-ECHOLALIA] TOOL_THINK consecutivo bloqueado. Se requiere acción física."
                },
                "WARNING",
            );
            if mission_type == MissionType::Planning {
                runtime.record_step();
                current_context.push_str(
                    "[PLAN INICIAL]: Ya reflexionaste sobre la tarea. No hagas código ni otra reflexión. Entrega ahora la hoja de ruta con TOOL_FINISH.\n\n",
                );
                forced_next_tool = Some((
                    "TOOL_FINISH".to_string(),
                    "Completa la hoja de ruta inicial en la respuesta conversacional.".to_string(),
                ));
            } else {
                current_context.push_str("[SISTEMA INTERNO - BLOQUEO DE BUCLE]: Acabas de usar TOOL_THINK en el turno anterior. PROHIBIDO encadenar otro TOOL_THINK consecutivo. Ahora debes ejecutar una herramienta física obligatoriamente (ej. TOOL_PROGRAMMER, TOOL_TERMINAL o TOOL_MAPPER) para avanzar.\n\n");
            }
            continue;
        }

        // ── MissionRuntime: step tracking + stall detection (single source of truth) ──
        runtime.record_step();
        if let Some(stall) = runtime.should_stall_recover(4) {
            // Only force a transition if we haven't just done one recently
            if runtime.current_step() > last_stall_recovery_step + 3 {
                last_stall_recovery_step = runtime.current_step();
                let stall_msg = format!(
                    "[STALL DETECTOR] {:?} detectado. Forzando transición de estrategia.",
                    stall
                );
                emit_event(&app_handle, runtime.current_step(), &stall_msg, "WARNING");
                if mission_type == MissionType::Planning && forced_next_tool.is_none() {
                    forced_next_tool = Some((
                        "TOOL_FINISH".to_string(),
                        "El planificador se estancó. Resume la hoja de ruta disponible y termina."
                            .to_string(),
                    ));
                } else if forced_next_tool.is_none() {
                    // Determine strategy transition based on physical reality from state_anchor
                    let has_existing_files = !runtime.state_anchor.existing_files.is_empty();
                    let runnable_test = runtime
                        .state_anchor
                        .existing_files
                        .iter()
                        .find(|file| {
                            is_test_artifact_path(file)
                                && test_file_has_runnable_cases(&workspace_path, file)
                        })
                        .and_then(|file| {
                            test_command_for_path(file).map(|command| (file.clone(), command))
                        });
                    let has_test_file = runnable_test.is_some();

                    if stall == crate::core::stall_detector::StallType::RepeatedCommand
                        || stall == crate::core::stall_detector::StallType::SameError
                        || stall == crate::core::stall_detector::StallType::RepeatedTool
                    {
                        forced_next_tool = Some((
                        "TOOL_PROGRAMMER".to_string(),
                        "Estancamiento grave: Bucle de herramientas repetidas o errores idénticos. NO repitas la acción. Usa TOOL_PROGRAMMER para corregir el código subyacente.".to_string(),
                    ));
                        current_context.push_str("[TRANSICIÓN FORZADA POR ESTANCAMIENTO GRAVE]: Bucle repetitivo detectado. Tienes PROHIBIDO repetir el comando. Transicionando a TOOL_PROGRAMMER para obligarte a hacer cambios en el código o la estrategia.\n\n");
                    } else if !has_existing_files {
                        forced_next_tool = Some((
                        "TOOL_PROGRAMMER".to_string(),
                        "Estancamiento detectado: El workspace no tiene archivos. Debes crear los archivos principales usando TOOL_PROGRAMMER.".to_string(),
                    ));
                        current_context.push_str("[TRANSICIÓN FORZADA POR ESTANCAMIENTO]: El workspace aún no tiene archivos. Procede de inmediato a crearlos con TOOL_PROGRAMMER.\n\n");
                    } else if !has_test_file {
                        if approved_consultation_task {
                            let next_file =
                                programmer_file_fallback(&journal, &workspace_path, &[]);
                            let phase = journal
                                .fases
                                .get(journal.fase_actual)
                                .map(|phase| format!("Fase {}", phase.numero))
                                .unwrap_or_else(|| "la fase actual".to_string());
                            forced_next_tool = Some((
                            "TOOL_PROGRAMMER".to_string(),
                            format!("Estancamiento detectado. Continúa {} creando el siguiente entregable web pendiente {:?}. Usa HTML, CSS y JavaScript. No generes verify_solution.py, requirements.txt ni código Python; conserva npm test como verificación local de la fase.", phase, next_file),
                        ));
                            current_context.push_str("[TRANSICIÓN FORZADA POR ESTANCAMIENTO]: La misión aprobada es una aplicación web. Continúa con el próximo archivo de su plan JavaScript; no inventes verificadores Python.\n\n");
                        } else {
                            let frontend_sources =
                                runtime.state_anchor.existing_files.iter().any(|file| {
                                    matches!(
                                        std::path::Path::new(file)
                                            .extension()
                                            .and_then(|value| value.to_str())
                                            .map(str::to_ascii_lowercase)
                                            .as_deref(),
                                        Some(
                                            "html"
                                                | "htm"
                                                | "css"
                                                | "js"
                                                | "mjs"
                                                | "cjs"
                                                | "ts"
                                                | "tsx"
                                        )
                                    )
                                });
                            if frontend_sources
                                && mission_requires_visual_verification(
                                    &original_prompt_parsed,
                                    &workspace_path,
                                )
                            {
                                let (next_tool, guidance) = web_validation_recovery_action(
                                    &original_prompt_parsed,
                                    &workspace_path,
                                    last_local_ui_url.as_deref(),
                                    last_http_server_task_id.as_deref(),
                                )
                                .unwrap_or_else(|| (
                                    "TOOL_VISION_EVALUATOR".into(),
                                    "Evalúa la página real del workspace; no inventes pruebas DOM para Node.".into(),
                                ));
                                current_role = AgentRole::Critic;
                                forced_next_tool = Some((next_tool.clone(), guidance.clone()));
                                current_context.push_str(&format!(
                                    "[TRANSICIÓN DE VALIDACIÓN WEB]: No crees un app.test.js para ejecutar código ligado al DOM con Node. Usa la siguiente acción disponible y prueba en el navegador los controles reales: {} — {}\n\n",
                                    next_tool, guidance
                                ));
                            } else {
                                let test_guidance = "Crea pruebas ejecutables con el lenguaje y el runner que ya usa este proyecto. Inspecciona primero sus archivos de configuración; no elijas Python por defecto ni añadas un lenguaje o dependencia solo para verificar.";
                                forced_next_tool = Some((
                                    "TOOL_PROGRAMMER".to_string(),
                                    format!("Estancamiento detectado: el código fuente ya existe, pero no se encontró una prueba ejecutable. {test_guidance}"),
                                ));
                                current_context.push_str(&format!("[TRANSICIÓN FORZADA POR ESTANCAMIENTO]: No se encontró una prueba ejecutable. Usa el runner y el lenguaje del proyecto; no inventes un verificador Python para código de otro lenguaje. {test_guidance}\n\n"));
                            }
                        }
                    } else {
                        if approved_consultation_task {
                            let test_cmd = journal
                                .fases
                                .get(journal.fase_actual)
                                .map(|phase| phase.criterio_de_exito.trim())
                                .filter(|command| !command.is_empty())
                                .unwrap_or("npm test")
                                .to_string();
                            forced_next_tool = Some((
                            "TOOL_TERMINAL".to_string(),
                            format!("Estancamiento detectado en la aplicación web. Ejecuta el criterio de aceptación de la fase: {}.", test_cmd),
                        ));
                            current_context.push_str(&format!(
                            "[TRANSICIÓN FORZADA POR ESTANCAMIENTO]: Ejecuta el criterio del MVP web: {}.\n\n",
                            test_cmd
                        ));
                        } else {
                            let (test_file, test_cmd) =
                                runnable_test.expect("has_test_file is derived from runnable_test");
                            forced_next_tool = Some((
                            "TOOL_TERMINAL".to_string(),
                            format!("Estancamiento detectado: Ejecuta el script de pruebas '{}' usando TOOL_TERMINAL con comando='{}'.", test_file, test_cmd),
                        ));
                            current_context.push_str(&format!("[TRANSICIÓN FORZADA POR ESTANCAMIENTO]: Ejecuta el test existente '{}' para verificar la solución.\n\n", test_cmd));
                        }
                    }
                }
            }
        }

        // A malformed programmer call with no targets is recoverable when the
        // active phase or acceptance contract already names a safe file. Repair
        // the arguments deterministically before schema validation instead of
        // asking the model to repeat the same empty call.
        if tool == "TOOL_PROGRAMMER"
            && raw_value
                .get("archivos_a_editar")
                .and_then(serde_json::Value::as_array)
                .map_or(true, |files| files.is_empty())
        {
            let explicit_files: Vec<String> = runtime
                .contract
                .acceptance_criteria
                .iter()
                .filter_map(|criterion| {
                    if let crate::core::mission_contract::VerificationMethod::FileExistence(file) =
                        &criterion.verification
                    {
                        Some(file.clone())
                    } else {
                        None
                    }
                })
                .collect();
            if let Some(file) =
                schema_recovery_programmer_target(&journal, &workspace_path, &explicit_files)
            {
                if let Some(object) = raw_value.as_object_mut() {
                    object.insert(
                        "archivos_a_editar".into(),
                        serde_json::json!([file.clone()]),
                    );
                    current_context.push_str(&format!(
                        "[RECUPERACIÓN DE ESQUEMA] Se completó el destino omitido con el archivo de la fase/contrato: {}. Continúa con una corrección concreta o completa sus requisitos; no inventes archivos auxiliares.\n\n",
                        file
                    ));
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!(
                            "[RECUPERACIÓN DE ESQUEMA] Destino omitido recuperado: {}.",
                            file
                        ),
                        "INFO",
                    );
                }
            }
        }

        // ── Arquitectura Cognitiva v4: Validación previa de esquema (P1) ──
        if let crate::core::schema_validator::SchemaValidationResult::Invalid(schema_err) =
            crate::core::schema_validator::SchemaValidator::validate_tool_payload(&tool, &raw_value)
        {
            if tool == "TOOL_PROGRAMMER" && approved_consultation_task {
                if let Some(file) = phase_pending_programmer_file(&journal, &workspace_path) {
                    forced_next_tool = Some((
                        "TOOL_PROGRAMMER".to_string(),
                        format!("La fase aprobada requiere crear el siguiente entregable pendiente: {}. Devuelve archivos_a_editar con ese nombre, no vacío, y genera una implementación real en HTML/CSS/JavaScript según corresponda. No uses Python ni edites archivos internos.", file),
                    ));
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!("[RECUPERACIÓN DE ESQUEMA] Se usará el próximo entregable de la fase: {}.", file),
                        "INFO",
                    );
                } else {
                    let command = journal
                        .fases
                        .get(journal.fase_actual)
                        .map(|phase| phase.criterio_de_exito.trim())
                        .filter(|command| !command.is_empty())
                        .unwrap_or("npm test")
                        .to_string();
                    forced_next_tool = Some((
                        "TOOL_TERMINAL".to_string(),
                        format!("Ya existen todos los archivos planificados para la fase. Ejecuta ahora su criterio de aceptación: {}", command),
                    ));
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "[RECUPERACIÓN DE ESQUEMA] Los entregables de la fase existen; se pasa a validarlos.",
                        "INFO",
                    );
                }
                continue;
            }
            consecutive_schema_errors = consecutive_schema_errors.saturating_add(1);
            emit_event(
                &app_handle,
                runtime.current_step(),
                &format!("[SCHEMA ERROR] {}", schema_err),
                "WARNING",
            );
            current_context.push_str(&format!(
                "[VALIDACIÓN DE ESQUEMA FALLIDA]: {}\nCorrige los argumentos del objeto JSON para la herramienta '{}'.\n\n",
                schema_err, tool
            ));
            // Taking the override above must not silently discard it when the model
            // returns malformed arguments for another tool.
            if forced_override.is_some() {
                forced_next_tool = forced_override.clone();
            }
            if consecutive_schema_errors >= 3 {
                let final_res = FinalResponse {
                    status: "ERROR".to_string(),
                    respuesta_conversacional: format!(
                        "Misión detenida: la herramienta '{}' devolvió argumentos inválidos en {} intentos consecutivos. Último error: {}",
                        tool, consecutive_schema_errors, schema_err
                    ),
                };
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    "[SCHEMA ERROR] Misión detenida tras tres decisiones inválidas consecutivas.",
                    "ERROR",
                );
                crate::llm::router::record_model_result(
                    &orchestrator_model,
                    &crate::llm::router::TaskType::Orchestrator,
                    false,
                    runtime.current_step(),
                );
                return Ok(serde_json::to_string(&final_res).unwrap());
            }
            continue;
        }
        consecutive_schema_errors = 0;

        // ── FORCED TOOL VALIDATION ────────────────────────────────────────────
        // If the system has determined the LLM is stuck in a tool-loop,
        // validate its decision against the forced tool constraint.
        if let Some((forced, override_msg)) = &forced_override {
            let allows_programmer_override =
                (forced == "TOOL_TESTER" || forced == "TOOL_VISION_EVALUATOR")
                    && tool == "TOOL_PROGRAMMER";
            if tool != *forced && !allows_programmer_override {
                intercept_consecutive += 1;
                let error_msg = format!(
                    "[INTERCEPT {}/3] Se te ordenó usar '{}' (razón: '{}'). Elegiste '{}'. Corrige tu elección.",
                    intercept_consecutive, forced, override_msg, tool
                );
                current_context.push_str(&format!("{}\n\n", error_msg));
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    &format!(
                        "[INTERCEPT {}/3] LLM desobedeció orden de usar {}",
                        intercept_consecutive, forced
                    ),
                    "WARNING",
                );

                // Terminal recovery has concrete arguments and can be executed directly.
                // Other tools need model-generated arguments; stop explicitly after three
                // disobedient turns instead of hiding the failure in a long loop.
                if intercept_consecutive >= 3
                    || (forced == "TOOL_PROGRAMMER" && intercept_consecutive >= 1)
                {
                    if forced == "TOOL_PROGRAMMER" {
                        let mut repair_files: Vec<String> = std::fs::read_dir(&workspace_path)
                            .ok()
                            .into_iter()
                            .flatten()
                            .flatten()
                            .filter_map(|entry| {
                                let path = entry.path();
                                if path.is_file()
                                    && matches!(
                                        path.extension().and_then(|value| value.to_str()),
                                        Some("html" | "css" | "js" | "ts" | "py" | "rs" | "json")
                                    )
                                {
                                    Some(entry.file_name().to_string_lossy().to_string())
                                } else {
                                    None
                                }
                            })
                            .collect();
                        for criterion in &runtime.contract.acceptance_criteria {
                            if let crate::core::mission_contract::VerificationMethod::FileExistence(file) = &criterion.verification {
                                if !repair_files.contains(file) { repair_files.push(file.clone()); }
                            }
                        }
                        repair_files.sort();
                        repair_files.dedup();
                        let target_model =
                            resolve_model_or_fallback(&programmer_model, &available_models);
                        let proposal = crate::core::policy::ActionProposal {
                            tool: "TOOL_PROGRAMMER".to_string(),
                            arguments: serde_json::json!({
                                "instruccion": format!("Objetivo global: {}\nReparación obligatoria: {}\nCorrige los archivos necesarios y el verificador; no repitas código sin cambios.", original_prompt_parsed, override_msg),
                                "archivos_a_editar": repair_files,
                                "context": format!("DIAGNÓSTICO SEMÁNTICO VIGENTE:\n{}", override_msg),
                                "model": target_model,
                                "repair_attempt": verifier_failures.max(programmer_failures) + 1,
                                "semantic_repair": true
                            }),
                            expected_effect: override_msg.clone(),
                            risk: crate::core::policy::RiskLevel::Safe,
                            world_hash: Some(runtime.current_world_hash()),
                        };
                        emit_event(&app_handle, runtime.current_step(), "[INTERCEPT] Ejecutando la reparación requerida con argumentos determinados por el contrato.", "WARNING");
                        match runtime.execute_action(&proposal).await {
                            Ok(obs)
                                if obs.status
                                    == crate::core::observation::ObservationStatus::Success =>
                            {
                                for file in &obs.files_affected {
                                    let full_path =
                                        std::path::Path::new(&workspace_path).join(file);
                                    let _ = app_handle.emit(
                                        "file-updated",
                                        serde_json::json!({"path": full_path.to_string_lossy()}),
                                    );
                                }
                                current_context.push_str(&format!(
                                    "[REPARACIÓN FORZADA COMPLETADA]: {}\n",
                                    obs.payload
                                ));
                                current_role = AgentRole::Critic;
                                intercept_consecutive = 0;
                                if let Some(command) = runtime.contract.acceptance_criteria.iter().find_map(|criterion| {
                                    if let crate::core::mission_contract::VerificationMethod::SemanticVerification { command } = &criterion.verification { Some(command.clone()) } else { None }
                                }) {
                                    forced_next_tool = Some(("TOOL_TERMINAL".to_string(), format!("ejecutar '{}'.", command)));
                                }
                                continue;
                            }
                            Ok(obs) => {
                                let message = format!("FORCED_PROGRAMMER_FAILED: {}", obs.payload);
                                emit_event(&app_handle, runtime.current_step(), &message, "ERROR");
                                programmer_failures += 1;
                                if programmer_failures >= 3 {
                                    persist_and_learn_failure!(&message);
                                    return Ok(serde_json::json!({"status":"ERROR", "respuesta_conversacional":format!("PROGRAMMER_REPAIR_EXHAUSTED: {} intentos reales fallidos. {}", programmer_failures, message)}).to_string());
                                }
                                current_context.push_str(&format!("\n{}\nEl borrador fue conservado. Corrige únicamente el archivo señalado por el diagnóstico.\n", message));
                                forced_next_tool = Some((
                                    "TOOL_PROGRAMMER".to_string(),
                                    format!("Reparación {} de 3. Corrige el borrador conservado según este diagnóstico exacto: {}", programmer_failures + 1, obs.payload),
                                ));
                                intercept_consecutive = 0;
                                continue;
                            }
                            Err(error) => {
                                let message = format!("FORCED_PROGRAMMER_BLOCKED: {}", error);
                                emit_event(&app_handle, runtime.current_step(), &message, "ERROR");
                                persist_and_learn_failure!(&message);
                                return Ok(serde_json::json!({"status":"ERROR", "respuesta_conversacional":message}).to_string());
                            }
                        }
                    }
                    if forced != "TOOL_TERMINAL" {
                        if forced == "TOOL_TESTER" || forced == "TOOL_VISION_EVALUATOR" {
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                &format!("[INTERCEPT] Modelo no generó llamada válida a {} tras 3 intentos. Continuando con la acción elegida...", forced),
                                "WARNING",
                            );
                            current_context.push_str(&format!(
                                "[INTERCEPT]: No se pudo forzar {}. Continuando con la ejecución del agente.\n\n",
                                forced
                            ));
                            forced_next_tool = None;
                            intercept_consecutive = 0;
                            if forced == "TOOL_TESTER" {
                                browser_interaction_validation_world_hash =
                                    Some(runtime.current_world_hash());
                            } else {
                                visual_validation_world_hash = Some(runtime.current_world_hash());
                            }
                            continue;
                        }
                        let message = format!("FORCED_TOOL_NOT_OBEYED: el modelo no generó una llamada válida a {} después de 3 intentos. Razón: {}", forced, override_msg);
                        emit_event(&app_handle, runtime.current_step(), &message, "ERROR");
                        persist_and_learn_failure!(&message);
                        return Ok(serde_json::json!({"status":"ERROR", "respuesta_conversacional":message}).to_string());
                    }
                    emit_event(&app_handle, runtime.current_step(),
                        "[INTERCEPT] Modelo ignoró la orden 3 veces. Ejecutando acción forzada directamente...", "WARNING");

                    // Determine what to hard-execute based on the forced tool
                    let forced_cmd_to_run = if forced == "TOOL_TERMINAL" {
                        // Run the command from the override_msg if it looks like a shell command,
                        // otherwise default to a safe directory listing
                        if override_msg.contains("type ")
                            || override_msg.contains("dir ")
                            || override_msg.contains("python ")
                            || override_msg.contains("cargo ")
                        {
                            // Extract the command portion after common prefixes
                            let cmd_part = override_msg
                                .split("ejecutar '")
                                .nth(1)
                                .and_then(|s| s.split('\'').next())
                                .or_else(|| {
                                    override_msg
                                        .split("comando: '")
                                        .nth(1)
                                        .and_then(|s| s.split('\'').next())
                                })
                                .or_else(|| override_msg.split("'TOOL_TERMINAL'. ").nth(1))
                                .map(|s| s.trim())
                                .unwrap_or("dir /b");
                            cmd_part.to_string()
                        } else {
                            // Safe fallback: list workspace files so context is enriched
                            format!("dir /b \"{}\"", workspace_path)
                        }
                    } else {
                        // For other forced tools, just inject the reason into context and continue
                        String::new()
                    };

                    if !forced_cmd_to_run.is_empty() {
                        let intercept_proposal = crate::core::policy::ActionProposal {
                            tool: "TOOL_TERMINAL".to_string(),
                            arguments: serde_json::json!({ "comando": forced_cmd_to_run.clone() }),
                            expected_effect: "Interceptor auto-exec after persistent loop"
                                .to_string(),
                            risk: crate::core::policy::PolicyEngine::classify_terminal_command(
                                &forced_cmd_to_run,
                            ),
                            world_hash: Some(runtime.current_world_hash()),
                        };

                        match runtime.execute_action(&intercept_proposal).await {
                            Ok(obs) => {
                                let digest = truncate_chars(&obs.payload, 3000);
                                let auto_msg = format!(
                                    "[INTERCEPTOR AUTO-EXEC] Ejecutó '{}' bajo autorización de runtime.\nResultado:\n{}\n\n",
                                    forced_cmd_to_run, digest
                                );
                                current_context.push_str(&auto_msg);
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    &format!("[INTERCEPTOR EXECUTED] {}", forced_cmd_to_run),
                                    "SUCCESS",
                                );
                            }
                            Err(e) => {
                                current_context.push_str(&format!(
                                    "[INTERCEPTOR AUTO-EXEC BLOQUEADO/FALLIDO]: {}\n\n",
                                    e
                                ));
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    &format!("[INTERCEPTOR ERROR] {}", e),
                                    "ERROR",
                                );
                            }
                        }
                    } else {
                        current_context.push_str(&format!(
                            "[INTERCEPTOR] Forzando abandono de herramienta '{}'. Razón: {}\n\n",
                            tool, override_msg
                        ));
                    }
                    // Reset intercept — the hard-exec counts as having obeyed the intent
                    intercept_consecutive = 0;
                    forced_next_tool = None;
                    // Skip to next iteration — the context is now enriched, model can proceed                    continue;
                } else {
                    // Re-queue the forced tool for the next iteration
                    forced_next_tool = forced_override.clone();
                }
                continue;
            } else {
                // LLM obeyed — reset counter
                intercept_consecutive = 0;
            }
        }

        // ── ROLE HARD LOCKS (FSM ENFORCEMENT) ─────────────────────────────────
        let is_forced_and_obeyed = forced_override.as_ref().map_or(false, |(f, _)| f == &tool);
        if !is_forced_and_obeyed && current_role == AgentRole::Planner {
            if tool == "TOOL_PROGRAMMER"
                || tool == "TOOL_TERMINAL"
                || tool == "TOOL_BACKGROUND_START"
                || tool == "TOOL_GIT"
            {
                current_role = AgentRole::Executor;
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    &format!(
                        "[FSM] Planificador -> Ejecutor: Transición automática para ejecutar {}.",
                        tool
                    ),
                    "INFO",
                );
            } else if [
                "TOOL_TESTER",
                "TOOL_BACKGROUND_READ",
                "TOOL_BACKGROUND_QUERY",
                "TOOL_BACKGROUND_KILL",
                "TOOL_ENV_MANAGER",
                "TOOL_ASSET_MANAGER",
                "TOOL_VISION_EVALUATOR",
                "TOOL_WORKSPACE_MANAGER",
            ]
            .contains(&tool.as_str())
            {
                let error_msg = format!(
                    "[ACCESO DENEGADO]: Eres el Planificador. No tienes permiso para usar {}. \
                    Tu rol es SOLO diseñar la arquitectura. \
                    NUNCA borres archivos existentes. \
                    Usa TOOL_THINK para transferir el control al Ejecutor cuando estés listo.",
                    tool
                );
                current_context.push_str(&format!("{}\n\n", error_msg));
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    &format!("[FSM LOCK] Planificador intentó usar {}", tool),
                    "WARNING",
                );
                continue;
            }
        } else if !is_forced_and_obeyed && current_role == AgentRole::Executor {
            if tool == "TOOL_FINISH" {
                current_role = AgentRole::Critic;
                emit_event(&app_handle, runtime.current_step(), "[FSM] EJECUTOR -> CRÍTICO: Implementación concluida. Transfiriendo al Crítico para validación final y cierre.", "INFO");
            } else if [
                "TOOL_TESTER",
                "TOOL_VISION_EVALUATOR",
                "TOOL_MAPPER",
                "TOOL_AST_INJECT",
                "TOOL_ASK_USER",
            ]
            .contains(&tool.as_str())
            {
                let error_msg = format!("[ACCESO DENEGADO]: Eres el Ejecutor. No tienes permiso para usar {}. Tu rol es escribir código directamente con TOOL_PROGRAMMER o comandos con TOOL_TERMINAL. Prohibido preguntar o pedir aclaraciones en fase de ejecución.", tool);
                current_context.push_str(&format!("{}\n\n", error_msg));
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    &format!("[FSM LOCK] Ejecutor intentó usar {}", tool),
                    "WARNING",
                );
                continue;
            }
        } else if !is_forced_and_obeyed && current_role == AgentRole::Critic {
            if ["TOOL_PROGRAMMER", "TOOL_MAPPER", "TOOL_AST_INJECT"].contains(&tool.as_str()) {
                if scoped_visual_review {
                    current_context.push_str(
                        "[REVISIÓN DE SOLO LECTURA]: La petición actual no autorizó cambios de código. No uses herramientas de escritura; entrega los hallazgos y el estado real de las pruebas.\n\n",
                    );
                    forced_next_tool = Some((
                        "TOOL_FINISH".to_string(),
                        "Termina el informe de revisión visual sin modificar archivos ni cerrar la fase original.".to_string(),
                    ));
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "[ALCANCE] Escritura bloqueada durante una revisión visual de solo lectura.",
                        "WARNING",
                    );
                    continue;
                }
                // FIX: If the Critic wants to fix code, we gracefully auto-transition to Executor
                // instead of throwing an angry [ACCESO DENEGADO] and forcing a loop.
                current_role = AgentRole::Executor;
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    &format!(
                        "[FSM] Critico -> Ejecutor: Transicion automatica para usar {}.",
                        tool
                    ),
                    "INFO",
                );
                // Let it fall through and execute normally as an Executor!
            }
        }

        let url = raw_value
            .get("url_a_investigar")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if respuesta_conv.is_empty() {
            respuesta_conv = conversational_response(raw_value.get("respuesta_conversacional"));
        }

        let mut archivos_vec = Vec::new();
        if let Some(arr) = raw_value
            .get("archivos_a_editar")
            .and_then(|v| v.as_array())
        {
            for item in arr {
                if let Some(s) = item.as_str() {
                    // ── Workspace Contamination Sanitizer (Fix 3) ──────────────────────
                    // The LLM sometimes injects paths from a previous workspace
                    // (e.g. "proxy-stack-windows") or absolute paths outside the current
                    // workspace. Normalize them to bare filenames so TOOL_PROGRAMMER
                    // always writes into the active workspace.
                    let normalized = {
                        let p = std::path::Path::new(s);
                        // If it's an absolute path AND doesn't start with current workspace,
                        // extract only the filename part.
                        if p.is_absolute() {
                            let inside_workspace = s.starts_with(&workspace_path)
                                || s.replace('/', "\\").starts_with(&workspace_path)
                                || s.replace('\\', "/")
                                    .starts_with(&workspace_path.replace('\\', "/"));
                            if inside_workspace {
                                // Strip workspace prefix → relative path
                                let stripped = s
                                    .trim_start_matches(&workspace_path)
                                    .trim_start_matches('/')
                                    .trim_start_matches('\\');
                                stripped.to_string()
                            } else {
                                // Foreign workspace — keep only the filename
                                p.file_name()
                                    .map(|f| f.to_string_lossy().to_string())
                                    .unwrap_or_else(|| s.to_string())
                            }
                        } else {
                            // Already relative — use as-is
                            s.to_string()
                        }
                    };
                    if !normalized.is_empty() {
                        archivos_vec.push(normalized);
                    }
                }
            }
        }

        if tool == "TOOL_PROGRAMMER" {
            let explicit_contract_files: Vec<String> = runtime
                .contract
                .acceptance_criteria
                .iter()
                .filter_map(|criterion| {
                    if let crate::core::mission_contract::VerificationMethod::FileExistence(file) =
                        &criterion.verification
                    {
                        Some(file.clone())
                    } else {
                        None
                    }
                })
                .collect();
            let missing_explicit_files: Vec<String> = explicit_contract_files
                .iter()
                .filter(|file| !std::path::Path::new(&workspace_path).join(file).is_file())
                .cloned()
                .collect();
            let starts_explicit_small_build = !missing_explicit_files.is_empty()
                && explicit_contract_files.len() <= 3
                && verifier_diagnostic.is_empty();
            if starts_explicit_small_build {
                // The user named the deliverables. Keep a small first transaction and
                // place CSS/JS inline when only one HTML file was requested.
                archivos_vec = missing_explicit_files;
            }
            for file in explicit_contract_files {
                if !std::path::Path::new(&workspace_path).join(&file).is_file()
                    && !archivos_vec.contains(&file)
                {
                    archivos_vec.push(file);
                }
            }
            if !verifier_diagnostic.is_empty() {
                if let Ok(entries) = std::fs::read_dir(&workspace_path) {
                    let mut related: Vec<String> = entries
                        .flatten()
                        .filter_map(|entry| {
                            let path = entry.path();
                            if path.is_file()
                                && matches!(
                                    path.extension().and_then(|v| v.to_str()),
                                    Some("html" | "css" | "js" | "ts" | "py")
                                )
                            {
                                let file = entry.file_name().to_string_lossy().to_string();
                                let is_verifier =
                                    file.starts_with("verify_") || file.starts_with("test_");
                                if is_verifier && !verifier_diagnostic.contains(&file) {
                                    None
                                } else {
                                    Some(file)
                                }
                            } else {
                                None
                            }
                        })
                        .collect();
                    related.sort();
                    for file in related {
                        if !archivos_vec.contains(&file) {
                            archivos_vec.push(file);
                        }
                    }
                }
            }
            archivos_vec.retain(|file| !is_managed_verifier(&workspace_path, file));
            if approved_consultation_task {
                if let Some(next_file) = phase_pending_programmer_file(&journal, &workspace_path) {
                    let model_targets = archivos_vec.clone();
                    archivos_vec = vec![next_file.clone()];
                    if model_targets != archivos_vec {
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &format!(
                                "[PESP OBJETIVO DE ARCHIVO] El modelo propuso {:?}; se limita la escritura al siguiente entregable pendiente {}.",
                                model_targets, next_file
                            ),
                            "INFO",
                        );
                    }
                    if let Some(object) = raw_value.as_object_mut() {
                        object.insert("archivos_a_editar".into(), serde_json::json!(archivos_vec));
                    }
                } else {
                    if let Some(audit) = consultation_mvp_audit
                        .as_ref()
                        .filter(|audit| !audit.issues.is_empty())
                    {
                        apply_consultation_mvp_audit_override(
                            &mut tool,
                            &mut current_role,
                            &mut comando,
                            &mut archivos_vec,
                            &mut raw_value,
                            &mut verifier_diagnostic,
                            audit,
                        );
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &format!(
                                "[PESP AUDITORÍA FUNCIONAL] Los archivos existen, pero la Fase 1 aún está incompleta: {}. Se corrigen {:?}.",
                                audit.issues.join(" | "), archivos_vec
                            ),
                            "WARNING",
                        );
                    } else if forced_test_repair {
                        if let Some((failed_command, failure_output)) = &last_failed_test_run {
                            apply_forced_test_repair_override(
                                &mut tool,
                                &mut current_role,
                                &mut comando,
                                &mut archivos_vec,
                                &mut raw_value,
                                &mut verifier_diagnostic,
                                failed_command,
                                failure_output,
                            );
                        }
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "[PESP REPARACIÓN] Se conserva TOOL_PROGRAMMER: una prueba fallida tiene prioridad sobre repetir la validación por presencia de archivos.",
                            "WARNING",
                        );
                    } else if forced_override
                        .as_ref()
                        .is_some_and(|(forced, _)| forced == "TOOL_FINISH")
                    {
                        // A successful phase verifier can leave only the explicit
                        // user review pending. Preserve that finish request instead
                        // of replacing it with the already-passing test command.
                        tool = "TOOL_FINISH".to_string();
                        current_role = AgentRole::Critic;
                        comando.clear();
                        archivos_vec.clear();
                        if let Some(object) = raw_value.as_object_mut() {
                            object.insert("herramienta".into(), serde_json::json!("TOOL_FINISH"));
                            object.insert("comando".into(), serde_json::Value::Null);
                            object.insert("archivos_a_editar".into(), serde_json::json!([]));
                        }
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "[PESP] Se conserva TOOL_FINISH para completar la revisión manual, sin repetir la prueba aprobada.",
                            "INFO",
                        );
                    } else {
                        let command = journal
                            .fases
                            .get(journal.fase_actual)
                            .map(|phase| phase.criterio_de_exito.trim())
                            .filter(|command| !command.is_empty())
                            .unwrap_or("npm test")
                            .to_string();
                        tool = "TOOL_TERMINAL".to_string();
                        comando = command.clone();
                        archivos_vec.clear();
                        if let Some(object) = raw_value.as_object_mut() {
                            object.insert("comando".into(), serde_json::json!(command));
                        }
                        emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "[PESP OBJETIVO DE ARCHIVO] Todos los archivos de la fase existen; se valida el criterio de aceptación en vez de reescribir documentación o historial.",
                        "INFO",
                    );
                    }
                }
            }
        }

        if tool == "TOOL_PROGRAMMER" && approved_consultation_task && journal.fase_actual == 1 {
            if let Some(audit) = consultation_firebase_audit
                .as_ref()
                .filter(|audit| !audit.issues.is_empty())
            {
                if apply_consultation_firebase_audit_override(
                    &mut tool,
                    &mut current_role,
                    &mut comando,
                    &mut archivos_vec,
                    &mut raw_value,
                    &mut verifier_diagnostic,
                    audit,
                ) {
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!(
                            "[PESP AUDITORÍA FIREBASE] Escritura limitada al defecto comprobado: {:?}.",
                            archivos_vec
                        ),
                        "WARNING",
                    );
                }
            }
        }

        // The audit must also override a model that selected THINK or TERMINAL
        // after all expected filenames appeared. File presence alone is not an
        // acceptance check for this multi-module MVP.
        if tool != "TOOL_PROGRAMMER"
            && approved_consultation_task
            && journal.fase_actual == 0
            && phase_pending_programmer_file(&journal, &workspace_path).is_none()
        {
            if let Some(audit) = consultation_mvp_audit.as_ref() {
                if apply_consultation_mvp_audit_override(
                    &mut tool,
                    &mut current_role,
                    &mut comando,
                    &mut archivos_vec,
                    &mut raw_value,
                    &mut verifier_diagnostic,
                    audit,
                ) {
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!(
                            "[PESP AUDITORÍA FUNCIONAL] Los archivos existen, pero la Fase 1 aún está incompleta: {}. Se corrigen {:?}.",
                            audit.issues.join(" | "), archivos_vec
                        ),
                        "WARNING",
                    );
                }
            }
        }

        let mut ast_nodes_vec = Vec::new();
        if let Some(arr) = raw_value.get("ast_nodes").and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(obj) = item.as_object() {
                    let intent = obj
                        .get("intent")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let parent_id = obj.get("parent_id").and_then(|v| v.as_u64()).unwrap_or(0);
                    let opcode = obj.get("opcode").and_then(|v| v.as_u64()).unwrap_or(2) as u8;
                    ast_nodes_vec.push((intent, parent_id, opcode));
                }
            }
        }

        if !checklist.is_empty() {
            emit_event(
                &app_handle,
                runtime.current_step(),
                &format!("Resumen Factual: {}", checklist),
                "PLANNING",
            );
        }

        // FORCED TOOL OVERRIDE REMOVED. Validation happens above.

        emit_event(
            &app_handle,
            runtime.current_step(),
            &format!("Decisión: {} - {}", tool, pensamiento),
            "DECISION",
        );
        current_context.push_str(&format!(
            "--- PASO {} ---\nAcción: {}\nArchivos objetivo: {:?}\n",
            runtime.current_step(),
            tool,
            archivos_vec
        ));

        // ── Journal: update per-step ─────────────────────────────────────
        crate::core::session_journal::update_journal(
            &mut journal,
            runtime.current_step(),
            &format!(
                "[PASO {}] {} - {}",
                runtime.current_step(),
                tool,
                pensamiento
            ),
            &tool,
            &archivos_vec,
            &workspace_path,
        );
        if let Err(e) = crate::core::session_journal::save_journal(&workspace_path, &journal) {
            emit_event(
                &app_handle,
                runtime.current_step(),
                &format!("[CHECKPOINT FAILED] {}", e),
                "FATAL",
            );
            let final_res = FinalResponse {
                status: "ERROR".to_string(),
                respuesta_conversacional: format!("[PERSISTENCE_FAILURE] Error crítico al actualizar diario de sesión en paso {}: {}. Misión abortada.", runtime.current_step(), e),
            };
            return Ok(serde_json::to_string(&final_res).unwrap());
        }

        // Registrar herramienta en el historial del Monitor de Cordura
        tool_history.push(tool.clone());
        if tool == "TOOL_PROGRAMMER" {
            programmer_calls_since_validation = programmer_calls_since_validation.saturating_add(1);
        }
        if tool_history.len() > 15 {
            tool_history.remove(0);
        }

        // Reset loop counters
        if tool != "TOOL_THINK" {
            think_consecutive = 0;
        }
        if tool != "TOOL_PROGRAMMER" {
            _programmer_consecutive = 0;
        }
        if tool != "TOOL_AUDITOR" {
            auditor_consecutive = 0;
        }
        if tool != "TOOL_MAPPER" {
            mapper_consecutive = 0;
        }
        if tool != "TOOL_LEARN" {
            learn_consecutive = 0;
        }
        if tool != "TOOL_ASK_USER" {
            ask_user_consecutive = 0;
        }
        // THINK↔PROGRAMMER alternation counter: only resets when NEITHER THINK nor PROGRAMMER
        // 🛡️ Commit 9: Alternation lock delegated to RecoveryEngine 🛡️
        // } removed for Commit 9

        // ── Arquitectura Cognitiva v4: Autorización por PolicyEngine ──
        let action_proposal = crate::core::policy::ActionProposal {
            tool: tool.clone(),
            arguments: raw_value.clone(),
            expected_effect: pensamiento.clone(),
            risk: match tool.as_str() {
                "TOOL_TERMINAL" | "TOOL_BACKGROUND_START" => {
                    crate::core::policy::PolicyEngine::classify_terminal_command(&comando)
                }
                "TOOL_GIT" => crate::core::policy::PolicyEngine::classify_terminal_command(
                    &format!("git {}", comando),
                ),
                _ => crate::core::policy::RiskLevel::Safe,
            },
            world_hash: Some(runtime.current_world_hash()),
        };

        // Some specialized branches still perform work directly in agent.rs.
        // They must pass the same registration, schema, budget and policy gate
        // as tools dispatched through MissionRuntime.
        if crate::core::tool_registry::ToolRegistry::is_known(&tool) {
            if let Err(auth_error) = runtime.authorize_action(&action_proposal) {
                current_context.push_str(&format!(
                    "[ACCIÓN BLOQUEADA ANTES DE EJECUTAR]: {}\n\n",
                    auth_error
                ));
                let needs_user = auth_error.starts_with("POLICY_REQUIRE_USER");
                if needs_user {
                    pending_user_approval = Some(action_proposal.clone());
                }
                let terminal_failure = auth_error.starts_with("TOOL_UNREGISTERED")
                    || auth_error.starts_with("BUDGET_EXHAUSTED")
                    || auth_error.starts_with("POLICY_DENY");
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    &format!("[ACTION GATE] {}", auth_error),
                    if needs_user { "WARNING" } else { "ERROR" },
                );
                if terminal_failure {
                    if auth_error.starts_with("BUDGET_EXHAUSTED") {
                        let reason = format!(
                            "BUDGET_EXHAUSTED: {}",
                            auth_error.trim_start_matches("BUDGET_EXHAUSTED:").trim()
                        );
                        persist_and_learn_failure!(&reason);
                    } else {
                        let reason = format!("MISSION_GATE_FAILURE: {}", auth_error);
                        persist_and_learn_failure!(&reason);
                    }
                    let final_res = FinalResponse {
                        status: "ERROR".to_string(),
                        respuesta_conversacional: format!(
                            "La acción se detuvo antes de ejecutarse: {}",
                            auth_error
                        ),
                    };
                    return Ok(serde_json::to_string(&final_res).unwrap());
                }
                let is_failed_build_verification = auth_error.starts_with("RECOVERY_BARRIER")
                    && last_failed_test_run.is_some()
                    && matches!(
                        mission_type,
                        MissionType::Construction | MissionType::Refactor | MissionType::Debug
                    );
                let next_tool = if needs_user {
                    "TOOL_ASK_USER"
                } else if is_failed_build_verification {
                    "TOOL_PROGRAMMER"
                } else {
                    "TOOL_THINK"
                };
                let test_repair_reason = last_failed_test_run
                    .as_ref()
                    .map(|(failed_command, output)| {
                        let specific_guidance =
                            if output.to_ascii_lowercase().contains("jest is not defined") {
                                "El proyecto ejecuta Node.js `node --test`; Jest no está instalado ni debe añadirse por este error. Reemplaza `jest.fn()` por un doble de almacenamiento local o las utilidades de `node:test`, y alinea las pruebas con las claves y el formato reales de app.js."
                            } else {
                                "Corrige el archivo y la implementación señalados por la salida, conserva el runner y las dependencias del proyecto y no repitas el comando sin cambios."
                            };
                        format!(
                            "La barrera bloqueó una acción repetida después de que fallara el verificador `{}`. Pasa directamente a TOOL_PROGRAMMER y repara el defecto real antes de volver a probar. Último error (resumen): {}. {}",
                            failed_command,
                            output.chars().take(1200).collect::<String>(),
                            specific_guidance
                        )
                    });
                let approval_question = if needs_user {
                    let command = action_proposal
                        .arguments
                        .get("comando")
                        .or_else(|| action_proposal.arguments.get("command"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    format!(
                        "¿Autorizas ejecutar exactamente la acción '{}'{}? Responde 'Autorizar esta acción' o 'Cancelar'. Motivo: {}",
                        action_proposal.tool,
                        if command.is_empty() { String::new() } else { format!(" con el comando `{}`", command) },
                        auth_error
                    )
                } else if next_tool == "TOOL_PROGRAMMER" {
                    current_role = AgentRole::Executor;
                    test_repair_reason.unwrap_or_else(|| {
                        format!(
                            "La acción repetida fue bloqueada: {}. Corrige la causa del fallo con TOOL_PROGRAMMER; no repitas la misma acción.",
                            auth_error
                        )
                    })
                } else {
                    format!(
                        "La acción no se ejecutó. Cambia la estrategia según este bloqueo: {}",
                        auth_error
                    )
                };
                forced_next_tool = Some((next_tool.to_string(), approval_question));
                continue;
            }
        }

        match tool.as_str() {
            "TOOL_TERMINAL" => {
                let cmd_lower = comando.to_lowercase();
                if cmd_lower.contains("firebase") && cmd_lower.contains("deploy") {
                    if let Err(reason) = crate::llm::phase_planner::firebase_deploy_preflight(
                        std::path::Path::new(&workspace_path),
                        &comando,
                    ) {
                        let message = format!(
                            "{} No se ejecutó Firebase Hosting. Configura el proyecto explícitamente en este workspace y vuelve a continuar.",
                            reason
                        );
                        emit_event(&app_handle, runtime.current_step(), &message, "ERROR");
                        persist_and_learn_failure!(&message);
                        return Ok(serde_json::json!({
                            "status": "ERROR",
                            "respuesta_conversacional": message
                        })
                        .to_string());
                    }
                }
                if is_long_running_server_command(&cmd_lower)
                    || (mission_requires_local_http_server(&original_prompt_parsed)
                        && is_local_html_open_command(&comando))
                {
                    let res_msg = "[SISTEMA INTERNO]: Para servir una aplicación web usa TOOL_BACKGROUND_START. Abrir `index.html` con `start` solo produce una URL file:// y no satisface un requisito de servidor HTTP local. En una página estática sin dependencias, ejecuta `python -m http.server 8000 --bind 127.0.0.1` en segundo plano; después lee sus logs y confirma la URL localhost antes de abrirla o evaluarla. [SISTEMA: Redirigiendo a TOOL_BACKGROUND_START...]";
                    current_context.push_str(&format!("{}\n\n", res_msg));
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "Servidor web bloqueado en TOOL_TERMINAL",
                        "WARNING",
                    );
                    programmer_cooldown_hits = 0;
                    forced_next_tool = Some(("TOOL_BACKGROUND_START".to_string(), comando.clone()));
                } else if comando.trim().is_empty() {
                    // Track consecutive empties — after 2, force a specific action
                    let empty_key = "__EMPTY_CMD__".to_string();
                    let empty_count = comandos_ejecutados_historico
                        .iter()
                        .filter(|c| *c == &empty_key)
                        .count();
                    comandos_ejecutados_historico.insert(empty_key);

                    if empty_count >= 2 && current_role == AgentRole::Critic {
                        // Critic keeps sending empty commands — force VISION_EVALUATOR
                        let msg =
                            "[SISTEMA]: Has enviado TOOL_TERMINAL sin comando 3 veces seguidas. \
                            ACCIÓN FORZADA: Debes usar TOOL_VISION_EVALUATOR ahora para verificar \
                            la UI, o TOOL_FINISH si ya terminaste.";
                        current_context.push_str(&format!("{}\n\n", msg));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "[SISTEMA] Comando vacío repetido — forzando TOOL_VISION_EVALUATOR",
                            "WARNING",
                        );
                        forced_next_tool = Some((
                            "TOOL_VISION_EVALUATOR".to_string(),
                            "Verifica visualmente la UI del proyecto creado".to_string(),
                        ));
                    } else {
                        let res_msg = format!(
                            "Error: El campo 'comando' está vacío. Debes especificar qué ejecutar. \
                            Ejemplos válidos para este paso: 'dir' para listar archivos, 'node --check script.js' para revisar sintaxis y 'node script.js' para ejecutar JS. Para un servidor HTTP usa TOOL_BACKGROUND_START. \
                            Intento vacío #{}/3 — al tercero se forzará TOOL_VISION_EVALUATOR.",
                            empty_count + 1
                        );
                        current_context.push_str(&format!("{}\n\n", res_msg));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &format!("Comando vacío ({}/3)", empty_count + 1),
                            "ERROR",
                        );
                    }
                    programmer_cooldown_hits = 0;
                } else if !is_forced_and_obeyed
                    && comandos_ejecutados_historico.contains(&format!(
                        "{}|{}",
                        comando.trim().to_lowercase(),
                        runtime.current_world_hash()
                    ))
                {
                    let res_msg = "[SISTEMA INTERNO]: Bucle detectado. Estás repitiendo exactamente el mismo comando. Si falló anteriormente, usa TOOL_PROGRAMMER o TOOL_AUDITOR para arreglar el código. Si ya tuvo éxito y solo estabas probando, la tarea está lista: usa TOOL_FINISH obligatoriamente.";
                    let command_key = format!(
                        "{}|{}",
                        comando.trim().to_lowercase(),
                        runtime.current_world_hash()
                    );
                    let repeated_command_succeeded =
                        comandos_exitosos_historico.contains(&command_key);
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "Comando repetido interceptado",
                        "WARNING",
                    );
                    if repeated_command_succeeded {
                        match runtime.can_complete() {
                            crate::core::completion_gate::CompletionDecision::Complete => {
                                current_role = AgentRole::Critic;
                                forced_next_tool = Some((
                                    "TOOL_FINISH".to_string(),
                                    "El comando de aceptación ya terminó con éxito y el CompletionGate confirma evidencia vigente. Cierra ahora sin volver a ejecutar pruebas.".to_string(),
                                ));
                                current_context.push_str(
                                    "[RECUPERACIÓN DE COMANDO REPETIDO]: La verificación ya pasó y el contrato está completo; se cierra la misión sin repetirla.\n\n",
                                );
                            }
                            crate::core::completion_gate::CompletionDecision::Incomplete(
                                missing,
                            ) => {
                                if approved_consultation_task
                                    && only_deliverables_review_pending(&missing)
                                {
                                    let reason = format!(
                                        "El criterio técnico ya pasó y solo falta revisión manual de los entregables ({}). No vuelvas a ejecutar la prueba ni reescribas código; usa TOOL_FINISH para mostrar la revisión de fase al usuario.",
                                        missing.join("; ")
                                    );
                                    current_role = AgentRole::Critic;
                                    forced_next_tool =
                                        Some(("TOOL_FINISH".to_string(), reason.clone()));
                                    current_context.push_str(&format!(
                                        "[RECUPERACIÓN DE COMANDO REPETIDO]: {}\n\n",
                                        reason
                                    ));
                                } else {
                                    let reason = format!(
                                        "El comando '{}' ya terminó con éxito y no debe repetirse. El CompletionGate aún marca estos criterios como pendientes:\n- {}\nCompleta o corrige el código y sus pruebas antes de volver a validar.",
                                        comando,
                                        missing.join("\n- ")
                                    );
                                    current_role = AgentRole::Executor;
                                    forced_next_tool =
                                        Some(("TOOL_PROGRAMMER".to_string(), reason.clone()));
                                    current_context.push_str(&format!(
                                        "[RECUPERACIÓN DE COMANDO REPETIDO]: {}\n\n",
                                        reason
                                    ));
                                }
                            }
                            crate::core::completion_gate::CompletionDecision::Blocked(reasons) => {
                                let reason = format!(
                                    "La verificación ya pasó, pero la misión está bloqueada por restricciones del contrato: {}. No repitas el comando.",
                                    reasons.join("; ")
                                );
                                current_context
                                    .push_str(&format!("[VERIFICACIÓN BLOQUEADA]: {}\n\n", reason));
                                emit_event(&app_handle, runtime.current_step(), &reason, "ERROR");
                                let final_res = FinalResponse {
                                    status: "ERROR".to_string(),
                                    respuesta_conversacional: reason,
                                };
                                return Ok(serde_json::to_string(&final_res).unwrap());
                            }
                        }
                    } else if current_role == AgentRole::Critic {
                        let msg = "[SISTEMA INTERNO]: Bucle de terminal detectado en el Crítico. Estás repitiendo el mismo comando de validación. Debes replantear tu estrategia de validación o solicitar correcciones con TOOL_PROGRAMMER.";
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "[SISTEMA] Bucle de Terminal en Crítico -> Forzando Replan",
                            "WARNING",
                        );
                        forced_next_tool = Some((
                            "TOOL_THINK".to_string(),
                            "Bucle detectado. Replantea tu estrategia de validación.".to_string(),
                        ));
                        current_context.push_str(&format!("{}\n\n", msg));
                    } else if current_role == AgentRole::Executor {
                        // Executor stuck repeating informational commands (dir, ls, etc.)
                        let cmd_lower = comando.to_lowercase();
                        let is_info_cmd = cmd_lower.trim() == "dir"
                            || cmd_lower.trim() == "ls"
                            || cmd_lower.trim() == "ls -la"
                            || cmd_lower.trim() == "dir /b";
                        if is_info_cmd {
                            let msg = "[SISTEMA INTERNO]: El Ejecutor está repitiendo un comando informacional (dir/ls). Esto indica estancamiento (STALL). PROHIBIDO repetir dir/ls sin cambios. Procede a escribir o probar código.";
                            emit_event(&app_handle, runtime.current_step(), "[SISTEMA] Ejecutor en loop informacional -> Forzando Acción Operativa", "WARNING");
                            let has_files = !runtime.state_anchor.existing_files.is_empty();
                            if !has_files {
                                forced_next_tool = Some(("TOOL_PROGRAMMER".to_string(), "El workspace está vacío. Crea los archivos requeridos usando TOOL_PROGRAMMER.".to_string()));
                            } else {
                                let existing_test = runtime
                                    .state_anchor
                                    .existing_files
                                    .iter()
                                    .find(|file| {
                                        is_test_artifact_path(file)
                                            && test_file_has_runnable_cases(&workspace_path, file)
                                    })
                                    .and_then(|file| {
                                        test_command_for_path(file)
                                            .map(|command| (file.clone(), command))
                                    });
                                if existing_test.is_none() {
                                    let frontend_sources =
                                        runtime.state_anchor.existing_files.iter().any(|file| {
                                            matches!(
                                                std::path::Path::new(file)
                                                    .extension()
                                                    .and_then(|value| value.to_str())
                                                    .map(str::to_ascii_lowercase)
                                                    .as_deref(),
                                                Some(
                                                    "html"
                                                        | "htm"
                                                        | "css"
                                                        | "js"
                                                        | "mjs"
                                                        | "cjs"
                                                        | "ts"
                                                        | "tsx"
                                                )
                                            )
                                        });
                                    if frontend_sources
                                        && mission_requires_visual_verification(
                                            &original_prompt_parsed,
                                            &workspace_path,
                                        )
                                    {
                                        if let Some((next_tool, guidance)) =
                                            web_validation_recovery_action(
                                                &original_prompt_parsed,
                                                &workspace_path,
                                                last_local_ui_url.as_deref(),
                                                last_http_server_task_id.as_deref(),
                                            )
                                        {
                                            current_role = AgentRole::Critic;
                                            forced_next_tool = Some((next_tool, guidance));
                                        }
                                    } else {
                                        let test_guidance = "Crea pruebas con el runner y el lenguaje ya usados por el proyecto; no supongas Python ni inventes dependencias.";
                                        forced_next_tool = Some((
                                            "TOOL_PROGRAMMER".to_string(),
                                            test_guidance.to_string(),
                                        ));
                                    }
                                } else {
                                    let (_test_file, test_cmd) = existing_test.unwrap();
                                    forced_next_tool =
                                        Some(("TOOL_TERMINAL".to_string(), test_cmd));
                                }
                            }
                            current_context.push_str(&format!("{}\n\n", msg));
                        } else {
                            current_context.push_str(&format!("{}\n\n", res_msg));
                        }
                    } else {
                        current_context.push_str(&format!("{}\n\n", res_msg));
                    }
                } else {
                    let cmd_lower = comando.to_lowercase();
                    if is_long_running_server_command(&cmd_lower)
                        || (mission_requires_local_http_server(&original_prompt_parsed)
                            && is_local_html_open_command(&comando))
                    {
                        let res_msg = "[SISTEMA INTERNO]: Para servir una aplicación web usa TOOL_BACKGROUND_START. Abrir `index.html` con `start` solo produce una URL file:// y no satisface un requisito de servidor HTTP local. En una página estática sin dependencias, ejecuta `python -m http.server 8000 --bind 127.0.0.1` en segundo plano; después lee sus logs y confirma la URL localhost antes de abrirla o evaluarla. [SISTEMA: Redirigiendo a TOOL_BACKGROUND_START...]";
                        current_context.push_str(&format!("{}\n\n", res_msg));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "Servidor web bloqueado en TOOL_TERMINAL",
                            "WARNING",
                        );
                        programmer_cooldown_hits = 0;
                        forced_next_tool =
                            Some(("TOOL_BACKGROUND_START".to_string(), comando.clone()));
                    } else {
                        programmer_cooldown_hits = 0;
                        comandos_ejecutados_historico.insert(format!(
                            "{}|{}",
                            comando.trim().to_lowercase(),
                            runtime.current_world_hash()
                        ));

                        // ── Pre-check: validate interpreter target extensions (P1) ────────────
                        if let Err(target_err) =
                            crate::core::schema_validator::validate_interpreter_target(&comando)
                        {
                            let warn_msg = format!("[SISTEMA]: {}", target_err);
                            current_context.push_str(&format!(
                                "Resultado TOOL_TERMINAL Error: {}\n\n",
                                warn_msg
                            ));
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                &format!("[PRE-CHECK] {}", target_err),
                                "ERROR",
                            );
                            forced_next_tool = Some((
                                "TOOL_PROGRAMMER".to_string(),
                                format!("Error de destino de intérprete: {}. Corrige la estrategia o crea un script adecuado.", target_err),
                            ));
                            continue;
                        }

                        // ── Pre-check: verify the script file exists before running it ─────────
                        let cmd_lower_check = comando.to_lowercase();
                        let script_file = if ["node ", "node.exe ", "nodejs "]
                            .iter()
                            .any(|prefix| cmd_lower_check.starts_with(prefix))
                        {
                            node_script_argument(&comando)
                        } else {
                            python_script_argument(&comando)
                        };
                        if let Some(script) = script_file {
                            if !script.is_empty() {
                                let script_path =
                                    std::path::Path::new(&workspace_path).join(&script);
                                if !script_path.exists() {
                                    let warn_msg = format!(
                                        "[SISTEMA]: El archivo '{}' NO existe en el workspace. No puedes ejecutar un archivo que no existe.\n\
                                        Primero crea el archivo con TOOL_PROGRAMMER, o verifica los archivos disponibles con TOOL_TERMINAL (dir).",
                                        script
                                    );
                                    current_context.push_str(&format!("{}\n\n", warn_msg));
                                    emit_event(
                                        &app_handle,
                                        runtime.current_step(),
                                        &format!("[PRE-CHECK] Archivo no encontrado: {}", script),
                                        "ERROR",
                                    );
                                    forced_next_tool = Some((
                                        "TOOL_PROGRAMMER".to_string(),
                                        format!("El archivo '{}' no existe en el workspace. Debes crear el archivo usando TOOL_PROGRAMMER antes de ejecutarlo.", script)
                                    ));
                                    continue;
                                }
                            }
                        }

                        // ── P1: Keep terminal actions atomic so errors remain attributable ──
                        if comando.contains("&&") {
                            let warn_msg = "[SISTEMA]: TOOL_TERMINAL ejecuta un solo comando por acción para que cada resultado pueda comprobarse. Divide la secuencia en acciones separadas y continúa solo después de revisar la salida.";
                            current_context.push_str(&format!(
                                "Resultado TOOL_TERMINAL Error: {}\n\n",
                                warn_msg
                            ));
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                "[PRE-CHECK] Comando concatenado rechazado",
                                "ERROR",
                            );
                            continue;
                        }

                        // ── P1: Pre-check Cargo.toml exists for cargo commands ─────────────
                        if cmd_lower_check.starts_with("cargo ") {
                            let cargo_toml_path =
                                std::path::Path::new(&workspace_path).join("Cargo.toml");
                            if !cargo_toml_path.exists() {
                                let warn_msg = "[SISTEMA]: PROJECT STRUCTURE INVALID: No se encontró `Cargo.toml`. Debes inicializar el proyecto Rust (ej. `cargo init` o crear el archivo) antes de poder usar comandos de cargo.";
                                current_context.push_str(&format!(
                                    "Resultado TOOL_TERMINAL Error: {}\n\n",
                                    warn_msg
                                ));
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    "[PRE-CHECK] Cargo.toml faltante",
                                    "ERROR",
                                );
                                forced_next_tool = Some((
                                    "TOOL_PROGRAMMER".to_string(),
                                    "No existe Cargo.toml. Crea Cargo.toml o inicializa el proyecto antes de usar cargo.".to_string()
                                ));
                                continue;
                            }
                        }

                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &format!("Ejecutando en terminal: {}", comando),
                            "ACTION",
                        );
                        // ── Route through Runtime Gateway (P0 fix) ───────────────────────────
                        // execute_action() = authorize_action (already passed) + ToolRegistry.dispatch()
                        // The TOOL_TERMINAL executor is registered above and calls execute_terminal_command().
                        let mut repair_decision = None;
                        let unified_res = match runtime.execute_action(&action_proposal).await {
                            Ok(obs)
                                if obs.status
                                    == crate::core::observation::ObservationStatus::Error =>
                            {
                                repair_decision = runtime.handle_observation(&obs);
                                Err(obs.payload)
                            }
                            Ok(obs) => Ok(obs),
                            Err(e) => Err(e),
                        };

                        // P0-D Fix: Typed Repair Transition
                        if let Some(crate::core::recovery::RecoveryDecision::RepairCriteria {
                            criteria,
                            recommended_tool,
                        }) = repair_decision.clone()
                        {
                            let failed_world = runtime.current_world_hash();
                            if last_failed_verifier_world != Some(failed_world) {
                                verifier_failures += 1;
                                last_failed_verifier_world = Some(failed_world);
                                repeated_verifier_without_change = 0;
                            } else {
                                repeated_verifier_without_change += 1;
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    "[SEMANTIC_VERIFICATION] El workspace no cambió desde el último fallo; este reintento no consume una reparación.",
                                    "WARNING",
                                );
                                if repeated_verifier_without_change >= 2 {
                                    let message = "VERIFIER_NO_PROGRESS: dos reparaciones consecutivas no produjeron ningún cambio físico. Se detuvo la misión para evitar un bucle.".to_string();
                                    emit_event(
                                        &app_handle,
                                        runtime.current_step(),
                                        &message,
                                        "ERROR",
                                    );
                                    persist_and_learn_failure!(&message);
                                    return Ok(serde_json::json!({"status":"ERROR", "respuesta_conversacional":message}).to_string());
                                }
                            }
                            let focused_criteria = focused_repair_criteria(&criteria, 2);
                            verifier_diagnostic = focused_criteria.join("\n");
                            current_context.push_str(&format!(
                                "\nVerificación fallida, diagnóstico real: {}\n",
                                verifier_diagnostic
                            ));
                            if verifier_failures >= 4 {
                                let message = format!("VERIFIER_REPAIR_EXHAUSTED: La verificación inicial y tres reparaciones reales fallaron. Archivos conservados. {}", verifier_diagnostic);
                                emit_event(&app_handle, runtime.current_step(), &message, "ERROR");
                                persist_and_learn_failure!(&message);
                                return Ok(serde_json::json!({"status":"ERROR", "respuesta_conversacional":message}).to_string());
                            }
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                &format!(
                                    "[SEMANTIC_VERIFICATION] Verificador falló. Transición a {}.",
                                    recommended_tool
                                ),
                                "WARNING",
                            );
                            current_role = AgentRole::Executor;
                            forced_next_tool = Some((
                                recommended_tool.clone(),
                                format!("Corrige los siguientes criterios semánticos que fallaron:\n- {}", focused_criteria.join("\n- "))
                            ));
                            continue; // Break the execution loop and transition instantly to repairing
                        }

                        match unified_res {
                            Ok(observation) => {
                                // execute_action() already records world snapshots before/after internally
                                let out = observation.payload.clone();
                                let _world_hash_before = observation.state_hash_before.unwrap_or(0);
                                if observation.exit_code == Some(0) {
                                    if let Some(url) = extract_local_ui_url(&out) {
                                        last_local_ui_url = Some(url.clone());
                                        emit_event(
                                            &app_handle,
                                            runtime.current_step(),
                                            &format!("[UI LOCAL] URL de vista detectada en la salida real: {url}"),
                                            "INFO",
                                        );
                                    }
                                }

                                // ── Package-install amnesia fix ──────────────────────────────────────
                                // If the command was a package install (pip install X, npm install X),
                                // unblock ALL previously-failed python/node script commands so they can
                                // be retried now that the missing dependency is installed.
                                let cmd_lower = comando.to_lowercase();
                                let is_pkg_install = cmd_lower.starts_with("pip install")
                                    || cmd_lower.starts_with("pip3 install")
                                    || cmd_lower.starts_with("npm install")
                                    || cmd_lower.starts_with("npm i ");
                                if is_pkg_install {
                                    comandos_ejecutados_historico.retain(|c| {
                                        let cl = c.to_lowercase();
                                        !cl.starts_with("python") && !cl.starts_with("node")
                                    });
                                    current_context.push_str("[SISTEMA: Librería instalada correctamente. Los comandos de ejecución de scripts que fallaron antes por dependencias faltantes han sido desbloqueados y pueden reintentarse ahora.]\n\n");
                                }
                                let digested_out = digest_terminal_output(&out, 2500);
                                // ── MissionRuntime: record successful observation with real world data ──

                                current_context.push_str(&format!("\n[RESULTADO REAL DE TERMINAL] Comando: {}\nExit code: {:?}\nSalida: {}\n", comando, observation.exit_code, digested_out));
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    &format!(
                                        "[TERMINAL] {} | exit={:?} | archivos cambiados={}\n{}",
                                        comando,
                                        observation.exit_code,
                                        observation.physical_files_changed.unwrap_or(0),
                                        digested_out
                                    ),
                                    "SUCCESS",
                                );
                                if observation.exit_code == Some(0) {
                                    reset_terminal_retry_circuits_after_success(
                                        &mut runtime,
                                        &mut retry_tracker,
                                    );
                                    if is_test_runner_command(&comando) {
                                        last_failed_test_run = None;
                                        if successful_test_run_has_behavioral_cases(
                                            &comando,
                                            &workspace_path,
                                            &digested_out,
                                        ) {
                                            programmer_calls_since_validation = 0;
                                            programmer_stall_recoveries = 0;
                                        } else {
                                            let message = "[VALIDATION] El runner terminó con código 0, pero no se encontraron casos con aserciones verificables; esto no reinicia el freno de progreso.";
                                            current_context.push_str(&format!("{}\n\n", message));
                                            emit_event(
                                                &app_handle,
                                                runtime.current_step(),
                                                message,
                                                "WARNING",
                                            );
                                        }
                                    }
                                    if is_node_syntax_check(&comando) {
                                        let notice = format!(
                                            "[DIAGNÓSTICO SUPERADO] La verificación actual `{}` terminó con código 0. Los errores de sintaxis anteriores de este archivo ya no describen el estado vigente; no los repitas como diagnóstico. Continúa con los requisitos funcionales que todavía no tengan evidencia.\n\n",
                                            comando.replace('`', "'")
                                        );
                                        current_context.push_str(&notice);
                                    }
                                    if let Some((next_tool, next_argument)) =
                                        web_validation_handoff_action(
                                            &original_prompt_parsed,
                                            &comando,
                                            programmer_stall_recoveries,
                                            last_local_ui_url.as_deref(),
                                            &workspace_path,
                                            last_http_server_task_id.as_deref(),
                                        )
                                    {
                                        // Override stale model/critic actions. If a server is
                                        // already running, read that task instead of starting
                                        // a duplicate process.
                                        current_role = AgentRole::Executor;
                                        forced_next_tool = Some((next_tool.clone(), next_argument));
                                        current_context.push_str(
                                            "[NO-PROGRESS GUARD] La sintaxis pasó, pero eso no demuestra que la aplicación funcione. La siguiente acción obligatoria es confirmar o iniciar el servidor HTTP local; después se leerán sus logs y se revisará la página real. No vuelvas a programar antes de esa comprobación.\n\n",
                                        );
                                        emit_event(
                                            &app_handle,
                                            runtime.current_step(),
                                            &format!(
                                                "[NO-PROGRESS GUARD] Sintaxis válida; se fuerza {} para continuar la revisión web.",
                                                next_tool
                                            ),
                                            "INFO",
                                        );
                                    }
                                    let successful_command_key = format!(
                                        "{}|{}",
                                        comando.trim().to_lowercase(),
                                        runtime.current_world_hash()
                                    );
                                    comandos_exitosos_historico
                                        .insert(successful_command_key.clone());
                                    // Record the post-command state too: test tools can create
                                    // temporary files, so the initial command key may have a
                                    // different world hash from the next turn.
                                    comandos_ejecutados_historico.insert(successful_command_key);
                                }
                                let required_web_review_pending =
                                    mission_requires_local_http_server(&original_prompt_parsed)
                                        && (last_local_ui_url.is_none()
                                            || (visual_validation_required
                                                && visual_validation_world_hash.is_none()));
                                if !required_web_review_pending
                                    && matches!(
                                        runtime.can_complete(),
                                        crate::core::completion_gate::CompletionDecision::Complete
                                    )
                                {
                                    current_role = AgentRole::Critic;
                                    forced_next_tool = Some(("TOOL_FINISH".into(), "El contrato tiene evidencia vigente para todos los criterios. Evalúa el cierre sin repetir las pruebas.".into()));
                                }
                                last_progress_step = runtime.current_step();
                            }
                            Err(err) => {
                                if comando.to_ascii_lowercase().contains("firebase")
                                    && comando.to_ascii_lowercase().contains("deploy")
                                    && is_firebase_hosting_spark_executable_rejection(&err)
                                {
                                    let message = "FIREBASE_HOSTING_ASSET_BLOCKED: Firebase rechazó la carga porque el plan Spark no admite archivos ejecutables. No se confirmó una publicación. Excluye todos los .bat/.cmd/.exe/.dll del directorio público en firebase.json, revisa el historial de Hosting y solo después vuelve a intentarlo.".to_string();
                                    emit_event(
                                        &app_handle,
                                        runtime.current_step(),
                                        &message,
                                        "ERROR",
                                    );
                                    persist_and_learn_failure!(&message);
                                    return Ok(serde_json::json!({
                                        "status": "ERROR",
                                        "respuesta_conversacional": message
                                    })
                                    .to_string());
                                }
                                // ── Auto ENV_MANAGER: detect binary-not-found and auto-install ──────────
                                let is_binary_missing = err.contains("is not recognized")
                                    || err.contains("not recognized as an internal")
                                    || err.contains("command not found")
                                    || (err.contains("The term")
                                        && err.contains("is not recognized"));

                                if is_binary_missing {
                                    // ── Step 1: Record error Observation through the full circuit ──
                                    // execute_action already records the observation and its
                                    // recovery decision. Only synthesize one when dispatch itself
                                    // returned Err without an Observation.
                                    if repair_decision.is_none() {
                                        let _ = runtime.observe_world();
                                        runtime.record_tool_call();
                                        let world_hash_after = runtime.current_world_hash();
                                        let mut err_obs =
                                            crate::core::observation::Observation::error(
                                                "TOOL_TERMINAL",
                                                &err,
                                                None,
                                                true,
                                                None,
                                            );
                                        err_obs.command = Some(comando.clone());
                                        err_obs.state_hash_after = Some(world_hash_after);
                                        runtime.record_observation(&err_obs);
                                    }

                                    // ── Step 2: RecoveryEngine classifies and produces RepairEnvironment ──
                                    let recovery_dec = reuse_or_record_recovery_decision(
                                        &mut runtime,
                                        repair_decision.clone(),
                                        "TOOL_TERMINAL",
                                        &err,
                                    );

                                    // ── Step 3: Execute TOOL_ENV_MANAGER as the recovery action via ActionProposal ──
                                    let binary =
                                        comando.split_whitespace().next().unwrap_or(&comando);
                                    emit_event(&app_handle, runtime.current_step(),
                                    &format!("[RECOVERY→TOOL_ENV_MANAGER] Binario '{}' no encontrado — ejecutando instalación automática por RecoveryEngine...", binary),
                                    "WARNING");
                                    if let crate::core::recovery::RecoveryDecision::Abort {
                                        reason,
                                    } = &recovery_dec
                                    {
                                        emit_event(
                                            &app_handle,
                                            runtime.current_step(),
                                            &format!("[RECOVERY] Abort: {}", reason),
                                            "FATAL",
                                        );
                                        runtime.fail_mission(reason.clone());
                                        break;
                                    }

                                    let env_proposal = crate::core::policy::ActionProposal {
                                        tool: "TOOL_ENV_MANAGER".to_string(),
                                        arguments: serde_json::json!({ "package": binary }),
                                        expected_effect: format!(
                                            "Auto-recovery install of missing binary {}",
                                            binary
                                        ),
                                        risk: crate::core::policy::RiskLevel::Moderate,
                                        world_hash: Some(runtime.current_world_hash()),
                                    };

                                    match runtime.execute_action(&env_proposal).await {
                                        Ok(obs) => {
                                            if obs.status == crate::core::observation::ObservationStatus::Success {
                                            current_context.push_str(&format!(
                                                "[RECOVERY] TOOL_ENV_MANAGER instaló '{}' automáticamente: {}\n\n\
                                                 Tu SIGUIENTE PASO OBLIGATORIO es reintentar el comando que falló: '{}'.\n\n",
                                                binary, obs.payload, comando
                                            ));
                                            emit_event(&app_handle, runtime.current_step(),
                                                &format!("Dependencia '{}' instalada. Reintenta el comando.", binary),
                                                "SUCCESS");
                                        } else {
                                            let res_msg = format!("Error: {}\n[AUTO-ENV FALLÓ] No se pudo instalar '{}': {}\nSe requiere intervención manual.", err, binary, obs.payload);
                                            current_context.push_str(&format!("Resultado: {}\n\n", res_msg));
                                            emit_event(&app_handle, runtime.current_step(), &res_msg, "ERROR");
                                        }
                                        }
                                        Err(e) => {
                                            let res_msg = format!("Error: {}\n[AUTO-ENV BLOQUEADO] No se pudo instalar '{}': {}", err, binary, e);
                                            current_context
                                                .push_str(&format!("Resultado: {}\n\n", res_msg));
                                            emit_event(
                                                &app_handle,
                                                runtime.current_step(),
                                                &res_msg,
                                                "ERROR",
                                            );
                                        }
                                    }
                                } else {
                                    let digested_err = digest_terminal_output(&err, 2500);
                                    let mut res_msg = format!("Error: {}", digested_err);
                                    if err.contains("ModuleNotFoundError")
                                        || err.contains("No module named")
                                    {
                                        // Extract module name from error for better hint
                                        let module_hint = if err.contains("No module named '") {
                                            err.split("No module named '")
                                                .nth(1)
                                                .and_then(|s| s.split('\'').next())
                                                .unwrap_or("<nombre_libreria>")
                                        } else {
                                            "<nombre_libreria>"
                                        };

                                        // ── CRITICAL: Distinguish local file vs external package ──────────
                                        // If module_hint.py EXISTS in the workspace, this is NOT a missing
                                        // pip package — it's an internal import error inside that local file.
                                        let local_py_exists = {
                                            let candidate = std::path::Path::new(&workspace_path)
                                                .join(format!("{}.py", module_hint));
                                            candidate.exists()
                                        };

                                        if local_py_exists {
                                            // Local file exists but cannot be imported → has internal errors
                                            res_msg.push_str(&format!(
                                            "\n\n[SISTEMA INTERNO] ⚠️ ATENCIÓN CRÍTICA: El archivo '{}.py' SÍ EXISTE en el workspace, \
                                            pero Python no puede importarlo. Esto significa que '{}.py' tiene un \
                                            ERROR INTERNO: puede ser un SyntaxError, un ImportError dentro de ese archivo, \
                                            o que importa otro módulo que aún no existe o tiene errores. \
                                            SOLUCIÓN OBLIGATORIA: Usa TOOL_AUDITOR para leer '{}.py' e identificar \
                                            qué línea está fallando. NO intentes 'pip install {}' — ese módulo \
                                            no es una librería externa, es un archivo LOCAL.",
                                            module_hint, module_hint, module_hint, module_hint
                                        ));
                                        } else {
                                            // Genuine missing external package
                                            let pip_was_tried =
                                                comandos_ejecutados_historico.iter().any(|c| {
                                                    c.starts_with("pip install")
                                                        || c.starts_with("pip3 install")
                                                });
                                            if pip_was_tried {
                                                res_msg.push_str(&format!(
                                                "\n\n[SISTEMA INTERNO] ADVERTENCIA CRÍTICA: Ya intentaste 'pip install {}' pero el módulo SIGUE SIN ENCONTRARSE. \
                                                Esto ocurre cuando tienes dos instalaciones de Python en tu máquina (ej. Miniconda + Python del sistema). \
                                                El pip instaló la librería en una instalación diferente a la que usa 'python'. \
                                                SOLUCIÓN OBLIGATORIA: En tu SIGUIENTE PASO usa TOOL_TERMINAL con el comando exacto: \
                                                'python -m pip install {}' — esto garantiza que pip usa el MISMO Python que corre el script.",
                                                module_hint, module_hint
                                            ));
                                            } else {
                                                res_msg.push_str(&format!(
                                                "\n\n[SISTEMA INTERNO TIP] Te falta una librería de Python. \
                                                Usa TOOL_TERMINAL con el comando: 'python -m pip install {}' \
                                                (NO uses solo 'pip install', usa 'python -m pip install' para garantizar que se instala en el intérprete correcto). \
                                                Luego vuelve a correr tu script.",
                                                module_hint
                                            ));
                                            }
                                        }
                                    }
                                    current_context
                                        .push_str(&format!("Resultado: {}\n\n", res_msg));
                                    emit_event(
                                        &app_handle,
                                        runtime.current_step(),
                                        &res_msg,
                                        "ERROR",
                                    );

                                    // ── Track error hash for semantic loop detection ──
                                    // If the same error output repeats 3+ times, sanity_monitor will escalate.
                                    {
                                        use std::collections::hash_map::DefaultHasher;
                                        use std::hash::{Hash, Hasher};
                                        let mut hasher = DefaultHasher::new();
                                        err.trim().hash(&mut hasher);
                                        let err_hash = hasher.finish();
                                        if last_error_hashes.len() >= 7 {
                                            last_error_hashes.pop_front();
                                        }
                                        last_error_hashes.push_back(err_hash);
                                    }

                                    // ── MissionRuntime: record error Observation → StallDetector + RecoveryEngine ──
                                    {
                                        // Reuse the decision produced from the action's real
                                        // Observation. Recording it a second time increments the
                                        // consecutive-failure circuit twice for one command.
                                        if repair_decision.is_none() {
                                            let _ = runtime.observe_world();
                                            runtime.record_tool_call();
                                            let world_hash_after = runtime.current_world_hash();
                                            let mut obs =
                                                crate::core::observation::Observation::error(
                                                    "TOOL_TERMINAL",
                                                    &err,
                                                    None,
                                                    true,
                                                    None,
                                                );
                                            obs.command = Some(comando.clone());
                                            obs.state_hash_after = Some(world_hash_after);
                                            runtime.record_observation(&obs);
                                        }
                                        let recovery_dec = reuse_or_record_recovery_decision(
                                            &mut runtime,
                                            repair_decision.clone(),
                                            "TOOL_TERMINAL",
                                            &err,
                                        );
                                        match &recovery_dec {
                                        crate::core::recovery::RecoveryDecision::RepairEnvironment { advice } => {
                                            emit_event(&app_handle, runtime.current_step(), &format!("[RECOVERY] Entorno: {}", advice), "WARNING");
                                        }
                                        crate::core::recovery::RecoveryDecision::Abort { reason } => {
                                            emit_event(&app_handle, runtime.current_step(), &format!("[RECOVERY] Abort: {}", reason), "FATAL");
                                            runtime.fail_mission(reason.clone());
                                            break;
                                        }
                                        _ => {}
                                    }
                                    }

                                    // ── SELF-REPAIR LOOP (ReAct pattern) ───────────────────

                                    // Classify the error type and choose the appropriate recovery strategy.
                                    // Professional agents (SWE-agent, Claude) never retry blindly.
                                    let error_type = crate::core::error_classifier::classify_error(
                                        &err, &res_msg, 1,
                                    );
                                    if is_test_runner_command(&comando) {
                                        last_failed_test_run = Some((comando.clone(), err.clone()));
                                    }
                                    let should_escalate =
                                        retry_tracker.record_failure("TOOL_TERMINAL", &error_type);

                                    if should_escalate {
                                        if error_type
                                            == crate::core::error_classifier::ErrorType::Blocked
                                        {
                                            // Genuine system/environment blocker — ask the user for help
                                            let escalation_msg = format!(
                                            "[BLOQUEO DEL SISTEMA] Fallo irrecuperable en TOOL_TERMINAL.\n\
                                            El comando requiere intervención del usuario (dependencia no instalable o permiso de admin).\n\
                                            Error: {}\n\
                                            Debes usar TOOL_ASK_USER para explicar qué dependencia externa se necesita.",
                                            err
                                        );
                                            current_context
                                                .push_str(&format!("{}\n\n", escalation_msg));
                                            emit_event(&app_handle, runtime.current_step(),
                                            "[SELF-REPAIR] Escalando bloqueo externo al usuario", "WARNING");
                                        } else {
                                            // Logic or test error — NEVER ask user to fix code!
                                            let self_heal_msg = format!(
                                            "[AUTO-REFLEXIÓN PROFUNDA] Múltiples reintentos en '{}'.\n\
                                            Salida del comando:\n{}\n\
                                            INSTRUCCIÓN DE AUTO-REPARACIÓN:\n\
                                            1. PROHIBIDO usar TOOL_ASK_USER para pedir al usuario que arregle código o tests.\n\
                                            2. Si falla una prueba, corrige con TOOL_PROGRAMMER el archivo que contiene el fallo o la aplicación; conserva el runner y el lenguaje del proyecto.\n\
                                            3. Si falta una prueba, crea una que ejercite comportamiento real en el stack existente; no cambies de lenguaje ni generes verificadores Python para otra plataforma.\n\
                                            4. Transición forzada a TOOL_PROGRAMMER para aplicar la solución directamente.",
                                            comando, err
                                        );
                                            current_context
                                                .push_str(&format!("{}\n\n", self_heal_msg));
                                            forced_next_tool = Some((
                                            "TOOL_PROGRAMMER".to_string(),
                                            "Corrige el archivo objetivo o flexibiliza el script de prueba.".to_string()
                                        ));
                                            emit_event(&app_handle, runtime.current_step(),
                                            "[AUTO-REFLEXIÓN] Redirigiendo a TOOL_PROGRAMMER para auto-reparar código/tests", "ACTION");
                                        }
                                    } else {
                                        // Inject specific repair guidance based on error type
                                        let repair_msg =
                                            crate::core::error_classifier::repair_prompt(
                                                &error_type,
                                                "TOOL_TERMINAL",
                                                &comando,
                                                &err,
                                                retry_tracker
                                                    .transient_retries
                                                    .max(retry_tracker.logic_retries),
                                            );
                                        current_context.push_str(&format!("{}\n\n", repair_msg));
                                        emit_event(&app_handle, runtime.current_step(),
                                        &format!("[SELF-REPAIR] Error {:?} — guiando al agente con estrategia de reparación",
                                            error_type),
                                        "WARNING");

                                        // A failing test is already ground-truth diagnosis. Send the
                                        // failed test output straight to the coder instead of spending
                                        // a turn asking a small model to reinterpret it.
                                        if error_type
                                            == crate::core::error_classifier::ErrorType::Logic
                                        {
                                            if is_test_runner_command(&comando) {
                                                current_role = AgentRole::Executor;
                                                let jest_guidance = if err
                                                    .to_ascii_lowercase()
                                                    .contains("jest is not defined")
                                                {
                                                    "No instales Jest: el runner configurado es `node --test`. Sustituye `jest.fn()` por un mock compatible con Node y prueba las funciones contra el estado y las claves reales de app.js."
                                                } else {
                                                    "Conserva el runner y corrige el código o la prueba que causó el error; no cambies de framework para ocultarlo."
                                                };
                                                forced_next_tool = Some((
                                                    "TOOL_PROGRAMMER".to_string(),
                                                    format!(
                                                        "Falló `{}`. Repara el error comprobado directamente antes de ejecutar otra prueba. Salida real: {}. {}",
                                                        comando,
                                                        err.chars().take(1600).collect::<String>(),
                                                        jest_guidance
                                                    ),
                                                ));
                                                emit_event(
                                                    &app_handle,
                                                    runtime.current_step(),
                                                    "[AUTO-REPARACIÓN] Prueba fallida: pasando directamente al programador con el error real.",
                                                    "ACTION",
                                                );
                                            } else {
                                                forced_next_tool = Some((
                                                    "TOOL_THINK".to_string(),
                                                    format!("Analiza el error: '{}'. Propone la corrección exacta antes de reintentar.", &err[..err.len().min(100)])
                                                ));
                                            }
                                            intercept_consecutive = 0;
                                        }
                                    }

                                    // ── FSM Transition: Critic → Executor on Terminal Failure ──
                                    if current_role == AgentRole::Critic {
                                        current_role = AgentRole::Executor;
                                        critic_feedback = Some(res_msg.clone());
                                        emit_event(&app_handle, runtime.current_step(),
                                        "[FSM] 🔬 CRÍTICO → ⚙️ EJECUTOR: Error en terminal, devolviendo al Ejecutor.",
                                        "WARNING");
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "TOOL_ASSET_MANAGER" => {
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    "Procesando TOOL_ASSET_MANAGER...",
                    "ACTION",
                );
                match runtime.execute_action(&action_proposal).await {
                    Ok(obs) => {
                        if obs.status == crate::core::observation::ObservationStatus::Success {
                            current_context.push_str(&format!(
                                "Resultado TOOL_ASSET_MANAGER: {}\n\n",
                                obs.payload
                            ));
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                "Asset descargado correctamente.",
                                "SUCCESS",
                            );
                        } else {
                            current_context.push_str(&format!(
                                "Error TOOL_ASSET_MANAGER: {}\n\n",
                                obs.payload
                            ));
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                &format!("Error: {}", obs.payload),
                                "ERROR",
                            );
                        }
                    }
                    Err(e) => {
                        current_context.push_str(&format!("Error TOOL_ASSET_MANAGER: {}\n\n", e));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &format!("Error: {}", e),
                            "ERROR",
                        );
                    }
                }
            }
            "TOOL_ENV_MANAGER" => {
                let cmd_trimmed = comando.trim();
                // Guard: detect when agent passes terminal commands or flags to ENV_MANAGER
                let looks_like_terminal_cmd = cmd_trimmed.contains(" -") // flags like -v, -h
                    || cmd_trimmed.starts_with("node ")
                    || cmd_trimmed.starts_with("python")
                    || cmd_trimmed.starts_with("npm ")
                    || cmd_trimmed.starts_with("npx ")
                    || cmd_trimmed.starts_with("pip ")
                    || cmd_trimmed.starts_with("dir")
                    || cmd_trimmed.starts_with("ls")
                    || cmd_trimmed.contains(".js")
                    || cmd_trimmed.contains(".py")
                    || cmd_trimmed.contains(".ts");

                if looks_like_terminal_cmd {
                    let warn_msg = format!(
                        "[SISTEMA]: TOOL_ENV_MANAGER RECHAZADO. '{}' parece un comando de terminal, no un nombre de paquete.\n\
                        TOOL_ENV_MANAGER solo acepta nombres de paquetes scoop (ej: 'nodejs', 'python', 'git').\n\
                        Para ejecutar comandos en la terminal usa TOOL_TERMINAL en su lugar.",
                        cmd_trimmed
                    );
                    current_context.push_str(&format!("{}\n\n", warn_msg));
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!(
                            "[ENV_MANAGER] Rechazado comando de terminal: {}",
                            cmd_trimmed
                        ),
                        "WARNING",
                    );
                } else if cmd_trimmed.is_empty() {
                    let res_msg = "Error: El paquete no puede estar vacío.";
                    current_context.push_str(&format!("{}\n\n", res_msg));
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "Paquete vacío",
                        "ERROR",
                    );
                } else if paquetes_instalados_historico.contains(&comando) {
                    let res_msg = "[SISTEMA INTERCEPTO] Error Crítico: Bucle infinito intentando instalar el mismo paquete repetidamente. Abortando misión.";
                    emit_event(&app_handle, runtime.current_step(), res_msg, "FATAL");
                    let final_res = FinalResponse {
                        status: "ERROR".to_string(),
                        respuesta_conversacional: format!("Se detectó un bucle intentando instalar múltiples veces el paquete '{}'. La instalación ya se ejecutó en este turno. Misión abortada.", comando),
                    };
                    crate::llm::router::record_model_result(
                        &orchestrator_model,
                        &crate::llm::router::TaskType::Orchestrator,
                        final_res.status == "FINISH",
                        runtime.current_step(),
                    );
                    return Ok(serde_json::to_string(&final_res).unwrap());
                } else {
                    paquetes_instalados_historico.insert(comando.clone());
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!("Módulo de Ingeniería de Entorno instalando: {}", comando),
                        "ACTION",
                    );
                    match runtime.execute_action(&action_proposal).await {
                        Ok(obs) => {
                            if obs.status == crate::core::observation::ObservationStatus::Success {
                                current_context.push_str(&format!(
                                    "Resultado TOOL_ENV_MANAGER: {}\n\n",
                                    obs.payload
                                ));
                                emit_event(&app_handle, runtime.current_step(), "Dependencia instalada correctamente. PATH recargado en caliente.", "SUCCESS");
                            } else {
                                current_context.push_str(&format!(
                                    "Resultado TOOL_ENV_MANAGER Error: {}\n\n",
                                    obs.payload
                                ));
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    &obs.payload,
                                    "ERROR",
                                );
                            }
                        }
                        Err(err) => {
                            current_context.push_str(&format!(
                                "Resultado TOOL_ENV_MANAGER Error: {}\n\n",
                                err
                            ));
                            emit_event(&app_handle, runtime.current_step(), &err, "ERROR");
                        }
                    }
                }
            }
            "TOOL_BACKGROUND_START" => {
                let original_background_command = comando.clone();
                let comando = local_http_server_command_for_request(
                    &original_prompt_parsed,
                    &original_background_command,
                )
                .unwrap_or(&original_background_command)
                .to_string();
                if comando != original_background_command {
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "[SERVIDOR LOCAL] Se reemplazó la apertura file:// por un servidor HTTP en segundo plano, según el mandato.",
                        "INFO",
                    );
                }
                if comando.trim().is_empty() {
                    if comandos_ejecutados_historico.contains(&format!(
                        "__EMPTY_BG_CMD__|{}",
                        runtime.current_world_hash()
                    )) {
                        let res_msg = "[SISTEMA INTERNO]: Advertencia: Estás en un bucle infinito de comandos vacíos. Abortando.";
                        emit_event(&app_handle, runtime.current_step(), res_msg, "FATAL");
                        let final_res = FinalResponse {
                            status: "ERROR".to_string(),
                            respuesta_conversacional: "Error interno del planificador asíncrono."
                                .to_string(),
                        };
                        crate::llm::router::record_model_result(
                            &orchestrator_model,
                            &crate::llm::router::TaskType::Orchestrator,
                            final_res.status == "FINISH",
                            runtime.current_step(),
                        );
                        return Ok(serde_json::to_string(&final_res).unwrap());
                    }
                    comandos_ejecutados_historico
                        .insert(format!("__EMPTY_BG_CMD__|{}", runtime.current_world_hash()));
                    let err_msg = "Error Crítico: El campo 'comando' está vacío. Debes especificar qué comando ejecutar en la terminal.";
                    current_context.push_str(&format!("{}\n\n", err_msg));
                    emit_event(&app_handle, runtime.current_step(), err_msg, "ERROR");
                } else if !is_forced_and_obeyed
                    && comandos_ejecutados_historico.contains(&format!(
                        "{}|{}",
                        comando.trim().to_lowercase(),
                        runtime.current_world_hash()
                    ))
                {
                    let res_msg = "[SISTEMA INTERNO]: Advertencia: Este servidor o proceso YA ESTÁ EN EJECUCIÓN en segundo plano. NO necesitas volver a iniciarlo. Usa TOOL_VISION_EVALUATOR o TOOL_FINISH.";
                    current_context.push_str(&format!("{}\n\n", res_msg));
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "Servidor ya en ejecución (bucle evitado).",
                        "WARNING",
                    );
                } else {
                    comandos_ejecutados_historico.insert(format!(
                        "{}|{}",
                        comando.trim().to_lowercase(),
                        runtime.current_world_hash()
                    ));
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!("Iniciando tarea asíncrona '{}': {}", task_id, comando),
                        "ACTION",
                    );
                    let background_proposal = crate::core::policy::ActionProposal {
                        tool: "TOOL_BACKGROUND_START".to_string(),
                        arguments: serde_json::json!({ "comando": comando.clone(), "task_id": task_id.clone() }),
                        expected_effect: "Iniciar el proceso solicitado y registrar su identificador para poder consultarlo o detenerlo".to_string(),
                        risk: crate::core::policy::PolicyEngine::classify_terminal_command(&comando),
                        world_hash: Some(runtime.current_world_hash()),
                    };
                    match runtime.execute_action(&background_proposal).await {
                        Ok(observation)
                            if observation.status
                                == crate::core::observation::ObservationStatus::Success =>
                        {
                            last_background_task_id = Some(task_id.clone());
                            let requires_http_confirmation =
                                mission_requires_local_http_server(&original_prompt_parsed)
                                    && is_long_running_server_command(&comando);
                            let sys_guidance = if requires_http_confirmation {
                                local_http_server_read_attempts = 0;
                                last_http_server_task_id = Some(task_id.clone());
                                forced_next_tool =
                                    Some(("TOOL_BACKGROUND_READ".to_string(), task_id.clone()));
                                "[SISTEMA INTERNO]: El proceso se inició, pero eso no confirma que sea un servidor activo. Lee sus logs ahora y confirma una URL HTTP localhost antes de abrirla o evaluarla."
                            } else {
                                "[SISTEMA INTERNO]: El proceso en segundo plano se inició. Consulta sus logs y verifica su efecto antes de continuar; no deduzcas que está activo solo por el acuse de inicio."
                            };
                            current_context.push_str(&format!(
                                "Resultado: {}\n{}\n\n",
                                observation.payload, sys_guidance
                            ));
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                &observation.payload,
                                "SUCCESS",
                            );
                            last_progress_step = runtime.current_step();
                        }
                        Ok(observation) => {
                            current_context.push_str(&format!(
                                "Resultado: Error iniciando tarea: {}\n\n",
                                observation.payload
                            ));
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                &observation.payload,
                                "ERROR",
                            );
                        }
                        Err(error) => {
                            current_context.push_str(&format!(
                                "Resultado: Error iniciando tarea: {}\n\n",
                                error
                            ));
                            emit_event(&app_handle, runtime.current_step(), &error, "ERROR");
                        }
                    }
                }
            }
            "TOOL_BACKGROUND_READ" | "TOOL_BACKGROUND_QUERY" => {
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    &format!("Leyendo logs asíncronos de '{}'", task_id),
                    "ACTION",
                );
                let read_proposal = crate::core::policy::ActionProposal {
                    tool: tool.clone(),
                    arguments: serde_json::json!({ "task_id": task_id.clone() }),
                    expected_effect: "Leer los registros del proceso identificado".to_string(),
                    risk: crate::core::policy::RiskLevel::Safe,
                    world_hash: Some(runtime.current_world_hash()),
                };
                match runtime.execute_action(&read_proposal).await {
                    Ok(observation)
                        if observation.status
                            == crate::core::observation::ObservationStatus::Success =>
                    {
                        let logged_url = extract_local_ui_url(&observation.payload);
                        let confirmed_url = if logged_url.is_some() {
                            logged_url.clone()
                        } else if mission_requires_local_http_server(&original_prompt_parsed) {
                            probe_background_local_http_server(&task_id).await
                        } else {
                            None
                        };
                        if let Some(url) = confirmed_url {
                            last_local_ui_url = Some(url.clone());
                            local_http_server_read_attempts = 0;
                            let confirmation_source = if logged_url.is_some() {
                                "logs"
                            } else {
                                "petición HTTP real"
                            };
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                &format!("[UI LOCAL] URL confirmada mediante {confirmation_source}: {url}"),
                                "SUCCESS",
                            );
                            if mission_requires_visual_verification(
                                &original_prompt_parsed,
                                &workspace_path,
                            ) {
                                forced_next_tool = Some(("TOOL_VISION_EVALUATOR".to_string(), url));
                            }
                        } else if mission_requires_local_http_server(&original_prompt_parsed) {
                            local_http_server_read_attempts =
                                local_http_server_read_attempts.saturating_add(1);
                            if local_http_server_read_attempts < 3 {
                                forced_next_tool =
                                    Some(("TOOL_BACKGROUND_READ".to_string(), task_id.clone()));
                                current_context.push_str(
                                    "[SERVIDOR LOCAL] Aún no aparece una URL HTTP en los logs. Vuelve a leer este mismo proceso; no abras index.html con file:// ni afirmes que el servidor quedó confirmado.\n\n",
                                );
                            } else {
                                let message = "LOCAL_SERVER_NOT_CONFIRMED: los logs no anunciaron una URL y la comprobación HTTP directa del servidor local tampoco recibió una respuesta válida tras tres intentos. Se conservó el trabajo y no se declaró prueba visual.";
                                emit_event(&app_handle, runtime.current_step(), message, "ERROR");
                                let _ = crate::core::kill_task(&task_id).await;
                                persist_and_learn_failure!(message);
                                return Ok(serde_json::json!({
                                    "status": "INCOMPLETE",
                                    "respuesta_conversacional": message
                                })
                                .to_string());
                            }
                        }
                        current_context
                            .push_str(&format!("Logs obtenidos:\n{}\n\n", observation.payload));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "Logs leídos correctamente.",
                            "SUCCESS",
                        );
                    }
                    Ok(observation) => {
                        current_context.push_str(&format!(
                            "[AUTO-DEBUGGER] Error al leer logs: {}\n\n",
                            observation.payload
                        ));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &observation.payload,
                            "ERROR",
                        );
                    }
                    Err(error) => {
                        let formatted = format_system_error(&error).await;
                        current_context.push_str(&format!(
                            "[AUTO-DEBUGGER] Error al leer logs: {}\n\n",
                            formatted
                        ));
                        emit_event(&app_handle, runtime.current_step(), &formatted, "ERROR");
                    }
                }
            }
            "TOOL_BACKGROUND_KILL" => {
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    &format!("Destruyendo tarea asíncrona '{}'", task_id),
                    "ACTION",
                );
                let kill_proposal = crate::core::policy::ActionProposal {
                    tool: "TOOL_BACKGROUND_KILL".to_string(),
                    arguments: serde_json::json!({ "task_id": task_id.clone() }),
                    expected_effect: "Detener el proceso identificado en segundo plano".to_string(),
                    risk: crate::core::policy::RiskLevel::Moderate,
                    world_hash: Some(runtime.current_world_hash()),
                };
                match runtime.execute_action(&kill_proposal).await {
                    Ok(observation)
                        if observation.status
                            == crate::core::observation::ObservationStatus::Success =>
                    {
                        current_context
                            .push_str(&format!("Resultado: {}\n\n", observation.payload));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &observation.payload,
                            "SUCCESS",
                        );
                    }
                    Ok(observation) => {
                        current_context.push_str(&format!(
                            "[AUTO-DEBUGGER] Error matando tarea: {}\n\n",
                            observation.payload
                        ));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &observation.payload,
                            "ERROR",
                        );
                    }
                    Err(error) => {
                        let formatted = format_system_error(&error).await;
                        current_context.push_str(&format!(
                            "[AUTO-DEBUGGER] Error matando tarea: {}\n\n",
                            formatted
                        ));
                        emit_event(&app_handle, runtime.current_step(), &formatted, "ERROR");
                    }
                }
            }
            "TOOL_WEB_SCRAPER" | "TOOL_BROWSE" => {
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    &format!("Extrayendo contenido de: {}", url),
                    "ACTION",
                );
                let browse_proposal = crate::core::policy::ActionProposal {
                    tool: tool.clone(),
                    arguments: serde_json::json!({ "url": url.clone() }),
                    expected_effect: "Extraer texto de la página solicitada para analizarla"
                        .to_string(),
                    risk: crate::core::policy::RiskLevel::Safe,
                    world_hash: Some(runtime.current_world_hash()),
                };
                match runtime.execute_action(&browse_proposal).await {
                    Ok(observation)
                        if observation.status
                            == crate::core::observation::ObservationStatus::Success =>
                    {
                        let preview = truncate_chars(&observation.payload, 1000);
                        current_context.push_str(&format!(
                            "Contenido web:\n{}{}\n\n",
                            preview,
                            if observation.payload.chars().count() > 1000 {
                                "... (truncado)"
                            } else {
                                ""
                            }
                        ));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "Contenido extraído con éxito.",
                            "SUCCESS",
                        );
                    }
                    Ok(observation) => {
                        current_context
                            .push_str(&format!("Error web: {}\n\n", observation.payload));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &observation.payload,
                            "ERROR",
                        );
                    }
                    Err(error) => {
                        current_context.push_str(&format!("Error web: {}\n\n", error));
                        emit_event(&app_handle, runtime.current_step(), &error, "ERROR");
                    }
                }
            }
            "TOOL_GIT" => {
                let git_command = if comando.trim().is_empty() {
                    "status".to_string()
                } else {
                    comando.trim().to_string()
                };
                let git_proposal = crate::core::policy::ActionProposal {
                    tool: "TOOL_GIT".to_string(),
                    arguments: serde_json::json!({ "comando": git_command.clone() }),
                    expected_effect: "Ejecutar el subcomando Git solicitado dentro del workspace"
                        .to_string(),
                    risk: crate::core::policy::PolicyEngine::classify_terminal_command(&format!(
                        "git {}",
                        git_command
                    )),
                    world_hash: Some(runtime.current_world_hash()),
                };
                match runtime.execute_action(&git_proposal).await {
                    Ok(observation)
                        if observation.status
                            == crate::core::observation::ObservationStatus::Success =>
                    {
                        current_context
                            .push_str(&format!("Resultado TOOL_GIT:\n{}\n\n", observation.payload));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "Operación Git completada.",
                            "SUCCESS",
                        );
                        last_progress_step = runtime.current_step();
                    }
                    Ok(observation) => {
                        current_context
                            .push_str(&format!("Error TOOL_GIT:\n{}\n\n", observation.payload));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &observation.payload,
                            "ERROR",
                        );
                    }
                    Err(error) => {
                        current_context.push_str(&format!("Error TOOL_GIT:\n{}\n\n", error));
                        emit_event(&app_handle, runtime.current_step(), &error, "ERROR");
                    }
                }
            }
            "TOOL_AUDITOR" => {
                auditor_consecutive += 1;
                if auditor_consecutive > 2 {
                    let msg = "[SISTEMA INTERNO]: Loop de auditoría detectado. Estás auditando demasiadas veces seguidas sin actuar. FORZANDO TOOL_THINK en el siguiente turno.";
                    current_context.push_str(&format!("{}\n\n", msg));
                    emit_event(&app_handle, runtime.current_step(), msg, "WARNING");
                    forced_next_tool = Some(("TOOL_THINK".to_string(), "Analizar auditorias previas y decidir siguiente paso. Si los archivos ya existen usa TOOL_PROGRAMMER para mejorarlos o TOOL_FINISH si todo está correcto.".to_string()));
                } else {
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "Auditando archivos locales...",
                        "ACTION",
                    );
                    match runtime.execute_action(&action_proposal).await {
                        Ok(observation)
                            if observation.status
                                == crate::core::observation::ObservationStatus::Success
                                && !observation.payload.trim().is_empty() =>
                        {
                            current_context.push_str(&format!(
                                "[REPORTE AUDITOR]\n{}\n\n",
                                observation.payload.trim()
                            ));
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                "Auditoría de código completada con respuesta del modelo.",
                                "SUCCESS",
                            );
                            mandatory_tools_executed.insert("TOOL_AUDITOR".to_string());
                        }
                        Ok(observation) => {
                            current_context.push_str(&format!(
                                "[AUDITORÍA INCOMPLETA]\n{}\n\n",
                                observation.payload
                            ));
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                &format!("Auditoría no completada: {}", observation.payload),
                                "WARNING",
                            );
                        }
                        Err(error) => {
                            current_context
                                .push_str(&format!("Error en TOOL_AUDITOR: {}\n\n", error));
                            emit_event(&app_handle, runtime.current_step(), &error, "ERROR");
                        }
                    }
                }
            }
            "TOOL_LOGIC_SOLVER" => {
                let sat_payload =
                    parse_sat_payload(&raw_value).and_then(|provided| match provided {
                        Some(instance) => Ok(Some(instance)),
                        None => extract_sat_payload(&user_message),
                    });

                let verdict = match sat_payload {
                    Ok(Some((n_vars, clauses))) => {
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &format!(
                                "⚡ [SPECTRASAT] Resolviendo instancia SAT: {} variables, {} cláusulas...",
                                n_vars,
                                clauses.len()
                            ),
                            "ACTION",
                        );
                        let sat_proposal = crate::core::policy::ActionProposal {
                            tool: "TOOL_LOGIC_SOLVER".to_string(),
                            arguments: serde_json::json!({
                                "n_vars": n_vars,
                                "clauses": clauses
                            }),
                            expected_effect: "Resolver la fórmula SAT y verificar el resultado contra todas las cláusulas originales".to_string(),
                            risk: crate::core::policy::RiskLevel::Safe,
                            world_hash: Some(runtime.current_world_hash()),
                        };
                        let result = match runtime.execute_action(&sat_proposal).await {
                            Ok(observation)
                                if observation.status
                                    == crate::core::observation::ObservationStatus::Success =>
                            {
                                observation.payload
                            }
                            Ok(observation) => observation.payload,
                            Err(error) => serde_json::json!({
                                "status": "TOOL_EXECUTION_ERROR",
                                "error": error
                            })
                            .to_string(),
                        };
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "[SPECTRASAT] El motor devolvió un resultado; verificando su estado.",
                            "INFO",
                        );
                        result
                    }
                    Ok(None) => {
                        // No se inventa una fórmula SAT a partir de código. Este
                        // camino es una revisión semántica separada y no certifica SAT.
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "[LOGIC_SOLVER] Analizando la lógica del código con el modelo local...",
                            "ACTION",
                        );
                        let real_files: Vec<String> = {
                            let hallucinated = archivos_vec.iter().any(|file| {
                                !std::path::Path::new(&workspace_path).join(file).exists()
                            });
                            if hallucinated || archivos_vec.is_empty() {
                                let mut found = Vec::new();
                                if let Ok(entries) = std::fs::read_dir(&workspace_path) {
                                    for entry in entries.flatten() {
                                        let path = entry.path();
                                        if let Some(ext) = path.extension() {
                                            let ext = ext.to_string_lossy().to_lowercase();
                                            if matches!(
                                                ext.as_str(),
                                                "py" | "rs" | "js" | "ts" | "go" | "c" | "cpp"
                                            ) {
                                                found.push(path.to_string_lossy().to_string());
                                            }
                                        }
                                    }
                                }
                                found
                            } else {
                                archivos_vec.clone()
                            }
                        };
                        let safe_files =
                            memory::read_files_safely(&workspace_path, real_files).await;
                        if safe_files.trim().is_empty() {
                            serde_json::json!({
                                "status": "ANALYSIS_UNAVAILABLE",
                                "error": "No encontré archivos de código accesibles para analizar."
                            })
                            .to_string()
                        } else {
                            delegate_to_logic_solver(&safe_files, &orchestrator_model).await
                        }
                    }
                    Err(error) => serde_json::json!({
                        "status": "INVALID_INPUT",
                        "error": error
                    })
                    .to_string(),
                };

                let mut parsed_status = verdict.clone();
                let mut assignment_msg = String::new();
                let mut solver_error = None;
                let mut has_structured_status = false;

                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&verdict) {
                    if let Some(s) = parsed.get("status").and_then(|v| v.as_str()) {
                        parsed_status = s.to_string();
                        has_structured_status = true;
                    }
                    solver_error = parsed
                        .get("error")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string);
                    if let Some(arr) = parsed.get("assignment").and_then(|v| v.as_array()) {
                        let vars: Vec<String> = arr
                            .iter()
                            .enumerate()
                            .map(|(i, v)| format!("v{} = {}", i + 1, v.as_bool().unwrap_or(false)))
                            .collect();
                        assignment_msg =
                            format!("\n\n[ASIGNACIÓN BOOLEANA ENCONTRADA]:\n{}", vars.join("\n"));
                    }
                }

                // ─── FSM: Veredicto obtenido → generar reporte y condicionar TOOL_FINISH ───
                if has_structured_status && parsed_status.starts_with("UNSAT") {
                    let unsat_msg = format!(
                        "⛔ VEREDICTO SPECTRASAT: {}\n\n\
                        AUDITORÍA DE INTEGRIDAD LÓGICA — SISTEMA CONTRADICTORIO DETECTADO\n\
                        El motor matemático SpectraSAT ha certificado que el sistema analizado \
                        es INSATISFACIBLE (UNSAT). No existe ninguna combinación de valores booleanos \
                        que cumpla todas las restricciones simultáneamente. Se detectó una contradicción lógica irresoluble.",
                        parsed_status
                    );
                    current_context.push_str(&format!("{}\n\n", unsat_msg));
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!("⛔ [SPECTRASAT] UNSAT certificado — sistema contradictorio"),
                        "WARNING",
                    );
                    if mission_type == MissionType::Analysis {
                        forced_next_tool = Some(("TOOL_FINISH".to_string(), unsat_msg));
                    }
                } else if has_structured_status && parsed_status.starts_with("SAT") {
                    let sat_msg = format!(
                        "✅ VEREDICTO SPECTRASAT: {}\n\n\
                        AUDITORÍA DE INTEGRIDAD LÓGICA — SISTEMA SATISFACIBLE\n\
                        El motor matemático SpectraSAT ha certificado que el conjunto de restricciones \
                        ES SATISFACIBLE (SAT). Existe al menos una asignación exacta de variables booleanas \
                        que cumple todas las restricciones simultáneamente.{}",
                        parsed_status, assignment_msg
                    );
                    current_context.push_str(&format!("{}\n\n", sat_msg));
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "✅ [SPECTRASAT] SAT certificado — sistema seguro",
                        "SUCCESS",
                    );
                    if mission_type == MissionType::Analysis {
                        forced_next_tool = Some(("TOOL_FINISH".to_string(), sat_msg));
                    }
                } else {
                    let is_unknown = parsed_status.starts_with("UNKNOWN");
                    let is_error_status = matches!(
                        parsed_status.as_str(),
                        "INVALID_INPUT"
                            | "TOOL_EXECUTION_ERROR"
                            | "ANALYSIS_UNAVAILABLE"
                            | "INTERNAL_VERIFICATION_FAILED"
                    );
                    let semantic_error = verdict
                        .to_lowercase()
                        .starts_with("error en verificación lógica:");
                    let status = if is_error_status || semantic_error {
                        "ERROR"
                    } else if is_unknown {
                        "WARNING"
                    } else {
                        "INFO"
                    };
                    let message = if let Some(error) = solver_error.as_deref() {
                        format!("[LOGIC_SOLVER] {} — {}", parsed_status, error)
                    } else if is_unknown {
                        format!(
                            "[SPECTRASAT] {}. No se pudo certificar un resultado.",
                            parsed_status
                        )
                    } else if is_error_status || semantic_error {
                        format!("[LOGIC_SOLVER] Error: {}", parsed_status)
                    } else {
                        "[LOGIC_SOLVER] Análisis semántico devuelto; no es un certificado SAT/UNSAT."
                            .to_string()
                    };
                    let label = if parsed_status == verdict {
                        "Análisis lógico-semántico"
                    } else {
                        "Resultado del motor lógico"
                    };
                    current_context.push_str(&format!(
                        "{} ({}): {}{}\n\n",
                        label,
                        parsed_status,
                        verdict,
                        solver_error
                            .as_deref()
                            .filter(|error| !verdict.contains(error))
                            .map(|error| format!("\nDiagnóstico: {}", error))
                            .unwrap_or_default()
                    ));
                    emit_event(&app_handle, runtime.current_step(), &message, status);
                }
            }
            "TOOL_WORKSPACE_MANAGER" => {
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    "Gestionando archivos del workspace...",
                    "ACTION",
                );
                if archivos_vec.is_empty() {
                    workspace_manager_error_consecutive += 1;
                    let err_msg = if workspace_manager_error_consecutive >= 3 {
                        // Force the agent out of the loop by injecting a strong directive
                        think_consecutive = 0;
                        mapper_consecutive = 0;
                        forced_next_tool = Some((
                            "TOOL_THINK".to_string(),
                            "Llevas varios intentos fallidos con TOOL_WORKSPACE_MANAGER sin proveer archivos. El workspace no necesita limpieza. Procede directamente a crear los archivos necesarios con TOOL_PROGRAMMER.".to_string()
                        ));
                        workspace_manager_error_consecutive = 0;
                        "[SISTEMA]: Bucle de TOOL_WORKSPACE_MANAGER detectado. No hay archivos que eliminar. \
                        El workspace ya está listo. DEBES usar TOOL_THINK ahora para planificar \
                        la creación de los archivos del proyecto con TOOL_PROGRAMMER."
                    } else {
                        "Error: TOOL_WORKSPACE_MANAGER requiere una lista de archivos a eliminar. \
                        Si el workspace está vacío o no necesitas borrar nada, usa TOOL_THINK \
                        para avanzar al siguiente paso."
                    };
                    current_context.push_str(&format!("{}\n\n", err_msg));
                    emit_event(&app_handle, runtime.current_step(), err_msg, "ERROR");
                } else {
                    workspace_manager_error_consecutive = 0;
                    match runtime.execute_action(&action_proposal).await {
                        Ok(obs) => {
                            if obs.status == crate::core::observation::ObservationStatus::Success {
                                current_context.push_str(&format!("Éxito: {}\n\n", obs.payload));
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    &format!(
                                        "Limpieza finalizada. {} afectados.",
                                        obs.files_affected.len()
                                    ),
                                    "SUCCESS",
                                );
                            } else {
                                current_context.push_str(&format!(
                                    "Errores durante la limpieza: {}\n\n",
                                    obs.payload
                                ));
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    &format!("Error en limpieza: {}", obs.payload),
                                    "ERROR",
                                );
                            }
                        }
                        Err(e) => {
                            current_context.push_str(&format!("Error de ejecución: {}\n\n", e));
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                &format!("Error WorkspaceManager: {}", e),
                                "ERROR",
                            );
                        }
                    }
                }
            }
            "TOOL_READ_FILE" => {
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    "[TOOL_READ_FILE] Leyendo archivo...",
                    "ACTION",
                );
                match runtime.execute_action(&action_proposal).await {
                    Ok(obs) => {
                        if obs.status == crate::core::observation::ObservationStatus::Success {
                            let contents = &obs.payload;
                            let c_len = contents.len();
                            let display = if c_len > 8000 {
                                &contents[..8000]
                            } else {
                                &contents[..]
                            };
                            let read_msg = format!(
                                "[TOOL_READ_FILE] Contenido:\n```\n{}\n```\n\n\
                                 Ahora tienes el contenido real del archivo. Usa TOOL_PROGRAMMER con el \
                                 campo 'buscar' copiado EXACTAMENTE del texto anterior.\n\n",
                                display
                            );
                            current_context.push_str(&read_msg);
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                &format!("Archivo leído: {} chars", c_len),
                                "SUCCESS",
                            );
                        } else {
                            current_context
                                .push_str(&format!("[TOOL_READ_FILE] Error: {}\n\n", obs.payload));
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                &format!("TOOL_READ_FILE Error: {}", obs.payload),
                                "ERROR",
                            );
                        }
                    }
                    Err(e) => {
                        current_context
                            .push_str(&format!("[TOOL_READ_FILE] Error de ejecución: {}\n\n", e));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &format!("TOOL_READ_FILE Error: {}", e),
                            "ERROR",
                        );
                    }
                }
            }
            "TOOL_THINK" => {
                think_consecutive += 1;
                if think_consecutive > 1 {
                    if mission_type == MissionType::Planning {
                        emit_event(&app_handle, runtime.current_step(), "[PLAN INICIAL] Reflexión repetida interceptada; se fuerza la entrega de la hoja de ruta.", "WARNING");
                        current_context.push_str("[PLAN INICIAL]: No repitas la reflexión ni inicies programación. Entrega ahora el plan con fases, entregables, supuestos y decisiones pendientes.\n\n");
                        forced_next_tool = Some((
                            "TOOL_FINISH".to_string(),
                            "Resume el plan inicial en la respuesta y termina.".to_string(),
                        ));
                    } else {
                        emit_event(&app_handle, runtime.current_step(), "[COOLDOWN] Bucle TOOL_THINK interceptado. NON_PROGRESS_THINK -> Forzando replan.", "WARNING");
                        current_context.push_str(&format!("PASO {}:\nNON_PROGRESS_THINK: No se permite reflexión consecutiva sin acción. El entorno no ha cambiado. DEBES ejecutar una acción física (TOOL_PROGRAMMER o TOOL_TERMINAL) o replantear completamente tu estrategia.\n\n", runtime.current_step()));

                        let has_files = !runtime.state_anchor.existing_files.is_empty();
                        if !has_files {
                            forced_next_tool = Some((
                                "TOOL_PROGRAMMER".to_string(),
                                "Workspace vacío. Escribe los archivos iniciales requeridos."
                                    .to_string(),
                            ));
                        } else {
                            // If files exist, force terminal to test/verify instead of getting stuck thinking
                            forced_next_tool =
                                Some(("TOOL_TERMINAL".to_string(), "dir".to_string()));
                        }
                    }
                } else {
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "Pensando y planificando...",
                        "ACTION",
                    );
                    current_context
                        .push_str(&format!("Reflexion Interna del Agente: {}\n\n", &comando));
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "Reflexion completada.",
                        "SUCCESS",
                    );
                    // Analysis and initial-planning requests remain in Planner; neither asks the Executor to code.
                    if current_role == AgentRole::Planner {
                        if mission_type == MissionType::Analysis
                            || mission_type == MissionType::Planning
                        {
                            let mode_message = if mission_type == MissionType::Planning {
                                "[FSM] MODO PLAN INICIAL: el Planificador permanece activo para preparar la hoja de ruta."
                            } else {
                                "[FSM] MODO ANALISIS: Planificador permanece activo. Usa TOOL_FINISH para responder."
                            };
                            emit_event(&app_handle, runtime.current_step(), mode_message, "INFO");
                        } else {
                            if !comando.trim().is_empty() {
                                acceptance_contract = Some(formato_contrato(&comando));

                                let cmd_lower = comando.to_lowercase();

                                // P0 Commit 4: Deterministic criteria
                                runtime.contract.add_criterion(
                                    &format!(
                                        "AC-{:03}",
                                        runtime.contract.acceptance_criteria.len() + 1
                                    ),
                                    &comando.chars().take(120).collect::<String>(),
                                    crate::core::mission_contract::VerificationMethod::TestPassed,
                                    true, // Core objective requires tests passing
                                );

                                if cmd_lower.contains("dashboard") || cmd_lower.contains("panel") {
                                    runtime.contract.add_criterion(
                                            &format!("AC-{:03}", runtime.contract.acceptance_criteria.len() + 1),
                                            "El archivo index.html debe existir",
                                            crate::core::mission_contract::VerificationMethod::FileExistence("index.html".to_string()),
                                            true,
                                        );
                                    runtime.contract.add_criterion(
                                            &format!("AC-{:03}", runtime.contract.acceptance_criteria.len() + 1),
                                            "Un elemento canvas debe estar presente para gráficos",
                                            crate::core::mission_contract::VerificationMethod::ContentMatches {
                                                file: "index.html".to_string(),
                                                regex: "<canvas".to_string()
                                            },
                                            true,
                                        );
                                    runtime.contract.add_criterion(
                                            &format!("AC-{:03}", runtime.contract.acceptance_criteria.len() + 1),
                                            "El script de validación (verify_dashboard.py) debe pasar al 100%",
                                            crate::core::mission_contract::VerificationMethod::CommandExitZero("python verify_dashboard.py".to_string()),
                                            true,
                                        );
                                }
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    &format!(
                                        "[CONTRATO] Criterios definidos ({} total)",
                                        runtime.contract.acceptance_criteria.len()
                                    ),
                                    "INFO",
                                );
                            }
                            current_role = AgentRole::Executor;
                            critic_feedback = None;
                            emit_event(&app_handle, runtime.current_step(), "[FSM] PLANIFICADOR -> EJECUTOR: Plan aprobado. Iniciando escritura de codigo.", "INFO");
                            forced_next_tool = Some((
                                    "TOOL_PROGRAMMER".to_string(),
                                    "Plan completado. Inicia la creación o modificación de los archivos con TOOL_PROGRAMMER.".to_string(),
                                ));
                        }
                    }
                }
            }
            "TOOL_MAPPER" => {
                mapper_consecutive += 1;
                if mapper_consecutive > 1 {
                    let msg = "[SISTEMA INTERNO]: Loop de TOOL_MAPPER detectado. El workspace no cambiara magicamente. FORZANDO TOOL_THINK en el siguiente turno.";
                    current_context.push_str(&format!("{}\n\n", msg));
                    emit_event(&app_handle, runtime.current_step(), msg, "WARNING");
                    forced_next_tool = Some((
                        "TOOL_THINK".to_string(),
                        "El mapper ha terminado. Iniciar ejecución de plan.".to_string(),
                    ));
                } else {
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "🗺️ Iniciando análisis de dependencias del workspace...",
                        "ACTION",
                    );
                    let graph = crate::core::dependency_mapper::analyze_workspace(&workspace_path);
                    let report = crate::core::dependency_mapper::format_graph_report(&graph);
                    let summary = format!(
                        "📊 Grafo generado: {} archivos | {} dependencias | {} nodos críticos | {} ciclos detectados",
                        graph.nodes.len(),
                        graph.edges.len(),
                        graph.god_nodes.len(),
                        graph.cycles.len()
                    );
                    if graph.nodes.is_empty() {
                        current_context.push_str(&format!(
                            "[TOOL_MAPPER] Análisis completado. Grafo persistido en .aura_graph.json\n\n{}\n\n\
                            [ADVERTENCIA INTERNA]: El workspace está completamente vacío (0 archivos). No hay nada que mapear.\n\
                            Para avanzar, DEBES usar TOOL_THINK para diseñar los archivos a crear, o usar TOOL_AST_INJECT para estructurar el proyecto.\n\n",
                            report
                        ));
                    } else {
                        current_context.push_str(&format!(
                            "[TOOL_MAPPER] Análisis completado. Grafo persistido en .aura_graph.json\n\n{}\n\n\
                            [INSTRUCCIÓN CRÍTICA]: El grafo de arriba es la REALIDAD FÍSICA del proyecto. \
                            Sigue el 'Orden de Escritura Recomendado' AL PIE DE LA LETRA. \
                            Usa TOOL_PROGRAMMER para escribir cada archivo en ese orden exacto. \
                            NO empieces por archivos que dependen de otros que aún no existen.\n\n",
                            report
                        ));
                    }
                    emit_event(&app_handle, runtime.current_step(), &summary, "SUCCESS");
                }
            }
            "TOOL_PROGRAMMER" => {
                let mut is_cooldown_blocked = false;

                // Block only if the LLM tries to edit ALREADY-EDITED files TWICE IN A ROW
                // WITHOUT running any terminal command in between.
                if !archivos_editados_historico.is_empty()
                    && comandos_ejecutados_historico.is_empty()
                {
                    is_cooldown_blocked = true;
                    // If there is at least one NEW file in the list, allow the action
                    if archivos_vec.is_empty() {
                        is_cooldown_blocked = false;
                    }
                    for f in &archivos_vec {
                        if !archivos_editados_historico.contains(f) {
                            is_cooldown_blocked = false;
                            break;
                        }
                    }
                }
                // FALLO FIX: Limit frontend cooldown exemption to max 2 cycles.
                let mut is_all_frontend = true;
                for f in &archivos_vec {
                    if !f.ends_with(".html") && !f.ends_with(".css") && !f.ends_with(".js") {
                        is_all_frontend = false;
                    }
                }
                if is_cooldown_blocked
                    && is_all_frontend
                    && !archivos_vec.is_empty()
                    && programmer_cooldown_hits < 2
                {
                    is_cooldown_blocked = false;
                }

                // CRITICAL FIX: Do NOT block TOOL_PROGRAMMER if the workspace has compilation/syntax errors.
                // The agent must be allowed to fix broken syntax before running tests!
                if is_cooldown_blocked && validate_workspace(&workspace_path).await.is_err() {
                    is_cooldown_blocked = false;
                }
                if is_cooldown_blocked
                    && (!verifier_diagnostic.is_empty()
                        || forced_override
                            .as_ref()
                            .is_some_and(|(forced, _)| forced == "TOOL_PROGRAMMER"))
                {
                    is_cooldown_blocked = false;
                }

                if is_cooldown_blocked {
                    programmer_cooldown_hits += 1;
                    if programmer_cooldown_hits >= 3 {
                        let res_msg = "[SISTEMA INTERCEPTO] Error Crítico: Bucle infinito de TOOL_PROGRAMMER detectado. Abortando misión.";
                        emit_event(&app_handle, runtime.current_step(), res_msg, "FATAL");
                        let final_res = FinalResponse {
                            status: "ERROR".to_string(),
                            respuesta_conversacional: format!("Me he quedado atascado editando repetidamente el mismo archivo ({:?}) sin probarlo en la terminal. He detenido la ejecución por seguridad.", archivos_vec),
                        };
                        crate::llm::router::record_model_result(
                            &orchestrator_model,
                            &crate::llm::router::TaskType::Orchestrator,
                            final_res.status == "FINISH",
                            runtime.current_step(),
                        );
                        return Ok(serde_json::to_string(&final_res).unwrap());
                    } else {
                        let interception = "[SISTEMA INTERCEPTO] Error Lógico: Estás intentando editar los mismos archivos por segunda vez consecutiva sin haber probado tu código en la terminal. DEBES ejecutar 'TOOL_TERMINAL' para probar el script y ver los errores antes de seguir programando.";
                        current_context.push_str(&format!("{}\n\n", interception));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "Bucle interceptado por Cooldown",
                            "WARNING",
                        );
                        forced_next_tool = Some(("TOOL_TERMINAL".to_string(), "Se forzó 'TOOL_TERMINAL'. DEBES ejecutar el script en la terminal para probarlo ahora mismo antes de seguir programando. RECUERDA: Debes proporcionar un comando válido en el campo 'comando' (ej. 'python main.py', 'npm start', o 'ls'). NO DEJES EL COMANDO VACÍO.".to_string()));
                    }
                } else {
                    // Valid programming action.
                    comandos_ejecutados_historico.clear();
                    comandos_exitosos_historico.clear();

                    // Keep the selected local coder throughout automatic recovery.
                    // On the audited machine, switching to 14B added about 189 seconds
                    // to one repair and still returned an incoherent verifier.
                    let preferred_programmer = &programmer_model;
                    let target_model = resolve_model_or_fallback(
                        if !preferred_programmer.is_empty()
                            && !preferred_programmer.to_lowercase().contains("embed")
                        {
                            preferred_programmer
                        } else {
                            &orchestrator_model
                        },
                        &available_models,
                    );
                    if target_model != programmer_model {
                        emit_event(&app_handle, runtime.current_step(), &format!("[MODELO] Reparación escalada a {} después de un fallo real con {}.", target_model, programmer_model), "INFO");
                    }

                    let mut prog_args = raw_value.clone();
                    if let Some(obj) = prog_args.as_object_mut() {
                        obj.insert(
                            "model".to_string(),
                            serde_json::Value::String(target_model.clone()),
                        );
                        obj.insert(
                            "context".to_string(),
                            serde_json::Value::String(if verifier_diagnostic.is_empty() {
                                format!(
                                    "{}\n[HISTORIAL RECIENTE]\n{}",
                                    runtime.state_anchor.format_prompt_block(),
                                    recent_context(&current_context, 3500)
                                )
                            } else {
                                format!("DIAGNÓSTICO SEMÁNTICO VIGENTE:\n{}", verifier_diagnostic)
                            }),
                        );
                        if !obj.contains_key("instruccion")
                            && !obj.contains_key("prompt")
                            && !obj.contains_key("task")
                        {
                            obj.insert(
                                "instruccion".to_string(),
                                serde_json::Value::String(user_message.clone()),
                            );
                        }
                        if !archivos_vec.is_empty() {
                            obj.insert(
                                "archivos_a_editar".to_string(),
                                serde_json::json!(archivos_vec),
                            );
                        }
                    }

                    let require_all_targets = prog_args
                        .get("require_all_targets")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false);
                    let file_instruction = if verifier_diagnostic.is_empty() {
                        format!(
                            "Incluye los entregables necesarios de esta lista: {:?}.",
                            archivos_vec
                        )
                    } else if require_all_targets {
                        format!("El auditor comprobó un defecto en cada archivo de esta lista: {:?}. Corrige todos los archivos enumerados en esta misma propuesta y devuelve exactamente un cambio por cada uno; no omitas ni cambies sus rutas.", archivos_vec)
                    } else {
                        format!("Puedes corregir cualquiera de estos archivos relacionados: {:?}. Devuelve únicamente los que necesiten cambios para resolver el diagnóstico real.", archivos_vec)
                    };
                    let pending_criteria = runtime.contract.pending_required_criteria();
                    let repair_blueprint = if approved_consultation_task && journal.fase_actual == 1
                    {
                        consultation_firebase_blueprint()
                    } else {
                        tactical_repair_blueprint(&original_prompt_parsed)
                    };
                    prog_args["instruccion"] = serde_json::json!(format!(
                        "Objetivo global: {}\nAcción actual: {}\n{} No reescribas otros entregables.\nRequisitos obligatorios aún pendientes:\n- {}\nGuía concreta del runtime: {}\nDiagnóstico de la operación anterior: {}\nÚltima verificación fallida: {}",
                        original_prompt_parsed, pensamiento, file_instruction,
                        if pending_criteria.is_empty() { "ninguno".to_string() } else { pending_criteria.join("\n- ") },
                        repair_blueprint,
                        runtime.state_anchor.last_error.as_deref().unwrap_or("ninguno"), verifier_diagnostic));
                    prog_args["repair_attempt"] = serde_json::json!(programmer_failures);
                    if programmer_failures > 0 {
                        if let Some(error) = runtime.state_anchor.last_error.as_deref() {
                            prog_args["repair_error"] = serde_json::json!(error);
                        }
                    }
                    let node_runtime_repair = last_failed_test_run
                        .as_ref()
                        .map(|(_, output)| {
                            is_node_test_jest_mock_failure(output)
                                || is_node_test_undefined_push_failure(output)
                                || is_node_test_uninitialized_storage_mock_failure(output)
                                || is_node_test_browser_document_failure(output)
                        })
                        .unwrap_or(false);
                    prog_args["semantic_repair"] =
                        serde_json::json!(!verifier_diagnostic.is_empty() && !node_runtime_repair);

                    let prog_proposal = crate::core::policy::ActionProposal {
                        tool: "TOOL_PROGRAMMER".to_string(),
                        arguments: prog_args,
                        expected_effect: pensamiento.clone(),
                        risk: crate::core::policy::RiskLevel::Safe,
                        world_hash: Some(runtime.current_world_hash()),
                    };

                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!(
                            "[ROUTER] Delegando a ProgrammerExecutor (modelo: {})...",
                            target_model
                        ),
                        "INFO",
                    );

                    match runtime.execute_action(&prog_proposal).await {
                        Ok(obs) => {
                            if obs.status == crate::core::observation::ObservationStatus::Success {
                                let written_files = obs.files_affected.clone();
                                for f in &written_files {
                                    let full_path = std::path::Path::new(&workspace_path).join(f);
                                    let _ = app_handle.emit(
                                        "file-updated",
                                        serde_json::json!({
                                            "path": full_path.to_string_lossy().to_string()
                                        }),
                                    );
                                    archivos_editados_historico.insert(f.clone());
                                    current_context.push_str(&format!(
                                        "[HECHO INMUTABLE — NO IGNORAR]: El archivo '{}' fue creado/modificado exitosamente en el paso {}. NO debe recrearse ni editarse sin razón técnica explícita.\n",
                                        f, runtime.current_step()
                                    ));
                                }

                                // A file write is progress, not proof that a phase works.
                                // The Critic's phase gate decides completion after validation.
                                if !journal.fases.is_empty() {
                                    if let Some(fase) = journal.fases.get_mut(journal.fase_actual) {
                                        fase.estado = "EN_PROGRESO".to_string();
                                        emit_event(
                                            &app_handle,
                                            runtime.current_step(),
                                            &format!(
                                                "[PESP] Archivos de la fase actualizados; pendientes de validación: {}",
                                                fase.descripcion
                                            ),
                                            "INFO",
                                        );
                                    }
                                    if let Err(e) = crate::core::session_journal::save_journal(
                                        &workspace_path,
                                        &journal,
                                    ) {
                                        emit_event(
                                            &app_handle,
                                            runtime.current_step(),
                                            &format!("[CHECKPOINT FAILED] {}", e),
                                            "FATAL",
                                        );
                                        let final_res = FinalResponse {
                                            status: "ERROR".to_string(),
                                            respuesta_conversacional: format!("[PERSISTENCE_FAILURE] Error crítico al actualizar fases: {}. Misión abortada.", e),
                                        };
                                        return Ok(serde_json::to_string(&final_res).unwrap());
                                    }
                                }

                                // Advance micro-metas in journal (legacy fallback)
                                if !journal.micro_metas.is_empty() {
                                    if let Some(mm) =
                                        journal.micro_metas.get_mut(journal.micro_meta_actual)
                                    {
                                        let all_done = mm.archivos.iter().all(|f| {
                                            written_files
                                                .iter()
                                                .any(|w: &String| w.contains(f.as_str()))
                                        });
                                        if all_done {
                                            mm.estado = "VERIFICADA".to_string();
                                            emit_event(
                                                &app_handle,
                                                runtime.current_step(),
                                                &format!(
                                                    "[PESP] ✅ Micro-Meta [{}/{}] VERIFICADA.",
                                                    journal.micro_meta_actual + 1,
                                                    journal.micro_metas.len()
                                                ),
                                                "SUCCESS",
                                            );
                                            if journal.micro_meta_actual + 1
                                                < journal.micro_metas.len()
                                            {
                                                journal.micro_meta_actual += 1;
                                                let next = journal.micro_metas
                                                    [journal.micro_meta_actual]
                                                    .descripcion
                                                    .clone();
                                                emit_event(&app_handle, runtime.current_step(), &format!("[PESP] 🔄 Avanzando a Micro-Meta [{}/{}]: {}", journal.micro_meta_actual + 1, journal.micro_metas.len(), next), "INFO");
                                            }
                                        } else {
                                            mm.estado = "EN_PROGRESO".to_string();
                                        }
                                        if let Err(e) = crate::core::session_journal::save_journal(
                                            &workspace_path,
                                            &journal,
                                        ) {
                                            emit_event(
                                                &app_handle,
                                                runtime.current_step(),
                                                &format!("[CHECKPOINT FAILED] {}", e),
                                                "FATAL",
                                            );
                                            let final_res = FinalResponse {
                                                status: "ERROR".to_string(),
                                                respuesta_conversacional: format!("[PERSISTENCE_FAILURE] Error crítico al actualizar micro-metas: {}. Misión abortada.", e),
                                            };
                                            return Ok(serde_json::to_string(&final_res).unwrap());
                                        }
                                    }
                                }

                                programmer_failures = 0;
                                runtime.recovery.record_success("TOOL_PROGRAMMER");
                                let workspace_progress = workspace_changed_after_programmer(
                                    &written_files,
                                    obs.physical_files_changed.unwrap_or(0),
                                );
                                if workspace_progress {
                                    if approved_consultation_task {
                                        reset_consultation_audit_stall_tracking(
                                            &mut last_consultation_audit_world,
                                            &mut repeated_consultation_audit_without_change,
                                            &mut last_consultation_audit_diagnostic,
                                        );
                                    }
                                    emit_event(
                                        &app_handle,
                                        runtime.current_step(),
                                        "[RECOVERY] Se confirmó un cambio físico en el workspace; los fallos terminales previos seguirán contando hasta que una comprobación termine correctamente.",
                                        "INFO",
                                    );
                                }
                                // Sprint 2: Phase/Micrometa-gated Executor->Critic transition
                                let has_phases = !journal.fases.is_empty();
                                let all_metas_done = journal.micro_metas.is_empty()
                                    || journal
                                        .micro_metas
                                        .iter()
                                        .all(|mm| mm.estado == "VERIFICADA");
                                if has_phases || all_metas_done {
                                    current_role = AgentRole::Critic;
                                    critic_feedback = None;
                                    emit_event(&app_handle, runtime.current_step(), "[FSM] EJECUTOR -> CRITICO: escritura completada. Validando entregables y verificaciones pendientes; la fase aún no está completada.", "INFO");
                                }

                                let explicit_msg = format!("Programador: Los archivos {:?} fueron escritos con éxito, Anti-Stub APROBADO.\nREGLA DE ESTADO: Los archivos ya existen físicamente. Ejecuta el criterio de aceptación de la fase o del contrato con TOOL_TERMINAL. Si falla, corrige el archivo causante. Si no hay criterio ni pruebas, crea una prueba del comportamiento real usando el lenguaje y runner del proyecto; no añadas un verificador Python a un proyecto de otro lenguaje. No repitas comandos ni asumas que un servidor local está activo.\n\n", written_files);
                                current_context.push_str(&explicit_msg);
                                last_progress_step = runtime.current_step();
                                comandos_ejecutados_historico.clear();
                                _no_tests_consecutive = 0;
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    &if written_files.len() == 1 {
                                        "Programación exitosa: 1 archivo afectado".to_string()
                                    } else {
                                        format!(
                                            "Programación exitosa: {} archivos afectados",
                                            written_files.len()
                                        )
                                    },
                                    "SUCCESS",
                                );
                                if let Some(command) = official_verifier_command(&runtime.contract)
                                {
                                    current_role = AgentRole::Critic;
                                    forced_next_tool = Some((
                                        "TOOL_TERMINAL".to_string(),
                                        format!("Ejecuta el verificador oficial '{}'.", command),
                                    ));
                                    emit_event(
                                        &app_handle,
                                        runtime.current_step(),
                                        &format!(
                                            "[RUNTIME ROUTER] Código escrito; la siguiente acción será verificar con '{}'.",
                                            command
                                        ),
                                        "INFO",
                                    );
                                }
                            } else {
                                programmer_failures += 1;
                                let recovery = runtime.handle_observation(&obs);
                                if programmer_failures >= 3 {
                                    let message = format!("PROGRAMMER_REPAIR_EXHAUSTED: {} intentos reales fallidos. Último diagnóstico: {}. Borrador conservado en .aura/programmer_failure.json.", programmer_failures, obs.payload);
                                    emit_event(
                                        &app_handle,
                                        runtime.current_step(),
                                        &message,
                                        "ERROR",
                                    );
                                    persist_and_learn_failure!(&message);
                                    return Ok(serde_json::json!({"status":"ERROR", "respuesta_conversacional":message}).to_string());
                                }
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    &format!(
                                        "[RECUPERACIÓN] Intento {}/3. Decisión: {:?}",
                                        programmer_failures, recovery
                                    ),
                                    "WARNING",
                                );
                                runtime.state_anchor.last_error = Some(obs.payload.clone());
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    &format!("Error detectado en programación: {}", obs.payload),
                                    "ERROR",
                                );
                                current_context.push_str(&format!("Programador: Fracasó con error:\n{}\n[SISTEMA]: Corrige este error en el próximo paso con TOOL_PROGRAMMER.\n\n", obs.payload));
                                current_role = AgentRole::Executor;
                                forced_next_tool = Some((
                                    "TOOL_PROGRAMMER".to_string(),
                                    format!(
                                        "Reparación {} de 3. Corrige exactamente este fallo y conserva el borrador útil: {}",
                                        programmer_failures + 1,
                                        obs.payload
                                    ),
                                ));
                            }
                        }
                        Err(e) => {
                            programmer_failures += 1;
                            if programmer_failures >= 3 {
                                let message = format!("PROGRAMMER_GATE_BLOCKED: {}. La misma vía falló {} veces; ejecución detenida sin declarar éxito.", e, programmer_failures);
                                emit_event(&app_handle, runtime.current_step(), &message, "ERROR");
                                persist_and_learn_failure!(&message);
                                return Ok(serde_json::json!({"status":"ERROR", "respuesta_conversacional":message}).to_string());
                            }
                            runtime.state_anchor.last_error = Some(e.clone());
                            current_context.push_str(&format!(
                                "Error ejecutando acción de programación: {}\n\n",
                                e
                            ));
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                &format!("Error TOOL_PROGRAMMER: {}", e),
                                "ERROR",
                            );
                            current_role = AgentRole::Executor;
                            forced_next_tool = Some((
                                "TOOL_PROGRAMMER".to_string(),
                                format!(
                                    "Reparación {} de 3. Corrige exactamente este fallo: {}",
                                    programmer_failures + 1,
                                    e
                                ),
                            ));
                        }
                    }
                }
            }

            "TOOL_ARCHITECT" => {
                if architect_used {
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "Bucle interceptado por Cooldown (Architect)",
                        "WARNING",
                    );
                    current_context.push_str(&format!("PASO {}:\nAcción: TOOL_ARCHITECT\nResultado: [SISTEMA INTERCEPTO] Error: Ya ejecutaste TOOL_ARCHITECT en este bucle. Tu única opción válida ahora es usar TOOL_FINISH para detenerte y resumir los resultados al usuario.\n\n", runtime.current_step()));
                } else {
                    architect_used = true;
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "Generando mapa arquitectónico del sistema...",
                        "ACTION",
                    );
                    let graph = crate::core::dependency_mapper::analyze_workspace(&workspace_path);
                    let report = crate::core::dependency_mapper::format_graph_report(&graph);
                    current_context.push_str(&format!("Reporte Arquitectónico:\n{}\n\n", report));
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "Mapa arquitectónico generado.",
                        "SUCCESS",
                    );
                }
            }
            "TOOL_VISION_EVALUATOR" => {
                if last_local_ui_url.is_none() {
                    if let Some((next_tool, reason)) = local_http_server_recovery_action(
                        &original_prompt_parsed,
                        &workspace_path,
                        last_http_server_task_id.as_deref(),
                    ) {
                        current_role = if next_tool == "TOOL_PROGRAMMER" {
                            AgentRole::Executor
                        } else {
                            AgentRole::Critic
                        };
                        current_context.push_str(&format!(
                            "[SERVIDOR HTTP REQUERIDO] La evaluación visual no puede usar file://; primero confirma HTTP localhost. Siguiente acción: {}.\n\n",
                            reason
                        ));
                        forced_next_tool = Some((next_tool.clone(), reason));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "[SERVIDOR HTTP REQUERIDO] La URL local todavía no está confirmada.",
                            "WARNING",
                        );
                        continue;
                    }
                }
                visual_validation_required |=
                    mission_requires_visual_verification(&original_prompt_parsed, &workspace_path);
                visual_validation_attempts = visual_validation_attempts.saturating_add(1);
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    if visual_validation_required {
                        "[VISION] Abriendo la URL real del proyecto y evaluando la captura..."
                    } else {
                        "[VISION] Evaluando la captura solicitada..."
                    },
                    "ACTION",
                );
                let vision_prompt = format!(
                    "Objetivo original: {}\nFase actual: {}\n{}",
                    original_prompt_parsed,
                    journal
                        .fases
                        .get(journal.fase_actual)
                        .map(|phase| phase.descripcion.as_str())
                        .unwrap_or("revisión visual de la misión"),
                    if comando.trim().is_empty() {
                        "Compara la pantalla con el objetivo. Indica evidencia visible, errores, elementos faltantes y limitaciones de una sola captura."
                    } else {
                        comando.as_str()
                    }
                );
                let url = extract_local_ui_url(&comando)
                    .or_else(|| last_local_ui_url.clone())
                    .or_else(|| extract_explicit_http_url(&comando))
                    .or_else(|| extract_explicit_http_url(&user_message));

                let action_proposal = crate::core::policy::ActionProposal {
                    tool: "TOOL_VISION_EVALUATOR".to_string(),
                    arguments: serde_json::json!({
                        "prompt": vision_prompt,
                        "url": url,
                        "require_target": visual_validation_required,
                    }),
                    expected_effect: "Visual UI evaluation".to_string(),
                    risk: crate::core::policy::RiskLevel::Safe,
                    world_hash: Some(runtime.current_world_hash()),
                };

                match runtime.execute_action(&action_proposal).await {
                    Ok(obs) => {
                        if obs.status == crate::core::observation::ObservationStatus::Success {
                            if visual_validation_required {
                                runtime.observe_world()?;
                                visual_validation_world_hash = Some(runtime.current_world_hash());
                                visual_validation_error = None;
                                visual_validation_result = Some(obs.payload.clone());
                                visual_validation_error_world_hash = None;
                            }
                            mandatory_tools_executed.insert("TOOL_VISION_EVALUATOR".to_string());
                            let vision_follow_up = if scoped_visual_review {
                                "Describe los hallazgos visibles y sus límites; esta petición es de solo lectura, así que no modifiques archivos. Conserva también el resultado de las pruebas funcionales disponibles."
                            } else {
                                "Si reporta defectos visibles, corrige el entregable afectado y vuelve a validar la versión actual. Una captura solo demuestra lo que se ve en esa página; conserva también las pruebas funcionales de los flujos solicitados."
                            };
                            current_context.push_str(&format!(
                                "[VISION EVALUATOR RESULTADO]\n{}\n\n[INSTRUCCIÓN]: Inspecciona el veredicto y sus límites. {}\n\n",
                                obs.payload, vision_follow_up
                            ));
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                &format!(
                                    "[VISION] Evaluacion completada: {}",
                                    &obs.payload.chars().take(120).collect::<String>()
                                ),
                                "SUCCESS",
                            );
                            if browser_interaction_validation_required
                                && browser_interaction_validation_world_hash
                                    != Some(runtime.current_world_hash())
                            {
                                if let Some(confirmed_url) =
                                    last_local_ui_url.as_deref().or(url.as_deref())
                                {
                                    let guidance = browser_interaction_test_guidance(
                                        confirmed_url,
                                        &original_prompt_parsed,
                                    );
                                    current_role = AgentRole::Critic;
                                    forced_next_tool =
                                        Some(("TOOL_TESTER".to_string(), guidance.clone()));
                                    current_context.push_str(&format!(
                                        "[SIGUIENTE VALIDACIÓN OBLIGATORIA]: La revisión visual pasó, pero faltan pruebas de interacción. {}\n\n",
                                        guidance
                                    ));
                                }
                            }
                        } else {
                            let msg =
                                format!("[VISION] Evaluación visual no aprobada: {}", obs.payload);
                            visual_validation_world_hash = None;
                            visual_validation_error = Some(obs.payload.clone());
                            visual_validation_result = Some(obs.payload.clone());
                            runtime.observe_world()?;
                            visual_validation_error_world_hash = Some(runtime.current_world_hash());
                            current_context.push_str(&format!("{}\n\n", &msg));
                            emit_event(&app_handle, runtime.current_step(), &msg, "ERROR");
                            if visual_validation_required
                                && visual_validation_attempts < 3
                                && !scoped_visual_review
                                && obs.payload.contains("VISUAL_QA_REJECTED")
                            {
                                current_role = AgentRole::Executor;
                                forced_next_tool = Some((
                                    "TOOL_THINK".to_string(),
                                    format!("La revisión de la captura real rechazó el aspecto o encontró requisitos visibles ausentes. Corrige los defectos descritos, conserva los flujos que ya funcionan y luego vuelve a ejecutar pruebas y una captura nueva. Diagnóstico visual: {}", obs.payload),
                                ));
                            } else if visual_validation_required
                                && visual_validation_attempts < 3
                                && (obs.payload.contains("VISUAL_TARGET_MISSING")
                                    || obs.payload.contains("VISUAL_CAPTURE_FAILED"))
                            {
                                current_role = AgentRole::Executor;
                                let (tool, guidance) = if let Some(task_id) =
                                    last_background_task_id.as_deref()
                                {
                                    (
                                        "TOOL_BACKGROUND_READ",
                                        format!("Lee los logs del servidor ya iniciado (task_id={task_id}), identifica su URL localhost real y continúa con la captura. No inventes el puerto."),
                                    )
                                } else {
                                    (
                                        "TOOL_THINK",
                                        format!("Resuelve el requisito de captura local usando el error. Revisa el comando de inicio disponible para este proyecto, inicia el frontend mediante TOOL_BACKGROUND_START si necesita servidor, confirma la URL en sus logs y luego vuelve a evaluar. Error: {}", obs.payload),
                                    )
                                };
                                forced_next_tool = Some((tool.to_string(), guidance));
                            }
                        }
                    }
                    Err(e) => {
                        let msg = format!("[VISION] Error de ejecución: {}", e);
                        visual_validation_world_hash = None;
                        visual_validation_error = Some(e.clone());
                        visual_validation_result = Some(e.clone());
                        runtime.observe_world()?;
                        visual_validation_error_world_hash = Some(runtime.current_world_hash());
                        current_context.push_str(&format!("{}\n\n", &msg));
                        emit_event(&app_handle, runtime.current_step(), &msg, "ERROR");
                        if visual_validation_required
                            && visual_validation_attempts < 3
                            && (e.contains("VISUAL_TARGET_MISSING")
                                || e.contains("VISUAL_CAPTURE_FAILED"))
                        {
                            current_role = AgentRole::Executor;
                            let (tool, guidance) = if let Some(task_id) =
                                last_background_task_id.as_deref()
                            {
                                (
                                    "TOOL_BACKGROUND_READ",
                                    format!("Lee los logs del servidor iniciado con task_id={task_id}, encuentra su URL localhost y vuelve a validar la página real."),
                                )
                            } else {
                                (
                                    "TOOL_THINK",
                                    format!("Resuelve la captura local: consulta el comando de inicio del proyecto, arranca el servidor con TOOL_BACKGROUND_START, confirma la URL real en sus logs y vuelve a evaluar. Error: {e}"),
                                )
                            };
                            forced_next_tool = Some((tool.to_string(), guidance));
                        }
                    }
                }
            }
            "TOOL_TESTER" => {
                let browser_test_requested =
                    crate::core::browser_automation::is_browser_test_command(&comando);
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    if browser_test_requested {
                        "Probando flujos reales en Chrome/Edge local..."
                    } else {
                        "Ejecutando suite de pruebas automatizadas..."
                    },
                    "ACTION",
                );
                if browser_test_requested {
                    let requested_url =
                        crate::core::browser_automation::browser_test_plan_url(&comando);
                    let confirmed_url = last_local_ui_url.as_deref();
                    let is_confirmed_target = requested_url.as_deref().ok().is_some_and(|url| {
                        confirmed_url
                            .is_some_and(|confirmed| same_local_server_origin(url, confirmed))
                    });
                    if !is_confirmed_target {
                        let (next_tool, guidance) = web_validation_recovery_action(
                            &original_prompt_parsed,
                            &workspace_path,
                            confirmed_url,
                            last_http_server_task_id.as_deref(),
                        )
                        .unwrap_or_else(|| (
                            "TOOL_BACKGROUND_START".into(),
                            "Inicia un servidor local para el frontend y confirma su URL mediante los logs antes de probarlo.".into(),
                        ));
                        current_role = AgentRole::Critic;
                        forced_next_tool = Some((next_tool.clone(), guidance.clone()));
                        let message = format!(
                            "[BROWSER_TEST_BLOCKED] La URL solicitada no coincide con una URL localhost confirmada por los logs. No se abrió ninguna página externa. Siguiente paso: {next_tool} — {guidance}"
                        );
                        current_context.push_str(&format!("{message}\n\n"));
                        emit_event(&app_handle, runtime.current_step(), &message, "WARNING");
                        continue;
                    }
                }
                match runtime.execute_action(&action_proposal).await {
                    Ok(obs) => {
                        if browser_test_requested {
                            // A real browser run is fresh behavioral evidence,
                            // including a failed run with actionable diagnostics.
                            programmer_calls_since_validation = 0;
                            programmer_stall_recoveries = 0;
                        }
                        if obs.status == crate::core::observation::ObservationStatus::Success {
                            tester_attempts = 0;
                            if browser_interaction_validation_required && !browser_test_requested {
                                let guidance = last_local_ui_url.as_deref().map(|url| {
                                    browser_interaction_test_guidance(url, &original_prompt_parsed)
                                });
                                if let Some(guidance) = guidance {
                                    current_role = AgentRole::Critic;
                                    forced_next_tool =
                                        Some(("TOOL_TESTER".to_string(), guidance.clone()));
                                    current_context.push_str(&format!(
                                        "[PRUEBAS DE LÓGICA PASARON, FALTA EL NAVEGADOR]: {}\n\n",
                                        guidance
                                    ));
                                    emit_event(
                                        &app_handle,
                                        runtime.current_step(),
                                        "Las pruebas del proyecto pasaron; siguen pendientes los flujos interactivos del navegador.",
                                        "WARNING",
                                    );
                                } else {
                                    current_role = AgentRole::Critic;
                                    if let Some((next_tool, guidance)) =
                                        web_validation_recovery_action(
                                            &original_prompt_parsed,
                                            &workspace_path,
                                            None,
                                            last_http_server_task_id.as_deref(),
                                        )
                                    {
                                        forced_next_tool = Some((next_tool, guidance));
                                    }
                                    current_context.push_str(
                                        "[PRUEBAS DE LÓGICA PASARON]: falta confirmar el servidor local para probar la interfaz interactiva.\n\n",
                                    );
                                }
                            } else if tester_success_hits >= 1 {
                                let res_msg = "[SISTEMA INTERCEPTO] Error Crítico: Bucle infinito de pruebas exitosas detectado. Abortando misión.";
                                emit_event(&app_handle, runtime.current_step(), res_msg, "FATAL");
                                let final_res = FinalResponse {
                                    status: "ERROR".to_string(),
                                    respuesta_conversacional: "Los tests ya pasaron con éxito, pero me quedé atascado ejecutándolos en bucle. He detenido el proceso para evitar un ciclo infinito. El cierre de la misión no está verificado.".to_string(),
                                };
                                crate::llm::router::record_model_result(
                                    &orchestrator_model,
                                    &crate::llm::router::TaskType::Orchestrator,
                                    final_res.status == "FINISH",
                                    runtime.current_step(),
                                );
                                return Ok(serde_json::to_string(&final_res).unwrap());
                            } else {
                                tester_success_hits += 1;
                                runtime.cognitive_state.metrics.successful_verifications += 1;
                                if browser_test_requested {
                                    browser_interaction_validation_world_hash =
                                        Some(runtime.current_world_hash());
                                }
                                mandatory_tools_executed.insert("TOOL_TESTER".to_string());
                                current_context.push_str(&format!("Resultado Tests:\n{}\n\n[INSTRUCCIÓN ESTRICTA DE SEGURIDAD]: La comprobación ejecutada pasó. Revisa qué cubrió y cuáles criterios siguen sin evidencia; no declares completa la misión si queda alguna prueba o revisión requerida. No repitas exactamente el mismo runner sobre el mismo estado.\n\n", obs.payload));
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    if browser_test_requested {
                                        "Los flujos interactivos del navegador pasaron."
                                    } else {
                                        "La suite de pruebas pasó."
                                    },
                                    "SUCCESS",
                                );
                            }
                        } else {
                            if browser_test_requested
                                && (obs.payload.starts_with("BROWSER_AUTOMATION_UNAVAILABLE")
                                    || obs.payload.starts_with("BROWSER_TEST_TIMEOUT"))
                            {
                                let message = format!(
                                    "BROWSER_VALIDATION_BLOCKED: no se pudo ejecutar el navegador automatizado. {}. El proyecto y los archivos se conservaron; la misión quedó disponible para reanudarse.",
                                    obs.payload
                                );
                                emit_event(&app_handle, runtime.current_step(), &message, "ERROR");
                                let _ = save_resumable_validation_block(
                                    &mut journal,
                                    &workspace_path,
                                    runtime.current_step(),
                                    "Critic",
                                    &current_context,
                                    &message,
                                );
                                let final_res = FinalResponse {
                                    status: "ERROR".to_string(),
                                    respuesta_conversacional: message,
                                };
                                return Ok(serde_json::to_string(&final_res).unwrap());
                            }
                            if let Some(crate::core::recovery::RecoveryDecision::RepairCriteria {
                                criteria,
                                recommended_tool,
                            }) = runtime.handle_observation(&obs)
                            {
                                emit_event(&app_handle, runtime.current_step(), &format!("[SEMANTIC_VERIFICATION] Fallo detectado. Forzando transición a {}.", recommended_tool), "WARNING");
                                current_role = AgentRole::Executor;
                                forced_next_tool = Some((
                                    recommended_tool.clone(),
                                    format!("Corrige los siguientes criterios semánticos que fallaron:\n- {}", criteria.join("\n- "))
                                ));
                                continue;
                            }

                            tester_attempts += 1;
                            if tester_attempts >= 3 {
                                emit_event(&app_handle, runtime.current_step(), "[CRITICAL_FAILURE] Los fallos de test alcanzaron el límite (3). Deteniendo la vía de pruebas.", "FATAL");
                                emit_event(&app_handle, runtime.current_step(), "[CONSERVACIÓN] Se mantienen los archivos actuales y el diagnóstico; no se ejecuta un rollback global.", "WARNING");
                                let final_res = FinalResponse {
                                    status: "ERROR".to_string(),
                                    respuesta_conversacional: "He alcanzado el límite máximo de fallos de pruebas. Se conservan los archivos y el diagnóstico para corregirlos; no se ha verificado la finalización.".to_string(),
                                };
                                crate::llm::router::record_model_result(
                                    &orchestrator_model,
                                    &crate::llm::router::TaskType::Orchestrator,
                                    final_res.status == "FINISH",
                                    runtime.current_step(),
                                );
                                return Ok(serde_json::to_string(&final_res).unwrap());
                            } else {
                                let fail_msg = &obs.payload;
                                let is_dep_error = fail_msg.contains("Cannot find module")
                                    || fail_msg.contains("jest: command not found")
                                    || fail_msg.contains("not recognized")
                                    || fail_msg.contains("[ENV_FAILURE]");

                                if is_dep_error {
                                    tester_attempts -= 1;
                                    let binary = if fail_msg.contains("[ENV_FAILURE]") {
                                        fail_msg.split('\'').nth(1).unwrap_or("").trim()
                                    } else {
                                        ""
                                    };
                                    if !binary.is_empty()
                                        && !paquetes_instalados_historico.contains(binary)
                                    {
                                        emit_event(&app_handle, runtime.current_step(),
                                            &format!("[AUTO-ENV] Tester detectó binario faltante '{}'. Instalando automáticamente...", binary),
                                            "WARNING");
                                        paquetes_instalados_historico.insert(binary.to_string());

                                        let env_prop = crate::core::policy::ActionProposal {
                                            tool: "TOOL_ENV_MANAGER".to_string(),
                                            arguments: serde_json::json!({ "package": binary }),
                                            expected_effect: format!(
                                                "Auto-install missing test binary {}",
                                                binary
                                            ),
                                            risk: crate::core::policy::RiskLevel::Moderate,
                                            world_hash: Some(runtime.current_world_hash()),
                                        };
                                        if let Ok(env_obs) = runtime.execute_action(&env_prop).await
                                        {
                                            if env_obs.status == crate::core::observation::ObservationStatus::Success {
                                                archivos_editados_historico.clear();
                                                comandos_ejecutados_historico.clear();
                                                current_context.push_str(&format!(
                                                    "[AUTO-ENV] Binario '{}' instalado automáticamente: {}\n\n\
                                                     Tu SIGUIENTE PASO OBLIGATORIO es volver a usar TOOL_TESTER.\n\n",
                                                    binary, env_obs.payload
                                                ));
                                                emit_event(&app_handle, runtime.current_step(),
                                                    &format!("'{}' instalado. Reintenta TOOL_TESTER.", binary),
                                                    "SUCCESS");
                                            }
                                        }
                                    } else {
                                        current_context.push_str(&format!(
                                            "[AUTO-FIX DEPENDENCIAS] Los tests fallaron por dependencias faltantes:\n{}\n\n\
                                            En tu próximo turno DEBES ELEGIR 'TOOL_TERMINAL' para instalar dependencias.\n\n",
                                            fail_msg
                                        ));
                                    }
                                } else {
                                    emit_event(&app_handle, runtime.current_step(), "Los tests fallaron. Se conservan los archivos y se activa Auto-Debugger.", "ERROR");
                                    emit_event(&app_handle, runtime.current_step(), "[CONSERVACIÓN] Se mantienen los archivos actuales y el diagnóstico; no se ejecuta un rollback global.", "WARNING");
                                    archivos_editados_historico.clear();
                                    comandos_ejecutados_historico.clear();
                                    current_role = AgentRole::Executor;
                                    critic_feedback = Some(fail_msg.clone());
                                    current_context.push_str(&format!("[AUTO-DEBUGGER] Los tests fallaron:\n{}\n\nLos archivos actuales siguen en disco. Debes corregirlos usando TOOL_PROGRAMMER.\n", fail_msg));
                                    forced_next_tool = Some(("TOOL_PROGRAMMER".to_string(), "Los tests fallaron, el sistema forzó TOOL_PROGRAMMER para corregir los errores.".to_string()));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        current_context
                            .push_str(&format!("Error ejecutando TOOL_TESTER: {}\n\n", e));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &format!("Error TOOL_TESTER: {}", e),
                            "ERROR",
                        );
                    }
                }
            }
            "TOOL_AST_INJECT" => {
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    "Inyectando nodos AST en Memoria Lógica (Zero-Trace)...",
                    "ACTION",
                );
                let nodes = ast_nodes_vec.iter().map(|(intent, parent_id, opcode)| {
                    serde_json::json!({ "intent": intent, "parent_id": parent_id, "opcode": opcode })
                }).collect::<Vec<_>>();
                let ast_proposal = crate::core::policy::ActionProposal {
                    tool: "TOOL_AST_INJECT".to_string(),
                    arguments: serde_json::json!({ "nodes": nodes }),
                    expected_effect:
                        "Insertar los nodos AST solicitados en el buffer compartido de la misión"
                            .to_string(),
                    risk: crate::core::policy::RiskLevel::Safe,
                    world_hash: Some(runtime.current_world_hash()),
                };
                match runtime.execute_action(&ast_proposal).await {
                    Ok(observation)
                        if observation.status
                            == crate::core::observation::ObservationStatus::Success =>
                    {
                        current_context.push_str(&format!(
                            "PASO {}:\nAcción: TOOL_AST_INJECT\nResultado:\n{}\n\n",
                            runtime.current_step(),
                            observation.payload
                        ));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &observation.payload,
                            "SUCCESS",
                        );
                    }
                    Ok(observation) => {
                        current_context.push_str(&format!(
                            "Error en TOOL_AST_INJECT: {}\n\n",
                            observation.payload
                        ));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &observation.payload,
                            "ERROR",
                        );
                    }
                    Err(error) => {
                        current_context
                            .push_str(&format!("Error en TOOL_AST_INJECT: {}\n\n", error));
                        emit_event(&app_handle, runtime.current_step(), &error, "ERROR");
                    }
                }
            }
            "TOOL_LEARN" => {
                learn_consecutive += 1;
                if learn_consecutive > 1 {
                    let msg = "[SISTEMA INTERNO]: Ya has aprendido este proyecto (loop infinito TOOL_LEARN detectado). DEBES USAR TOOL_FINISH INMEDIATAMENTE PARA TERMINAR LA TAREA.";
                    forced_next_tool = Some((
                        "TOOL_FINISH".to_string(),
                        "La memoria ya está indexada, finalizando tarea obligatoriamente."
                            .to_string(),
                    ));
                    current_context.push_str(&format!("{}\n\n", msg));
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "Bucle de TOOL_LEARN detectado, forzando finalización.",
                        "WARNING",
                    );
                } else {
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "Guardando conocimiento en la Memoria Permanente (RAG)...",
                        "ACTION",
                    );
                    match runtime.execute_action(&action_proposal).await {
                        Ok(observation)
                            if observation.status
                                == crate::core::observation::ObservationStatus::Success =>
                        {
                            current_context.push_str(&format!(
                                "Resultado TOOL_LEARN: {}\n\n",
                                observation.payload
                            ));
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                "Memoria indexada correctamente.",
                                "SUCCESS",
                            );
                        }
                        Ok(observation) => {
                            current_context.push_str(&format!(
                                "Error en TOOL_LEARN: {}\n\n",
                                observation.payload
                            ));
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                &observation.payload,
                                "ERROR",
                            );
                        }
                        Err(error) => {
                            current_context
                                .push_str(&format!("Error en TOOL_LEARN: {}\n\n", error));
                            emit_event(&app_handle, runtime.current_step(), &error, "ERROR");
                        }
                    }
                }
            }
            "TOOL_CREATE_RUNNER" => {
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    "🏃 Generando runners de ejecución (test, build, dev, lint)...",
                    "ACTION",
                );
                match runtime.execute_action(&action_proposal).await {
                    Ok(observation)
                        if observation.status
                            == crate::core::observation::ObservationStatus::Success =>
                    {
                        current_context
                            .push_str(&format!("[TOOL_CREATE_RUNNER] {}\n\n", observation.payload));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &observation.payload,
                            "SUCCESS",
                        );
                    }
                    Ok(observation) => {
                        current_context
                            .push_str(&format!("[TOOL_CREATE_RUNNER] {}\n\n", observation.payload));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &observation.payload,
                            "WARNING",
                        );
                    }
                    Err(error) => {
                        current_context
                            .push_str(&format!("[TOOL_CREATE_RUNNER] Error: {}\n\n", error));
                        emit_event(&app_handle, runtime.current_step(), &error, "ERROR");
                    }
                }
            }
            // ── Fase 2: TOOL_CONTAINER (Docker/Podman) ──
            "TOOL_CONTAINER" => {
                let parts: Vec<&str> = comando.splitn(3, ' ').collect();
                if parts.len() < 2 {
                    let err = "Error: El comando TOOL_CONTAINER debe ser 'accion imagen/id [comando]'. Ejemplo: 'run nginx:latest'";
                    current_context.push_str(&format!("Error en TOOL_CONTAINER: {}\n\n", err));
                    emit_event(&app_handle, runtime.current_step(), err, "ERROR");
                } else {
                    let action_str = parts[0];
                    let image_or_id = parts[1];
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!("Contenedor: {} {}", action_str, image_or_id),
                        "ACTION",
                    );

                    let container_proposal = crate::core::policy::ActionProposal {
                        tool: "TOOL_CONTAINER".to_string(),
                        arguments: serde_json::json!({ "comando": comando.clone() }),
                        expected_effect: "Ejecutar la operación de contenedor solicitada usando el workspace autorizado".to_string(),
                        risk: if matches!(action_str.to_lowercase().as_str(), "rm" | "remove") {
                            crate::core::policy::RiskLevel::Moderate
                        } else {
                            crate::core::policy::RiskLevel::Safe
                        },
                        world_hash: Some(runtime.current_world_hash()),
                    };
                    match runtime.execute_action(&container_proposal).await {
                        Ok(observation)
                            if observation.status
                                == crate::core::observation::ObservationStatus::Success =>
                        {
                            current_context.push_str(&format!(
                                "Resultado TOOL_CONTAINER:\n{}\n\n",
                                observation.payload
                            ));
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                &observation.payload,
                                "SUCCESS",
                            );
                            last_progress_step = runtime.current_step();
                        }
                        Ok(observation) => {
                            current_context.push_str(&format!(
                                "Error TOOL_CONTAINER:\n{}\n\n",
                                observation.payload
                            ));
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                &observation.payload,
                                "ERROR",
                            );
                        }
                        Err(error) => {
                            current_context
                                .push_str(&format!("Error TOOL_CONTAINER:\n{}\n\n", error));
                            emit_event(&app_handle, runtime.current_step(), &error, "ERROR");
                        }
                    }
                }
            }
            // ── Fase 4: TOOL_SCHEDULER (Cron tasks) ──
            "TOOL_SCHEDULER" => {
                let scheduler_proposal = crate::core::policy::ActionProposal {
                    tool: "TOOL_SCHEDULER".to_string(),
                    arguments: serde_json::json!({ "comando": comando.clone() }),
                    expected_effect: "Validar y guardar una tarea programada en el archivo persistente del scheduler".to_string(),
                    risk: crate::core::policy::RiskLevel::Safe,
                    world_hash: Some(runtime.current_world_hash()),
                };
                match runtime.execute_action(&scheduler_proposal).await {
                    Ok(observation)
                        if observation.status
                            == crate::core::observation::ObservationStatus::Success =>
                    {
                        current_context.push_str(&format!(
                            "Resultado TOOL_SCHEDULER: {}\n\n",
                            observation.payload
                        ));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &observation.payload,
                            "SUCCESS",
                        );
                        last_progress_step = runtime.current_step();
                    }
                    Ok(observation) => {
                        current_context.push_str(&format!(
                            "Error TOOL_SCHEDULER: {}\n\n",
                            observation.payload
                        ));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &observation.payload,
                            "ERROR",
                        );
                    }
                    Err(error) => {
                        current_context.push_str(&format!("Error TOOL_SCHEDULER: {}\n\n", error));
                        emit_event(&app_handle, runtime.current_step(), &error, "ERROR");
                    }
                }
            }
            "TOOL_WEB_SEARCH" => {
                let query = if !comando.trim().is_empty() {
                    comando.clone()
                } else {
                    url.clone()
                };
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    &format!("Buscando información web: {}", query),
                    "ACTION",
                );
                match runtime.execute_action(&action_proposal).await {
                    Ok(obs)
                        if obs.status == crate::core::observation::ObservationStatus::Success =>
                    {
                        current_context.push_str(&format!(
                            "Resultado TOOL_WEB_SEARCH (incluye títulos, enlaces y fragmentos):\n{}\n\n",
                            obs.payload
                        ));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "Búsqueda web completada.",
                            "SUCCESS",
                        );
                        last_progress_step = runtime.current_step();
                    }
                    Ok(obs) => {
                        planning_research_failed = true;
                        current_context.push_str(&format!(
                            "No se pudo completar TOOL_WEB_SEARCH: {}\nSi continúas, declara esta limitación y no inventes fuentes.\n\n",
                            obs.payload
                        ));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "La búsqueda web no produjo resultados verificables.",
                            "WARNING",
                        );
                    }
                    Err(error) => {
                        planning_research_failed = true;
                        current_context.push_str(&format!(
                            "Error TOOL_WEB_SEARCH: {}\nSi continúas, declara esta limitación y no inventes fuentes.\n\n",
                            error
                        ));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &format!("Error en búsqueda web: {}", error),
                            "WARNING",
                        );
                    }
                }
            }
            "TOOL_SEARCH" => {
                // FALLO #6 FIX: use `comando` as query first, fall back to `url_a_investigar`.
                let search_query = if !comando.trim().is_empty() {
                    comando.clone()
                } else {
                    url.clone()
                };
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    &format!("Consultando Memoria Permanente para: {}", search_query),
                    "ACTION",
                );
                let search_proposal = crate::core::policy::ActionProposal {
                    tool: "TOOL_SEARCH".to_string(),
                    arguments: serde_json::json!({ "query": search_query }),
                    expected_effect: "Buscar en la memoria local y, si no hay coincidencias, leer un archivo seguro solicitado".to_string(),
                    risk: crate::core::policy::RiskLevel::Safe,
                    world_hash: Some(runtime.current_world_hash()),
                };
                match runtime.execute_action(&search_proposal).await {
                    Ok(observation)
                        if observation.status
                            == crate::core::observation::ObservationStatus::Success =>
                    {
                        current_context.push_str(&format!(
                            "Resultado TOOL_SEARCH:\n{}\n\n",
                            observation.payload
                        ));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "Búsqueda local completada con resultados.",
                            "SUCCESS",
                        );
                    }
                    Ok(observation) => {
                        current_context.push_str(&format!(
                            "Error en TOOL_SEARCH: {}\n\n",
                            observation.payload
                        ));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &observation.payload,
                            "WARNING",
                        );
                    }
                    Err(error) => {
                        current_context.push_str(&format!("Error en TOOL_SEARCH: {}\n\n", error));
                        emit_event(&app_handle, runtime.current_step(), &error, "ERROR");
                    }
                }
            }
            "TOOL_ASK_USER" => {
                ask_user_consecutive += 1;
                if ask_user_consecutive > 1 {
                    // Anti-stalling protection: Do not allow the agent to prompt the user repeatedly
                    let abort_msg = "[SISTEMA INTERNO]: ⚠️ TOOL_ASK_USER BLOQUEADO. Ya consultaste al usuario en el turno inmediato anterior. Prohibido volver a preguntar. Debes implementar o verificar la solución de inmediato con TOOL_PROGRAMMER o TOOL_TERMINAL.";
                    current_context.push_str(&format!("{}\n\n", abort_msg));
                    emit_event(&app_handle, runtime.current_step(), "TOOL_ASK_USER bloqueado por repetición consecutiva. Forzando implementación.", "WARNING");
                    current_role = AgentRole::Executor;
                    forced_next_tool = Some((
                        "TOOL_PROGRAMMER".to_string(),
                        "Implementa el código directamente sin hacer más preguntas al usuario."
                            .to_string(),
                    ));
                    continue;
                }

                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    "Solicitando información al usuario...",
                    "ACTION",
                );
                let mut question = comando.clone();
                if question.trim().is_empty() {
                    question = raw_value
                        .get("respuesta_conversacional")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                }
                if question.trim().is_empty() {
                    question = raw_value.get("pensamiento")
                        .and_then(|v| v.as_str())
                        .unwrap_or("El sistema se atascó o completó una tarea, pero no dejó un mensaje. ¿Cómo deseas proceder?")
                        .to_string();
                }
                let approval_pending = pending_user_approval.is_some();
                let options = if approval_pending {
                    vec!["Autorizar esta acción".to_string(), "Cancelar".to_string()]
                } else {
                    Vec::new()
                };

                let ask_proposal = crate::core::policy::ActionProposal {
                    tool: "TOOL_ASK_USER".to_string(),
                    arguments: serde_json::json!({
                        "question": question.clone(),
                        "options": options,
                        "context": current_context.clone()
                    }),
                    expected_effect: "Solicitar una aclaración real al usuario mediante la interfaz de la aplicación".to_string(),
                    risk: crate::core::policy::RiskLevel::Safe,
                    world_hash: Some(runtime.current_world_hash()),
                };
                match runtime.execute_action(&ask_proposal).await {
                    Ok(observation)
                        if observation.status
                            == crate::core::observation::ObservationStatus::Success =>
                    {
                        let answer = observation.payload;
                        current_context.push_str(&format!(
                            "Pregunta al usuario: {}\nRespuesta del usuario: {}\n\n",
                            question, answer
                        ));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "Respuesta del usuario recibida.",
                            "SUCCESS",
                        );
                        if let Some(pending_action) = pending_user_approval.take() {
                            if is_explicit_approval(&answer) {
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    &format!(
                                        "Ejecutando la acción aprobada: {}",
                                        pending_action.tool
                                    ),
                                    "ACTION",
                                );
                                match runtime
                                    .execute_action_with_user_approval(&pending_action, true)
                                    .await
                                {
                                    Ok(observation)
                                        if observation.status
                                            == crate::core::observation::ObservationStatus::Success =>
                                    {
                                        current_context.push_str(&format!(
                                            "Acción aprobada ejecutada ({}): {}\n\n",
                                            pending_action.tool, observation.payload
                                        ));
                                        emit_event(
                                            &app_handle,
                                            runtime.current_step(),
                                            "La acción aprobada terminó correctamente.",
                                            "SUCCESS",
                                        );
                                        last_progress_step = runtime.current_step();
                                    }
                                    Ok(observation) => {
                                        current_context.push_str(&format!(
                                            "La acción aprobada no pudo completarse ({}): {}\n\n",
                                            pending_action.tool, observation.payload
                                        ));
                                        emit_event(
                                            &app_handle,
                                            runtime.current_step(),
                                            &observation.payload,
                                            "ERROR",
                                        );
                                    }
                                    Err(error) => {
                                        current_context.push_str(&format!(
                                            "La acción aprobada fue bloqueada o falló ({}): {}\n\n",
                                            pending_action.tool, error
                                        ));
                                        emit_event(&app_handle, runtime.current_step(), &error, "ERROR");
                                    }
                                }
                            } else {
                                current_context.push_str(&format!(
                                    "Acción '{}' cancelada; el usuario no dio autorización explícita.\n\n",
                                    pending_action.tool
                                ));
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    "Acción cancelada por falta de autorización explícita.",
                                    "INFO",
                                );
                            }
                        } else {
                            // User clarified the task — immediately transition to Executor with TOOL_PROGRAMMER.
                            current_role = AgentRole::Executor;
                            forced_next_tool = Some(("TOOL_PROGRAMMER".to_string(), format!("El usuario ya aclaró los requerimientos: '{}'. Implementa el código inmediatamente.", answer)));
                        }
                    }
                    Ok(observation) => {
                        current_context.push_str(&format!(
                            "Error al consultar al usuario: {}\n\n",
                            observation.payload
                        ));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &format!("Error ASK_USER: {}", observation.payload),
                            "WARNING",
                        );
                    }
                    Err(error) => {
                        current_context
                            .push_str(&format!("Error al consultar al usuario: {}\n\n", error));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &format!("Error ASK_USER: {}", error),
                            "ERROR",
                        );
                    }
                }
            }
            "TOOL_FINISH" => {
                if last_local_ui_url.is_none() {
                    if let Some((next_tool, reason)) = local_http_server_recovery_action(
                        &original_prompt_parsed,
                        &workspace_path,
                        last_http_server_task_id.as_deref(),
                    ) {
                        current_role = if next_tool == "TOOL_PROGRAMMER" {
                            AgentRole::Executor
                        } else {
                            AgentRole::Critic
                        };
                        current_context.push_str(&format!(
                            "[SERVIDOR HTTP REQUERIDO] No se puede cerrar la misión porque falta confirmar la URL HTTP localhost. Siguiente acción: {}.\n\n",
                            reason
                        ));
                        forced_next_tool = Some((next_tool.clone(), reason));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "[COMPLETION GATE] Se bloqueó el cierre porque falta la URL HTTP local exigida.",
                            "WARNING",
                        );
                        continue;
                    }
                }
                visual_validation_required |=
                    mission_requires_visual_verification(&original_prompt_parsed, &workspace_path)
                        || scoped_visual_review;
                let mut scoped_visual_rejection = false;
                if visual_validation_required {
                    runtime.observe_world()?;
                    let current_hash = runtime.current_world_hash();
                    scoped_visual_rejection = scoped_visual_review
                        && visual_validation_error_world_hash == Some(current_hash)
                        && visual_validation_error.as_deref().is_some_and(|error| {
                            error.contains("VISUAL_QA_REJECTED")
                                && visual_capture_from_error(&workspace_path, error).is_some()
                        });
                    if visual_validation_world_hash != Some(current_hash)
                        && !scoped_visual_rejection
                    {
                        let latest_error =
                            if visual_validation_error_world_hash == Some(current_hash) {
                                visual_validation_error.clone()
                            } else {
                                visual_validation_error = None;
                                visual_validation_error_world_hash = None;
                                None
                            };
                        if let Some(error) = latest_error.as_deref() {
                            if vision_result_needs_manual_review(error) {
                                if let Some(capture_path) =
                                    visual_capture_from_error(&workspace_path, error)
                                {
                                    let review_question = format!(
                                        "La IA de visión no pudo confirmar la captura con seguridad. La imagen quedó guardada aquí:\n{}\n\nAbre la captura y comprueba que aparece la aplicación local, que no está en blanco y que no muestra errores visuales evidentes. ¿Apruebas esta revisión visual manual?",
                                        capture_path.display()
                                    );
                                    match crate::core::ask_user::ask_user_async(
                                        &app_handle,
                                        review_question,
                                        vec![
                                            "Aprobar captura".to_string(),
                                            "Solicitar correcciones".to_string(),
                                        ],
                                        format!(
                                            "Objetivo: {}. Resultado de la IA visual: {}",
                                            original_prompt_parsed, error
                                        ),
                                    )
                                    .await
                                    {
                                        Ok(reply) if reply == "Aprobar captura" => {
                                            runtime.evidence_graph.record_with_hash(
                                                crate::core::evidence::EvidenceKind::UserConfirmation,
                                                "user",
                                                "La captura local de la interfaz fue revisada manualmente",
                                                &reply,
                                                1.0,
                                                runtime.current_step(),
                                                Some(current_hash),
                                            )?;
                                            visual_validation_world_hash = Some(current_hash);
                                            visual_validation_error = None;
                                            visual_validation_error_world_hash = None;
                                            mandatory_tools_executed
                                                .insert("TOOL_VISION_EVALUATOR".to_string());
                                            current_context.push_str(&format!(
                                                "[REVISIÓN VISUAL MANUAL APROBADA] La IA visual no pudo confirmar la captura; el usuario revisó y aprobó {}.\n\n",
                                                capture_path.display()
                                            ));
                                            emit_event(
                                                &app_handle,
                                                runtime.current_step(),
                                                "[GATEKEEPER VISUAL] El usuario revisó y aprobó la captura que la IA no pudo clasificar.",
                                                "SUCCESS",
                                            );
                                        }
                                        Ok(_) => {
                                            let feedback = crate::core::ask_user::ask_user_async(
                                                &app_handle,
                                                "¿Qué elemento visible debe corregirse? Describe el problema concreto para que Aura repare la interfaz y vuelva a probarla.".to_string(),
                                                vec!["Escribir observaciones".to_string()],
                                                format!("Captura a revisar: {}", capture_path.display()),
                                            )
                                            .await
                                            .unwrap_or_else(|_| "El usuario solicitó correcciones visuales; inspecciona la captura antes de editar.".to_string());
                                            visual_validation_error = Some(format!(
                                                "VISUAL_QA_REJECTED: el usuario pidió correcciones: {feedback}"
                                            ));
                                            visual_validation_error_world_hash = Some(current_hash);
                                            current_role = AgentRole::Executor;
                                            forced_next_tool = Some((
                                                "TOOL_THINK".to_string(),
                                                format!(
                                                    "El usuario revisó la captura {} y solicita corregir: {feedback}. Repara solo esos defectos visibles, ejecuta las pruebas y captura de nuevo la interfaz local.",
                                                    capture_path.display()
                                                ),
                                            ));
                                            emit_event(
                                                &app_handle,
                                                runtime.current_step(),
                                                "[GATEKEEPER VISUAL] El usuario solicitó una corrección visual; la fase permanece abierta.",
                                                "WARNING",
                                            );
                                            continue;
                                        }
                                        Err(question_error) => {
                                            let message = format!(
                                                "VISUAL_REVIEW_REQUIRED: la IA no pudo aprobar la captura y no se pudo abrir la revisión manual ({question_error}). La imagen quedó guardada en {}.",
                                                capture_path.display()
                                            );
                                            emit_event(
                                                &app_handle,
                                                runtime.current_step(),
                                                &message,
                                                "ERROR",
                                            );
                                            let _ = save_resumable_validation_block(
                                                &mut journal,
                                                &workspace_path,
                                                runtime.current_step(),
                                                "Critic",
                                                &current_context,
                                                &message,
                                            );
                                            let final_res = FinalResponse {
                                                status: "ERROR".to_string(),
                                                respuesta_conversacional: message,
                                            };
                                            return Ok(serde_json::to_string(&final_res).unwrap());
                                        }
                                    }
                                    if visual_validation_world_hash == Some(current_hash) {
                                        // User approval supplies the visual acceptance that the local model could not.
                                    } else {
                                        continue;
                                    }
                                }
                            }
                        }
                        if visual_validation_world_hash != Some(current_hash) {
                            let unavailable = latest_error
                                .as_deref()
                                .is_some_and(|error| error.contains("VISUAL_QA_UNAVAILABLE"));
                            let manual_review_available = latest_error
                                .as_deref()
                                .is_some_and(vision_result_needs_manual_review)
                                && latest_error
                                    .as_deref()
                                    .and_then(|error| {
                                        visual_capture_from_error(&workspace_path, error)
                                    })
                                    .is_some();
                            if (unavailable && !manual_review_available)
                                || visual_validation_attempts >= 3
                                || visual_finish_recovery_attempts >= 3
                            {
                                let reason = latest_error.unwrap_or_else(|| {
                                "No existe una captura visual aprobada para la versión actual del workspace.".to_string()
                            });
                                let message = format!(
                                "VISUAL_VALIDATION_BLOCKED: La interfaz no puede declararse completa sin una captura real aprobada. {}. El proyecto y sus archivos se conservaron; resuelve la dependencia indicada y continúa para repetir la validación.",
                                reason
                            );
                                emit_event(&app_handle, runtime.current_step(), &message, "ERROR");
                                if let Err(error) = save_resumable_validation_block(
                                    &mut journal,
                                    &workspace_path,
                                    runtime.current_step(),
                                    "Critic",
                                    &current_context,
                                    &message,
                                ) {
                                    emit_event(
                                        &app_handle,
                                        runtime.current_step(),
                                        &format!("[CHECKPOINT FAILED] {error}"),
                                        "FATAL",
                                    );
                                }
                                let final_res = FinalResponse {
                                    status: "ERROR".to_string(),
                                    respuesta_conversacional: message,
                                };
                                return Ok(serde_json::to_string(&final_res).unwrap());
                            }

                            visual_finish_recovery_attempts =
                                visual_finish_recovery_attempts.saturating_add(1);
                            if latest_error
                                .as_deref()
                                .is_some_and(|error| error.contains("VISUAL_QA_REJECTED"))
                            {
                                current_role = AgentRole::Executor;
                                forced_next_tool = Some((
                                "TOOL_THINK".to_string(),
                                format!(
                                    "No cierres: la revisión visual rechazó la interfaz. Repara los defectos descritos y conserva el resto de funciones. Luego ejecuta las pruebas funcionales y solicita una captura nueva de la aplicación local. Diagnóstico: {}",
                                    latest_error.as_deref().unwrap_or_default()
                                ),
                            ));
                                emit_event(
                                &app_handle,
                                runtime.current_step(),
                                "[GATEKEEPER VISUAL] La captura detectó defectos. La fase no avanzará hasta corregirlos y volver a capturar.",
                                "WARNING",
                            );
                            } else if latest_error.is_some() {
                                current_role = AgentRole::Executor;
                                forced_next_tool = Some((
                                "TOOL_THINK".to_string(),
                                format!(
                                    "No cierres: la captura de la aplicación local no pudo aprobarse. Resuelve la URL o el proceso usando el error real, vuelve a iniciar/consultar el servidor si corresponde, y después captura la página del workspace. No uses una captura del escritorio como sustituto. Error: {}",
                                    latest_error.as_deref().unwrap_or_default()
                                ),
                            ));
                                emit_event(
                                &app_handle,
                                runtime.current_step(),
                                "[GATEKEEPER VISUAL] La captura no pudo verificarse; se resolverá el origen antes del cierre.",
                                "WARNING",
                            );
                            } else {
                                current_role = AgentRole::Critic;
                                forced_next_tool = Some((
                                "TOOL_VISION_EVALUATOR".to_string(),
                                format!(
                                    "Valida ahora la versión actual de la interfaz local contra este objetivo y la fase: {}. Debes evaluar la página real, conservar la captura en .aura/evidence/visual y devolver un veredicto visual explícito.",
                                    original_prompt_parsed
                                ),
                            ));
                                emit_event(
                                &app_handle,
                                runtime.current_step(),
                                "[GATEKEEPER VISUAL] Captura pendiente para la versión actual; no se cerrará la fase todavía.",
                                "INFO",
                            );
                            }
                            continue;
                        }
                    }
                }

                if browser_interaction_validation_required {
                    runtime.observe_world()?;
                    let current_hash = runtime.current_world_hash();
                    if browser_interaction_validation_world_hash != Some(current_hash) {
                        if browser_interaction_validation_world_hash.is_some() {
                            browser_interaction_validation_world_hash = None;
                        }
                        let (next_tool, guidance) = web_validation_recovery_action(
                            &original_prompt_parsed,
                            &workspace_path,
                            last_local_ui_url.as_deref(),
                            last_http_server_task_id.as_deref(),
                        )
                        .unwrap_or_else(|| (
                            "TOOL_TESTER".into(),
                            "Crea un plan BROWSER_TEST con pasos que cubran los flujos interactivos pedidos.".into(),
                        ));
                        current_role = AgentRole::Critic;
                        forced_next_tool = Some((next_tool.clone(), guidance.clone()));
                        let message = format!(
                            "[COMPLETION GATE] La misión exige pruebas reales de interacción en navegador para el estado actual. Falta evidencia BROWSER_TEST. Acción: {next_tool} — {guidance}"
                        );
                        current_context.push_str(&format!("{message}\n\n"));
                        emit_event(&app_handle, runtime.current_step(), &message, "WARNING");
                        continue;
                    }
                }

                if let Some((depth, guidance)) = console_validation_profile(&original_prompt_parsed)
                {
                    let current_hash = runtime.current_world_hash();
                    if !has_current_console_runtime_evidence(
                        &runtime.evidence_graph,
                        current_hash,
                        &workspace_path,
                    ) {
                        if console_runtime_recovery_attempts >= 2 {
                            let message = format!(
                                "CONSOLE_RUNTIME_VALIDATION_BLOCKED: No hay evidencia de que el programa de consola se haya ejecutado correctamente en el estado actual. Perfil solicitado: {depth}. {guidance} El trabajo se conservó y la misión quedó disponible para reanudarse."
                            );
                            emit_event(&app_handle, runtime.current_step(), &message, "ERROR");
                            if let Err(error) = save_resumable_validation_block(
                                &mut journal,
                                &workspace_path,
                                runtime.current_step(),
                                "Critic",
                                &current_context,
                                &message,
                            ) {
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    &format!("[CHECKPOINT FAILED] {error}"),
                                    "FATAL",
                                );
                            }
                            let final_res = FinalResponse {
                                status: "ERROR".to_string(),
                                respuesta_conversacional: message,
                            };
                            return Ok(serde_json::to_string(&final_res).unwrap());
                        }

                        console_runtime_recovery_attempts =
                            console_runtime_recovery_attempts.saturating_add(1);
                        let message = format!(
                            "[GATEKEEPER CONSOLA] La fase no se cierra sin una ejecución real y exitosa del programa. Perfil: {depth}. {guidance} Inspecciona los archivos y scripts existentes, escoge el comando real de ejecución del proyecto, usa entradas representativas solicitadas y comprueba stdout/stderr y el código de salida. Las pruebas unitarias por sí solas no sustituyen esta ejecución."
                        );
                        current_role = AgentRole::Critic;
                        forced_next_tool = Some(("TOOL_TERMINAL".to_string(), message.clone()));
                        current_context.push_str(&format!("{message}\n\n"));
                        emit_event(&app_handle, runtime.current_step(), &message, "WARNING");
                        continue;
                    }
                }

                // ── Mandatory Tool Checklist enforcement (Bug 1 fix) ─────────────────
                // If the user's prompt required specific tools (TOOL_TESTER, TOOL_VISION_EVALUATOR)
                // and they haven't been executed yet, block TOOL_FINISH and instruct the agent.
                let missing_mandatory: Vec<&String> = mandatory_tools_required
                    .iter()
                    .filter(|t| !mandatory_tools_executed.contains(*t))
                    .collect();
                if !missing_mandatory.is_empty() {
                    let vision_is_missing = missing_mandatory
                        .iter()
                        .any(|tool| tool.as_str() == "TOOL_VISION_EVALUATOR");
                    let terminal_vision_failure = visual_validation_error
                        .as_deref()
                        .is_some_and(|error| error.contains("VISUAL_QA_UNAVAILABLE"));
                    if vision_is_missing
                        && (terminal_vision_failure || visual_validation_attempts >= 3)
                    {
                        let reason = visual_validation_error.clone().unwrap_or_else(|| {
                            "La evaluación visual no se completó tras tres intentos.".to_string()
                        });
                        let message = format!(
                            "VISUAL_VALIDATION_BLOCKED: La herramienta visual fue exigida pero no produjo una evaluación válida. {reason}. El trabajo se conservó y la misión quedó disponible para reanudarse."
                        );
                        emit_event(&app_handle, runtime.current_step(), &message, "ERROR");
                        if let Err(error) = save_resumable_validation_block(
                            &mut journal,
                            &workspace_path,
                            runtime.current_step(),
                            "Critic",
                            &current_context,
                            &message,
                        ) {
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                &format!("[CHECKPOINT FAILED] {error}"),
                                "FATAL",
                            );
                        }
                        let final_res = FinalResponse {
                            status: "ERROR".to_string(),
                            respuesta_conversacional: message,
                        };
                        return Ok(serde_json::to_string(&final_res).unwrap());
                    }
                    let missing_list: Vec<&str> =
                        missing_mandatory.iter().map(|s| s.as_str()).collect();
                    let block_msg = format!(
                        "[SISTEMA]: TOOL_FINISH BLOQUEADO. El mandato del usuario exige que ejecutes \
                        las siguientes herramientas ANTES de finalizar: {:?}. \
                        Debes ejecutarlas ahora. No puedes usar TOOL_FINISH hasta que todas estén completas.",
                        missing_list
                    );
                    current_context.push_str(&format!("{}\n\n", block_msg));
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!(
                            "[FINISH BLOQUEADO] Faltan herramientas obligatorias: {:?}",
                            missing_list
                        ),
                        "WARNING",
                    );
                    continue;
                }

                // =======================================================
                // PESP v2 — Intercept TOOL_FINISH for Phase Advancement
                // =======================================================
                if approved_consultation_task && journal.fase_actual == 1 {
                    let audit = crate::llm::phase_planner::audit_consultation_firebase(
                        std::path::Path::new(&workspace_path),
                    );
                    if !audit.issues.is_empty() {
                        let diagnostic = audit.issues.join("\n");
                        verifier_diagnostic = diagnostic.clone();
                        current_role = AgentRole::Executor;
                        forced_next_tool = Some((
                            "TOOL_PROGRAMMER".to_string(),
                            format!(
                                "Fase 2 BLOQUEADA por auditoría Firebase. Repara solo estos defectos: {}. Archivos: {:?}.",
                                diagnostic, audit.repair_files
                            ),
                        ));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "[GATEKEEPER FIREBASE] Fase 2 bloqueada: la integración o las pruebas de emulador siguen incompletas.",
                            "ERROR",
                        );
                        continue;
                    }
                    if !successful_command_for_world(
                        &comandos_exitosos_historico,
                        "npm run test:firebase",
                        runtime.current_world_hash(),
                    ) {
                        current_role = AgentRole::Executor;
                        forced_next_tool = Some((
                            "TOOL_TERMINAL".to_string(),
                            "Fase 2 BLOQUEADA: ejecuta `npm run test:firebase` y revisa su salida real antes de solicitar el cierre. `npm test` no valida Firebase.".to_string(),
                        ));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "[GATEKEEPER FIREBASE] Fase 2 bloqueada: no hay evidencia vigente de la prueba de emuladores.",
                            "ERROR",
                        );
                        continue;
                    }
                }
                if !scoped_visual_review
                    && !journal.fases.is_empty()
                    && journal.fase_actual < journal.fases.len() - 1
                {
                    let phase_num = journal.fases[journal.fase_actual].numero;
                    let phase_desc = journal.fases[journal.fase_actual].descripcion.clone();

                    // ── GATEKEEPER ESTRICTO DE FASE (Con resolución inteligente de alias) ──
                    let current_phase = &journal.fases[journal.fase_actual];
                    let missing_files: Vec<String> = current_phase
                        .archivos
                        .iter()
                        .filter(|arch| !is_phase_file_satisfied(&workspace_path, arch))
                        .cloned()
                        .collect();

                    if !missing_files.is_empty() {
                        let block_msg = format!("[GATEKEEPER] ❌ Fase {} BLOQUEADA. Faltan archivos requeridos en disco: {:?}", phase_num, missing_files);
                        emit_event(&app_handle, runtime.current_step(), &block_msg, "FATAL");
                        current_context.push_str(&format!("{}\n[ACCIÓN OBLIGATORIA]: No puedes avanzar de fase sin crear estos archivos. Usa TOOL_PROGRAMMER para crearlos.\n\n", block_msg));
                        current_role = AgentRole::Executor;
                        continue;
                    }

                    if let Err(compile_err) = validate_workspace(&workspace_path).await {
                        let block_msg = format!("[GATEKEEPER] ❌ Fase {} BLOQUEADA. Errores de sintaxis/compilación detectados:\n{}", phase_num, compile_err);
                        emit_event(&app_handle, runtime.current_step(), &format!("[GATEKEEPER] ❌ Fase {} BLOQUEADA por errores de sintaxis en el código.", phase_num), "FATAL");
                        current_context.push_str(&format!("{}\n[ACCIÓN OBLIGATORIA]: El código generado está incompleto o tiene errores de sintaxis. Usa TOOL_PROGRAMMER para corregir o completar el archivo.\n\n", block_msg));
                        current_role = AgentRole::Executor;
                        continue;
                    }

                    emit_event(&app_handle, runtime.current_step(), &format!("⏸ [PAUSA INTERACTIVA] Fase {} completada. Esperando aprobación del usuario...", phase_num), "WARNING");

                    let question = format!("He completado la Fase {}: '{}'. ¿Deseas que avance a la siguiente fase, o quieres revisar/cambiar algo?", phase_num, phase_desc);
                    let options = vec![
                        "Aprobar y Continuar".to_string(),
                        "Modificar Instrucciones".to_string(),
                        "Detener Agente".to_string(),
                    ];

                    // Pause execution and ask user
                    match crate::core::ask_user::ask_user_async(
                        &app_handle,
                        question,
                        options,
                        current_context.clone(),
                    )
                    .await
                    {
                        Ok(reply) => {
                            if reply == "Detener Agente" {
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    "El usuario detuvo la ejecución.",
                                    "ERROR",
                                );
                                let final_res = FinalResponse {
                                    status: "CANCELLED".to_string(),
                                    respuesta_conversacional: "Detenido por el usuario".to_string(),
                                };
                                return Ok(serde_json::to_string(&final_res).unwrap());
                            } else if reply != "Aprobar y Continuar" {
                                let user_feedback = format!(
                                    "[FEEDBACK DEL USUARIO EN PAUSA INTERACTIVA]: {}",
                                    reply
                                );
                                current_context.push_str(&format!("{}\n\n", user_feedback));
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    "Feedback del usuario recibido. Ajustando plan.",
                                    "ACTION",
                                );
                                continue;
                            }
                        }
                        Err(e) => {
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                &format!("Pausa interactiva interrumpida: {}", e),
                                "ERROR",
                            );
                            let final_res = FinalResponse {
                                status: "CANCELLED".to_string(),
                                respuesta_conversacional: "Interrumpido".to_string(),
                            };
                            return Ok(serde_json::to_string(&final_res).unwrap());
                        }
                    }
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!(
                            "✅ [FASE {} COMPLETADA] Avanzando a la siguiente...",
                            journal.fases[journal.fase_actual].numero
                        ),
                        "SUCCESS",
                    );

                    // Mark current phase as completed
                    journal.fases[journal.fase_actual].estado = "COMPLETADA".to_string();
                    // Advance to next phase
                    journal.fase_actual += 1;
                    journal.fases[journal.fase_actual].estado = "EN_PROGRESO".to_string();
                    if let Err(e) =
                        crate::core::session_journal::save_journal(&workspace_path, &journal)
                    {
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &format!("[CHECKPOINT FAILED] {}", e),
                            "FATAL",
                        );
                        let final_res = FinalResponse {
                            status: "ERROR".to_string(),
                            respuesta_conversacional: format!("[PERSISTENCE_FAILURE] Error crítico al avanzar de fase en el diario: {}. Misión abortada.", e),
                        };
                        return Ok(serde_json::to_string(&final_res).unwrap());
                    }

                    let new_phase_msg = format!(
                        "[SISTEMA PESP] Fase anterior completada. Iniciando Fase {}/{}: {}\nUsa TOOL_PROGRAMMER o TOOL_THINK para comenzar el trabajo de esta nueva fase.",
                        journal.fase_actual + 1, journal.fases.len(), journal.fases[journal.fase_actual].descripcion
                    );
                    current_context.push_str(&format!("{}\n\n", new_phase_msg));
                    current_role = AgentRole::Planner;
                    continue; // Advance the next phase before evaluating mission completion.
                } else if !scoped_visual_review
                    && !journal.fases.is_empty()
                    && journal.fase_actual == journal.fases.len() - 1
                {
                    let current_phase = &journal.fases[journal.fase_actual];
                    let phase_num = current_phase.numero;

                    let missing_files: Vec<String> = current_phase
                        .archivos
                        .iter()
                        .filter(|arch| !is_phase_file_satisfied(&workspace_path, arch))
                        .cloned()
                        .collect();

                    if !missing_files.is_empty() {
                        let block_msg = format!("[GATEKEEPER FINAL] ❌ Misión NO puede cerrarse. Faltan archivos requeridos de la Fase {}: {:?}", phase_num, missing_files);
                        emit_event(&app_handle, runtime.current_step(), &block_msg, "FATAL");
                        current_context.push_str(&format!("{}\n[ACCIÓN OBLIGATORIA]: Crea los archivos pendientes usando TOOL_PROGRAMMER antes de concluir.\n\n", block_msg));
                        current_role = AgentRole::Executor;
                        continue;
                    }

                    if let Err(compile_err) = validate_workspace(&workspace_path).await {
                        let block_msg = format!("[GATEKEEPER FINAL] ❌ Misión NO puede cerrarse por error sintáctico:\n{}", compile_err);
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "[GATEKEEPER FINAL] Error sintáctico en disco. Exigiendo corrección.",
                            "FATAL",
                        );
                        current_context.push_str(&format!("{}\n[ACCIÓN OBLIGATORIA]: Corrige los errores de sintaxis antes de finalizar.\n\n", block_msg));
                        current_role = AgentRole::Executor;
                        continue;
                    }
                }

                // Manual criteria require an actual reply from the user, bound to this snapshot.
                runtime.observe_world()?;
                let review_hash = runtime.current_world_hash();
                let mut technical_contract = runtime.contract.clone();
                technical_contract.acceptance_criteria.retain(|ac| {
                    !matches!(
                        ac.verification,
                        crate::core::mission_contract::VerificationMethod::ManualReview
                    )
                });
                let technical_ready = (technical_contract.acceptance_criteria.is_empty()
                    && technical_contract.required_evidence.is_empty())
                    || matches!(
                        crate::core::completion_gate::CompletionGate::evaluate(
                            &technical_contract,
                            &runtime.cognitive_state,
                            &runtime.evidence_graph,
                            review_hash,
                            std::path::Path::new(&workspace_path)
                        ),
                        crate::core::completion_gate::CompletionDecision::Complete
                    );
                let pending_reviews: Vec<_> = if mission_type == MissionType::Planning {
                    Vec::new()
                } else {
                    runtime
                        .contract
                        .acceptance_criteria
                        .iter()
                        .filter(|ac| {
                            technical_ready
                                && ac.required
                                && matches!(
                                    ac.verification,
                                    crate::core::mission_contract::VerificationMethod::ManualReview
                                )
                        })
                        .filter(|ac| {
                            !runtime.evidence_graph.has_valid_manual_evidence_for_state(
                                &format!("{} verified manually", ac.id),
                                1.0,
                                review_hash,
                            )
                        })
                        .cloned()
                        .collect()
                };
                for criterion in pending_reviews {
                    let question = if mission_type == MissionType::Planning {
                        format!(
                            "Plan inicial propuesto:\n\n{}\n\n¿Esta hoja de ruta representa lo que quieres construir?",
                            respuesta_conv
                        )
                    } else {
                        format!(
                            "Revisa los entregables en {}. ¿Se cumple este criterio? {}",
                            workspace_path, criterion.description
                        )
                    };
                    let answer = crate::core::ask_user::ask_user_async(
                        &app_handle,
                        question,
                        vec!["Aprobar criterio".into(), "Requiere correcciones".into()],
                        format!(
                            "Misión: {}. Criterio: {}",
                            original_prompt_parsed, criterion.id
                        ),
                    )
                    .await?;
                    if answer == "Aprobar criterio" {
                        let review_step = runtime.current_step();
                        runtime.evidence_graph.record_with_hash(
                            crate::core::evidence::EvidenceKind::UserConfirmation,
                            "user",
                            &format!("{} verified manually", criterion.id),
                            &answer,
                            1.0,
                            review_step,
                            Some(review_hash),
                        )?;
                    } else {
                        current_context.push_str(&format!(
                            "\n[REVISIÓN DEL USUARIO] {}: {}\n",
                            criterion.id, answer
                        ));
                    }
                }
                runtime.observe_world()?;

                // ↀ CompletionGate delegado a MissionRuntime (fuente única de verdad) ↀ
                let completion_decision = if mission_type == MissionType::Planning {
                    if planning_research_failed
                        && !respuesta_conv
                            .to_lowercase()
                            .contains("no se pudo verificar")
                    {
                        respuesta_conv.push_str(
                            "\n\nInvestigación web: no se pudo verificar información actual de Firebase porque la búsqueda no devolvió resultados. Este plan es preliminar y no cita fuentes externas.",
                        );
                    }
                    if !planning_response_complete(
                        &respuesta_conv,
                        planning_research_failed,
                        &original_prompt_parsed,
                    ) {
                        respuesta_conv = build_fallback_initial_plan(
                            &original_prompt_parsed,
                            planning_research_failed,
                        );
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "[PLAN INICIAL] El borrador del modelo no pasó la revisión; se generó una hoja de ruta estructurada automáticamente.",
                            "INFO",
                        );
                    }
                    if planning_response_complete(
                        &respuesta_conv,
                        planning_research_failed,
                        &original_prompt_parsed,
                    ) {
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "[COMPLETION GATE] Plan inicial validado: incluye fases, entregables, supuestos y decisiones pendientes.",
                            "SUCCESS",
                        );
                        crate::core::completion_gate::CompletionDecision::Complete
                    } else {
                        let research_requirement = if planning_research_failed {
                            " Como la búsqueda falló, declara explícitamente 'No se pudo verificar información actual de Firebase' y no presentes datos externos como confirmados."
                        } else {
                            ""
                        };
                        let mut gaps = Vec::new();
                        if !initial_plan_response_complete(&respuesta_conv) {
                            gaps.push("El plan debe incluir al menos dos fases, entregables, supuestos y decisiones pendientes.".to_string());
                        }
                        gaps.extend(
                            missing_requested_plan_coverage(
                                &original_prompt_parsed,
                                &respuesta_conv,
                            )
                            .into_iter()
                            .map(|item| format!("Falta cubrir el requisito explícito: {item}.")),
                        );
                        let lower_request = original_prompt_parsed.to_lowercase();
                        let local_first = lower_request.contains("local")
                            && (lower_request.contains("luego")
                                || lower_request.contains("después")
                                || lower_request.contains("despues"))
                            && (lower_request.contains("firebase")
                                || lower_request.contains("despleg"));
                        if local_first && !plan_tests_locally_before_deploying(&respuesta_conv) {
                            gaps.push("Respeta el orden pedido: pruebas locales en la primera fase y despliegue Firebase en una fase posterior.".to_string());
                        }
                        if !research_requirement.is_empty() {
                            gaps.push(research_requirement.trim().to_string());
                        }
                        crate::core::completion_gate::CompletionDecision::Incomplete(gaps)
                    }
                } else {
                    runtime.can_complete()
                };
                match completion_decision {
                    crate::core::completion_gate::CompletionDecision::Incomplete(
                        missing_reasons,
                    ) => {
                        let block_msg = format!("[COMPLETION GATE] ⚠︠ Finalización rechazada. Requisitos pendientes:\n{}", missing_reasons.join("\n"));
                        emit_event(&app_handle, runtime.current_step(), &block_msg, "WARNING");
                        if mission_type == MissionType::Planning {
                            current_context.push_str(&format!(
                                "{}\n[CORRECCIÓN OBLIGATORIA DEL PLAN]: No repitas una reflexión ni una frase genérica de cierre. Devuelve ahora TOOL_FINISH con las fases, los módulos solicitados que faltan, entregables verificables, supuestos y decisiones pendientes. En cada fase escribe objetivo, entregables y condición de finalización. Si el usuario pidió probar localmente antes de desplegar, pon los emuladores/pruebas en la fase inicial y el despliegue en una posterior. No solicites una decisión fiscal para iniciar una fase local; confirma la jurisdicción solo antes de implementar esa parte fiscal. Si la búsqueda web falló, declara 'No se pudo verificar información actual de Firebase' y no inventes fuentes.\n\n",
                                block_msg
                            ));
                            forced_next_tool = Some((
                                "TOOL_FINISH".to_string(),
                                "Corrige el plan ahora: cubre todos los módulos solicitados y el orden local primero, despliegue después; cada fase debe tener objetivo, entregables verificables y condición de salida. No preguntes el país para empezar la fase local. Si la búsqueda falló, aclara que no se pudo verificar Firebase y no inventes fuentes.".to_string(),
                            ));
                        } else {
                            current_context.push_str(&format!("{}\n[ACCIÓN OBLIGATORIA]: Resuelve estos puntos antes de llamar a TOOL_FINISH.\n\n", block_msg));
                        }
                        continue;
                    }
                    crate::core::completion_gate::CompletionDecision::Blocked(block_reasons) => {
                        let block_msg = format!(
                            "[COMPLETION GATE] 🛑 Misión bloqueada por restricciones:\n{}",
                            block_reasons.join("\n")
                        );
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "[COMPLETION GATE] Bloqueado por restricciones.",
                            "FATAL",
                        );
                        current_context.push_str(&format!("{}\n\n", block_msg));
                        continue;
                    }
                    crate::core::completion_gate::CompletionDecision::Complete => {
                        emit_event(&app_handle, runtime.current_step(), "🎯 [COMPLETION GATE] Verificación superada: Todos los criterios cumplidos.", "SUCCESS");
                    }
                }

                if scoped_visual_review {
                    let finding = if scoped_visual_rejection {
                        visual_validation_error.clone().unwrap_or_else(|| {
                            "La evaluación visual detectó elementos que requieren revisión."
                                .to_string()
                        })
                    } else {
                        "La página local se cargó y la captura actual recibió aprobación visual."
                            .to_string()
                    };
                    journal.status = "EN_PROGRESO".to_string();
                    journal.interrupted = false;
                    journal.ultimo_paso = runtime.current_step();
                    journal.ultimo_estado =
                        "Revisión visual aislada completada; la fase original sigue pendiente."
                            .to_string();
                    journal.fsm_role = Some("Planner".to_string());
                    journal.fsm_step = 0;
                    journal.fsm_context = None;
                    if let Err(error) =
                        crate::core::session_journal::save_journal(&workspace_path, &journal)
                    {
                        let final_res = FinalResponse {
                            status: "ERROR".to_string(),
                            respuesta_conversacional: format!(
                                "La revisión terminó, pero no se pudo conservar el estado de la fase original: {error}"
                            ),
                        };
                        return Ok(serde_json::to_string(&final_res).unwrap());
                    }

                    let evidence_text = visual_validation_result.as_deref().unwrap_or(&finding);
                    let evidence_path = visual_capture_from_error(&workspace_path, evidence_text)
                        .map(|path| format!("\nCaptura: {}", path.display()))
                        .unwrap_or_default();
                    let response = if respuesta_conv.trim().is_empty() {
                        format!("{finding}{evidence_path}")
                    } else {
                        format!("{}\n\n{}{}", respuesta_conv.trim(), finding, evidence_path)
                    };
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "[REVISION VISUAL] Informe entregado; la fase PESP original permanece en el estado guardado.",
                        "SUCCESS",
                    );
                    let final_res = FinalResponse {
                        status: "FINISH".to_string(),
                        respuesta_conversacional: response,
                    };
                    return Ok(serde_json::to_string(&final_res).unwrap());
                }

                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    "Bucle completado exitosamente.",
                    "FINISH",
                );
                // ── Journal: mark completed ──
                if let Err(e) = crate::core::session_journal::close_journal(
                    &mut journal,
                    "COMPLETADO",
                    &workspace_path,
                ) {
                    emit_event(&app_handle, runtime.current_step(), &e, "FATAL");
                }

                // ── Fase 1: Clear interrupt flag ──
                if let Err(e) = crate::core::mission_persist::clear_interrupt(&workspace_path) {
                    emit_event(&app_handle, runtime.current_step(), &e, "FATAL");
                }

                // ── Fase 3: Guardar Memoria Episódica (Multi-sesión) ──
                crate::core::episodic_memory::save_episode(
                    &workspace_path,
                    &journal.objetivo,
                    "COMPLETADO",
                    &journal.herramientas_usadas,
                    &journal.archivos_tocados,
                );

                // ── Arquitectura Cognitiva v4: Registrar Experiencia en Memoria ──
                let profile = crate::core::project_profile::ProjectProfile::detect(&workspace_path);
                let lang_str = format!("{:?}", profile.primary);
                let exp_rec = crate::core::experience::ExperienceStore::create_record(
                    &workspace_path,
                    &journal.objetivo,
                    &lang_str,
                    journal.herramientas_usadas.clone(),
                    runtime.current_step(),
                    crate::core::experience::ExperienceOutcome::Success,
                    vec![
                        "Misión completada con todos los criterios y pruebas aprobados."
                            .to_string(),
                    ],
                );
                let _ = crate::core::experience::ExperienceStore::record_experience(&exp_rec);

                // ── AL-v1: Registrar Experiencia en Adaptive Learning ─────────────────
                let al_elapsed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0)
                    .saturating_sub(al_start_ms);

                let al_result = crate::core::learning::LearningResult {
                    outcome: crate::core::learning::LearningOutcome::Success,
                    metrics: crate::core::learning::OutcomeMetrics {
                        steps: runtime.steps_taken(),
                        tool_calls: runtime.cognitive_state.metrics.tool_calls,
                        failed_actions: 0,
                        recovery_actions: runtime.recovery_count(),
                        verification_attempts: 1,
                        successful_verifications: 1,
                        elapsed_ms: al_elapsed,
                    },
                    failures: vec![],
                    recovery: None,
                };
                let _ = al_engine
                    .record_outcome(
                        al_fingerprint.clone(),
                        orchestrator_model.clone(),
                        applied_strategy.clone(),
                        al_result,
                        applied_strategy_confidence,
                        runtime.mission_id.clone(),
                        None,
                    )
                    .await;

                let mut respuesta_conv = respuesta_conv;
                if respuesta_conv.trim().is_empty() {
                    let mut files_summary = String::new();
                    for f in &journal.archivos_tocados {
                        files_summary.push_str(&format!("- `{}`\n", f));
                    }
                    respuesta_conv = format!(
                        "🎯 **Misión Concluida Exitosamente**\n\n\
                        **Objetivo:** {}\n\n\
                        **Archivos Verificados en Disco:**\n{}\n\
                        Todas las fases completaron sus criterios de aceptación y pasaron las pruebas de sintaxis.",
                        journal.objetivo,
                        if files_summary.is_empty() { "- Archivos del proyecto validados\n".to_string() } else { files_summary }
                    );
                }

                let final_res = FinalResponse {
                    status: "FINISH".to_string(),
                    respuesta_conversacional: respuesta_conv,
                };
                crate::core::hide_workspace_internal_files(&workspace_path);
                crate::llm::router::record_model_result(
                    &orchestrator_model,
                    &crate::llm::router::TaskType::Orchestrator,
                    final_res.status == "FINISH",
                    runtime.current_step(),
                );
                return Ok(serde_json::to_string(&final_res).unwrap());
            }
            _ => {
                // ── Smart unknown-tool interceptor ──────────────────────────────────────
                // Detect common patterns where the LLM invents tool names that are
                // actually shell commands. Auto-redirect to TOOL_TERMINAL.
                let tool_lower = tool.to_lowercase();
                let shell_like = tool_lower.starts_with("npm")
                    || tool_lower.starts_with("npx")
                    || tool_lower.starts_with("pip")
                    || tool_lower.starts_with("node")
                    || tool_lower.starts_with("python")
                    || tool_lower.starts_with("git ")
                    || tool_lower.starts_with("cargo")
                    || tool_lower.starts_with("rustup")
                    || tool_lower.starts_with("mkdir")
                    || tool_lower.starts_with("cd ");

                if shell_like {
                    // Convert the invented tool name into a TOOL_TERMINAL command
                    let terminal_cmd = if comando.trim().is_empty() {
                        tool.clone() // use the tool name as the command
                    } else {
                        format!("{} {}", tool, comando)
                    };
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!(
                            "[AUTO-REDIRECT] '{}' → TOOL_TERMINAL: {}",
                            tool, terminal_cmd
                        ),
                        "WARNING",
                    );
                    current_context.push_str(&format!(
                        "[SISTEMA]: La herramienta '{}' no existe. Fue redirigida automáticamente a TOOL_TERMINAL con el comando '{}'.\n\
                        RECUERDA: Para ejecutar comandos de shell SIEMPRE usa TOOL_TERMINAL con el campo 'comando'. Ejemplos: TOOL_TERMINAL+npm audit fix, TOOL_TERMINAL+npm install, TOOL_TERMINAL+pip install X.\n\n",
                        tool, terminal_cmd
                    ));
                    tool = format!("TOOL_TERMINAL");
                    comando = terminal_cmd;
                    // Authorize redirected command via runtime
                    let auto_proposal = crate::core::policy::ActionProposal {
                        tool: "TOOL_TERMINAL".to_string(),
                        arguments: serde_json::json!({ "comando": comando.clone() }),
                        expected_effect: "Auto-redirected shell command".to_string(),
                        risk: crate::core::policy::PolicyEngine::classify_terminal_command(
                            &comando,
                        ),
                        world_hash: Some(runtime.current_world_hash()),
                    };

                    if let Err(auth_err) = runtime.authorize_action(&auto_proposal) {
                        current_context
                            .push_str(&format!("[AUTO-REDIRECT RECHAZADO]: {}\n\n", auth_err));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &format!("[AUTO-REDIRECT RECHAZADO] {}", auth_err),
                            "ERROR",
                        );
                    } else {
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &format!("Ejecutando en terminal: {}", comando),
                            "ACTION",
                        );
                        comandos_ejecutados_historico.insert(format!(
                            "{}|{}",
                            comando.trim().to_lowercase(),
                            runtime.current_world_hash()
                        ));
                        // P0 fix: route through Runtime Gateway, not direct execution
                        let mut repair_decision = None;
                        let unified_auto_res = match runtime.execute_action(&auto_proposal).await {
                            Ok(obs)
                                if obs.status
                                    == crate::core::observation::ObservationStatus::Error =>
                            {
                                repair_decision = runtime.handle_observation(&obs);
                                Err(obs.payload)
                            }
                            Ok(obs) => Ok(obs),
                            Err(e) => Err(e),
                        };

                        if let Some(crate::core::recovery::RecoveryDecision::RepairCriteria {
                            criteria,
                            recommended_tool,
                        }) = repair_decision
                        {
                            emit_event(&app_handle, runtime.current_step(), &format!("[SEMANTIC_VERIFICATION] Verificador falló (auto). Transición a {}.", recommended_tool), "WARNING");
                            current_role = AgentRole::Executor;
                            forced_next_tool = Some((
                                recommended_tool.clone(),
                                format!("Corrige los siguientes criterios semánticos que fallaron:\n- {}", criteria.join("\n- "))
                            ));
                            continue;
                        }

                        match unified_auto_res {
                            Ok(obs) => {
                                current_context.push_str(&format!(
                                    "Resultado TOOL_TERMINAL (auto): {}\n\n",
                                    obs.payload
                                ));
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    &format!(
                                        "Auto-terminal OK: {}",
                                        truncate_chars(&obs.payload, 120)
                                    ),
                                    "SUCCESS",
                                );
                            }
                            Err(e) => {
                                current_context.push_str(&format!(
                                    "Resultado TOOL_TERMINAL (auto) Error: {}\n\n",
                                    e
                                ));
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    &format!("Auto-terminal Error: {}", e),
                                    "ERROR",
                                );
                            }
                        }
                    }
                } else {
                    unknown_tool_consecutive += 1;
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!("Herramienta desconocida: {}", tool),
                        "WARNING",
                    );
                    if unknown_tool_consecutive >= 3 {
                        forced_next_tool = Some((
                            "TOOL_THINK".to_string(),
                            "El sistema bloqueó mi acceso porque intenté usar herramientas inventadas que no existen en el prompt. Transfiero el control para evitar bucles de alucinación.".to_string()
                        ));
                        unknown_tool_consecutive = 0;
                        current_context.push_str(&format!("Error crítico: uso de herramienta desconocida '{}'. [FSM FORZANDO TOOL_THINK]\n\n", tool));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "[FSM] Agente inventando herramientas. Forzando TOOL_THINK.",
                            "WARNING",
                        );
                    } else {
                        current_context.push_str(&format!("Advertencia: Intentaste usar herramienta desconocida '{}'. Para comandos de shell usa TOOL_TERMINAL. Usa solo herramientas del catálogo.\n\n", tool));
                    }
                }
            }
        }

        // ─── RECORD COMMAND TRAIL ───
        let trail_role = match current_role {
            AgentRole::Planner => "Planner",
            AgentRole::Executor => "Executor",
            AgentRole::Critic => "Critic",
        };
        // FIX: Only check the NEW context added since the last step (the delta),
        // not the entire accumulated current_context which always contains past ERROR strings.
        // We use last_error_hashes as a proxy: if a hash was just added this step, it's an error.
        let step_had_error = if tool == "TOOL_FINISH" {
            false
        } else {
            // Check if the most recent terminal error hash was added THIS step
            // by checking if last_error_hashes grew this iteration (compared to pre-step size).
            // Simplified: check the last 80 chars of context for fresh error signals.
            // SAFETY: Use char-boundary-safe slice to avoid panicking on multibyte UTF-8
            // characters (e.g. accented letters, emojis in Task Charter injected by ContextMonitor).
            let context_tail = {
                let raw_offset = current_context.len().saturating_sub(800);
                // Walk forward from raw_offset until we land on a valid char boundary.
                let safe_offset = (raw_offset..=current_context.len())
                    .find(|&i| current_context.is_char_boundary(i))
                    .unwrap_or(current_context.len());
                &current_context[safe_offset..]
            };
            context_tail.contains("[PATCH_FAIL]")
                || context_tail.contains("FATAL")
                || context_tail.contains("error[E")   // Rust compiler errors
                || context_tail.contains("[ENV_FAILURE]")
                || context_tail.contains("Traceback (most recent call last)")
        };
        let trail_result = if step_had_error {
            StepResult::Error
        } else {
            StepResult::Success
        };
        let trail_error = if trail_result == StepResult::Error {
            Some(format!("Tool: {}", tool))
        } else {
            None
        };

        command_trail.add_step(
            runtime.current_step(),
            trail_role,
            &tool,
            &comando,
            archivos_vec.clone(),
            trail_result,
            trail_error,
            0, // duration - could be enhanced later
            &current_context,
        );
        command_trail.save(&workspace_path);
    } // End of while loop

    let (final_status, reason) = match &runtime.terminal_state {
        Some(crate::core::mission_runtime::RuntimeTerminalState::Failed(msg)) => {
            emit_event(
                &app_handle,
                runtime.current_step(),
                &format!("Abortando misión: {}", msg),
                "FATAL",
            );
            (
                "ERROR",
                format!("La misión ha sido abortada por un error crítico: {}", msg),
            )
        }
        Some(crate::core::mission_runtime::RuntimeTerminalState::WaitingUser(prompt)) => {
            ("WAITING_USER", prompt.clone())
        }
        Some(crate::core::mission_runtime::RuntimeTerminalState::Completed) => {
            ("FINISH", "Misión completada con éxito.".to_string())
        }
        _ => {
            emit_event(
                &app_handle,
                runtime.current_step(),
                "Límite máximo de pasos alcanzado. Bucle abortado.",
                "FATAL",
            );
            (
                "ERROR",
                format!(
                    "He alcanzado el límite máximo de {} pasos sin llegar a una conclusión. \
                 Por favor, revisa el historial de pasos y proporciona más contexto.",
                    runtime.budget.total_steps
                ),
            )
        }
    };

    // ── Journal: mark status ──
    journal.status = if final_status == "FINISH" {
        "COMPLETADO".to_string()
    } else {
        "ESPERANDO".to_string()
    };
    if let Err(e) = crate::core::session_journal::save_journal(&workspace_path, &journal) {
        emit_event(
            &app_handle,
            runtime.current_step(),
            &format!("[CHECKPOINT FAILED] {}", e),
            "FATAL",
        );
    }

    let final_res = FinalResponse {
        status: final_status.to_string(),
        respuesta_conversacional: reason,
    };
    Ok(serde_json::to_string(&final_res).unwrap())
}

/// Registers the concrete executor adapters for all 29 known tools.
/// Mission-specific policy and completion checks remain in MissionRuntime.
pub fn register_default_tools(
    runtime: &mut crate::core::mission_runtime::MissionRuntime,
    workspace_path: &str,
    original_prompt_parsed: &str,
    app_handle: Option<AppHandle>,
    orchestrator_model: String,
    agent_workspace: std::sync::Arc<
        std::sync::Mutex<
            chronos_vfs::workspace::AgentWorkspace<chronos_vfs::aura_bridge::AuraAstNode>,
        >,
    >,
) {
    use std::sync::Arc;

    // TOOL_TERMINAL: core command execution
    {
        let _ws = workspace_path.to_string();
        let _ = runtime.tool_registry.register(
            "TOOL_TERMINAL",
            Arc::new(move |ws, args| {
                let workspace = ws.clone();
                Box::pin(async move {
                    let cmd = args["comando"]
                        .as_str()
                        .or_else(|| args["command"].as_str())
                        .unwrap_or("")
                        .to_string();
                    crate::core::execute_terminal_command_detailed(&workspace, &cmd).await
                })
            }),
        );
    }

    // TOOL_WORKSPACE_MANAGER: secure deletion through WorkspaceResolver
    {
        let _ws = workspace_path.to_string();
        let _ = runtime.tool_registry.register("TOOL_WORKSPACE_MANAGER", Arc::new(move |ws, args| {
            let workspace = ws.clone();
            Box::pin(async move {
                use crate::core::workspace_resolver::WorkspaceResolver;
                let files: Vec<String> = if let Some(arr) = args.get("archivos").and_then(|v| v.as_array()) {
                    arr.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect()
                } else if let Some(arr) = args.get("files").and_then(|v| v.as_array()) {
                    arr.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect()
                } else if let Some(s) = args.get("archivo").and_then(|v| v.as_str()) {
                    vec![s.to_string()]
                } else {
                    vec![]
                };

                if files.is_empty() {
                    return Ok(crate::core::tool_registry::ExecutionResult::error(
                        "TOOL_WORKSPACE_MANAGER requiere una lista de archivos a eliminar en 'archivos'.",
                        1
                    ));
                }

                let mut borrados = Vec::new();
                let mut errores = Vec::new();

                for f in &files {
                    match WorkspaceResolver::resolve_existing_path(&workspace, f) {
                        Ok(target_path) => {
                            if target_path.is_dir() {
                                match std::fs::remove_dir_all(&target_path) {
                                    Ok(_) => borrados.push(f.clone()),
                                    Err(e) => errores.push(format!("No se pudo borrar {}: {}", f, e)),
                                }
                            } else {
                                match std::fs::remove_file(&target_path) {
                                    Ok(_) => borrados.push(f.clone()),
                                    Err(e) => errores.push(format!("No se pudo borrar {}: {}", f, e)),
                                }
                            }
                        },
                        Err(e) => {
                            errores.push(format!("Ruta rechazada por seguridad '{}': {}", f, e));
                        }
                    }
                }

                let mut out = String::new();
                if !borrados.is_empty() {
                    out.push_str(&format!("Archivos/carpetas eliminados: {:?}\n", borrados));
                }
                let err_str = errores.join("\n");
                let exit_code = if errores.is_empty() { 0 } else { 1 };

                let mut res = if exit_code == 0 {
                    crate::core::tool_registry::ExecutionResult::success(out)
                } else {
                    let mut r = crate::core::tool_registry::ExecutionResult::error(err_str, exit_code);
                    r.stdout = out;
                    r
                };
                res.files_affected = borrados;
                res.cwd = Some(workspace);
                Ok(res)
            })
        }));
    }

    // TOOL_READ_FILE: secure read through WorkspaceResolver
    {
        let _ws = workspace_path.to_string();
        let _ = runtime.tool_registry.register(
            "TOOL_READ_FILE",
            Arc::new(move |ws, args| {
                let workspace = ws.clone();
                Box::pin(async move {
                    use crate::core::workspace_resolver::WorkspaceResolver;
                    let file_arg = args
                        .get("archivo")
                        .or_else(|| args.get("file"))
                        .or_else(|| args.get("comando"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();

                    if file_arg.is_empty() {
                        return Ok(crate::core::tool_registry::ExecutionResult::error(
                            "TOOL_READ_FILE requiere el nombre o ruta del archivo en 'archivo'.",
                            1,
                        ));
                    }

                    let target =
                        match WorkspaceResolver::resolve_existing_path(&workspace, file_arg) {
                            Ok(p) => p,
                            Err(e) => {
                                return Ok(crate::core::tool_registry::ExecutionResult::error(
                                    format!(
                                        "Seguridad: Archivo fuera del workspace o inválido: {}",
                                        e
                                    ),
                                    1,
                                ));
                            }
                        };

                    match tokio::fs::read_to_string(&target).await {
                        Ok(contents) => {
                            let mut res =
                                crate::core::tool_registry::ExecutionResult::success(contents);
                            res.command = Some(file_arg.to_string());
                            res.cwd = Some(workspace);
                            Ok(res)
                        }
                        Err(e) => Ok(crate::core::tool_registry::ExecutionResult::error(
                            format!("Error leyendo {}: {}", file_arg, e),
                            1,
                        )),
                    }
                })
            }),
        );
    }

    // TOOL_PROGRAMMER: real execution via ProgrammerExecutor
    {
        let _ws = workspace_path.to_string();
        let _ = runtime.tool_registry.register(
            "TOOL_PROGRAMMER",
            Arc::new(move |ws, args| {
                let workspace = ws.clone();
                Box::pin(async move {
                    crate::core::programmer_executor::ProgrammerExecutor::execute(&workspace, args)
                        .await
                })
            }),
        );
    }

    // TOOL_TESTER: real execution via execute_tester_detailed
    {
        let _ws = workspace_path.to_string();
        let _ = runtime.tool_registry.register(
            "TOOL_TESTER",
            Arc::new(move |ws, args| {
                let workspace = ws.clone();
                Box::pin(async move {
                    let command = args
                        .get("comando")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    if crate::core::browser_automation::is_browser_test_command(command) {
                        crate::core::browser_automation::execute_browser_test_command(
                            &workspace, command,
                        )
                        .await
                    } else {
                        crate::core::tester::execute_tester_detailed(&workspace).await
                    }
                })
            }),
        );
    }

    // TOOL_FINISH: signals intent to complete; validated by CompletionGate
    {
        let _ = runtime.tool_registry.register(
            "TOOL_FINISH",
            Arc::new(|_ws, _args| {
                Box::pin(async {
                    Ok(crate::core::tool_registry::ExecutionResult::success(
                        "FINISH_SIGNALED",
                    ))
                })
            }),
        );
    }

    // TOOL_ENV_MANAGER: real execution via execute_env_manager_detailed
    {
        let _ = runtime.tool_registry.register(
            "TOOL_ENV_MANAGER",
            Arc::new(move |_ws, args| {
                Box::pin(async move {
                    let package = args
                        .get("package")
                        .or_else(|| args.get("comando"))
                        .or_else(|| args.get("command"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    crate::core::env_manager::execute_env_manager_detailed(package).await
                })
            }),
        );
    }

    // TOOL_ASSET_MANAGER: real execution via asset_fetcher with WorkspaceResolver
    {
        let _ws = workspace_path.to_string();
        let _ = runtime.tool_registry.register(
            "TOOL_ASSET_MANAGER",
            Arc::new(move |ws, args| {
                let workspace = ws.clone();
                Box::pin(async move {
                    use crate::core::workspace_resolver::WorkspaceResolver;
                    let cmd = args
                        .get("comando")
                        .or_else(|| args.get("command"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let parts: Vec<&str> = cmd.split('|').collect();
                    if parts.len() != 2 {
                        return Ok(crate::core::tool_registry::ExecutionResult::error(
                            "TOOL_ASSET_MANAGER requiere formato 'query|output_path'",
                            1,
                        ));
                    }
                    let query = parts[0].trim();
                    let rel_path = parts[1].trim();
                    let target_path =
                        match WorkspaceResolver::resolve_create_path(&workspace, rel_path) {
                            Ok(p) => p,
                            Err(e) => {
                                return Ok(crate::core::tool_registry::ExecutionResult::error(
                                    format!("Path rejected: {}", e),
                                    1,
                                ))
                            }
                        };
                    match crate::net::asset_fetcher::download_asset(
                        query,
                        &target_path.to_string_lossy(),
                    )
                    .await
                    {
                        Ok(msg) => {
                            let mut res = crate::core::tool_registry::ExecutionResult::success(msg);
                            res.files_affected = vec![rel_path.to_string()];
                            res.cwd = Some(workspace);
                            Ok(res)
                        }
                        Err(e) => Ok(crate::core::tool_registry::ExecutionResult::error(e, 1)),
                    }
                })
            }),
        );
    }

    // TOOL_BACKGROUND_START: real execution via start_background_task
    {
        let _ws = workspace_path.to_string();
        let _ = runtime.tool_registry.register(
            "TOOL_BACKGROUND_START",
            Arc::new(move |ws, args| {
                let workspace = ws.clone();
                Box::pin(async move {
                    let cmd = args
                        .get("comando")
                        .or_else(|| args.get("command"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let task_id = args
                        .get("task_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("bg_task");
                    match start_background_task(&workspace, task_id, cmd).await {
                        Ok(out) => Ok(crate::core::tool_registry::ExecutionResult::success(out)),
                        Err(e) => Ok(crate::core::tool_registry::ExecutionResult::error(e, 1)),
                    }
                })
            }),
        );
    }

    // TOOL_BACKGROUND_READ: real execution via read_task_logs
    {
        let _ = runtime.tool_registry.register(
            "TOOL_BACKGROUND_READ",
            Arc::new(move |_ws, args| {
                Box::pin(async move {
                    let task_id = args
                        .get("task_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("bg_task");
                    match read_task_logs(task_id).await {
                        Ok(logs) => Ok(crate::core::tool_registry::ExecutionResult::success(logs)),
                        Err(e) => Ok(crate::core::tool_registry::ExecutionResult::error(e, 1)),
                    }
                })
            }),
        );
    }

    // TOOL_BACKGROUND_KILL: real execution via kill_task
    {
        let _ = runtime.tool_registry.register(
            "TOOL_BACKGROUND_KILL",
            Arc::new(move |_ws, args| {
                Box::pin(async move {
                    let task_id = args
                        .get("task_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("bg_task");
                    match kill_task(task_id).await {
                        Ok(msg) => Ok(crate::core::tool_registry::ExecutionResult::success(msg)),
                        Err(e) => Ok(crate::core::tool_registry::ExecutionResult::error(e, 1)),
                    }
                })
            }),
        );
    }

    // TOOL_BACKGROUND_QUERY: alias to read_task_logs
    {
        let _ = runtime.tool_registry.register(
            "TOOL_BACKGROUND_QUERY",
            Arc::new(move |_ws, args| {
                Box::pin(async move {
                    let task_id = args
                        .get("task_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("bg_task");
                    match read_task_logs(task_id).await {
                        Ok(logs) => Ok(crate::core::tool_registry::ExecutionResult::success(logs)),
                        Err(e) => Ok(crate::core::tool_registry::ExecutionResult::error(e, 1)),
                    }
                })
            }),
        );
    }

    // TOOL_WEB_SEARCH: live web search used by the Planner and research tasks.
    {
        let _ = runtime.tool_registry.register(
            "TOOL_WEB_SEARCH",
            Arc::new(move |_ws, args| {
                Box::pin(async move {
                    let query = args
                        .get("comando")
                        .or_else(|| args.get("query"))
                        .or_else(|| args.get("url_a_investigar"))
                        .and_then(|value| value.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if query.is_empty() {
                        return Ok(crate::core::tool_registry::ExecutionResult::error(
                            "TOOL_WEB_SEARCH requiere una consulta en 'comando' o 'query'.",
                            1,
                        ));
                    }
                    match crate::net::search_web(&query).await {
                        Ok(results) => Ok(crate::core::tool_registry::ExecutionResult::success(
                            results,
                        )),
                        Err(error) => {
                            Ok(crate::core::tool_registry::ExecutionResult::error(error, 1))
                        }
                    }
                })
            }),
        );
    }

    // TOOL_WEB_SCRAPER: real execution via fetch_url_text
    {
        let _ = runtime.tool_registry.register(
            "TOOL_WEB_SCRAPER",
            Arc::new(move |_ws, args| {
                Box::pin(async move {
                    let url = args
                        .get("url_a_investigar")
                        .or_else(|| args.get("url"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    match crate::net::fetch_url_text(url).await {
                        Ok(content) => Ok(crate::core::tool_registry::ExecutionResult::success(
                            content,
                        )),
                        Err(e) => Ok(crate::core::tool_registry::ExecutionResult::error(e, 1)),
                    }
                })
            }),
        );
    }

    // TOOL_BROWSE: real execution via fetch_url_text
    {
        let _ = runtime.tool_registry.register(
            "TOOL_BROWSE",
            Arc::new(move |_ws, args| {
                Box::pin(async move {
                    let url = args
                        .get("url")
                        .or_else(|| args.get("url_a_investigar"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    match crate::net::fetch_url_text(url).await {
                        Ok(content) => Ok(crate::core::tool_registry::ExecutionResult::success(
                            content,
                        )),
                        Err(e) => Ok(crate::core::tool_registry::ExecutionResult::error(e, 1)),
                    }
                })
            }),
        );
    }

    // TOOL_GIT: real execution via execute_terminal_command_detailed
    {
        let _ws = workspace_path.to_string();
        let _ = runtime.tool_registry.register(
            "TOOL_GIT",
            Arc::new(move |ws, args| {
                let workspace = ws.clone();
                Box::pin(async move {
                    let subcmd = args
                        .get("comando")
                        .or_else(|| args.get("command"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("status");
                    let full_cmd = format!("git {}", subcmd);
                    crate::core::execute_terminal_command_detailed(&workspace, &full_cmd).await
                })
            }),
        );
    }

    // TOOL_THINK: cognitive step recorded in execution result
    {
        let _ = runtime.tool_registry.register(
            "TOOL_THINK",
            Arc::new(move |_ws, args| {
                Box::pin(async move {
                    let thought = args
                        .get("pensamiento")
                        .or_else(|| args.get("thought"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("Pensamiento cognitivo registrado.")
                        .to_string();
                    Ok(crate::core::tool_registry::ExecutionResult::success(
                        thought,
                    ))
                })
            }),
        );
    }

    // TOOL_AUDITOR: real audit reading workspace files
    {
        let _ws = workspace_path.to_string();
        let model = orchestrator_model.clone();
        let _ = runtime.tool_registry.register(
            "TOOL_AUDITOR",
            Arc::new(move |ws, args| {
                let workspace = ws.clone();
                let model = model.clone();
                Box::pin(async move {
                    let mut files: Vec<String> =
                        ["archivos", "archivos_auditar", "archivos_a_editar"]
                            .iter()
                            .filter_map(|key| args.get(*key).and_then(|value| value.as_array()))
                            .flat_map(|values| values.iter().filter_map(serde_json::Value::as_str))
                            .map(str::to_string)
                            .collect();
                    if files.is_empty() {
                        let root = match std::path::Path::new(&workspace).canonicalize() {
                            Ok(root) => root,
                            Err(error) => {
                                return Ok(crate::core::tool_registry::ExecutionResult::error(
                                    format!("TOOL_AUDITOR no pudo abrir el workspace: {}", error),
                                    1,
                                ))
                            }
                        };
                        let tree = match crate::memory::get_workspace_tree_internal(
                            root.to_string_lossy().to_string(),
                        )
                        .await
                        {
                            Ok(tree) => tree,
                            Err(error) => {
                                return Ok(crate::core::tool_registry::ExecutionResult::error(
                                    error, 1,
                                ))
                            }
                        };
                        files.extend(tree.into_iter().filter_map(|entry| {
                            if entry.is_dir
                                || !matches!(
                                    std::path::Path::new(&entry.path)
                                        .extension()
                                        .and_then(|value| value.to_str()),
                                    Some(
                                        "py" | "rs"
                                            | "js"
                                            | "ts"
                                            | "go"
                                            | "c"
                                            | "cpp"
                                            | "html"
                                            | "htm"
                                            | "css"
                                            | "json"
                                            | "md"
                                            | "toml"
                                            | "yaml"
                                            | "yml"
                                    )
                                )
                            {
                                return None;
                            }
                            std::path::Path::new(&entry.path)
                                .strip_prefix(&root)
                                .ok()
                                .map(|relative| relative.to_string_lossy().replace('\\', "/"))
                        }));
                    }
                    files.sort();
                    files.dedup();
                    if files.is_empty() {
                        return Ok(crate::core::tool_registry::ExecutionResult::error(
                            "TOOL_AUDITOR no encontró archivos de código para revisar.",
                            1,
                        ));
                    }
                    let safe_files = memory::read_files_safely(&workspace, files).await;
                    if safe_files.trim().is_empty() {
                        return Ok(crate::core::tool_registry::ExecutionResult::error(
                            "TOOL_AUDITOR no pudo leer archivos accesibles dentro del workspace.",
                            1,
                        ));
                    }
                    let report = delegate_to_auditor(&safe_files, &model).await;
                    if report.starts_with("Error en auditoría:") {
                        Ok(crate::core::tool_registry::ExecutionResult::error(
                            report, 1,
                        ))
                    } else {
                        Ok(crate::core::tool_registry::ExecutionResult::success(report))
                    }
                })
            }),
        );
    }

    // TOOL_MAPPER: real dependency analysis via analyze_workspace
    {
        let _ws = workspace_path.to_string();
        let _ = runtime.tool_registry.register(
            "TOOL_MAPPER",
            Arc::new(move |ws, _args| {
                let workspace = ws.clone();
                Box::pin(async move {
                    let graph = crate::core::dependency_mapper::analyze_workspace(&workspace);
                    let report = crate::core::dependency_mapper::format_graph_report(&graph);
                    Ok(crate::core::tool_registry::ExecutionResult::success(report))
                })
            }),
        );
    }

    // TOOL_AST_INJECT: inserts validated intent nodes into the mission's shared VFS.
    {
        let agent_workspace = agent_workspace.clone();
        let _ = runtime.tool_registry.register(
            "TOOL_AST_INJECT",
            Arc::new(move |_ws, args| {
                let agent_workspace = agent_workspace.clone();
                Box::pin(async move {
                    let nodes = if let Some(values) = args.get("nodes").and_then(|v| v.as_array()) {
                        values.iter().filter_map(|value| {
                            let intent = value.get("intent")?.as_str()?.trim();
                            if intent.is_empty() { return None; }
                            let parent_id = value.get("parent_id").and_then(serde_json::Value::as_u64).unwrap_or(0);
                            let opcode = value.get("opcode").and_then(serde_json::Value::as_u64)
                                .and_then(|value| u8::try_from(value).ok()).unwrap_or(2);
                            Some((intent.to_string(), parent_id, opcode))
                        }).collect::<Vec<_>>()
                    } else {
                        args.get("intent").or_else(|| args.get("comando"))
                            .and_then(serde_json::Value::as_str)
                            .filter(|value| !value.trim().is_empty())
                            .map(|intent| {
                                let parent_id = args.get("parent_id").and_then(serde_json::Value::as_u64).unwrap_or(0);
                                let opcode = args.get("opcode").and_then(serde_json::Value::as_u64)
                                    .and_then(|value| u8::try_from(value).ok()).unwrap_or(2);
                                vec![(intent.to_string(), parent_id, opcode)]
                            }).unwrap_or_default()
                    };
                    if nodes.is_empty() {
                        return Ok(crate::core::tool_registry::ExecutionResult::error(
                            "TOOL_AST_INJECT requiere al menos un nodo con intent no vacío.", 1,
                        ));
                    }
                    let mut report = Vec::with_capacity(nodes.len());
                    for (index, (intent, parent_id, opcode)) in nodes.iter().enumerate() {
                        let node_id = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default().as_nanos() as u64 + index as u64;
                        let node = chronos_vfs::aura_bridge::AuraIntentTranslator::tokenize_intent(
                            *opcode, *parent_id, node_id, intent, [0u8; 16],
                        );
                        let summary = format!("NodeID={} Hash={} Opcode={:?}", node.node_id, node.content_hash, node.opcode);
                        let mut workspace = match agent_workspace.lock() {
                            Ok(workspace) => workspace,
                            Err(_) => return Ok(crate::core::tool_registry::ExecutionResult::error(
                                "TOOL_AST_INJECT no pudo acceder al buffer AST compartido (mutex envenenado).", 1,
                            )),
                        };
                        if let Err(error) = workspace.push_node(node) {
                            return Ok(crate::core::tool_registry::ExecutionResult::error(
                                format!("TOOL_AST_INJECT no pudo insertar el nodo: {}", error), 1,
                            ));
                        }
                        report.push(summary);
                    }
                    Ok(crate::core::tool_registry::ExecutionResult::success(format!(
                        "{} nodo(s) insertado(s) en el buffer AST compartido: {}",
                        report.len(), report.join("; ")
                    )))
                })
            }),
        );
    }

    // TOOL_CONTAINER: real container execution
    {
        let _ws = workspace_path.to_string();
        let _ = runtime.tool_registry.register(
            "TOOL_CONTAINER",
            Arc::new(move |ws, args| {
                let workspace = ws.clone();
                Box::pin(async move {
                    let cmd = args
                        .get("comando")
                        .or_else(|| args.get("command"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let parts: Vec<&str> = cmd.splitn(3, ' ').collect();
                    if parts.len() < 2 {
                        return Ok(crate::core::tool_registry::ExecutionResult::error(
                            "TOOL_CONTAINER requiere 'accion imagen/id [comando]'",
                            1,
                        ));
                    }
                    let action_enum = crate::core::container::ContainerAction::from_str(parts[0]);
                    let image = parts[1];
                    let run_cmd = if parts.len() > 2 { parts[2] } else { "" };
                    match crate::core::container::container_exec(
                        action_enum,
                        image,
                        run_cmd,
                        &workspace,
                    )
                    .await
                    {
                        Ok(out) => Ok(crate::core::tool_registry::ExecutionResult::success(out)),
                        Err(e) => Ok(crate::core::tool_registry::ExecutionResult::error(e, 1)),
                    }
                })
            }),
        );
    }

    // TOOL_VISION_EVALUATOR: visual evaluation check via core::vision
    {
        let require_target =
            mission_requires_visual_verification(original_prompt_parsed, workspace_path);
        let _ = runtime.tool_registry.register(
            "TOOL_VISION_EVALUATOR",
            Arc::new(move |ws, args| {
                let workspace = std::path::PathBuf::from(ws);
                Box::pin(async move {
                    let prompt = args
                        .get("prompt")
                        .or_else(|| args.get("comando"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("Evalua la calidad visual de esta pantalla.");
                    let url = args.get("url").and_then(|v| v.as_str());
                    let require_target = args
                        .get("require_target")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(require_target);
                    match crate::core::vision::evaluate_vision(
                        prompt,
                        false,
                        url,
                        &workspace,
                        require_target,
                    )
                    .await
                    {
                        Ok(res) => Ok(crate::core::tool_registry::ExecutionResult::success(res)),
                        Err(e) => Ok(crate::core::tool_registry::ExecutionResult::error(
                            format!("Error visual: {}", e),
                            1,
                        )),
                    }
                })
            }),
        );
    }

    // TOOL_ASK_USER: actual interactive prompt through the Tauri event bridge
    {
        let app = app_handle.clone();
        let _ = runtime.tool_registry.register(
            "TOOL_ASK_USER",
            Arc::new(move |_ws, args| {
                let app = app.clone();
                Box::pin(async move {
                    let Some(app) = app else {
                        return Ok(crate::core::tool_registry::ExecutionResult::error(
                            "TOOL_ASK_USER requiere una interfaz Tauri activa.",
                            1,
                        ));
                    };
                    let question = args
                        .get("pregunta")
                        .or_else(|| args.get("question"))
                        .or_else(|| args.get("comando"))
                        .and_then(|v| v.as_str())
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or("Confirmación requerida")
                        .to_string();
                    let options = args
                        .get("opciones")
                        .or_else(|| args.get("options"))
                        .and_then(|value| value.as_array())
                        .map(|values| {
                            values
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .map(str::to_string)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let context = args
                        .get("context")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    match crate::core::ask_user::ask_user_async(&app, question, options, context)
                        .await
                    {
                        Ok(answer) => {
                            Ok(crate::core::tool_registry::ExecutionResult::success(answer))
                        }
                        Err(error) => {
                            Ok(crate::core::tool_registry::ExecutionResult::error(error, 1))
                        }
                    }
                })
            }),
        );
    }

    // TOOL_LEARN: index the selected workspace in persistent project memory
    {
        let _ = runtime.tool_registry.register(
            "TOOL_LEARN",
            Arc::new(move |ws, _args| {
                let workspace = ws.clone();
                Box::pin(async move {
                    match crate::core::memory::index_project(&workspace).await {
                        Ok(report) => {
                            Ok(crate::core::tool_registry::ExecutionResult::success(report))
                        }
                        Err(error) => {
                            Ok(crate::core::tool_registry::ExecutionResult::error(error, 1))
                        }
                    }
                })
            }),
        );
    }

    // TOOL_CREATE_RUNNER: real runner generation
    {
        let _ws = workspace_path.to_string();
        let prompt = original_prompt_parsed.to_string();
        let _ = runtime.tool_registry.register(
            "TOOL_CREATE_RUNNER",
            Arc::new(move |ws, _args| {
                let workspace = ws.clone();
                let p = prompt.clone();
                Box::pin(async move {
                    let runners = generate_project_runners(&workspace, &p).await;
                    let names: Vec<String> = runners
                        .iter()
                        .map(|f| {
                            f.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string()
                        })
                        .collect();
                    if names.is_empty() {
                        return Ok(crate::core::tool_registry::ExecutionResult::error(
                            "No se generaron runners: no se detectó un proyecto compatible en el workspace.",
                            1,
                        ));
                    }
                    Ok(crate::core::tool_registry::ExecutionResult::success(
                        format!("Runners generados: {:?}", names),
                    ))
                })
            }),
        );
    }

    // TOOL_SCHEDULER: real task registration
    {
        let _ws = workspace_path.to_string();
        let _ = runtime.tool_registry.register(
            "TOOL_SCHEDULER",
            Arc::new(move |ws, args| {
                let workspace = ws.clone();
                Box::pin(async move {
                    let cmd = args
                        .get("comando")
                        .or_else(|| args.get("command"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let parts: Vec<&str> = cmd.splitn(2, '|').collect();
                    if parts.len() < 2 {
                        return Ok(crate::core::tool_registry::ExecutionResult::error(
                            "TOOL_SCHEDULER requiere 'cron_expr|objetivo'",
                            1,
                        ));
                    }
                    match crate::core::scheduler::register_task(
                        parts[1].trim(),
                        &workspace,
                        parts[0].trim(),
                        parts[1].trim(),
                    ) {
                        Ok(id) => Ok(crate::core::tool_registry::ExecutionResult::success(
                            format!("Tarea programada ID {}", id),
                        )),
                        Err(error) => {
                            Ok(crate::core::tool_registry::ExecutionResult::error(error, 1))
                        }
                    }
                })
            }),
        );
    }

    // TOOL_SEARCH: real memory query & safe file reading
    {
        let _ws = workspace_path.to_string();
        let _ = runtime.tool_registry.register(
            "TOOL_SEARCH",
            Arc::new(move |ws, args| {
                let workspace = ws.clone();
                Box::pin(async move {
                    let query = args
                        .get("query")
                        .or_else(|| args.get("comando"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let memory_result = crate::core::memory::query_memory(query, &workspace).await;
                    let memory_detail = match &memory_result {
                        Ok(detail) => detail.clone(),
                        Err(error) => format!("La búsqueda en memoria falló: {}", error),
                    };
                    match memory_result {
                        Ok(msg)
                            if !msg.trim().is_empty()
                                && !msg.starts_with("La memoria de este workspace está vacía")
                                && !msg.starts_with("No se encontró contexto histórico relevante") =>
                        {
                            Ok(crate::core::tool_registry::ExecutionResult::success(msg))
                        }
                        _ => {
                            let results =
                                memory::read_files_safely(&workspace, vec![query.to_string()])
                                    .await;
                            if results.trim().is_empty() {
                                Ok(crate::core::tool_registry::ExecutionResult::error(
                                    format!("No encontré resultados relevantes de memoria ni un archivo legible para '{}'. Detalle: {}", query, memory_detail),
                                    1,
                                ))
                            } else {
                                Ok(crate::core::tool_registry::ExecutionResult::success(results))
                            }
                        }
                    }
                })
            }),
        );
    }

    // TOOL_LOGIC_SOLVER: real SAT solving via SpectraSAT
    {
        let _ = runtime.tool_registry.register(
            "TOOL_LOGIC_SOLVER",
            Arc::new(move |_ws, args| {
                Box::pin(async move {
                    if let crate::core::schema_validator::SchemaValidationResult::Invalid(
                        error,
                    ) = crate::core::schema_validator::SchemaValidator::validate_tool_payload(
                        "TOOL_LOGIC_SOLVER",
                        &args,
                    ) {
                        return Ok(crate::core::tool_registry::ExecutionResult::error(
                            error, 2,
                        ));
                    }

                    let Some(n_vars) = args
                        .get("n_vars")
                        .and_then(|value| value.as_u64())
                        .and_then(|value| usize::try_from(value).ok())
                    else {
                        return Ok(crate::core::tool_registry::ExecutionResult::error(
                            "El ejecutor SAT requiere n_vars y clauses. La revisión de código usa la ruta semántica del agente.",
                            2,
                        ));
                    };
                    let Some(clauses) = args.get("clauses").and_then(|value| value.as_array())
                    else {
                        return Ok(crate::core::tool_registry::ExecutionResult::error(
                            "El ejecutor SAT requiere 'clauses' como una matriz de cláusulas.",
                            2,
                        ));
                    };
                    let clauses: Vec<Vec<i32>> = clauses
                        .iter()
                        .map(|clause| {
                            clause
                                .as_array()
                                .expect("payload schema validated clause arrays")
                                .iter()
                                .map(|literal| {
                                    i32::try_from(
                                        literal
                                            .as_i64()
                                            .expect("payload schema validated integer literals"),
                                    )
                                    .expect("payload schema validated i32 literals")
                                })
                                .collect()
                        })
                        .collect();

                    let verdict = crate::llm::solve_with_spectrasat(n_vars, clauses);
                    let status = serde_json::from_str::<serde_json::Value>(&verdict)
                        .ok()
                        .and_then(|value| {
                            value
                                .get("status")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_string)
                        })
                        .unwrap_or_default();
                    if status.starts_with("SAT") || status.starts_with("UNSAT") {
                        Ok(crate::core::tool_registry::ExecutionResult::success(verdict))
                    } else {
                        Ok(crate::core::tool_registry::ExecutionResult::error(
                            verdict, 2,
                        ))
                    }
                })
            }),
        );
    }

    // TOOL_ARCHITECT: real execution via dependency_mapper
    {
        let _ws = workspace_path.to_string();
        let _ = runtime.tool_registry.register(
            "TOOL_ARCHITECT",
            Arc::new(move |ws, _args| {
                let workspace = ws.clone();
                Box::pin(async move {
                    let graph = crate::core::dependency_mapper::analyze_workspace(&workspace);
                    let report = crate::core::dependency_mapper::format_graph_report(&graph);
                    Ok(crate::core::tool_registry::ExecutionResult::success(report))
                })
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mission_runtime::MissionRuntime;
    use crate::core::tool_registry::KNOWN_TOOLS;

    #[test]
    fn visual_gate_is_scoped_to_ui_deliverables_and_not_existing_workspace_ui() {
        assert!(mission_requires_visual_verification(
            "Construye una aplicación web de consultas y revisa cómo se ve en navegador",
            "C:/workspace"
        ));
        assert!(!mission_requires_visual_verification(
            "Corrige el calculador de un programa de consola CLI y valida su salida",
            "C:/workspace"
        ));
        assert!(phase_has_visual_deliverable(&[
            "index.html".to_string(),
            "styles.css".to_string()
        ]));
        assert!(!phase_has_visual_deliverable(&[
            "src/main.rs".to_string(),
            "tests/cli_test.rs".to_string()
        ]));
    }

    #[test]
    fn explicit_http_server_requirement_converts_file_open_to_a_background_server() {
        let prompt = "Inicia la aplicación con un servidor HTTP local y pruébala en el navegador";
        assert!(mission_requires_local_http_server(prompt));
        assert!(is_local_html_open_command("start index.html"));
        assert_eq!(
            local_http_server_command_for_request(prompt, "start index.html"),
            Some("python -m http.server 8000 --bind 127.0.0.1")
        );
        assert_eq!(
            local_http_server_command_for_request(prompt, "npx serve -s ."),
            Some("python -m http.server 8000 --bind 127.0.0.1")
        );
        assert_eq!(
            local_http_server_command_for_request(
                "Abre el archivo HTML directamente",
                "start index.html"
            ),
            None
        );
        assert_eq!(
            local_http_server_command_for_request(prompt, "python -m http.server 8000"),
            None
        );

        let workspace =
            std::env::temp_dir().join(format!("aura-local-http-gate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("index.html"), "<main>demo</main>").unwrap();
        let workspace_path = workspace.to_string_lossy().to_string();
        assert_eq!(
            local_http_server_recovery_action(prompt, &workspace_path, None),
            Some((
                "TOOL_BACKGROUND_START".into(),
                "python -m http.server 8000 --bind 127.0.0.1".into()
            ))
        );
        assert_eq!(
            local_http_server_recovery_action(prompt, &workspace_path, Some("web-server")),
            Some(("TOOL_BACKGROUND_READ".into(), "web-server".into()))
        );
        std::fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn local_server_probe_url_is_derived_only_from_loopback_bindings() {
        assert_eq!(
            local_http_url_from_server_command("python -m http.server 8000 --bind 127.0.0.1"),
            Some("http://127.0.0.1:8000/".into())
        );
        assert_eq!(
            local_http_url_from_server_command("python -m http.server --bind localhost"),
            Some("http://127.0.0.1:8000/".into())
        );
        assert_eq!(
            local_http_url_from_server_command("python -m http.server 8123 --bind 0.0.0.0"),
            Some("http://127.0.0.1:8123/".into())
        );
        assert_eq!(
            local_http_url_from_server_command("python -m http.server 8000 --bind 192.168.1.2"),
            None
        );
        assert_eq!(local_http_url_from_server_command("npm run dev"), None);
    }

    #[test]
    fn adaptive_router_excludes_vision_and_embedding_only_models_from_text_candidates() {
        assert!(is_adaptive_text_model_candidate("qwen2.5-coder:7b"));
        assert!(is_adaptive_text_model_candidate("gemma3:4b"));
        assert!(!is_adaptive_text_model_candidate("moondream:latest"));
        assert!(!is_adaptive_text_model_candidate("llama3.2-vision:11b"));
        assert!(!is_adaptive_text_model_candidate("nomic-embed-text:latest"));

        let guidance = adaptive_strategy_guidance(
            &crate::core::learning::strategy::StrategyKind::CompileFirst,
        );
        assert!(guidance.contains("sintaxis correcta por sí sola no prueba la tarea"));
        let history = crate::core::learning::RecommendationReason::GlobalHistory { sample_size: 8 };
        assert!(!learned_strategy_has_reliable_evidence(
            0.9, &history, 18, 0
        ));
        assert!(!learned_strategy_has_reliable_evidence(0.6, &history, 8, 2));
        assert!(learned_strategy_has_reliable_evidence(0.8, &history, 8, 2));
        assert!(!learned_strategy_has_reliable_evidence(
            0.8,
            &crate::core::learning::RecommendationReason::Exploration,
            8,
            2
        ));
        assert_eq!(
            adaptive_failure_details("NO_PROGRESS_EXHAUSTED"),
            ("NoProgress", Some("TOOL_PROGRAMMER"))
        );
        assert_eq!(
            adaptive_failure_details("LOCAL_SERVER_NOT_CONFIRMED"),
            ("LocalServer", Some("TOOL_BACKGROUND_READ"))
        );
        assert_eq!(
            adaptive_failure_details("POLICY_DENY: blocked"),
            ("Policy", None)
        );
    }

    #[test]
    fn syntax_only_stall_recovery_routes_to_real_web_validation() {
        let prompt = "Construye y prueba con un servidor HTTP local y abre la página en navegador";
        assert!(should_force_web_validation_after_stall(
            prompt,
            "node --check script.js",
            1,
            None
        ));
        assert!(!should_force_web_validation_after_stall(
            prompt,
            "node --check script.js",
            0,
            None
        ));
        assert!(!should_force_web_validation_after_stall(
            prompt,
            "node --check script.js",
            1,
            Some("http://127.0.0.1:8000/")
        ));
        assert!(!should_force_web_validation_after_stall(
            "Corrige una biblioteca sin interfaz web",
            "node --check script.js",
            1,
            None
        ));

        let workspace =
            std::env::temp_dir().join(format!("aura-web-handoff-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("index.html"), "<main>demo</main>").unwrap();
        let workspace_path = workspace.to_string_lossy().to_string();
        assert_eq!(
            web_validation_handoff_action(
                prompt,
                "node --check script.js",
                1,
                None,
                &workspace_path,
                Some("active-server-task"),
            ),
            Some(("TOOL_BACKGROUND_READ".into(), "active-server-task".into()))
        );
        assert_eq!(
            web_validation_handoff_action(
                prompt,
                "node --check script.js",
                1,
                None,
                &workspace_path,
                None,
            ),
            Some((
                "TOOL_BACKGROUND_START".into(),
                "python -m http.server 8000 --bind 127.0.0.1".into()
            ))
        );
        std::fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn browser_flows_are_required_and_bound_to_the_confirmed_local_server() {
        let prompt = "Prueba en el navegador los flujos de login, registro, citas y persistencia tras recargar";
        assert!(mission_requires_browser_interaction_verification(prompt));
        assert!(!mission_requires_browser_interaction_verification(
            "Construye una página web profesional y revisa su apariencia"
        ));
        assert!(same_local_server_origin(
            "http://127.0.0.1:8000/appointments",
            "http://127.0.0.1:8000/"
        ));
        assert!(!same_local_server_origin(
            "http://127.0.0.1:8001/",
            "http://127.0.0.1:8000/"
        ));
        assert!(!same_local_server_origin(
            "https://example.com/",
            "http://127.0.0.1:8000/"
        ));

        let workspace =
            std::env::temp_dir().join(format!("aura-browser-route-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("index.html"), "<main></main>").unwrap();
        let workspace = workspace.to_string_lossy().to_string();
        assert_eq!(
            web_validation_recovery_action(prompt, &workspace, None, Some("web-task")),
            Some(("TOOL_BACKGROUND_READ".into(), "web-task".into()))
        );
        let (tool, guidance) = web_validation_recovery_action(
            prompt,
            &workspace,
            Some("http://127.0.0.1:8000/"),
            Some("web-task"),
        )
        .unwrap();
        assert_eq!(tool, "TOOL_TESTER");
        assert!(guidance.contains("BROWSER_TEST:"));
        assert!(guidance.contains("persistencia"));
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn prior_failure_episodes_are_loaded_only_for_the_same_mission_follow_up() {
        assert!(should_inject_prior_episodes(true, false));
        assert!(should_inject_prior_episodes(false, true));
        assert!(!should_inject_prior_episodes(false, false));
    }

    #[test]
    fn forced_local_server_log_and_visual_checks_are_resolved_without_a_model_guess() {
        let contract = crate::core::mission_contract::MissionContract::new(
            "Construye y valida una aplicación web local.",
        );
        let start = deterministic_forced_decision(
            "C:/workspace",
            "TOOL_BACKGROUND_START",
            "python -m http.server 8000 --bind 127.0.0.1",
            &contract,
            &[],
        )
        .unwrap();
        assert_eq!(
            start["comando"],
            "python -m http.server 8000 --bind 127.0.0.1"
        );

        let read = deterministic_forced_decision(
            "C:/workspace",
            "TOOL_BACKGROUND_READ",
            "consulta-clara-server",
            &contract,
            &[],
        )
        .unwrap();
        assert_eq!(read["task_id"], "consulta-clara-server");

        let vision = deterministic_forced_decision(
            "C:/workspace",
            "TOOL_VISION_EVALUATOR",
            "http://127.0.0.1:8000/",
            &contract,
            &[],
        )
        .unwrap();
        assert!(vision["comando"]
            .as_str()
            .unwrap()
            .contains("http://127.0.0.1:8000/"));
    }

    #[test]
    fn console_validation_scales_from_smoke_test_to_requested_workflows() {
        assert_eq!(
            console_validation_profile("Crea un programa de consola para calcular el total"),
            Some((
                "BÁSICA",
                "Ejecuta el caso principal con una entrada real; comprueba la salida observable y el código de salida. No afirmes funcionalidades que no ejecutaste."
            ))
        );
        let (_, guidance) = console_validation_profile(
            "Crea un programa de consola con login, registro, permisos, reportes y casos límite detallados",
        )
        .unwrap();
        assert!(guidance.contains("cada flujo"));
    }

    #[test]
    fn visual_review_follow_up_is_scoped_and_does_not_request_edits() {
        let review = "revisa si la pagina esta bien estruturada y biseno profesional desde el login a los modulo levanta local y prueba";
        assert!(is_scoped_visual_review(review));
        assert_eq!(
            resolve_scoped_visual_review_instruction("continua", review, true).as_deref(),
            Some(review)
        );
        assert_eq!(
            resolve_scoped_visual_review_instruction("continua", review, false),
            None
        );
        assert!(!is_scoped_visual_review(
            "revisa la pagina y corrige los defectos visuales que encuentres"
        ));
        assert!(scoped_review_forbids_action("TOOL_PROGRAMMER", ""));
        assert!(scoped_review_forbids_action("TOOL_TERMINAL", "npm install"));
        assert!(!scoped_review_forbids_action("TOOL_TERMINAL", "npm test"));
        assert!(!scoped_review_forbids_action(
            "TOOL_BACKGROUND_START",
            "npm run dev"
        ));
    }

    #[test]
    fn console_completion_requires_a_successful_current_workspace_run() {
        use crate::core::evidence::{EvidenceGraph, EvidenceKind, StructuredFact};

        assert!(is_console_runtime_command("python main.py"));
        assert!(is_console_runtime_command("cargo run --quiet"));
        assert!(!is_console_runtime_command("cargo test"));
        assert!(!is_console_runtime_command("npm test"));

        let workspace =
            std::env::temp_dir().join(format!("aura console run {}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let workspace = workspace
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let mut evidence = EvidenceGraph::new();
        evidence
            .record_structured(
                EvidenceKind::CommandExitCode,
                "TOOL_TERMINAL",
                StructuredFact::CommandResult {
                    command: "cargo test".to_string(),
                    cwd: workspace.clone(),
                    exit_code: 0,
                    stdout_hash: "test".to_string(),
                    stderr_hash: String::new(),
                },
                0.85,
                2,
                Some(42),
            )
            .unwrap();
        assert!(!has_current_console_runtime_evidence(
            &evidence, 42, &workspace
        ));

        evidence
            .record_structured(
                EvidenceKind::CommandExitCode,
                "TOOL_TERMINAL",
                StructuredFact::CommandResult {
                    command: "cargo run --quiet".to_string(),
                    cwd: workspace.clone(),
                    exit_code: 0,
                    stdout_hash: "runtime".to_string(),
                    stderr_hash: String::new(),
                },
                0.85,
                3,
                Some(42),
            )
            .unwrap();
        assert!(has_current_console_runtime_evidence(
            &evidence, 42, &workspace
        ));
        assert!(!has_current_console_runtime_evidence(
            &evidence, 43, &workspace
        ));

        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn manual_visual_fallback_uses_only_the_capture_named_by_the_failure() {
        let workspace =
            std::env::temp_dir().join(format!("aura visual evidence {}", uuid::Uuid::new_v4()));
        let directory = workspace.join(".aura").join("evidence").join("visual");
        std::fs::create_dir_all(&directory).unwrap();
        let capture = directory.join("ui-current.png");
        std::fs::write(&capture, b"image").unwrap();
        let relative = capture
            .strip_prefix(&workspace)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let error = format!("VISUAL_QA_UNCERTAIN: Captura: {relative}\nUNCERTAIN");
        assert_eq!(
            visual_capture_from_error(&workspace.to_string_lossy(), &error),
            Some(capture.canonicalize().unwrap())
        );
        assert!(visual_capture_from_error(
            &workspace.to_string_lossy(),
            "VISUAL_QA_UNCERTAIN: no se incluyó una ruta"
        )
        .is_none());
        assert!(vision_result_needs_manual_review("VISUAL_QA_UNCERTAIN"));
        assert!(!vision_result_needs_manual_review("VISUAL_QA_REJECTED"));
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn test_runner_classifier_covers_common_local_runners_without_matching_installs() {
        for command in [
            "npm test",
            "npm run test -- --watch=false",
            "pnpm test",
            "yarn test",
            "bun test",
            "node --test tests/app.test.js",
            "cargo test -p aura",
            "python -m pytest tests",
            "go test ./...",
            "dotnet test",
        ] {
            assert!(
                is_test_runner_command(command),
                "should recognize: {command}"
            );
        }
        for command in ["npm install jest", "npm run dev", "node app.js", "dir"] {
            assert!(!is_test_runner_command(command), "should reject: {command}");
        }
    }

    #[test]
    fn firebase_spark_executable_rejection_is_terminal_and_not_a_generic_retry() {
        let output = "Request to https://firebasehosting.googleapis.com/v1beta1/projects/123/sites/site/versions/abc:populateFiles had HTTP Error: 400, Executable files are forbidden on the Spark billing plan.";
        assert!(is_firebase_hosting_spark_executable_rejection(output));
        assert!(!is_firebase_hosting_spark_executable_rejection(
            "Firebase deploy failed: permission denied"
        ));
    }

    #[test]
    fn failed_test_repair_has_priority_over_phase_acceptance_validation() {
        let forced = (
            "TOOL_PROGRAMMER".to_string(),
            "Repair the failed test before retrying.".to_string(),
        );
        assert!(is_forced_test_repair(Some(&forced), true));
        assert!(!is_forced_test_repair(Some(&forced), false));
        let terminal = ("TOOL_TERMINAL".to_string(), "npm test".to_string());
        assert!(!is_forced_test_repair(Some(&terminal), true));
        assert!(only_deliverables_review_pending(&[
            "[AC-DELIVERABLES] Revisión de los entregables solicitados".into(),
        ]));
        assert!(!only_deliverables_review_pending(&[
            "[AC-DELIVERABLES] Revisión de los entregables solicitados".into(),
            "[AC-VALIDATION] Pruebas locales".into(),
        ]));
        assert!(!only_deliverables_review_pending(&[]));
        assert!(consultation_audit_diagnostic_improved(
            "app.js no conecta registro\nindex.html no tiene agenda",
            "index.html no tiene agenda",
        ));
        assert!(!consultation_audit_diagnostic_improved(
            "app.js no conecta registro\nindex.html no tiene agenda",
            "app.js no conecta registro\nindex.html no tiene agenda",
        ));
        assert!(!consultation_audit_diagnostic_improved(
            "app.js no conecta registro\nindex.html no tiene agenda",
            "app.js sigue sin conectar registro\nindex.html no tiene agenda",
        ));
        let mut audit_world = None;
        let mut audit_stalls = 0;
        let mut audit_diagnostic = String::new();
        assert_eq!(
            update_consultation_audit_stall_count(
                &mut audit_world,
                &mut audit_stalls,
                &mut audit_diagnostic,
                41,
                "app.js sin eventos\nindex.html sin formulario",
            ),
            0
        );
        assert_eq!(
            update_consultation_audit_stall_count(
                &mut audit_world,
                &mut audit_stalls,
                &mut audit_diagnostic,
                41,
                "app.js sin eventos\nindex.html sin formulario",
            ),
            1
        );
        assert_eq!(
            update_consultation_audit_stall_count(
                &mut audit_world,
                &mut audit_stalls,
                &mut audit_diagnostic,
                41,
                "index.html sin formulario",
            ),
            0,
            "an improving audit must reset the stale-world retry counter"
        );
        assert_eq!(
            update_consultation_audit_stall_count(
                &mut audit_world,
                &mut audit_stalls,
                &mut audit_diagnostic,
                41,
                "index.html sin formulario",
            ),
            1
        );
        assert_eq!(
            update_consultation_audit_stall_count(
                &mut audit_world,
                &mut audit_stalls,
                &mut audit_diagnostic,
                41,
                "index.html sin formulario",
            ),
            2
        );
        reset_consultation_audit_stall_tracking(
            &mut audit_world,
            &mut audit_stalls,
            &mut audit_diagnostic,
        );
        assert_eq!(audit_stalls, 0);
        let jest_failure = "> node --test tests/app.test.js\nReferenceError: jest is not defined";
        assert!(is_node_test_jest_mock_failure(jest_failure));
        assert!(is_node_test_jest_mock_failure(
            "> node --test tests/app.test.js\nTypeError: mockStorage.getItem.mockReturnValue is not a function"
        ));
        assert!(is_node_test_jest_mock_failure(
            "> node --test tests/app.test.js\nTypeError: mockStorage.getItem.mockImplementation is not a function"
        ));
        assert!(!is_node_test_jest_mock_failure(
            "Jest is not defined in a project using a different test runner"
        ));
        let undefined_push = "> node --test tests/app.test.js\nTypeError: Cannot read properties of undefined (reading 'push')\n    at registerUser (file:///workspace/app.js:3:15)";
        assert!(is_node_test_undefined_push_failure(undefined_push));
        assert!(!is_node_test_undefined_push_failure(
            "TypeError: Cannot read properties of undefined (reading 'push')"
        ));
        let storage_mock_failure = "> node --test tests/app.test.js\nTypeError: Cannot read properties of undefined (reading 'set')\n    at Object.setItem (file:///workspace/tests/app.test.js:8:38)";
        assert!(is_node_test_uninitialized_storage_mock_failure(
            storage_mock_failure
        ));
        assert!(!is_node_test_uninitialized_storage_mock_failure(
            "TypeError: Cannot read properties of undefined (reading 'set')"
        ));
        let browser_document_failure = "> node --test tests/app.test.js\nReferenceError: document is not defined\n    at file:///workspace/app.js:72:3";
        assert!(is_node_test_browser_document_failure(
            browser_document_failure
        ));
        assert!(!is_node_test_browser_document_failure(
            "ReferenceError: document is not defined"
        ));

        let mut tool = "TOOL_TERMINAL".to_string();
        let mut role = AgentRole::Critic;
        let mut command = "npm test".to_string();
        let mut files = vec![
            "app.js".to_string(),
            "tests/app.test.js".to_string(),
            "index.html".to_string(),
        ];
        let mut payload = serde_json::json!({
            "herramienta": "TOOL_TERMINAL",
            "comando": "npm test"
        });
        let mut diagnostic = String::new();

        apply_forced_test_repair_override(
            &mut tool,
            &mut role,
            &mut command,
            &mut files,
            &mut payload,
            &mut diagnostic,
            "npm test",
            jest_failure,
        );

        assert_eq!(tool, "TOOL_PROGRAMMER");
        assert_eq!(role, AgentRole::Executor);
        assert!(command.is_empty());
        assert_eq!(files, vec!["tests/app.test.js"]);
        assert_eq!(payload["herramienta"], "TOOL_PROGRAMMER");
        assert_eq!(payload["archivos_a_editar"], serde_json::json!(files));
        assert!(payload["instruccion"]
            .as_str()
            .unwrap()
            .contains("node:test"));
        assert!(payload["instruccion"].as_str().unwrap().contains("Map"));
        assert!(payload["instruccion"]
            .as_str()
            .unwrap()
            .contains("No uses jest.fn()"));
        assert!(diagnostic.contains("jest is not defined"));
    }

    #[test]
    fn node_test_undefined_push_repair_targets_source_and_tests_with_specific_guidance() {
        let failure = "> node --test tests/app.test.js\nTypeError: Cannot read properties of undefined (reading 'push')\n    at registerUser (file:///workspace/app.js:3:15)";
        let mut tool = "TOOL_TERMINAL".to_string();
        let mut role = AgentRole::Critic;
        let mut command = "npm test".to_string();
        let mut files = vec![
            "app.js".to_string(),
            "tests/app.test.js".to_string(),
            "index.html".to_string(),
            "styles.css".to_string(),
        ];
        let mut payload = serde_json::json!({"herramienta":"TOOL_TERMINAL","comando":"npm test"});
        let mut diagnostic = String::new();

        apply_forced_test_repair_override(
            &mut tool,
            &mut role,
            &mut command,
            &mut files,
            &mut payload,
            &mut diagnostic,
            "npm test",
            failure,
        );

        assert_eq!(tool, "TOOL_PROGRAMMER");
        assert_eq!(files, vec!["app.js", "tests/app.test.js"]);
        let instruction = payload["instruccion"].as_str().unwrap();
        assert!(instruction.contains("colecciones ausentes"));
        assert!(instruction.contains("claves correctas"));
        assert!(instruction.contains("no elimines ni debilites pruebas"));
        assert!(diagnostic.contains("app.js"));
    }

    #[test]
    fn node_test_uninitialized_storage_mock_repair_targets_tests_only() {
        let failure = "> node --test tests/app.test.js\nTypeError: Cannot read properties of undefined (reading 'set')\n    at Object.setItem (file:///workspace/tests/app.test.js:8:38)";
        let mut tool = "TOOL_TERMINAL".to_string();
        let mut role = AgentRole::Critic;
        let mut command = "npm test".to_string();
        let mut files = vec![
            "app.js".to_string(),
            "tests/app.test.js".to_string(),
            "index.html".to_string(),
        ];
        let mut payload = serde_json::json!({"herramienta":"TOOL_TERMINAL","comando":"npm test"});
        let mut diagnostic = String::new();

        apply_forced_test_repair_override(
            &mut tool,
            &mut role,
            &mut command,
            &mut files,
            &mut payload,
            &mut diagnostic,
            "npm test",
            failure,
        );

        assert_eq!(tool, "TOOL_PROGRAMMER");
        assert_eq!(files, vec!["tests/app.test.js"]);
        let instruction = payload["instruccion"].as_str().unwrap();
        assert!(instruction.contains("const storage = new Map()"));
        assert!(instruction.contains("no cambies el runner ni app.js"));
        assert!(diagnostic.contains("reading 'set'"));
    }

    #[test]
    fn node_test_browser_document_failure_repairs_the_source_module_only() {
        let failure = "> node --test tests/app.test.js\nReferenceError: document is not defined\n    at file:///workspace/app.js:72:3";
        let mut tool = "TOOL_TERMINAL".to_string();
        let mut role = AgentRole::Critic;
        let mut command = "npm test".to_string();
        let mut files = vec![
            "app.js".to_string(),
            "tests/app.test.js".to_string(),
            "index.html".to_string(),
        ];
        let mut payload = serde_json::json!({"herramienta":"TOOL_TERMINAL","comando":"npm test"});
        let mut diagnostic = String::new();

        apply_forced_test_repair_override(
            &mut tool,
            &mut role,
            &mut command,
            &mut files,
            &mut payload,
            &mut diagnostic,
            "npm test",
            failure,
        );

        assert_eq!(tool, "TOOL_PROGRAMMER");
        assert_eq!(files, vec!["app.js"]);
        assert!(payload["instruccion"]
            .as_str()
            .unwrap()
            .contains("typeof document !== 'undefined'"));
        assert!(diagnostic.contains("document is not defined"));
    }

    #[test]
    fn terminal_observation_reuses_recovery_decision_without_double_counting() {
        let mut runtime = MissionRuntime::new(".", "test recovery counter", 10);
        for expected_count in 1..=3 {
            let observation = crate::core::observation::Observation::error(
                "TOOL_TERMINAL",
                "test failed",
                Some(1),
                true,
                None,
            );
            let observed = runtime
                .handle_observation(&observation)
                .expect("terminal error should produce one recovery decision");
            let selected = reuse_or_record_recovery_decision(
                &mut runtime,
                Some(observed.clone()),
                "TOOL_TERMINAL",
                "test failed",
            );
            assert_eq!(selected, observed);
            assert!(!matches!(
                selected,
                crate::core::recovery::RecoveryDecision::Abort { .. }
            ));
            assert_eq!(runtime.recovery.total_recoveries(), expected_count);
        }
    }

    #[test]
    fn programmer_file_changes_do_not_clear_failed_terminal_retry_budget() {
        let mut runtime = MissionRuntime::new(".", "test progress retry reset", 10);
        let mut retry_tracker = crate::core::error_classifier::RetryTracker::new();

        for _ in 0..4 {
            runtime.plan_recovery("TOOL_TERMINAL", "test failed");
            retry_tracker.record_failure(
                "TOOL_TERMINAL",
                &crate::core::error_classifier::ErrorType::Logic,
            );
        }
        assert_eq!(runtime.recovery.total_recoveries(), 4);
        assert_eq!(retry_tracker.total_error_count, 4);
        assert!(!workspace_changed_after_programmer(&[], 0));
        assert!(workspace_changed_after_programmer(&["app.js".into()], 0,));
        assert_eq!(runtime.recovery.total_recoveries(), 4);
        assert_eq!(retry_tracker.total_error_count, 4);

        reset_terminal_retry_circuits_after_success(&mut runtime, &mut retry_tracker);
        assert_eq!(runtime.recovery.total_recoveries(), 0);
        assert_eq!(retry_tracker.total_error_count, 0);

        for _ in 0..4 {
            let decision = runtime.plan_recovery("TOOL_TERMINAL", "test failed");
            assert!(!matches!(
                decision,
                crate::core::recovery::RecoveryDecision::Abort { .. }
            ));
            retry_tracker.record_failure(
                "TOOL_TERMINAL",
                &crate::core::error_classifier::ErrorType::Logic,
            );
        }
    }

    #[test]
    fn approval_parser_accepts_explicit_answers_and_rejects_qualified_or_negative_ones() {
        for answer in ["Sí", "SI, AUTORIZO", "Autorizar esta acción", "I approve"] {
            assert!(is_explicit_approval(answer), "should accept: {answer}");
        }
        for answer in [
            "",
            "No, autorizo",
            "Sí, pero solo después de revisar",
            "Yes, but do not run it yet",
            "continuar",
        ] {
            assert!(!is_explicit_approval(answer), "should reject: {answer}");
        }
    }

    #[test]
    fn sat_input_parsing_never_silently_drops_invalid_literals() {
        let valid = parse_sat_payload(&serde_json::json!({
            "n_vars": 2,
            "clauses": [[1, -2], []]
        }))
        .unwrap()
        .unwrap();
        assert_eq!(valid, (2, vec![vec![1, -2], vec![]]));

        assert!(parse_sat_payload(&serde_json::json!({
            "n_vars": 1,
            "clauses": [[1, "ignored"]]
        }))
        .is_err());
        assert!(extract_sat_payload("Resuelve [[1, no-es-un-literal]]").is_err());
        assert_eq!(
            extract_sat_payload("Analiza la lógica de este código").unwrap(),
            None
        );
    }

    #[test]
    fn sat_input_can_be_recovered_from_the_user_message_without_changing_literals() {
        let instance = extract_sat_payload("Resuelve este CNF: [[1, -2], [2]]")
            .unwrap()
            .unwrap();
        assert_eq!(instance, (2, vec![vec![1, -2], vec![2]]));
    }

    #[test]
    fn python_module_commands_are_not_treated_as_workspace_scripts() {
        assert_eq!(python_script_argument("python -m unittest -v"), None);
        assert_eq!(python_script_argument("python3 -m pytest tests"), None);
        assert_eq!(python_script_argument("py -c print(1)"), None);
        assert_eq!(
            python_script_argument("python -X utf8 verify.py"),
            Some("verify.py".into())
        );
        assert_eq!(
            python_script_argument("python verify.py"),
            Some("verify.py".into())
        );
    }

    #[test]
    fn web_consultation_mission_gets_a_javascript_only_implementation_contract() {
        let blueprint = tactical_repair_blueprint(
            "[MODO IMPLEMENTACION DE PLAN APROBADO] aplicación web de consulta, facturas, contabilidad y Firebase",
        );
        assert!(blueprint.contains("SOLAMENTE con HTML, CSS y JavaScript"));
        assert!(blueprint.contains("no uses Python/Flask"));
        assert!(blueprint.contains("localStorage"));
        assert!(blueprint.contains("node:test"));
        assert!(blueprint.contains("autenticación segura de producción"));
    }

    #[test]
    fn nested_javascript_tests_are_recognized_as_tests() {
        assert!(is_test_artifact_path("tests/app.test.js"));
        assert!(is_test_artifact_path("tests\\ledger.spec.ts"));
        assert!(is_test_artifact_path("verify_solution.py"));
        assert!(!is_test_artifact_path("app.js"));
        assert_eq!(
            test_command_for_path("tests/app.test.js").as_deref(),
            Some("node --test \"tests/app.test.js\"")
        );
        assert_eq!(
            test_command_for_path("checks/verify_index.py").as_deref(),
            Some("python \"checks/verify_index.py\"")
        );
        assert_eq!(test_command_for_path("tests/suite.rs"), None);
    }

    #[test]
    fn node_precheck_parses_quoted_script_paths_and_ignores_inline_code() {
        assert_eq!(
            node_script_argument("node --test \"app.test.js\""),
            Some("app.test.js".into())
        );
        assert_eq!(
            node_script_argument("node --check \"tests/Consulta Clara.test.js\""),
            Some("tests/Consulta Clara.test.js".into())
        );
        assert_eq!(node_script_argument("node -e \"console.log(1)\""), None);
    }

    #[test]
    fn repeated_programmer_loop_routes_to_a_real_available_check() {
        let contract = crate::core::mission_contract::MissionContract::new(
            "Construye una aplicación web local y verifica sus flujos.",
        );
        let workspace = std::env::temp_dir().join(format!(
            "aura-programmer-loop-verifier-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(workspace.join("tests")).unwrap();
        std::fs::write(workspace.join("script.js"), "console.log('app');\n").unwrap();
        std::fs::write(workspace.join("index.html"), "<main></main>\n").unwrap();
        let workspace_path = workspace.to_string_lossy().to_string();

        assert_eq!(
            programmer_loop_validation_command(
                &["index.html".into(), "script.js".into(), "styles.css".into()],
                &contract,
                &workspace_path,
            )
            .as_deref(),
            Some("node --check \"script.js\"")
        );
        let fake_test = "const { test, expect } = require('node:test');\nfunction testLogin() {}\nmodule.exports = { testLogin };\n";
        std::fs::write(workspace.join("tests/app.test.js"), fake_test).unwrap();
        assert_eq!(
            programmer_loop_validation_command(
                &[
                    "index.html".into(),
                    "script.js".into(),
                    "tests/app.test.js".into()
                ],
                &contract,
                &workspace_path,
            )
            .as_deref(),
            Some("node --check \"script.js\"")
        );
        std::fs::write(
            workspace.join("tests/app.test.js"),
            "const { test } = require('node:test');\ntest('renders appointments', () => {});\n",
        )
        .unwrap();
        assert_eq!(
            programmer_loop_validation_command(
                &[
                    "index.html".into(),
                    "script.js".into(),
                    "tests/app.test.js".into()
                ],
                &contract,
                &workspace_path,
            )
            .as_deref(),
            Some("node --check \"script.js\""),
            "an empty Node test callback must not be treated as a useful validator"
        );
        let behavioral_test = "const { test } = require('node:test');\nconst assert = require('node:assert/strict');\ntest('renders appointments', () => { assert.equal(1, 1); });\n";
        std::fs::write(workspace.join("tests/app.test.js"), behavioral_test).unwrap();
        assert_eq!(
            programmer_loop_validation_command(
                &[
                    "index.html".into(),
                    "script.js".into(),
                    "tests/app.test.js".into()
                ],
                &contract,
                &workspace_path,
            )
            .as_deref(),
            Some("node --test \"tests/app.test.js\"")
        );
        assert!(successful_test_run_has_behavioral_cases(
            "node --test \"tests/app.test.js\"",
            &workspace_path,
            "ℹ tests 2\nℹ pass 2\n"
        ));
        let fake_dom_test = "const { test, expect } = require('node:test');\ntest('login', () => { expect(console.log).toHaveBeenCalledWith('ok'); });\n";
        std::fs::write(workspace.join("tests/app.test.js"), fake_dom_test).unwrap();
        assert!(!test_file_has_runnable_cases(
            &workspace_path,
            "tests/app.test.js"
        ));
        std::fs::write(
            workspace.join("tests/app.test.js"),
            "const { test } = require('node:test');\ntest('renders appointments', () => {});\n",
        )
        .unwrap();
        assert!(!successful_test_run_has_behavioral_cases(
            "node --test \"tests/app.test.js\"",
            &workspace_path,
            "✔ app.test.js (50ms)\nℹ tests 1\nℹ pass 1\n"
        ));
        std::fs::remove_file(workspace.join("tests/app.test.js")).unwrap();
        assert_eq!(
            programmer_loop_validation_command(&["index.html".into()], &contract, &workspace_path,),
            None
        );
        assert_eq!(
            programmer_loop_validation_command(
                &[
                    "index.html".into(),
                    "missing.js".into(),
                    "tests/app.test.js".into()
                ],
                &contract,
                &workspace_path,
            ),
            None,
            "stale workspace entries must not be sent to the terminal as validators"
        );
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn empty_programmer_target_recovers_only_from_the_active_phase_or_contract() {
        let root =
            std::env::temp_dir().join(format!("aura-schema-target-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let workspace = root.to_string_lossy().to_string();
        let mut journal = crate::core::session_journal::SessionJournal::default();
        journal.fases.push(crate::core::session_journal::Fase {
            numero: 1,
            archivos: vec!["index.html".into(), "app.js".into()],
            ..Default::default()
        });

        assert_eq!(
            schema_recovery_programmer_target(&journal, &workspace, &[]).as_deref(),
            Some("index.html")
        );
        std::fs::write(root.join("index.html"), "<main></main>").unwrap();
        assert_eq!(
            schema_recovery_programmer_target(&journal, &workspace, &[]).as_deref(),
            Some("app.js")
        );
        std::fs::write(root.join("app.js"), "export {};\n").unwrap();
        assert_eq!(
            schema_recovery_programmer_target(&journal, &workspace, &[]).as_deref(),
            Some("index.html")
        );

        journal.fases.clear();
        assert_eq!(
            schema_recovery_programmer_target(&journal, &workspace, &[]),
            None,
            "without a planned or contract target, recovery must not invent a filename"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn consultation_audit_overrides_a_repeated_test_command_with_a_targeted_repair() {
        let audit = crate::llm::phase_planner::ConsultationMvpAudit {
            issues: vec![
                "app.js contiene un stub".into(),
                "tests/app.test.js no registra casos".into(),
            ],
            repair_files: vec!["app.js".into(), "tests/app.test.js".into()],
        };
        let mut tool = "TOOL_TERMINAL".to_string();
        let mut role = AgentRole::Critic;
        let mut command = "npm test".to_string();
        let mut files = Vec::new();
        let mut payload = serde_json::json!({"herramienta":"TOOL_TERMINAL", "comando":"npm test"});
        let mut diagnostic = String::new();

        assert!(apply_consultation_mvp_audit_override(
            &mut tool,
            &mut role,
            &mut command,
            &mut files,
            &mut payload,
            &mut diagnostic,
            &audit,
        ));
        assert_eq!(tool, "TOOL_PROGRAMMER");
        assert_eq!(role, AgentRole::Executor);
        assert!(command.is_empty());
        assert_eq!(files, vec!["tests/app.test.js"]);
        assert_eq!(payload["herramienta"], "TOOL_PROGRAMMER");
        assert_eq!(
            payload["archivos_a_editar"],
            serde_json::json!(["tests/app.test.js"])
        );
        assert_eq!(payload["require_all_targets"], true);
        assert!(diagnostic.contains("tests/app.test.js no registra casos"));
        assert!(!diagnostic.contains("app.js contiene un stub"));
    }

    #[test]
    fn approved_mission_replaces_stale_generic_phase_plan_until_a_phase_is_complete() {
        let mut journal = crate::core::session_journal::SessionJournal::default();
        journal.plan_generado = true;
        journal.fases = vec![crate::core::session_journal::Fase {
            numero: 1,
            descripcion: "Create project structure".into(),
            archivos: vec!["README.md".into(), "requirements.txt".into()],
            criterio_de_exito: "python --version".into(),
            estado: "EN_PROGRESO".into(),
        }];
        assert!(needs_approved_plan_recovery(true, &journal));
        journal.fases[0].estado = "COMPLETADA".into();
        journal.fases.push(crate::core::session_journal::Fase {
            numero: 2,
            descripcion: "Build application".into(),
            archivos: vec!["app.py".into()],
            criterio_de_exito: "python app.py".into(),
            estado: "EN_PROGRESO".into(),
        });
        assert!(needs_approved_plan_recovery(true, &journal));
        journal.fases[0] = crate::core::session_journal::Fase {
            numero: 1,
            descripcion: "MVP web local".into(),
            archivos: vec![
                "index.html".into(),
                "styles.css".into(),
                "app.js".into(),
                "package.json".into(),
                "tests/app.test.js".into(),
            ],
            criterio_de_exito: "npm test".into(),
            estado: "COMPLETADA".into(),
        };
        journal.fases.push(crate::core::session_journal::Fase {
            numero: 2,
            descripcion: "Firebase Auth y Firestore con emuladores".into(),
            archivos: vec!["firebase-config.js".into()],
            criterio_de_exito: "npm test".into(),
            estado: "EN_PROGRESO".into(),
        });
        journal.fases.push(crate::core::session_journal::Fase {
            numero: 3,
            descripcion: "Hosting".into(),
            archivos: vec![".firebaserc".into()],
            criterio_de_exito: "firebase deploy --only hosting".into(),
            estado: "PENDIENTE".into(),
        });
        assert!(needs_approved_plan_recovery(true, &journal));
        journal.fases[1].criterio_de_exito = "npm run test:firebase".into();
        assert!(!needs_approved_plan_recovery(true, &journal));
        assert!(!needs_approved_plan_recovery(false, &journal));
    }

    #[test]
    fn resumed_local_agenda_replaces_a_phase_that_only_mentions_login() {
        let mut journal = crate::core::session_journal::SessionJournal::default();
        journal.plan_generado = true;
        journal.fases = vec![crate::core::session_journal::Fase {
            numero: 1,
            descripcion: "Implementar login y registro, validación y buen contraste".into(),
            archivos: vec!["index.html".into(), "styles.css".into(), "script.js".into()],
            criterio_de_exito: "Abrir en navegador".into(),
            estado: "EN_PROGRESO".into(),
        }];
        assert!(needs_local_consultation_agenda_plan_recovery(
            true, &journal
        ));
        journal.fases[0].estado = "COMPLETADA".into();
        assert!(!needs_local_consultation_agenda_plan_recovery(
            true, &journal
        ));
        assert!(!needs_local_consultation_agenda_plan_recovery(
            false, &journal
        ));
    }

    #[test]
    fn completed_phase_plan_does_not_request_an_arbitrary_programmer_target() {
        let root = std::env::temp_dir().join(format!("aura-phase-files-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("tests")).unwrap();
        for file in [
            "index.html",
            "styles.css",
            "app.js",
            "package.json",
            "tests/app.test.js",
        ] {
            let path = root.join(file);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, "present").unwrap();
        }
        let journal = crate::core::session_journal::SessionJournal {
            fases: vec![crate::core::session_journal::Fase {
                numero: 1,
                descripcion: "MVP web local".into(),
                archivos: vec![
                    "index.html".into(),
                    "styles.css".into(),
                    "app.js".into(),
                    "package.json".into(),
                    "tests/app.test.js".into(),
                ],
                criterio_de_exito: "npm test".into(),
                estado: "EN_PROGRESO".into(),
            }],
            ..Default::default()
        };
        assert_eq!(
            phase_pending_programmer_file(&journal, root.to_str().unwrap()),
            None
        );
        assert!(programmer_file_fallback(&journal, root.to_str().unwrap(), &[]).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn programmer_recovers_missing_or_internal_targets_from_the_active_phase() {
        let root =
            std::env::temp_dir().join(format!("aura-programmer-args-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let mut journal = crate::core::session_journal::SessionJournal::default();
        journal.fases = vec![crate::core::session_journal::Fase {
            numero: 1,
            descripcion: "MVP local".into(),
            archivos: vec!["index.html".into(), "styles.css".into(), "app.js".into()],
            criterio_de_exito: "pruebas locales".into(),
            estado: "EN_PROGRESO".into(),
        }];

        let mut omitted = serde_json::json!({"archivos_a_editar": []});
        let first =
            recover_programmer_file_args(&mut omitted, &journal, root.to_str().unwrap(), &[])
                .expect("empty tool args should recover the first pending phase deliverable");
        assert_eq!(first, vec!["index.html"]);
        assert_eq!(
            omitted["archivos_a_editar"],
            serde_json::json!(["index.html"])
        );

        std::fs::write(root.join("index.html"), "<!doctype html>").unwrap();
        let mut internal_only = serde_json::json!({"archivos_a_editar": ["fenix_chat.json"]});
        let next =
            recover_programmer_file_args(&mut internal_only, &journal, root.to_str().unwrap(), &[])
                .expect("internal-only targets should recover the next pending product file");
        assert_eq!(next, vec!["styles.css"]);
        assert_eq!(
            internal_only["archivos_a_editar"],
            serde_json::json!(["styles.css"])
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn forced_think_reuses_diagnosis_without_an_orchestrator_retry() {
        let reason = "Analiza ModuleNotFoundError: flask y propone una corrección.";
        let contract = crate::core::mission_contract::MissionContract::from_objective(
            "Corrige la aplicación web",
        );
        let decision = deterministic_forced_decision(".", "TOOL_THINK", reason, &contract, &[])
            .expect("forced TOOL_THINK is a deterministic control transfer");
        assert_eq!(decision["herramienta"], "TOOL_THINK");
        assert_eq!(decision["comando"], reason);
    }

    #[tokio::test]
    async fn test_all_29_known_tools_have_registered_executors() {
        let temp_dir =
            std::env::temp_dir().join(format!("aura_tools_test_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let ws = temp_dir.to_str().unwrap();

        let mut runtime = MissionRuntime::new(ws, "Tool inventory verification mission", 50);
        let ast_workspace = std::sync::Arc::new(std::sync::Mutex::new(
            chronos_vfs::workspace::AgentWorkspace::<chronos_vfs::aura_bridge::AuraAstNode>::new(1)
                .unwrap(),
        ));
        register_default_tools(
            &mut runtime,
            ws,
            "Build a high reliability verification system",
            None,
            "test-model".to_string(),
            ast_workspace.clone(),
        );

        // 1. Assert exactly 29 tools in KNOWN_TOOLS
        assert_eq!(
            KNOWN_TOOLS.len(),
            29,
            "KNOWN_TOOLS must contain exactly 29 tools"
        );

        // 2. Assert all 29 known tools are registered
        for tool_name in KNOWN_TOOLS {
            assert!(
                runtime.tool_registry.is_registered(tool_name),
                "Missing registered executor for tool '{}'",
                tool_name
            );
        }

        // 3. Assert registered count in ToolRegistry matches KNOWN_TOOLS.len()
        assert_eq!(
            runtime.tool_registry.registered_count(),
            29,
            "ToolRegistry must have exactly 29 registered executors"
        );

        // 4. Assert dispatching to each tool resolves to a real executor and doesn't fail with TOOL_UNREGISTERED
        let think_res = runtime
            .tool_registry
            .dispatch(
                "TOOL_THINK",
                serde_json::json!({ "thought": "cogito" }),
                ".",
            )
            .await;
        assert!(think_res.is_ok());
        assert_eq!(think_res.unwrap().stdout, "cogito");

        let finish_res = runtime
            .tool_registry
            .dispatch("TOOL_FINISH", serde_json::Value::Null, ".")
            .await;
        assert!(finish_res.is_ok());
        assert_eq!(finish_res.unwrap().stdout, "FINISH_SIGNALED");

        let logic_res = runtime
            .tool_registry
            .dispatch(
                "TOOL_LOGIC_SOLVER",
                serde_json::json!({
                    "n_vars": 1,
                    "clauses": [[1]]
                }),
                ".",
            )
            .await;
        assert!(logic_res.is_ok());
        assert!(logic_res.unwrap().stdout.contains("SAT_CERTIFIED"));

        let missing_logic_input = runtime
            .tool_registry
            .dispatch("TOOL_LOGIC_SOLVER", serde_json::Value::Null, ".")
            .await
            .unwrap();
        assert_ne!(missing_logic_input.exit_code, 0);
        assert!(missing_logic_input
            .stderr
            .contains("requiere n_vars y clauses"));

        let ast_result = runtime
            .tool_registry
            .dispatch(
                "TOOL_AST_INJECT",
                serde_json::json!({ "intent": "register a real AST node" }),
                ws,
            )
            .await
            .unwrap();
        assert_eq!(ast_result.exit_code, 0);
        assert!(ast_result.stdout.contains("insertado"));
        let ast_capacity_result = runtime
            .tool_registry
            .dispatch(
                "TOOL_AST_INJECT",
                serde_json::json!({ "intent": "the shared buffer is now full" }),
                ws,
            )
            .await
            .unwrap();
        assert_ne!(ast_capacity_result.exit_code, 0);
        assert!(ast_capacity_result.stderr.contains("Buffer full"));

        let ask_user_without_ui = runtime
            .tool_registry
            .dispatch(
                "TOOL_ASK_USER",
                serde_json::json!({ "question": "Are you there?" }),
                ws,
            )
            .await
            .unwrap();
        assert_ne!(ask_user_without_ui.exit_code, 0);
        assert!(ask_user_without_ui.stderr.contains("interfaz Tauri activa"));

        let empty_audit = runtime
            .tool_registry
            .dispatch("TOOL_AUDITOR", serde_json::Value::Null, ws)
            .await
            .unwrap();
        assert_ne!(empty_audit.exit_code, 0);
        assert!(empty_audit.stderr.contains("no encontró archivos"));

        let empty_runner = runtime
            .tool_registry
            .dispatch("TOOL_CREATE_RUNNER", serde_json::Value::Null, ws)
            .await
            .unwrap();
        assert_ne!(empty_runner.exit_code, 0);
        assert!(empty_runner.stderr.contains("no se detectó un proyecto"));

        let missing_search = runtime
            .tool_registry
            .dispatch(
                "TOOL_SEARCH",
                serde_json::json!({ "query": format!("missing_{}", uuid::Uuid::new_v4()) }),
                ws,
            )
            .await
            .unwrap();
        assert_ne!(missing_search.exit_code, 0);
        assert!(missing_search.stderr.contains("No encontré resultados"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_pesp_file_writes_do_not_complete_or_advance_a_phase() {
        let temp_dir = std::env::temp_dir().join("aura_test_pesp_advancement");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let ws = temp_dir.to_str().unwrap();

        let mut journal = crate::core::session_journal::load_journal(ws);
        journal.fases = vec![
            crate::core::session_journal::Fase {
                numero: 1,
                descripcion: "Dashboard UI".to_string(),
                archivos: vec![
                    "cyber_sentinel.html".to_string(),
                    "style.css".to_string(),
                    "script.js".to_string(),
                ],
                criterio_de_exito: "dir".to_string(),
                estado: "PENDIENTE".to_string(),
            },
            crate::core::session_journal::Fase {
                numero: 2,
                descripcion: "Verification Script".to_string(),
                archivos: vec!["verify_dashboard.py".to_string()],
                criterio_de_exito: "python verify_dashboard.py".to_string(),
                estado: "PENDIENTE".to_string(),
            },
        ];
        journal.fase_actual = 0;

        // Step 1: Write Phase 1 files
        std::fs::write(temp_dir.join("cyber_sentinel.html"), "<html></html>").unwrap();
        std::fs::write(temp_dir.join("style.css"), "body{}").unwrap();
        std::fs::write(temp_dir.join("script.js"), "console.log(1);").unwrap();

        let written_files = vec![
            "cyber_sentinel.html".to_string(),
            "style.css".to_string(),
            "script.js".to_string(),
        ];

        // Writing every planned file is progress only. Validation and the phase
        // gate must happen before marking it complete or advancing.
        if let Some(fase) = journal.fases.get_mut(journal.fase_actual) {
            let files_written = fase.archivos.iter().all(|f| {
                written_files
                    .iter()
                    .any(|w: &String| w.contains(f.as_str()))
            });
            if files_written {
                fase.estado = "EN_PROGRESO".to_string();
            }
        }

        assert_eq!(journal.fases[0].estado, "EN_PROGRESO");
        assert_eq!(journal.fase_actual, 0);
        assert_eq!(journal.fases[1].descripcion, "Verification Script");

        // Verify WorldState sees the files immediately
        let world = crate::core::world_state::WorldState::capture(ws)
            .expect("WorldState capture must succeed");
        assert_eq!(world.files.len(), 3);
        assert!(world.files.contains_key("cyber_sentinel.html"));
        assert!(world.files.contains_key("style.css"));
        assert!(world.files.contains_key("script.js"));

        // Verify repo map does not claim empty directory
        let repo_map = crate::core::map::generate_repo_map(std::path::Path::new(ws));
        assert!(repo_map.contains("cyber_sentinel.html"));
        assert!(!repo_map.contains("(directorio vacío)"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
    #[test]
    fn building_and_verifying_is_a_construction_mission() {
        assert_eq!(
            classify_mission(
                "Construye cyber_sentinel.html y crea verify_dashboard.py, ejecútalo y verifica"
            ),
            MissionType::Construction
        );
    }

    #[test]
    fn broad_initial_application_plan_stays_in_planning_mode() {
        let prompt = "vamos a contruir una aplicacion tipo web; por ahora este es el plan inicial";
        assert!(is_initial_plan_request(prompt));
        assert_eq!(classify_mission(prompt), MissionType::Planning);
        assert_eq!(
            classify_mission("Construye la aplicación y crea los archivos ahora"),
            MissionType::Construction
        );
    }

    #[test]
    fn approved_plan_marker_switches_the_original_plan_to_construction() {
        let request = "[MODO IMPLEMENTACION DE PLAN APROBADO] Solicitud original: vamos a construir una aplicación web; por ahora este es el plan inicial. Fase 1: pruebas locales. Fase 2: Firebase.";
        assert!(!is_initial_plan_request(request));
        assert_eq!(classify_mission(request), MissionType::Construction);
    }

    #[test]
    fn planning_mode_only_allows_research_thinking_and_finish() {
        assert!(planning_tool_allowed("TOOL_WEB_SEARCH"));
        assert!(planning_tool_allowed("TOOL_THINK"));
        assert!(planning_tool_allowed("TOOL_FINISH"));
        assert!(!planning_tool_allowed("TOOL_PROGRAMMER"));
        assert!(!planning_tool_allowed("TOOL_TERMINAL"));
    }

    #[test]
    fn initial_plan_requires_phases_deliverables_assumptions_and_open_decisions() {
        let complete = "Fase 1: MVP. Entregables: login y consultas. Fase 2: facturación. Supuestos: Firebase. Decisiones pendientes: país.";
        assert!(initial_plan_response_complete(complete));
        assert!(!initial_plan_response_complete(
            "Plan listo. Fase 1: login."
        ));
        assert!(!initial_plan_response_complete(""));
    }

    #[test]
    fn failed_research_must_be_disclosed_before_plan_can_finish() {
        let request = "Aplicación de consultas y clientes con login y registro, facturación y contabilidad; probar local y luego desplegar en Firebase.";
        let complete = "Fase 1: aplicación local: pruebas locales del login, registro, consultas y clientes. Entregables: MVP funcional. Fase 2: facturación y contabilidad. Entregables: módulos verificables y despliegue en Firebase Hosting. Supuestos: Firebase. Decisiones pendientes: confirmar país antes de requisitos fiscales.";
        assert!(!planning_response_complete(complete, true, request));
        assert!(planning_response_complete(
            &format!(
                "{} No se pudo verificar información actual de Firebase.",
                complete
            ),
            true,
            request
        ));
        assert!(planning_response_complete(complete, false, request));
    }

    #[test]
    fn initial_plan_cannot_omit_requested_modules_or_reverse_local_first_order() {
        let request = "Crear aplicación de consultas con clientes, login, facturación y contabilidad; probar local y luego desplegar en Firebase.";
        let shallow = "Fase 1: Firebase y login. Entregables: acceso. Fase 2: despliegue. Supuestos: conexión. Decisiones pendientes: país.";
        assert!(!planning_response_complete(shallow, false, request));

        let reversed = "Fase 1: despliegue en Firebase Hosting. Entregables: sitio publicado. Fase 2: pruebas locales de login, registro, consultas, clientes, facturación y contabilidad. Supuestos: Firebase. Decisiones pendientes: país.";
        assert!(!planning_response_complete(reversed, false, request));

        let missing = missing_requested_plan_coverage(request, shallow);
        assert!(missing.contains(&"consultas o citas".to_string()));
        assert!(missing.contains(&"contabilidad".to_string()));
    }

    #[test]
    fn fallback_plan_recovers_the_reported_seven_billion_model_loop() {
        let request = "vamos a contruir una aplicacion tipo web probamos local y luego desplegamos en firebase la aplicacion sera de consulta debe tener todas las ociones que tiene un sistema de consulta con un login legante que permita registro y el neogico es de consulta se hacen facuta y en el sistema tenga modulo de contabilidad por ahora este es el plan inicial";
        let fallback = build_fallback_initial_plan(request, false);
        assert!(planning_response_complete(&fallback, false, request));
        assert!(fallback.contains("### Fase 1 — MVP y pruebas locales"));
        assert!(fallback.contains("### Fase 2 — Firebase y despliegue"));
        assert!(fallback.contains("facturas"));

        let fallback_after_search_error = build_fallback_initial_plan(request, true);
        assert!(planning_response_complete(
            &fallback_after_search_error,
            true,
            request
        ));
    }

    #[test]
    fn structured_conversational_plan_is_preserved_for_completion_gate() {
        let value = serde_json::json!({
            "Fase 1": { "Entregables": ["Login"] },
            "Fase 2": { "Entregables": ["Consultas"] },
            "Supuestos": ["Firebase"],
            "Decisiones pendientes": ["País"]
        });
        let response = conversational_response(Some(&value));
        assert!(initial_plan_response_complete(&response));
        assert!(response.contains("Login"));
    }

    #[test]
    fn runtime_routes_forced_programming_without_asking_the_orchestrator() {
        let mut contract =
            crate::core::mission_contract::MissionContract::new("Construir dashboard");
        contract.add_criterion(
            "file",
            "Debe existir index.html",
            crate::core::mission_contract::VerificationMethod::FileExistence(
                "index.html".to_string(),
            ),
            true,
        );
        let decision = deterministic_forced_decision(
            ".",
            "TOOL_PROGRAMMER",
            "Plan completado. Inicia la creación.",
            &contract,
            &[],
        )
        .expect("the contract provides an exact target");
        assert_eq!(decision["herramienta"], "TOOL_PROGRAMMER");
        assert_eq!(
            decision["archivos_a_editar"],
            serde_json::json!(["index.html"])
        );
    }

    #[test]
    fn runtime_routes_official_verifier_without_model_round_trip() {
        let mut contract =
            crate::core::mission_contract::MissionContract::new("Verificar dashboard");
        contract.add_criterion(
            "verify",
            "El verificador debe pasar",
            crate::core::mission_contract::VerificationMethod::SemanticVerification {
                command: "python verify_dashboard.py".to_string(),
            },
            true,
        );
        let decision = deterministic_forced_decision(
            ".",
            "TOOL_TERMINAL",
            "Ejecuta el verificador oficial.",
            &contract,
            &[],
        )
        .expect("the semantic contract provides the exact command");
        assert_eq!(decision["herramienta"], "TOOL_TERMINAL");
        assert_eq!(decision["comando"], "python verify_dashboard.py");
    }

    #[test]
    fn context_filter_runs_on_the_current_prompt_and_unicode_truncation_is_safe() {
        let clean = sanitize_runtime_context(
            "válido\nC:\\old\\scratch\\proxy-stack\\app.py\núltimo estado",
            "C:\\work\\aura sentinel",
        );
        assert!(clean.contains("válido"));
        assert!(clean.contains("último estado"));
        assert!(!clean.contains("proxy-stack"));
        assert_eq!(truncate_chars("áéíóú", 3), "áéí");
        assert_eq!(recent_context("uno🙂dos", 4), "🙂dos");
    }

    #[test]
    fn repair_focus_removes_duplicate_categories_for_small_models() {
        let criteria = vec![
            "Radar animado ausente".to_string(),
            "Radar sin nodos de amenaza".to_string(),
            "Telemetría de paquetes incompleta".to_string(),
            "Diseño glassmorphism ausente".to_string(),
        ];
        let focused = focused_repair_criteria(&criteria, 2);
        assert_eq!(focused.len(), 2);
        assert_eq!(focused[0], "Radar animado ausente");
        assert_eq!(focused[1], "Telemetría de paquetes incompleta");
    }

    #[test]
    fn managed_verifier_is_recognized_from_absolute_state_paths() {
        let known = vec!["C:/workspace/verify_dashboard.py".to_string()];
        assert!(file_is_already_known(&known, "verify_dashboard.py"));
        assert!(!file_is_already_known(&known, "dashboard.html"));
    }
}

fn persist_failed_journal(
    journal: &mut crate::core::session_journal::SessionJournal,
    workspace_path: &str,
    app: &AppHandle,
    step: u32,
    reason: &str,
) {
    journal.ultimo_estado = reason.chars().take(1000).collect();
    if let Err(error) =
        crate::core::session_journal::close_journal(journal, "FALLIDO", workspace_path)
    {
        emit_event(app, step, &error, "FATAL");
    }
    crate::core::episodic_memory::save_failure_episode(
        &journal.session_id,
        workspace_path,
        &journal.objetivo,
        reason,
        &journal.herramientas_usadas,
        &journal.archivos_tocados,
    );
}

async fn record_adaptive_failure(
    engine: &crate::core::learning::LearningEngine,
    fingerprint: &crate::core::learning::fingerprint::TaskFingerprint,
    model: &str,
    strategy: &crate::core::learning::strategy::StrategyKind,
    confidence: f32,
    runtime: &crate::core::mission_runtime::MissionRuntime,
    started_at_ms: u64,
    reason: &str,
) {
    let (class, tool) = adaptive_failure_details(reason);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(started_at_ms);
    let metrics = &runtime.cognitive_state.metrics;
    let result = crate::core::learning::LearningResult {
        outcome: crate::core::learning::LearningOutcome::Failed,
        metrics: crate::core::learning::OutcomeMetrics {
            steps: runtime.steps_taken(),
            tool_calls: metrics.tool_calls,
            failed_actions: metrics.failed_actions,
            recovery_actions: metrics.recovery_actions,
            verification_attempts: metrics.verification_attempts,
            successful_verifications: metrics.successful_verifications,
            elapsed_ms: now_ms.saturating_sub(started_at_ms),
        },
        failures: vec![crate::core::learning::FailureInfo {
            class: class.to_string(),
            tool: tool.map(str::to_string),
            step: runtime.current_step(),
        }],
        recovery: Some(crate::core::learning::RecoveryRecord {
            strategy: "Abort".to_string(),
            succeeded: false,
        }),
    };
    if let Err(error) = engine
        .record_outcome(
            fingerprint.clone(),
            model.to_string(),
            strategy.clone(),
            result,
            confidence,
            runtime.mission_id.clone(),
            Some(format!("{}_{}", runtime.mission_id, class)),
        )
        .await
    {
        eprintln!("[LearningEngine] FAILURE_RECORD_WARN: {error}");
    }
}
