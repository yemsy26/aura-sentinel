use serde::Serialize;
use tauri::{AppHandle, Emitter};
use crate::memory;
use crate::core::{
    execute_terminal_command, start_background_task, read_task_logs, kill_task,
    validate_workspace, format_system_error,
    runner_generator::generate_standard_runners,
    command_trail::StepResult, // CommandTrail used inline via full path in the trail block
};
use super::{call_ollama, delegate_to_programmer, delegate_to_auditor, delegate_to_logic_solver, ProgrammerOutput};

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

/// Intenta recuperar código válido desde respuestas del programador cuando el JSON estricto se rompe.
/// Busca bloques Markdown (```html, ```python, etc.) o subcadenas JSON para no descartar código funcional.
fn try_salvage_programmer_output(raw: &str, requested_files: &[String]) -> Option<ProgrammerOutput> {
    // 1. Intentar encontrar subcadena JSON válida entre el primer '{' y el último '}'
    if let (Some(first_brace), Some(last_brace)) = (raw.find('{'), raw.rfind('}')) {
        if last_brace > first_brace {
            let candidate = &raw[first_brace..=last_brace];
            if let Ok(po) = serde_json::from_str::<ProgrammerOutput>(candidate) {
                return Some(po);
            }
        }
    }
    
    // 2. Extraer bloques de código Markdown estructurados
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
            let code_start = start + block_prefix.len();
            if let Some(end) = raw[code_start..].find("```") {
                let code = raw[code_start..code_start + end].trim();
                if !code.is_empty() {
                    for file in requested_files {
                        let f_ext = file.split('.').last().unwrap_or("").to_lowercase();
                        if f_ext == *ext_match || requested_files.len() == 1 {
                            return Some(ProgrammerOutput {
                                pensamiento: Some("Código recuperado automáticamente desde bloque Markdown".to_string()),
                                explicacion_tecnica: format!("Extracción resiliente de bloque ```{}```", tag),
                                cambios: vec![
                                    crate::memory::Cambio {
                                        archivo: file.clone(),
                                        buscar: "".to_string(),
                                        reemplazar: code.to_string(),
                                    }
                                ],
                            });
                        }
                    }
                }
            }
        }
    }
    None
}

/// Detecta si el workspace contiene archivos HTML (entorno web/frontend)
fn has_html_files(workspace_path: &str) -> bool {
    if let Ok(world) = crate::core::world_state::WorldState::capture(workspace_path) {
        return world.files.keys().any(|f| f.to_lowercase().ends_with(".html") || f.to_lowercase().ends_with(".htm"));
    }
    false
}

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
            || l_lower.contains("assert") {
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
        format!("{}...\n[Salida recortada para eficiencia]", &result[..max_chars])
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
    Analysis,      // "analiza", "describe", "explica", "qué hay"
    Construction,  // "crea", "implementa", "build"
    Refactor,      // "mejora", "optimiza", "refactoriza"
    Debug,         // "arregla", "bug", "error", "fix"
    Execution,     // "ejecuta", "corre", "prueba", "testea", "run", "verify"
}

fn classify_mission(msg: &str) -> MissionType {
    let m = msg.to_lowercase();
    let execution = ["ejecuta", "ejecutar", "corre", "correr", "testea", "run", "execute", "verifica", "verificar"];
    let construction = ["construye", "construir", "crea", "crear", "implementa", "implementar",
                        "escribe", "escribir", "genera", "generar", "programa", "programar",
                        "desarrolla", "desarrollar", "build", "create", "write", "microservicio"];
    let debug = ["arregla", "corrige", "bug", "falla", "fallo", "fix", "debug", "broken",
                 "no funciona", "no compila", "sale error", "hay un error"];
    let refactor = ["refactoriza", "refactorizar", "mejora", "optimiza", "limpia el", "reorganiza", "simplifica"];
    let analysis = ["analiza", "analisa", "analice", "analisis", "que hay", "qué hay", "que sistema",
                    "qué sistema", "describe", "explica", "muéstrame", "muestrame", "que tiene",
                    "qué tiene", "que contiene", "qué contiene", "inspect", "analyze", "show me",
                    "que es", "qué es", "que tipo", "qué tipo", "que hace", "qué hace",
                    "analisa este", "analiza este", "revisa este",
                    "auditoría", "auditoria", "audita", "sat", "lógica", "logica", "satisfacibilidad",
                    "donde quedaste", "dónde quedaste", "donde te quedaste", "dónde te quedaste",
                    "en que quedamos", "en qué quedamos", "en que quedaste", "en qué quedaste",
                    "que falta", "qué falta", "que queda", "qué queda", "como va", "cómo va",
                    "estado actual", "estado del proyecto", "cual es el estado", "cuál es el estado",
                    "resumen del estado", "informe", "reporte", "status"];

    if execution.iter().any(|w| m.contains(w))    { return MissionType::Execution; }
    if construction.iter().any(|w| m.contains(w)) { return MissionType::Construction; }
    if debug.iter().any(|w| m.contains(w))        { return MissionType::Debug; }
    if refactor.iter().any(|w| m.contains(w))     { return MissionType::Refactor; }
    if analysis.iter().any(|w| m.contains(w))     { return MissionType::Analysis; }
    MissionType::Construction
}

/// Helper to verify if a required file from a phase is strictly satisfied on disk.
/// Requires exact file presence and non-empty content (size > 0).
/// Does not allow loose extension matching or inline script shortcuts.
fn is_phase_file_satisfied(workspace_path: &str, file_name: &str) -> bool {
    let ws = std::path::Path::new(workspace_path);
    crate::core::workspace_resolver::WorkspaceResolver::file_exists(ws, file_name).unwrap_or(false)
}

/// Helper to auto-extract commands from the agent's thought when comando is empty.
fn extract_command_from_thought(pensamiento: &str, workspace_path: &str) -> Option<String> {
    for quote in ['`', '"', '\''] {
        let parts: Vec<&str> = pensamiento.split(quote).collect();
        if parts.len() >= 3 {
            for chunk in parts.chunks(2).skip(1) {
                if let Some(&cand) = chunk.first() {
                    let cand_trim = cand.trim();
                    if cand_trim.starts_with("python ")
                        || cand_trim.starts_with("node ")
                        || cand_trim == "dir"
                        || cand_trim == "ls"
                        || cand_trim.starts_with("npm ")
                        || cand_trim.starts_with("cargo ")
                    {
                        return Some(cand_trim.to_string());
                    }
                    if cand_trim.ends_with(".py") && std::path::Path::new(workspace_path).join(cand_trim).exists() {
                        return Some(format!("python {}", cand_trim));
                    }
                    if cand_trim.ends_with(".js") && std::path::Path::new(workspace_path).join(cand_trim).exists() {
                        return Some(format!("node {}", cand_trim));
                    }
                }
            }
        }
    }

    let lower = pensamiento.to_lowercase();
    for word in lower.split_whitespace() {
        let clean = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '_' && c != '-');
        if clean.ends_with(".py") {
            let script_path = std::path::Path::new(workspace_path).join(clean);
            if script_path.exists() {
                return Some(format!("python {}", clean));
            }
        } else if clean.ends_with(".js") {
            let script_path = std::path::Path::new(workspace_path).join(clean);
            if script_path.exists() {
                return Some(format!("node {}", clean));
            }
        }
    }

    None
}

/// Formats the acceptance contract from the Planner's TOOL_THINK 'comando' field.
fn formato_contrato(cmd: &str) -> String {
    format!("CRITERIOS DE EXITO DEFINIDOS POR EL PLANIFICADOR:\n{}", cmd)
}

/// Auto-genera runners (test, build, dev, lint) para el proyecto detectando el lenguaje
async fn generate_project_runners(workspace_path: &str, prompt: &str) -> Vec<std::path::PathBuf> {
    use std::path::Path;
    
    let project_root = Path::new(workspace_path);
    let _all_generated: Vec<std::path::PathBuf> = Vec::new();
    
    let projects = detect_projects(workspace_path);
    let mut all_runners = Vec::new();

    for proj in projects {
        let language = proj.language;
        let root = proj.root;

        let mut lang = language.clone();
        if lang == "unknown" {
            let prompt_lower = prompt.to_lowercase();
            if prompt_lower.contains("rust") || prompt_lower.contains("cargo") { lang = "rust".to_string(); }
            else if prompt_lower.contains("python") || prompt_lower.contains("django") || prompt_lower.contains("flask") || prompt_lower.contains("fastapi") { lang = "python".to_string(); }
            else if prompt_lower.contains("javascript") || prompt_lower.contains("node") || prompt_lower.contains("react") || prompt_lower.contains("vue") || prompt_lower.contains("npm") { lang = "javascript".to_string(); }
            else if prompt_lower.contains("typescript") || prompt_lower.contains("tsx") || prompt_lower.contains("ts ") { lang = "typescript".to_string(); }
            else if prompt_lower.contains("go ") || prompt_lower.contains("golang") { lang = "go".to_string(); }
            else if prompt_lower.contains("java") || prompt_lower.contains("spring") || prompt_lower.contains("maven") || prompt_lower.contains("gradle") { lang = "java".to_string(); }
        }

        if lang != "unknown" {
            let mut runners = generate_for_language(&lang, &root).await;
            all_runners.append(&mut runners);
        }
    }
    all_runners
}

async fn generate_for_language(language: &str, project_root: &std::path::Path) -> Vec<std::path::PathBuf> {
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
    ).await {
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
        if path.join("Cargo.toml").exists() { lang = "rust".to_string(); }
        else if path.join("package.json").exists() {
            if path.join("tsconfig.json").exists() { lang = "typescript".to_string(); }
            else { lang = "javascript".to_string(); }
        }
        else if path.join("requirements.txt").exists() || path.join("pyproject.toml").exists() || path.join("main.py").exists() { lang = "python".to_string(); }
        else if path.join("go.mod").exists() { lang = "go".to_string(); }
        else if path.join("pom.xml").exists() { lang = "java".to_string(); }
        else if path.join("build.gradle").exists() || path.join("build.gradle.kts").exists() { lang = "kotlin".to_string(); }
        else if path.join("composer.json").exists() { lang = "php".to_string(); }
        else if path.join("pubspec.yaml").exists() { lang = "dart".to_string(); }
        else if path.join("Package.swift").exists() { lang = "swift".to_string(); }
        else if std::fs::read_dir(&path).map(|entries| entries.filter_map(|e| e.ok()).any(|e| e.path().extension().map(|ext| ext == "csproj").unwrap_or(false))).unwrap_or(false) { lang = "csharp".to_string(); }
        else if path.join("Gemfile").exists() { lang = "ruby".to_string(); }
        else if path.join("foundry.toml").exists() || path.join("hardhat.config.js").exists() || path.join("hardhat.config.ts").exists() { lang = "solidity".to_string(); }

        if lang != "unknown" {
            projects.push(ProjectDescriptor { root: path, language: lang });
        }
    }
    
    if projects.is_empty() {
        projects.push(ProjectDescriptor { root: ws.to_path_buf(), language: "unknown".to_string() });
    }
    
    projects
}

pub const DEFAULT_ORCHESTRATOR_MODEL: &str = "qwen2.5-coder:7b";
#[allow(dead_code)]
pub const DEFAULT_PROGRAMMER_MODEL: &str = "qwen2.5-coder:7b";

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
        m.as_str() == req || m.starts_with(&format!("{}:", req)) || (m_base == req_base && (m.ends_with(":latest") || !req.contains(':')))
    }) {
        return found.clone();
    }

    // 3. Si coincide prefijo completo
    if let Some(found) = available_models.iter().find(|m| m.starts_with(req) || req.starts_with(m.as_str())) {
        return found.clone();
    }

    // 4. Fallback de seguridad si el modelo solicitado no existe en Ollama
    if let Some(valid) = available_models.iter().find(|m| !m.contains("embed") && (m.contains("coder") || m.contains("qwen"))) {
        valid.clone()
    } else if let Some(valid) = available_models.iter().find(|m| !m.contains("embed")) {
        valid.clone()
    } else {
        DEFAULT_ORCHESTRATOR_MODEL.to_string()
    }
}

pub async fn run_agent_loop(
    mut user_message: String,
    workspace_path: String,
    _tree_json: String,
    orchestrator_model: String,
    programmer_model: String,
    app_handle: AppHandle
) -> Result<String, String> {
    
    // PRE-FLIGHT CHECK
    emit_event(&app_handle, 0, "Ejecutando validación ambiental (Pre-Flight Check)...", "ACTION");
    let available_models = match crate::core::env_check::validate_environment(&workspace_path).await {
        Ok(models) => models,
        Err(env_errors) => {
            let error_msg = env_errors.join("\n");
            emit_event(&app_handle, 0, &format!("[ENV_FAILURE] Fallo pre-vuelo:\n{}", error_msg), "FATAL");
            let final_res = FinalResponse {
                status: "FINISH".to_string(),
                respuesta_conversacional: format!("[ENV_FAILURE] No puedo continuar porque el entorno no cumple con los requisitos mínimos:\n{}\n\nPor favor, soluciona esto e intenta de nuevo.", error_msg),
            };
            return Ok(serde_json::to_string(&final_res).unwrap());
        }
    };
    emit_event(&app_handle, 0, "Pre-Flight Check superado.", "SUCCESS");

    // Resolver modelos de forma global respetando estrictamente la selección del usuario.
    let orchestrator_model = resolve_model_or_fallback(&orchestrator_model, &available_models);
    let programmer_model = resolve_model_or_fallback(&programmer_model, &available_models);
    emit_event(&app_handle, 0, &format!("⚙️ [CEREBRO GLOBAL ACTIVO] Modelo: {}", orchestrator_model), "INFO");
            
    let mut current_context = String::new();
    
    // ── Inject current workspace state first ──────────────────────────────
    // Always show the LLM what ACTUALLY exists in the workspace right now.
    // This prevents hallucinating a blank-slate project when files already exist.
    {
        let mut existing_files = Vec::new();
        fn scan_workspace_files(dir: &std::path::Path, files: &mut Vec<String>, depth: usize) {
            if depth > 5 { return; }
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                    // Skip node_modules, .git, __pycache__, hidden dirs
                    if name.starts_with('.') || name == "node_modules" || name == "__pycache__" || name == "target" { continue; }
                    if path.is_dir() {
                        scan_workspace_files(&path, files, depth + 1);
                    } else {
                        files.push(path.to_string_lossy().to_string());
                    }
                }
            }
        }
        scan_workspace_files(std::path::Path::new(&workspace_path), &mut existing_files, 0);
        if !existing_files.is_empty() {
            let relative_existing: Vec<String> = existing_files.iter()
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
        let source_file_count = existing_files.iter().filter(|f| {
            let fl = f.to_lowercase();
            fl.ends_with(".py") || fl.ends_with(".js") || fl.ends_with(".ts")
            || fl.ends_with(".tsx") || fl.ends_with(".jsx") || fl.ends_with(".rs")
            || fl.ends_with(".go")
        }).count();

        if source_file_count >= 3 {
            emit_event(&app_handle, 0, "🗺️ [AUTO-MAPPER] Proyecto multi-archivo detectado. Generando grafo de dependencias...", "ACTION");
            let graph = crate::core::dependency_mapper::analyze_workspace(&workspace_path);
            let report = crate::core::dependency_mapper::format_graph_report(&graph);
            current_context.push_str(&format!(
                "[AUTO-MAPPER] Grafo de dependencias generado automáticamente para este proyecto.\n\n{}\n\n",
                report
            ));
            emit_event(&app_handle, 0,
                &format!("🗺️ Grafo listo: {} archivos | {} dependencias", graph.nodes.len(), graph.edges.len()),
                "SUCCESS");
        }
    }

    // ── RAG Memory (secondary, clearly labelled) ───────────────────────────
    // RAG provides OPTIONAL historical patterns. It NEVER overrides the current task.
    // The current task is ALWAYS the user_message above.
    if let Ok(historia) = crate::core::memory::query_memory(&user_message).await {
        if !historia.contains("vacía") && !historia.contains("No se encontró") {
            current_context.push_str(&format!(
                "[CONTEXTO HISTÓRICO OPCIONAL — solo como referencia de patrones, NO como objetivo actual]:\n{}\n\n",
                historia
            ));
        }
    }
    let mut archivos_editados_historico: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut comandos_ejecutados_historico: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut paquetes_instalados_historico: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut architect_used = false;
    let mut tester_attempts = 0;
    let mut tester_success_hits = 0;
    let mut programmer_cooldown_hits = 0;
    let mut original_prompt_parsed = if let Some(idx) = user_message.find("\n\nGuía de Traducción Técnica") {
        let text = &user_message[..idx];
        text.replace("Petición Original del Usuario: ", "").trim().to_string()
    } else {
        user_message.clone()
    };
    let mut no_tests_consecutive = 0u32;
    let mut think_consecutive = 0u32;
    let mut _programmer_consecutive = 0u32; // reserved for future per-tool cooldown
    // ── THINK↔PROGRAMMER alternation loop detector ─────────────────────────
    // Tracks consecutive steps that are ONLY THINK or PROGRAMMER with no
    // TERMINAL, TESTER, or FINISH in between. If this reaches >= 8 steps,
    // we force TOOL_TERMINAL to break the loop.
    let mut think_programmer_alternation_count = 0u32;
    let mut auditor_consecutive = 0u32;
    let mut mapper_consecutive = 0u32;
    let mut critic_fsm_lock_consecutive = 0u32;
    let mut workspace_manager_error_consecutive = 0u32;
    let mut learn_consecutive = 0u32;
    let mut unknown_tool_consecutive = 0u32;

    // ── Mandatory Tool Checklist (Bug 1 fix) ──────────────────────────────────
    // Parse user_message for required tools and enforce them before TOOL_FINISH.
    let mandatory_tools_required: std::collections::HashSet<String> = {
        let mut required = std::collections::HashSet::new();
        let msg_upper = original_prompt_parsed.to_uppercase();
        if msg_upper.contains("TOOL_TESTER") { required.insert("TOOL_TESTER".to_string()); }
        if msg_upper.contains("TOOL_VISION_EVALUATOR") { required.insert("TOOL_VISION_EVALUATOR".to_string()); }
        if msg_upper.contains("TOOL_AUDITOR") { required.insert("TOOL_AUDITOR".to_string()); }
        required
    };
    let mut mandatory_tools_executed: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut forced_next_tool: Option<(String, String)> = None;
    let mut intercept_consecutive: u32 = 0; // track consecutive LLM disobedience
    let mut ask_user_consecutive: u32 = 0; // track consecutive user questions to prevent stalling loops
    // FIX-B3: Per-file patch failure counter. When a file accumulates 3+ PATCH_FAILs in a row,
    // escalate to full-file overwrite mode instead of retrying failed patches forever.
    let mut patch_fail_counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    // ── Semantic Error Loop Detector — ring buffer of last 7 terminal output hashes ──
    // Populated whenever a TOOL_TERMINAL command produces an error. If identical errors repeat,
    // sanity_monitor escalates to RED and forces TOOL_THINK.
    let mut last_error_hashes: std::collections::VecDeque<u64> = std::collections::VecDeque::with_capacity(7);

    let mut retry_tracker = crate::core::error_classifier::RetryTracker::new();
    let mut agent_workspace = chronos_vfs::workspace::AgentWorkspace::<chronos_vfs::aura_bridge::AuraAstNode>::new(1_048_576).unwrap();
    // ── Context Window Tiered Monitor (Devin 2.0 / OSS 2025 Pattern) ────────
    let context_monitor = crate::core::context_monitor::ContextMonitor::new(6000, &original_prompt_parsed);

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
        MissionType::Analysis     => "🔍 ANÁLISIS",
        MissionType::Construction => "🏗️ CONSTRUCCIÓN",
        MissionType::Refactor     => "♻️ REFACTORING",
        MissionType::Debug        => "🐛 DEBUG",
        MissionType::Execution    => "⚡ EJECUCIÓN/VERIFICACIÓN",
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
    let is_continuation_command = {
        let msg_trim = user_message.trim().to_lowercase();
        msg_trim == "continua" || msg_trim == "continuar" || msg_trim == "continue" || msg_trim == "sigue" || msg_trim == "adelante"
    };

    // ── Fase 1: Register workspace in global index for auto-resume ────────────
    crate::core::mission_persist::register_workspace(&workspace_path);

    // ── Fase 3: Inject episodic memory context ────────────────────────────────
    let episode_context = crate::core::episodic_memory::get_episode_context(3);
    if !episode_context.is_empty() {
        current_context.push_str(&episode_context);
    }

    // ── Arquitectura Cognitiva v4: Inyectar Contexto de Experiencias Previas ──
    let exp_context = crate::core::experience::ExperienceStore::build_experience_context(&original_prompt_parsed, "");
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
        user_message = journal.objetivo.clone();
        original_prompt_parsed = journal.objetivo.clone();
        journal.interrupted = true; // Signals restoration block below
    } else {
        // Any new user prompt (not an explicit continuation) resets the phase plan for the new objective
        journal.plan_generado = false;
        journal.fases.clear();
        journal.fase_actual = 0;
        journal.objetivo = user_message.clone();
    }

    // Ensure internal files in workspace are hidden on Windows
    crate::core::hide_workspace_internal_files(&workspace_path);

    // ── MissionRuntime: cognitive governor built AFTER objective is resolved ──
    // This ensures the runtime contract has the correct objective for continuations.
    let mut runtime = crate::core::mission_runtime::MissionRuntime::new(
        &workspace_path,
        &original_prompt_parsed,  // <-- now guaranteed to be the resolved objective
        50,
    );

    // Initial contract criteria: every mission must have explicit deliverables & validation
    if runtime.contract.acceptance_criteria.is_empty() {
        runtime.contract.add_criterion(
            "AC-DELIVERABLES",
            "Implementación y generación de entregables solicitados en el workspace",
            crate::core::mission_contract::VerificationMethod::ManualReview,
            true,
        );
        runtime.contract.add_criterion(
            "AC-VALIDATION",
            "Validación de sintaxis, consistencia y pruebas funcionales del workspace",
            crate::core::mission_contract::VerificationMethod::TestPassed,
            true,
        );
    }

    // ── FINAL-6: Register all tool executors with ToolRegistry ────────────────
    // Must happen BEFORE the mission loop. ToolRegistry is now the sole dispatch
    // authority — execute_action() resolves which code runs via registry, not ad-hoc.
    {
        use std::sync::Arc;

        // TOOL_TERMINAL: core command execution (guards/redirect logic stay in match arm)
        {
            let ws = workspace_path.clone();
            let _ = runtime.tool_registry.register("TOOL_TERMINAL", Arc::new(move |args| {
                let workspace = ws.clone();
                Box::pin(async move {
                    let cmd = args["comando"].as_str().unwrap_or("").to_string();
                    execute_terminal_command(&workspace, &cmd).await
                        .map_err(|e| e.to_string())
                })
            }));
        }

        // TOOL_PROGRAMMER: complex retry+LLM logic handled in match arm for now.
        // Registered so ToolRegistry validates the name; extraction to standalone fn is next.
        {
            let _ = runtime.tool_registry.register("TOOL_PROGRAMMER", Arc::new(|_args| {
                Box::pin(async {
                    Err("TOOL_PROGRAMMER: dispatched via match arm (extraction pending)".to_string())
                })
            }));
        }

        // TOOL_TESTER: same — match arm handles complex compilation+verification logic.
        {
            let _ = runtime.tool_registry.register("TOOL_TESTER", Arc::new(|_args| {
                Box::pin(async {
                    Err("TOOL_TESTER: dispatched via match arm (extraction pending)".to_string())
                })
            }));
        }

        // TOOL_FINISH: CompletionGate is the authority (match arm validates and signals)
        {
            let _ = runtime.tool_registry.register("TOOL_FINISH", Arc::new(|_args| {
                Box::pin(async { Ok("FINISH_SIGNALED".to_string()) })
            }));
        }

        // Remaining tools: stub dispatchers satisfy registry validation.
        // Match arm handles actual execution for each.
        for tool in &[
            "TOOL_ENV_MANAGER", "TOOL_MAPPER", "TOOL_AST_INJECT",
            "TOOL_CONTAINER", "TOOL_WORKSPACE_MANAGER",
            "TOOL_BACKGROUND_START", "TOOL_BACKGROUND_QUERY",
            "TOOL_BROWSE", "TOOL_GIT", "TOOL_THINK",
            "TOOL_AUDITOR", "TOOL_VISION_EVALUATOR", "TOOL_ASK_USER",
            "TOOL_READ_FILE", "TOOL_BACKGROUND_READ", "TOOL_BACKGROUND_KILL",
            "TOOL_ASSET_MANAGER", "TOOL_WEB_SCRAPER", "TOOL_LEARN",
        ] {
            let tool_name = tool.to_string();
            let _ = runtime.tool_registry.register(tool, Arc::new(move |_args| {
                let name = tool_name.clone();
                Box::pin(async move { Ok(format!("{}_REGISTERED", name)) })
            }));
        }
    }

    // ── AL-v1: Adaptive Learning Setup ───────────────────────────────────────
    let al_project_profile = crate::core::project_profile::ProjectProfile::detect(&workspace_path);
    let _ = runtime.observe_world();
    let al_fingerprint = crate::core::learning::FingerprintBuilder::from_mission_with_world(
        &runtime.contract, &al_project_profile, runtime.world.as_ref(),
    );
    let al_persistence = crate::core::learning::LearningPersistence::new();
    let al_store: crate::core::learning::SharedExperienceStore = {
        let s = al_persistence.load_experiences(500);
        std::sync::Arc::new(tokio::sync::RwLock::new(s))
    };
    let al_model_stats = al_persistence.load_model_stats();
    let al_strategy_stats = al_persistence.load_strategy_stats();
    let al_router = crate::core::learning::AdaptiveRouter::new(
        al_model_stats, al_strategy_stats,
        al_store.clone(), &runtime.mission_id,
    );
    let al_recommendation = al_router.recommend_with_context(
        &al_fingerprint,
        None, // No StateSignature yet at mission start
        Some(runtime.budget_remaining()), // remaining budget from runtime
        &available_models,
    ).await;
    let al_engine = crate::core::learning::LearningEngine::with_store(al_store.clone());
    let al_start_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64).unwrap_or(0);
    emit_event(&app_handle, 0,
        &format!("[ADAPTIVE LEARNING] Estrategia recomendada: {:?} | Modelo: {} (confianza: {:.0}%)",
            al_recommendation.strategy, al_recommendation.model, al_recommendation.confidence * 100.0),
        "INFO");

    journal.workspace_path = workspace_path.clone();
    journal.status = "EN_PROGRESO".to_string();
    journal.herramientas_usadas.clear();
    journal.archivos_tocados.clear();
