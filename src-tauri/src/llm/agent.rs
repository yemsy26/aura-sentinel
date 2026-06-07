use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use crate::memory;
use crate::core::{execute_terminal_command, start_background_task, read_task_logs, kill_task, validate_workspace, format_system_error};
use super::{get_embedding, call_ollama, delegate_to_programmer, delegate_to_auditor, ProgrammerOutput};

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
    let mut forced_next_tool: Option<&str> = None;
    let mut step_count = 1;
    let max_steps = 20;

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
    
    let mut task_complexity = crate::llm::router::Complexity::GeneralCode;
    
    while step_count <= max_steps {
        let agent_prompt = format!(
            "Eres el Cerebro Planificador de Aura-Sentinel. Eres un ingeniero políglota. Actualmente soportas [Python, JS/TS, Rust, Go, C++]. Antes de programar, detecta el lenguaje del proyecto y ajusta tus herramientas de validación al estándar del lenguaje detectado.\n\
            Funcionarás en un bucle autónomo. Analiza el Objetivo, el Contexto y el Historial para decidir UNA ÚNICA HERRAMIENTA a utilizar en este turno.\n\
            Objetivo Original: {}\n\
            Contexto del Proyecto (Archivos): {}\n\
            Historial de Pasos Ejecutados Hasta Ahora:\n{}\n\n\
            REGLA DE ORO: Si ya ejecutaste todas las acciones que pidió el usuario en el Objetivo Original, tu ÚNICA opción válida es usar 'TOOL_FINISH'. NO repitas pasos ni inventes problemas que no existen.\n\n\
            Catálogo de Herramientas (Elige SOLO UNA):\n\
            1. 'TOOL_TERMINAL': Para comandos síncronos y de un solo uso (npm install, pip, cargo build). Rellena 'comando' y 'task_id'.\n\
            2. 'TOOL_BACKGROUND_START': Para arrancar servidores que correrán infinitamente en segundo plano (python -m http.server, npm run dev). Rellena 'comando' y 'task_id'.\n\
            3. 'TOOL_BACKGROUND_READ': Para leer los logs en vivo de un servidor asíncrono. Rellena 'task_id'.\n\
            4. 'TOOL_BACKGROUND_KILL': Para apagar un servidor asíncrono. Rellena 'task_id'.\n\
            5. 'TOOL_PROGRAMMER': Para escribir o modificar código fuente físico en el disco. Rellena 'archivos_a_editar' con la lista de archivos.\n\
            6. 'TOOL_WEB_SCRAPER': Para extraer contenido de una URL. Rellena 'url_a_investigar'.\n\
            7. 'TOOL_AUDITOR': Para auditar código estático o leer archivos locales si no sabes cómo están hechos. Rellena 'archivos_a_editar'.\n\
            8. 'TOOL_FINISH': Cuando el objetivo principal se haya completado con éxito, o si es imposible continuar. Rellena 'respuesta_conversacional' con la respuesta final para el usuario. ¡USALA SIEMPRE QUE HAYAS TERMINADO!\n\
            9. 'TOOL_ARCHITECT': Analiza la estructura y dependencias. No rellena argumentos. REGLA: Después de usarla, DEBES usar TOOL_FINISH obligatoriamente para resumirle los hallazgos al usuario.\n\
            10. 'TOOL_TESTER': Ejecuta suites de pruebas automatizadas SOLO si el proyecto tiene archivos de test (test_*.py, *.test.js, etc.) o configuración de test (pytest.ini, jest.config.js). ¡PELIGRO! Esta herramienta NO ESCRIBE ni implementa pruebas. SOLO LAS EJECUTA. Para scripts simples (como hello.py) SIN archivos de test, usa TOOL_TERMINAL en su lugar. Para escribir o arreglar un test, usa TOOL_PROGRAMMER.\n\
            11. 'TOOL_LEARN': Indexa el conocimiento de un proyecto exitoso en la memoria permanente de Aura. Úsala si el proyecto funciona o después de un TOOL_TESTER exitoso. No requiere argumentos.\n\
            12. 'TOOL_SEARCH': Consulta explícitamente la memoria histórica para buscar cómo resolviste problemas similares antes. Rellena 'url_a_investigar' con el término de búsqueda.\n\
            13. 'TOOL_ENV_MANAGER': Instala dependencias o lenguajes faltantes en el sistema operativo de forma automática y recarga el PATH en caliente. Rellena 'comando' SOLO con el NOMBRE del paquete (ej. 'go', 'node', 'python'). ¡PROHIBIDO pasar comandos enteros como 'scoop install python' o 'apt-get install'! El sistema lo hará por ti.\n\
            Antes de tomar tu decisión, DEBES rellenar el campo 'checklist_mental'. En este campo, enumera mentalmente todos los pasos que pidió el usuario, qué pasos ya se han cumplido en el historial, y cuál es el paso exacto que falta ahora mismo. \n\
            REGLA DE ORO DE FINALIZACIÓN: NUNCA puedes elegir la herramienta 'TOOL_FINISH' a menos que tu 'checklist_mental' confirme explícitamente que el 100% de los verbos y acciones solicitadas por el usuario se han ejecutado con éxito.\n\n\
            MANUAL DE OPERACIONES ANTIGRAVITY (DOMAIN KNOWLEDGE):\n\
            - Scaffolding Frontend: Si el usuario pide crear una web desde cero, usa 'TOOL_TERMINAL' con 'npx -y create-vite@latest frontend --template vanilla' (o similar) en lugar de intentar escribir archivos manualmente.\n\
            - Backend Rápidos: Si piden un servidor, crea el código físico con 'TOOL_PROGRAMMER' y luego levántalo con 'TOOL_BACKGROUND_START'.\n\
            - Firebase Deploy: Si piden desplegar a producción/Firebase, asume que 'firebase-tools' está instalado y usa 'TOOL_TERMINAL' con 'firebase init hosting' o 'firebase deploy --only hosting'. Asegúrate de compilar antes si es necesario (ej. 'npm run build').\n\
            - Resolución de Errores: Si un comando falla, lee los logs o la consola, usa 'TOOL_PROGRAMMER' para arreglar el código, y vuelve a intentar.\n\
            - Estrictud JSON (Auto-Debugger): Si recibes una alerta [AUTO-DEBUGGER] tras un fallo de TOOL_TESTER, tu ÚNICA tarea es usar TOOL_PROGRAMMER para re-escribir y arreglar el código defectuoso. ¡PROHIBIDO volver a usar TOOL_TESTER sin antes haber modificado el código!\n\
            - Modo Arquitecto: Si al usar TOOL_ARCHITECT el campo de confianza es BAJA, no tomes decisiones de refactorización automáticas. Reporta los hallazgos al usuario y solicita confirmación manual.\n\
            - Auto-Healing (Archivos Perdidos): Si la terminal lanza un error de tipo 'No such file or directory' o 'can\\'t open file', significa que el script o archivo que intentas ejecutar o leer no existe. Debes usar obligatoriamente 'TOOL_PROGRAMMER' para crear el archivo en lugar de usar otras herramientas como TOOL_ENV_MANAGER.\n\
            - Auto-Testing: Tu objetivo no es solo escribir código, sino entregar sistemas funcionales. Si el proyecto tiene archivos de test (test_*.py, *.spec.js, etc.), valida con TOOL_TESTER. Si el proyecto es un script simple SIN archivos de test (como hello.py), valida con TOOL_TERMINAL ejecutando el script directamente.\n\
            - Auto-Healing (Pre-Flight): Si la terminal reporta explícitamente que un comando 'no se reconoce' (ej. 'is not recognized as an internal or external command' o 'command not found'), usa TOOL_ENV_MANAGER para instalarlo. ¡NUNCA uses TOOL_ENV_MANAGER si el error es de sintaxis (SyntaxError), falta de archivos, o fallos de código! Para errores de código usa TOOL_PROGRAMMER.\n\n\
            REGLAS DE ESTADO (STATE MACHINE):\n\
            - DESPUÉS de usar TOOL_PROGRAMMER con éxito, es OBLIGATORIO usar TOOL_TERMINAL o TOOL_TESTER para ejecutar y validar tus cambios. ADEMÁS, DEBES IGNORAR completamente cualquier error previo en el historial, ya que el código acaba de ser reparado.\n\
            - SI TOOL_TESTER FALLA, es OBLIGATORIO usar TOOL_PROGRAMMER en el siguiente paso para arreglar el código. ESTÁ PROHIBIDO usar TOOL_TESTER dos veces seguidas si los tests fallan.\n\
            - SI TOOL_TESTER TIENE ÉXITO, estás OBLIGADO a usar TOOL_FINISH en el siguiente paso. ESTÁ PROHIBIDO usar TOOL_TESTER dos veces seguidas si los tests pasaron.\n\
            - SI TOOL_TERMINAL TIENE ÉXITO y su salida demuestra que cumpliste el último objetivo del usuario, es OBLIGATORIO usar TOOL_FINISH en el siguiente paso. ¡PROHIBIDO repetir el mismo comando de TOOL_TERMINAL si ya funcionó!\n\
            - SI TOOL_TERMINAL falla constantemente, es OBLIGATORIO usar TOOL_FINISH para evitar bucles de comandos infinitos.\n\
            - SI TOOL_ENV_MANAGER TIENE ÉXITO, NO PUEDES volver a usar TOOL_ENV_MANAGER. Debes continuar tu tarea con TOOL_TERMINAL, TOOL_PROGRAMMER o TOOL_TESTER.\n\
            - SI TOOL_ENV_MANAGER FALLA, es OBLIGATORIO usar TOOL_FINISH en el siguiente paso para pedir intervención manual.\n\n\
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
            user_message, tree_json, current_context
        );
        
        emit_event(&app_handle, step_count, "Analizando estado y planificando siguiente paso...", "PLANNING");
        
        let orchestrator_model = crate::llm::router::get_best_model(&crate::llm::router::Complexity::Orchestrator, &available_models)
            .unwrap_or_else(|_| ORCHESTRATOR_MODEL.to_string());

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
        
        let raw_value: serde_json::Value = match serde_json::from_str(&agent_res) {
            Ok(v) => {
                println!("LLM RAW RESPONSE: {}", agent_res);
                v
            },
            Err(e) => {
                println!("LLM RAW RESPONSE ERROR: {}", agent_res);
                emit_event(&app_handle, step_count, &format!("Error parseando decisión ({}). Abortando bucle.", e), "ERROR");
                let final_res = FinalResponse { status: "ERROR".to_string(), respuesta_conversacional: "Error interno en el planificador.".to_string() };
                return Ok(serde_json::to_string(&final_res).unwrap());
            }
        };
        
        let checklist = raw_value.get("checklist_mental").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let tool = raw_value.get("herramienta").and_then(|v| v.as_str()).unwrap_or("UNKNOWN").to_uppercase();
        let pensamiento = raw_value.get("pensamiento").and_then(|v| v.as_str()).unwrap_or("Sin pensamiento").to_string();
        let comando = raw_value.get("comando").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let task_id = raw_value.get("task_id").and_then(|v| v.as_str()).unwrap_or("default_task").to_string();
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

        // ── FORCED TOOL OVERRIDE ────────────────────────────────────────────
        // If the system has determined the LLM is stuck in a tool-loop,
        // override its decision and force a specific tool.
        let tool = if let Some(forced) = forced_next_tool.take() {
            let override_msg = format!(
                "[SISTEMA INTERCEPTO] El LLM eligió '{}' pero el sistema lo bloqueó. \
                Se forzó '{}' porque el proyecto no tiene archivos de test y el usuario los pidió. \
                DEBES crear los archivos de test ahora con TOOL_PROGRAMMER.",
                tool, forced
            );
            current_context.push_str(&format!("{}\n\n", override_msg));
            emit_event(&app_handle, step_count, &format!("[INTERCEPT] LLM forzado de {} a {}", tool, forced), "WARNING");
            forced.to_string()
        } else {
            tool
        };

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
                    let res_msg = "[SISTEMA INTERCEPTO] Error Crítico: Bucle infinito detectado en el terminal. Abortando misión.";
                    emit_event(&app_handle, step_count, res_msg, "FATAL");
                    let final_res = FinalResponse {
                        status: "ERROR".to_string(),
                        respuesta_conversacional: format!("Se detectó un bucle intentando ejecutar múltiples veces el comando '{}'. Por favor, revisa el entorno o instala las dependencias manualmente.", comando),
                    };
                    return Ok(serde_json::to_string(&final_res).unwrap());
                } else {
                    comandos_ejecutados_historico.insert(comando.clone());
                    emit_event(&app_handle, step_count, &format!("Ejecutando en terminal: {}", comando), "ACTION");
                    match execute_terminal_command(&workspace_path, &comando).await {
                        Ok(out) => {
                            let res_msg = format!("Éxito: {}", out);
                            current_context.push_str(&format!("Resultado: {}\n\n[SISTEMA: El comando tuvo éxito. Tu SIGUIENTE PASO OBLIGATORIO es usar TOOL_TESTER para validar si el entorno ya está correcto. NO VUELVAS A USAR TOOL_TERMINAL BAJO NINGUNA CIRCUNSTANCIA EN EL PRÓXIMO TURNO.]\n\n", res_msg));
                            emit_event(&app_handle, step_count, &res_msg, "SUCCESS");
                            // Guardar los cambios hechos por la terminal en Git-Shield
                            let _ = crate::core::create_git_backup(&workspace_path, "Aura-Sentinel: Git-Shield Auto-Backup (Terminal)").await;
                        },
                        Err(err) => {
                            // ── Auto ENV_MANAGER: detect binary-not-found and auto-install ──────────
                            let is_binary_missing = err.contains("is not recognized")
                                || err.contains("not recognized as an internal")
                                || err.contains("command not found")
                                || err.contains("No such file or directory")
                                || err.contains("cannot find the path")
                                || err.contains("The term") && err.contains("is not recognized");

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
                                let res_msg = format!("Error: {}", err);
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
                emit_event(&app_handle, step_count, &format!("Iniciando tarea asíncrona '{}': {}", task_id, comando), "ACTION");
                match start_background_task(&workspace_path, &task_id, &comando).await {
                    Ok(out) => {
                        current_context.push_str(&format!("Resultado: {}\n\n", out));
                        emit_event(&app_handle, step_count, &out, "SUCCESS");
                    },
                    Err(err) => {
                        current_context.push_str(&format!("Resultado: Error iniciando tarea: {}\n\n", err));
                        emit_event(&app_handle, step_count, &format!("Error: {}", err), "ERROR");
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
            "TOOL_PROGRAMMER" => {
                let mut ya_editados = true;
                if archivos_vec.is_empty() { ya_editados = false; }
                for f in &archivos_vec {
                    if !archivos_editados_historico.contains(f) {
                        ya_editados = false;
                        break;
                    }
                }
                
                if ya_editados {
                    programmer_cooldown_hits += 1;
                    if programmer_cooldown_hits >= 3 {
                        let res_msg = "[SISTEMA INTERCEPTO] Error Crítico: Bucle infinito de TOOL_PROGRAMMER detectado. Abortando misión.";
                        emit_event(&app_handle, step_count, res_msg, "FATAL");
                        let final_res = FinalResponse {
                            status: "ERROR".to_string(),
                            respuesta_conversacional: format!("Me he quedado atascado editando repetidamente el mismo archivo ({:?}) sin probarlo. He detenido la ejecución por seguridad.", archivos_vec),
                        };
                        return Ok(serde_json::to_string(&final_res).unwrap());
                    } else {
                        let interception = "[SISTEMA INTERCEPTO] Error Lógico: Ya editaste estos archivos en un turno anterior con éxito. ASUME QUE EL CÓDIGO FUE ESCRITO CORRECTAMENTE. No repitas esta acción. Actualiza tu checklist mental y avanza al siguiente paso (usa TOOL_TESTER) o usa TOOL_FINISH.";
                        current_context.push_str(&format!("{}\n\n", interception));
                        emit_event(&app_handle, step_count, "Bucle interceptado por Cooldown", "WARNING");
                        forced_next_tool = Some("TOOL_TESTER");
                    }
                } else {
                emit_event(&app_handle, step_count, "Delegando a Qwen para modificar código físico...", "ACTION");
                let safe_files = memory::read_files_safely(&workspace_path, archivos_vec.clone()).await;
                let context_for_qwen = format!("Historial Bucle:\n{}\nArchivos:\n{}", current_context, safe_files);
                
                let mut qwen_prompt = format!("Instrucción principal: {}\nDebes crear/modificar los archivos solicitados usando el JSON estructurado.", user_message);
                let target_model = match crate::llm::router::get_best_model(&task_complexity, &available_models) {
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
                            if let Ok(prog_output) = serde_json::from_str::<ProgrammerOutput>(&json_res) {
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
                                            emit_event(&app_handle, step_count, "Validando compilación...", "VALIDATING");
                                            
                                            match validate_workspace(&workspace_path).await {
                                                Ok(_) => {
                                                    emit_event(&app_handle, step_count, "Validación exitosa.", "SUCCESS");
                                                    let _ = memory::update_last_memory_status(&workspace_path, "COMPILACIÓN_EXITOSA").await;
                                                    let explicit_msg = format!("Programador: Los archivos {:?} fueron modificados y compilados con éxito. ¡LA TAREA DE ESCRITURA ESTÁ COMPLETA! Ahora DEBES usar TOOL_TESTER o avanzar a la siguiente tarea diferente.\n\n", archivos_vec);
                                                    current_context.push_str(&explicit_msg);
                                                    exito_bucle_programador = true;
                                                    // FIX: Limpiar el historial de comandos fallidos porque el entorno ha cambiado y comandos previos ahora pueden funcionar
                                                    comandos_ejecutados_historico.clear();
                                                    for f in &archivos_vec {
                                                        archivos_editados_historico.insert(f.clone());
                                                    }
                                                },
                                                Err(e) => {
                                                    emit_event(&app_handle, step_count, &format!("Error detectado: {}", e), "ERROR");
                                                    qwen_prompt = format!("El código causó este error:\n{}\nSoluciónalo y genera un nuevo JSON.", e);
                                                    max_intentos -= 1;
                                                }
                                            }
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

                        if no_tests_consecutive >= 3 {
                            let force_msg = format!(
                                "[SISTEMA INTERCEPTO] TOOL_TESTER fue llamado {} veces sin detectar tests. \
                                El sistema cambia de estrategia. En tu próximo turno DEBES ELEGIR 'TOOL_TERMINAL'. \
                                Archivos actuales en el workspace:\n{}\n\n\
                                EJECUTA: Identifica qué framework de testing necesita este proyecto (según su lenguaje) y usa TOOL_TERMINAL para inicializarlo e instalar sus dependencias. Si ya está instalado, ejecuta el comando de pruebas en la terminal.",
                                no_tests_consecutive, workspace_listing
                            );
                            current_context.push_str(&format!("{}\n\n", force_msg));
                            emit_event(&app_handle, step_count, &format!("[SISTEMA] Cambiando estrategia a TOOL_TERMINAL tras {} intentos.", no_tests_consecutive), "FATAL");

                        } else if no_tests_consecutive >= 2 {
                            let force_msg = format!(
                                "[SISTEMA INTERCEPTO] TOOL_TESTER fue llamado {} veces sin archivos de test detectados. \
                                En tu próximo turno DEBES ELEGIR 'TOOL_PROGRAMMER'. \
                                Archivos actuales en el workspace:\n{}\n\n\
                                DEBES crear TODOS los archivos que falten para la suite de pruebas del lenguaje en el que estés trabajando.\n\
                                1) Archivos de configuración (ej: package.json, go.mod, Cargo.toml, hardhat.config.js)\n\
                                2) El archivo de pruebas correspondiente\n\
                                3) El código fuente principal a probar\n\
                                Incluye TODOS estos archivos en 'archivos_a_editar' usando RUTAS RELATIVAS.",
                                no_tests_consecutive, workspace_listing
                            );
                            current_context.push_str(&format!("{}\n\n", force_msg));
                            emit_event(&app_handle, step_count, &format!("[SISTEMA] TOOL_TESTER bloqueado. Solicitando TOOL_PROGRAMMER con listado de archivos.", ), "WARNING");

                        } else {
                            // First hit — tell the LLM clearly
                            let no_test_msg = format!(
                                "[SISTEMA] No se detectaron archivos de test reconocidos para ningún lenguaje soportado. \
                                Archivos actuales en el workspace:\n{}\n\n\
                                Si el usuario pidió tests: DEBES usar TOOL_PROGRAMMER para crear los archivos de prueba y configuración del framework adecuado. \
                                Luego usa TOOL_TERMINAL para instalar dependencias, y después vuelve a usar TOOL_TESTER. \
                                SOLO usa TOOL_FINISH si el usuario NO pidió tests.",
                                workspace_listing
                            );
                            current_context.push_str(&format!("{}\n\n", no_test_msg));
                            emit_event(&app_handle, step_count, "Sin suite de tests detectada. Revisa si debes crear los archivos de test primero.", "WARNING");
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
                                    forced_next_tool = Some("TOOL_TERMINAL");
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
                                forced_next_tool = Some("TOOL_TERMINAL");
                            }
                            } else {
                                // Real test logic failure — revert and let Qwen fix the code
                                emit_event(&app_handle, step_count, "Tests fallaron. Revertiendo cambios y activando Auto-Debugger...", "ERROR");
                                let _ = crate::core::restore_git_backup(&workspace_path).await;
                                archivos_editados_historico.clear();
                                comandos_ejecutados_historico.clear();
                                task_complexity = crate::llm::router::Complexity::HighComplexityFix;
                                emit_event(&app_handle, step_count, "[ROUTER] Tarea compleja detectada tras fallo de tests. Escalando modelo experto...", "ACTION");
                                current_context.push_str(&format!("[AUTO-DEBUGGER] Los tests fallaron estrepitosamente:\n{}\n\nEl sistema ha restaurado el código usando Git-Shield. Debes generar una nueva y mejor solución usando TOOL_PROGRAMMER.\n", fail_msg));
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
