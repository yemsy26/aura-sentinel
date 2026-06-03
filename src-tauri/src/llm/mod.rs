use serde::{Deserialize, Serialize};
use crate::{memory::{self, Cambio}, core::validate_workspace};

#[derive(Serialize)]
struct OllamaRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
    format: &'a str,
}

#[derive(Deserialize)]
struct OllamaResponse {
    response: String,
}

#[derive(Deserialize, Serialize)]
struct OrchestratorOutput {
    intencion: String,
    respuesta_conversacional: String,
    pensamiento: String,
    archivos_a_analizar: Vec<String>,
    modelo_sugerido: String,
}

#[derive(Deserialize, Serialize)]
struct ProgrammerOutput {
    explicacion_tecnica: String,
    cambios: Vec<Cambio>,
}

#[derive(Serialize)]
struct PipelineResponse {
    orquestador: OrchestratorOutput,
    programador: serde_json::Value,
    operacion_fisica: String,
    eventos_validacion: Vec<String>,
}

async fn call_ollama(model: &str, prompt: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let url = "http://localhost:11434/api/generate";
    
    let payload = OllamaRequest {
        model,
        prompt,
        stream: false,
        format: "json",
    };

    let res = client.post(url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Error conectando a Ollama: {}", e))?;

    if res.status().is_success() {
        let ollama_res: OllamaResponse = res.json().await
            .map_err(|e| format!("Error parseando la respuesta JSON: {}", e))?;
        Ok(ollama_res.response)
    } else {
        Err(format!("Error de Ollama. Status: {}", res.status()))
    }
}

async fn delegate_to_programmer(task: &str, file_contents: &str, model: &str) -> Result<String, String> {
    let system_prompt = format!(
        "Eres Aura-Sentinel, el Ingeniero Ejecutor. Tu tarea es modificar el código basado en la instrucción del usuario y el código proporcionado.\n\
        Instrucción: {}\n\n\
        Códigos proporcionados:\n{}\n\n\
        DEBES responder ÚNICAMENTE con un JSON válido con la siguiente estructura:\n\
        {{\n\
          \"explicacion_tecnica\": \"Breve resumen de lo modificado\",\n\
          \"cambios\": [\n\
            {{\n\
              \"archivo\": \"ruta/al/archivo\",\n\
              \"buscar\": \"código exacto a reemplazar\",\n\
              \"reemplazar\": \"nuevo código\"\n\
            }}\n\
          ]\n\
        }}",
        task, file_contents
    );

    call_ollama(model, &system_prompt).await
}

#[tauri::command]
pub async fn process_user_prompt(user_message: String, workspace_path: String) -> Result<String, String> {
    let orchestrator_model = "llama3.1:8b";
    let mut eventos_validacion: Vec<String> = Vec::new();
    
    // Paso 1: Orquestador
    let workspace_tree_nodes = memory::get_workspace_tree_internal(workspace_path.clone()).await?;
    let tree_json = serde_json::to_string(&workspace_tree_nodes)
        .unwrap_or_else(|_| "[]".to_string());

    let system_prompt = format!(
        "Rol: Eres Aura-Sentinel, el Orquestador Arquitectónico.\n\
        Contexto: Este es el mapa actual del proyecto: {}\n\
        Tarea del Usuario: {}\n\
        Instrucción Estricta: Clasifica la intención del usuario en 'CHAT' (para saludos, charla general o preguntas) o 'ACCION' (si pide modificar o analizar código explícitamente). Tu respuesta DEBE ser ÚNICAMENTE un objeto JSON válido con la siguiente estructura, sin texto adicional ni markdown:\n\
        {{\n\
          \"intencion\": \"CHAT o ACCION\",\n\
          \"respuesta_conversacional\": \"Tu respuesta natural al usuario\",\n\
          \"pensamiento\": \"Breve justificación de tu decisión\",\n\
          \"archivos_a_analizar\": [\"ruta/al/archivo1.rs\"], // Vacío si la intencion es CHAT\n\
          \"modelo_sugerido\": \"qwen2.5-coder:7b\"\n\
        }}", 
        tree_json, user_message
    );

    let orchestrator_res = call_ollama(orchestrator_model, &system_prompt).await?;
    
    let orch_output: OrchestratorOutput = match serde_json::from_str(&orchestrator_res) {
        Ok(data) => data,
        Err(_) => return Ok(orchestrator_res),
    };

    if orch_output.intencion == "CHAT" || orch_output.archivos_a_analizar.is_empty() {
        let pipeline_res = PipelineResponse {
            orquestador: orch_output,
            programador: serde_json::Value::Null,
            operacion_fisica: "Modo CHAT: Sin operaciones de disco.".to_string(),
            eventos_validacion: vec![],
        };
        return serde_json::to_string(&pipeline_res).map_err(|e| format!("Error serializando pipeline: {}", e));
    }

    // Paso 2: Lectura Segura de Archivos
    let mut file_contents = String::new();
    if !orch_output.archivos_a_analizar.is_empty() {
        file_contents = memory::read_files_safely(&workspace_path, orch_output.archivos_a_analizar.clone()).await;
    }
    
    let programmer_model = if orch_output.modelo_sugerido.trim().is_empty() {
        "qwen2.5-coder:7b"
    } else {
        &orch_output.modelo_sugerido
    };

    // Paso 3: Delegación al Ejecutor
    let final_json_str = delegate_to_programmer(&user_message, &file_contents, programmer_model).await?;

    // Fase 7/8: Aplicación Física y Auto-Reparación
    let mut operacion_fisica_log = "No se detectaron cambios aplicables.".to_string();
    let mut prog_output_final: serde_json::Value = serde_json::json!({});

    // Intentamos parsear el JSON del programador
    if let Ok(mut prog_output) = serde_json::from_str::<ProgrammerOutput>(&final_json_str) {
        
        // Loop de auto-healing (máximo 2 intentos por seguridad)
        let mut max_intentos = 2;
        let mut loop_activo = true;

        while loop_activo && max_intentos > 0 {
            if !prog_output.cambios.is_empty() {
                match memory::apply_code_changes(&workspace_path, prog_output.cambios.clone()).await {
                    Ok(msg) => {
                        operacion_fisica_log = msg;
                        
                        // FASE 8: BUCLE DE CURACIÓN (Validación)
                        eventos_validacion.push("[VALIDACIÓN] Ejecutando cargo check en el workspace...".to_string());
                        
                        match validate_workspace(&workspace_path).await {
                            Ok(_) => {
                                // Compilación exitosa
                                let _ = memory::update_last_memory_status(&workspace_path, "COMPILACIÓN_EXITOSA").await;
                                eventos_validacion.push("[ÉXITO] El código compila correctamente. Memoria actualizada.".to_string());
                                loop_activo = false; // Salimos del bucle con éxito
                            },
                            Err(error_log) => {
                                // Fallo de compilación
                                eventos_validacion.push(format!("[ERROR DETECTADO] Fallo de compilación. Iniciando protocolo de Auto-Reparación con el modelo ejecutor..."));
                                
                                max_intentos -= 1;
                                if max_intentos > 0 {
                                    // Generar nuevo prompt estricto de reparación
                                    let repair_prompt = format!(
                                        "El código que modificaste causó el siguiente error de compilación: \n{}\nAnaliza el error y genera un nuevo JSON de cambios (con buscar y reemplazar) para solucionarlo de inmediato. DEBES responder ÚNICAMENTE con un JSON válido con la estructura requerida.",
                                        error_log
                                    );
                                    
                                    // Re-leemos los archivos para darle el contexto actualizado (porque fueron modificados en disco).
                                    let content_releido = memory::read_files_safely(&workspace_path, orch_output.archivos_a_analizar.clone()).await;
                                    
                                    match delegate_to_programmer(&repair_prompt, &content_releido, programmer_model).await {
                                        Ok(new_json) => {
                                            if let Ok(new_prog) = serde_json::from_str::<ProgrammerOutput>(&new_json) {
                                                prog_output = new_prog; // Actualizamos para el siguiente bucle
                                            } else {
                                                loop_activo = false;
                                                eventos_validacion.push("[FATAL] El modelo no devolvió JSON válido en la auto-reparación.".to_string());
                                            }
                                        },
                                        Err(e) => {
                                            loop_activo = false;
                                            eventos_validacion.push(format!("[FATAL] Error conectando con el modelo en reparación: {}", e));
                                        }
                                    }
                                } else {
                                    let _ = memory::update_last_memory_status(&workspace_path, "COMPILACIÓN_FALLIDA").await;
                                    eventos_validacion.push("[FATAL] Máximo de intentos de auto-reparación alcanzado.".to_string());
                                }
                            }
                        }
                    },
                    Err(e) => {
                        operacion_fisica_log = format!("Error en aplicación física: {}", e);
                        loop_activo = false;
                    }
                }
            } else {
                loop_activo = false;
            }
        }
        
        prog_output_final = serde_json::to_value(prog_output).unwrap_or_else(|_| serde_json::json!({}));
    } else {
        prog_output_final = serde_json::from_str(&final_json_str).unwrap_or_else(|_| serde_json::json!({ "raw": final_json_str }));
    }

    let pipeline_res = PipelineResponse {
        orquestador: orch_output,
        programador: prog_output_final,
        operacion_fisica: operacion_fisica_log,
        eventos_validacion,
    };

    serde_json::to_string(&pipeline_res).map_err(|e| format!("Error serializando pipeline: {}", e))
}