if let Err(e) = crate::core::session_journal::save_journal(&workspace_path, &journal) { emit_event(&app_handle, runtime.current_step(), &format!("[CHECKPOINT FAILED] {}", e), "FATAL"); }
    emit_event(&app_handle, 0, "[DIARIO] Misión registrada en diario de sesión.", "INFO");
    emit_event(&app_handle, 0, &format!("[MISIÓN] Tipo clasificado: {} — El agente operará en modo apropiado.", mission_label), "INFO");
    
    // ─── COMMAND TRAIL (registro estructurado de pasos) ───
    use crate::core::command_trail::CommandTrail;
    let mut command_trail = CommandTrail::load_or_new(&workspace_path, &original_prompt_parsed);
    command_trail.save(&workspace_path);
    
    // ─── AUTO-GENERATE RUNNERS (test, build, dev, lint) ───
    // Detectar lenguaje y generar scripts de ejecución internamente en .aura/runtime/runners
    let runners_generated = generate_project_runners(&workspace_path, &original_prompt_parsed).await;
    if !runners_generated.is_empty() {
        emit_event(&app_handle, 0, &format!("🏃 Runners internos activos en .aura/runtime/runners: {}", runners_generated.iter().map(|p| p.file_name().unwrap().to_string_lossy()).collect::<Vec<_>>().join(", ")), "SUCCESS");
    }
    
    // =======================================================
    // PESP v2 — Generación del Plan de Fases (Paso 0)
    // =======================================================
    if !journal.plan_generado && (mission_type == MissionType::Construction || mission_type == MissionType::Refactor) {
        emit_event(&app_handle, 0, "🏗️ [ARQUITECTO DE FASES] Analizando tarea para dividirla en fases...", "PLANNING");
        let model_for_planner = &orchestrator_model;
        let fases = crate::llm::phase_planner::generate_phase_plan(&original_prompt_parsed, model_for_planner).await;
        
        journal.fases = fases.clone();
        journal.fase_actual = 0;
        journal.plan_generado = true;
        if let Err(e) = crate::core::session_journal::save_journal(&workspace_path, &journal) { emit_event(&app_handle, runtime.current_step(), &format!("[CHECKPOINT FAILED] {}", e), "FATAL"); }
        
        let plan_desc = fases.iter().map(|f| format!("Fase {}: {}", f.numero, f.descripcion)).collect::<Vec<_>>().join(" | ");
        emit_event(&app_handle, 0, &format!("ðŸ—ºï¸  Plan generado: {}", plan_desc), "SUCCESS");
    }
    let mut _task_complexity = crate::llm::router::TaskContext { task_type: crate::llm::router::TaskType::GeneralCode, language: None };
    
    
    // ── SESSION PERSISTENCE (RESTORE IF INTERRUPTED) ──
    if journal.interrupted {
        if let Some(saved_role) = &journal.fsm_role {
            match saved_role.as_str() {
                "Executor" => current_role = AgentRole::Executor,
                "Critic"   => current_role = AgentRole::Critic,
                _          => current_role = AgentRole::Planner,
            }
        }
        // H-2: Runtime controls its own state restoration
        if journal.fsm_step > 0 {
            runtime.restore_step(journal.fsm_step);
        }
        // Runtime budget is the SOLE authority — reset to 50 steps for continuation
        runtime.budget = crate::core::step_budget::StepBudget::new(50);
        journal.interrupted = false;
        if let Err(e) = crate::core::session_journal::save_journal(&workspace_path, &journal) { emit_event(&app_handle, runtime.current_step(), &format!("[CHECKPOINT FAILED] {}", e), "FATAL"); }
        if let Some(ctx) = &journal.fsm_context {
            if !ctx.is_empty() {
                current_context = ctx.clone();
            }
        }
        emit_event(&app_handle, runtime.current_step(), &format!("[SESSION RESTORED] Misión retomada exitosamente desde el paso {}. Presupuesto activo extendido a {} pasos.", runtime.current_step(), runtime.budget_remaining()), "SUCCESS");
    }

    while !runtime.is_budget_exhausted() {
        // ── H-3: Runtime Integrity Guard — check at each cognitive cycle ──
        let invariant_violations = runtime.check_invariants();
        if !invariant_violations.is_empty() {
            emit_event(&app_handle, runtime.current_step(),
                &format!("[INVARIANT] Violaciones detectadas: {:?}", invariant_violations),
                "WARNING");
        }

        // ── Cancellation Check: Interrupción inmediata solicitada por el usuario ──
        if crate::llm::is_agent_cancelled() {
            // H-5: Cancellation is a control event, NOT a tool failure
            let cancel_obs = crate::core::observation::Observation::cancelled(
                "AGENT", "Misión detenida por el usuario"
            );
            runtime.record_observation(&cancel_obs);
            emit_event(&app_handle, runtime.current_step(), "🛑 [CANCELADO] Misión detenida por el usuario.", "WARNING");
            journal.interrupted = true;
            journal.status = "INTERRUMPIDO".to_string();
            journal.fsm_step = runtime.current_step(); // sync step before saving
            if let Err(e) = crate::core::session_journal::save_journal(&workspace_path, &journal) { emit_event(&app_handle, runtime.current_step(), &format!("[CHECKPOINT FAILED] {}", e), "FATAL"); }
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
        ) { emit_event(&app_handle, runtime.current_step(), &e, "FATAL"); }

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
            if let Some((hint, forced_tool_opt)) = crate::core::sanity_monitor::build_correction_hint(&report) {
                current_context.push_str(&hint);
                emit_event(&app_handle, runtime.current_step(), &format!("[CORDURA] {}", report.recommendation), "WARNING");
                // RED level: mechanically force the tool — don't just inject text
                if let Some(forced_tool_name) = forced_tool_opt {
                    if forced_next_tool.is_none() {
                        forced_next_tool = Some((
                            forced_tool_name.clone(),
                            format!("[CORDURA-RED] Forzando {} — loop o error semántico detectado.", forced_tool_name),
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
                                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                                if !name.starts_with('.') {
                                    created_files.push(name);
                                }
                            }
                        }
                    }

                    emit_event(&app_handle, runtime.current_step(), "✅ Misión completada. Todos los entregables validados.", "SUCCESS");

                    let al_elapsed = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64).unwrap_or(0)
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
                    let _ = al_engine.record_outcome(
                        al_fingerprint.clone(),
                        orchestrator_model.clone(),
                        al_recommendation.strategy.clone(),
                        al_result,
                        al_recommendation.confidence,
                        runtime.mission_id.clone(),
                        None,
                    ).await;

                    let final_res = FinalResponse {
                        status: "FINISH".to_string(),
                        respuesta_conversacional: format!(
                            "### 🛡︠ Misión Completada con Éxito\n\n\
                            Se han implementado y validado todos los componentes del proyecto:\n\
                            {}\n\n\
                            Todos los archivos pasaron las pruebas de compilación y verificación al 100%.",
                            created_files.iter().map(|f| format!("- `{}`", f)).collect::<Vec<_>>().join("\n")
                        ),
                    };
                    return Ok(serde_json::to_string(&final_res).unwrap());
                } else if deliverables_ok && !completion_ok {
                    // Files exist but CompletionGate not satisfied   honest incomplete status
                    emit_event(&app_handle, runtime.current_step(), "⠸︠ Presupuesto agotado   entregables presentes pero criterios de misión incompletos.", "WARNING");

                    let al_elapsed = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64).unwrap_or(0)
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
                    let _ = al_engine.record_outcome(
                        al_fingerprint.clone(),
                        orchestrator_model.clone(),
                        al_recommendation.strategy.clone(),
                        al_result,
                        al_recommendation.confidence,
                        runtime.mission_id.clone(),
                        None,
                    ).await;

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
            if let Err(e) = crate::core::session_journal::save_journal(&workspace_path, &journal) { emit_event(&app_handle, runtime.current_step(), &format!("[CHECKPOINT FAILED] {}", e), "FATAL"); }

            let mut workspace_files: Vec<String> = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&workspace_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
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
                status: "FINISH".to_string(),
                respuesta_conversacional: pause_msg,
            };
            return Ok(serde_json::to_string(&final_res).unwrap());
        }

        // ── PESP: Inject micro-meta progress status into context ─────────────
        // Tells the LLM exactly where it is in the global project plan every turn.

        // ── PESP v2: Inject phase progress status into context ─────────────
        // Tells the LLM exactly where it is in the global project plan every turn.
        if !journal.fases.is_empty() {
            let total = journal.fases.len();
            let progress: String = journal.fases.iter().enumerate().map(|(i, f)| {
                let icon = match f.estado.as_str() {
                    "COMPLETADA"  => "✅",
                    "EN_PROGRESO" => "🔄",
                    "FALLIDA"     => "❌",
                    _             => "⏳",
                };
                format!("  {} [{}/{}] {} → {}", icon, i + 1, total, f.descripcion, f.estado)
            }).collect::<Vec<_>>().join("\n");
            let current_f = journal.fases.get(journal.fase_actual)
                .map(|f| format!("(Fase {}/{}) {}\n    Criterio de Éxito: {}", f.numero, total, f.descripcion, f.criterio_de_exito))
                .unwrap_or_else(|| "(todas completadas)".to_string());
            let pesp_status = format!(
                "[ESTADO DE FASES DEL PROYECTO — PESP PROTOCOL]\n{}\n\n📍 FASE ACTUAL EN EJECUCIÓN:\n{}\n\n",
                progress,
                current_f
            );
            let existing = current_context.clone();
            current_context = format!("{}\n{}", pesp_status, existing);
        } else if !journal.micro_metas.is_empty() {
            let total = journal.micro_metas.len();
            let progress: String = journal.micro_metas.iter().enumerate().map(|(i, mm)| {
                let icon = match mm.estado.as_str() {
                    "VERIFICADA"  => "✅",
                    "COMPLETADA"  => "✅",
                    "EN_PROGRESO" => "🔄",
                    _             => "⏳",
                };
                format!("  {} [{}/{}] {} → {}", icon, i + 1, total, mm.descripcion, mm.estado)
            }).collect::<Vec<_>>().join("\n");
            let current_mm = journal.micro_metas.get(journal.micro_meta_actual)
                .map(|mm| mm.descripcion.clone())
                .unwrap_or_else(|| "(todas completadas)".to_string());
            let pesp_status = format!(
                "[ESTADO DE MICRO-METAS DEL PROYECTO — PESP PROTOCOL]\n{}\nMICRO-META ACTUAL: [{}/{}] {}\n\n",
                progress,
                journal.micro_meta_actual + 1,
                total,
                current_mm
            );
            // Prepend to context so it's always at the top
            let existing = current_context.clone();
            current_context = format!("{}{}", pesp_status, existing);
        }
        // ── Context Window Tiered Monitor & Intelligent Compaction (Devin 2.0 / OSS 2025 Pattern) ──
        let (fill_pct, ctx_status) = context_monitor.status(current_context.len());
        if context_monitor.should_compact(current_context.len()) {
            emit_event(&app_handle, runtime.current_step(), &format!("[MEMORIA] Compactando ventana de contexto ({:.0}% uso) preservando Objetivo Inmutable...", fill_pct * 100.0), "INFO");
            current_context = context_monitor.compact_context(&current_context);
            emit_event(&app_handle, runtime.current_step(), "[MEMORIA] Contexto compactado exitosamente sin pérdida del objetivo.", "SUCCESS");
        } else if ctx_status == crate::core::context_monitor::ContextStatus::ApproachingLimit {
            emit_event(&app_handle, runtime.current_step(), &format!("[MEMORIA] Ventana al {:.0}% de capacidad — operando con normalidad.", fill_pct * 100.0), "INFO");
        }


        let mut forced_override: Option<(String, String)> = None;
        if let Some((forced, override_msg)) = forced_next_tool.take() {
            let intercept_log = format!("[SISTEMA INTERCEPTO] En el turno anterior se decidió forzarte a usar: {}. Razón: {}", forced, override_msg);
            current_context.push_str(&format!("{}\n\n", intercept_log));
            forced_override = Some((forced, override_msg));
        }
        
        let mut extra_prompt = String::new();
        if let Some((forced, _)) = &forced_override {
            extra_prompt = format!("\n\nREGLA ESTRICTA E INQUEBRANTABLE PARA ESTE TURNO:\nDEBES Y TIENES QUE ELEGIR '{}' COMO TU HERRAMIENTA. NO ELIJAS OTRA O EL SISTEMA FALLARÁ. Ignora cualquier otra regla y genera un JSON válido para la herramienta {}.", forced, forced);
        }

        // ── LIVE WORKSPACE SCAN (Always 100% fresh on every step) ──
        let (live_workspace_context, workspace_is_empty) = {
            let mut live_files = Vec::new();
            fn scan_live_files(dir: &std::path::Path, files: &mut Vec<String>, depth: usize) {
                if depth > 5 { return; }
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                        if name.starts_with('.') || name == "node_modules" || name == "__pycache__" || name == "target" { continue; }
                        if path.is_dir() {
                            scan_live_files(&path, files, depth + 1);
                        } else {
                            files.push(path.to_string_lossy().to_string());
                        }
                    }
                }
            }
            scan_live_files(std::path::Path::new(&workspace_path), &mut live_files, 0);
            let is_empty = live_files.is_empty();
            let ctx = if is_empty {
                "El proyecto está completamente vacío. Aún no has creado ningún archivo físico.".to_string()
            } else {
                let repo_map = crate::core::map::generate_repo_map(std::path::Path::new(&workspace_path));
                let relative_files: Vec<String> = live_files.iter()
                    .map(|f| {
                        f.strip_prefix(&workspace_path)
                            .unwrap_or(f)
                            .trim_start_matches(['/', '\\'])
                            .to_string()
                    })
                    .collect();
                format!("{}\n\nARCHIVOS EXISTENTES EN EL WORKSPACE (rutas relativas):\n{}", repo_map, relative_files.join("\n"))
            };
            (ctx, is_empty)
        };

        // --- EVITAR DESBORDAMIENTO DE CONTEXTO (Garantizando Objetivo Inmutable) ---
        if context_monitor.should_compact(current_context.len()) {
            current_context = context_monitor.compact_context(&current_context);
        }

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
        let contract_block = acceptance_contract.as_deref().map(|c| format!("[CONTRATO DE ACEPTACION DEL PLANIFICADOR]\n{}\n", c)).unwrap_or_default();

        let json_schema = format!("Tu respuesta DEBE ser ÚNICAMENTE un objeto JSON (sin markdown, sin texto extra):\n\
            {{\n\
              \"checklist_mental\": \"<tareas cumplidas vs faltantes>\",\n\
              \"herramienta\": \"<NOMBRE_HERRAMIENTA>\",\n\
              \"pensamiento\": \"Razonamiento lógico de tu decisión\",\n\
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
            3. Si el workspace está vacío o no hay archivos creados, tu tarea NO está cumplida: usa TOOL_PROGRAMMER con el nombre de los archivos a crear.
            4. El workspace actual es: {ws}. NUNCA uses rutas absolutas de proyectos anteriores ni de otros directorios.",
            ws = workspace_path);

        let agent_prompt = match current_role {
            // Planner - Enrutamiento Semántico Avanzado (Zero-Hint Routing)
            AgentRole::Planner => format!(
                "[PLANIFICADOR / ZERO-HINT ROUTER]\nObjetivo del Usuario: {}\nWorkspace Actual: {}\n{}\nHistorial de Conversación:\n{}\n\n\
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
                user_message, live_workspace_context, extra_prompt, current_context,
                critic_feedback_block, analysis_fast_path, json_schema
            ),
            // Executor - compressed to <200 tokens
            AgentRole::Executor => format!(
                "[EJECUTOR] Objetivo: {}\nWorkspace: {}\n{}\nHistorial:\n{}\n\nTOOLS PERMITIDOS: TOOL_PROGRAMMER, TOOL_TERMINAL, TOOL_CONTAINER, TOOL_ASSET_MANAGER, TOOL_BACKGROUND_START.\n- TOOL_CONTAINER: Comando = 'run/exec/stop/activate_env image/id'. Úsalo para sandbox, testing en Docker/Podman o para activar entornos virtuales (venv, nvm, cargo).\n- TOOL_ENV_MANAGER: *SOLO* para instalar binarios scoop.\nREGLAS: ANTI-STUB (no pass/TODO/funciones vacias). Si el workspace está vacío o la tarea no está terminada, TU PRIMERA ACCION OBLIGATORIA ES TOOL_PROGRAMMER con el nombre de los archivos a crear en 'archivos_a_editar' (NUNCA [] vacío). Genera código modular, atómico y preferiblemente archivo por archivo para evitar respuestas gigantes. No uses TOOL_TESTER ni TOOL_FINISH. PROHIBIDO usar TOOL_ASK_USER (eres el ejecutor: escribe código e implementa directamente).\n[REGLA SCRIPTS DE PRUEBA]: Al crear/modificar verify_*.py o test_*.py: 1) Valida semántica (regex o checks independientes de atributos) sin asumir orden rígido en HTML. 2) Comprueba booleanos o 'PASS' correctamente y retorna sys.exit(0) si pasan. 3) Si un test falla, eres 100% autónomo para auto-depurarlo con TOOL_PROGRAMMER. 4) En Python NUNCA uses llaves '}}' para cerrar bloques y escapa comillas internas con \\\" para evitar SyntaxError.\nEJEMPLOS TOOL_TERMINAL: Para 'npm install' usa TOOL_TERMINAL con comando='npm install'. NUNCA inventes herramientas como 'NPM INSTALL'.\n\n{}{}",
                user_message, live_workspace_context, extra_prompt, current_context,
                critic_feedback_block, json_schema
            ),
            // Critic - compressed to <200 tokens
            AgentRole::Critic => format!(
                "[CRITICO] Objetivo: {}\nWorkspace: {}\n{}\nHistorial:\n{}\n\nTOOLS PERMITIDOS: TOOL_TESTER, TOOL_TERMINAL, TOOL_VISION_EVALUATOR, TOOL_FINISH, TOOL_ASK_USER.\nREGLAS: Usa TOOL_TESTER/TOOL_TERMINAL para validar. Si hay errores describelos con precisión. Solo TOOL_FINISH si todo pasa al 100%%. PROHIBIDO usar TOOL_ASK_USER para fallos de tests o código (transfiere a TOOL_PROGRAMMER para corregir el código o el test).\n[REGLA FRONTEND]: Si el proyecto contiene archivos HTML (ej. index.html, cyber_sentinel.html, etc.), es frontend. NO uses `node script.js` sobre archivos HTML. Si existe un script de prueba de verificación (ej. verify_*.py o test_*.py), ejecútalo con TOOL_TERMINAL; si pasa al 100%% o no hay tests pendientes, usa TOOL_FINISH.\n\n{}{}",
                user_message, live_workspace_context, extra_prompt, current_context,
                contract_block, json_schema
            ),
        };

        // ── Context Sanitizer: strip any reference to foreign workspaces ────────
        // Prevents the LLM from re-learning stale workspace paths from its own
        // history (e.g. previous sessions or foreign directory paths).
        current_context = {
            let stale_markers = [
                "proxy-stack-windows",
                "proxy-stack",
                "\\proxy-",
                "/proxy-",
            ];
            let mut ctx = current_context.clone();
            for marker in &stale_markers {
                if ctx.contains(marker) {
                    ctx = ctx.lines()
                        .filter(|line| !line.contains(marker))
                        .collect::<Vec<_>>()
                        .join("\n");
                }
            }
            // Strip foreign absolute paths if mentioning other scratch workspaces
            let ws_clean = workspace_path.trim().replace('/', "\\");
            if !ws_clean.is_empty() {
                ctx = ctx.lines()
                    .filter(|line| {
                        if let Some(pos) = line.find("scratch\\") {
                            let after = &line[pos + 8..];
                            let foreign_folder = after.split(&['\\', '/', ' ', '"', '\'', '`'][..]).next().unwrap_or("");
                            if !foreign_folder.is_empty() && !ws_clean.contains(foreign_folder) && foreign_folder != "aura sentinel" {
                                return false;
                            }
                        }
                        true
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
            }
            ctx
        };

        // El modelo del orquestador respeta la selección global del usuario (ya resuelto en resolve_model_or_fallback)
        let role_label = match current_role {
            AgentRole::Planner  => "🧠 PLANIFICADOR",
            AgentRole::Executor => "⚙️ EJECUTOR",
            AgentRole::Critic   => "🔬 CRÍTICO",
        };
        emit_event(&app_handle, runtime.current_step(), &format!("[{}] Pensando con {}...", role_label, orchestrator_model), "PLANNING");

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
            if let Some((hint2, forced_tool_opt2)) = crate::core::sanity_monitor::build_correction_hint(&sanity) {
                current_context.push_str(&hint2);
                emit_event(&app_handle, runtime.current_step(), &format!("[⚕️ CORDURA {}] {}", sanity.level, &sanity.recommendation.chars().take(80).collect::<String>()), "WARNING");
                if let Some(forced_tool_name2) = forced_tool_opt2 {
                    if forced_next_tool.is_none() {
                        forced_next_tool = Some((
                            forced_tool_name2.clone(),
                            format!("[CORDURA-RED] Forzando {} — loop o error semántico detectado.", forced_tool_name2),
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
            ) { emit_event(&app_handle, runtime.current_step(), &e, "FATAL"); }
        }

        let mut agent_res = match call_ollama(&orchestrator_model, &agent_prompt).await {
            Ok(res) => res,
            Err(e) => {
                emit_event(&app_handle, runtime.current_step(), &format!("Error de conexión: {}", e), "ERROR");
                return Err(e);
            }
        };
        
        // Limpiar JSON
        agent_res = agent_res.trim().to_string();
        if agent_res.starts_with("```json") { agent_res = agent_res.trim_start_matches("```json").to_string(); }
        else if agent_res.starts_with("```") { agent_res = agent_res.trim_start_matches("```").to_string(); }
        if agent_res.ends_with("```") { agent_res = agent_res.trim_end_matches("```").to_string(); }
        agent_res = agent_res.trim().to_string();
        
        let clean_agent_res = strip_think_tags(agent_res.clone());
        let raw_value: serde_json::Value = match serde_json::from_str(&clean_agent_res) {
            Ok(v) => {
                println!("LLM RAW RESPONSE: {}", agent_res);
                json_error_count = 0; // Reset error count on success
                v
            },
            Err(e) => {
                println!("LLM RAW RESPONSE ERROR: {}", agent_res);
                json_error_count += 1;
                if json_error_count >= 5 {
                    emit_event(&app_handle, runtime.current_step(), &format!("Error parseando decisión ({}). Máximos reintentos (5) alcanzados. Abortando bucle.", e), "ERROR");
                    let final_res = FinalResponse { status: "ERROR".to_string(), respuesta_conversacional: "Fallo crítico persistente en la estructura JSON del planificador.".to_string() };
                    crate::llm::router::record_model_result(&orchestrator_model, &crate::llm::router::TaskType::Orchestrator, final_res.status == "FINISH", runtime.current_step());
                    return Ok(serde_json::to_string(&final_res).unwrap());
                } else {
                    emit_event(&app_handle, runtime.current_step(), &format!("Error de sintaxis JSON (intento {}/5). Reintentando...", json_error_count), "WARNING");
                    current_context.push_str(&format!("[SISTEMA INTERNO] Tu respuesta anterior no era un JSON válido. Error: {}. Genera SOLO un objeto JSON estrictamente válido según la estructura requerida, sin texto adicional antes o después del JSON.\n\n", e));
                    continue;
                }
            }
        };
        let checklist = raw_value.get("checklist_mental").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let mut tool = raw_value.get("herramienta").and_then(|v| v.as_str()).unwrap_or("UNKNOWN").to_uppercase();
        let pensamiento = raw_value.get("pensamiento").and_then(|v| v.as_str()).unwrap_or("Sin pensamiento").to_string();
        let mut comando = raw_value.get("comando").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let task_id = raw_value.get("task_id").and_then(|v| v.as_str()).unwrap_or("default_task").to_string();
        let mut respuesta_conv = raw_value.get("respuesta_conversacional").and_then(|v| v.as_str()).unwrap_or("").to_string();

        // Si el modelo solo quiere responder conversacionalmente (ej. "herramienta": null o "null"),
        // y adjuntó una respuesta para el usuario, enrutar limpiamente como TOOL_FINISH para no entrar en bucle de error.
        if (tool == "UNKNOWN" || tool == "NULL" || tool == "NONE" || tool.is_empty()) && !respuesta_conv.trim().is_empty() {
            tool = "TOOL_FINISH".to_string();
        }

        // Auto-extract command if tool is TOOL_TERMINAL and comando is empty
        if tool == "TOOL_TERMINAL" && comando.trim().is_empty() {
            if let Some(auto_cmd) = extract_command_from_thought(&pensamiento, &workspace_path) {
                emit_event(&app_handle, runtime.current_step(), &format!("[AUTO-EXTRACT] Comando recuperado del pensamiento: {}", auto_cmd), "INFO");
                comando = auto_cmd;
            }
        }

        // ── MissionRuntime: step tracking + stall detection (single source of truth) ──
        runtime.record_step();
        if let Some(stall) = runtime.should_stall_recover(4) {
            let stall_msg = format!("[STALL DETECTOR] {:?} detectado. Forzando verificación real del filesystem.", stall);
            emit_event(&app_handle, runtime.current_step(), &stall_msg, "WARNING");
            // RepeatedTool stall: force a dir command to anchor LLM to filesystem reality
            if matches!(stall, crate::core::stall_detector::StallType::RepeatedTool)
                && forced_next_tool.is_none()
            {
                forced_next_tool = Some((
                    "TOOL_TERMINAL".to_string(),
                    format!(
                        "[STALL RECOVERY] Verificando estado real del workspace. {}",
                        "Ejecuta 'dir' y ajusta tu plan según los archivos que realmente existen."
                    ),
                ));
                current_context.push_str(
                    "[CORRECCIÓN FORZADA]: Estás en un bucle. Verifica los archivos existentes con 'dir' antes de continuar.\n\n"
                );
            }
        }

        // ── Arquitectura Cognitiva v4: Validación previa de esquema (P1) ──
        if let crate::core::schema_validator::SchemaValidationResult::Invalid(schema_err) = 
            crate::core::schema_validator::SchemaValidator::validate_tool_payload(&tool, &raw_value) {
            emit_event(&app_handle, runtime.current_step(), &format!("[SCHEMA ERROR] {}", schema_err), "WARNING");
            current_context.push_str(&format!(
                "[VALIDACIÓN DE ESQUEMA FALLIDA]: {}\nCorrige los argumentos del objeto JSON para la herramienta '{}'.\n\n",
                schema_err, tool
            ));            continue;
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
                emit_event(&app_handle, runtime.current_step(),
                    &format!("[INTERCEPT {}/3] LLM desobedeció orden de usar {}", intercept_consecutive, forced),
                    "WARNING");

                // FIX-A1: Never give up — if model ignores the forced tool 3 times in a row,
                // HARD-EXECUTE the forced action directly without asking the LLM again.
                // This is the only reliable way to break a loop with a stubborn small model.
                if intercept_consecutive >= 3 {
                    emit_event(&app_handle, runtime.current_step(),
                        "[INTERCEPT] Modelo ignoró la orden 3 veces. Ejecutando acción forzada directamente...", "WARNING");

                    // Determine what to hard-execute based on the forced tool
                    let forced_cmd_to_run = if forced == "TOOL_TERMINAL" {
                        // Run the command from the override_msg if it looks like a shell command,
                        // otherwise default to a safe directory listing
                        if override_msg.contains("type ") || override_msg.contains("dir ") || override_msg.contains("python ") || override_msg.contains("cargo ") {
                            // Extract the command portion after common prefixes
                            let cmd_part = override_msg
                                .split("ejecutar '").nth(1).and_then(|s| s.split('\'').next())
                                .or_else(|| override_msg.split("comando: '").nth(1).and_then(|s| s.split('\'').next()))
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
                            expected_effect: "Interceptor auto-exec after persistent loop".to_string(),
                            risk: crate::core::policy::PolicyEngine::classify_terminal_command(&forced_cmd_to_run),
                        };

                        if let Err(auth_err) = runtime.authorize_action(&intercept_proposal) {
                            current_context.push_str(&format!(
                                "[INTERCEPTOR AUTO-EXEC BLOQUEADO]: {}\n\n",
                                auth_err
                            ));
                            emit_event(&app_handle, runtime.current_step(),
                                &format!("[INTERCEPTOR AUTO-EXEC BLOQUEADO] {}", auth_err),
                                "ERROR");
                        } else {
                            // Execute authorized action
                            match execute_terminal_command(&workspace_path, &forced_cmd_to_run).await {
                                Ok(output) => {
                                    let out_len = output.len();
                                    let digest = if out_len > 3000 { &output[..3000] } else { &output[..] };
                                    let auto_msg = format!(
                                        "[INTERCEPTOR AUTO-EXEC] Ejecuté '{}' bajo autorización de runtime.\nResultado:\n{}\n\n",
                                        forced_cmd_to_run, digest
                                    );
                                    current_context.push_str(&auto_msg);
                                    emit_event(&app_handle, runtime.current_step(),
                                        &format!("[INTERCEPTOR AUTO-EXEC] Ejecución directa completada: {} chars", out_len),
                                        "SUCCESS");
                                }
                                Err(e) => {
                                    current_context.push_str(&format!(
                                        "[INTERCEPTOR AUTO-EXEC] Intenté ejecutar '{}' pero falló: {}\n\n",
                                        forced_cmd_to_run, e
                                    ));
                                }
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
                }                continue;
            } else {
                // LLM obeyed — reset counter
                intercept_consecutive = 0;
            }
        }

        // ── ROLE HARD LOCKS (FSM ENFORCEMENT) ─────────────────────────────────
        let is_forced_and_obeyed = forced_override.as_ref().map_or(false, |(f, _)| f == &tool);
        if !is_forced_and_obeyed && current_role == AgentRole::Planner {
            if tool == "TOOL_PROGRAMMER" || tool == "TOOL_TERMINAL" || tool == "TOOL_BACKGROUND_START" {
                current_role = AgentRole::Executor;
                emit_event(&app_handle, runtime.current_step(), &format!("[FSM] Planificador -> Ejecutor: Transición automática para ejecutar {}.", tool), "INFO");
            } else if ["TOOL_TESTER", "TOOL_BACKGROUND_READ", "TOOL_BACKGROUND_KILL", "TOOL_ENV_MANAGER",
                "TOOL_ASSET_MANAGER", "TOOL_VISION_EVALUATOR", "TOOL_WORKSPACE_MANAGER"].contains(&tool.as_str()) {
                let error_msg = format!(
                    "[ACCESO DENEGADO]: Eres el Planificador. No tienes permiso para usar {}. \
                    Tu rol es SOLO diseñar la arquitectura. \
                    NUNCA borres archivos existentes. \
                    Usa TOOL_THINK para transferir el control al Ejecutor cuando estés listo.",
                    tool
                );
                current_context.push_str(&format!("{}\n\n", error_msg));
                emit_event(&app_handle, runtime.current_step(), &format!("[FSM LOCK] Planificador intentó usar {}", tool), "WARNING");                continue;
            }
        } else if !is_forced_and_obeyed && current_role == AgentRole::Executor {
            if tool == "TOOL_FINISH" {
                current_role = AgentRole::Critic;
                emit_event(&app_handle, runtime.current_step(), "[FSM] EJECUTOR -> CRÍTICO: Implementación concluida. Transfiriendo al Crítico para validación final y cierre.", "INFO");
            } else if ["TOOL_TESTER", "TOOL_VISION_EVALUATOR", "TOOL_MAPPER", "TOOL_AST_INJECT", "TOOL_ASK_USER"].contains(&tool.as_str()) {
                let error_msg = format!("[ACCESO DENEGADO]: Eres el Ejecutor. No tienes permiso para usar {}. Tu rol es escribir código directamente con TOOL_PROGRAMMER o comandos con TOOL_TERMINAL. Prohibido preguntar o pedir aclaraciones en fase de ejecución.", tool);
                current_context.push_str(&format!("{}\n\n", error_msg));
                emit_event(&app_handle, runtime.current_step(), &format!("[FSM LOCK] Ejecutor intentó usar {}", tool), "WARNING");                continue;
            }
        } else if !is_forced_and_obeyed && current_role == AgentRole::Critic {
              if ["TOOL_PROGRAMMER", "TOOL_MAPPER", "TOOL_AST_INJECT"].contains(&tool.as_str()) {
                  // FIX: If the Critic wants to fix code, we gracefully auto-transition to Executor
                  // instead of throwing an angry [ACCESO DENEGADO] and forcing a loop.
                  current_role = AgentRole::Executor;
                  emit_event(&app_handle, runtime.current_step(), &format!("[FSM] Critico -> Ejecutor: Transicion automatica para usar {}.", tool), "INFO");
                  // Let it fall through and execute normally as an Executor!
              }
              critic_fsm_lock_consecutive = 0;
        }
        
        let url = raw_value.get("url_a_investigar").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if respuesta_conv.is_empty() {
            respuesta_conv = raw_value.get("respuesta_conversacional").and_then(|v| v.as_str()).unwrap_or("").to_string();
        }
        
        let mut archivos_vec = Vec::new();
        if let Some(arr) = raw_value.get("archivos_a_editar").and_then(|v| v.as_array()) {
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
                            let inside_workspace = s.starts_with(&workspace_path) ||
                                s.replace('/', "\\").starts_with(&workspace_path) ||
                                s.replace('\\', "/").starts_with(&workspace_path.replace('\\', "/"));
                            if inside_workspace {
                                // Strip workspace prefix → relative path
                                let stripped = s.trim_start_matches(&workspace_path)
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
        
        let mut ast_nodes_vec = Vec::new();
        if let Some(arr) = raw_value.get("ast_nodes").and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(obj) = item.as_object() {
                    let intent = obj.get("intent").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let parent_id = obj.get("parent_id").and_then(|v| v.as_u64()).unwrap_or(0);
                    let opcode = obj.get("opcode").and_then(|v| v.as_u64()).unwrap_or(2) as u8;
                    ast_nodes_vec.push((intent, parent_id, opcode));
                }
            }
        }
        
        if !checklist.is_empty() {
            emit_event(&app_handle, runtime.current_step(), &format!("Checklist Mental: {}", checklist), "PLANNING");
        }

        // FORCED TOOL OVERRIDE REMOVED. Validation happens above.

        emit_event(&app_handle, runtime.current_step(), &format!("Decisión: {} - {}", tool, pensamiento), "DECISION");
        current_context.push_str(&format!("--- PASO {} ---\nChecklist Mental: {}\nDecidiste: {}\nPensamiento: {}\n", runtime.current_step(), checklist, tool, pensamiento));

        // ── Journal: update per-step ─────────────────────────────────────
        crate::core::session_journal::update_journal(
            &mut journal,
            runtime.current_step(),
            &format!("[PASO {}] {} - {}", runtime.current_step(), tool, pensamiento),
            &tool,
            &archivos_vec,
            &workspace_path,
        );
        if let Err(e) = crate::core::session_journal::save_journal(&workspace_path, &journal) { emit_event(&app_handle, runtime.current_step(), &format!("[CHECKPOINT FAILED] {}", e), "FATAL"); }
        
        // Registrar herramienta en el historial del Monitor de Cordura
        tool_history.push(tool.clone());
        if tool_history.len() > 15 {
            tool_history.remove(0);
        }

        // Reset loop counters
        if tool != "TOOL_THINK" { think_consecutive = 0; }
        if tool != "TOOL_PROGRAMMER" { _programmer_consecutive = 0; }
        if tool != "TOOL_AUDITOR" { auditor_consecutive = 0; }
        if tool != "TOOL_MAPPER" { mapper_consecutive = 0; }
        if tool != "TOOL_LEARN" { learn_consecutive = 0; }
        if tool != "TOOL_ASK_USER" { ask_user_consecutive = 0; }
        // THINK↔PROGRAMMER alternation counter: only resets when NEITHER THINK nor PROGRAMMER
        if tool == "TOOL_THINK" || tool == "TOOL_PROGRAMMER" {
            think_programmer_alternation_count += 1;
        } else {
            think_programmer_alternation_count = 0;
        }
        // If alternation hits ≥ 8 steps without any TERMINAL/TESTER/FINISH, force TOOL_TERMINAL
        if think_programmer_alternation_count >= 8 {
            let loop_msg = format!(
                "[SISTEMA INTERNO]: ⚠️ LOOP DETECTADO: {} pasos alternando THINK↔PROGRAMMER sin ejecutar nada en terminal. \
                El modelo debe VERIFICAR su trabajo con comandos reales. FORZANDO TOOL_TERMINAL.",
                think_programmer_alternation_count
            );
            eprintln!("{}", loop_msg);
            emit_event(&app_handle, runtime.current_step(), &loop_msg, "WARNING");
            current_context.push_str(&format!("{}\n\n", loop_msg));
            // Force terminal to run the verification/test script
            if forced_next_tool.is_none() {
                forced_next_tool = Some((
                    "TOOL_TERMINAL".to_string(),
                    "Ejecuta el script de verificación del proyecto o abre el archivo principal para comprobar que existe y tiene contenido.".to_string(),
                ));
            }
            think_programmer_alternation_count = 0; // reset after intervention
        }

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
        };

        if let Err(auth_err) = runtime.authorize_action(&action_proposal) {
            emit_event(&app_handle, runtime.current_step(), &format!("[ACTION REJECTED] {}", auth_err), "ERROR");
            current_context.push_str(&format!("{}\n[ACCIÓN RECHAZADA]: Corrige los parámetros o la herramienta elegida.\n\n", auth_err));
            continue;
        }

        match tool.as_str() {
            "TOOL_TERMINAL" => {
                let cmd_lower = comando.to_lowercase();
                if cmd_lower.contains("http-server") || cmd_lower.contains("npm start") || cmd_lower.contains("npm run dev") || cmd_lower.contains("python -m http.server") || cmd_lower.contains("flask run") || cmd_lower.contains("uvicorn") {
                    let res_msg = "[SISTEMA INTERNO]: Has intentado iniciar un servidor web continuo (http-server, npm start, etc.) usando TOOL_TERMINAL. Esto bloquea la terminal infinitamente y rompe el agente.\nSi el objetivo es probar HTML/JS estático, usa `start index.html` para abrirlo directamente en el navegador sin servidor.\nSi requieres obligatoriamente un backend, DEBES usar TOOL_BACKGROUND_START. [SISTEMA: Redirigiendo automticamente a TOOL_BACKGROUND_START...]";
                    current_context.push_str(&format!("{}\n\n", res_msg));
                    emit_event(&app_handle, runtime.current_step(), "Servidor web bloqueado en TOOL_TERMINAL", "WARNING");
                    programmer_cooldown_hits = 0; forced_next_tool = Some(("TOOL_BACKGROUND_START".to_string(), comando.clone()));
                } else if comando.trim().is_empty() {
                    // Track consecutive empties — after 2, force a specific action
                    let empty_key = "__EMPTY_CMD__".to_string();
                    let empty_count = comandos_ejecutados_historico.iter().filter(|c| *c == &empty_key).count();
                    comandos_ejecutados_historico.insert(empty_key);

                    if empty_count >= 2 && current_role == AgentRole::Critic {
                        // Critic keeps sending empty commands — force VISION_EVALUATOR
                        let msg = "[SISTEMA]: Has enviado TOOL_TERMINAL sin comando 3 veces seguidas. \
                            ACCIÓN FORZADA: Debes usar TOOL_VISION_EVALUATOR ahora para verificar \
                            la UI, o TOOL_FINISH si ya terminaste.";
                        current_context.push_str(&format!("{}\n\n", msg));
                        emit_event(&app_handle, runtime.current_step(), "[SISTEMA] Comando vacío repetido — forzando TOOL_VISION_EVALUATOR", "WARNING");
                        forced_next_tool = Some(("TOOL_VISION_EVALUATOR".to_string(),
                            "Verifica visualmente la UI del proyecto creado".to_string()));
                    } else {
                        let res_msg = format!(
                            "Error: El campo 'comando' está vacío. Debes especificar qué ejecutar. \
                            Ejemplos válidos para este paso: 'start index.html' para abrir el \
                            navegador, 'ls' para listar archivos, 'node script.js' para ejecutar JS. \
                            Intento vacío #{}/3 — al tercero se forzará TOOL_VISION_EVALUATOR.",
                            empty_count + 1
                        );
                        current_context.push_str(&format!("{}\n\n", res_msg));
                        emit_event(&app_handle, runtime.current_step(), &format!("Comando vacío ({}/3)", empty_count + 1), "ERROR");
                    }
                    programmer_cooldown_hits = 0;
                } else if !is_forced_and_obeyed && comandos_ejecutados_historico.contains(&comando) {
                    let res_msg = "[SISTEMA INTERNO]: Bucle detectado. Estás repitiendo exactamente el mismo comando. Si falló anteriormente, usa TOOL_PROGRAMMER o TOOL_AUDITOR para arreglar el código. Si ya tuvo éxito y solo estabas probando, la tarea está lista: usa TOOL_FINISH obligatoriamente.";
                    emit_event(&app_handle, runtime.current_step(), "Comando repetido interceptado", "WARNING");
                    if current_role == AgentRole::Critic {
                        let msg = "[SISTEMA INTERNO]: Bucle de terminal detectado en el Crítico. Estás repitiendo el mismo comando de validación. Debes replantear tu estrategia de validación o solicitar correcciones con TOOL_PROGRAMMER.";
                        emit_event(&app_handle, runtime.current_step(), "[SISTEMA] Bucle de Terminal en Crítico -> Forzando Replan", "WARNING");
                        forced_next_tool = Some(("TOOL_THINK".to_string(), "Bucle detectado. Replantea tu estrategia de validación.".to_string()));
                        current_context.push_str(&format!("{}\n\n", msg));
                    } else if current_role == AgentRole::Executor {
                        // Executor stuck repeating informational commands (dir, ls, etc.)
                        let cmd_lower = comando.to_lowercase();
                        let is_info_cmd = cmd_lower.trim() == "dir"
                            || cmd_lower.trim() == "ls"
                            || cmd_lower.trim() == "ls -la"
                            || cmd_lower.trim() == "dir /b";
                        if is_info_cmd {
                            let msg = "[SISTEMA INTERNO]: El Ejecutor está repitiendo un comando informacional (dir/ls). Esto indica estancamiento (STALL). Reevalúa qué acción falta para completar los criterios.";
                            emit_event(&app_handle, runtime.current_step(), "[SISTEMA] Ejecutor en loop informacional -> Forzando Replan", "WARNING");
                            forced_next_tool = Some(("TOOL_THINK".to_string(), "Estancamiento en comandos de información. Revisa el plan original y continúa trabajando.".to_string()));
                            current_context.push_str(&format!("{}\n\n", msg));
                        } else {
                            current_context.push_str(&format!("{}\n\n", res_msg));
                        }
                    } else {
                        current_context.push_str(&format!("{}\n\n", res_msg));
                    }
                } else {
                    let cmd_lower = comando.to_lowercase();
                    if cmd_lower.contains("http-server") || cmd_lower.contains("npm start") || cmd_lower.contains("npm run dev") || cmd_lower.contains("python -m http.server") || cmd_lower.contains("flask run") || cmd_lower.contains("uvicorn") {
                        let res_msg = "[SISTEMA INTERNO]: Has intentado iniciar un servidor web continuo (http-server, npm start, etc.) usando TOOL_TERMINAL. Esto bloquea la terminal infinitamente y rompe el agente.\nSi el objetivo es probar HTML/JS estático, usa `start index.html` para abrirlo directamente en el navegador sin servidor.\nSi requieres obligatoriamente un backend, DEBES usar TOOL_BACKGROUND_START. [SISTEMA: Redirigiendo automticamente a TOOL_BACKGROUND_START...]";
                        current_context.push_str(&format!("{}\n\n", res_msg));
                        emit_event(&app_handle, runtime.current_step(), "Servidor web bloqueado en TOOL_TERMINAL", "WARNING");
                        programmer_cooldown_hits = 0; forced_next_tool = Some(("TOOL_BACKGROUND_START".to_string(), comando.clone()));
                    } else {
                        programmer_cooldown_hits = 0;
                        comandos_ejecutados_historico.insert(comando.clone());

                        // ── Pre-check: verify the script file exists before running it ─────────
                        let cmd_lower_check = comando.to_lowercase();
                        let script_file = if cmd_lower_check.starts_with("node ") {
                            cmd_lower_check.trim_start_matches("node ")
                                .split_whitespace()
                                .find(|arg| !arg.starts_with('-'))
                        } else if cmd_lower_check.starts_with("python ") || cmd_lower_check.starts_with("python3 ") {
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
                                let script_path = std::path::Path::new(&workspace_path).join(script);
                                if !script_path.exists() {
                                    let warn_msg = format!(
                                        "[SISTEMA]: El archivo '{}' NO existe en el workspace. No puedes ejecutar un archivo que no existe.\n\
                                        Primero crea el archivo con TOOL_PROGRAMMER, o verifica los archivos disponibles con TOOL_TERMINAL (dir).",
                                        script
                                    );
                                    current_context.push_str(&format!("{}\n\n", warn_msg));
                                    emit_event(&app_handle, runtime.current_step(), &format!("[PRE-CHECK] Archivo no encontrado: {}", script), "ERROR");
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
                            current_context.push_str(&format!("Resultado TOOL_TERMINAL Error: {}\n\n", warn_msg));
                            emit_event(&app_handle, runtime.current_step(), "[PRE-CHECK] Comando concatenado rechazado", "ERROR");
                            continue;
                        }

                        // ── P1: Pre-check Cargo.toml exists for cargo commands ─────────────
                        if cmd_lower_check.starts_with("cargo ") {
                            let cargo_toml_path = std::path::Path::new(&workspace_path).join("Cargo.toml");
                            if !cargo_toml_path.exists() {
                                let warn_msg = "[SISTEMA]: PROJECT STRUCTURE INVALID: No se encontró `Cargo.toml`. Debes inicializar el proyecto Rust (ej. `cargo init` o crear el archivo) antes de poder usar comandos de cargo.";
                                current_context.push_str(&format!("Resultado TOOL_TERMINAL Error: {}\n\n", warn_msg));
                                emit_event(&app_handle, runtime.current_step(), "[PRE-CHECK] Cargo.toml faltante", "ERROR");
                                forced_next_tool = Some((
                                    "TOOL_PROGRAMMER".to_string(),
                                    "No existe Cargo.toml. Crea Cargo.toml o inicializa el proyecto antes de usar cargo.".to_string()
                                ));
                                continue;
                            }
                        }

                        emit_event(&app_handle, runtime.current_step(), &format!("Ejecutando en terminal: {}", comando), "ACTION");
                        // ── Route through Runtime Gateway (P0 fix) ───────────────────────────
                        // execute_action() = authorize_action (already passed) + ToolRegistry.dispatch()
                        // The TOOL_TERMINAL executor is registered above and calls execute_terminal_command().
                        let unified_res = match runtime.execute_action(&action_proposal).await {
                            Ok(obs) if obs.status == crate::core::observation::ObservationStatus::Error => Err(obs.payload),
                            Ok(obs) => Ok(obs),
                            Err(e) => Err(e),
                        };
                        match unified_res {
                        Ok(observation) => {
                            // execute_action() already records world snapshots before/after internally
                            let out = observation.payload.clone();
                            let world_hash_before = observation.state_hash_before.unwrap_or(0);
                            let world_hash_after = observation.state_hash_after.unwrap_or(0);
                            runtime.record_tool_call();

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
                            let res_msg = format!("Éxito: {}", digested_out);
                            // ── MissionRuntime: record successful observation with real world data ──
                            {
                                let mut obs = crate::core::observation::Observation::success(
                                    "TOOL_TERMINAL", &digested_out, vec![],
                                );
                                obs.command = Some(comando.clone());
                                obs.state_hash_before = Some(world_hash_before);
                                obs.state_hash_after = Some(world_hash_after);
                                runtime.record_observation(&obs);

                                // P0: Record structured execution evidence bound to state_hash
                                let trimmed_cmd = comando.trim();
                                let _ = runtime.evidence_graph.record_with_hash(
                                    crate::core::evidence::EvidenceKind::CommandExitCode,
                                    "TOOL_TERMINAL",
                                    &format!("{} passes", trimmed_cmd),
                                    "0",
                                    1.0,
                                    runtime.current_step(),
                                    Some(world_hash_after)
                                );

                                let cmd_lower = trimmed_cmd.to_lowercase();
                                if cmd_lower.contains("test") {
                                    let test_claim = if cmd_lower.contains("cargo test") {
                                        "cargo test passes"
                                    } else {
                                        "tests pass"
                                    };
                                    let _ = runtime.evidence_graph.record_with_hash(
                                        crate::core::evidence::EvidenceKind::Test,
                                        "TOOL_TERMINAL",
                                        test_claim,
                                        "0",
                                        1.0,
                                        runtime.current_step(),
                                        Some(world_hash_after)
                                    );
                                }
                            }
                            // ── Silent-success auto-verifier ─────────────────────────────────────
                            // When a script runs successfully but prints nothing to stdout,
                            // the LLM cannot confirm the task is done and loops. Fix: scan the
                            // workspace for recently-modified output files and inject a preview.
                            let is_script_run = {
                                let cl = comando.to_lowercase();
                                // Bug 3 fix: only trigger for python scripts or node scripts
                                // that are NOT browser-JS (browser JS has no require/import of node modules)
                                let is_node = cl.starts_with("node ") || cl == "node app.js" || cl == "node index.js";
                                let is_python = cl.starts_with("python") || cl.starts_with("python3");
                                // For node, skip auto-verifier if it's a browser project (HTML files exist)
                                let is_browser_project = has_html_files(&workspace_path);
                                (is_python) || (is_node && !is_browser_project)
                            };
                            let output_is_empty = out.trim().is_empty() || out.trim().len() < 20;
                            if is_script_run && output_is_empty {
                                let output_extensions = ["json", "txt", "csv", "html", "xml", "log", "md"];
                                let mut found_outputs: Vec<String> = Vec::new();
                                if let Ok(entries) = std::fs::read_dir(&workspace_path) {
                                    for entry in entries.flatten() {
                                        let path = entry.path();
                                        if path.is_file() {
                                            let ext = path.extension()
                                                .and_then(|e| e.to_str())
                                                .unwrap_or("")
                                                .to_lowercase();
                                            if output_extensions.contains(&ext.as_str()) {
                                                // Only files modified in the last 60 seconds
                                                if let Ok(meta) = path.metadata() {
                                                    if let Ok(modified) = meta.modified() {
                                                        if let Ok(elapsed) = modified.elapsed() {
                                                            if elapsed.as_secs() < 60 {
                                                                let fname = path.file_name()
                                                                    .unwrap_or_default()
                                                                    .to_string_lossy()
                                                                    .to_string();
                                                                let content = std::fs::read_to_string(&path)
                                                                    .unwrap_or_default();
                                                                let preview = if content.len() > 800 {
                                                                    format!("{}... (truncado, {} bytes totales)", &content[..800], content.len())
                                                                } else {
                                                                    content.clone()
                                                                };
                                                                found_outputs.push(format!(
                                                                    "📄 ARCHIVO GENERADO: {} ({} bytes)\nContenido:\n{}", 
                                                                    fname, content.len(), preview
                                                                ));
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                if !found_outputs.is_empty() {
                                    let _ = runtime.evidence_graph.record_with_hash(
                                        crate::core::evidence::EvidenceKind::RuntimeCheck,
                                        "TOOL_TERMINAL",
                                        "Script generó archivos de salida verificados",
                                        &format!("{} archivos encontrados", found_outputs.len()),
                                        0.95,
                                        runtime.current_step(),
                                        Some(world_hash_after)
                                    );
                                    current_context.push_str(&format!(
                                        "Resultado: {}✅\n\n[SISTEMA: El script no imprimió salida en consola, PERO generó los siguientes archivos de salida que CONFIRMAN que la tarea fue completada exitosamente:]\n\n{}\n\n[SISTEMA: Los archivos de salida existen y tienen contenido. Tu ÚNICO PASO VàLIDO AHORA es usar 'TOOL_FINISH' para reportarle esto al usuario. ESTà PROHIBIDO volver a ejecutar el script.]\n\n",
                                        res_msg,
                                        found_outputs.join("\n\n")
                                    ));
                                    emit_event(&app_handle, runtime.current_step(), &format!("✅ Script OK   {} archivo(s) de salida generados", found_outputs.len()), "SUCCESS");
                                } else {
                                    let cmd_lower = comando.to_lowercase();
                                    let stdout_lower = res_msg.to_lowercase();
                                    let is_test_cmd = cmd_lower.contains("verify") || cmd_lower.contains("test");
                                    let test_passed = stdout_lower.contains("all checks passed")
                                        || stdout_lower.contains("100%")
                                        || stdout_lower.contains("fully verified")
                                        || stdout_lower.contains("verification passed")
                                        || (stdout_lower.contains("[pass]") && !stdout_lower.contains("[fail]"))
                                        || ((out.contains("0 failed") || out.contains("tests passed") || out.contains("100%")) && !out.contains("FAILED"));

                                    if is_test_cmd && test_passed {
                                        let _ = runtime.evidence_graph.record_with_hash(
                                            crate::core::evidence::EvidenceKind::Test,
                                            "TOOL_TERMINAL",
                                            "Script de verificación pasó al 100%",
                                            "PASSED",
                                            0.99,
                                            runtime.current_step(),
                                            Some(world_hash_after)
                                        );
                                        forced_next_tool = Some((
                                            "TOOL_FINISH".to_string(),
                                            "La verificación pasó al 100%. Genera el reporte final y concluye la tarea.".to_string()
                                        ));
                                        current_context.push_str(&format!(
                                            "Resultado: {}\n\n[SISTEMA: ✅ EL SCRIPT DE VERIFICACIÓN PASÓ AL 100%. Tu ÚNICO PASO OBLIGATORIO AHORA es usar 'TOOL_FINISH' para entregar el reporte final. ESTà PROHIBIDO volver a ejecutar el test.]\n\n",
                                            res_msg
                                        ));
                                    } else {
                                        current_context.push_str(&format!("Resultado: {}\n\n[SISTEMA: El comando en terminal se ejecutó con éxito. Analiza este resultado. Si esto completa el objetivo final del usuario, tu SIGUIENTE PASO OBLIGATORIO es usar 'TOOL_FINISH'. Si aún faltan pasos, continúa. NO uses TOOL_TESTER a menos que el usuario haya pedido pruebas automatizadas.]\n\n", res_msg));
                                    }
                                    emit_event(&app_handle, runtime.current_step(), &res_msg, "SUCCESS");
                                }
                            }
                            last_progress_step = runtime.current_step();
                            // Guardar los cambios hechos por la terminal en Git-Shield
                            let _ = crate::core::create_git_backup(&workspace_path, "Aura-Sentinel: Git-Shield Auto-Backup (Terminal)").await;
                        },
                        Err(err) => {
                            // ── Auto ENV_MANAGER: detect binary-not-found and auto-install ──────────
                            let is_binary_missing = err.contains("is not recognized")
                                || err.contains("not recognized as an internal")
                                || err.contains("command not found")
                                || (err.contains("The term") && err.contains("is not recognized"));

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

                                // ── Step 3: Execute TOOL_ENV_MANAGER as the recovery action ──
                                let binary = comando.split_whitespace().next().unwrap_or(&comando);
                                emit_event(&app_handle, runtime.current_step(),
                                    &format!("[RECOVERY→TOOL_ENV_MANAGER] Binario '{}' no encontrado — ejecutando instalación automática por RecoveryEngine...", binary),
                                    "WARNING");
                                let _ = recovery_dec; // RecoveryEngine already consulted; install proceeds

                                match crate::core::env_manager::install_dependency(binary).await {
                                    Ok(install_msg) => {
                                        // ── Produce Observation for the ENV_MANAGER action ──
                                        let env_obs = crate::core::observation::Observation::success(
                                            "TOOL_ENV_MANAGER",
                                            &install_msg,
                                            vec![],
                                        );
                                        runtime.record_observation(&env_obs);
                                        runtime.record_tool_call();
                                        current_context.push_str(&format!(
                                            "[RECOVERY] TOOL_ENV_MANAGER instaló '{}' automáticamente: {}\n\n\
                                             Tu SIGUIENTE PASO OBLIGATORIO es reintentar el comando que falló: '{}'.\n\n",
                                            binary, install_msg, comando
                                        ));
                                        emit_event(&app_handle, runtime.current_step(),
                                            &format!("Dependencia '{}' instalada. Reintenta el comando.", binary),
                                            "SUCCESS");
                                    },
                                    Err(install_err) => {
                                        // ── Produce error Observation for the failed ENV_MANAGER ──
                                        let env_err_obs = crate::core::observation::Observation::error(
                                            "TOOL_ENV_MANAGER",
                                            &install_err,
                                            None,
                                            false,
                                            Some("Intervención manual requerida para instalar la dependencia.".to_string()),
                                        );
                                        runtime.record_observation(&env_err_obs);
                                        runtime.record_tool_call();
                                        let res_msg = format!("Error: {}\n[AUTO-ENV FALLÓ] No se pudo instalar '{}': {}\nSe requiere intervención manual.", err, binary, install_err);
                                        current_context.push_str(&format!("Resultado: {}\n\n", res_msg));
                                        emit_event(&app_handle, runtime.current_step(), &res_msg, "ERROR");
                                    }
                                }
                            } else {
                                let digested_err = digest_terminal_output(&err, 2500);
                                let mut res_msg = format!("Error: {}", digested_err);
                                if err.contains("ModuleNotFoundError") || err.contains("No module named") {
                                    // Extract module name from error for better hint
                                    let module_hint = if err.contains("No module named '") {
                                        err.split("No module named '").nth(1)
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
                                        let pip_was_tried = comandos_ejecutados_historico.iter()
                                            .any(|c| c.starts_with("pip install") || c.starts_with("pip3 install"));
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
                                current_context.push_str(&format!("Resultado: {}\n\n", res_msg));
                                emit_event(&app_handle, runtime.current_step(), &res_msg, "ERROR");

                                // ── Track error hash for semantic loop detection ──
                                // If the same error output repeats 3+ times, sanity_monitor will escalate.
                                {
                                    use std::hash::{Hash, Hasher};
                                    use std::collections::hash_map::DefaultHasher;
                                    let mut hasher = DefaultHasher::new();
                                    err.trim().hash(&mut hasher);
                                    let err_hash = hasher.finish();
                                    if last_error_hashes.len() >= 7 { last_error_hashes.pop_front(); }
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
                                    let recovery_dec = runtime.plan_recovery("TOOL_TERMINAL", &err);
                                    match &recovery_dec {
                                        crate::core::recovery::RecoveryDecision::RepairEnvironment { advice } => {
                                            emit_event(&app_handle, runtime.current_step(), &format!("[RECOVERY] Entorno: {}", advice), "WARNING");
                                        }
                                        crate::core::recovery::RecoveryDecision::Abort { reason } => {
                                            emit_event(&app_handle, runtime.current_step(), &format!("[RECOVERY] Abort: {}", reason), "FATAL");
                                        }
                                        _ => {}
                                    }
                                }

                                // ── SELF-REPAIR LOOP (ReAct pattern) ───────────────────

                                // Classify the error type and choose the appropriate recovery strategy.
                                // Professional agents (SWE-agent, Claude) never retry blindly.
                                let error_type = crate::core::error_classifier::classify_error(
                                    &err, &res_msg, 1);
                                let should_escalate = retry_tracker.record_failure("TOOL_TERMINAL", &error_type);

                                if should_escalate {
                                    if error_type == crate::core::error_classifier::ErrorType::Blocked {
                                        // Genuine system/environment blocker — ask the user for help
                                        let escalation_msg = format!(
                                            "[BLOQUEO DEL SISTEMA] Fallo irrecuperable en TOOL_TERMINAL.\n\
                                            El comando requiere intervención del usuario (dependencia no instalable o permiso de admin).\n\
                                            Error: {}\n\
                                            Debes usar TOOL_ASK_USER para explicar qué dependencia externa se necesita.",
                                            err
                                        );
                                        current_context.push_str(&format!("{}\n\n", escalation_msg));
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
                                        current_context.push_str(&format!("{}\n\n", self_heal_msg));
                                        forced_next_tool = Some((
                                            "TOOL_PROGRAMMER".to_string(),
                                            "Corrige el archivo objetivo o flexibiliza el script de prueba.".to_string()
                                        ));
                                        emit_event(&app_handle, runtime.current_step(),
                                            "[AUTO-REFLEXIÓN] Redirigiendo a TOOL_PROGRAMMER para auto-reparar código/tests", "ACTION");
                                    }
                                } else {
                                    // Inject specific repair guidance based on error type
                                    let repair_msg = crate::core::error_classifier::repair_prompt(
                                        &error_type, "TOOL_TERMINAL", &comando, &err,
                                        retry_tracker.transient_retries.max(retry_tracker.logic_retries)
                                    );
                                    current_context.push_str(&format!("{}\n\n", repair_msg));
                                    emit_event(&app_handle, runtime.current_step(),
                                        &format!("[SELF-REPAIR] Error {:?} — guiando al agente con estrategia de reparación",
                                            error_type),
                                        "WARNING");

                                    // For Logic errors: force a THINK step before the next retry
                                    if error_type == crate::core::error_classifier::ErrorType::Logic {
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
            },
            "TOOL_ASSET_MANAGER" => {
                let parts: Vec<&str> = comando.split('|').collect();
                if parts.len() != 2 {
                    let err = "Error: El comando para TOOL_ASSET_MANAGER debe tener el formato 'query|output_path'";
                    current_context.push_str(&format!("{}\n\n", err));
                    emit_event(&app_handle, runtime.current_step(), err, "ERROR");
                } else {
                    let query = parts[0].trim();
                    let output_path = std::path::Path::new(&workspace_path).join(parts[1].trim());
                    let out_str = output_path.to_string_lossy().to_string();
                    emit_event(&app_handle, runtime.current_step(), &format!("Generando asset '{}'...", query), "ACTION");
                    match crate::net::asset_fetcher::download_asset(query, &out_str).await {
                        Ok(msg) => {
                            current_context.push_str(&format!("Resultado TOOL_ASSET_MANAGER: {}\n\n", msg));
                            emit_event(&app_handle, runtime.current_step(), "Asset descargado correctamente.", "SUCCESS");
                        },
                        Err(e) => {
                            current_context.push_str(&format!("Error TOOL_ASSET_MANAGER: {}\n\n", e));
                            emit_event(&app_handle, runtime.current_step(), &format!("Error: {}", e), "ERROR");
                        }
                    }
                }
            },
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
                    emit_event(&app_handle, runtime.current_step(), &format!("[ENV_MANAGER] Rechazado comando de terminal: {}", cmd_trimmed), "WARNING");
                } else if cmd_trimmed.is_empty() {
                    let res_msg = "Error: El paquete no puede estar vacío.";
                    current_context.push_str(&format!("{}\n\n", res_msg));
                    emit_event(&app_handle, runtime.current_step(), "Paquete vacío", "ERROR");
                } else if paquetes_instalados_historico.contains(&comando) {
                    let res_msg = "[SISTEMA INTERCEPTO] Error Crítico: Bucle infinito intentando instalar el mismo paquete repetidamente. Abortando misión.";
                    emit_event(&app_handle, runtime.current_step(), res_msg, "FATAL");
                    let final_res = FinalResponse {
                        status: "FINISH".to_string(), // Frontend safe format
                        respuesta_conversacional: format!("Se detectó un bucle intentando instalar múltiples veces el paquete '{}'. La instalación ya se ejecutó en este turno. Misión abortada.", comando),
                    };
                    crate::llm::router::record_model_result(&orchestrator_model, &crate::llm::router::TaskType::Orchestrator, final_res.status == "FINISH", runtime.current_step());
                    return Ok(serde_json::to_string(&final_res).unwrap());
                } else {
                    paquetes_instalados_historico.insert(comando.clone());
                    emit_event(&app_handle, runtime.current_step(), &format!("Módulo de Ingeniería de Entorno instalando: {}", comando), "ACTION");
                    match crate::core::env_manager::install_dependency(&comando).await {
                        Ok(msg) => {
                            current_context.push_str(&format!("Resultado TOOL_ENV_MANAGER: {}\n\n", msg));
                            emit_event(&app_handle, runtime.current_step(), "Dependencia instalada correctamente. PATH recargado en caliente.", "SUCCESS");
                        },
                        Err(err) => {
                            current_context.push_str(&format!("Resultado TOOL_ENV_MANAGER Error: {}\n\n", err));
                            emit_event(&app_handle, runtime.current_step(), &err, "ERROR");
                        }
                    }
                }
            },
            "TOOL_BACKGROUND_START" => {
                if comando.trim().is_empty() {
                    if comandos_ejecutados_historico.contains("__EMPTY_BG_CMD__") {
                        let res_msg = "[SISTEMA INTERNO]: Advertencia: Estás en un bucle infinito de comandos vacíos. Abortando.";
                        emit_event(&app_handle, runtime.current_step(), res_msg, "FATAL");
                        let final_res = FinalResponse { status: "ERROR".to_string(), respuesta_conversacional: "Error interno del planificador asíncrono.".to_string() };
                        crate::llm::router::record_model_result(&orchestrator_model, &crate::llm::router::TaskType::Orchestrator, final_res.status == "FINISH", runtime.current_step());
                        return Ok(serde_json::to_string(&final_res).unwrap());
                    }
                    comandos_ejecutados_historico.insert("__EMPTY_BG_CMD__".to_string());
                    let err_msg = "Error Crítico: El campo 'comando' está vacío. Debes especificar qué comando ejecutar en la terminal.";
                    current_context.push_str(&format!("{}\n\n", err_msg));
                    emit_event(&app_handle, runtime.current_step(), err_msg, "ERROR");
                } else if !is_forced_and_obeyed && comandos_ejecutados_historico.contains(&comando) {
                    let res_msg = "[SISTEMA INTERNO]: Advertencia: Este servidor o proceso YA ESTÁ EN EJECUCIÓN en segundo plano. NO necesitas volver a iniciarlo. Usa TOOL_VISION_EVALUATOR o TOOL_FINISH.";
                    current_context.push_str(&format!("{}\n\n", res_msg));
                    emit_event(&app_handle, runtime.current_step(), "Servidor ya en ejecución (bucle evitado).", "WARNING");
                } else {
                    comandos_ejecutados_historico.insert(comando.clone());
                    emit_event(&app_handle, runtime.current_step(), &format!("Iniciando tarea asíncrona '{}': {}", task_id, comando), "ACTION");
                    match start_background_task(&workspace_path, &task_id, &comando).await {
                        Ok(out) => {

                            let sys_guidance = "[SISTEMA INTERNO]: El proceso en segundo plano ha sido INICIADO EXITOSAMENTE. NO repitas este comando. Ahora DEBES continuar con la misin usando otras herramientas (por ejemplo, TOOL_VISION_EVALUATOR, TOOL_TESTER, o TOOL_FINISH si has terminado).";
                            current_context.push_str(&format!("Resultado: {}\n{}\n\n", out, sys_guidance));
                            emit_event(&app_handle, runtime.current_step(), &out, "SUCCESS");
                        },
                        Err(err) => {
                            current_context.push_str(&format!("Resultado: Error iniciando tarea: {}\n\n", err));
                            emit_event(&app_handle, runtime.current_step(), &format!("Error: {}", err), "ERROR");
                        }
                    }
                }
            },
            "TOOL_BACKGROUND_READ" => {
                emit_event(&app_handle, runtime.current_step(), &format!("Leyendo logs asíncronos de '{}'", task_id), "ACTION");
                match read_task_logs(&task_id).await {
                    Ok(logs) => {
                        current_context.push_str(&format!("Logs obtenidos:\n{}\n\n", logs));
                        emit_event(&app_handle, runtime.current_step(), "Logs leídos correctamente.", "SUCCESS");
                    },
                    Err(err) => {
                        let fmt_err = format_system_error(&err).await;
                        current_context.push_str(&format!("[AUTO-DEBUGGER] Error al leer logs: {}\n\n", fmt_err));
                        emit_event(&app_handle, runtime.current_step(), &fmt_err, "ERROR");
                    }
                }
            },
            "TOOL_BACKGROUND_KILL" => {
                emit_event(&app_handle, runtime.current_step(), &format!("Destruyendo tarea asíncrona '{}'", task_id), "ACTION");
                match kill_task(&task_id).await {
                    Ok(msg) => {
                        current_context.push_str(&format!("Resultado: {}\n\n", msg));
                        emit_event(&app_handle, runtime.current_step(), &msg, "SUCCESS");
                    },
                    Err(err) => {
                        let fmt_err = format_system_error(&err).await;
                        current_context.push_str(&format!("[AUTO-DEBUGGER] Error matando tarea: {}\n\n", fmt_err));
                        emit_event(&app_handle, runtime.current_step(), &fmt_err, "ERROR");
                    }
                }
            },
            "TOOL_WEB_SCRAPER" => {
                emit_event(&app_handle, runtime.current_step(), &format!("Extrayendo contenido de: {}", url), "ACTION");
                match crate::net::fetch_url_text(&url).await {
                    Ok(content) => {
                        let preview = if content.len() > 1000 { format!("{}... (truncado)", &content[..1000]) } else { content.clone() };
                        current_context.push_str(&format!("Contenido web:\n{}\n\n", preview));
                        emit_event(&app_handle, runtime.current_step(), "Contenido extraído con éxito.", "SUCCESS");
                    },
                    Err(err) => {
                        current_context.push_str(&format!("Error web: {}\n\n", err));
                        emit_event(&app_handle, runtime.current_step(), &err, "ERROR");
                    }
                }
            },
            "TOOL_AUDITOR" => {
                auditor_consecutive += 1;
                if auditor_consecutive > 2 {
                    let msg = "[SISTEMA INTERNO]: Loop de auditoría detectado. Estás auditando demasiadas veces seguidas sin actuar. FORZANDO TOOL_THINK en el siguiente turno.";
                    current_context.push_str(&format!("{}\n\n", msg));
                    emit_event(&app_handle, runtime.current_step(), msg, "WARNING");
                    forced_next_tool = Some(("TOOL_THINK".to_string(), "Analizar auditorias previas y decidir siguiente paso. Si los archivos ya existen usa TOOL_PROGRAMMER para mejorarlos o TOOL_FINISH si todo está correcto.".to_string()));
                } else {
                    emit_event(&app_handle, runtime.current_step(), "Auditando archivos locales...", "ACTION");
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
                                        if matches!(ext.as_str(), "html"|"css"|"js"|"ts"|"py"|"rs"|"go"|"json"|"md") {
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
                    let safe_files = memory::read_files_safely(&workspace_path, files_to_audit.clone()).await;
                    let raw_reporte = delegate_to_auditor(&safe_files, &orchestrator_model).await;
                    let struct_prompt = format!("Convierte este reporte a un JSON con campos: archivos, problema, accion_sugerida. Responde SOLO el JSON. REPORTE:\n{}", &raw_reporte);
                    let structured = call_ollama(&orchestrator_model, &struct_prompt).await
                        .unwrap_or_else(|_| raw_reporte.clone());
                    let structured = structured.trim().to_string();
                    current_context.push_str(&format!("[REPORTE AUDITOR ESTRUCTURADO]\n{}\n\n", structured));
                    emit_event(&app_handle, runtime.current_step(), &format!("Auditoria completada. {} archivos.", files_to_audit.len()), "SUCCESS");
                }
                mandatory_tools_executed.insert("TOOL_AUDITOR".to_string());
            },
            "TOOL_LOGIC_SOLVER" => {
                // ─── MODO 1A: SAT nativo — n_vars + clauses en el JSON del agente ───
                let n_vars_opt  = raw_value.get("n_vars").and_then(|v| v.as_u64()).map(|v| v as usize);
                let clauses_opt = raw_value.get("clauses").and_then(|v| v.as_array()).map(|arr| {
                    arr.iter().filter_map(|row| {
                        row.as_array().map(|r| r.iter().filter_map(|x| x.as_i64().map(|i| i as i32)).collect::<Vec<i32>>())
                    }).collect::<Vec<Vec<i32>>>()
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
                            if c == '[' { depth += 1; }
                            else if c == ']' { depth -= 1; }
                            
                            if depth == 0 && i > 0 {
                                end = i + 1;
                                break;
                            }
                        }
                        if end == 0 { return None; }
                        
                        let array_str = &sub[..end];
                        let parsed: serde_json::Value = serde_json::from_str(array_str).ok()?;
                        let outer = parsed.as_array()?;
                        let clauses: Vec<Vec<i32>> = outer.iter().filter_map(|row| {
                            row.as_array().map(|r| r.iter().filter_map(|x| x.as_i64().map(|i| i as i32)).collect())
                        }).collect();
                        if clauses.is_empty() { return None; }
                        // n_vars = max abs value across all literals
                        let n_vars = clauses.iter().flatten().map(|x| x.unsigned_abs() as usize).max().unwrap_or(0);
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
                    emit_event(&app_handle, runtime.current_step(), &format!("⚡ [SPECTRASAT] Resolviendo instancia SAT: {} vars, {} cláusulas...", n_vars, clauses.len()), "ACTION");
                    let result = crate::llm::solve_with_spectrasat(n_vars, clauses);
                    emit_event(&app_handle, runtime.current_step(), &format!("âš¡ [SPECTRASAT] Veredicto: {}", result), "SUCCESS");
                    result
                } else {
                    // ─── MODO 2: Análisis semántico de código vía LLM (fallback real) ───
                    emit_event(&app_handle, runtime.current_step(), "🔍 [LOGIC_SOLVER] Analizando código con motor lógico-semántico...", "ACTION");
                    let real_files: Vec<String> = {
                        let hallucinated = archivos_vec.iter().any(|f| {
                            !std::path::Path::new(&workspace_path).join(f).exists()
                        });
                        if hallucinated || archivos_vec.is_empty() {
                            let mut found = Vec::new();
                            if let Ok(entries) = std::fs::read_dir(&workspace_path) {
                                for entry in entries.flatten() {
                                    let p = entry.path();
                                    if let Some(ext) = p.extension() {
                                        let ext = ext.to_string_lossy().to_lowercase();
                                        if matches!(ext.as_str(), "py"|"rs"|"js"|"ts"|"go"|"c"|"cpp") {
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
                        let vars: Vec<String> = arr.iter().enumerate().map(|(i, v)| {
                            format!("v{} = {}", i + 1, v.as_bool().unwrap_or(false))
                        }).collect();
                        assignment_msg = format!("\n\n[ASIGNACIÓN BOOLEANA ENCONTRADA]:\n{}", vars.join("\n"));
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
                    emit_event(&app_handle, runtime.current_step(), &format!("⛔ [SPECTRASAT] UNSAT certificado — sistema contradictorio"), "WARNING");
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
                    emit_event(&app_handle, runtime.current_step(), "✅ [SPECTRASAT] SAT certificado — sistema seguro", "SUCCESS");
                    if mission_type == MissionType::Analysis {
                        forced_next_tool = Some(("TOOL_FINISH".to_string(), sat_msg));
                    }
                } else {
                    current_context.push_str(&format!("Veredicto SpectraSAT: {}\n\n", parsed_status));
                    emit_event(&app_handle, runtime.current_step(), "✅ Verificación lógica completada.", "SUCCESS");
                }

            },
            "TOOL_WORKSPACE_MANAGER" => {
                emit_event(&app_handle, runtime.current_step(), "Gestionando archivos del workspace...", "ACTION");
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
                    let mut borrados = Vec::new();
                    let mut errores = Vec::new();
                    for f in &archivos_vec {
                        let target_path = std::path::Path::new(&workspace_path).join(f);
                        if target_path.exists() {
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
                        } else {
                            errores.push(format!("El archivo {} no existe.", f));
                        }
                    }
                    let mut res_msg = String::new();
                    if !borrados.is_empty() {
                        res_msg.push_str(&format!("Éxito: Se borraron permanentemente los siguientes archivos/carpetas: {:?}\n", borrados));
                    }
                    if !errores.is_empty() {
                        res_msg.push_str(&format!("Errores durante la limpieza: {:?}\n", errores));
                    }
                    current_context.push_str(&format!("{}\n\n", res_msg));
                    emit_event(&app_handle, runtime.current_step(), &format!("Limpieza finalizada. {} borrados.", borrados.len()), "SUCCESS");
                }
            },
            "TOOL_THINK" => {
                    think_consecutive += 1;
                    if think_consecutive > 3 {
                        emit_event(&app_handle, runtime.current_step(), "[COOLDOWN] Bucle TOOL_THINK interceptado. Usa otra herramienta.", "WARNING");
                        current_context.push_str(&format!("PASO {}:\nTOOL_THINK bloqueado: bucle detectado. DEBES usar TOOL_PROGRAMMER, TOOL_TERMINAL o TOOL_FINISH.\n\n", runtime.current_step()));
                        forced_next_tool = Some(("TOOL_PROGRAMMER".to_string(), "Forzado para romper bucle de reflexion. Escribe codigo real.".to_string()));
                    } else {
                        emit_event(&app_handle, runtime.current_step(), "Pensando y planificando...", "ACTION");
                        current_context.push_str(&format!("Reflexion Interna del Agente: {}\n\n", &comando));
                        emit_event(&app_handle, runtime.current_step(), "Reflexion completada.", "SUCCESS");
                        // Sprint 1+2 FSM: Planner -> Executor on TOOL_THINK (except Analysis missions)
                        if current_role == AgentRole::Planner {
                            if mission_type == MissionType::Analysis {
                                emit_event(&app_handle, runtime.current_step(), "[FSM] MODO ANALISIS: Planificador permanece activo. Usa TOOL_FINISH para responder.", "INFO");
                            } else {
                                if !comando.trim().is_empty() {
                                    acceptance_contract = Some(formato_contrato(&comando));
                                    runtime.contract.add_criterion(
                                        &format!("AC-{:03}", runtime.contract.acceptance_criteria.len() + 1),
                                        &comando.chars().take(120).collect::<String>(),
                                        crate::core::mission_contract::VerificationMethod::ManualReview,
                                        false,
                                    );
                                    emit_event(&app_handle, runtime.current_step(), &format!("[CONTRATO] Criterios definidos: {}", comando.chars().take(80).collect::<String>()), "INFO");
                                }
                                current_role = AgentRole::Executor;
                                critic_feedback = None;
                                emit_event(&app_handle, runtime.current_step(), "[FSM] PLANIFICADOR -> EJECUTOR: Plan aprobado. Iniciando escritura de codigo.", "INFO");
                            }
                        }
                    }
            },
            "TOOL_MAPPER" => {
                mapper_consecutive += 1;
                if mapper_consecutive > 1 {
                    let msg = "[SISTEMA INTERNO]: Loop de TOOL_MAPPER detectado. El workspace no cambiara magicamente. FORZANDO TOOL_THINK en el siguiente turno.";
                    current_context.push_str(&format!("{}\n\n", msg));
                    emit_event(&app_handle, runtime.current_step(), msg, "WARNING");
                    forced_next_tool = Some(("TOOL_THINK".to_string(), "El mapper ha terminado. Iniciar ejecución de plan.".to_string()));
                } else {
                    emit_event(&app_handle, runtime.current_step(), "🗺️ Iniciando análisis de dependencias del workspace...", "ACTION");
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
            },
            "TOOL_PROGRAMMER" => {
                let mut is_cooldown_blocked = false;
                
                // Block only if the LLM tries to edit ALREADY-EDITED files TWICE IN A ROW
                // WITHOUT running any terminal command in between.
                if !archivos_editados_historico.is_empty() && comandos_ejecutados_historico.is_empty() {
                    is_cooldown_blocked = true;
                    // If there is at least one NEW file in the list, allow the action
                    if archivos_vec.is_empty() { is_cooldown_blocked = false; }
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
                  if is_cooldown_blocked && is_all_frontend && !archivos_vec.is_empty() && programmer_cooldown_hits < 2 {
                      is_cooldown_blocked = false;
                  }

                // CRITICAL FIX: Do NOT block TOOL_PROGRAMMER if the workspace has compilation/syntax errors.
                // The agent must be allowed to fix broken syntax before running tests!
                if is_cooldown_blocked && validate_workspace(&workspace_path).await.is_err() {
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
                        crate::llm::router::record_model_result(&orchestrator_model, &crate::llm::router::TaskType::Orchestrator, final_res.status == "FINISH", runtime.current_step());
                        return Ok(serde_json::to_string(&final_res).unwrap());
                    } else {
                        let interception = "[SISTEMA INTERCEPTO] Error Lógico: Estás intentando editar los mismos archivos por segunda vez consecutiva sin haber probado tu código en la terminal. DEBES ejecutar 'TOOL_TERMINAL' para probar el script y ver los errores antes de seguir programando.";
                        current_context.push_str(&format!("{}\n\n", interception));
                        emit_event(&app_handle, runtime.current_step(), "Bucle interceptado por Cooldown", "WARNING");
                        forced_next_tool = Some(("TOOL_TERMINAL".to_string(), "Se forzó 'TOOL_TERMINAL'. DEBES ejecutar el script en la terminal para probarlo ahora mismo antes de seguir programando. RECUERDA: Debes proporcionar un comando válido en el campo 'comando' (ej. 'python main.py', 'npm start', o 'ls'). NO DEJES EL COMANDO VACÍO.".to_string()));
                    }
                } else {
                    // Valid programming action. 
                    // Clear the terminal history so the LLM must test again after this programming phase.
                    comandos_ejecutados_historico.clear();

                    // CRITICAL AUTO-HEAL: If the workspace currently fails validation (e.g. syntax error in verify_dashboard.py),
                    // automatically inject any failing files into archivos_vec so the programmer model receives them,
                    // even if a small model forgot to include them in archivos_a_editar.
                    let mut compile_alert_note = String::new();
                    if let Err(compile_err) = validate_workspace(&workspace_path).await {
                        let failing_files = crate::core::extract_workspace_files_from_error(&workspace_path, &compile_err);
                        if !failing_files.is_empty() {
                            compile_alert_note = format!(
                                "\n\n⚠️ ALERTA CRÍTICA DE AUTO-HEAL: El workspace tiene un error de sintaxis/compilación en los archivos {:?}:\n{}\nDEBES incluir {:?} en tu lista de 'cambios' y reparar el error de sintaxis.",
                                failing_files, compile_err, failing_files
                            );
                        }
                        for ff in failing_files {
                            if !archivos_vec.contains(&ff) {
                                emit_event(&app_handle, runtime.current_step(), &format!("[AUTO-HEAL] Archivo con error detectado e inyectado a programación: {}", ff), "INFO");
                                archivos_vec.push(ff);
                            }
                        }
                    }

                    let mut safe_files = memory::read_files_safely(&workspace_path, archivos_vec.clone()).await;
                    let mut context_for_qwen = format!("Historial Bucle:\n{}\nArchivos:\n{}", current_context, safe_files);
                
                // If there's a recent Critic thought in the context, we should append it to the programmer's instructions
                let recent_critic_thought = if current_context.contains("Critico -> Ejecutor") || current_context.contains("--- PASO") {
                    format!("

[FEEDBACK DEL CRITICO O PASO RECIENTE]:
{}
(Asegurate de corregir los errores mencionados y cumplir con este feedback en tu codigo).", pensamiento)
                } else {
                    String::new()
                };

                let mut qwen_prompt = format!("Instrucción principal: {}{}{}
DEBES crear/modificar los archivos solicitados con implementaciones COMPLETAS y REALES. PROHIBIDO usar 'pass', 'TODO', funciones vacías, NotImplementedError o cualquier placeholder. Cada función debe tener lógica funcional real.", user_message, compile_alert_note, recent_critic_thought);

                let target_model = resolve_model_or_fallback(
                    if !programmer_model.is_empty() && !programmer_model.to_lowercase().contains("embed") {
                        &programmer_model
                    } else {
                        &orchestrator_model
                    },
                    &available_models,
                );
                emit_event(&app_handle, runtime.current_step(), &format!("[ROUTER] Cerebro Programador Seleccionado: {}", target_model), "INFO");

                let mut exito_bucle_programador = false;
                let mut max_intentos = 3;
                
                while max_intentos > 0 && !exito_bucle_programador {
                    if crate::llm::is_agent_cancelled() {
                        emit_event(&app_handle, runtime.current_step(), "🛑 [CANCELADO] Deteniendo iteración de programación por usuario.", "WARNING");
                        break;
                    }
                    match delegate_to_programmer(&qwen_prompt, &context_for_qwen, &target_model).await {
                        Ok(json_res) => {
                            let mut clean_json_res = strip_think_tags(json_res.clone());
                            if let Some(s) = clean_json_res.find("```json") {
                                if let Some(e) = clean_json_res[s+7..].find("```") {
                                    clean_json_res = clean_json_res[s+7..s+7+e].trim().to_string();
                                }
                            } else if let Some(s) = clean_json_res.find("```") {
                                if let Some(e) = clean_json_res[s+3..].find("```") {
                                    clean_json_res = clean_json_res[s+3..s+3+e].trim().to_string();
                                }
                            }
                            let parsed_output = serde_json::from_str::<ProgrammerOutput>(&clean_json_res)
                                .ok()
                                .or_else(|| try_salvage_programmer_output(&json_res, &archivos_vec));

                            if let Some(prog_output) = parsed_output {
                                if !prog_output.cambios.is_empty() {
                                    emit_event(&app_handle, runtime.current_step(), "Activando Git-Shield: Creando punto de retorno...", "PLANNING");
                                    if let Err(e) = crate::core::create_git_backup(&workspace_path, "Aura-Sentinel: Git-Shield Auto-Backup").await {
                                        emit_event(&app_handle, runtime.current_step(), &format!("Fallo Crítico en Git-Shield: {}", e), "FATAL");
                                        current_context.push_str(&format!("Git-Shield Error: {}. NO SE REALIZARON CAMBIOS.\n\n", e));
                                        break;
                                    }
                                    match memory::apply_code_changes(&workspace_path, prog_output.cambios.clone()).await {
                                        Ok(msg) => {
                                            emit_event(&app_handle, runtime.current_step(), &msg, "SUCCESS");
                                            
                                            // SPRINT 3: Emit file-updated for Hot Reloading
                                            for cambio in &prog_output.cambios {
                                                let full_path = std::path::Path::new(&workspace_path).join(&cambio.archivo);
                                                let _ = app_handle.emit("file-updated", serde_json::json!({
                                                    "path": full_path.to_string_lossy().to_string()
                                                }));
                                            }



                                            // ── CAPA 2: ANTI-STUB ENFORCER ────────────────────────────────────
                                            // Inspect every file for stub patterns BEFORE running validation.
                                            let mut stub_rejections: Vec<String> = Vec::new();
                                            for cambio in &prog_output.cambios {
                                                let file_path = &cambio.archivo;
                                                let content = &cambio.reemplazar;
                                                let report = crate::core::stub_enforcer::detect_stubs(content, file_path);
                                                if report.has_stubs {
                                                    stub_rejections.push(report.rejection_message);
                                                }
                                            }

                                            if !stub_rejections.is_empty() {
                                                // Stubs found — reject the write and force a rewrite
                                                let combined = stub_rejections.join("\n\n");
                                                emit_event(&app_handle, runtime.current_step(), &format!("[ANTI-STUB] ❌ {} archivo(s) rechazados por código incompleto. Exigiendo reescritura...", stub_rejections.len()), "FATAL");
                                                // Rollback the written files
                                                let _ = crate::core::restore_git_backup(&workspace_path).await;
                                                qwen_prompt = format!(
                                                    "{}\n\n[ERROR DE REVISIÓN]: Los archivos fueron rechazados por el sistema Anti-Stub:\n{}\n\nREESCRIBE COMPLETAMENTE O CORRIGE EL CÓDIGO. IMPLEMENTACIÓN REAL OBLIGATORIA.",
                                                    qwen_prompt, combined
                                                );
                                                max_intentos -= 1;
                                            } else {
                                                // Stubs passed — now run full compile validation
                                                emit_event(&app_handle, runtime.current_step(), "Validando compilación...", "VALIDATING");
                                                match validate_workspace(&workspace_path).await {
                                                    Ok(_) => {
                                                        emit_event(&app_handle, runtime.current_step(), "Validación exitosa.", "SUCCESS");
                                                        let _ = memory::update_last_memory_status(&workspace_path, "COMPILACIÓN_EXITOSA").await;

                                                        if true {
                                                            // ── CAPA 4: Advance micro-meta in journal ───────────────────────
                                                            let written_files: Vec<String> = prog_output.cambios.iter()
                                                                .map(|c| c.archivo.clone()).collect();
                                                            if !journal.micro_metas.is_empty() {
                                                                if let Some(mm) = journal.micro_metas.get_mut(journal.micro_meta_actual) {
                                                                    let all_done = mm.archivos.iter().all(|f| {
                                                                        written_files.iter().any(|w: &String| w.contains(f.as_str()))
                                                                    });
                                                                    if all_done {
                                                                        mm.estado = "VERIFICADA".to_string();
                                                                        emit_event(&app_handle, runtime.current_step(), &format!("[PESP] ✅ Micro-Meta [{}/{}] VERIFICADA.", journal.micro_meta_actual + 1, journal.micro_metas.len()), "SUCCESS");
                                                                        if journal.micro_meta_actual + 1 < journal.micro_metas.len() {
                                                                            journal.micro_meta_actual += 1;
                                                                            let next = journal.micro_metas[journal.micro_meta_actual].descripcion.clone();
                                                                            emit_event(&app_handle, runtime.current_step(), &format!("[PESP] 🔄 Avanzando a Micro-Meta [{}/{}]: {}", journal.micro_meta_actual + 1, journal.micro_metas.len(), next), "INFO");
                                                                        }
                                                                    } else {
                                                                        mm.estado = "EN_PROGRESO".to_string();
                                                                    }
                                                                    if let Err(e) = crate::core::session_journal::save_journal(&workspace_path, &journal) { emit_event(&app_handle, runtime.current_step(), &format!("[CHECKPOINT FAILED] {}", e), "FATAL"); }
                                                                }
                                                            }

                                                            let explicit_msg = format!("Programador: Los archivos {:?} fueron escritos con éxito, Anti-Stub APROBADO.\n⚠️ REGLA DE ESTADO OBLIGATORIA: Ahora DEBES usar 'TOOL_TERMINAL' en tu próximo turno para ejecutar el script o archivo principal y verificar que funciona sin errores. NO repitas TOOL_PROGRAMMER ni uses TOOL_FINISH hasta ver los resultados en la terminal.\n\n", written_files);
                                                            current_context.push_str(&explicit_msg);
                                                            exito_bucle_programador = true;
                                                            last_progress_step = runtime.current_step();
                                                            comandos_ejecutados_historico.clear();

                                                            // ── H-6: TOOL_PROGRAMMER → Observation + World diff ──
                                                            let _ = runtime.observe_world();
                                                            let prog_hash_after = runtime.current_world_hash();
                                                            let mut prog_obs = crate::core::observation::Observation::success(
                                                                "TOOL_PROGRAMMER",
                                                                &format!("{} archivos escritos y validados exitosamente", written_files.len()),
                                                                written_files.clone(),
                                                            );
                                                            prog_obs.state_hash_after = Some(prog_hash_after);
                                                            runtime.record_observation(&prog_obs);
                                                            runtime.record_tool_call();

                                                            // Sprint 2: Micrometa-gated Executor->Critic transition
                                                            let all_metas_done = journal.micro_metas.is_empty()
                                                                || journal.micro_metas.iter().all(|mm| mm.estado == "VERIFICADA");
                                                            if all_metas_done {
                                                                current_role = AgentRole::Critic;
                                                                critic_feedback = None;
                                                                emit_event(&app_handle, runtime.current_step(), "[FSM] EJECUTOR -> CRITICO: Todas las micro-metas completadas. Iniciando validacion.", "INFO");
                                                            } else {
                                                                let pending: Vec<String> = journal.micro_metas.iter()
                                                                    .filter(|mm| mm.estado != "VERIFICADA")
                                                                    .map(|mm| mm.descripcion.clone())
                                                                    .collect();
                                                                let pending_str = pending.join(", ");
                                                                let msg = format!("[FSM] Ejecutor permanece activo. Micro-metas pendientes: {}. Completa todos los archivos antes de pasar al Critico.", pending_str);
                                                                current_context.push_str(&format!("{}\n\n", &msg));
                                                                emit_event(&app_handle, runtime.current_step(), &msg, "INFO");
                                                            }
                                                            for f in &written_files {
                                                                archivos_editados_historico.insert(f.clone());
                                                                // ── HECHO INMUTABLE: Previene que el modelo olvide archivos recién creados ──
                                                                current_context.push_str(&format!(
                                                                    "[HECHO INMUTABLE — NO IGNORAR]: El archivo '{}' fue creado/modificado exitosamente en el paso {}. NO debe recrearse ni editarse sin razón técnica explícita.\n",
                                                                    f, runtime.current_step()
                                                                ));
                                                            }
                                                        } else {
                                                            // Integration failed — revert and force fix
                                                            let _ = crate::core::restore_git_backup(&workspace_path).await;
                                                            archivos_editados_historico.clear();
                                                            max_intentos -= 1;
                                                        }
                                                    },
                                                    Err(e) => {
                                                        emit_event(&app_handle, runtime.current_step(), &format!("Error detectado: {}", e), "ERROR");
                                                        
                                                        // Auto-inject any broken workspace files into archivos_vec
                                                        let new_failing = crate::core::extract_workspace_files_from_error(&workspace_path, &e);
                                                        let mandatory_note = if !new_failing.is_empty() {
                                                            format!("\n⚠️ OBLIGATORIO: Tu lista de 'cambios' DEBE incluir un arreglo para los archivos {:?}. No modifiques otros archivos hasta que este error esté resuelto.", new_failing)
                                                        } else {
                                                            String::new()
                                                        };

                                                        qwen_prompt = format!("{}\n\n[ERROR DE COMPILACIÓN/EJECUCIÓN]: El código causó este error:\n{}\n{}\nSoluciónalo y genera un nuevo JSON asegurándote de escapar correctamente los strings y reparar todos los archivos afectados. REGLA ESTRICTA: DEBES RESPONDER ÚNICAMENTE CON UN JSON VÁLIDO (sin texto fuera del JSON).", qwen_prompt, e, mandatory_note);
                                                        
                                                        for ff in &new_failing {
                                                            if !archivos_vec.contains(ff) {
                                                                emit_event(&app_handle, runtime.current_step(), &format!("[AUTO-HEAL] Inyectando archivo afectado para reintento: {}", ff), "INFO");
                                                                archivos_vec.push(ff.clone());
                                                            }
                                                        }
                                                        if e.contains("package.json") && !archivos_vec.contains(&"package.json".to_string()) {
                                                            archivos_vec.push("package.json".to_string());
                                                        }


                                                        // Refresh safe_files and context_for_qwen so the next attempt contains the broken file content!
                                                        safe_files = memory::read_files_safely(&workspace_path, archivos_vec.clone()).await;
                                                        context_for_qwen = format!("Historial Bucle:\n{}\nArchivos:\n{}", current_context, safe_files);

                                                        max_intentos -= 1;
                                                    }
                                                }
                                            } // end stubs-clean else
                                        },
                                        Err(e) => {
                                            emit_event(&app_handle, runtime.current_step(), &format!("Error escribiendo archivos: {}", e), "ERROR");

                                            // FIX-B3: Track per-file patch failures. After 3 consecutive
                                            // failures on the same file, escalate to full-file overwrite mode.
                                            let is_patch_fail = e.contains("[PATCH_FAIL]");
                                            if is_patch_fail {
                                                // Extract which files failed from the error message
                                                let failed_files: Vec<String> = prog_output.cambios.iter()
                                                    .map(|c| c.archivo.clone())
                                                    .collect();

                                                let mut overwrite_files: Vec<String> = Vec::new();
                                                for fname in &failed_files {
                                                    let count = patch_fail_counts.entry(fname.clone()).or_insert(0);
                                                    *count += 1;
                                                    if *count >= 3 {
                                                        overwrite_files.push(fname.clone());
                                                    }
                                                }

                                                if !overwrite_files.is_empty() {
                                                    // Escalate: inject overwrite instruction and force TOOL_PROGRAMMER
                                                    let overwrite_msg = format!(
                                                        "\n\n⛔ ESCALACIÓN CRÍTICA DE PATCH: Los archivos {:?} han fallado {} veces seguidas con parches.\n\
                                                         El modelo está alucinando el contenido del archivo.\n\
                                                         INSTRUCCIÓN OBLIGATORIA: En tu PRÓXIMA llamada a TOOL_PROGRAMMER, usa buscar: \"\" (cadena vacía)\n\
                                                         para SOBREESCRIBIR el archivo completo. NO uses fragmentos de texto en 'buscar'.\n\
                                                         Escribe el archivo COMPLETO en el campo 'reemplazar'.",
                                                        overwrite_files,
                                                        patch_fail_counts.get(&overwrite_files[0]).copied().unwrap_or(3)
                                                    );
                                                    current_context.push_str(&overwrite_msg);
                                                    emit_event(&app_handle, runtime.current_step(),
                                                        &format!("[PATCH-ESCALATION] {} archivo(s) requieren overwrite completo tras 3 fallos", overwrite_files.len()),
                                                        "WARNING");
                                                    // Reset counters for these files so next cycle is fresh
                                                    for fname in &overwrite_files {
                                                        patch_fail_counts.insert(fname.clone(), 0);
                                                    }
                                                } else {
                                                    // Not yet at 3 fails — normal error feedback
                                                    current_context.push_str(&format!("Programador Falló al escribir (PATCH_FAIL): {}\n\n", e));
                                                }
                                            } else {
                                                current_context.push_str(&format!("Programador Falló al escribir: {}\n\n", e));
                                            }
                                            break;
                                        }
                                    }
                                } else {
                                    // No changes proposed — this can be legitimate (files already correct)
                                    emit_event(&app_handle, runtime.current_step(), "El programador no propuso cambios. Los archivos ya están al día.", "WARNING");
                                    current_context.push_str("Programador: No se propusieron cambios (los archivos ya están correctos o no hay nada que modificar). El Ejecutor puede usar TOOL_TERMINAL para verificar o TOOL_FINISH si la tarea está lista.\n\n");
                                    exito_bucle_programador = true; // treat as success, not failure
                                    break;
                                }
                            } else {
                                emit_event(&app_handle, runtime.current_step(), &format!("El programador devolvió JSON inválido (Intento {} restantes)", max_intentos - 1), "ERROR");
                                qwen_prompt = format!("{}\n\n[ERROR CRÍTICO]: Tu respuesta anterior NO fue un JSON válido o falló la serialización. REGLA ESTRICTA: Tu salida debe ser ÚNICAMENTE un objeto JSON analizable, sin texto adicional antes o después. Si usas bloques markdown, asegúrate de que el JSON interno sea correcto.", qwen_prompt);
                                max_intentos -= 1;
                            }
                        },
                        Err(e) => {
                            emit_event(&app_handle, runtime.current_step(), &format!("Error llamando a Qwen: {}", e), "ERROR");
                            current_context.push_str(&format!("Programador: Falla de red: {}\n\n", e));
                            break;
                        }
                    }
                }
                
                if !exito_bucle_programador {
                    emit_event(&app_handle, runtime.current_step(), "El programador no pudo resolver la tarea tras varios intentos. Replanificando...", "WARNING");
                    current_context.push_str("Programador: Fracasó tras múltiples intentos. [SISTEMA]: NO uses TOOL_FINISH para cerrar la fase. Si el archivo es demasiado complejo, debes dividirlo en subarchivos o implementar funciones más simples en pasos sucesivos.\n\n");
                    current_role = AgentRole::Planner;
                } else if exito_bucle_programador {
                    // TOOL_PROGRAMMER succeeded — reset the NoTests loop counter
                    // so TOOL_TESTER can be used again to verify the newly created test files
                    no_tests_consecutive = 0;
                }
                }
            },

            "TOOL_ARCHITECT" => {
                if architect_used {
                    emit_event(&app_handle, runtime.current_step(), "Bucle interceptado por Cooldown (Architect)", "WARNING");
                    current_context.push_str(&format!("PASO {}:\nAcción: TOOL_ARCHITECT\nResultado: [SISTEMA INTERCEPTO] Error: Ya ejecutaste TOOL_ARCHITECT en este bucle. Tu única opción válida ahora es usar TOOL_FINISH para detenerte y resumir los resultados al usuario.\n\n", runtime.current_step()));
                } else {
                    architect_used = true;
                    emit_event(&app_handle, runtime.current_step(), "Generando mapa arquitectónico del sistema...", "ACTION");
                    let graph = crate::core::dependency_mapper::analyze_workspace(&workspace_path);
                    let report = crate::core::dependency_mapper::format_graph_report(&graph);
                    current_context.push_str(&format!("Reporte Arquitectónico:\n{}\n\n", report));
                    emit_event(&app_handle, runtime.current_step(), "Mapa arquitectónico generado.", "SUCCESS");
                }
            },
            "TOOL_VISION_EVALUATOR" => {
                emit_event(&app_handle, runtime.current_step(), "[VISION] Capturando pantalla y evaluando calidad visual...", "ACTION");
                // Sprint 3: Connect evaluate_vision from core::vision
                let vision_prompt = if !comando.trim().is_empty() {
                    comando.clone()
                } else {
                    format!("Evalua la calidad visual de esta pantalla. Describe: 1) Si la UI se ve correcta, 2) Errores visibles, 3) Elementos faltantes. Objetivo original: {}", user_message)
                };

                // Auto-open URL if found in comando or user_message
                let text_to_search = format!("{} {}", comando, user_message);
                let text_lower = text_to_search.to_lowercase();
                if let Some(idx) = text_lower.find("http://").or_else(|| text_lower.find("https://")) {
                    let end_idx = text_to_search[idx..].find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '`').unwrap_or(text_to_search.len() - idx);
                    let url = &text_to_search[idx..idx + end_idx];
                    emit_event(&app_handle, runtime.current_step(), &format!("[VISION] Abriendo navegador en {}", url), "ACTION");
                    
                    #[cfg(target_os = "windows")]
                    let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
                    #[cfg(not(target_os = "windows"))]
                    let _ = std::process::Command::new("open").arg(url).spawn();
                    
                    // Wait for browser to open and render
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }

                match crate::core::vision::evaluate_vision(&vision_prompt, false, None).await {
                    Ok(vision_result) => {
                        current_context.push_str(&format!("[VISION EVALUATOR RESULTADO]\n{}\n\n[INSTRUCCIÓN ESTRICTA DE SEGURIDAD]: LA VALIDACIÓN VISUAL HA SIDO COMPLETADA. SI EL MANDATO DEL USUARIO FUE CUMPLIDO, EN TU SIGUIENTE PASO DEBES ELEGIR OBLIGATORIAMENTE 'TOOL_FINISH'. NO REPITAS HERRAMIENTAS DE VALIDACIÓN.\n\n", vision_result));
                        emit_event(&app_handle, runtime.current_step(), &format!("[VISION] Evaluacion completada: {}", &vision_result.chars().take(120).collect::<String>()), "SUCCESS");
                    },
                    Err(e) => {
                        let msg = format!("[VISION] Error al capturar pantalla: {}. Verifica que haya una ventana abierta.", e);
                        current_context.push_str(&format!("{}\n\n", &msg));
                        emit_event(&app_handle, runtime.current_step(), &msg, "ERROR");
                    }
                }
                // Mark as executed for mandatory checklist (whether it succeeded or not)
                mandatory_tools_executed.insert("TOOL_VISION_EVALUATOR".to_string());
            },
            "TOOL_TESTER" => {
                emit_event(&app_handle, runtime.current_step(), "Ejecutando suite de pruebas automatizadas...", "ACTION");
                // Mark as executed for mandatory checklist regardless of result
                mandatory_tools_executed.insert("TOOL_TESTER".to_string());
                match crate::core::tester::run_tests(&workspace_path).await {
                    crate::core::tester::TestResult::NoTests => {
                        // ── Web project detection ────────────────────────────────────────────
                        // HTML/CSS/JS projects have no test runner. Treat them as PASSED and
                        // auto-advance to TOOL_VISION_EVALUATOR instead of looping endlessly.
                        let is_web_project = has_html_files(&workspace_path);
                        if is_web_project {
                            let web_pass_msg = "[TOOL_TESTER] Proyecto web estático detectado (archivos HTML encontrados). \
                                No existe una suite de tests unitarios del sistema, pero la estructura ha sido validada. \
                                TESTER: APROBADO para proyectos web. \
                                SIGUIENTE PASO: usa TOOL_VISION_EVALUATOR para verificar la UI o TOOL_FINISH si completaste todos los requerimientos.";
                            current_context.push_str(&format!("{}\n\n", web_pass_msg));
                            emit_event(&app_handle, runtime.current_step(), "[TESTER] Proyecto web → APROBADO.", "SUCCESS");
                            forced_next_tool = Some(("TOOL_VISION_EVALUATOR".to_string(),
                                "Verificar visualmente la UI del dashboard creado".to_string()));
                            // Skip the rest of NoTests handling
                        } else {

                        no_tests_consecutive += 1;

                        // Build a workspace file listing so the LLM can see what actually exists
                        let workspace_listing = {
                            let mut files = Vec::new();
                            fn list_dir_recursive(dir: &std::path::Path, files: &mut Vec<String>, depth: usize) {
                                if depth > 4 { return; }
                                if let Ok(entries) = std::fs::read_dir(dir) {
                                    for entry in entries.flatten() {
                                        let path = entry.path();
                                        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                                        if name.starts_with('.') || name == "node_modules" || name == "__pycache__" { continue; }
                                        files.push(path.to_string_lossy().to_string());
                                        if path.is_dir() { list_dir_recursive(&path, files, depth + 1); }
                                    }
                                }
                            }
                            list_dir_recursive(std::path::Path::new(&workspace_path), &mut files, 0);
                            if files.is_empty() {
                                "(directorio vacío)".to_string()
                            } else {
                                files.join("\n")
                            }
                        };

                        if no_tests_consecutive >= 2 {
                            let force_msg = format!(
                                "[SISTEMA INTERCEPTO] TOOL_TESTER fue llamado {} veces sin detectar tests. \
                                El sistema cambia de estrategia. Si esto es un script simple, NO crees tests. \
                                En tu próximo turno DEBES ELEGIR 'TOOL_TERMINAL' para probar manualmente, o 'TOOL_FINISH' si ya terminaste y confirmaste que todo funciona (especialmente si es HTML).\n\
                                Archivos actuales en el workspace:\n{}\n",
                                no_tests_consecutive, workspace_listing
                            );
                            current_context.push_str(&format!("{}\n\n", force_msg));
                            emit_event(&app_handle, runtime.current_step(), &format!("[SISTEMA] Cambiando estrategia a TOOL_TERMINAL tras {} intentos.", no_tests_consecutive), "WARNING");
                            forced_next_tool = Some(("TOOL_TERMINAL".to_string(), "El proyecto no tiene archivos de test. Se forzó TOOL_TERMINAL para que ejecutes el script manualmente (ej. 'python script.py' o 'start index.html'). NUNCA uses 'node' para ejecutar archivos .html. Si es un archivo HTML web o ya lo verificaste, puedes simplemente no hacer nada y prepararte para terminar.".to_string()));

                        } else {
                            // First hit — tell the LLM clearly
                            let no_test_msg = format!(
                                "[SISTEMA] No se detectaron archivos de test reconocidos para ningún lenguaje soportado. \
                                Archivos actuales en el workspace:\n{}\n\n\
                                Si tu proyecto es un script sencillo, NO INTENTES CREAR TESTS. Usa TOOL_TERMINAL para ejecutarlo.\n\
                                SOLO si el usuario pidió tests explícitamente: usa TOOL_PROGRAMMER para crearlos.",
                                workspace_listing
                            );
                            current_context.push_str(&format!("{}\n\n", no_test_msg));
                            emit_event(&app_handle, runtime.current_step(), "Sin suite de tests detectada. Usa TOOL_TERMINAL para scripts simples.", "WARNING");
                        } // end else (non-web project)
                        } // end else (is_web_project check)
                    },
                    crate::core::tester::TestResult::Passed(success_msg) => {
                        tester_attempts = 0;
                        if tester_success_hits >= 1 {
                            let res_msg = "[SISTEMA INTERCEPTO] Error Crítico: Bucle infinito de pruebas exitosas detectado. Abortando misión.";
                            emit_event(&app_handle, runtime.current_step(), res_msg, "FATAL");
                            let final_res = FinalResponse {
                                status: "FINISH".to_string(),
                                respuesta_conversacional: "Los tests ya pasaron con éxito, pero me quedé atascado ejecutándolos en bucle. He detenido el proceso para evitar un ciclo infinito. Misión cumplida.".to_string(),
                            };
                            crate::llm::router::record_model_result(&orchestrator_model, &crate::llm::router::TaskType::Orchestrator, final_res.status == "FINISH", runtime.current_step());
                            return Ok(serde_json::to_string(&final_res).unwrap());
                        } else {
                            tester_success_hits += 1;
                            runtime.cognitive_state.metrics.successful_verifications += 1;
                            // ── H-7: TOOL_TESTER → Observation only. Runtime derives Evidence. ──
                            let test_obs = crate::core::observation::Observation::success(
                                "TOOL_TESTER",
                                &success_msg,
                                vec![],
                            );
                            runtime.record_observation(&test_obs);
                            runtime.record_tool_call();
                            current_context.push_str(&format!("Resultado Tests:\n{}\n\n[INSTRUCCIÓN ESTRICTA DE SEGURIDAD]: LOS TESTS PASARON EXITOSAMENTE. LA TAREA ESTÁ COMPLETADA. EN TU SIGUIENTE PASO DEBES ELEGIR OBLIGATORIAMENTE 'TOOL_FINISH'. NO REPITAS TOOL_TESTER.\n\n", success_msg));
                            emit_event(&app_handle, runtime.current_step(), "Todos los tests pasaron exitosamente. Iniciando Auto-Indexación...", "SUCCESS");
                            // AUTO INDEXACIÓN SILENCIOSA
                            if let Ok(msg) = crate::core::memory::index_project(&workspace_path).await {
                                current_context.push_str(&format!("Memoria Vectorial: {}\n\n", msg));
                            }
                        }
                    },
                    crate::core::tester::TestResult::Failed(fail_msg) => {
                        tester_attempts += 1;
                        if tester_attempts >= 3 {
                            emit_event(&app_handle, runtime.current_step(), "[CRITICAL_FAILURE] Fallos de test superan el límite (3). Revertiendo...", "FATAL");
                            let _ = crate::core::restore_git_backup(&workspace_path).await;
                            let final_res = FinalResponse {
                                status: "FINISH".to_string(),
                                respuesta_conversacional: "He alcanzado el límite máximo de fallos de pruebas. El código era inviable. He restaurado el proyecto a su último estado funcional (Rollback). Por favor, revisa mi código y ayuda a solucionar los tests.".to_string(),
                            };
                            crate::llm::router::record_model_result(&orchestrator_model, &crate::llm::router::TaskType::Orchestrator, final_res.status == "FINISH", runtime.current_step());
                            return Ok(serde_json::to_string(&final_res).unwrap());
                        } else {
                            // ── Detect dependency/install errors vs logic errors ─────────────
                            // If jest/npm/module is missing, auto-fix by running npm install
                            // instead of asking Qwen to rewrite code (which won't help).
                            let is_dep_error = fail_msg.contains("Cannot find module")
                                || fail_msg.contains("jest: command not found")
                                || fail_msg.contains("jest.cmd")
                                || fail_msg.contains("not recognized")
                                || fail_msg.contains("is not recognized")
                                || fail_msg.contains("JSONError")
                                || fail_msg.contains("SyntaxError: Unexpected token")
                                    && fail_msg.contains("package.json")
                                || fail_msg.contains("ENOENT")
                                    && (fail_msg.contains("jest") || fail_msg.contains("node_modules"))
                                || fail_msg.contains("npm ERR! Missing script: test")
                                || fail_msg.contains("Error: no test specified")
                                || fail_msg.contains("The system cannot find the file specified")
                                || fail_msg.contains("NotFound")
                                || fail_msg.contains("No such file or directory")
                                || fail_msg.contains("does not contain main module")
                                || fail_msg.contains("[ENV_FAILURE]")
                                || fail_msg.contains("HHE3")
                                || fail_msg.contains("HHE22")
                                || fail_msg.contains("Hardhat config file found")
                                || fail_msg.contains("non-local installation of Hardhat");

                            if is_dep_error {
                                // Don't revert — the code itself is fine, deps are just missing
                                tester_attempts -= 1; // Don't count this as a real test failure

                                // ── Auto ENV_MANAGER: if a specific binary is missing, install it ──
                                let is_env_failure = fail_msg.contains("[ENV_FAILURE]");
                                if is_env_failure {
                                    // Extract the missing binary name from the [ENV_FAILURE] message
                                    // Pattern: "No se encontró el comando 'X'"
                                    let binary = fail_msg
                                        .split('\'')
                                        .nth(1)
                                        .unwrap_or("")
                                        .trim();

                                    if !binary.is_empty() && !paquetes_instalados_historico.contains(binary) {
                                        emit_event(&app_handle, runtime.current_step(),
                                            &format!("[AUTO-ENV] Tester detectó binario faltante '{}'. Instalando automáticamente...", binary),
                                            "WARNING");
                                        paquetes_instalados_historico.insert(binary.to_string());

                                        match crate::core::env_manager::install_dependency(binary).await {
                                            Ok(install_msg) => {
                                                // Reset tester history so it can be retried
                                                archivos_editados_historico.clear();
                                                comandos_ejecutados_historico.clear();
                                                current_context.push_str(&format!(
                                                    "[AUTO-ENV] Binario '{}' instalado automáticamente: {}\n\n\
                                                     Tu SIGUIENTE PASO OBLIGATORIO es volver a usar TOOL_TESTER.\n\n",
                                                    binary, install_msg
                                                ));
                                                emit_event(&app_handle, runtime.current_step(),
                                                    &format!("'{}' instalado. Reintenta TOOL_TESTER.", binary),
                                                    "SUCCESS");
                                            },
                                            Err(install_err) => {
                                                current_context.push_str(&format!(
                                                    "[AUTO-ENV FALLÓ] No se pudo instalar '{}': {}\n\
                                                     Requiere intervención manual. Usa TOOL_FINISH.\n\n",
                                                    binary, install_err
                                                ));
                                                emit_event(&app_handle, runtime.current_step(),
                                                    &format!("Fallo al auto-instalar '{}': {}", binary, install_err),
                                                    "ERROR");
                                            }
                                        }
                                    } else {
                                        // Already attempted or no binary name found — fall back to guidance
                                    emit_event(&app_handle, runtime.current_step(), "Tests fallaron por dependencias faltantes. Solicito TOOL_TERMINAL...", "ERROR");
                                    current_context.push_str(&format!(
                                        "[AUTO-FIX DEPENDENCIAS] Los tests fallaron por dependencias o configuración faltante:\n{}\n\n\
                                        Tu SIGUIENTE PASO OBLIGATORIO es usar 'TOOL_TERMINAL' con 'npm install' o el instalador adecuado al lenguaje.\n\n",
                                        fail_msg
                                    ));
                                    
                                }
                            } else {
                                // Dependency error but no specific binary — guide the LLM
                                emit_event(&app_handle, runtime.current_step(), "Tests fallaron por dependencias faltantes. Solicito TOOL_TERMINAL...", "ERROR");
                                current_context.push_str(&format!(
                                    "[AUTO-FIX DEPENDENCIAS] Los tests fallaron por dependencias o configuración faltante (no por errores de lógica):\n{}\n\n\
                                    En tu próximo turno DEBES ELEGIR 'TOOL_TERMINAL'. \
                                    Si el proyecto es Node.js: Rellena el campo 'comando' con 'npm install'. \
                                    Si el proyecto es Go: Rellena el campo 'comando' con 'go mod init app && go mod tidy'. \
                                    REGLA ESTRICTA: Después de ejecutar TOOL_TERMINAL con éxito, tu SIGUIENTE PASO OBLIGATORIO es volver a usar TOOL_TESTER.",
                                    fail_msg
                                ));
                                
                            }
                            } else {
                                // Real test logic failure — revert and let Qwen fix the code
                                emit_event(&app_handle, runtime.current_step(), "Tests fallaron. Revertiendo cambios y activando Auto-Debugger...", "ERROR");
                                let _ = crate::core::restore_git_backup(&workspace_path).await;
                                archivos_editados_historico.clear();
                                comandos_ejecutados_historico.clear();
                                _task_complexity = crate::llm::router::TaskContext { task_type: crate::llm::router::TaskType::HighComplexityFix, language: None };
                            emit_event(&app_handle, runtime.current_step(), "[ROUTER] Tarea compleja detectada tras fallo de tests. Escalando modelo experto...", "ACTION");
                                current_role = AgentRole::Executor;
                                critic_feedback = Some(fail_msg.clone());
                                emit_event(&app_handle, runtime.current_step(), "[FSM] 🔬 CRÍTICO → ⚙️ EJECUTOR: Tests fallaron, enviando feedback al Ejecutor.", "WARNING");
                                current_context.push_str(&format!("[AUTO-DEBUGGER] Los tests fallaron estrepitosamente:\n{}\n\nEl sistema ha restaurado el código usando Git-Shield. Debes generar una nueva y mejor solución usando TOOL_PROGRAMMER.\n", fail_msg));
                                forced_next_tool = Some(("TOOL_PROGRAMMER".to_string(), "Los tests fallaron estrepitosamente, el sistema forzó TOOL_PROGRAMMER para que generes una nueva y mejor solución.".to_string()));
                            }
                        }
                    }
                }
            },
            "TOOL_AST_INJECT" => {
                emit_event(&app_handle, runtime.current_step(), "Inyectando nodos AST en Memoria Lógica (Zero-Trace)...", "ACTION");
                let mut report = String::new();
                for (idx, (intent, parent_id, opcode)) in ast_nodes_vec.iter().enumerate() {
                    let node_id = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64) + idx as u64;
                    let meta = [0u8; 16];
                    let node = chronos_vfs::aura_bridge::AuraIntentTranslator::tokenize_intent(
                        (*opcode).into(),
                        *parent_id,
                        node_id,
                        intent,
                        meta,
                    );
                    // Clone the display fields before node is moved into push_node
                    let node_id_display   = node.node_id;
                    let node_hash_display = node.content_hash.clone();
                    let node_op_display   = node.opcode.clone();
                    if let Err(_) = agent_workspace.push_node(node) {
                        report.push_str(&format!("- Error crítico: Buffer Zero-Trace lleno al insertar nodo {}\n", node_id));
                        break;
                    }
                    report.push_str(&format!("- Inyectado: NodeID={} Hash={}\n  Opcode: {:?}\n  Contenido: {}\n", node_id_display, node_hash_display, node_op_display, intent));
                }
                current_context.push_str(&format!("PASO {}:\nAcción: TOOL_AST_INJECT\nResultado:\n{}\n\n", runtime.current_step(), report));
                emit_event(&app_handle, runtime.current_step(), &format!("{} nodos AST inyectados exitosamente en RAM.", ast_nodes_vec.len()), "SUCCESS");
            },
            "TOOL_LEARN" => {
                learn_consecutive += 1;
                if learn_consecutive > 1 {
                    let msg = "[SISTEMA INTERNO]: Ya has aprendido este proyecto (loop infinito TOOL_LEARN detectado). DEBES USAR TOOL_FINISH INMEDIATAMENTE PARA TERMINAR LA TAREA.";
                    forced_next_tool = Some(("TOOL_FINISH".to_string(), "La memoria ya está indexada, finalizando tarea obligatoriamente.".to_string()));
                    current_context.push_str(&format!("{}\n\n", msg));
                    emit_event(&app_handle, runtime.current_step(), "Bucle de TOOL_LEARN detectado, forzando finalización.", "WARNING");
                } else {
                    emit_event(&app_handle, runtime.current_step(), "Guardando conocimiento en la Memoria Permanente (RAG)...", "ACTION");
                    match crate::core::memory::index_project(&workspace_path).await {
                        Ok(msg) => {
                            current_context.push_str(&format!("Resultado TOOL_LEARN: {}\n\n", msg));
                            emit_event(&app_handle, runtime.current_step(), "Memoria indexada correctamente.", "SUCCESS");
                        },
                        Err(err) => {
                            current_context.push_str(&format!("Error en TOOL_LEARN: {}\n\n", err));
                            emit_event(&app_handle, runtime.current_step(), &err, "ERROR");
                        }
                    }
                }
            },
            "TOOL_CREATE_RUNNER" => {
                emit_event(&app_handle, runtime.current_step(), "🏃 Generando runners de ejecución (test, build, dev, lint)...", "ACTION");
                let runners = generate_project_runners(&workspace_path, &original_prompt_parsed).await;
                if runners.is_empty() {
                    current_context.push_str("[TOOL_CREATE_RUNNER] No se generaron runners (lenguaje no detectado o no soportado).\n\n");
                    emit_event(&app_handle, runtime.current_step(), "No se generaron runners (lenguaje desconocido).", "WARNING");
                } else {
                    let names: Vec<String> = runners.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
                    current_context.push_str(&format!("[TOOL_CREATE_RUNNER] Runners internos generados en .aura/runtime/runners: {}\n\n", names.join(", ")));
                    emit_event(&app_handle, runtime.current_step(), &format!("Runners internos generados en .aura/runtime/runners: {}", names.join(", ")), "SUCCESS");
                }
            },
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
                    
                    emit_event(&app_handle, runtime.current_step(), &format!("Contenedor: {} {}", action_str, image_or_id), "ACTION");
                    
                    let action_enum = crate::core::container::ContainerAction::from_str(action_str);
                    match crate::core::container::container_exec(action_enum, image_or_id, run_cmd, &workspace_path).await {
                        Ok(out) => {
                            current_context.push_str(&format!("Resultado TOOL_CONTAINER:\n{}\n\n", out));
                            emit_event(&app_handle, runtime.current_step(), "Comando de contenedor exitoso.", "SUCCESS");
                            last_progress_step = runtime.current_step();
                        },
                        Err(e) => {
                            current_context.push_str(&format!("Error TOOL_CONTAINER:\n{}\n\n", e));
                            emit_event(&app_handle, runtime.current_step(), &e, "ERROR");
                        }
                    }
                }
            },
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
                    let id = crate::core::scheduler::register_task(desc, &workspace_path, cron, desc);
                    current_context.push_str(&format!("Tarea programada exitosamente (ID: {}). Cron: {}\n\n", id, cron));
                    emit_event(&app_handle, runtime.current_step(), &format!("📅 Tarea '{}' programada ({})", desc, cron), "SUCCESS");
                    last_progress_step = runtime.current_step();
                }
            },
            "TOOL_SEARCH" => {
                // FALLO #6 FIX: use `comando` as query first, fall back to `url_a_investigar`.
                let search_query = if !comando.trim().is_empty() { comando.clone() } else { url.clone() };
                emit_event(&app_handle, runtime.current_step(), &format!("Consultando Memoria Permanente para: {}", search_query), "ACTION");
                match crate::core::memory::query_memory(&search_query).await {
                    Ok(msg) => {
                        current_context.push_str(&format!("Resultado TOOL_SEARCH:\n{}\n\n", msg));
                        emit_event(&app_handle, runtime.current_step(), "Búsqueda en memoria completada.", "SUCCESS");
                    },
                    Err(err) => {
                        current_context.push_str(&format!("Error en TOOL_SEARCH: {}\n\n", err));
                        emit_event(&app_handle, runtime.current_step(), &err, "ERROR");
                    }
                }
            },
            "TOOL_ASK_USER" => {
                ask_user_consecutive += 1;
                if ask_user_consecutive > 1 {
                    // Anti-stalling protection: Do not allow the agent to prompt the user repeatedly
                    let abort_msg = "[SISTEMA INTERNO]: ⚠️ TOOL_ASK_USER BLOQUEADO. Ya consultaste al usuario en el turno inmediato anterior. Prohibido volver a preguntar. Debes implementar o verificar la solución de inmediato con TOOL_PROGRAMMER o TOOL_TERMINAL.";
                    current_context.push_str(&format!("{}\n\n", abort_msg));
                    emit_event(&app_handle, runtime.current_step(), "TOOL_ASK_USER bloqueado por repetición consecutiva. Forzando implementación.", "WARNING");
                    current_role = AgentRole::Executor;
                    forced_next_tool = Some(("TOOL_PROGRAMMER".to_string(), "Implementa el código directamente sin hacer más preguntas al usuario.".to_string()));                    continue;
                }

                emit_event(&app_handle, runtime.current_step(), "Solicitando información al usuario...", "ACTION");
                let mut question = comando.clone();
                if question.trim().is_empty() {
                    question = raw_value.get("respuesta_conversacional")
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
                match crate::core::ask_user::ask_user_async(&app_handle, question.clone(), options, current_context.clone()).await {
                    Ok(answer) => {
                        current_context.push_str(&format!("Pregunta al usuario: {}\nRespuesta del usuario: {}\n\n", question, answer));
                        emit_event(&app_handle, runtime.current_step(), "Respuesta del usuario recibida.", "SUCCESS");
                        // User clarified the task — immediately transition to Executor with TOOL_PROGRAMMER
                        current_role = AgentRole::Executor;
                        forced_next_tool = Some(("TOOL_PROGRAMMER".to_string(), format!("El usuario ya aclaró los requerimientos: '{}'. Implementa el código inmediatamente.", answer)));
                    },
                    Err(e) => {
                        current_context.push_str(&format!("Error al consultar al usuario: {}\n\n", e));
                        emit_event(&app_handle, runtime.current_step(), &format!("Error ASK_USER: {}", e), "ERROR");
                    }
                }
            },
            "TOOL_FINISH" => {
                // ── Mandatory Tool Checklist enforcement (Bug 1 fix) ─────────────────
                // If the user's prompt required specific tools (TOOL_TESTER, TOOL_VISION_EVALUATOR)
                // and they haven't been executed yet, block TOOL_FINISH and instruct the agent.
                let missing_mandatory: Vec<&String> = mandatory_tools_required
                    .iter()
                    .filter(|t| !mandatory_tools_executed.contains(*t))
                    .collect();
                if !missing_mandatory.is_empty() {
                    let missing_list: Vec<&str> = missing_mandatory.iter().map(|s| s.as_str()).collect();
                    let block_msg = format!(
                        "[SISTEMA]: TOOL_FINISH BLOQUEADO. El mandato del usuario exige que ejecutes \
                        las siguientes herramientas ANTES de finalizar: {:?}. \
                        Debes ejecutarlas ahora. No puedes usar TOOL_FINISH hasta que todas estén completas.",
                        missing_list
                    );
                    current_context.push_str(&format!("{}\n\n", block_msg));
                    emit_event(&app_handle, runtime.current_step(),
                        &format!("[FINISH BLOQUEADO] Faltan herramientas obligatorias: {:?}", missing_list),
                        "WARNING");                    continue;
                }

                // =======================================================
                // PESP v2 — Intercept TOOL_FINISH for Phase Advancement
                // =======================================================
                if !journal.fases.is_empty() && journal.fase_actual < journal.fases.len() - 1 {
                    let phase_num = journal.fases[journal.fase_actual].numero;
                    let phase_desc = journal.fases[journal.fase_actual].descripcion.clone();

                    // ── GATEKEEPER ESTRICTO DE FASE (Con resolución inteligente de alias) ──
                    let current_phase = &journal.fases[journal.fase_actual];
                    let missing_files: Vec<String> = current_phase.archivos.iter()
                        .filter(|arch| !is_phase_file_satisfied(&workspace_path, arch))
                        .cloned()
                        .collect();

                    if !missing_files.is_empty() {
                        let block_msg = format!("[GATEKEEPER] ❌ Fase {} BLOQUEADA. Faltan archivos requeridos en disco: {:?}", phase_num, missing_files);
                        emit_event(&app_handle, runtime.current_step(), &block_msg, "FATAL");
                        current_context.push_str(&format!("{}\n[ACCIÓN OBLIGATORIA]: No puedes avanzar de fase sin crear estos archivos. Usa TOOL_PROGRAMMER para crearlos.\n\n", block_msg));
                        current_role = AgentRole::Executor;                        continue;
                    }

                    if let Err(compile_err) = validate_workspace(&workspace_path).await {
                        let block_msg = format!("[GATEKEEPER] ❌ Fase {} BLOQUEADA. Errores de sintaxis/compilación detectados:\n{}", phase_num, compile_err);
                        emit_event(&app_handle, runtime.current_step(), &format!("[GATEKEEPER] ❌ Fase {} BLOQUEADA por errores de sintaxis en el código.", phase_num), "FATAL");
                        current_context.push_str(&format!("{}\n[ACCIÓN OBLIGATORIA]: El código generado está incompleto o tiene errores de sintaxis. Usa TOOL_PROGRAMMER para corregir o completar el archivo.\n\n", block_msg));
                        current_role = AgentRole::Executor;                        continue;
                    }
                    
                    emit_event(&app_handle, runtime.current_step(), &format!("⏸ [PAUSA INTERACTIVA] Fase {} completada. Esperando aprobación del usuario...", phase_num), "WARNING");
                    
                    let question = format!("He completado la Fase {}: '{}'. ¿Deseas que avance a la siguiente fase, o quieres revisar/cambiar algo?", phase_num, phase_desc);
                    let options = vec!["Aprobar y Continuar".to_string(), "Modificar Instrucciones".to_string(), "Detener Agente".to_string()];
                    
                    // Pause execution and ask user
                    match crate::core::ask_user::ask_user_async(&app_handle, question, options, current_context.clone()).await {
                        Ok(reply) => {
                            if reply == "Detener Agente" {
                                emit_event(&app_handle, runtime.current_step(), "El usuario detuvo la ejecución.", "ERROR");
                                let final_res = FinalResponse { status: "FINISH".to_string(), respuesta_conversacional: "Detenido por el usuario".to_string() };
                                return Ok(serde_json::to_string(&final_res).unwrap());
                            } else if reply != "Aprobar y Continuar" {
                                let user_feedback = format!("[FEEDBACK DEL USUARIO EN PAUSA INTERACTIVA]: {}", reply);
                                current_context.push_str(&format!("{}\n\n", user_feedback));
                                emit_event(&app_handle, runtime.current_step(), "Feedback del usuario recibido. Ajustando plan.", "ACTION");                                continue;
                            }
                        },
                        Err(e) => {
                            emit_event(&app_handle, runtime.current_step(), &format!("Pausa interactiva interrumpida: {}", e), "ERROR");
                            let final_res = FinalResponse { status: "FINISH".to_string(), respuesta_conversacional: "Interrumpido".to_string() };
                            return Ok(serde_json::to_string(&final_res).unwrap());
                        }
                    }
                    emit_event(&app_handle, runtime.current_step(), &format!("✅ [FASE {} COMPLETADA] Avanzando a la siguiente...", journal.fases[journal.fase_actual].numero), "SUCCESS");
                    
                    // Mark current phase as completed
                    journal.fases[journal.fase_actual].estado = "COMPLETADA".to_string();
                    // Advance to next phase
                    journal.fase_actual += 1;
                    journal.fases[journal.fase_actual].estado = "EN_PROGRESO".to_string();
                    if let Err(e) = crate::core::session_journal::save_journal(&workspace_path, &journal) { emit_event(&app_handle, runtime.current_step(), &format!("[CHECKPOINT FAILED] {}", e), "FATAL"); }
                    
                    let new_phase_msg = format!(
                        "[SISTEMA PESP] Fase anterior completada. Iniciando Fase {}/{}: {}\nUsa TOOL_PROGRAMMER o TOOL_THINK para comenzar el trabajo de esta nueva fase.",
                        journal.fase_actual + 1, journal.fases.len(), journal.fases[journal.fase_actual].descripcion
                    );
                    current_context.push_str(&format!("{}\n\n", new_phase_msg));
                    current_role = AgentRole::Planner; // Reset role                    continue; // Do NOT terminate the agent loop
                } else if !journal.fases.is_empty() && journal.fase_actual == journal.fases.len() - 1 {
                    let current_phase = &journal.fases[journal.fase_actual];
                    let phase_num = current_phase.numero;

                    let missing_files: Vec<String> = current_phase.archivos.iter()
                        .filter(|arch| !is_phase_file_satisfied(&workspace_path, arch))
                        .cloned()
                        .collect();

                    if !missing_files.is_empty() {
                        let block_msg = format!("[GATEKEEPER FINAL] ❌ Misión NO puede cerrarse. Faltan archivos requeridos de la Fase {}: {:?}", phase_num, missing_files);
                        emit_event(&app_handle, runtime.current_step(), &block_msg, "FATAL");
                        current_context.push_str(&format!("{}\n[ACCIÓN OBLIGATORIA]: Crea los archivos pendientes usando TOOL_PROGRAMMER antes de concluir.\n\n", block_msg));
                        current_role = AgentRole::Executor;                        continue;
                    }

                    if let Err(compile_err) = validate_workspace(&workspace_path).await {
                        let block_msg = format!("[GATEKEEPER FINAL] ❌ Misión NO puede cerrarse por error sintáctico:\n{}", compile_err);
                        emit_event(&app_handle, runtime.current_step(), "[GATEKEEPER FINAL] Error sintáctico en disco. Exigiendo corrección.", "FATAL");
                        current_context.push_str(&format!("{}\n[ACCIÓN OBLIGATORIA]: Corrige los errores de sintaxis antes de finalizar.\n\n", block_msg));
                        current_role = AgentRole::Executor;                        continue;
                    }

                }

                // ↀ CompletionGate delegado a MissionRuntime (fuente única de verdad) ↀ
                let completion_decision = runtime.can_complete();
                match completion_decision {
                    crate::core::completion_gate::CompletionDecision::Incomplete(missing_reasons) => {
                        let block_msg = format!("[COMPLETION GATE] ⚠︠ Finalización rechazada. Requisitos pendientes:\n{}", missing_reasons.join("\n"));
                        emit_event(&app_handle, runtime.current_step(), "[COMPLETION GATE] Criterios de misión aún no satisfechos.", "WARNING");
                        current_context.push_str(&format!("{}\n[ACCIÓN OBLIGATORIA]: Resuelve estos puntos antes de llamar a TOOL_FINISH.\n\n", block_msg));                        continue;
                    },
                    crate::core::completion_gate::CompletionDecision::Blocked(block_reasons) => {
                        let block_msg = format!("[COMPLETION GATE] 🛑 Misión bloqueada por restricciones:\n{}", block_reasons.join("\n"));
                        emit_event(&app_handle, runtime.current_step(), "[COMPLETION GATE] Bloqueado por restricciones.", "FATAL");
                        current_context.push_str(&format!("{}\n\n", block_msg));                        continue;
                    },
                    crate::core::completion_gate::CompletionDecision::Complete => {
                        emit_event(&app_handle, runtime.current_step(), "🎯 [COMPLETION GATE] Verificación superada: Todos los criterios cumplidos.", "SUCCESS");
                    }
                }

                emit_event(&app_handle, runtime.current_step(), "Bucle completado exitosamente.", "FINISH");
                // ── Journal: mark completed ──
                if let Err(e) = crate::core::session_journal::close_journal(&mut journal, "COMPLETADO", &workspace_path) { emit_event(&app_handle, runtime.current_step(), &e, "FATAL"); }
                
                // ── Fase 1: Clear interrupt flag ──
                if let Err(e) = crate::core::mission_persist::clear_interrupt(&workspace_path) { emit_event(&app_handle, runtime.current_step(), &e, "FATAL"); }

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
                    vec!["Misión completada con todos los criterios y pruebas aprobados.".to_string()],
                );
                let _ = crate::core::experience::ExperienceStore::record_experience(&exp_rec);

                // ── AL-v1: Registrar Experiencia en Adaptive Learning ─────────────────
                let al_elapsed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64).unwrap_or(0)
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
                let _ = al_engine.record_outcome(
                    al_fingerprint.clone(),
                    orchestrator_model.clone(),
                    al_recommendation.strategy.clone(),
                    al_result,
                    al_recommendation.confidence,
                    runtime.mission_id.clone(),
                    None,
                ).await;

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
                crate::llm::router::record_model_result(&orchestrator_model, &crate::llm::router::TaskType::Orchestrator, final_res.status == "FINISH", runtime.current_step());
                return Ok(serde_json::to_string(&final_res).unwrap());
            },
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
                    emit_event(&app_handle, runtime.current_step(), &format!("[AUTO-REDIRECT] '{}' → TOOL_TERMINAL: {}", tool, terminal_cmd), "WARNING");
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
                        risk: crate::core::policy::PolicyEngine::classify_terminal_command(&comando),
                    };

                    if let Err(auth_err) = runtime.authorize_action(&auto_proposal) {
                        current_context.push_str(&format!(
                            "[AUTO-REDIRECT RECHAZADO]: {}\n\n",
                            auth_err
                        ));
                        emit_event(&app_handle, runtime.current_step(),
                            &format!("[AUTO-REDIRECT RECHAZADO] {}", auth_err),
                            "ERROR");
                    } else {
                        emit_event(&app_handle, runtime.current_step(), &format!("Ejecutando en terminal: {}", comando), "ACTION");
                        comandos_ejecutados_historico.insert(comando.clone());
                        // P0 fix: route through Runtime Gateway, not direct execution
                        let unified_auto_res = match runtime.execute_action(&auto_proposal).await {
                            Ok(obs) if obs.status == crate::core::observation::ObservationStatus::Error => Err(obs.payload),
                            Ok(obs) => Ok(obs),
                            Err(e) => Err(e),
                        };
                        match unified_auto_res {
                            Ok(obs) => {
                                current_context.push_str(&format!("Resultado TOOL_TERMINAL (auto): {}\n\n", obs.payload));
                                emit_event(&app_handle, runtime.current_step(), &format!("Auto-terminal OK: {}", &obs.payload[..obs.payload.len().min(120)]), "SUCCESS");
                            }
                            Err(e) => {
                                current_context.push_str(&format!("Resultado TOOL_TERMINAL (auto) Error: {}\n\n", e));
                                emit_event(&app_handle, runtime.current_step(), &format!("Auto-terminal Error: {}", e), "ERROR");
                            }
                        }
                    }
                } else if tool == "TOOL_READ_FILE" {
                    // FIX-B4: TOOL_READ_FILE is referenced in PATCH_FAIL advice but was never implemented.
                    // Auto-handle it: read the file indicated in `comando` and inject content into context.
                    // `archivos_a_editar` is only in scope inside TOOL_PROGRAMMER, so we rely on `comando` here.
                    let file_to_read = if !comando.trim().is_empty() {
                        let p = std::path::Path::new(comando.trim());
                        if p.is_absolute() {
                            p.to_path_buf()
                        } else {
                            std::path::Path::new(&workspace_path).join(comando.trim())
                        }
                    } else {
                        // No filename given — fallback: list workspace root
                        std::path::Path::new(&workspace_path).to_path_buf()
                    };

                    emit_event(&app_handle, runtime.current_step(), &format!("[TOOL_READ_FILE] Leyendo: {}", file_to_read.display()), "ACTION");
                    match tokio::fs::read_to_string(&file_to_read).await {
                        Ok(contents) => {
                            let c_len = contents.len();
                            let display = if c_len > 8000 { &contents[..8000] } else { &contents[..] };
                            let read_msg = format!(
                                "[TOOL_READ_FILE] Contenido de '{}':\n```\n{}\n```\n\n\
                                 Ahora tienes el contenido real del archivo. Usa TOOL_PROGRAMMER con el \
                                 campo 'buscar' copiado EXACTAMENTE del texto anterior.\n\n",
                                file_to_read.display(), display
                            );
                            current_context.push_str(&read_msg);
                            emit_event(&app_handle, runtime.current_step(), &format!("Archivo leído: {} chars", c_len), "SUCCESS");
                        }
                        Err(e) => {
                            current_context.push_str(&format!(
                                "[TOOL_READ_FILE] Error leyendo '{}': {}. \
                                 Puede que el archivo no exista todavía — usa TOOL_PROGRAMMER con buscar: \"\" para crearlo.\n\n",
                                file_to_read.display(), e
                            ));
                            emit_event(&app_handle, runtime.current_step(), &format!("TOOL_READ_FILE Error: {}", e), "ERROR");
                        }
                    }
                } else {
                    unknown_tool_consecutive += 1;
                    emit_event(&app_handle, runtime.current_step(), &format!("Herramienta desconocida: {}", tool), "WARNING");
                    if unknown_tool_consecutive >= 3 {
                        forced_next_tool = Some((
                            "TOOL_THINK".to_string(),
                            "El sistema bloqueó mi acceso porque intenté usar herramientas inventadas que no existen en el prompt. Transfiero el control para evitar bucles de alucinación.".to_string()
                        ));
                        unknown_tool_consecutive = 0;
                        current_context.push_str(&format!("Error crítico: uso de herramienta desconocida '{}'. [FSM FORZANDO TOOL_THINK]\n\n", tool));
                        emit_event(&app_handle, runtime.current_step(), "[FSM] Agente inventando herramientas. Forzando TOOL_THINK.", "WARNING");
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
        let trail_result = if step_had_error { StepResult::Error } else { StepResult::Success };
        let trail_error = if trail_result == StepResult::Error { Some(format!("Tool: {}", tool)) } else { None };

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
        command_trail.save(&workspace_path);    }
    
    emit_event(&app_handle, runtime.current_step(), "Límite máximo de pasos alcanzado. Bucle abortado.", "FATAL");
    // ── Journal: mark as waiting for user ──
    journal.status = "ESPERANDO".to_string();
    if let Err(e) = crate::core::session_journal::save_journal(&workspace_path, &journal) { emit_event(&app_handle, runtime.current_step(), &format!("[CHECKPOINT FAILED] {}", e), "FATAL"); }
    let final_res = FinalResponse {
        status: "FINISH".to_string(),
        respuesta_conversacional: format!(
            "He alcanzado el límite máximo de {} pasos sin llegar a una conclusión. \
             Por favor, revisa el historial de pasos y proporciona más contexto.",
            runtime.budget.total_steps
        ),
    };
    Ok(serde_json::to_string(&final_res).unwrap())
}











