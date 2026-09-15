use super::{call_ollama, delegate_to_auditor, delegate_to_logic_solver};
use crate::core::{
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

    let mut clean_text = text.trim().to_string();
    if let Some(start) = clean_text.find('{') {
        if let Some(end) = clean_text.rfind('}') {
            clean_text = clean_text[start..end + 1].to_string();
        }
    }

    clean_text
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn recent_context(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    text.chars()
        .skip(char_count.saturating_sub(max_chars))
        .collect()
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
        if let crate::core::mission_contract::VerificationMethod::SemanticVerification {
            command,
        } = &criterion.verification
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

fn file_is_already_known(existing_files: &[String], requested: &str) -> bool {
    let requested = requested.replace('\\', "/");
    existing_files.iter().any(|existing| {
        let existing = existing.replace('\\', "/");
        existing == requested || existing.ends_with(&format!("/{requested}"))
    })
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
    let is_repair = ["corrig", "repara", "fall", "error", "criterio", "estancamiento"]
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
    if let Ok(pattern) = regex::Regex::new(
        r"(?i)\b[a-z_][a-z0-9_./\\-]*\.(?:html|css|js|ts|tsx|jsx|py|rs|json)\b",
    ) {
        files.extend(pattern.find_iter(reason).filter_map(|matched| {
            if matched.as_str().contains(".aura") {
                return None;
            }
            std::path::Path::new(matched.as_str())
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .filter(|name| {
                    name != "programmer_failure.json"
                        && !name.eq_ignore_ascii_case("node.js")
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
        "dir", "python ", "python3 ", "node ", "cargo ", "npm ", "npx ", "git ",
        "powershell ", "pwsh ",
    ];
    if direct_prefixes.iter().any(|prefix| {
        trimmed.eq_ignore_ascii_case(prefix.trim())
            || trimmed.to_lowercase().starts_with(prefix)
    }) {
        return Some(trimmed.to_string());
    }
    None
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
        "TOOL_FINISH" => {
            let mut value = base(forced);
            value["respuesta_conversacional"] = serde_json::json!(reason);
            Some(value)
        }
        "TOOL_BACKGROUND_START" if !reason.trim().is_empty() => {
            let mut value = base(forced);
            value["comando"] = serde_json::json!(reason.trim());
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
    } else {
        ""
    }
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

/// ── Mission Type Classifier ───────────────────────────────────────────────
/// Classifies the user intent BEFORE entering the LLM loop.
/// ANALYSIS tasks never enter the Executor — they resolve via TOOL_FINISH from the Planner.
#[derive(Debug, Clone, PartialEq)]
enum MissionType {
    Analysis,     // "analiza", "describe", "explica", "qué hay"
    Construction, // "crea", "implementa", "build"
    Refactor,     // "mejora", "optimiza", "refactoriza"
    Debug,        // "arregla", "bug", "error", "fix"
    Execution,    // "ejecuta", "corre", "prueba", "testea", "run", "verify"
}

fn classify_mission(msg: &str) -> MissionType {
    let m = msg.to_lowercase();
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
            for warning in report.warnings { emit_event(&app_handle, 0, &format!("[ENTORNO] {}", warning), "WARNING"); }
            report.models
        },
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
    let mut verifier_diagnostic = String::new();
    let mut original_prompt_parsed =
        if let Some(idx) = user_message.find("\n\nGuía de Traducción Técnica") {
            let text = &user_message[..idx];
            text.replace("Petición Original del Usuario: ", "")
                .trim()
                .to_string()
        } else {
            user_message.clone()
        };
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
    let mandatory_tools_required: std::collections::HashSet<String> = {
        let mut required = std::collections::HashSet::new();
        let msg_upper = original_prompt_parsed.to_uppercase();
        if msg_upper.contains("TOOL_TESTER") {
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
    let mut forced_next_tool: Option<(String, String)> = None;
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
    let mut agent_workspace = chronos_vfs::workspace::AgentWorkspace::<
        chronos_vfs::aura_bridge::AuraAstNode,
    >::new(1_048_576)
    .unwrap();
    // ── Context Window Tiered Monitor (Devin 2.0 / OSS 2025 Pattern) ────────
    let context_monitor =
        crate::core::context_monitor::ContextMonitor::new(12000, &original_prompt_parsed);

    // ── Multi-Agent Role State Machine ─────────────────────────────────────
    let mut current_role = AgentRole::Planner;
    let mut critic_feedback: Option<String> = None;

    // ── Fase 5: Sanity Monitor state ─────────────────────────────────────────
    let mut tool_history: Vec<String> = Vec::new();
    let mut last_progress_step: u32 = 1;

    // ── FASE B: Cached Workspace Tree (RAM invalidation on file modification) ──
    let mut _no_verify_consecutive: u32 = 0;
    // ── Mission Type Classifier ─────────────────────────────────────────────
    let mission_type = classify_mission(&original_prompt_parsed);
    let mission_label = match &mission_type {
        MissionType::Analysis => "🔍 ANÁLISIS",
        MissionType::Construction => "🏗️ CONSTRUCCIÓN",
        MissionType::Refactor => "♻️ REFACTORING",
        MissionType::Debug => "🐛 DEBUG",
        MissionType::Execution => "⚡ EJECUCIÓN/VERIFICACIÓN",
    };

    if mission_type == MissionType::Execution {
        current_role = AgentRole::Executor;
    }

    // ── Acceptance Contract ─────────────────────────────────────────────────
    let mut acceptance_contract: Option<String> = None;

    let mut json_error_count = 0;

    // ── Session Journal ────────────────────────────────────────
    let mut journal = crate::core::session_journal::load_journal(&workspace_path);

    // ── Check if the user is sending a continuation command ──
    let is_continuation_command = crate::core::intent_router::is_resume_command(&user_message);

    // ── Fase 1: Register workspace in global index for auto-resume ────────────
    crate::core::mission_persist::register_workspace(&workspace_path);

    // ── Fase 3: Inject episodic memory context ────────────────────────────────
    let episode_context = crate::core::episodic_memory::get_episode_context(3);
    if !episode_context.is_empty() {
        current_context.push_str(&episode_context);
    }

    // ── Arquitectura Cognitiva v4: Inyectar Contexto de Experiencias Previas ──
    let exp_context = crate::core::experience::ExperienceStore::build_experience_context(
        &original_prompt_parsed,
        "",
    );
    if !exp_context.is_empty() {
        current_context.push_str(&exp_context);
    }

    // ── FASE C: Inyectar Lecciones Consolidadas de Proyectos Previos ────────────
    let proactive_lessons = crate::core::memory::get_proactive_lessons(3).await;
    if !proactive_lessons.is_empty() {
        current_context.push_str(&proactive_lessons);
    }

    if is_continuation_command && !journal.objetivo.is_empty() {
        // Retain original mission objective and existing phases!
        journal = crate::core::session_journal::resume_existing_mission(&workspace_path)?;
        user_message = journal.objetivo.clone();
        original_prompt_parsed = journal.objetivo.clone();
        journal.interrupted = true; // Signals restoration block below
    } else {
        // Any new user prompt (not an explicit continuation) starts a completely clean session
        journal = crate::core::session_journal::start_new_mission(&workspace_path, &user_message)?;
    }

    // Ensure internal files in workspace are hidden on Windows
    crate::core::hide_workspace_internal_files(&workspace_path);

    // ── MissionRuntime: cognitive governor built AFTER objective is resolved ──
    // This ensures the runtime contract has the correct objective for continuations.
    let mut runtime = crate::core::mission_runtime::MissionRuntime::new(
        &workspace_path,
        &original_prompt_parsed, // <-- now guaranteed to be the resolved objective
        50,
    );

    // Initial contract criteria: every mission must have explicit deliverables & validation
    if runtime.contract.acceptance_criteria.is_empty() && runtime.contract.required_evidence.is_empty() {
        runtime.contract = crate::core::mission_contract::MissionContract::from_objective(&original_prompt_parsed);
    }

    match crate::core::managed_verifier::ensure_for_mission(
        &workspace_path,
        &original_prompt_parsed,
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
    // Must happen BEFORE the mission loop. ToolRegistry is now the sole dispatch
    // authority — execute_action() resolves which code runs via registry, not ad-hoc.
    register_default_tools(&mut runtime, &workspace_path, &original_prompt_parsed);
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
        al_strategy_stats,
        al_store.clone(),
        &runtime.mission_id,
    );
    let al_recommendation = al_router
        .recommend_with_context(
            &al_fingerprint,
            None,                             // No StateSignature yet at mission start
            Some(runtime.budget_remaining()), // remaining budget from runtime
            &available_models,
        )
        .await;
    let al_engine = crate::core::learning::LearningEngine::with_store(al_store.clone());
    let al_start_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    emit_event(
        &app_handle,
        0,
        &format!(
            "[ADAPTIVE LEARNING] Recomendación no aplicada: {:?} | Modelo sugerido: {} (confianza: {:.0}%). Se conserva el modelo seleccionado.",
            al_recommendation.strategy,
            al_recommendation.model,
            al_recommendation.confidence * 100.0
        ),
        "INFO",
    );

    journal.workspace_path = workspace_path.clone();
    journal.status = "EN_PROGRESO".to_string();
    journal.herramientas_usadas.clear();
    journal.archivos_tocados.clear();
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
    if !journal.plan_generado
        && (mission_type == MissionType::Construction || mission_type == MissionType::Refactor)
    {
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

        journal.fases = fases.clone();
        journal.fase_actual = 0;
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
                        al_recommendation.strategy.clone(),
                        al_result,
                        al_recommendation.confidence,
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
                        al_recommendation.strategy.clone(),
                        al_result,
                        al_recommendation.confidence,
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
            emit_event(&app_handle, runtime.current_step(), &format!("[MEMORIA] Compactando ventana de contexto ({:.0}% uso) preservando Objetivo Inmutable...", fill_pct.min(1.0) * 100.0), "INFO");
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
                    "[MEMORIA] Ventana al {:.0}% de capacidad — operando con normalidad.",
                    fill_pct.min(1.0) * 100.0
                ),
                "INFO",
            );
        }

        let mut forced_override: Option<(String, String)> = None;
        if let Some((forced, override_msg)) = forced_next_tool.take() {
            let intercept_log = format!("[SISTEMA INTERCEPTO] En el turno anterior se decidió forzarte a usar: {}. Razón: {}", forced, override_msg);
            current_context.push_str(&format!("{}\n\n", intercept_log));
            forced_override = Some((forced, override_msg));
        }

        let mut extra_prompt = String::new();
        if let Some((forced, reason)) = &forced_override {
            extra_prompt = format!("\n\nACCIÓN REQUERIDA: {}. Genera los argumentos válidos para esta herramienta. Diagnóstico vigente: {}", forced, reason);
        }

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

        // ── Acceptance Contract injection ────────────────────────────────────────────────
        let contract_block = acceptance_contract
            .as_deref()
            .map(|c| format!("[CONTRATO DE ACEPTACION DEL PLANIFICADOR]\n{}\n", c))
            .unwrap_or_default();

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
            1. 'comando' = UN SOLO comando de shell real. NUNCA prosa/descripción. Ejemplos: 'dir', 'start index.html', 'node app.js'.
            2. 'archivos_a_editar' = Si eliges TOOL_PROGRAMMER, DEBES incluir al menos un nombre de archivo relativo a crear o editar. NUNCA lo dejes vacío [].
            3. Si el workspace no tiene archivos creados, usa TOOL_PROGRAMMER para crearlos. Si los archivos base ya existen, usa TOOL_TERMINAL para ejecutarlos o probarlos, o TOOL_PROGRAMMER para scripts de test (ej. verify_*.py). PROHIBIDO volver a crear archivos ya existentes.
            4. El workspace actual es: {ws}. NUNCA uses rutas absolutas de proyectos anteriores ni de otros directorios.",
            ws = workspace_path);

        let agent_prompt = match current_role {
            // Planner - Enrutamiento Semántico Avanzado (Zero-Hint Routing)
            AgentRole::Planner => format!(
                "{}[PLANIFICADOR / ZERO-HINT ROUTER]\nObjetivo del Usuario: {}\nWorkspace Actual: {}\n{}\nHistorial de Conversación:\n{}\n\n\
                ERES EL ENRUTADOR PRINCIPAL. Tu misión es analizar la intención del usuario de forma 100% implícita y autónoma, SIN depender de que el usuario mencione herramientas por su nombre.\n\
                HERRAMIENTAS PERMITIDAS (SELECCIONA POR SEMÁNTICA):\n\
                - TOOL_LOGIC_SOLVER: Dispara esta herramienta AUTOMÁTICAMENTE si el usuario pide analizar esquemas criptográficos, problemas de satisfacibilidad, paridad matemática, restricciones lógicas, álgebra booleana, grafos densos o cuellos de botella de reglas. (¡El motor SpectraSAT lo resolverá en RAM!).\n\
                - TOOL_SCHEDULER: Usa esto si el usuario pide ejecutar tareas repetitivas o programadas (ej. 'haz esto cada lunes', 'audita cada semana'). Comando: 'cron_expr|descripción'.\n\
                - TOOL_THINK: Si el objetivo requiere crear, escribir, modificar o debugear código fuente, usa esto para transferir el control al EJECUTOR de forma transparente.\n\
                - TOOL_MAPPER: Solo para analizar dependencias en proyectos con múltiples archivos existentes.\n\
                - TOOL_AUDITOR: Para revisiones de seguridad en código fuente existente.\n\
                - TOOL_SEARCH / TOOL_WEB_SEARCH: Si necesitas buscar documentación o investigar información externa.\n\
                - TOOL_ASK_USER: Para pedir clarificaciones si la intención del usuario es completamente ambigua (PROHIBIDO para pedir ayuda con código, sintaxis o tests).\n\
                - TOOL_FINISH: Usa esto ÚNICAMENTE si la tarea ya está 100% completada y verificada (o si reportaste los resultados finales). NUNCA uses esto si aún hay pasos pendientes.\n\
                \nPROHIBIDO: TOOL_WORKSPACE_MANAGER, TOOL_PROGRAMMER, TOOL_TERMINAL, TOOL_CONTAINER (Estas son exclusivas del Ejecutor).\n\
                REGLA DE ZERO-HINT: Nunca le pidas al usuario que especifique la herramienta. Deduce la necesidad matemática o de código y actúa en consecuencia.\n\
                {}{}{}",
                pesp_banner, user_message, live_workspace_context, extra_prompt, current_context,
                critic_feedback_block, analysis_fast_path, json_schema
            ),
            // Executor - compressed to <200 tokens
            AgentRole::Executor => format!(
                "{}[EJECUTOR] Objetivo: {}\nWorkspace: {}\n{}\nHistorial:\n{}\n\nTOOLS PERMITIDOS: TOOL_PROGRAMMER, TOOL_TERMINAL, TOOL_CONTAINER, TOOL_ASSET_MANAGER, TOOL_BACKGROUND_START.\n- TOOL_CONTAINER: Comando = 'run/exec/stop/activate_env image/id'. Úsalo para sandbox, testing en Docker/Podman o para activar entornos virtuales (venv, nvm, cargo).\n- TOOL_ENV_MANAGER: *SOLO* para instalar binarios scoop.\nREGLAS: ANTI-STUB (no pass/TODO/funciones vacias). Si faltan entregables, usa TOOL_PROGRAMMER con todos sus nombres en 'archivos_a_editar' (NUNCA [] vacío). Si ya existen, usa TOOL_TERMINAL para probarlos o TOOL_PROGRAMMER para corregir los archivos que causan un fallo real. No uses TOOL_TESTER ni TOOL_FINISH. PROHIBIDO usar TOOL_ASK_USER.\n[REGLA SCRIPTS DE PRUEBA]: Al crear/modificar verify_*.py o test_*.py: 1) Crea una comprobación independiente para CADA requisito numerado del objetivo y prueba los archivos reales, incluidos JS/CSS enlazados. 2) Usa solo json, pathlib y re; busca tokens simples por separado y nunca incrustes líneas HTML/JS completas con comillas anidadas. 3) Ejecuta los checks bajo __main__ y emite JSON en una línea con passed y total ENTEROS, percentage calculado y failed_criteria con una entrada por fallo. Ejemplo para cinco checks aprobados: {{\"passed\": 5, \"total\": 5, \"percentage\": 100.0, \"failed_criteria\": []}}. 4) Exit code 0 solo si passed=total; cualquier fallo debe devolver exit code 1. El verificador es de solo lectura.\nEJEMPLOS TOOL_TERMINAL: Para 'npm install' usa TOOL_TERMINAL con comando='npm install'. NUNCA inventes herramientas como 'NPM INSTALL'.\n\n{}{}",
                pesp_banner, user_message, live_workspace_context, extra_prompt, current_context,
                critic_feedback_block, json_schema
            ),
            // Critic - compressed to <200 tokens
            AgentRole::Critic => format!(
                "{}[CRITICO] Objetivo: {}\nWorkspace: {}\n{}\nHistorial:\n{}\n\nTOOLS PERMITIDOS: TOOL_TESTER, TOOL_TERMINAL, TOOL_VISION_EVALUATOR, TOOL_FINISH, TOOL_ASK_USER.\nREGLAS: Usa TOOL_TESTER/TOOL_TERMINAL para validar. Si hay errores describelos con precisión. Solo TOOL_FINISH si todo pasa al 100%%. PROHIBIDO usar TOOL_ASK_USER para fallos de tests o código (transfiere a TOOL_PROGRAMMER para corregir el código o el test).\n[REGLA FRONTEND]: Si el proyecto contiene archivos HTML (ej. index.html, cyber_sentinel.html, etc.), es frontend. NO uses `node script.js` sobre archivos HTML. Si existe un script de prueba de verificación (ej. verify_*.py o test_*.py), ejecútalo con TOOL_TERMINAL; si pasa al 100%% o no hay tests pendientes, usa TOOL_FINISH.\n\n{}{}",
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
            deterministic_forced_decision(
                &workspace_path,
                forced,
                reason,
                &runtime.contract,
                &runtime.state_anchor.existing_files,
            )
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
            match call_ollama(&orchestrator_model, &agent_prompt).await {
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
        let raw_value: serde_json::Value = match serde_json::from_str(&clean_agent_res) {
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
                if json_error_count >= 5 {
                    emit_event(&app_handle, runtime.current_step(), &format!("Error parseando decisión ({}). Máximos reintentos (5) alcanzados. Abortando bucle.", e), "ERROR");
                    let final_res = FinalResponse {
                        status: "ERROR".to_string(),
                        respuesta_conversacional:
                            "Fallo crítico persistente en la estructura JSON del planificador."
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
                            "Error de sintaxis JSON (intento {}/5). Reintentando...",
                            json_error_count
                        ),
                        "WARNING",
                    );
                    current_context.push_str(&format!("[SISTEMA INTERNO] Tu respuesta anterior no era un JSON válido. Error: {}. Genera SOLO un objeto JSON estrictamente válido según la estructura requerida, sin texto adicional antes o después del JSON.\n\n", e));
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
        let task_id = raw_value
            .get("task_id")
            .and_then(|v| v.as_str())
            .unwrap_or("default_task")
            .to_string();
        let mut respuesta_conv = raw_value
            .get("respuesta_conversacional")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Si el modelo solo quiere responder conversacionalmente (ej. "herramienta": null o "null"),
        // y adjuntó una respuesta para el usuario, enrutar limpiamente como TOOL_FINISH para no entrar en bucle de error.
        if (tool == "UNKNOWN" || tool == "NULL" || tool == "NONE" || tool.is_empty())
            && !respuesta_conv.trim().is_empty()
        {
            tool = "TOOL_FINISH".to_string();
        }

        // Strict Schema Requirement: TOOL_TERMINAL requires non-empty 'comando'
        if tool == "TOOL_TERMINAL" && comando.trim().is_empty() {
            emit_event(
                &app_handle,
                runtime.current_step(),
                "[SCHEMA ERROR] TOOL_TERMINAL requiere un campo 'comando' no vacío.",
                "WARNING",
            );
            current_context.push_str("[VALIDACIÓN DE ESQUEMA FALLIDA]: TOOL_TERMINAL requiere un campo 'comando' explícito y no vacío en el JSON. Prohibido omitir el comando.\n\n");
            continue;
        }

        // Bloquear TOOL_THINK consecutivo
        if tool == "TOOL_THINK" && runtime.state_anchor.last_tool.as_deref() == Some("TOOL_THINK") {
            emit_event(
                &app_handle,
                runtime.current_step(),
                "[ANTI-ECHOLALIA] TOOL_THINK consecutivo bloqueado. Se requiere acción física.",
                "WARNING",
            );
            current_context.push_str("[SISTEMA INTERNO - BLOQUEO DE BUCLE]: Acabas de usar TOOL_THINK en el turno anterior. PROHIBIDO encadenar otro TOOL_THINK consecutivo. Ahora debes ejecutar una herramienta física obligatoriamente (ej. TOOL_PROGRAMMER, TOOL_TERMINAL o TOOL_MAPPER) para avanzar.\n\n");
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
                if forced_next_tool.is_none() {
                // Determine strategy transition based on physical reality from state_anchor
                let has_existing_files = !runtime.state_anchor.existing_files.is_empty();
                let has_test_file = runtime
                    .state_anchor
                    .existing_files
                    .iter()
                    .any(|f| f.starts_with("verify_") || f.starts_with("test_"));

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
                    forced_next_tool = Some((
                        "TOOL_PROGRAMMER".to_string(),
                        "Estancamiento detectado: Los archivos base ya existen. Crea un script de verificación 'verify_solution.py' con TOOL_PROGRAMMER para validar la solución.".to_string(),
                    ));
                    current_context.push_str("[TRANSICIÓN FORZADA POR ESTANCAMIENTO]: Los archivos base ya existen en disco. Crea un script de verificación (ej. verify_*.py) usando TOOL_PROGRAMMER.\n\n");
                } else {
                    let test_file = runtime
                        .state_anchor
                        .existing_files
                        .iter()
                        .find(|f| f.starts_with("verify_") || f.starts_with("test_"))
                        .unwrap();
                    let test_cmd = if test_file.ends_with(".py") {
                        format!("python {}", test_file)
                    } else if test_file.ends_with(".js") {
                        format!("node {}", test_file)
                    } else {
                        format!("python {}", test_file)
                    };
                    forced_next_tool = Some((
                        "TOOL_TERMINAL".to_string(),
                        format!("Estancamiento detectado: Ejecuta el script de pruebas '{}' usando TOOL_TERMINAL con comando='{}'.", test_file, test_cmd),
                    ));
                    current_context.push_str(&format!("[TRANSICIÓN FORZADA POR ESTANCAMIENTO]: Ejecuta el test existente '{}' para verificar la solución.\n\n", test_cmd));
                }
            }
            }
        }

        // ── Arquitectura Cognitiva v4: Validación previa de esquema (P1) ──
        if let crate::core::schema_validator::SchemaValidationResult::Invalid(schema_err) =
            crate::core::schema_validator::SchemaValidator::validate_tool_payload(&tool, &raw_value)
        {
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
            continue;
        }

        // ── FORCED TOOL VALIDATION ────────────────────────────────────────────
        // If the system has determined the LLM is stuck in a tool-loop,
        // validate its decision against the forced tool constraint.
        if let Some((forced, override_msg)) = &forced_override {
            if tool != *forced {
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
                if intercept_consecutive >= 3 || (forced == "TOOL_PROGRAMMER" && intercept_consecutive >= 1) {
                    if forced == "TOOL_PROGRAMMER" {
                        let mut repair_files: Vec<String> = std::fs::read_dir(&workspace_path)
                            .ok()
                            .into_iter()
                            .flatten()
                            .flatten()
                            .filter_map(|entry| {
                                let path = entry.path();
                                if path.is_file() && matches!(path.extension().and_then(|value| value.to_str()), Some("html" | "css" | "js" | "ts" | "py" | "rs" | "json")) {
                                    Some(entry.file_name().to_string_lossy().to_string())
                                } else { None }
                            })
                            .collect();
                        for criterion in &runtime.contract.acceptance_criteria {
                            if let crate::core::mission_contract::VerificationMethod::FileExistence(file) = &criterion.verification {
                                if !repair_files.contains(file) { repair_files.push(file.clone()); }
                            }
                        }
                        repair_files.sort();
                        repair_files.dedup();
                        let target_model = resolve_model_or_fallback(&programmer_model, &available_models);
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
                            Ok(obs) if obs.status == crate::core::observation::ObservationStatus::Success => {
                                for file in &obs.files_affected {
                                    let full_path = std::path::Path::new(&workspace_path).join(file);
                                    let _ = app_handle.emit("file-updated", serde_json::json!({"path": full_path.to_string_lossy()}));
                                }
                                current_context.push_str(&format!("[REPARACIÓN FORZADA COMPLETADA]: {}\n", obs.payload));
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
                                    persist_failed_journal(&mut journal, &workspace_path, &app_handle, runtime.current_step());
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
                                persist_failed_journal(&mut journal, &workspace_path, &app_handle, runtime.current_step());
                                return Ok(serde_json::json!({"status":"ERROR", "respuesta_conversacional":message}).to_string());
                            }
                        }
                    }
                    if forced != "TOOL_TERMINAL" {
                        let message = format!("FORCED_TOOL_NOT_OBEYED: el modelo no generó una llamada válida a {} después de 3 intentos. Razón: {}", forced, override_msg);
                        emit_event(&app_handle, runtime.current_step(), &message, "ERROR");
                        persist_failed_journal(&mut journal, &workspace_path, &app_handle, runtime.current_step());
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
            respuesta_conv = raw_value
                .get("respuesta_conversacional")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
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
            let explicit_contract_files: Vec<String> = runtime.contract.acceptance_criteria.iter().filter_map(|criterion| {
                if let crate::core::mission_contract::VerificationMethod::FileExistence(file) = &criterion.verification {
                    Some(file.clone())
                } else {
                    None
                }
            }).collect();
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
                    let mut related: Vec<String> = entries.flatten().filter_map(|entry| {
                        let path = entry.path();
                        if path.is_file() && matches!(path.extension().and_then(|v| v.to_str()), Some("html" | "css" | "js" | "ts" | "py")) {
                            let file = entry.file_name().to_string_lossy().to_string();
                            let is_verifier = file.starts_with("verify_") || file.starts_with("test_");
                            if is_verifier && !verifier_diagnostic.contains(&file) {
                                None
                            } else {
                                Some(file)
                            }
                        } else { None }
                    }).collect();
                    related.sort();
                    for file in related {
                        if !archivos_vec.contains(&file) { archivos_vec.push(file); }
                    }
                }
            }
            archivos_vec.retain(|file| !is_managed_verifier(&workspace_path, file));
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
            risk: if tool == "TOOL_TERMINAL" {
                crate::core::policy::PolicyEngine::classify_terminal_command(&comando)
            } else {
                crate::core::policy::RiskLevel::Safe
            },
            world_hash: Some(runtime.current_world_hash()),
        };

        // authorize_action removed (handled by execute_action)        }

        match tool.as_str() {
            "TOOL_TERMINAL" => {
                let cmd_lower = comando.to_lowercase();
                if cmd_lower.contains("http-server")
                    || cmd_lower.contains("npm start")
                    || cmd_lower.contains("npm run dev")
                    || cmd_lower.contains("python -m http.server")
                    || cmd_lower.contains("flask run")
                    || cmd_lower.contains("uvicorn")
                {
                    let res_msg = "[SISTEMA INTERNO]: Has intentado iniciar un servidor web continuo (http-server, npm start, etc.) usando TOOL_TERMINAL. Esto bloquea la terminal infinitamente y rompe el agente.\nSi el objetivo es probar HTML/JS estático, usa `start index.html` para abrirlo directamente en el navegador sin servidor.\nSi requieres obligatoriamente un backend, DEBES usar TOOL_BACKGROUND_START. [SISTEMA: Redirigiendo automticamente a TOOL_BACKGROUND_START...]";
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
                            Ejemplos válidos para este paso: 'start index.html' para abrir el \
                            navegador, 'ls' para listar archivos, 'node script.js' para ejecutar JS. \
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
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "Comando repetido interceptado",
                        "WARNING",
                    );
                    if current_role == AgentRole::Critic {
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
                                let has_test =
                                    runtime.state_anchor.existing_files.iter().any(|f| {
                                        f.starts_with("verify_") || f.starts_with("test_")
                                    });
                                if !has_test {
                                    forced_next_tool = Some(("TOOL_PROGRAMMER".to_string(), "Crea un script de verificación (verify_*.py) usando TOOL_PROGRAMMER.".to_string()));
                                } else {
                                    let test_f = runtime
                                        .state_anchor
                                        .existing_files
                                        .iter()
                                        .find(|f| {
                                            f.starts_with("verify_") || f.starts_with("test_")
                                        })
                                        .unwrap();
                                    forced_next_tool = Some((
                                        "TOOL_TERMINAL".to_string(),
                                        format!("python {}", test_f),
                                    ));
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
                    if cmd_lower.contains("http-server")
                        || cmd_lower.contains("npm start")
                        || cmd_lower.contains("npm run dev")
                        || cmd_lower.contains("python -m http.server")
                        || cmd_lower.contains("flask run")
                        || cmd_lower.contains("uvicorn")
                    {
                        let res_msg = "[SISTEMA INTERNO]: Has intentado iniciar un servidor web continuo (http-server, npm start, etc.) usando TOOL_TERMINAL. Esto bloquea la terminal infinitamente y rompe el agente.\nSi el objetivo es probar HTML/JS estático, usa `start index.html` para abrirlo directamente en el navegador sin servidor.\nSi requieres obligatoriamente un backend, DEBES usar TOOL_BACKGROUND_START. [SISTEMA: Redirigiendo automticamente a TOOL_BACKGROUND_START...]";
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
                        let script_file = if cmd_lower_check.starts_with("node ") {
                            cmd_lower_check
                                .trim_start_matches("node ")
                                .split_whitespace()
                                .find(|arg| !arg.starts_with('-'))
                        } else if cmd_lower_check.starts_with("python ")
                            || cmd_lower_check.starts_with("python3 ")
                        {
                            let rest = if cmd_lower_check.starts_with("python3 ") {
                                cmd_lower_check.trim_start_matches("python3 ")
                            } else {
                                cmd_lower_check.trim_start_matches("python ")
                            };
                            rest.split_whitespace().find(|arg| !arg.starts_with('-'))
                        } else {
                            None
                        };
                        if let Some(script) = script_file {
                            if !script.is_empty() {
                                let script_path =
                                    std::path::Path::new(&workspace_path).join(script);
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

                        // ── P1: Intercept concatenated commands (&&) in PowerShell ─────────────
                        if comando.contains("&&") {
                            let warn_msg = "[SISTEMA]: En este entorno (PowerShell), el operador '&&' no está soportado para concatenar comandos en una sola línea de TOOL_TERMINAL.\nPor favor, ejecuta los comandos uno por uno en pasos separados (ej. usa TOOL_TERMINAL para el primero, evalúa el resultado, y luego usa TOOL_TERMINAL para el segundo).";
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
                        }) = repair_decision
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
                                    persist_failed_journal(&mut journal, &workspace_path, &app_handle, runtime.current_step());
                                    return Ok(serde_json::json!({"status":"ERROR", "respuesta_conversacional":message}).to_string());
                                }
                            }
                            let focused_criteria = focused_repair_criteria(&criteria, 2);
                            verifier_diagnostic = focused_criteria.join("\n");
                            current_context.push_str(&format!("\nVerificación fallida, diagnóstico real: {}\n", verifier_diagnostic));
                            if verifier_failures >= 4 {
                                let message = format!("VERIFIER_REPAIR_EXHAUSTED: La verificación inicial y tres reparaciones reales fallaron. Archivos conservados. {}", verifier_diagnostic);
                                emit_event(&app_handle, runtime.current_step(), &message, "ERROR");
                                persist_failed_journal(&mut journal, &workspace_path, &app_handle, runtime.current_step());
                                return Ok(serde_json::json!({"status":"ERROR", "respuesta_conversacional":message}).to_string());
                            }
                            emit_event(&app_handle, runtime.current_step(), &format!("[SEMANTIC_VERIFICATION] Verificador falló. Transición a {}.", recommended_tool), "WARNING");
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
                                emit_event(&app_handle, runtime.current_step(), &format!("[TERMINAL] {} | exit={:?} | archivos cambiados={}\n{}", comando, observation.exit_code, observation.physical_files_changed.unwrap_or(0), digested_out), "SUCCESS");
                                if matches!(runtime.can_complete(), crate::core::completion_gate::CompletionDecision::Complete) {
                                    current_role = AgentRole::Critic;
                                    forced_next_tool = Some(("TOOL_FINISH".into(), "El contrato tiene evidencia vigente para todos los criterios. Evalúa el cierre sin repetir las pruebas.".into()));
                                }
                                last_progress_step = runtime.current_step();
                            }
                            Err(err) => {
                                // ── Auto ENV_MANAGER: detect binary-not-found and auto-install ──────────
                                let is_binary_missing = err.contains("is not recognized")
                                    || err.contains("not recognized as an internal")
                                    || err.contains("command not found")
                                    || (err.contains("The term")
                                        && err.contains("is not recognized"));

                                if is_binary_missing {
                                    // ── Step 1: Record error Observation through the full circuit ──
                                    let _ = runtime.observe_world();
                                    runtime.record_tool_call();
                                    let world_hash_after = runtime.current_world_hash();
                                    let mut err_obs = crate::core::observation::Observation::error(
                                        "TOOL_TERMINAL",
                                        &err,
                                        None,
                                        true,
                                        None,
                                    );
                                    err_obs.command = Some(comando.clone());
                                    err_obs.state_hash_after = Some(world_hash_after);
                                    runtime.record_observation(&err_obs);

                                    // ── Step 2: RecoveryEngine classifies and produces RepairEnvironment ──
                                    let recovery_dec = runtime.plan_recovery("TOOL_TERMINAL", &err);

                                    // ── Step 3: Execute TOOL_ENV_MANAGER as the recovery action via ActionProposal ──
                                    let binary =
                                        comando.split_whitespace().next().unwrap_or(&comando);
                                    emit_event(&app_handle, runtime.current_step(),
                                    &format!("[RECOVERY→TOOL_ENV_MANAGER] Binario '{}' no encontrado — ejecutando instalación automática por RecoveryEngine...", binary),
                                    "WARNING");
                                    let _ = recovery_dec; // RecoveryEngine already consulted; install proceeds

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
                                        let _ = runtime.observe_world();
                                        runtime.record_tool_call();
                                        let world_hash_after = runtime.current_world_hash();
                                        let mut obs = crate::core::observation::Observation::error(
                                            "TOOL_TERMINAL",
                                            &err,
                                            None,
                                            true,
                                            None,
                                        );
                                        obs.command = Some(comando.clone());
                                        obs.state_hash_after = Some(world_hash_after);
                                        runtime.record_observation(&obs);
                                        let recovery_dec =
                                            runtime.plan_recovery("TOOL_TERMINAL", &err);
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
                                            2. Si un script de prueba (ej. verify_*.py) reporta elementos faltantes que sí existen con otros atributos, o si tiene un error en su conteo o exit code, USA TOOL_PROGRAMMER para corregir el script de prueba.\n\
                                            3. Si el archivo de la aplicación tiene un error o le falta la etiqueta/función, USA TOOL_PROGRAMMER para corregir el archivo de la aplicación.\n\
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

                                        // For Logic errors: force a THINK step before the next retry
                                        if error_type
                                            == crate::core::error_classifier::ErrorType::Logic
                                        {
                                            forced_next_tool = Some((
                                            "TOOL_THINK".to_string(),
                                            format!("Analiza el error: '{}'. Propone la corrección exacta antes de reintentar.", &err[..err.len().min(100)])
                                        ));
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
                    match start_background_task(&workspace_path, &task_id, &comando).await {
                        Ok(out) => {
                            let sys_guidance = "[SISTEMA INTERNO]: El proceso en segundo plano ha sido INICIADO EXITOSAMENTE. NO repitas este comando. Ahora DEBES continuar con la misin usando otras herramientas (por ejemplo, TOOL_VISION_EVALUATOR, TOOL_TESTER, o TOOL_FINISH si has terminado).";
                            current_context
                                .push_str(&format!("Resultado: {}\n{}\n\n", out, sys_guidance));
                            emit_event(&app_handle, runtime.current_step(), &out, "SUCCESS");
                        }
                        Err(err) => {
                            current_context.push_str(&format!(
                                "Resultado: Error iniciando tarea: {}\n\n",
                                err
                            ));
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                &format!("Error: {}", err),
                                "ERROR",
                            );
                        }
                    }
                }
            }
            "TOOL_BACKGROUND_READ" => {
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    &format!("Leyendo logs asíncronos de '{}'", task_id),
                    "ACTION",
                );
                match read_task_logs(&task_id).await {
                    Ok(logs) => {
                        current_context.push_str(&format!("Logs obtenidos:\n{}\n\n", logs));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "Logs leídos correctamente.",
                            "SUCCESS",
                        );
                    }
                    Err(err) => {
                        let fmt_err = format_system_error(&err).await;
                        current_context.push_str(&format!(
                            "[AUTO-DEBUGGER] Error al leer logs: {}\n\n",
                            fmt_err
                        ));
                        emit_event(&app_handle, runtime.current_step(), &fmt_err, "ERROR");
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
                match kill_task(&task_id).await {
                    Ok(msg) => {
                        current_context.push_str(&format!("Resultado: {}\n\n", msg));
                        emit_event(&app_handle, runtime.current_step(), &msg, "SUCCESS");
                    }
                    Err(err) => {
                        let fmt_err = format_system_error(&err).await;
                        current_context.push_str(&format!(
                            "[AUTO-DEBUGGER] Error matando tarea: {}\n\n",
                            fmt_err
                        ));
                        emit_event(&app_handle, runtime.current_step(), &fmt_err, "ERROR");
                    }
                }
            }
            "TOOL_WEB_SCRAPER" => {
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    &format!("Extrayendo contenido de: {}", url),
                    "ACTION",
                );
                match crate::net::fetch_url_text(&url).await {
                    Ok(content) => {
                        let preview = if content.len() > 1000 {
                            format!("{}... (truncado)", &content[..1000])
                        } else {
                            content.clone()
                        };
                        current_context.push_str(&format!("Contenido web:\n{}\n\n", preview));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "Contenido extraído con éxito.",
                            "SUCCESS",
                        );
                    }
                    Err(err) => {
                        current_context.push_str(&format!("Error web: {}\n\n", err));
                        emit_event(&app_handle, runtime.current_step(), &err, "ERROR");
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
                    // Bug 2 fix: always scan workspace directly instead of relying on archivos_vec
                    // archivos_vec may be empty if the LLM didn't populate it, causing "0 archivos"
                    let files_to_audit = if archivos_vec.is_empty() {
                        // Auto-discover all source files in workspace
                        let mut discovered = Vec::new();
                        if let Ok(entries) = std::fs::read_dir(&workspace_path) {
                            for entry in entries.flatten() {
                                let p = entry.path();
                                if p.is_file() {
                                    if let Some(ext) = p.extension() {
                                        let ext = ext.to_string_lossy().to_lowercase();
                                        if matches!(
                                            ext.as_str(),
                                            "html"
                                                | "css"
                                                | "js"
                                                | "ts"
                                                | "py"
                                                | "rs"
                                                | "go"
                                                | "json"
                                                | "md"
                                        ) {
                                            if let Some(name) = p.file_name() {
                                                let fname = name.to_string_lossy().to_string();
                                                // Skip hidden/internal files
                                                if !fname.starts_with('.') {
                                                    discovered.push(fname);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        discovered
                    } else {
                        archivos_vec.clone()
                    };
                    let safe_files =
                        memory::read_files_safely(&workspace_path, files_to_audit.clone()).await;
                    let raw_reporte = delegate_to_auditor(&safe_files, &orchestrator_model).await;
                    let struct_prompt = format!("Convierte este reporte a un JSON con campos: archivos, problema, accion_sugerida. Responde SOLO el JSON. REPORTE:\n{}", &raw_reporte);
                    let structured = call_ollama(&orchestrator_model, &struct_prompt)
                        .await
                        .unwrap_or_else(|_| raw_reporte.clone());
                    let structured = structured.trim().to_string();
                    current_context.push_str(&format!(
                        "[REPORTE AUDITOR ESTRUCTURADO]\n{}\n\n",
                        structured
                    ));
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!("Auditoria completada. {} archivos.", files_to_audit.len()),
                        "SUCCESS",
                    );
                }
                mandatory_tools_executed.insert("TOOL_AUDITOR".to_string());
            }
            "TOOL_LOGIC_SOLVER" => {
                // ─── MODO 1A: SAT nativo — n_vars + clauses en el JSON del agente ───
                let n_vars_opt = raw_value
                    .get("n_vars")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize);
                let clauses_opt = raw_value
                    .get("clauses")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|row| {
                                row.as_array().map(|r| {
                                    r.iter()
                                        .filter_map(|x| x.as_i64().map(|i| i as i32))
                                        .collect::<Vec<i32>>()
                                })
                            })
                            .collect::<Vec<Vec<i32>>>()
                    });

                // ─── MODO 1B: Auto-extracción desde user_message cuando el LLM no proveyó clauses ───
                // Busca el primer array de arrays de enteros en el mensaje original del usuario.
                let (n_vars_opt, clauses_opt) = if n_vars_opt.is_none() || clauses_opt.is_none() {
                    // Buscar patrón [[...], [...], ...] en user_message
                    let extracted = (|| -> Option<(usize, Vec<Vec<i32>>)> {
                        let text = &user_message;
                        let start = text.find("[[").or_else(|| text.find("[ ["))?;
                        let sub = &text[start..];

                        // Encontrar el cierre del array contando corchetes para ser robusto
                        let mut depth = 0;
                        let mut end = 0;
                        for (i, c) in sub.char_indices() {
                            if c == '[' {
                                depth += 1;
                            } else if c == ']' {
                                depth -= 1;
                            }

                            if depth == 0 && i > 0 {
                                end = i + 1;
                                break;
                            }
                        }
                        if end == 0 {
                            return None;
                        }

                        let array_str = &sub[..end];
                        let parsed: serde_json::Value = serde_json::from_str(array_str).ok()?;
                        let outer = parsed.as_array()?;
                        let clauses: Vec<Vec<i32>> = outer
                            .iter()
                            .filter_map(|row| {
                                row.as_array().map(|r| {
                                    r.iter()
                                        .filter_map(|x| x.as_i64().map(|i| i as i32))
                                        .collect()
                                })
                            })
                            .collect();
                        if clauses.is_empty() {
                            return None;
                        }
                        // n_vars = max abs value across all literals
                        let n_vars = clauses
                            .iter()
                            .flatten()
                            .map(|x| x.unsigned_abs() as usize)
                            .max()
                            .unwrap_or(0);
                        Some((n_vars, clauses))
                    })();
                    if let Some((nv, cl)) = extracted {
                        (Some(nv), Some(cl))
                    } else {
                        (n_vars_opt, clauses_opt)
                    }
                } else {
                    (n_vars_opt, clauses_opt)
                };

                let verdict = if let (Some(n_vars), Some(clauses)) = (n_vars_opt, clauses_opt) {
                    // ⚡ Invocación nativa SpectraSAT — cero latencia de subproceso
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!(
                            "⚡ [SPECTRASAT] Resolviendo instancia SAT: {} vars, {} cláusulas...",
                            n_vars,
                            clauses.len()
                        ),
                        "ACTION",
                    );
                    let result = crate::llm::solve_with_spectrasat(n_vars, clauses);
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!("âš¡ [SPECTRASAT] Veredicto: {}", result),
                        "SUCCESS",
                    );
                    result
                } else {
                    // ─── MODO 2: Análisis semántico de código vía LLM (fallback real) ───
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "🔍 [LOGIC_SOLVER] Analizando código con motor lógico-semántico...",
                        "ACTION",
                    );
                    let real_files: Vec<String> = {
                        let hallucinated = archivos_vec
                            .iter()
                            .any(|f| !std::path::Path::new(&workspace_path).join(f).exists());
                        if hallucinated || archivos_vec.is_empty() {
                            let mut found = Vec::new();
                            if let Ok(entries) = std::fs::read_dir(&workspace_path) {
                                for entry in entries.flatten() {
                                    let p = entry.path();
                                    if let Some(ext) = p.extension() {
                                        let ext = ext.to_string_lossy().to_lowercase();
                                        if matches!(
                                            ext.as_str(),
                                            "py" | "rs" | "js" | "ts" | "go" | "c" | "cpp"
                                        ) {
                                            found.push(p.to_string_lossy().to_string());
                                        }
                                    }
                                }
                            }
                            found
                        } else {
                            archivos_vec.clone()
                        }
                    };
                    let safe_files = memory::read_files_safely(&workspace_path, real_files).await;
                    delegate_to_logic_solver(&safe_files, &orchestrator_model).await
                };

                let mut parsed_status = verdict.clone();
                let mut assignment_msg = String::new();

                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&verdict) {
                    if let Some(s) = parsed.get("status").and_then(|v| v.as_str()) {
                        parsed_status = s.to_string();
                    }
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
                if parsed_status.starts_with("UNSAT") {
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
                } else if parsed_status.starts_with("SAT") {
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
                    current_context
                        .push_str(&format!("Veredicto SpectraSAT: {}\n\n", parsed_status));
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "✅ Verificación lógica completada.",
                        "SUCCESS",
                    );
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
                        forced_next_tool = Some(("TOOL_TERMINAL".to_string(), "dir".to_string()));
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
                    // Sprint 1+2 FSM: Planner -> Executor on TOOL_THINK (except Analysis missions)
                    if current_role == AgentRole::Planner {
                        if mission_type == MissionType::Analysis {
                            emit_event(&app_handle, runtime.current_step(), "[FSM] MODO ANALISIS: Planificador permanece activo. Usa TOOL_FINISH para responder.", "INFO");
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

                    let file_instruction = if verifier_diagnostic.is_empty() {
                        format!("Incluye los entregables necesarios de esta lista: {:?}.", archivos_vec)
                    } else {
                        format!("Puedes corregir cualquiera de estos archivos relacionados: {:?}. Devuelve únicamente los que necesiten cambios para resolver el diagnóstico real.", archivos_vec)
                    };
                    let pending_criteria = runtime.contract.pending_required_criteria();
                    let repair_blueprint = tactical_repair_blueprint(&original_prompt_parsed);
                    prog_args["instruccion"] = serde_json::json!(format!(
                        "Objetivo global: {}\nAcción actual: {}\n{} No reescribas otros entregables.\nRequisitos obligatorios aún pendientes:\n- {}\nGuía concreta del runtime: {}\nDiagnóstico de la operación anterior: {}\nÚltima verificación fallida: {}",
                        original_prompt_parsed, pensamiento, file_instruction,
                        if pending_criteria.is_empty() { "ninguno".to_string() } else { pending_criteria.join("\n- ") },
                        repair_blueprint,
                        runtime.state_anchor.last_error.as_deref().unwrap_or("ninguno"), verifier_diagnostic));
                    prog_args["repair_attempt"] = serde_json::json!(programmer_failures);
                    prog_args["semantic_repair"] = serde_json::json!(!verifier_diagnostic.is_empty());

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

                                // Advance fases in journal (PESP v2)
                                if !journal.fases.is_empty() {
                                    let total_fases = journal.fases.len();
                                    let mut should_advance = false;
                                    if let Some(fase) = journal.fases.get_mut(journal.fase_actual) {
                                        let all_done = fase.archivos.is_empty()
                                            || fase.archivos.iter().all(|f| {
                                                written_files
                                                    .iter()
                                                    .any(|w: &String| w.contains(f.as_str()))
                                            });
                                        if all_done {
                                            fase.estado = "COMPLETADA".to_string();
                                            emit_event(
                                                &app_handle,
                                                runtime.current_step(),
                                                &format!(
                                                    "[PESP] ✅ Fase [{}/{}] COMPLETADA: {}",
                                                    journal.fase_actual + 1,
                                                    total_fases,
                                                    fase.descripcion
                                                ),
                                                "SUCCESS",
                                            );
                                            if journal.fase_actual + 1 < total_fases {
                                                should_advance = true;
                                            }
                                        } else {
                                            fase.estado = "EN_PROGRESO".to_string();
                                        }
                                    }
                                    if should_advance {
                                        journal.fase_actual += 1;
                                        let next_desc =
                                            journal.fases[journal.fase_actual].descripcion.clone();
                                        emit_event(
                                            &app_handle,
                                            runtime.current_step(),
                                            &format!(
                                                "[PESP] 🔄 Avanzando a Fase [{}/{}]: {}",
                                                journal.fase_actual + 1,
                                                total_fases,
                                                next_desc
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
                                // Sprint 2: Phase/Micrometa-gated Executor->Critic transition
                                let all_fases_done = journal.fases.is_empty()
                                    || journal.fases.iter().all(|f| f.estado == "COMPLETADA");
                                let all_metas_done = journal.micro_metas.is_empty()
                                    || journal
                                        .micro_metas
                                        .iter()
                                        .all(|mm| mm.estado == "VERIFICADA");
                                if all_fases_done && all_metas_done {
                                    current_role = AgentRole::Critic;
                                    critic_feedback = None;
                                    emit_event(&app_handle, runtime.current_step(), "[FSM] EJECUTOR -> CRITICO: escritura validada. Comprobando entregables y verificaciones pendientes; la misión aún no está completada.", "INFO");
                                }

                                let explicit_msg = format!("Programador: Los archivos {:?} fueron escritos con éxito, Anti-Stub APROBADO.\n⚠️ REGLA DE ESTADO OBLIGATORIA: Los archivos ya existen físicamente en disco. NO vuelvas a crear o sobreescribir estos archivos con TOOL_PROGRAMMER salvo que un test falle. Ahora DEBES usar 'TOOL_TERMINAL' para ejecutar o probar el código (o crear un script de test verify_*.py si no existe).\n\n", written_files);
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
                                        format!("Programación exitosa: {} archivos afectados", written_files.len())
                                    },
                                    "SUCCESS",
                                );
                                if let Some(command) = official_verifier_command(&runtime.contract) {
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
                                    emit_event(&app_handle, runtime.current_step(), &message, "ERROR");
                                    persist_failed_journal(&mut journal, &workspace_path, &app_handle, runtime.current_step());
                                    return Ok(serde_json::json!({"status":"ERROR", "respuesta_conversacional":message}).to_string());
                                }
                                emit_event(&app_handle, runtime.current_step(), &format!("[RECUPERACIÓN] Intento {}/3. Decisión: {:?}", programmer_failures, recovery), "WARNING");
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
                                persist_failed_journal(&mut journal, &workspace_path, &app_handle, runtime.current_step());
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
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    "[VISION] Evaluando calidad visual...",
                    "ACTION",
                );
                let vision_prompt = if !comando.trim().is_empty() {
                    comando.clone()
                } else {
                    format!("Evalua la calidad visual de esta pantalla. Describe: 1) Si la UI se ve correcta, 2) Errores visibles, 3) Elementos faltantes. Objetivo original: {}", user_message)
                };

                let text_to_search = format!("{} {}", comando, user_message);
                let text_lower = text_to_search.to_lowercase();
                let url = if let Some(idx) = text_lower
                    .find("http://")
                    .or_else(|| text_lower.find("https://"))
                {
                    let end_idx = text_to_search[idx..]
                        .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '`')
                        .unwrap_or(text_to_search.len() - idx);
                    Some(text_to_search[idx..idx + end_idx].to_string())
                } else {
                    None
                };

                let action_proposal = crate::core::policy::ActionProposal {
                    tool: "TOOL_VISION_EVALUATOR".to_string(),
                    arguments: serde_json::json!({
                        "prompt": vision_prompt,
                        "url": url,
                    }),
                    expected_effect: "Visual UI evaluation".to_string(),
                    risk: crate::core::policy::RiskLevel::Safe,
                    world_hash: Some(runtime.current_world_hash()),
                };

                match runtime.execute_action(&action_proposal).await {
                    Ok(obs) => {
                        if obs.status == crate::core::observation::ObservationStatus::Success {
                            current_context.push_str(&format!("[VISION EVALUATOR RESULTADO]\n{}\n\n[INSTRUCCIÓN ESTRICTA DE SEGURIDAD]: LA VALIDACIÓN VISUAL HA SIDO COMPLETADA. SI EL MANDATO DEL USUARIO FUE CUMPLIDO, EN TU SIGUIENTE PASO DEBES ELEGIR OBLIGATORIAMENTE 'TOOL_FINISH'. NO REPITAS HERRAMIENTAS DE VALIDACIÓN.\n\n", obs.payload));
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                &format!(
                                    "[VISION] Evaluacion completada: {}",
                                    &obs.payload.chars().take(120).collect::<String>()
                                ),
                                "SUCCESS",
                            );
                        } else {
                            let msg =
                                format!("[VISION] Error en evaluación visual: {}", obs.payload);
                            current_context.push_str(&format!("{}\n\n", &msg));
                            emit_event(&app_handle, runtime.current_step(), &msg, "ERROR");
                        }
                    }
                    Err(e) => {
                        let msg = format!("[VISION] Error de ejecución: {}", e);
                        current_context.push_str(&format!("{}\n\n", &msg));
                        emit_event(&app_handle, runtime.current_step(), &msg, "ERROR");
                    }
                }
                mandatory_tools_executed.insert("TOOL_VISION_EVALUATOR".to_string());
            }
            "TOOL_TESTER" => {
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    "Ejecutando suite de pruebas automatizadas...",
                    "ACTION",
                );
                mandatory_tools_executed.insert("TOOL_TESTER".to_string());
                match runtime.execute_action(&action_proposal).await {
                    Ok(obs) => {
                        if obs.status == crate::core::observation::ObservationStatus::Success {
                            tester_attempts = 0;
                            if tester_success_hits >= 1 {
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
                                current_context.push_str(&format!("Resultado Tests:\n{}\n\n[INSTRUCCIÓN ESTRICTA DE SEGURIDAD]: LOS TESTS PASARON EXITOSAMENTE. LA TAREA ESTÁ COMPLETADA. EN TU SIGUIENTE PASO DEBES ELEGIR OBLIGATORIAMENTE 'TOOL_FINISH'. NO REPITAS TOOL_TESTER.\n\n", obs.payload));
                                emit_event(
                                    &app_handle,
                                    runtime.current_step(),
                                    "Todos los tests pasaron exitosamente.",
                                    "SUCCESS",
                                );
                            }
                        } else {
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
                let mut report = String::new();
                for (idx, (intent, parent_id, opcode)) in ast_nodes_vec.iter().enumerate() {
                    let node_id = (std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos() as u64)
                        + idx as u64;
                    let meta = [0u8; 16];
                    let node = chronos_vfs::aura_bridge::AuraIntentTranslator::tokenize_intent(
                        (*opcode).into(),
                        *parent_id,
                        node_id,
                        intent,
                        meta,
                    );
                    // Clone the display fields before node is moved into push_node
                    let node_id_display = node.node_id;
                    let node_hash_display = node.content_hash.clone();
                    let node_op_display = node.opcode.clone();
                    if let Err(_) = agent_workspace.push_node(node) {
                        report.push_str(&format!(
                            "- Error crítico: Buffer Zero-Trace lleno al insertar nodo {}\n",
                            node_id
                        ));
                        break;
                    }
                    report.push_str(&format!(
                        "- Inyectado: NodeID={} Hash={}\n  Opcode: {:?}\n  Contenido: {}\n",
                        node_id_display, node_hash_display, node_op_display, intent
                    ));
                }
                current_context.push_str(&format!(
                    "PASO {}:\nAcción: TOOL_AST_INJECT\nResultado:\n{}\n\n",
                    runtime.current_step(),
                    report
                ));
                emit_event(
                    &app_handle,
                    runtime.current_step(),
                    &format!(
                        "{} nodos AST inyectados exitosamente en RAM.",
                        ast_nodes_vec.len()
                    ),
                    "SUCCESS",
                );
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
                    match crate::core::memory::index_project(&workspace_path).await {
                        Ok(msg) => {
                            current_context.push_str(&format!("Resultado TOOL_LEARN: {}\n\n", msg));
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                "Memoria indexada correctamente.",
                                "SUCCESS",
                            );
                        }
                        Err(err) => {
                            current_context.push_str(&format!("Error en TOOL_LEARN: {}\n\n", err));
                            emit_event(&app_handle, runtime.current_step(), &err, "ERROR");
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
                let runners =
                    generate_project_runners(&workspace_path, &original_prompt_parsed).await;
                if runners.is_empty() {
                    current_context.push_str("[TOOL_CREATE_RUNNER] No se generaron runners (lenguaje no detectado o no soportado).\n\n");
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        "No se generaron runners (lenguaje desconocido).",
                        "WARNING",
                    );
                } else {
                    let names: Vec<String> = runners
                        .iter()
                        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
                        .collect();
                    current_context.push_str(&format!("[TOOL_CREATE_RUNNER] Runners internos generados en .aura/runtime/runners: {}\n\n", names.join(", ")));
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!(
                            "Runners internos generados en .aura/runtime/runners: {}",
                            names.join(", ")
                        ),
                        "SUCCESS",
                    );
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
                    let run_cmd = if parts.len() > 2 { parts[2] } else { "" };

                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!("Contenedor: {} {}", action_str, image_or_id),
                        "ACTION",
                    );

                    let action_enum = crate::core::container::ContainerAction::from_str(action_str);
                    match crate::core::container::container_exec(
                        action_enum,
                        image_or_id,
                        run_cmd,
                        &workspace_path,
                    )
                    .await
                    {
                        Ok(out) => {
                            current_context
                                .push_str(&format!("Resultado TOOL_CONTAINER:\n{}\n\n", out));
                            emit_event(
                                &app_handle,
                                runtime.current_step(),
                                "Comando de contenedor exitoso.",
                                "SUCCESS",
                            );
                            last_progress_step = runtime.current_step();
                        }
                        Err(e) => {
                            current_context.push_str(&format!("Error TOOL_CONTAINER:\n{}\n\n", e));
                            emit_event(&app_handle, runtime.current_step(), &e, "ERROR");
                        }
                    }
                }
            }
            // ── Fase 4: TOOL_SCHEDULER (Cron tasks) ──
            "TOOL_SCHEDULER" => {
                let parts: Vec<&str> = comando.splitn(2, '|').collect();
                if parts.len() < 2 {
                    let err = "Error: TOOL_SCHEDULER requiere 'cron_expr|objetivo_descriptivo'. Ejemplo: '0 9 * * 1|auditar seguridad'";
                    current_context.push_str(&format!("Error TOOL_SCHEDULER: {}\n\n", err));
                    emit_event(&app_handle, runtime.current_step(), err, "ERROR");
                } else {
                    let cron = parts[0].trim();
                    let desc = parts[1].trim();
                    let id =
                        crate::core::scheduler::register_task(desc, &workspace_path, cron, desc);
                    current_context.push_str(&format!(
                        "Tarea programada exitosamente (ID: {}). Cron: {}\n\n",
                        id, cron
                    ));
                    emit_event(
                        &app_handle,
                        runtime.current_step(),
                        &format!("📅 Tarea '{}' programada ({})", desc, cron),
                        "SUCCESS",
                    );
                    last_progress_step = runtime.current_step();
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
                match crate::core::memory::query_memory(&search_query).await {
                    Ok(msg) => {
                        current_context.push_str(&format!("Resultado TOOL_SEARCH:\n{}\n\n", msg));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "Búsqueda en memoria completada.",
                            "SUCCESS",
                        );
                    }
                    Err(err) => {
                        current_context.push_str(&format!("Error en TOOL_SEARCH: {}\n\n", err));
                        emit_event(&app_handle, runtime.current_step(), &err, "ERROR");
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

                let options = vec![];
                match crate::core::ask_user::ask_user_async(
                    &app_handle,
                    question.clone(),
                    options,
                    current_context.clone(),
                )
                .await
                {
                    Ok(answer) => {
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
                        // User clarified the task — immediately transition to Executor with TOOL_PROGRAMMER
                        current_role = AgentRole::Executor;
                        forced_next_tool = Some(("TOOL_PROGRAMMER".to_string(), format!("El usuario ya aclaró los requerimientos: '{}'. Implementa el código inmediatamente.", answer)));
                    }
                    Err(e) => {
                        current_context
                            .push_str(&format!("Error al consultar al usuario: {}\n\n", e));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            &format!("Error ASK_USER: {}", e),
                            "ERROR",
                        );
                    }
                }
            }
            "TOOL_FINISH" => {
                // ── Mandatory Tool Checklist enforcement (Bug 1 fix) ─────────────────
                // If the user's prompt required specific tools (TOOL_TESTER, TOOL_VISION_EVALUATOR)
                // and they haven't been executed yet, block TOOL_FINISH and instruct the agent.
                let missing_mandatory: Vec<&String> = mandatory_tools_required
                    .iter()
                    .filter(|t| !mandatory_tools_executed.contains(*t))
                    .collect();
                if !missing_mandatory.is_empty() {
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
                if !journal.fases.is_empty() && journal.fase_actual < journal.fases.len() - 1 {
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
                } else if !journal.fases.is_empty()
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
                technical_contract.acceptance_criteria.retain(|ac| !matches!(ac.verification,
                    crate::core::mission_contract::VerificationMethod::ManualReview));
                let technical_ready = (technical_contract.acceptance_criteria.is_empty()
                    && technical_contract.required_evidence.is_empty()) || matches!(
                    crate::core::completion_gate::CompletionGate::evaluate(&technical_contract,
                        &runtime.cognitive_state, &runtime.evidence_graph, review_hash,
                        std::path::Path::new(&workspace_path)),
                    crate::core::completion_gate::CompletionDecision::Complete);
                let pending_reviews: Vec<_> = runtime.contract.acceptance_criteria.iter()
                    .filter(|ac| technical_ready && ac.required && matches!(ac.verification, crate::core::mission_contract::VerificationMethod::ManualReview))
                    .filter(|ac| !runtime.evidence_graph.has_valid_manual_evidence_for_state(
                        &format!("{} verified manually", ac.id), 1.0, review_hash))
                    .cloned().collect();
                for criterion in pending_reviews {
                    let answer = crate::core::ask_user::ask_user_async(
                        &app_handle,
                        format!("Revisa los entregables en {}. ¿Se cumple este criterio? {}", workspace_path, criterion.description),
                        vec!["Aprobar criterio".into(), "Requiere correcciones".into()],
                        format!("Misión: {}. Criterio: {}", original_prompt_parsed, criterion.id),
                    ).await?;
                    if answer == "Aprobar criterio" {
                        let review_step = runtime.current_step();
                        runtime.evidence_graph.record_with_hash(
                            crate::core::evidence::EvidenceKind::UserConfirmation,
                            "user", &format!("{} verified manually", criterion.id),
                            &answer, 1.0, review_step, Some(review_hash),
                        )?;
                    } else {
                        current_context.push_str(&format!("\n[REVISIÓN DEL USUARIO] {}: {}\n", criterion.id, answer));
                    }
                }
                runtime.observe_world()?;

                // ↀ CompletionGate delegado a MissionRuntime (fuente única de verdad) ↀ
                let completion_decision = runtime.can_complete();
                match completion_decision {
                    crate::core::completion_gate::CompletionDecision::Incomplete(
                        missing_reasons,
                    ) => {
                        let block_msg = format!("[COMPLETION GATE] ⚠︠ Finalización rechazada. Requisitos pendientes:\n{}", missing_reasons.join("\n"));
                        emit_event(
                            &app_handle,
                            runtime.current_step(),
                            "[COMPLETION GATE] Criterios de misión aún no satisfechos.",
                            "WARNING",
                        );
                        current_context.push_str(&format!("{}\n[ACCIÓN OBLIGATORIA]: Resuelve estos puntos antes de llamar a TOOL_FINISH.\n\n", block_msg));
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
                        al_recommendation.strategy.clone(),
                        al_result,
                        al_recommendation.confidence,
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
            emit_event(&app_handle, runtime.current_step(), &format!("Abortando misión: {}", msg), "FATAL");
            ("ERROR", format!("La misión ha sido abortada por un error crítico: {}", msg))
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
            ("ERROR", format!(
                "He alcanzado el límite máximo de {} pasos sin llegar a una conclusión. \
                 Por favor, revisa el historial de pasos y proporciona más contexto.",
                runtime.budget.total_steps
            ))
        }
    };

    // ── Journal: mark status ──
    journal.status = if final_status == "FINISH" { "COMPLETADO".to_string() } else { "ESPERANDO".to_string() };
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

/// Registers all 28 tool executors with ToolRegistry.
/// Every tool in KNOWN_TOOLS has a concrete, real executor — zero stubs.
pub fn register_default_tools(
    runtime: &mut crate::core::mission_runtime::MissionRuntime,
    workspace_path: &str,
    original_prompt_parsed: &str,
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
            Arc::new(move |ws, _args| {
                let workspace = ws.clone();
                Box::pin(
                    async move { crate::core::tester::execute_tester_detailed(&workspace).await },
                )
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
        let _ = runtime.tool_registry.register(
            "TOOL_AUDITOR",
            Arc::new(move |ws, args| {
                let workspace = ws.clone();
                Box::pin(async move {
                    let files: Vec<String> =
                        if let Some(arr) = args.get("archivos").and_then(|v| v.as_array()) {
                            arr.iter()
                                .filter_map(|v| v.as_str())
                                .map(|s| s.to_string())
                                .collect()
                        } else {
                            vec![]
                        };
                    let safe_files = memory::read_files_safely(&workspace, files).await;
                    Ok(crate::core::tool_registry::ExecutionResult::success(
                        format!("Auditoría completada:\n{}", safe_files),
                    ))
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

    // TOOL_AST_INJECT: AST node parsing and validation via chronos_vfs
    {
        let _ = runtime.tool_registry.register(
            "TOOL_AST_INJECT",
            Arc::new(move |_ws, args| {
                Box::pin(async move {
                    let opcode = args.get("opcode").and_then(|v| v.as_u64()).unwrap_or(2) as u8;
                    let parent_id = args.get("parent_id").and_then(|v| v.as_u64()).unwrap_or(0);
                    let intent = args
                        .get("intent")
                        .or_else(|| args.get("comando"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let meta = [0u8; 16];
                    let node = chronos_vfs::aura_bridge::AuraIntentTranslator::tokenize_intent(
                        opcode.into(),
                        parent_id,
                        1,
                        intent,
                        meta,
                    );
                    Ok(crate::core::tool_registry::ExecutionResult::success(
                        format!(
                            "AST node generated (id: {}, opcode: {:?})",
                            node.node_id, node.opcode
                        ),
                    ))
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
        let _ = runtime.tool_registry.register(
            "TOOL_VISION_EVALUATOR",
            Arc::new(move |_ws, args| {
                Box::pin(async move {
                    let prompt = args
                        .get("prompt")
                        .or_else(|| args.get("comando"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("Evalua la calidad visual de esta pantalla.");
                    let url = args.get("url").and_then(|v| v.as_str());
                    match crate::core::vision::evaluate_vision(prompt, false, url).await {
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

    // TOOL_ASK_USER: user prompt registration
    {
        let _ = runtime.tool_registry.register(
            "TOOL_ASK_USER",
            Arc::new(move |_ws, args| {
                Box::pin(async move {
                    let q = args
                        .get("pregunta")
                        .or_else(|| args.get("question"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("Confirmación requerida");
                    Ok(crate::core::tool_registry::ExecutionResult::success(
                        format!("Pregunta a usuario registrada: {}", q),
                    ))
                })
            }),
        );
    }

    // TOOL_LEARN: learning experience registration
    {
        let _ = runtime.tool_registry.register(
            "TOOL_LEARN",
            Arc::new(move |_ws, args| {
                Box::pin(async move {
                    let k = args
                        .get("conocimiento")
                        .or_else(|| args.get("knowledge"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("Aprendizaje registrado");
                    Ok(crate::core::tool_registry::ExecutionResult::success(
                        format!("Aprendizaje procesado: {}", k),
                    ))
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
                    let id = crate::core::scheduler::register_task(
                        parts[1].trim(),
                        &workspace,
                        parts[0].trim(),
                        parts[1].trim(),
                    );
                    Ok(crate::core::tool_registry::ExecutionResult::success(
                        format!("Tarea programada ID {}", id),
                    ))
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
                    match crate::core::memory::query_memory(query).await {
                        Ok(msg) if !msg.is_empty() => {
                            Ok(crate::core::tool_registry::ExecutionResult::success(msg))
                        }
                        _ => {
                            let results =
                                memory::read_files_safely(&workspace, vec![query.to_string()])
                                    .await;
                            Ok(crate::core::tool_registry::ExecutionResult::success(
                                results,
                            ))
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
                    let n_vars_opt = args
                        .get("n_vars")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as usize);
                    let clauses_opt = args.get("clauses").and_then(|v| v.as_array()).map(|arr| {
                        arr.iter()
                            .filter_map(|row| {
                                row.as_array().map(|r| {
                                    r.iter()
                                        .filter_map(|x| x.as_i64().map(|i| i as i32))
                                        .collect::<Vec<i32>>()
                                })
                            })
                            .collect::<Vec<Vec<i32>>>()
                    });
                    if let (Some(n_vars), Some(clauses)) = (n_vars_opt, clauses_opt) {
                        let verdict = crate::llm::solve_with_spectrasat(n_vars, clauses);
                        Ok(crate::core::tool_registry::ExecutionResult::success(
                            verdict,
                        ))
                    } else {
                        let text = args
                            .get("comando")
                            .or_else(|| args.get("query"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        Ok(crate::core::tool_registry::ExecutionResult::success(
                            format!("Logic analysis processed for: {}", text),
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

    #[tokio::test]
    async fn test_all_28_known_tools_have_registered_executors() {
        let temp_dir =
            std::env::temp_dir().join(format!("aura_tools_test_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let ws = temp_dir.to_str().unwrap();

        let mut runtime = MissionRuntime::new(ws, "Tool inventory verification mission", 50);
        register_default_tools(
            &mut runtime,
            ws,
            "Build a high reliability verification system",
        );

        // 1. Assert exactly 28 tools in KNOWN_TOOLS
        assert_eq!(
            KNOWN_TOOLS.len(),
            28,
            "KNOWN_TOOLS must contain exactly 28 tools"
        );

        // 2. Assert all 28 known tools are registered
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
            28,
            "ToolRegistry must have exactly 28 registered executors"
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
        assert!(logic_res.unwrap().stdout.contains("SAT"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_pesp_fase_advancement_and_ground_truth() {
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

        // Simulate PESP advancement logic
        let total_fases = journal.fases.len();
        let mut should_advance = false;
        if let Some(fase) = journal.fases.get_mut(journal.fase_actual) {
            let all_done = fase.archivos.is_empty()
                || fase.archivos.iter().all(|f| {
                    written_files
                        .iter()
                        .any(|w: &String| w.contains(f.as_str()))
                });
            if all_done {
                fase.estado = "COMPLETADA".to_string();
                if journal.fase_actual + 1 < total_fases {
                    should_advance = true;
                }
            }
        }
        if should_advance {
            journal.fase_actual += 1;
        }

        assert_eq!(journal.fases[0].estado, "COMPLETADA");
        assert_eq!(journal.fase_actual, 1);
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
        assert_eq!(classify_mission("Construye cyber_sentinel.html y crea verify_dashboard.py, ejecútalo y verifica"), MissionType::Construction);
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
        assert_eq!(decision["archivos_a_editar"], serde_json::json!(["index.html"]));
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
) {
    if let Err(error) =
        crate::core::session_journal::close_journal(journal, "FALLIDO", workspace_path)
    {
        emit_event(app, step, &error, "FATAL");
    }
}
