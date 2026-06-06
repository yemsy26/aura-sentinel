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
            
    let mut current_context = String::new();
    let mut archivos_editados_historico: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut architect_used = false;
    let mut step_count = 1;
    let max_steps = 10;
    
    while step_count <= max_steps {
        let agent_prompt = format!(
            "Eres el Cerebro Planificador de Aura-Sentinel. Funcionarás en un bucle autónomo. Analiza el Objetivo, el Contexto y el Historial para decidir UNA ÚNICA HERRAMIENTA a utilizar en este turno.\n\
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
            8. 'TOOL_FINISH': Cuando el objetivo principal se haya completado con éxito, o si es imposible continuar. Rellena 'respuesta_conversacional' con la respuesta final para el usuario. ¡USALA SIEMPRE QUE HAYAS TERMINADO!\n            9. 'TOOL_ARCHITECT': Analiza la estructura y dependencias. No rellena argumentos. REGLA: Después de usarla, DEBES usar TOOL_FINISH obligatoriamente para resumirle los hallazgos al usuario.\n\
            Antes de tomar tu decisión, DEBES rellenar el campo 'checklist_mental'. En este campo, enumera mentalmente todos los pasos que pidió el usuario, qué pasos ya se han cumplido en el historial, y cuál es el paso exacto que falta ahora mismo. \n\
            REGLA DE ORO DE FINALIZACIÓN: NUNCA puedes elegir la herramienta 'TOOL_FINISH' a menos que tu 'checklist_mental' confirme explícitamente que el 100% de los verbos y acciones solicitadas por el usuario se han ejecutado con éxito.\n\n\
            MANUAL DE OPERACIONES ANTIGRAVITY (DOMAIN KNOWLEDGE):\n\
            - Scaffolding Frontend: Si el usuario pide crear una web desde cero, usa 'TOOL_TERMINAL' con 'npx -y create-vite@latest frontend --template vanilla' (o similar) en lugar de intentar escribir archivos manualmente.\n\
            - Backend Rápidos: Si piden un servidor, crea el código físico con 'TOOL_PROGRAMMER' y luego levántalo con 'TOOL_BACKGROUND_START'.\n\
            - Firebase Deploy: Si piden desplegar a producción/Firebase, asume que 'firebase-tools' está instalado y usa 'TOOL_TERMINAL' con 'firebase init hosting' o 'firebase deploy --only hosting'. Asegúrate de compilar antes si es necesario (ej. 'npm run build').\n\
            - Resolución de Errores: Si un comando falla, lee los logs o la consola, usa 'TOOL_PROGRAMMER' para arreglar el código, y vuelve a intentar.\n            - Estrictud JSON (Auto-Debugger): Si recibes una alerta [AUTO-DEBUGGER], tu ÚNICA tarea es corregir la sintaxis o el ID que falló. No intentes ejecutar código nuevo hasta que la herramienta devuelva [SUCCESS].\n            - Modo Arquitecto: Si al usar TOOL_ARCHITECT el campo de confianza es BAJA, no tomes decisiones de refactorización automáticas. Reporta los hallazgos al usuario y solicita confirmación manual.\n\n\
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
        
        let mut agent_res = match call_ollama(ORCHESTRATOR_MODEL, &agent_prompt).await {
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
        emit_event(&app_handle, step_count, &format!("Decisión: {} - {}", tool, pensamiento), "DECISION");
        current_context.push_str(&format!("--- PASO {} ---\nChecklist Mental: {}\nDecidiste: {}\nPensamiento: {}\n", step_count, checklist, tool, pensamiento));
        
        match tool.as_str() {
            "TOOL_TERMINAL" => {
                emit_event(&app_handle, step_count, &format!("Ejecutando en terminal: {}", comando), "ACTION");
                match execute_terminal_command(&workspace_path, &comando).await {
                    Ok(out) => {
                        let res_msg = format!("Éxito: {}", out);
                        current_context.push_str(&format!("Resultado: {}\n\n", res_msg));
                        emit_event(&app_handle, step_count, &res_msg, "SUCCESS");
                    },
                    Err(err) => {
                        let res_msg = format!("Error: {}", err);
                        current_context.push_str(&format!("Resultado: {}\n\n", res_msg));
                        emit_event(&app_handle, step_count, &res_msg, "ERROR");
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
                    let interception = "[SISTEMA INTERCEPTO] Error Lógico: Ya editaste estos archivos en un turno anterior con éxito. ASUME QUE EL CÓDIGO FUE ESCRITO CORRECTAMENTE. No repitas esta acción. Actualiza tu checklist mental y avanza al siguiente paso o usa TOOL_FINISH.";
                    current_context.push_str(&format!("{}\n\n", interception));
                    emit_event(&app_handle, step_count, "Bucle interceptado por Cooldown", "WARNING");
                } else {
                emit_event(&app_handle, step_count, "Delegando a Qwen para modificar código físico...", "ACTION");
                let safe_files = memory::read_files_safely(&workspace_path, archivos_vec.clone()).await;
                let context_for_qwen = format!("Historial Bucle:\n{}\nArchivos:\n{}", current_context, safe_files);
                
                let mut qwen_prompt = format!("Instrucción principal: {}\nDebes crear/modificar los archivos solicitados usando el JSON estructurado.", user_message);
                
                let mut exito_bucle_programador = false;
                let mut max_intentos = 3;
                
                while max_intentos > 0 && !exito_bucle_programador {
                    match delegate_to_programmer(&qwen_prompt, &context_for_qwen, PROGRAMMER_MODEL).await {
                        Ok(json_res) => {
                            if let Ok(prog_output) = serde_json::from_str::<ProgrammerOutput>(&json_res) {
                                if !prog_output.cambios.is_empty() {
                                    match memory::apply_code_changes(&workspace_path, prog_output.cambios.clone()).await {
                                        Ok(msg) => {
                                            emit_event(&app_handle, step_count, &msg, "SUCCESS");
                                            emit_event(&app_handle, step_count, "Validando compilación...", "VALIDATING");
                                            
                                            match validate_workspace(&workspace_path).await {
                                                Ok(_) => {
                                                    emit_event(&app_handle, step_count, "Validación exitosa.", "SUCCESS");
                                                    let _ = memory::update_last_memory_status(&workspace_path, "COMPILACIÓN_EXITOSA").await;
                                                    current_context.push_str("Programador: Código modificado y validado exitosamente.\n\n");
                                                    exito_bucle_programador = true;
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
            "TOOL_FINISH" => {
                emit_event(&app_handle, step_count, "Bucle completado exitosamente.", "FINISH");
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
    let final_res = FinalResponse {
        status: "FINISH".to_string(),
        respuesta_conversacional: "He alcanzado el límite máximo de 10 pasos sin llegar a una conclusión.".to_string(),
    };
    Ok(serde_json::to_string(&final_res).unwrap())
}
