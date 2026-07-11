use serde::Serialize;
use tauri::{AppHandle, Emitter};
use crate::memory;
use crate::core::{
    execute_terminal_command, start_background_task, read_task_logs, kill_task,
    validate_workspace, format_system_error,
    runner_generator::generate_standard_runners,
    command_trail::{CommandTrail, StepResult},
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

#[derive(serde::Serialize, serde::Deserialize)]
struct AgentState {
    current_role: AgentRole,
    critic_feedback: Option<String>,
    acceptance_contract: Option<String>,
    step_count: u32,
    current_context: String,
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
}

fn classify_mission(msg: &str) -> MissionType {
    let m = msg.to_lowercase();
    let analysis = ["analiza", "analisa", "analice", "analisis", "que hay", "qué hay", "que sistema",
                    "qué sistema", "describe", "explica", "muéstrame", "muestrame", "que tiene",
                    "qué tiene", "que contiene", "qué contiene", "inspect", "analyze", "show me",
                    "que es", "qué es", "que tipo", "qué tipo", "que hace", "qué hace",
                    "analisa este", "analiza este", "revisa este"];
    let debug = ["arregla", "corrige", "bug", "falla", "fallo", "fix", "debug", "broken",
                 "no funciona", "no compila", "sale error", "hay un error"];
    let refactor = ["refactoriza", "refactorizar", "mejora", "optimiza", "limpia el", "reorganiza", "simplifica"];

    if analysis.iter().any(|w| m.contains(w)) { return MissionType::Analysis; }
    if debug.iter().any(|w| m.contains(w))    { return MissionType::Debug; }
    if refactor.iter().any(|w| m.contains(w)) { return MissionType::Refactor; }
    MissionType::Construction
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
    
    // Detectar lenguaje por archivos existentes
    let language = detect_project_language(workspace_path);
    if language == "unknown" {
        // Intentar inferir del prompt
        let prompt_lower = prompt.to_lowercase();
        if prompt_lower.contains("rust") || prompt_lower.contains("cargo") {
            return generate_for_language("rust", project_root).await;
        } else if prompt_lower.contains("python") || prompt_lower.contains("django") || prompt_lower.contains("flask") || prompt_lower.contains("fastapi") {
            return generate_for_language("python", project_root).await;
        } else if prompt_lower.contains("javascript") || prompt_lower.contains("node") || prompt_lower.contains("react") || prompt_lower.contains("vue") || prompt_lower.contains("npm") {
            return generate_for_language("javascript", project_root).await;
        } else if prompt_lower.contains("typescript") || prompt_lower.contains("tsx") || prompt_lower.contains("ts ") {
            return generate_for_language("typescript", project_root).await;
        } else if prompt_lower.contains("go ") || prompt_lower.contains("golang") {
            return generate_for_language("go", project_root).await;
        } else if prompt_lower.contains("java") || prompt_lower.contains("spring") || prompt_lower.contains("maven") || prompt_lower.contains("gradle") {
            return generate_for_language("java", project_root).await;
        } else if prompt_lower.contains("kotlin") || prompt_lower.contains("android") {
            return generate_for_language("kotlin", project_root).await;
        } else if prompt_lower.contains("php") || prompt_lower.contains("laravel") {
            return generate_for_language("php", project_root).await;
        } else if prompt_lower.contains("dart") || prompt_lower.contains("flutter") {
            return generate_for_language("dart", project_root).await;
        } else if prompt_lower.contains("swift") || prompt_lower.contains("ios") {
            return generate_for_language("swift", project_root).await;
        } else if prompt_lower.contains("c#") || prompt_lower.contains("csharp") || prompt_lower.contains(".net") {
            return generate_for_language("csharp", project_root).await;
        } else if prompt_lower.contains("ruby") || prompt_lower.contains("rails") {
            return generate_for_language("ruby", project_root).await;
        } else if prompt_lower.contains("swift") || prompt_lower.contains("ios") {
            return generate_for_language("swift", project_root).await;
        } else if prompt_lower.contains("dart") || prompt_lower.contains("flutter") {
            return generate_for_language("dart", project_root).await;
        } else if prompt_lower.contains("c#") || prompt_lower.contains("csharp") || prompt_lower.contains(".net") {
            return generate_for_language("csharp", project_root).await;
        } else if prompt_lower.contains("ruby") || prompt_lower.contains("rails") {
            return generate_for_language("ruby", project_root).await;
        } else if prompt_lower.contains("solidity") || prompt_lower.contains("foundry") || prompt_lower.contains("hardhat") {
            return generate_for_language("solidity", project_root).await;
        }
        return Vec::new();
    }
    
    generate_for_language(&language, project_root).await
}

async fn generate_for_language(language: &str, _project_root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let (_test_cmd, _build_cmd, _dev_cmd, _lint_cmd) = match language {
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
        std::path::Path::new("."),
        "rust",
        Some("cargo test".to_string()),
        Some("cargo build".to_string()),
        Some("cargo run".to_string()),
        Some("cargo clippy".to_string()),
    ).await {
        Ok(paths) => paths,
        Err(_) => Vec::new(),
    }
}

// Helper para detectar lenguaje del proyecto
fn detect_project_language(workspace_path: &str) -> String {
    use std::path::Path;
    let path = Path::new(workspace_path);
    
    if path.join("Cargo.toml").exists() { return "rust".to_string(); }
    if path.join("package.json").exists() {
        // Verificar si es TypeScript
        if Path::new(workspace_path).join("tsconfig.json").exists() {
            return "typescript".to_string();
        }
        return "javascript".to_string();
    }
    if path.join("requirements.txt").exists() || path.join("pyproject.toml").exists() || path.join("main.py").exists() {
        return "python".to_string();
    }
    if path.join("go.mod").exists() { return "go".to_string(); }
    if path.join("pom.xml").exists() { return "java".to_string(); }
    if path.join("build.gradle").exists() || path.join("build.gradle.kts").exists() { return "kotlin".to_string(); }
    if path.join("composer.json").exists() { return "php".to_string(); }
    if path.join("pubspec.yaml").exists() { return "dart".to_string(); }
    if path.join("Package.swift").exists() { return "swift".to_string(); }
    if path.join("Cargo.toml").exists() { return "rust".to_string(); }
    if path.join("*.csproj").exists() || std::fs::read_dir(workspace_path).map(|entries| entries.filter_map(|e| e.ok()).any(|e| e.path().extension().map(|ext| ext == "csproj").unwrap_or(false))).unwrap_or(false) {
        return "csharp".to_string();
    }
    if path.join("Gemfile").exists() { return "ruby".to_string(); }
    if path.join("foundry.toml").exists() || path.join("hardhat.config.js").exists() || path.join("hardhat.config.ts").exists() {
        return "solidity".to_string();
    }
    "unknown".to_string()
}

pub const ORCHESTRATOR_MODEL: &str = "llama3.1:8b";
#[allow(dead_code)]
pub const PROGRAMMER_MODEL: &str = "qwen2.5-coder:7b";

pub async fn run_agent_loop(
    user_message: String,
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
            current_context.push_str(&format!(
                "[ESTADO ACTUAL DEL WORKSPACE] Los siguientes archivos YA EXISTEN en el proyecto. \
                Antes de crear nada, verifica si estos archivos ya cumplen el objetivo:\n{}\n\n",
                existing_files.join("\n")
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
    let original_prompt_parsed = if let Some(idx) = user_message.find("\n\nGuía de Traducción Técnica") {
        let text = &user_message[..idx];
        text.replace("Petición Original del Usuario: ", "").trim().to_string()
    } else {
        user_message.clone()
    };
    let mut no_tests_consecutive = 0u32;
    let mut think_consecutive = 0u32;
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
    let mut retry_tracker = crate::core::error_classifier::RetryTracker::new();
    let mut agent_workspace = chronos_vfs::workspace::AgentWorkspace::<chronos_vfs::aura_bridge::AuraAstNode>::new(1_048_576).unwrap();

    // ── Multi-Agent Role State Machine ─────────────────────────────────────
    let mut current_role = AgentRole::Planner;
    let mut critic_feedback: Option<String> = None;

    // ── Mission Type Classifier ─────────────────────────────────────────────
    let mission_type = classify_mission(&original_prompt_parsed);
    let mission_label = match &mission_type {
        MissionType::Analysis     => "🔍 ANÁLISIS",
        MissionType::Construction => "🏗️ CONSTRUCCIÓN",
        MissionType::Refactor     => "♻️ REFACTORING",
        MissionType::Debug        => "🐛 DEBUG",
    };

    // ── Acceptance Contract ─────────────────────────────────────────────────
    // Defined by the Planner when it calls TOOL_THINK.
    // The Critic uses this to know exactly when the task is complete.
    let mut acceptance_contract: Option<String> = None;

    let mut step_count = 1;
    let max_steps = 50;
    let mut json_error_count = 0;

    // ── Session Journal ────────────────────────────────────────
    // Persist mission state to disk so sleep/restart doesn’t lose context.
    let mut journal = crate::core::session_journal::load_journal(&workspace_path);
    
    // Si la sesión anterior terminó o falló, limpiamos las fases para que el Arquitecto pueda crear un plan nuevo para esta nueva misión.
    if journal.status == "COMPLETADO" || journal.status == "ERROR" || journal.status == "FINISH" {
        journal.plan_generado = false;
        journal.fases.clear();
        journal.fase_actual = 0;
    }

    journal.objetivo = user_message.clone();
    journal.workspace_path = workspace_path.clone();
    journal.status = "EN_PROGRESO".to_string();
    journal.herramientas_usadas.clear();
    journal.archivos_tocados.clear();
crate::core::session_journal::save_journal(&workspace_path, &journal);
    emit_event(&app_handle, 0, "[DIARIO] Misión registrada en diario de sesión.", "INFO");
    emit_event(&app_handle, 0, &format!("[MISIÓN] Tipo clasificado: {} — El agente operará en modo apropiado.", mission_label), "INFO");
    
    // ─── COMMAND TRAIL (registro estructurado de pasos) ───
    use crate::core::command_trail::CommandTrail;
    let mut command_trail = CommandTrail::load_or_new(&workspace_path, &original_prompt_parsed);
    command_trail.save(&workspace_path);
    
    // ─── AUTO-GENERATE RUNNERS (test, build, dev, lint) ───
    // Detectar lenguaje y generar scripts de ejecución si no existen
    let runners_generated = generate_project_runners(&workspace_path, &original_prompt_parsed).await;
    if !runners_generated.is_empty() {
        emit_event(&app_handle, 0, &format!("🏃 Runners generados: {}", runners_generated.iter().map(|p| p.file_name().unwrap().to_string_lossy()).collect::<Vec<_>>().join(", ")), "SUCCESS");
    }
    
    // =======================================================
    // PESP v2 — Generación del Plan de Fases (Paso 0)
    // =======================================================
    if !journal.plan_generado && (mission_type == MissionType::Construction || mission_type == MissionType::Refactor) {
        emit_event(&app_handle, 0, "🏗️ [ARQUITECTO DE FASES] Analizando tarea para dividirla en fases...", "PLANNING");
        let model_for_planner = "qwen2.5-coder:7b";
        let fases = crate::llm::phase_planner::generate_phase_plan(&original_prompt_parsed, model_for_planner).await;
        
        journal.fases = fases.clone();
        journal.fase_actual = 0;
        journal.plan_generado = true;
        crate::core::session_journal::save_journal(&workspace_path, &journal);
        
        let plan_desc = fases.iter().map(|f| format!("Fase {}: {}", f.numero, f.descripcion)).collect::<Vec<_>>().join(" | ");
        emit_event(&app_handle, 0, &format!("🗺️ Plan generado: {}", plan_desc), "SUCCESS");
    }
    let mut _task_complexity = crate::llm::router::TaskContext { task_type: crate::llm::router::TaskType::GeneralCode, language: None };
    
    
    // ── SESSION PERSISTENCE (LOAD) ──
    let session_file = std::path::Path::new(&workspace_path).join(".aura_session.json");
    if session_file.exists() {
        if let Ok(state_json) = std::fs::read_to_string(&session_file) {
            if let Ok(state) = serde_json::from_str::<AgentState>(&state_json) {
                current_role = state.current_role;
                critic_feedback = state.critic_feedback;
                acceptance_contract = state.acceptance_contract;
                step_count = state.step_count;
                current_context = state.current_context;
                emit_event(&app_handle, step_count, "[SESSION RESTORED] El agente ha recuperado su estado anterior.", "SYSTEM");
            }
        }
    }

    while step_count <= max_steps {

        // ── SESSION PERSISTENCE (SAVE) ──
        let state = AgentState {
            current_role: current_role.clone(),
            critic_feedback: critic_feedback.clone(),
            acceptance_contract: acceptance_contract.clone(),
            step_count,
            current_context: current_context.clone(),
        };
        if let Ok(state_json) = serde_json::to_string_pretty(&state) {
            let _ = std::fs::write(std::path::Path::new(&workspace_path).join(".aura_session.json"), state_json);
        }
        // ── EMERGENCY EXIT: step budget exhausted ────────────────────────
        if step_count == max_steps {
            let emergency_msg = format!(
                "[SISTEMA EMERGENCIA] El agente ha consumido {} pasos sin terminar la tarea. \
                Esto indica un bucle irrecuperable. Se fuerza terminación automática.",
                max_steps
            );
            emit_event(&app_handle, step_count, &emergency_msg, "FATAL");
            let final_res = FinalResponse {
                status: "FINISH".to_string(),
                respuesta_conversacional: format!(
                    "La tarea fue interrumpida tras {} pasos sin converger. \
                    Los archivos creados hasta ahora están en el workspace. \
                    Por favor revisa manualmente el resultado y reintenta con instrucciones más simples.",
                    max_steps
                ),
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
        // ── Context Compression: every 7 steps, summarize to prevent LLM saturation ──
        // SPRINT 1 FIX: Compress more aggressively. At >3000 chars the LLM starts hallucinating.
        if step_count > 1 && step_count % 7 == 1 && current_context.len() > 3000 {
            emit_event(&app_handle, step_count, "[MEMORIA] Comprimiendo historial para liberar ventana de contexto...", "INFO");
            let compress_prompt = format!(
                "Resume en maximo 5 bullet points el siguiente historial. Conserva SOLO: objetivo original, archivos creados/modificados, errores criticos pendientes, ultimo estado. Responde SOLO con el resumen.\n\nHISTORIAL:\n{}",
                &current_context[..current_context.len().min(6000)]
            );
            if let Ok(summary) = call_ollama(&orchestrator_model, &compress_prompt).await {
                let compressed = format!("[CONTEXTO COMPRIMIDO EN PASO {}]\n{}\n\n", step_count, summary);
                current_context = compressed;
                emit_event(&app_handle, step_count, "[MEMORIA] Contexto comprimido exitosamente.", "SUCCESS");
            }
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
        let live_workspace_context = if live_files.is_empty() {
            "El proyecto está completamente vacío. Aún no has creado ningún archivo físico.".to_string()
        } else {
            live_files.join("\n")
        };

        // --- EVITAR DESBORDAMIENTO DE CONTEXTO (SPRINT 1: limite estricto 6000) ---
        // Reducido de 10000 a 6000: los modelos locales se saturan antes y alucinan.
        if current_context.len() > 6000 {
            let offset = current_context.len() - 6000;
            if let Some(cut) = current_context[offset..].find("PASO") {
                current_context = format!("...[HISTORIAL RECORTADO POR LIMITE DE MEMORIA - solo ultimos pasos]...\n{}", &current_context[offset + cut..]);
            } else {
                current_context = format!("...[HISTORIAL RECORTADO]...\n{}", &current_context[offset..]);
            }
        }
        // ── Critic → Executor feedback block ──────────────────────────────────
        let critic_feedback_block = if let Some(ref fb) = critic_feedback {
            format!("\n\n[REPORTE DEL CRÍTICO — DEBES CORREGIR ESTOS PROBLEMAS ANTES DE CONTINUAR]:\n{}\n", fb)
        } else {
            String::new()
        };

        // ── Analysis Fast Path: inject into Planner context ─────────────────────────────
        let analysis_fast_path = if mission_type == MissionType::Analysis {
            if live_files.is_empty() {
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
            REGLAS CRITICAS DEL JSON:\n\
            1. 'comando' = UN SOLO comando de shell real. NUNCA prosa/descripción. Ejemplos: 'dir', 'start index.html', 'node app.js'.\n\
            2. 'archivos_a_editar' = SOLO nombres de archivo relativos (sin rutas absolutas). Ej: ['index.html'] NO ['C:\\Users\\...\\index.html'].\n\
            3. El workspace actual es: {ws}. NUNCA uses rutas de proyectos anteriores (proxy-stack-windows, etc).",
            ws = workspace_path);

        let agent_prompt = match current_role {
            // Planner - compressed to <200 tokens
            AgentRole::Planner => format!(
                "[PLANIFICADOR] Objetivo: {}\nWorkspace: {}\n{}\nHistorial:\n{}\n\nTOOLS PERMITIDOS PLANNER: TOOL_FINISH, TOOL_AST_INJECT, TOOL_MAPPER, TOOL_THINK, TOOL_AUDITOR, TOOL_SEARCH, TOOL_ASK_USER.\nPROHIBIDO: TOOL_WORKSPACE_MANAGER (no borres archivos), TOOL_PROGRAMMER, TOOL_TERMINAL.\nREGLA CRITICA DE REPETICION: Solo usa TOOL_FINISH si los archivos existentes en el Workspace corresponden EXACTAMENTE al objetivo solicitado. Si los archivos tienen nombres que no coinciden con el objetivo (ej. el usuario pide 'juego' pero existen 'projectController.js'), DEBES crear los archivos correctos. NUNCA uses TOOL_FINISH porque 'hay algun archivo' si ese archivo no cumple el objetivo.\nREGLA CRITICA TOOL_MAPPER: TOOL_MAPPER SOLO es valido para proyectos de codigo con multiples archivos fuente que necesitan analisis de dependencias. Para tareas simples usa directamente TOOL_THINK.\n{}{}{}",
                user_message, live_workspace_context, extra_prompt, current_context,
                critic_feedback_block, analysis_fast_path, json_schema
            ),
            // Executor - compressed to <200 tokens
            AgentRole::Executor => format!(
                "[EJECUTOR] Objetivo: {}\nWorkspace: {}\n{}\nHistorial:\n{}\n\nTOOLS PERMITIDOS: TOOL_PROGRAMMER, TOOL_TERMINAL, TOOL_ASSET_MANAGER, TOOL_BACKGROUND_START, TOOL_ASK_USER. Usa TOOL_ENV_MANAGER *SOLO* para instalar binarios scoop (ej: 'nodejs', 'git', no comandos).\nREGLAS: ANTI-STUB (no pass/TODO/funciones vacias). UN archivo por TOOL_PROGRAMMER. No uses TOOL_TESTER ni TOOL_FINISH.\nEJEMPLOS TOOL_TERMINAL: Para correr 'npm audit fix' usa TOOL_TERMINAL con comando='npm audit fix'. Para 'npm install' usa TOOL_TERMINAL con comando='npm install'. Para 'pip install X' usa TOOL_TERMINAL con comando='pip install X'. NUNCA inventes herramientas como 'NPM AUDIT' o 'NPM INSTALL' — todas las acciones de terminal van con TOOL_TERMINAL.\n\n{}{}",
                user_message, live_workspace_context, extra_prompt, current_context,
                critic_feedback_block, json_schema
            ),
            // Critic - compressed to <200 tokens
            AgentRole::Critic => format!(
                "[CRITICO] Objetivo: {}\nWorkspace: {}\n{}\nHistorial:\n{}\n\nTOOLS PERMITIDOS: TOOL_TESTER, TOOL_TERMINAL, TOOL_VISION_EVALUATOR, TOOL_FINISH, TOOL_ASK_USER.\nREGLAS: Usa TOOL_TESTER/TOOL_TERMINAL para validar. Si hay errores describelos. Solo TOOL_FINISH si todo pasa al 100%%. Usa TOOL_ASK_USER si necesitas que el usuario revise o confirme algo.\n[REGLA FRONTEND]: Si hay un index.html, es frontend. NO uses `node script.js` ni comandos terminales backend. Usa TOOL_VISION_EVALUATOR o simplemente TOOL_FINISH.\n\n{}{}",
                user_message, live_workspace_context, extra_prompt, current_context,
                contract_block, json_schema
            ),
        };

        // ── Context Sanitizer: strip any reference to foreign workspaces ────────
        // Prevents the LLM from re-learning stale workspace paths from its own
        // history (e.g. "proxy-stack-windows" from a previous session).
        // FALLO #5 FIX: reassign outer `current_context` directly (no shadow).
        // Previously `let mut current_context` created a shadow that was discarded at loop end.
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
                    // Remove entire lines that contain the marker
                    ctx = ctx.lines()
                        .filter(|line| !line.contains(marker))
                        .collect::<Vec<_>>()
                        .join("\n");
                }
            }
            ctx
        };

        let orchestrator_model = crate::llm::router::get_best_model(&crate::llm::router::TaskContext { task_type: crate::llm::router::TaskType::Orchestrator, language: None }, &available_models, &app_handle, 0).await
            .unwrap_or_else(|_| orchestrator_model.to_string());
        // Cache the model name for context compression (avoids re-resolving every 10 steps)

        let role_label = match current_role {
            AgentRole::Planner  => "🧠 PLANIFICADOR",
            AgentRole::Executor => "⚙️ EJECUTOR",
            AgentRole::Critic   => "🔬 CRÍTICO",
        };
        emit_event(&app_handle, step_count, &format!("[{}] Pensando con {}...", role_label, orchestrator_model), "PLANNING");

        let mut agent_res = match call_ollama(&orchestrator_model, &agent_prompt).await {
            Ok(res) => res,
            Err(e) => {
                emit_event(&app_handle, step_count, &format!("Error de conexión: {}", e), "ERROR");
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
                    emit_event(&app_handle, step_count, &format!("Error parseando decisión ({}). Máximos reintentos (5) alcanzados. Abortando bucle.", e), "ERROR");
                    let final_res = FinalResponse { status: "ERROR".to_string(), respuesta_conversacional: "Fallo crítico persistente en la estructura JSON del planificador.".to_string() };
                    crate::llm::router::record_model_result(&orchestrator_model, &crate::llm::router::TaskType::Orchestrator, final_res.status == "FINISH", step_count);
                    return Ok(serde_json::to_string(&final_res).unwrap());
                } else {
                    emit_event(&app_handle, step_count, &format!("Error de sintaxis JSON (intento {}/5). Reintentando...", json_error_count), "WARNING");
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

        // ── FORCED TOOL VALIDATION ────────────────────────────────────────────
        // If the system has determined the LLM is stuck in a tool-loop,
        // validate its decision against the forced tool constraint.
        if let Some((forced, override_msg)) = &forced_override {
            if tool != *forced {
                intercept_consecutive += 1;
                let release = intercept_consecutive >= 3;
                let extra = if release {
                    "MÁXIMO ALCANZADO: orden cancelada, usa la herramienta que prefieras."
                } else {
                    "Corrige tu elección y usa la herramienta indicada."
                };
                let error_msg = format!(
                    "[INTERCEPT {}/3] Se te ordenó usar '{}' (razón: '{}'). Elegiste '{}'. {}",
                    intercept_consecutive, forced, override_msg, tool, extra
                );
                current_context.push_str(&format!("{}\n\n", error_msg));
                emit_event(&app_handle, step_count,
                    &format!("[INTERCEPT {}/3] LLM desobedeció orden de usar {}", intercept_consecutive, forced),
                    "WARNING");

                if release {
                    // Give up forcing — let the agent choose freely
                    forced_next_tool = None;
                    intercept_consecutive = 0;
                    emit_event(&app_handle, step_count,
                        "[INTERCEPT] Orden cancelada tras 3 intentos. Agente libre.", "WARNING");
                } else {
                    forced_next_tool = forced_override.clone();
                }
                step_count += 1;
                continue;
            } else {
                // LLM obeyed — reset counter
                intercept_consecutive = 0;
            }
        }

        // ── ROLE HARD LOCKS (FSM ENFORCEMENT) ─────────────────────────────────
        let is_forced_and_obeyed = forced_override.as_ref().map_or(false, |(f, _)| f == &tool);
        if !is_forced_and_obeyed && current_role == AgentRole::Planner {
            // TOOL_WORKSPACE_MANAGER is explicitly blocked: in a real test (prueba 4, paso 1)
            // the Planner called it without parameters and deleted the freshly-created project files.
            if ["TOOL_PROGRAMMER", "TOOL_TESTER", "TOOL_TERMINAL", "TOOL_BACKGROUND_START",
                "TOOL_BACKGROUND_READ", "TOOL_BACKGROUND_KILL", "TOOL_ENV_MANAGER",
                "TOOL_ASSET_MANAGER", "TOOL_VISION_EVALUATOR", "TOOL_WORKSPACE_MANAGER"].contains(&tool.as_str()) {
                let error_msg = format!(
                    "[ACCESO DENEGADO]: Eres el Planificador. No tienes permiso para usar {}. \
                    Tu rol es SOLO diseñar la arquitectura. \
                    NUNCA borres archivos existentes. \
                    Usa TOOL_THINK para transferir el control al Ejecutor cuando estés listo.",
                    tool
                );
                current_context.push_str(&format!("{}\n\n", error_msg));
                emit_event(&app_handle, step_count, &format!("[FSM LOCK] Planificador intentó usar {}", tool), "WARNING");
                step_count += 1;
                continue;
            }
        } else if !is_forced_and_obeyed && current_role == AgentRole::Executor {
            if ["TOOL_TESTER", "TOOL_FINISH", "TOOL_VISION_EVALUATOR", "TOOL_MAPPER", "TOOL_AST_INJECT"].contains(&tool.as_str()) {
                let error_msg = format!("[ACCESO DENEGADO]: Eres el Ejecutor. No tienes permiso para usar {}. Tu rol es escribir código. Si terminaste, asegúrate de que tu código esté listo y pasa Anti-Stub. El motor te pasará al Crítico automáticamente.", tool);
                current_context.push_str(&format!("{}\n\n", error_msg));
                emit_event(&app_handle, step_count, &format!("[FSM LOCK] Ejecutor intentó usar {}", tool), "WARNING");
                step_count += 1;
                continue;
            }
        } else if !is_forced_and_obeyed && current_role == AgentRole::Critic {
            if ["TOOL_PROGRAMMER", "TOOL_MAPPER", "TOOL_AST_INJECT"].contains(&tool.as_str()) {
                critic_fsm_lock_consecutive += 1;
                let error_msg = format!("[ACCESO DENEGADO]: Eres el Crítico. No tienes permiso para usar {}. No puedes escribir código físico. Si el código falla, usa TOOL_TERMINAL, y si hay errores, el sistema te regresará al Ejecutor. NO uses TOOL_PROGRAMMER.", tool);
                current_context.push_str(&format!("{}\n\n", error_msg));
                emit_event(&app_handle, step_count, &format!("[FSM LOCK] Crítico intentó usar {}", tool), "WARNING");

                // ── Anti-bucle del Crítico: si lleva demasiados FSM LOCKs
                // seguidos, significa que el modelo está atascado intentando
                // escribir archivos que faltan. Forzamos retorno al Ejecutor
                // con un mensaje de feedback específico.
                if critic_fsm_lock_consecutive >= 3 {
                    // SPRINT 1 FIX: Validación Dinámica FSM.
                    // archivos_vec is declared later in the loop; here we do a live scan
                    // of the workspace to find what's actually on disk, and use current_context
                    // to detect what the LLM intended to produce.
                    let mut ws_files: Vec<String> = Vec::new();
                    fn scan_ws(dir: &std::path::Path, out: &mut Vec<String>, depth: u8) {
                        if depth > 3 { return; }
                        if let Ok(rd) = std::fs::read_dir(dir) {
                            for entry in rd.flatten() {
                                let p = entry.path();
                                if p.is_dir() {
                                    let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                                    if !name.starts_with('.') && name != "node_modules" && name != "target" {
                                        scan_ws(&p, out, depth + 1);
                                    }
                                } else {
                                    out.push(p.file_name().unwrap_or_default().to_string_lossy().to_string());
                                }
                            }
                        }
                    }
                    scan_ws(std::path::Path::new(&workspace_path), &mut ws_files, 0);

                    let feedback = if ws_files.is_empty() {
                        "[REPORTE DEL CRITICO]: Bucle detectado y el workspace esta completamente VACIO. \
                        El Ejecutor no ha creado ningun archivo fisico todavia. Usa TOOL_PROGRAMMER para empezar.".to_string()
                    } else {
                        format!(
                            "[REPORTE DEL CRITICO]: Bucle detectado. Archivos actualmente en disco: {:?}. \
                            Si faltan archivos, crealos con TOOL_PROGRAMMER. Si todos existen, verifica con TOOL_TERMINAL.",
                            ws_files
                        )
                    };
                    critic_feedback = Some(feedback.clone());
                    current_role = AgentRole::Executor;
                    critic_fsm_lock_consecutive = 0;
                    think_consecutive = 0;
                    mapper_consecutive = 0;
                    comandos_ejecutados_historico.clear(); // allow fresh terminal commands
                    emit_event(&app_handle, step_count, 
                        "[FSM] CRITICO -> EJECUTOR: Bucle detectado, validacion dinamica aplicada.", 
                        "WARNING");
                    current_context.push_str(&format!("{}\n\n", feedback));
                }

                step_count += 1;
                continue;
            }
            // Reset lock counter when Critic does something valid
            critic_fsm_lock_consecutive = 0;
        }
        
        let url = raw_value.get("url_a_investigar").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let respuesta_conv = raw_value.get("respuesta_conversacional").and_then(|v| v.as_str()).unwrap_or("").to_string();
        
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
            emit_event(&app_handle, step_count, &format!("Checklist Mental: {}", checklist), "PLANNING");
        }

        // FORCED TOOL OVERRIDE REMOVED. Validation happens above.

        emit_event(&app_handle, step_count, &format!("Decisión: {} - {}", tool, pensamiento), "DECISION");
        current_context.push_str(&format!("--- PASO {} ---\nChecklist Mental: {}\nDecidiste: {}\nPensamiento: {}\n", step_count, checklist, tool, pensamiento));

        // ── Journal: update per-step ─────────────────────────────────────
        crate::core::session_journal::update_journal(
            &mut journal,
            step_count,
            &format!("[PASO {}] {} - {}", step_count, tool, pensamiento),
            &tool,
            &archivos_vec,
            &workspace_path,
        );
        crate::core::session_journal::save_journal(&workspace_path, &journal);
        
        // Reset loop counters
        if tool != "TOOL_THINK" { think_consecutive = 0; }
        if tool != "TOOL_AUDITOR" { auditor_consecutive = 0; }
        if tool != "TOOL_MAPPER" { mapper_consecutive = 0; }
        if tool != "TOOL_LEARN" { learn_consecutive = 0; }

        match tool.as_str() {
            "TOOL_TERMINAL" => {
                let cmd_lower = comando.to_lowercase();
                if cmd_lower.contains("http-server") || cmd_lower.contains("npm start") || cmd_lower.contains("npm run dev") || cmd_lower.contains("python -m http.server") || cmd_lower.contains("flask run") || cmd_lower.contains("uvicorn") {
                    let res_msg = "[SISTEMA INTERNO]: Has intentado iniciar un servidor web continuo (http-server, npm start, etc.) usando TOOL_TERMINAL. Esto bloquea la terminal infinitamente y rompe el agente.\nSi el objetivo es probar HTML/JS estático, usa `start index.html` para abrirlo directamente en el navegador sin servidor.\nSi requieres obligatoriamente un backend, DEBES usar TOOL_BACKGROUND_START. [SISTEMA: Redirigiendo automticamente a TOOL_BACKGROUND_START...]";
                    current_context.push_str(&format!("{}\n\n", res_msg));
                    emit_event(&app_handle, step_count, "Servidor web bloqueado en TOOL_TERMINAL", "WARNING");
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
                        emit_event(&app_handle, step_count, "[SISTEMA] Comando vacío repetido — forzando TOOL_VISION_EVALUATOR", "WARNING");
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
                        emit_event(&app_handle, step_count, &format!("Comando vacío ({}/3)", empty_count + 1), "ERROR");
                    }
                    programmer_cooldown_hits = 0;
                } else if comandos_ejecutados_historico.contains(&comando) {
                    let res_msg = "[SISTEMA INTERNO]: Bucle detectado. Estás repitiendo exactamente el mismo comando. Si falló anteriormente, usa TOOL_PROGRAMMER o TOOL_AUDITOR para arreglar el código. Si ya tuvo éxito y solo estabas probando, la tarea está lista: usa TOOL_FINISH obligatoriamente.";
                    emit_event(&app_handle, step_count, "Comando repetido interceptado", "WARNING");
                    if current_role == AgentRole::Critic {
                        let msg = "[SISTEMA INTERNO]: Bucle de terminal detectado en el Crítico. Estás repitiendo el mismo comando. Esto suele significar que la prueba ya tuvo éxito y no hay más errores. El sistema está FORZANDO la acción TOOL_FINISH para terminar la tarea o la fase actual de forma segura.";
                        emit_event(&app_handle, step_count, "[SISTEMA] Bucle de Terminal en Crítico -> Forzando FINISH", "WARNING");
                        forced_next_tool = Some(("TOOL_FINISH".to_string(), "Las pruebas parecen haber concluido. Usa TOOL_FINISH.".to_string()));
                        current_context.push_str(&format!("{}\n\n", msg));
                    } else if current_role == AgentRole::Executor {
                        // Executor stuck repeating informational commands (dir, ls, etc.) = task is done
                        let cmd_lower = comando.to_lowercase();
                        let is_info_cmd = cmd_lower.trim() == "dir"
                            || cmd_lower.trim() == "ls"
                            || cmd_lower.trim() == "ls -la"
                            || cmd_lower.trim() == "dir /b";
                        if is_info_cmd {
                            let msg = "[SISTEMA INTERNO]: El Ejecutor está repitiendo un comando informacional (dir/ls). Esto indica que la tarea ya fue completada. FORZANDO TOOL_FINISH para cerrar la misión.";
                            emit_event(&app_handle, step_count, "[SISTEMA] Ejecutor en loop informacional -> Forzando FINISH", "WARNING");
                            forced_next_tool = Some(("TOOL_FINISH".to_string(), "La tarea ya fue completada según el historial de comandos. Resume los resultados al usuario.".to_string()));
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
                        emit_event(&app_handle, step_count, "Servidor web bloqueado en TOOL_TERMINAL", "WARNING");
                        programmer_cooldown_hits = 0; forced_next_tool = Some(("TOOL_BACKGROUND_START".to_string(), comando.clone()));
                    } else {
                        programmer_cooldown_hits = 0;
                        comandos_ejecutados_historico.insert(comando.clone());

                        // ── Pre-check: verify the script file exists before running it ─────────
                        let cmd_lower_check = comando.to_lowercase();
                        let script_file = if cmd_lower_check.starts_with("node ") {
                            Some(comando.trim_start_matches("node ").trim().split_whitespace().next().unwrap_or(""))
                        } else if cmd_lower_check.starts_with("python ") || cmd_lower_check.starts_with("python3 ") {
                            Some(comando.splitn(2, ' ').nth(1).unwrap_or("").trim().split_whitespace().next().unwrap_or(""))
                        } else {
                            None
                        };
                        if let Some(script) = script_file {
                            if !script.is_empty() && !script.starts_with('-') {
                                let script_path = std::path::Path::new(&workspace_path).join(script);
                                if !script_path.exists() {
                                    let warn_msg = format!(
                                        "[SISTEMA]: El archivo '{}' NO existe en el workspace. No puedes ejecutar un archivo que no existe.\n\
                                        Primero crea el archivo con TOOL_PROGRAMMER, o verifica los archivos disponibles con TOOL_TERMINAL (dir).",
                                        script
                                    );
                                    current_context.push_str(&format!("{}\n\n", warn_msg));
                                    emit_event(&app_handle, step_count, &format!("[PRE-CHECK] Archivo no encontrado: {}", script), "ERROR");
                                    continue;
                                }
                            }
                        }

                        emit_event(&app_handle, step_count, &format!("Ejecutando en terminal: {}", comando), "ACTION");
                        match execute_terminal_command(&workspace_path, &comando).await {
                        Ok(out) => {

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
                            let res_msg = format!("Éxito: {}", out);
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
                                // For node, skip auto-verifier if it's a browser project (index.html exists)
                                let is_browser_project = std::path::Path::new(&workspace_path).join("index.html").exists();
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
                                    current_context.push_str(&format!(
                                        "Resultado: {}✅\n\n[SISTEMA: El script no imprimió salida en consola, PERO generó los siguientes archivos de salida que CONFIRMAN que la tarea fue completada exitosamente:]\n\n{}\n\n[SISTEMA: Los archivos de salida existen y tienen contenido. Tu ÚNICO PASO VÁLIDO AHORA es usar 'TOOL_FINISH' para reportarle esto al usuario. ESTÁ PROHIBIDO volver a ejecutar el script.]\n\n",
                                        res_msg,
                                        found_outputs.join("\n\n")
                                    ));
                                    emit_event(&app_handle, step_count, &format!("✅ Script OK — {} archivo(s) de salida generados", found_outputs.len()), "SUCCESS");
                                } else {
                                    current_context.push_str(&format!("Resultado: {}\n\n[SISTEMA: El comando en terminal se ejecutó con éxito. Analiza este resultado. Si esto completa el objetivo final del usuario, tu SIGUIENTE PASO OBLIGATORIO es usar 'TOOL_FINISH'. Si aún faltan pasos, continúa. NO uses TOOL_TESTER a menos que el usuario haya pedido pruebas automatizadas.]\n\n", res_msg));
                                    emit_event(&app_handle, step_count, &res_msg, "SUCCESS");
                                }
                            } else {
                                current_context.push_str(&format!("Resultado: {}\n\n[SISTEMA: El comando en terminal se ejecutó con éxito. Analiza este resultado. Si esto completa el objetivo final del usuario, tu SIGUIENTE PASO OBLIGATORIO es usar 'TOOL_FINISH'. Si aún faltan pasos, continúa. NO uses TOOL_TESTER a menos que el usuario haya pedido pruebas automatizadas.]\n\n", res_msg));
                                emit_event(&app_handle, step_count, &res_msg, "SUCCESS");
                            }
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
                                // Extract likely binary name from the failed command (first word)
                                let binary = comando.split_whitespace().next().unwrap_or(&comando);
                                emit_event(&app_handle, step_count,
                                    &format!("[AUTO-ENV] Binario '{}' no encontrado. Invocando TOOL_ENV_MANAGER automáticamente...", binary),
                                    "WARNING");

                                match crate::core::env_manager::install_dependency(binary).await {
                                    Ok(install_msg) => {
                                        // Reset command history so the original command can be retried

                                        current_context.push_str(&format!(
                                            "[AUTO-ENV] TOOL_ENV_MANAGER instaló '{}' automáticamente: {}\n\n\
                                             Tu SIGUIENTE PASO OBLIGATORIO es reintentar el comando que falló: '{}'.\n\n",
                                            binary, install_msg, comando
                                        ));
                                        emit_event(&app_handle, step_count,
                                            &format!("Dependencia '{}' instalada. Reintenta el comando.", binary),
                                            "SUCCESS");
                                    },
                                    Err(install_err) => {
                                        let res_msg = format!("Error: {}\n[AUTO-ENV FALLÓ] No se pudo instalar '{}': {}\nSe requiere intervención manual.", err, binary, install_err);
                                        current_context.push_str(&format!("Resultado: {}\n\n", res_msg));
                                        emit_event(&app_handle, step_count, &res_msg, "ERROR");
                                    }
                                }
                            } else {
                                let mut res_msg = format!("Error: {}", err);
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
                                emit_event(&app_handle, step_count, &res_msg, "ERROR");

                                // ── SELF-REPAIR LOOP (ReAct pattern) ───────────────────
                                // Classify the error type and choose the appropriate recovery strategy.
                                // Professional agents (SWE-agent, Claude) never retry blindly.
                                let error_type = crate::core::error_classifier::classify_error(
                                    &err, &res_msg, 1);
                                let should_escalate = retry_tracker.record_failure("TOOL_TERMINAL", &error_type);

                                if should_escalate {
                                    // Too many retries — ask the user for help
                                    let escalation_msg = format!(
                                        "[SELF-REPAIR] Múltiples intentos fallidos en TOOL_TERMINAL. \
                                        El agente no puede resolver este error solo.\n\
                                        Error: {}\n\
                                        Debes usar TOOL_ASK_USER (o TOOL_FINISH si la tarea está parcialmente completa) \
                                        para explicar al usuario qué está bloqueado y qué necesita.",
                                        err
                                    );
                                    current_context.push_str(&format!("{}\n\n", escalation_msg));
                                    emit_event(&app_handle, step_count,
                                        "[SELF-REPAIR] Escalando al usuario tras múltiples fallos", "WARNING");
                                } else {
                                    // Inject specific repair guidance based on error type
                                    let repair_msg = crate::core::error_classifier::repair_prompt(
                                        &error_type, "TOOL_TERMINAL", &comando, &err,
                                        retry_tracker.transient_retries.max(retry_tracker.logic_retries)
                                    );
                                    current_context.push_str(&format!("{}\n\n", repair_msg));
                                    emit_event(&app_handle, step_count,
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
                                    emit_event(&app_handle, step_count,
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
                    emit_event(&app_handle, step_count, err, "ERROR");
                } else {
                    let query = parts[0].trim();
                    let output_path = std::path::Path::new(&workspace_path).join(parts[1].trim());
                    let out_str = output_path.to_string_lossy().to_string();
                    emit_event(&app_handle, step_count, &format!("Generando asset '{}'...", query), "ACTION");
                    match crate::net::asset_fetcher::download_asset(query, &out_str).await {
                        Ok(msg) => {
                            current_context.push_str(&format!("Resultado TOOL_ASSET_MANAGER: {}\n\n", msg));
                            emit_event(&app_handle, step_count, "Asset descargado correctamente.", "SUCCESS");
                        },
                        Err(e) => {
                            current_context.push_str(&format!("Error TOOL_ASSET_MANAGER: {}\n\n", e));
                            emit_event(&app_handle, step_count, &format!("Error: {}", e), "ERROR");
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
                    emit_event(&app_handle, step_count, &format!("[ENV_MANAGER] Rechazado comando de terminal: {}", cmd_trimmed), "WARNING");
                } else if cmd_trimmed.is_empty() {
                    let res_msg = "Error: El paquete no puede estar vacío.";
                    current_context.push_str(&format!("{}\n\n", res_msg));
                    emit_event(&app_handle, step_count, "Paquete vacío", "ERROR");
                } else if paquetes_instalados_historico.contains(&comando) {
                    let res_msg = "[SISTEMA INTERCEPTO] Error Crítico: Bucle infinito intentando instalar el mismo paquete repetidamente. Abortando misión.";
                    emit_event(&app_handle, step_count, res_msg, "FATAL");
                    let final_res = FinalResponse {
                        status: "FINISH".to_string(), // Frontend safe format
                        respuesta_conversacional: format!("Se detectó un bucle intentando instalar múltiples veces el paquete '{}'. La instalación ya se ejecutó en este turno. Misión abortada.", comando),
                    };
                    crate::llm::router::record_model_result(&orchestrator_model, &crate::llm::router::TaskType::Orchestrator, final_res.status == "FINISH", step_count);
                    return Ok(serde_json::to_string(&final_res).unwrap());
                } else {
                    paquetes_instalados_historico.insert(comando.clone());
                    emit_event(&app_handle, step_count, &format!("Módulo de Ingeniería de Entorno instalando: {}", comando), "ACTION");
                    match crate::core::env_manager::install_dependency(&comando).await {
                        Ok(msg) => {
                            current_context.push_str(&format!("Resultado TOOL_ENV_MANAGER: {}\n\n", msg));
                            emit_event(&app_handle, step_count, "Dependencia instalada correctamente. PATH recargado en caliente.", "SUCCESS");
                        },
                        Err(err) => {
                            current_context.push_str(&format!("Resultado TOOL_ENV_MANAGER Error: {}\n\n", err));
                            emit_event(&app_handle, step_count, &err, "ERROR");
                        }
                    }
                }
            },
            "TOOL_BACKGROUND_START" => {
                if comando.trim().is_empty() {
                    if comandos_ejecutados_historico.contains("__EMPTY_BG_CMD__") {
                        let res_msg = "[SISTEMA INTERNO]: Advertencia: Estás en un bucle infinito de comandos vacíos. Abortando.";
                        emit_event(&app_handle, step_count, res_msg, "FATAL");
                        let final_res = FinalResponse { status: "ERROR".to_string(), respuesta_conversacional: "Error interno del planificador asíncrono.".to_string() };
                        crate::llm::router::record_model_result(&orchestrator_model, &crate::llm::router::TaskType::Orchestrator, final_res.status == "FINISH", step_count);
                        return Ok(serde_json::to_string(&final_res).unwrap());
                    }
                    comandos_ejecutados_historico.insert("__EMPTY_BG_CMD__".to_string());
                    let err_msg = "Error Crítico: El campo 'comando' está vacío. Debes especificar qué comando ejecutar en la terminal.";
                    current_context.push_str(&format!("{}\n\n", err_msg));
                    emit_event(&app_handle, step_count, err_msg, "ERROR");
                } else if comandos_ejecutados_historico.contains(&comando) {
                    let res_msg = "[SISTEMA INTERNO]: Advertencia: Este servidor o proceso YA ESTÁ EN EJECUCIÓN en segundo plano. NO necesitas volver a iniciarlo. Usa TOOL_VISION_EVALUATOR o TOOL_FINISH.";
                    current_context.push_str(&format!("{}\n\n", res_msg));
                    emit_event(&app_handle, step_count, "Servidor ya en ejecución (bucle evitado).", "WARNING");
                } else {
                    comandos_ejecutados_historico.insert(comando.clone());
                    emit_event(&app_handle, step_count, &format!("Iniciando tarea asíncrona '{}': {}", task_id, comando), "ACTION");
                    match start_background_task(&workspace_path, &task_id, &comando).await {
                        Ok(out) => {

                            let sys_guidance = "[SISTEMA INTERNO]: El proceso en segundo plano ha sido INICIADO EXITOSAMENTE. NO repitas este comando. Ahora DEBES continuar con la misin usando otras herramientas (por ejemplo, TOOL_VISION_EVALUATOR, TOOL_TESTER, o TOOL_FINISH si has terminado).";
                            current_context.push_str(&format!("Resultado: {}\n{}\n\n", out, sys_guidance));
                            emit_event(&app_handle, step_count, &out, "SUCCESS");
                        },
                        Err(err) => {
                            current_context.push_str(&format!("Resultado: Error iniciando tarea: {}\n\n", err));
                            emit_event(&app_handle, step_count, &format!("Error: {}", err), "ERROR");
                        }
                    }
                }
            },
            "TOOL_BACKGROUND_READ" => {
                emit_event(&app_handle, step_count, &format!("Leyendo logs asíncronos de '{}'", task_id), "ACTION");
                match read_task_logs(&task_id).await {
                    Ok(logs) => {
                        current_context.push_str(&format!("Logs obtenidos:\n{}\n\n", logs));
                        emit_event(&app_handle, step_count, "Logs leídos correctamente.", "SUCCESS");
                    },
                    Err(err) => {
                        let fmt_err = format_system_error(&err).await;
                        current_context.push_str(&format!("[AUTO-DEBUGGER] Error al leer logs: {}\n\n", fmt_err));
                        emit_event(&app_handle, step_count, &fmt_err, "ERROR");
                    }
                }
            },
            "TOOL_BACKGROUND_KILL" => {
                emit_event(&app_handle, step_count, &format!("Destruyendo tarea asíncrona '{}'", task_id), "ACTION");
                match kill_task(&task_id).await {
                    Ok(msg) => {
                        current_context.push_str(&format!("Resultado: {}\n\n", msg));
                        emit_event(&app_handle, step_count, &msg, "SUCCESS");
                    },
                    Err(err) => {
                        let fmt_err = format_system_error(&err).await;
                        current_context.push_str(&format!("[AUTO-DEBUGGER] Error matando tarea: {}\n\n", fmt_err));
                        emit_event(&app_handle, step_count, &fmt_err, "ERROR");
                    }
                }
            },
            "TOOL_WEB_SCRAPER" => {
                emit_event(&app_handle, step_count, &format!("Extrayendo contenido de: {}", url), "ACTION");
                match crate::net::fetch_url_text(&url).await {
                    Ok(content) => {
                        let preview = if content.len() > 1000 { format!("{}... (truncado)", &content[..1000]) } else { content.clone() };
                        current_context.push_str(&format!("Contenido web:\n{}\n\n", preview));
                        emit_event(&app_handle, step_count, "Contenido extraído con éxito.", "SUCCESS");
                    },
                    Err(err) => {
                        current_context.push_str(&format!("Error web: {}\n\n", err));
                        emit_event(&app_handle, step_count, &err, "ERROR");
                    }
                }
            },
            "TOOL_AUDITOR" => {
                auditor_consecutive += 1;
                if auditor_consecutive > 2 {
                    let msg = "[SISTEMA INTERNO]: Loop de auditoría detectado. Estás auditando demasiadas veces seguidas sin actuar. FORZANDO TOOL_THINK en el siguiente turno.";
                    current_context.push_str(&format!("{}\n\n", msg));
                    emit_event(&app_handle, step_count, msg, "WARNING");
                    forced_next_tool = Some(("TOOL_THINK".to_string(), "Analizar auditorias previas y decidir siguiente paso. Si los archivos ya existen usa TOOL_PROGRAMMER para mejorarlos o TOOL_FINISH si todo está correcto.".to_string()));
                } else {
                    emit_event(&app_handle, step_count, "Auditando archivos locales...", "ACTION");
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
                    emit_event(&app_handle, step_count, &format!("Auditoria completada. {} archivos.", files_to_audit.len()), "SUCCESS");
                }
                mandatory_tools_executed.insert("TOOL_AUDITOR".to_string());
            },
            "TOOL_LOGIC_SOLVER" => {
                emit_event(&app_handle, step_count, "Iniciando Z3-Logic Solver...", "ACTION");
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
                let reporte = delegate_to_logic_solver(&safe_files, &orchestrator_model).await;
                current_context.push_str(&format!("Reporte de Verificación Formal (Logic Solver):\n{}\n\nRevisa los problemas matemáticos o lógicos detectados antes de programar o testear.\n\n", reporte));
                emit_event(&app_handle, step_count, "Verificación Lógica completada.", "SUCCESS");
            },
            "TOOL_WORKSPACE_MANAGER" => {
                emit_event(&app_handle, step_count, "Gestionando archivos del workspace...", "ACTION");
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
                    emit_event(&app_handle, step_count, err_msg, "ERROR");
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
                    emit_event(&app_handle, step_count, &format!("Limpieza finalizada. {} borrados.", borrados.len()), "SUCCESS");
                }
            },
            "TOOL_THINK" => {
                    think_consecutive += 1;
                    if think_consecutive > 3 {
                        emit_event(&app_handle, step_count, "[COOLDOWN] Bucle TOOL_THINK interceptado. Usa otra herramienta.", "WARNING");
                        current_context.push_str(&format!("PASO {}:\nTOOL_THINK bloqueado: bucle detectado. DEBES usar TOOL_PROGRAMMER, TOOL_TERMINAL o TOOL_FINISH.\n\n", step_count));
                        forced_next_tool = Some(("TOOL_PROGRAMMER".to_string(), "Forzado para romper bucle de reflexion. Escribe codigo real.".to_string()));
                    } else {
                        emit_event(&app_handle, step_count, "Pensando y planificando...", "ACTION");
                        current_context.push_str(&format!("Reflexion Interna del Agente: {}\n\n", &comando));
                        emit_event(&app_handle, step_count, "Reflexion completada.", "SUCCESS");
                        // Sprint 1+2 FSM: Planner -> Executor on TOOL_THINK (except Analysis missions)
                        if current_role == AgentRole::Planner {
                            if mission_type == MissionType::Analysis {
                                emit_event(&app_handle, step_count, "[FSM] MODO ANALISIS: Planificador permanece activo. Usa TOOL_FINISH para responder.", "INFO");
                            } else {
                                if !comando.trim().is_empty() {
                                    acceptance_contract = Some(formato_contrato(&comando));
                                    emit_event(&app_handle, step_count, &format!("[CONTRATO] Criterios definidos: {}", comando.chars().take(80).collect::<String>()), "INFO");
                                }
                                current_role = AgentRole::Executor;
                                critic_feedback = None;
                                emit_event(&app_handle, step_count, "[FSM] PLANIFICADOR -> EJECUTOR: Plan aprobado. Iniciando escritura de codigo.", "INFO");
                            }
                        }
                    }
            },
            "TOOL_MAPPER" => {
                mapper_consecutive += 1;
                if mapper_consecutive > 1 {
                    let msg = "[SISTEMA INTERNO]: Loop de TOOL_MAPPER detectado. El workspace no cambiara magicamente. FORZANDO TOOL_THINK en el siguiente turno.";
                    current_context.push_str(&format!("{}\n\n", msg));
                    emit_event(&app_handle, step_count, msg, "WARNING");
                    forced_next_tool = Some(("TOOL_THINK".to_string(), "El mapper ha terminado. Iniciar ejecución de plan.".to_string()));
                } else {
                    emit_event(&app_handle, step_count, "🗺️ Iniciando análisis de dependencias del workspace...", "ACTION");
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
                    emit_event(&app_handle, step_count, &summary, "SUCCESS");
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
                
                if is_cooldown_blocked {
                    programmer_cooldown_hits += 1;
                    if programmer_cooldown_hits >= 3 {
                        let res_msg = "[SISTEMA INTERCEPTO] Error Crítico: Bucle infinito de TOOL_PROGRAMMER detectado. Abortando misión.";
                        emit_event(&app_handle, step_count, res_msg, "FATAL");
                        let final_res = FinalResponse {
                            status: "ERROR".to_string(),
                            respuesta_conversacional: format!("Me he quedado atascado editando repetidamente el mismo archivo ({:?}) sin probarlo en la terminal. He detenido la ejecución por seguridad.", archivos_vec),
                        };
                        crate::llm::router::record_model_result(&orchestrator_model, &crate::llm::router::TaskType::Orchestrator, final_res.status == "FINISH", step_count);
                        return Ok(serde_json::to_string(&final_res).unwrap());
                    } else {
                        let interception = "[SISTEMA INTERCEPTO] Error Lógico: Estás intentando editar los mismos archivos por segunda vez consecutiva sin haber probado tu código en la terminal. DEBES ejecutar 'TOOL_TERMINAL' para probar el script y ver los errores antes de seguir programando.";
                        current_context.push_str(&format!("{}\n\n", interception));
                        emit_event(&app_handle, step_count, "Bucle interceptado por Cooldown", "WARNING");
                        forced_next_tool = Some(("TOOL_TERMINAL".to_string(), "Se forzó 'TOOL_TERMINAL'. DEBES ejecutar el script en la terminal para probarlo ahora mismo antes de seguir programando. RECUERDA: Debes proporcionar un comando válido en el campo 'comando' (ej. 'python main.py', 'npm start', o 'ls'). NO DEJES EL COMANDO VACÍO.".to_string()));
                    }
                } else {
                    // Valid programming action. 
                    // Clear the terminal history so the LLM must test again after this programming phase.
                    comandos_ejecutados_historico.clear();
                let safe_files = memory::read_files_safely(&workspace_path, archivos_vec.clone()).await;
                let context_for_qwen = format!("Historial Bucle:\n{}\nArchivos:\n{}", current_context, safe_files);
                
                let mut qwen_prompt = format!("Instrucción principal: {}\nDEBES crear/modificar los archivos solicitados con implementaciones COMPLETAS y REALES. PROHIBIDO usar 'pass', 'TODO', funciones vacías, NotImplementedError o cualquier placeholder. Cada función debe tener lógica funcional real.", user_message);
                let target_model = programmer_model.clone();
                emit_event(&app_handle, step_count, &format!("[ROUTER] Cerebro Programador Seleccionado: {}", target_model), "INFO");

                let mut exito_bucle_programador = false;
                let mut max_intentos = 3;
                
                while max_intentos > 0 && !exito_bucle_programador {
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
                            if let Ok(prog_output) = serde_json::from_str::<ProgrammerOutput>(&clean_json_res) {
                                if !prog_output.cambios.is_empty() {
                                    emit_event(&app_handle, step_count, "Activando Git-Shield: Creando punto de retorno...", "PLANNING");
                                    if let Err(e) = crate::core::create_git_backup(&workspace_path, "Aura-Sentinel: Git-Shield Auto-Backup").await {
                                        emit_event(&app_handle, step_count, &format!("Fallo Crítico en Git-Shield: {}", e), "FATAL");
                                        current_context.push_str(&format!("Git-Shield Error: {}. NO SE REALIZARON CAMBIOS.\n\n", e));
                                        break;
                                    }
                                    match memory::apply_code_changes(&workspace_path, prog_output.cambios.clone()).await {
                                        Ok(msg) => {
                                            emit_event(&app_handle, step_count, &msg, "SUCCESS");
                                            
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
                                                emit_event(&app_handle, step_count, &format!("[ANTI-STUB] ❌ {} archivo(s) rechazados por código incompleto. Exigiendo reescritura...", stub_rejections.len()), "FATAL");
                                                // Rollback the written files
                                                let _ = crate::core::restore_git_backup(&workspace_path).await;
                                                qwen_prompt = format!(
                                                    "{}\n\n[ERROR DE REVISIÓN]: Los archivos fueron rechazados por el sistema Anti-Stub:\n{}\n\nREESCRIBE COMPLETAMENTE O CORRIGE EL CÓDIGO. IMPLEMENTACIÓN REAL OBLIGATORIA.",
                                                    qwen_prompt, combined
                                                );
                                                max_intentos -= 1;
                                            } else {
                                                // Stubs passed — now run full compile validation
                                                emit_event(&app_handle, step_count, "Validando compilación...", "VALIDATING");
                                                match validate_workspace(&workspace_path).await {
                                                    Ok(_) => {
                                                        emit_event(&app_handle, step_count, "Validación exitosa.", "SUCCESS");
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
                                                                        emit_event(&app_handle, step_count, &format!("[PESP] ✅ Micro-Meta [{}/{}] VERIFICADA.", journal.micro_meta_actual + 1, journal.micro_metas.len()), "SUCCESS");
                                                                        if journal.micro_meta_actual + 1 < journal.micro_metas.len() {
                                                                            journal.micro_meta_actual += 1;
                                                                            let next = journal.micro_metas[journal.micro_meta_actual].descripcion.clone();
                                                                            emit_event(&app_handle, step_count, &format!("[PESP] 🔄 Avanzando a Micro-Meta [{}/{}]: {}", journal.micro_meta_actual + 1, journal.micro_metas.len(), next), "INFO");
                                                                        }
                                                                    } else {
                                                                        mm.estado = "EN_PROGRESO".to_string();
                                                                    }
                                                                    crate::core::session_journal::save_journal(&workspace_path, &journal);
                                                                }
                                                            }

                                                            let explicit_msg = format!("Programador: Los archivos {:?} fueron escritos con éxito, Anti-Stub APROBADO.\n⚠️ REGLA DE ESTADO OBLIGATORIA: Ahora DEBES usar 'TOOL_TERMINAL' en tu próximo turno para ejecutar el script o archivo principal y verificar que funciona sin errores. NO repitas TOOL_PROGRAMMER ni uses TOOL_FINISH hasta ver los resultados en la terminal.\n\n", written_files);
                                                            current_context.push_str(&explicit_msg);
                                                            exito_bucle_programador = true;
                                                            comandos_ejecutados_historico.clear();
                                                            // Sprint 2: Micrometa-gated Executor->Critic transition
                                                            let all_metas_done = journal.micro_metas.is_empty()
                                                                || journal.micro_metas.iter().all(|mm| mm.estado == "VERIFICADA");
                                                            if all_metas_done {
                                                                current_role = AgentRole::Critic;
                                                                critic_feedback = None;
                                                                emit_event(&app_handle, step_count, "[FSM] EJECUTOR -> CRITICO: Todas las micro-metas completadas. Iniciando validacion.", "INFO");
                                                            } else {
                                                                let pending: Vec<String> = journal.micro_metas.iter()
                                                                    .filter(|mm| mm.estado != "VERIFICADA")
                                                                    .map(|mm| mm.descripcion.clone())
                                                                    .collect();
                                                                let pending_str = pending.join(", ");
                                                                let msg = format!("[FSM] Ejecutor permanece activo. Micro-metas pendientes: {}. Completa todos los archivos antes de pasar al Critico.", pending_str);
                                                                current_context.push_str(&format!("{}\n\n", &msg));
                                                                emit_event(&app_handle, step_count, &msg, "INFO");
                                                            }
                                                            for f in &written_files {
                                                                archivos_editados_historico.insert(f.clone());
                                                            }
                                                        } else {
                                                            // Integration failed — revert and force fix
                                                            let _ = crate::core::restore_git_backup(&workspace_path).await;
                                                            archivos_editados_historico.clear();
                                                            max_intentos -= 1;
                                                        }
                                                    },
                                                    Err(e) => {
                                                        emit_event(&app_handle, step_count, &format!("Error detectado: {}", e), "ERROR");
                                                        qwen_prompt = format!("{}\n\n[ERROR DE COMPILACIÓN/EJECUCIÓN]: El código que generaste causó este error:\n{}\n\nSoluciónalo y genera un nuevo JSON asegurándote de escapar correctamente los strings. REGLA ESTRICTA: DEBES RESPONDER ÚNICAMENTE CON UN JSON VÁLIDO (sin texto fuera del JSON).", qwen_prompt, e);
                                                        if e.contains("package.json") && !archivos_vec.contains(&"package.json".to_string()) {
                                                            archivos_vec.push("package.json".to_string());
                                                        }
                                                        max_intentos -= 1;
                                                    }
                                                }
                                            } // end stubs-clean else
                                        },
                                        Err(e) => {
                                            emit_event(&app_handle, step_count, &format!("Error escribiendo archivos: {}", e), "ERROR");
                                            current_context.push_str(&format!("Programador Falló al escribir: {}\n\n", e));
                                            break;
                                        }
                                    }
                                } else {
                                    // No changes proposed — this can be legitimate (files already correct)
                                    emit_event(&app_handle, step_count, "El programador no propuso cambios. Los archivos ya están al día.", "WARNING");
                                    current_context.push_str("Programador: No se propusieron cambios (los archivos ya están correctos o no hay nada que modificar). El Ejecutor puede usar TOOL_TERMINAL para verificar o TOOL_FINISH si la tarea está lista.\n\n");
                                    exito_bucle_programador = true; // treat as success, not failure
                                    break;
                                }
                            } else {
                                emit_event(&app_handle, step_count, &format!("El programador devolvió JSON inválido (Intento {} restantes)", max_intentos - 1), "ERROR");
                                qwen_prompt = format!("{}\n\n[ERROR CRÍTICO]: Tu respuesta anterior NO fue un JSON válido o falló la serialización. REGLA ESTRICTA: Tu salida debe ser ÚNICAMENTE un objeto JSON analizable, sin texto adicional antes o después. Si usas bloques markdown, asegúrate de que el JSON interno sea correcto.", qwen_prompt);
                                max_intentos -= 1;
                            }
                        },
                        Err(e) => {
                            emit_event(&app_handle, step_count, &format!("Error llamando a Qwen: {}", e), "ERROR");
                            current_context.push_str(&format!("Programador: Falla de red: {}\n\n", e));
                            break;
                        }
                    }
                }
                
                if !exito_bucle_programador {
                    emit_event(&app_handle, step_count, "El programador devolvió JSON inválido o falló completamente. Forzando fin.", "FATAL");
                    current_context.push_str("Programador: Fracasó tras múltiples intentos o JSON inválido. [SISTEMA] FORZANDO TOOL_FINISH.\n\n");
                    forced_next_tool = Some(("TOOL_FINISH".to_string(), "El programador no pudo resolver los errores de sintaxis tras varios intentos.".to_string()));
                } else if exito_bucle_programador {
                    // TOOL_PROGRAMMER succeeded — reset the NoTests loop counter
                    // so TOOL_TESTER can be used again to verify the newly created test files
                    no_tests_consecutive = 0;
                }
                }
            },

            "TOOL_ARCHITECT" => {
                if architect_used {
                    emit_event(&app_handle, step_count, "Bucle interceptado por Cooldown (Architect)", "WARNING");
                    current_context.push_str(&format!("PASO {}:\nAcción: TOOL_ARCHITECT\nResultado: [SISTEMA INTERCEPTO] Error: Ya ejecutaste TOOL_ARCHITECT en este bucle. Tu única opción válida ahora es usar TOOL_FINISH para detenerte y resumir los resultados al usuario.\n\n", step_count));
                } else {
                    architect_used = true;
                    emit_event(&app_handle, step_count, "Generando mapa arquitectónico del sistema...", "ACTION");
                    let graph = crate::core::dependency_mapper::analyze_workspace(&workspace_path);
                    let report = crate::core::dependency_mapper::format_graph_report(&graph);
                    current_context.push_str(&format!("Reporte Arquitectónico:\n{}\n\n", report));
                    emit_event(&app_handle, step_count, "Mapa arquitectónico generado.", "SUCCESS");
                }
            },
            "TOOL_VISION_EVALUATOR" => {
                emit_event(&app_handle, step_count, "[VISION] Capturando pantalla y evaluando calidad visual...", "ACTION");
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
                    emit_event(&app_handle, step_count, &format!("[VISION] Abriendo navegador en {}", url), "ACTION");
                    
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
                        emit_event(&app_handle, step_count, &format!("[VISION] Evaluacion completada: {}", &vision_result.chars().take(120).collect::<String>()), "SUCCESS");
                    },
                    Err(e) => {
                        let msg = format!("[VISION] Error al capturar pantalla: {}. Verifica que haya una ventana abierta.", e);
                        current_context.push_str(&format!("{}\n\n", &msg));
                        emit_event(&app_handle, step_count, &msg, "ERROR");
                    }
                }
                // Mark as executed for mandatory checklist (whether it succeeded or not)
                mandatory_tools_executed.insert("TOOL_VISION_EVALUATOR".to_string());
            },
            "TOOL_TESTER" => {
                emit_event(&app_handle, step_count, "Ejecutando suite de pruebas automatizadas...", "ACTION");
                // Mark as executed for mandatory checklist regardless of result
                mandatory_tools_executed.insert("TOOL_TESTER".to_string());
                match crate::core::tester::run_tests(&workspace_path).await {
                    crate::core::tester::TestResult::NoTests => {
                        // ── Web project detection ────────────────────────────────────────────
                        // HTML/CSS/JS projects have no test runner. Treat them as PASSED and
                        // auto-advance to TOOL_VISION_EVALUATOR instead of looping endlessly.
                        let is_web_project = std::path::Path::new(&workspace_path).join("index.html").exists();
                        if is_web_project {
                            let web_pass_msg = "[TOOL_TESTER] Proyecto web estático detectado (index.html encontrado). \
                                No existe una suite de tests unitarios, pero el código ha sido VALIDADO VISUALMENTE. \
                                TESTER: APROBADO para proyectos web. \
                                SIGUIENTE PASO OBLIGATORIO: usa TOOL_VISION_EVALUATOR para verificar la UI en pantalla.";
                            current_context.push_str(&format!("{}\n\n", web_pass_msg));
                            emit_event(&app_handle, step_count, "[TESTER] Proyecto web → APROBADO. Forzando TOOL_VISION_EVALUATOR.", "SUCCESS");
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
                            emit_event(&app_handle, step_count, &format!("[SISTEMA] Cambiando estrategia a TOOL_TERMINAL tras {} intentos.", no_tests_consecutive), "WARNING");
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
                            emit_event(&app_handle, step_count, "Sin suite de tests detectada. Usa TOOL_TERMINAL para scripts simples.", "WARNING");
                        } // end else (non-web project)
                        } // end else (is_web_project check)
                    },
                    crate::core::tester::TestResult::Passed(success_msg) => {
                        tester_attempts = 0;
                        if tester_success_hits >= 1 {
                            let res_msg = "[SISTEMA INTERCEPTO] Error Crítico: Bucle infinito de pruebas exitosas detectado. Abortando misión.";
                            emit_event(&app_handle, step_count, res_msg, "FATAL");
                            let final_res = FinalResponse {
                                status: "FINISH".to_string(),
                                respuesta_conversacional: "Los tests ya pasaron con éxito, pero me quedé atascado ejecutándolos en bucle. He detenido el proceso para evitar un ciclo infinito. Misión cumplida.".to_string(),
                            };
                            crate::llm::router::record_model_result(&orchestrator_model, &crate::llm::router::TaskType::Orchestrator, final_res.status == "FINISH", step_count);
                            return Ok(serde_json::to_string(&final_res).unwrap());
                        } else {
                            tester_success_hits += 1;
                            current_context.push_str(&format!("Resultado Tests:\n{}\n\n[INSTRUCCIÓN ESTRICTA DE SEGURIDAD]: LOS TESTS PASARON EXITOSAMENTE. LA TAREA ESTÁ COMPLETADA. EN TU SIGUIENTE PASO DEBES ELEGIR OBLIGATORIAMENTE 'TOOL_FINISH'. NO REPITAS TOOL_TESTER.\n\n", success_msg));
                            emit_event(&app_handle, step_count, "Todos los tests pasaron exitosamente. Iniciando Auto-Indexación...", "SUCCESS");
                            // AUTO INDEXACIÓN SILENCIOSA
                            if let Ok(msg) = crate::core::memory::index_project(&workspace_path).await {
                                current_context.push_str(&format!("Memoria Vectorial: {}\n\n", msg));
                            }
                        }
                    },
                    crate::core::tester::TestResult::Failed(fail_msg) => {
                        tester_attempts += 1;
                        if tester_attempts >= 3 {
                            emit_event(&app_handle, step_count, "[CRITICAL_FAILURE] Fallos de test superan el límite (3). Revertiendo...", "FATAL");
                            let _ = crate::core::restore_git_backup(&workspace_path).await;
                            let final_res = FinalResponse {
                                status: "FINISH".to_string(),
                                respuesta_conversacional: "He alcanzado el límite máximo de fallos de pruebas. El código era inviable. He restaurado el proyecto a su último estado funcional (Rollback). Por favor, revisa mi código y ayuda a solucionar los tests.".to_string(),
                            };
                            crate::llm::router::record_model_result(&orchestrator_model, &crate::llm::router::TaskType::Orchestrator, final_res.status == "FINISH", step_count);
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
                                        emit_event(&app_handle, step_count,
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
                                                emit_event(&app_handle, step_count,
                                                    &format!("'{}' instalado. Reintenta TOOL_TESTER.", binary),
                                                    "SUCCESS");
                                            },
                                            Err(install_err) => {
                                                current_context.push_str(&format!(
                                                    "[AUTO-ENV FALLÓ] No se pudo instalar '{}': {}\n\
                                                     Requiere intervención manual. Usa TOOL_FINISH.\n\n",
                                                    binary, install_err
                                                ));
                                                emit_event(&app_handle, step_count,
                                                    &format!("Fallo al auto-instalar '{}': {}", binary, install_err),
                                                    "ERROR");
                                            }
                                        }
                                    } else {
                                        // Already attempted or no binary name found — fall back to guidance
                                    emit_event(&app_handle, step_count, "Tests fallaron por dependencias faltantes. Solicito TOOL_TERMINAL...", "ERROR");
                                    current_context.push_str(&format!(
                                        "[AUTO-FIX DEPENDENCIAS] Los tests fallaron por dependencias o configuración faltante:\n{}\n\n\
                                        Tu SIGUIENTE PASO OBLIGATORIO es usar 'TOOL_TERMINAL' con 'npm install' o el instalador adecuado al lenguaje.\n\n",
                                        fail_msg
                                    ));
                                    
                                }
                            } else {
                                // Dependency error but no specific binary — guide the LLM
                                emit_event(&app_handle, step_count, "Tests fallaron por dependencias faltantes. Solicito TOOL_TERMINAL...", "ERROR");
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
                                emit_event(&app_handle, step_count, "Tests fallaron. Revertiendo cambios y activando Auto-Debugger...", "ERROR");
                                let _ = crate::core::restore_git_backup(&workspace_path).await;
                                archivos_editados_historico.clear();
                                comandos_ejecutados_historico.clear();
                                _task_complexity = crate::llm::router::TaskContext { task_type: crate::llm::router::TaskType::HighComplexityFix, language: None };
                            emit_event(&app_handle, step_count, "[ROUTER] Tarea compleja detectada tras fallo de tests. Escalando modelo experto...", "ACTION");
                                current_role = AgentRole::Executor;
                                critic_feedback = Some(fail_msg.clone());
                                emit_event(&app_handle, step_count, "[FSM] 🔬 CRÍTICO → ⚙️ EJECUTOR: Tests fallaron, enviando feedback al Ejecutor.", "WARNING");
                                current_context.push_str(&format!("[AUTO-DEBUGGER] Los tests fallaron estrepitosamente:\n{}\n\nEl sistema ha restaurado el código usando Git-Shield. Debes generar una nueva y mejor solución usando TOOL_PROGRAMMER.\n", fail_msg));
                                forced_next_tool = Some(("TOOL_PROGRAMMER".to_string(), "Los tests fallaron estrepitosamente, el sistema forzó TOOL_PROGRAMMER para que generes una nueva y mejor solución.".to_string()));
                            }
                        }
                    }
                }
            },
            "TOOL_AST_INJECT" => {
                emit_event(&app_handle, step_count, "Inyectando nodos AST en Memoria Lógica (Zero-Trace)...", "ACTION");
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
                current_context.push_str(&format!("PASO {}:\nAcción: TOOL_AST_INJECT\nResultado:\n{}\n\n", step_count, report));
                emit_event(&app_handle, step_count, &format!("{} nodos AST inyectados exitosamente en RAM.", ast_nodes_vec.len()), "SUCCESS");
            },
            "TOOL_LEARN" => {
                learn_consecutive += 1;
                if learn_consecutive > 1 {
                    let msg = "[SISTEMA INTERNO]: Ya has aprendido este proyecto (loop infinito TOOL_LEARN detectado). DEBES USAR TOOL_FINISH INMEDIATAMENTE PARA TERMINAR LA TAREA.";
                    forced_next_tool = Some(("TOOL_FINISH".to_string(), "La memoria ya está indexada, finalizando tarea obligatoriamente.".to_string()));
                    current_context.push_str(&format!("{}\n\n", msg));
                    emit_event(&app_handle, step_count, "Bucle de TOOL_LEARN detectado, forzando finalización.", "WARNING");
                } else {
                    emit_event(&app_handle, step_count, "Guardando conocimiento en la Memoria Permanente (RAG)...", "ACTION");
                    match crate::core::memory::index_project(&workspace_path).await {
                        Ok(msg) => {
                            current_context.push_str(&format!("Resultado TOOL_LEARN: {}\n\n", msg));
                            emit_event(&app_handle, step_count, "Memoria indexada correctamente.", "SUCCESS");
                        },
                        Err(err) => {
                            current_context.push_str(&format!("Error en TOOL_LEARN: {}\n\n", err));
                            emit_event(&app_handle, step_count, &err, "ERROR");
                        }
                    }
                }
            },
            "TOOL_CREATE_RUNNER" => {
                emit_event(&app_handle, step_count, "🏃 Generando runners de ejecución (test, build, dev, lint)...", "ACTION");
                let runners = generate_project_runners(&workspace_path, &original_prompt_parsed).await;
                if runners.is_empty() {
                    current_context.push_str("[TOOL_CREATE_RUNNER] No se generaron runners (lenguaje no detectado o no soportado).\n\n");
                    emit_event(&app_handle, step_count, "No se generaron runners (lenguaje desconocido).", "WARNING");
                } else {
                    let names: Vec<String> = runners.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
                    current_context.push_str(&format!("[TOOL_CREATE_RUNNER] Runners generados: {}\n\n", names.join(", ")));
                    emit_event(&app_handle, step_count, &format!("Runners generados: {}", names.join(", ")), "SUCCESS");
                }
            },
            "TOOL_SEARCH" => {
                // FALLO #6 FIX: use `comando` as query first, fall back to `url_a_investigar`.
                let search_query = if !comando.trim().is_empty() { comando.clone() } else { url.clone() };
                emit_event(&app_handle, step_count, &format!("Consultando Memoria Permanente para: {}", search_query), "ACTION");
                match crate::core::memory::query_memory(&search_query).await {
                    Ok(msg) => {
                        current_context.push_str(&format!("Resultado TOOL_SEARCH:\n{}\n\n", msg));
                        emit_event(&app_handle, step_count, "Búsqueda en memoria completada.", "SUCCESS");
                    },
                    Err(err) => {
                        current_context.push_str(&format!("Error en TOOL_SEARCH: {}\n\n", err));
                        emit_event(&app_handle, step_count, &err, "ERROR");
                    }
                }
            },
            "TOOL_ASK_USER" => {
                emit_event(&app_handle, step_count, "Solicitando información al usuario...", "ACTION");
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
                        emit_event(&app_handle, step_count, "Respuesta del usuario recibida.", "SUCCESS");
                    },
                    Err(e) => {
                        current_context.push_str(&format!("Error al consultar al usuario: {}\n\n", e));
                        emit_event(&app_handle, step_count, &format!("Error ASK_USER: {}", e), "ERROR");
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
                    emit_event(&app_handle, step_count,
                        &format!("[FINISH BLOQUEADO] Faltan herramientas obligatorias: {:?}", missing_list),
                        "WARNING");
                    step_count += 1;
                    continue;
                }

                // =======================================================
                // PESP v2 — Intercept TOOL_FINISH for Phase Advancement
                // =======================================================
                if !journal.fases.is_empty() && journal.fase_actual < journal.fases.len() - 1 {
                    let phase_num = journal.fases[journal.fase_actual].numero;
                    let phase_desc = &journal.fases[journal.fase_actual].descripcion;
                    
                    emit_event(&app_handle, step_count, &format!("⏸ [PAUSA INTERACTIVA] Fase {} completada. Esperando aprobación del usuario...", phase_num), "WARNING");
                    
                    let question = format!("He completado la Fase {}: '{}'. ¿Deseas que avance a la siguiente fase, o quieres revisar/cambiar algo?", phase_num, phase_desc);
                    let options = vec!["Aprobar y Continuar".to_string(), "Modificar Instrucciones".to_string(), "Detener Agente".to_string()];
                    
                    // Pause execution and ask user
                    match crate::core::ask_user::ask_user_async(&app_handle, question, options, current_context.clone()).await {
                        Ok(reply) => {
                            if reply == "Detener Agente" {
                                emit_event(&app_handle, step_count, "El usuario detuvo la ejecución.", "ERROR");
                                let final_res = FinalResponse { status: "FINISH".to_string(), respuesta_conversacional: "Detenido por el usuario".to_string() };
                                return Ok(serde_json::to_string(&final_res).unwrap());
                            } else if reply != "Aprobar y Continuar" {
                                let user_feedback = format!("[FEEDBACK DEL USUARIO EN PAUSA INTERACTIVA]: {}", reply);
                                current_context.push_str(&format!("{}\n\n", user_feedback));
                                emit_event(&app_handle, step_count, "Feedback del usuario recibido. Ajustando plan.", "ACTION");
                                step_count += 1;
                                continue;
                            }
                        },
                        Err(e) => {
                            emit_event(&app_handle, step_count, &format!("Pausa interactiva interrumpida: {}", e), "ERROR");
                            let final_res = FinalResponse { status: "FINISH".to_string(), respuesta_conversacional: "Interrumpido".to_string() };
                            return Ok(serde_json::to_string(&final_res).unwrap());
                        }
                    }
                    emit_event(&app_handle, step_count, &format!("✅ [FASE {} COMPLETADA] Avanzando a la siguiente...", journal.fases[journal.fase_actual].numero), "SUCCESS");
                    
                    // Mark current phase as completed
                    journal.fases[journal.fase_actual].estado = "COMPLETADA".to_string();
                    // Advance to next phase
                    journal.fase_actual += 1;
                    journal.fases[journal.fase_actual].estado = "EN_PROGRESO".to_string();
                    crate::core::session_journal::save_journal(&workspace_path, &journal);
                    
                    let new_phase_msg = format!(
                        "[SISTEMA PESP] Fase anterior completada. Iniciando Fase {}/{}: {}\nUsa TOOL_PROGRAMMER o TOOL_THINK para comenzar el trabajo de esta nueva fase.",
                        journal.fase_actual + 1, journal.fases.len(), journal.fases[journal.fase_actual].descripcion
                    );
                    current_context.push_str(&format!("{}\n\n", new_phase_msg));
                    current_role = AgentRole::Planner; // Reset role
                    
                    step_count += 1;
                    continue; // Do NOT terminate the agent loop
                } else if !journal.fases.is_empty() && journal.fase_actual == journal.fases.len() - 1 {
                    journal.fases[journal.fase_actual].estado = "COMPLETADA".to_string();
                }
                emit_event(&app_handle, step_count, "Bucle completado exitosamente.", "FINISH");
                // ── Journal: mark completed ──
                crate::core::session_journal::close_journal(&mut journal, "COMPLETADO", &workspace_path);
                let final_res = FinalResponse {
                    status: "FINISH".to_string(),
                    respuesta_conversacional: respuesta_conv,
                };
                crate::llm::router::record_model_result(&orchestrator_model, &crate::llm::router::TaskType::Orchestrator, final_res.status == "FINISH", step_count);
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
                    emit_event(&app_handle, step_count, &format!("[AUTO-REDIRECT] '{}' → TOOL_TERMINAL: {}", tool, terminal_cmd), "WARNING");
                    current_context.push_str(&format!(
                        "[SISTEMA]: La herramienta '{}' no existe. Fue redirigida automáticamente a TOOL_TERMINAL con el comando '{}'.\n\
                        RECUERDA: Para ejecutar comandos de shell SIEMPRE usa TOOL_TERMINAL con el campo 'comando'. Ejemplos: TOOL_TERMINAL+npm audit fix, TOOL_TERMINAL+npm install, TOOL_TERMINAL+pip install X.\n\n",
                        tool, terminal_cmd
                    ));
                    tool = format!("TOOL_TERMINAL");
                    comando = terminal_cmd;
                    // Re-enter as TOOL_TERMINAL by continuing the outer loop
                    // We need to push back and re-process — instead, execute inline
                    emit_event(&app_handle, step_count, &format!("Ejecutando en terminal: {}", comando), "ACTION");
                    comandos_ejecutados_historico.insert(comando.clone());
                    match execute_terminal_command(&workspace_path, &comando).await {
                        Ok(out) => {
                            current_context.push_str(&format!("Resultado TOOL_TERMINAL (auto): {}\n\n", out));
                            emit_event(&app_handle, step_count, &format!("Auto-terminal OK: {}", &out[..out.len().min(120)]), "SUCCESS");
                        }
                        Err(e) => {
                            current_context.push_str(&format!("Resultado TOOL_TERMINAL (auto) Error: {}\n\n", e));
                            emit_event(&app_handle, step_count, &format!("Auto-terminal Error: {}", e), "ERROR");
                        }
                    }
                } else {
                    unknown_tool_consecutive += 1;
                    emit_event(&app_handle, step_count, &format!("Herramienta desconocida: {}", tool), "WARNING");
                    if unknown_tool_consecutive >= 3 {
                        forced_next_tool = Some((
                            "TOOL_THINK".to_string(),
                            "El sistema bloqueó mi acceso porque intenté usar herramientas inventadas que no existen en el prompt. Transfiero el control para evitar bucles de alucinación.".to_string()
                        ));
                        unknown_tool_consecutive = 0;
                        current_context.push_str(&format!("Error crítico: uso de herramienta desconocida '{}'. [FSM FORZANDO TOOL_THINK]\n\n", tool));
                        emit_event(&app_handle, step_count, "[FSM] Agente inventando herramientas. Forzando TOOL_THINK.", "WARNING");
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
        let trail_result = if tool == "TOOL_FINISH" { StepResult::Success } else if current_context.contains("ERROR") || current_context.contains("FATAL") { StepResult::Error } else { StepResult::Success };
        let trail_error = if trail_result == StepResult::Error { Some(format!("Tool: {}", tool)) } else { None };
        command_trail.add_step(
            step_count,
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
        
        step_count += 1;
    }
    
    emit_event(&app_handle, step_count, "Límite máximo de pasos alcanzado. Bucle abortado.", "FATAL");
    // ── Journal: mark as waiting for user ──
    journal.status = "ESPERANDO".to_string();
    crate::core::session_journal::save_journal(&workspace_path, &journal);
    let final_res = FinalResponse {
        status: "FINISH".to_string(),
        respuesta_conversacional: format!(
            "He alcanzado el límite máximo de {} pasos sin llegar a una conclusión. \
             Por favor, revisa el historial de pasos y proporciona más contexto.",
            max_steps
        ),
    };
    Ok(serde_json::to_string(&final_res).unwrap())
}

