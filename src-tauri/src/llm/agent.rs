use serde::Serialize;
use tauri::{AppHandle, Emitter};
use crate::memory;
use crate::core::{execute_terminal_command, start_background_task, read_task_logs, kill_task, validate_workspace, format_system_error};
use super::{call_ollama, delegate_to_programmer, delegate_to_auditor, delegate_to_logic_solver, ProgrammerOutput};

fn strip_think_tags(mut text: String) -> String {
    while let (Some(start), Some(end)) = (text.find("<think>"), text.find("</think>")) {
        if end + 8 <= text.len() {
            text.replace_range(start..end + 8, "");
        } else {
            break;
        }
    }
    
    // Also strip ```json and ``` if they exist wrapping the whole thing
    let text = text.trim();
    let text = text.strip_prefix("```json").unwrap_or(text);
    let text = text.strip_prefix("```").unwrap_or(text);
    let text = text.strip_suffix("```").unwrap_or(text);
    text.trim().to_string()
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

pub const ORCHESTRATOR_MODEL: &str = "llama3.1:8b";
pub const PROGRAMMER_MODEL: &str = "qwen2.5-coder:7b";

pub async fn run_agent_loop(
    user_message: String,
    workspace_path: String,
    tree_json: String,
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
    let mut no_tests_consecutive = 0u32;
    let mut forced_next_tool: Option<(String, String)> = None;
    let mut step_count = 1;
    let max_steps = 50000;
    let mut json_error_count = 0;

    // ── Session Journal ────────────────────────────────────────
    // Persist mission state to disk so sleep/restart doesn’t lose context.
    let mut journal = crate::core::session_journal::load_journal(&workspace_path);
    journal.objetivo = user_message.clone();
    journal.workspace_path = workspace_path.clone();
    journal.status = "EN_PROGRESO".to_string();
    journal.herramientas_usadas.clear();
    journal.archivos_tocados.clear();
    crate::core::session_journal::save_journal(&workspace_path, &journal);
    emit_event(&app_handle, 0, "[DIARIO] Misión registrada en diario de sesión.", "INFO");
    
    let mut task_complexity = crate::llm::router::TaskContext { task_type: crate::llm::router::TaskType::GeneralCode, language: None };
    
    while step_count <= max_steps {
        // ── PESP: Inject micro-meta progress status into context ─────────────
        // Tells the LLM exactly where it is in the global project plan every turn.
        if !journal.micro_metas.is_empty() {
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

        let agent_prompt = format!(
            "Eres el Cerebro Planificador de Aura-Sentinel. Eres un ingeniero políglota. Actualmente soportas [Python, JS/TS, Rust, Go, C++]. Antes de programar, detecta el lenguaje del proyecto y ajusta tus herramientas de validación al estándar del lenguaje detectado.\n\
            Funcionarás en un bucle autónomo. Analiza el Objetivo, el Contexto y el Historial para decidir UNA ÚNICA HERRAMIENTA a utilizar en este turno.\n\
            Objetivo Original: {}\n\
            Contexto del Proyecto (Archivos Actuales Reales FÍSICOS en el disco): {}\n\
            {}\n\
            REGLA DE REALIDAD FÍSICA: La lista de archivos de arriba es la ÚNICA VERDAD. Si un archivo NO aparece en esa lista, significa que NO EXISTE y debes crearlo (ya sea escribiéndolo a mano con TOOL_PROGRAMMER, o generándolo con TOOL_TERMINAL usando comandos como npx, cargo, pip, etc). ¡No asumas que ya existe!\n\
            Historial de Pasos Ejecutados Hasta Ahora:\n{}\n\n\
            REGLA DE ORO: Si ya ejecutaste todas las acciones que pidió el usuario en el Objetivo Original, tu ÚNICA opción válida es usar 'TOOL_FINISH'. NO repitas pasos ni inventes problemas que no existen.\n\n\
            PROTOCOLO DE INGENIERÍA PROFESIONAL (PESP) — OBLIGATORIO PARA PROYECTOS COMPLEJOS:\n\
            ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\
            SI el objetivo del usuario requiere MÚLTIPLES ARCHIVOS, MÓDULOS o COMPONENTES (ej: un juego, una API, un sistema con UI+lógica+datos):\n\
            PASO 0 OBLIGATORIO — TOOL_THINK: Antes de cualquier TOOL_PROGRAMMER, debes declarar:\n\
              a) CONTRATO DEL SISTEMA: lista TODOS los archivos del sistema final con sus firmas de funciones exactas\n\
              b) GRAFO DE DEPENDENCIAS: quién importa a quién\n\
              c) MICRO-METAS en orden de dependencias (los módulos más fundamentales primero)\n\
            REGLA DE MICRO-META: Cada TOOL_PROGRAMMER debe escribir UN SOLO archivo completamente (o 2 si son triviales y relacionados).\n\
            REGLA ANTI-STUB ABSOLUTA: PROHIBIDO escribir código con funciones vacías. Ejemplos de lo que está PROHIBIDO:\n\
              - Python: 'pass' como cuerpo de función o método\n\
              - Python: '# TODO', '# implement', '# Dibujar el tablero' seguido de pass\n\
              - Python: 'raise NotImplementedError'\n\
              - Rust: 'todo!()', 'unimplemented!()'\n\
              - JS/TS: 'throw new Error(\"not implemented\")'\n\
            CADA FUNCIÓN DEBE TENER LÓGICA REAL. Si no sabes cómo implementarla, pregunta al usuario ANTES de escribir.\n\
            REGLA ANTI-ALUCINACIÓN ESTRICTA: ¡NO ALUCINES! Nunca asumas que un archivo o backend ha sido creado a menos que VEAS en tu historial que tú mismo usaste explícitamente 'TOOL_PROGRAMMER' o 'TOOL_TERMINAL' (ej. npx/cargo) para crearlo. Planearlo en tu 'Checklist Mental' o usar 'TOOL_THINK' NO crea el código. ¡Debes programarlo físicamente!\n\
            [EJEMPLO DE CHECKLIST MENTAL CORRECTO]:\n\
              Paso 1: TOOL_THINK - Analizar dependencias de la tarea.\n\
              Paso 2: TOOL_PROGRAMMER - Crear el archivo_A del modelo de datos.\n\
              Paso 3: TOOL_TERMINAL - Ejecutar archivo_A para probar sintaxis.\n\
              Paso 4: TOOL_PROGRAMMER - Crear el archivo_B de la lógica de negocio.\n\
              Paso 5: TOOL_TERMINAL - Probar todo el sistema.\n\
              Paso 6: TOOL_FINISH - Terminar.\n\
            ¡NUNCA TE SALTES PASOS Y NUNCA USES PLACEHOLDERS!\n\
            REGLA DE VERIFICACIÓN DE INTEGRACIÓN: Después de escribir cada archivo, usa TOOL_TERMINAL para verificar que se importa sin errores.\n\
            ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\
            Catálogo de Herramientas (Elige SOLO UNA):\n\
            1. 'TOOL_TERMINAL': Para comandos síncronos y de un solo uso (npm install, pip, cargo build). Rellena 'comando' y 'task_id'.\n\
            2. 'TOOL_BACKGROUND_START': Para arrancar servidores que correrán infinitamente en segundo plano (python -m http.server, npm run dev). Rellena 'comando' y 'task_id'.\n\
            3. 'TOOL_BACKGROUND_READ': Para leer los logs en vivo de un servidor asíncrono. Rellena 'task_id'.\n\
            4. 'TOOL_BACKGROUND_KILL': Para apagar un servidor asíncrono. Rellena 'task_id'.\n\
            5. 'TOOL_PROGRAMMER': Para escribir o modificar código fuente físico en el disco. Rellena 'archivos_a_editar' con la lista de archivos.\n\
            6. 'TOOL_AUDITOR': Para auditar código estático o leer archivos locales si no sabes cómo están hechos. Rellena 'archivos_a_editar'.\n\
            7. 'TOOL_FINISH': Cuando el objetivo principal se haya completado con éxito, o si es imposible continuar. Rellena 'respuesta_conversacional' con la respuesta final para el usuario. ¡USALA SIEMPRE QUE HAYAS TERMINADO!\n\
            8. 'TOOL_ARCHITECT': Analiza la estructura y dependencias. No rellena argumentos. REGLA: Después de usarla, DEBES usar TOOL_FINISH obligatoriamente para resumirle los hallazgos al usuario.\n\
            9. 'TOOL_TESTER': Ejecuta suites de pruebas automatizadas SOLO si el proyecto tiene archivos de test (test_*.py, *.test.js, etc.) o configuración de test (pytest.ini, jest.config.js). ¡PELIGRO! Esta herramienta NO ESCRIBE ni implementa pruebas. SOLO LAS EJECUTA. Para scripts simples SIN archivos de test, usa TOOL_TERMINAL en su lugar. Para escribir o arreglar un test, usa TOOL_PROGRAMMER.\n\
            10. 'TOOL_LEARN': Indexa el conocimiento de un proyecto exitoso en la memoria permanente de Aura. Úsala si el proyecto funciona o después de un TOOL_TESTER exitoso. No requiere argumentos.\n\
            11. 'TOOL_SEARCH': Consulta explícitamente la memoria histórica para buscar cómo resolviste problemas similares antes. Rellena 'url_a_investigar' con el término de búsqueda.\n\
            12. 'TOOL_ENV_MANAGER': SOLO para instalar programas binarios base del Sistema Operativo (ej. python, node, git). PROHIBIDO usar para paquetes de lenguajes (pip/npm). Rellena 'comando' con el nombre del binario.
            14. 'TOOL_LOGIC_SOLVER': Solver Z3 de verificación matemática para encontrar bucles infinitos o dead code. ÚSALO SOLO si has fallado repetidamente al arreglar un código. NUNCA asumas que el usuario te pidió usar esto a menos que haya un error lógico persistente. No requiere argumentos.
            15. 'TOOL_WORKSPACE_MANAGER': Para borrar permanentemente archivos basura o temporales físicos del proyecto y mantener todo impecable. Rellena 'archivos_a_editar' con los archivos a borrar.
            16. 'TOOL_THINK': Para razonar internamente, pausar y planificar el siguiente paso sin afectar el entorno. Rellena 'comando' con tu pensamiento.\n\
            17. 'TOOL_MAPPER': Analiza ESTÁTICAMENTE las dependencias del workspace y genera un GRAFO DE CONOCIMIENTO. Úsalo OBLIGATORIAMENTE al inicio de cualquier proyecto con 3+ archivos. Devuelve: (a) orden de escritura topológico — en qué orden exacto debes crear los archivos para que sus imports funcionen desde el primer momento; (b) nodos críticos — módulos compartidos que deben implementarse PRIMERO; (c) dependencias circulares — ciclos a evitar. No requiere argumentos. REGLA: Después de usar TOOL_MAPPER, usa TOOL_THINK para elaborar el plan de implementación basado en el grafo antes de programar nada.\n\
            Antes de tomar tu decisión, DEBES rellenar el campo 'checklist_mental'. En este campo, enumera mentalmente todos los pasos que pidió el usuario, qué pasos ya se han cumplido en el historial, y cuál es el paso exacto que falta ahora mismo. \n\
            REGLA DE ORO DE FINALIZACIÓN: NUNCA puedes elegir la herramienta 'TOOL_FINISH' a menos que tu 'checklist_mental' confirme explícitamente que el 100% de los verbos solicitados (ej. 'crea', 'ejecuta', 'prueba', 'valida') se han ejecutado FÍSICAMENTE en el historial con ÉXITO. Si el usuario pidió 'ejecuta X', y X falló o no se ejecutó, ESTÁ ESTRICTAMENTE PROHIBIDO USAR TOOL_FINISH. En su lugar, debes usar TOOL_PROGRAMMER para arreglar el error y luego TOOL_TERMINAL para volver a probar. NUNCA te rindas.\n\n\
            MANUAL DE OPERACIONES ANTIGRAVITY (DOMAIN KNOWLEDGE):
            - Omitir Hallucinaciones: NUNCA asumas que el usuario te pidió realizar una acción solo porque leíste una descripción en la lista de herramientas o en este manual. Cíñete ESTRICTAMENTE al 'Objetivo Original'.
            REGLAS ESTRICTAS:
            1. CRÍTICO: Escribe los scripts y archivos SIEMPRE en la RAÍZ del proyecto a menos que el usuario especifique UNA SUBCARPETA EXPLÍCITA. Para tareas simples de UN SOLO ARCHIVO (scraper.py, script.py, etc.) NUNCA crees subcarpetas (como 'scraper_project/'). El archivo va DIRECTAMENTE en la raíz, sin carpetas contenedoras.
            2. Cuando uses TOOL_TERMINAL para ejecutar un script, DEBES usar el MISMO NOMBRE exacto del archivo que aparece en el Contexto del Proyecto arriba. NUNCA adivines ni inventes nombres de archivos. Lee el Contexto.
            3. Analiza con cuidado si ya usaste una herramienta y falló. No la repitas ciegamente.
            4. Si el historial dice que la tarea ya terminó, ignóralo si aún no has escrito y probado el código solicitado.
            - Resolución de Errores: Si un comando falla, lee los logs o la consola, usa 'TOOL_PROGRAMMER' para arreglar el código, y vuelve a intentar.
            - Auto-Testing: Si el proyecto es un script simple, ejecuta el script directamente en consola usando TOOL_TERMINAL. Si falla, usa TOOL_PROGRAMMER para repararlo.
            - Instalación de Librerías vs Binarios (CRÍTICO): Usa 'TOOL_TERMINAL' para instalar librerías de tu lenguaje (ej. 'pip install paquete_python', 'npm install paquete_node'). Usa 'TOOL_ENV_MANAGER' EXCLUSIVAMENTE para instalar binarios del sistema base (ej. 'python', 'node', 'git') si la consola dice 'is not recognized'. ¡ESTÁ PROHIBIDO usar TOOL_ENV_MANAGER para paquetes de pip o npm!
            - Auto-Healing (Pre-Flight): Si un comando falla por 'is not recognized', usa TOOL_ENV_MANAGER (si es binario base) o TOOL_TERMINAL (si es paquete de código de pip/npm). ¡NUNCA uses TOOL_ENV_MANAGER para errores de código (usa TOOL_PROGRAMMER)!

            REGLAS DE ESTADO (STATE MACHINE):
            - DESPUÉS de usar TOOL_PROGRAMMER con éxito, es OBLIGATORIO usar TOOL_TERMINAL o TOOL_TESTER para ejecutar y validar tus cambios. ADEMÁS, DEBES IGNORAR completamente cualquier error previo en el historial, ya que el código acaba de ser reparado.\n\
            - SI TOOL_TESTER FALLA, es OBLIGATORIO usar TOOL_PROGRAMMER en el siguiente paso para arreglar el código. ESTÁ PROHIBIDO usar TOOL_TESTER dos veces seguidas si los tests fallan.\n\
            - SI TOOL_TESTER TIENE ÉXITO, estás OBLIGADO a usar TOOL_FINISH en el siguiente paso. ESTÁ PROHIBIDO usar TOOL_TESTER dos veces seguidas si los tests pasaron.\n\
            - SI TOOL_TERMINAL TIENE ÉXITO y su salida demuestra que cumpliste el último objetivo del usuario, es OBLIGATORIO usar TOOL_FINISH en el siguiente paso. ¡PROHIBIDO repetir el mismo comando de TOOL_TERMINAL si ya funcionó!\n\
            - SI TOOL_TERMINAL falla constantemente, NO uses TOOL_FINISH. Debes usar TOOL_PROGRAMMER para investigar por qué falla, agregar logs de depuración (print), y volver a intentar con TOOL_TERMINAL.\n\
            - SI TOOL_ENV_MANAGER TIENE ÉXITO, NO PUEDES volver a usar TOOL_ENV_MANAGER. Debes continuar tu tarea con TOOL_TERMINAL, TOOL_PROGRAMMER o TOOL_TESTER.\n\
            - SI TOOL_ENV_MANAGER FALLA, es OBLIGATORIO usar TOOL_FINISH en el siguiente paso para pedir intervención manual.\n\
            - REGLA DE VERIFICACIÓN OBLIGATORIA: Si el mensaje original del usuario contiene palabras como 'prueba', 'pruébalo', 'ejecuta', 'ejecutalo', 'verifica', 'corre', 'abre', 'test', 'comprueba' o 'demuestra', entonces TOOL_TERMINAL ES OBLIGATORIO antes de TOOL_FINISH. No puedes declarar el trabajo terminado sin haber ejecutado físicamente el resultado y comprobado que no produce errores. Si el comando falla, debes corregirlo con TOOL_PROGRAMMER y volver a ejecutar.\n\
            - REGLA DE BORRADO: Si el usuario pide 'borra', 'elimina', 'borra el archivo X' como parte de la tarea, debes usar TOOL_WORKSPACE_MANAGER para borrar ese archivo PRIMERO antes de crear el nuevo. No puedes asumir que el archivo fue borrado por el historial.\n\n\
            Tu respuesta DEBE ser ÚNICAMENTE un objeto JSON con esta estructura exacta (sin markdown extra):\n\
            {{\n\
              \"checklist_mental\": \"<Análisis de tareas cumplidas vs faltantes>\",\n\
              \"herramienta\": \"<NOMBRE_HERRAMIENTA>\",\n\
              \"pensamiento\": \"Breve razonamiento lógico de tu decisión actual\",\n\
              \"comando\": \"<comando_a_ejecutar o null>\",\n\
              \"task_id\": \"<id_de_la_tarea o null>\",\n\
              \"url_a_investigar\": \"<url o null>\",\n\
              \"archivos_a_editar\": [\"ruta/archivo1\", \"ruta/archivo2\"],\n\
              \"respuesta_conversacional\": \"<respuesta al usuario o null>\"\n\
            }}",
            user_message, live_workspace_context, extra_prompt, current_context
        );
        
        let orchestrator_model = crate::llm::router::get_best_model(&crate::llm::router::TaskContext { task_type: crate::llm::router::TaskType::Orchestrator, language: None }, &available_models, &app_handle, 0).await
            .unwrap_or_else(|_| ORCHESTRATOR_MODEL.to_string());
        
        emit_event(&app_handle, step_count, &format!("Analizando estado y planificando siguiente paso con {}...", orchestrator_model), "PLANNING");

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
                    return Ok(serde_json::to_string(&final_res).unwrap());
                } else {
                    emit_event(&app_handle, step_count, &format!("Error de sintaxis JSON (intento {}/5). Reintentando...", json_error_count), "WARNING");
                    current_context.push_str(&format!("[SISTEMA INTERNO] Tu respuesta anterior no era un JSON válido. Error: {}. Genera SOLO un objeto JSON estrictamente válido según la estructura requerida, sin texto adicional antes o después del JSON.\n\n", e));
                    continue;
                }
            }
        };
        let checklist = raw_value.get("checklist_mental").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let tool = raw_value.get("herramienta").and_then(|v| v.as_str()).unwrap_or("UNKNOWN").to_uppercase();
        let pensamiento = raw_value.get("pensamiento").and_then(|v| v.as_str()).unwrap_or("Sin pensamiento").to_string();
        let comando = raw_value.get("comando").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let task_id = raw_value.get("task_id").and_then(|v| v.as_str()).unwrap_or("default_task").to_string();

        // ── FORCED TOOL VALIDATION ────────────────────────────────────────────
        // If the system has determined the LLM is stuck in a tool-loop,
        // validate its decision against the forced tool constraint.
        if let Some((forced, override_msg)) = &forced_override {
            if tool != *forced {
                let error_msg = format!("[SISTEMA INTERCEPTO] Error Lógico: El sistema te ordenó explícitamente usar la herramienta '{}' por la siguiente razón: '{}'. Sin embargo, tú elegiste '{}'. Corrige tu elección inmediatamente y usa '{}'.", forced, override_msg, tool, forced);
                current_context.push_str(&format!("{}\n\n", error_msg));
                emit_event(&app_handle, step_count, &format!("[INTERCEPT] LLM desobedeció orden de usar {}", forced), "WARNING");
                
                // Restauramos el forced_next_tool para el próximo ciclo
                forced_next_tool = forced_override.clone();
                step_count += 1;
                continue;
            }
        }
        
        let url = raw_value.get("url_a_investigar").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let respuesta_conv = raw_value.get("respuesta_conversacional").and_then(|v| v.as_str()).unwrap_or("").to_string();
        
        let mut archivos_vec = Vec::new();
        if let Some(arr) = raw_value.get("archivos_a_editar").and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(s) = item.as_str() {
                    archivos_vec.push(s.to_string());
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
        match tool.as_str() {
            "TOOL_TERMINAL" => {
                if comando.trim().is_empty() {
                    let res_msg = "Error: El comando no puede estar vacío. Rellena el campo 'comando'.";
                    current_context.push_str(&format!("{}\n\n", res_msg));
                    emit_event(&app_handle, step_count, "Comando vacío", "ERROR");
                } else if comandos_ejecutados_historico.contains(&comando) {
                    let res_msg = "[SISTEMA INTERNO]: Advertencia: Estás repitiendo un comando fallido. Repetirlo no lo arreglará. Usa TOOL_PROGRAMMER o TOOL_AUDITOR.";
                    emit_event(&app_handle, step_count, res_msg, "WARNING");
                    current_context.push_str(&format!("{}\n\n", res_msg));
                    forced_next_tool = Some(("TOOL_PROGRAMMER".to_string(), "El comando de terminal se repitió y falló anteriormente. El sistema te ha forzado a usar TOOL_PROGRAMMER para arreglar el script primero.".to_string()));
                } else {
                    programmer_cooldown_hits = 0;
                    comandos_ejecutados_historico.insert(comando.clone());
                    emit_event(&app_handle, step_count, &format!("Ejecutando en terminal: {}", comando), "ACTION");
                    match execute_terminal_command(&workspace_path, &comando).await {
                        Ok(out) => {
                            comandos_ejecutados_historico.remove(&comando);
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
                                cl.starts_with("python") || cl.starts_with("node")
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
                                        comandos_ejecutados_historico.remove(&comando);
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
                            }
                        }
                    }
                }
            },
            "TOOL_ENV_MANAGER" => {
                if comando.trim().is_empty() {
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
                    if comandos_ejecutados_historico.contains(&"__EMPTY_BG_CMD__".to_string()) {
                        let res_msg = "[SISTEMA INTERNO]: Advertencia: Estás en un bucle infinito de comandos vacíos. Abortando.";
                        emit_event(&app_handle, step_count, res_msg, "FATAL");
                        let final_res = FinalResponse { status: "ERROR".to_string(), respuesta_conversacional: "Error interno del planificador asíncrono.".to_string() };
                        return Ok(serde_json::to_string(&final_res).unwrap());
                    }
                    comandos_ejecutados_historico.insert("__EMPTY_BG_CMD__".to_string());
                    let err_msg = "Error Crítico: El campo 'comando' está vacío. Debes especificar qué comando ejecutar en la terminal.";
                    current_context.push_str(&format!("{}\n\n", err_msg));
                    emit_event(&app_handle, step_count, err_msg, "ERROR");
                } else if comandos_ejecutados_historico.contains(&comando) {
                    let res_msg = "[SISTEMA INTERNO]: Advertencia: Estás repitiendo un comando de background fallido. Repetirlo no lo arreglará. Usa TOOL_PROGRAMMER.";
                    emit_event(&app_handle, step_count, res_msg, "FATAL");
                    let final_res = FinalResponse { status: "ERROR".to_string(), respuesta_conversacional: format!("Bucle en background con comando: {}", comando) };
                    return Ok(serde_json::to_string(&final_res).unwrap());
                } else {
                    comandos_ejecutados_historico.insert(comando.clone());
                    emit_event(&app_handle, step_count, &format!("Iniciando tarea asíncrona '{}': {}", task_id, comando), "ACTION");
                    match start_background_task(&workspace_path, &task_id, &comando).await {
                        Ok(out) => {
                            comandos_ejecutados_historico.remove(&comando);
                            current_context.push_str(&format!("Resultado: {}\n\n", out));
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
                emit_event(&app_handle, step_count, "Auditando archivos locales...", "ACTION");
                let safe_files = memory::read_files_safely(&workspace_path, archivos_vec.clone()).await;
                let reporte = delegate_to_auditor(&safe_files, ORCHESTRATOR_MODEL).await;
                current_context.push_str(&format!("Reporte Auditor:\n{}\n\n", reporte));
                emit_event(&app_handle, step_count, "Auditoría completada.", "SUCCESS");
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
                let reporte = delegate_to_logic_solver(&safe_files, ORCHESTRATOR_MODEL).await;
                current_context.push_str(&format!("Reporte de Verificación Formal (Logic Solver):\n{}\n\nRevisa los problemas matemáticos o lógicos detectados antes de programar o testear.\n\n", reporte));
                emit_event(&app_handle, step_count, "Verificación Lógica completada.", "SUCCESS");
            },
            "TOOL_WORKSPACE_MANAGER" => {
                emit_event(&app_handle, step_count, "Gestionando archivos del workspace...", "ACTION");
                if archivos_vec.is_empty() {
                    let err_msg = "Error: TOOL_WORKSPACE_MANAGER requiere una lista de archivos a eliminar.";
                    current_context.push_str(&format!("{}\n\n", err_msg));
                    emit_event(&app_handle, step_count, err_msg, "ERROR");
                } else {
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
                emit_event(&app_handle, step_count, "Pensando y reflexionando...", "ACTION");
                current_context.push_str(&format!("Reflexión Interna del Agente: {}\n\n", comando));
                emit_event(&app_handle, step_count, "Reflexión completada. Puedes continuar ejecutando otra herramienta.", "SUCCESS");
            },
            "TOOL_MAPPER" => {
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
                current_context.push_str(&format!(
                    "[TOOL_MAPPER] Análisis completado. Grafo persistido en .aura_graph.json\n\n{}\n\n\
                    [INSTRUCCIÓN CRÍTICA]: El grafo de arriba es la REALIDAD FÍSICA del proyecto. \
                    Sigue el 'Orden de Escritura Recomendado' AL PIE DE LA LETRA. \
                    Usa TOOL_PROGRAMMER para escribir cada archivo en ese orden exacto. \
                    NO empieces por archivos que dependen de otros que aún no existen.\n\n",
                    report
                ));
                emit_event(&app_handle, step_count, &summary, "SUCCESS");
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
                
                if is_cooldown_blocked {
                    programmer_cooldown_hits += 1;
                    if programmer_cooldown_hits >= 3 {
                        let res_msg = "[SISTEMA INTERCEPTO] Error Crítico: Bucle infinito de TOOL_PROGRAMMER detectado. Abortando misión.";
                        emit_event(&app_handle, step_count, res_msg, "FATAL");
                        let final_res = FinalResponse {
                            status: "ERROR".to_string(),
                            respuesta_conversacional: format!("Me he quedado atascado editando repetidamente el mismo archivo ({:?}) sin probarlo en la terminal. He detenido la ejecución por seguridad.", archivos_vec),
                        };
                        return Ok(serde_json::to_string(&final_res).unwrap());
                    } else {
                        let interception = "[SISTEMA INTERCEPTO] Error Lógico: Estás intentando editar los mismos archivos por segunda vez consecutiva sin haber probado tu código en la terminal. DEBES ejecutar 'TOOL_TERMINAL' para probar el script y ver los errores antes de seguir programando.";
                        current_context.push_str(&format!("{}\n\n", interception));
                        emit_event(&app_handle, step_count, "Bucle interceptado por Cooldown", "WARNING");
                        forced_next_tool = Some(("TOOL_TERMINAL".to_string(), "Se forzó 'TOOL_TERMINAL'. DEBES ejecutar el script en la terminal para probarlo ahora mismo antes de seguir programando.".to_string()));
                    }
                } else {
                    // Valid programming action. 
                    // Clear the terminal history so the LLM must test again after this programming phase.
                    comandos_ejecutados_historico.clear();
                let safe_files = memory::read_files_safely(&workspace_path, archivos_vec.clone()).await;
                let context_for_qwen = format!("Historial Bucle:\n{}\nArchivos:\n{}", current_context, safe_files);
                
                let mut qwen_prompt = format!("Instrucción principal: {}\nDEBES crear/modificar los archivos solicitados con implementaciones COMPLETAS y REALES. PROHIBIDO usar 'pass', 'TODO', funciones vacías, NotImplementedError o cualquier placeholder. Cada función debe tener lógica funcional real.", user_message);
                let target_model = match crate::llm::router::get_best_model(&task_complexity, &available_models, &app_handle, step_count).await {
                    Ok(m) => m,
                    Err(e) => {
                        let msg = format!("[ENV_FAILURE] {}", e);
                        emit_event(&app_handle, step_count, &msg, "FATAL");
                        current_context.push_str(&format!("{}\n\n", msg));
                        break;
                    }
                };
                emit_event(&app_handle, step_count, &format!("[ROUTER] Cerebro Programador Seleccionado: {}", target_model), "INFO");

                let mut exito_bucle_programador = false;
                let mut max_intentos = 3;
                
                while max_intentos > 0 && !exito_bucle_programador {
                    match delegate_to_programmer(&qwen_prompt, &context_for_qwen, &target_model).await {
                        Ok(json_res) => {
                            let clean_json_res = strip_think_tags(json_res.clone());
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
                                                    "{}\n\nREESCRIBE COMPLETAMENTE. IMPLEMENTACIÓN REAL OBLIGATORIA.",
                                                    combined
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
                                                        qwen_prompt = format!("El código causó este error:\n{}\nSoluciónalo y genera un nuevo JSON.", e);
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
                                    emit_event(&app_handle, step_count, "El programador no propuso cambios.", "WARNING");
                                    current_context.push_str("Programador: No se propusieron cambios.\n\n");
                                    break;
                                }
                            } else {
                                emit_event(&app_handle, step_count, "El programador devolvió JSON inválido.", "ERROR");
                                current_context.push_str("Programador: JSON inválido.\n\n");
                                break;
                            }
                        },
                        Err(e) => {
                            emit_event(&app_handle, step_count, &format!("Error llamando a Qwen: {}", e), "ERROR");
                            current_context.push_str(&format!("Programador: Falla de red: {}\n\n", e));
                            break;
                        }
                    }
                }
                
                if !exito_bucle_programador && max_intentos == 0 {
                    emit_event(&app_handle, step_count, "Max intentos de auto-sanación alcanzado. Fallo físico.", "FATAL");
                    current_context.push_str("Programador: Fracasó tras múltiples intentos.\n\n");
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
                    match crate::core::architect::generate_dependency_map(&workspace_path) {
                        Ok(report) => {
                            current_context.push_str(&format!("Reporte Arquitectónico:\n{}\n\n", report));
                            emit_event(&app_handle, step_count, "Mapa arquitectónico generado.", "SUCCESS");
                        },
                        Err(e) => {
                            current_context.push_str(&format!("Error en Arquitecto: {}\n\n", e));
                            emit_event(&app_handle, step_count, &e, "ERROR");
                        }
                    }
                }
            },
            "TOOL_TESTER" => {
                emit_event(&app_handle, step_count, "Ejecutando suite de pruebas automatizadas...", "ACTION");
                match crate::core::tester::run_tests(&workspace_path).await {
                    crate::core::tester::TestResult::NoTests => {
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
                                En tu próximo turno DEBES ELEGIR 'TOOL_TERMINAL' para ejecutar el script directamente y ver si funciona, o 'TOOL_FINISH' si ya terminaste el objetivo.\n\
                                Archivos actuales en el workspace:\n{}\n",
                                no_tests_consecutive, workspace_listing
                            );
                            current_context.push_str(&format!("{}\n\n", force_msg));
                            emit_event(&app_handle, step_count, &format!("[SISTEMA] Cambiando estrategia a TOOL_TERMINAL tras {} intentos.", no_tests_consecutive), "WARNING");
                            forced_next_tool = Some(("TOOL_TERMINAL".to_string(), "El proyecto no tiene archivos de test. Se forzó TOOL_TERMINAL para que ejecutes el script manualmente en su lugar.".to_string()));

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
                        }
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
                                task_complexity = crate::llm::router::TaskContext { task_type: crate::llm::router::TaskType::HighComplexityFix, language: None };
                                emit_event(&app_handle, step_count, "[ROUTER] Tarea compleja detectada tras fallo de tests. Escalando modelo experto...", "ACTION");
                                current_context.push_str(&format!("[AUTO-DEBUGGER] Los tests fallaron estrepitosamente:\n{}\n\nEl sistema ha restaurado el código usando Git-Shield. Debes generar una nueva y mejor solución usando TOOL_PROGRAMMER.\n", fail_msg));
                                forced_next_tool = Some(("TOOL_PROGRAMMER".to_string(), "Los tests fallaron estrepitosamente, el sistema forzó TOOL_PROGRAMMER para que generes una nueva y mejor solución.".to_string()));
                            }
                        }
                    }
                }
            },
            "TOOL_LEARN" => {
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
            },
            "TOOL_SEARCH" => {
                emit_event(&app_handle, step_count, &format!("Consultando Memoria Permanente para: {}", url), "ACTION");
                match crate::core::memory::query_memory(&url).await {
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
            "TOOL_FINISH" => {
                emit_event(&app_handle, step_count, "Bucle completado exitosamente.", "FINISH");
                // ── Journal: mark completed ──
                crate::core::session_journal::close_journal(&mut journal, "COMPLETADO", &workspace_path);
                let final_res = FinalResponse {
                    status: "FINISH".to_string(),
                    respuesta_conversacional: respuesta_conv,
                };
                return Ok(serde_json::to_string(&final_res).unwrap());
            },
            _ => {
                emit_event(&app_handle, step_count, &format!("Herramienta desconocida: {}", tool), "WARNING");
                current_context.push_str(&format!("Advertencia: Intentaste usar herramienta desconocida '{}'.\n\n", tool));
            }
        }
        
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
