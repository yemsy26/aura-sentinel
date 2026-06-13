use super::call_ollama_text;
use tauri::AppHandle;
use crate::llm::agent::emit_event;

pub async fn translate_to_technical_intent(user_input: &str, app_handle: &AppHandle, chat_history: &[String]) -> String {
    // Usa la API HTTP de Ollama en lugar del CLI para ser independiente del PATH del sistema
    let mut available_models = Vec::new();
    if let Ok(res) = reqwest::Client::new()
        .get("http://localhost:11434/api/tags")
        .send()
        .await
    {
        if let Ok(json) = res.json::<serde_json::Value>().await {
            if let Some(models) = json.get("models").and_then(|m| m.as_array()) {
                for model in models {
                    if let Some(name) = model.get("name").and_then(|n| n.as_str()) {
                        available_models.push(name.to_string());
                    }
                }
            }
        }
    }

    let task_ctx = crate::llm::router::TaskContext {
        task_type: crate::llm::router::TaskType::FastTrack,
        language: None,
    };
    
    let model = crate::llm::router::get_best_model(&task_ctx, &available_models, app_handle, 0).await
        .unwrap_or_else(|_| "qwen2.5-coder:7b".to_string());

    let context_str = if chat_history.is_empty() {
        "".to_string()
    } else {
        format!("CONTEXTO RECIENTE DE LA CONVERSACIÓN:\n{}\n\n", chat_history.join("\n"))
    };

    let system_prompt = format!(
        "Eres el Analista de Intenciones (NLU) de Aura-Sentinel. \
        Tu única función es leer lo que dice el usuario y clasificar la petición en uno de estos tres tipos, DEVOLVIENDO UN ÚNICO OBJETO JSON VÁLIDO (sin texto extra):\n\n\
        TIPOS DE INTENCIÓN:\n\
        1. \"CONVERSATION\": El usuario solo está saludando, haciendo charla general, haciendo PREGUNTAS BÁSICAS DE CONOCIMIENTO (ej: \"qué es btc\"), o haciendo PETICIONES DE PROGRAMACIÓN MUY AMBIGUAS (ej: \"quiero crear un sistema en python\", \"haz una app\"). Si la petición es tan vaga que no sabes qué programar exactamente, DEBES responder preguntando por más detalles y NO usar AGENTIC_TASK.\n\
        2. \"FAST_TRACK_OS\": El usuario quiere ejecutar un comando nativo sencillo en la terminal (ej: crear carpeta, listar, ping) que no requiere un agente de programación complejo. \n\
        ⚠️ SISTEMA OPERATIVO: WINDOWS. Los comandos 'os_command' DEBEN ser sintaxis CMD de Windows. \n\
        COMANDOS WINDOWS OBLIGATORIOS: usa 'del' (NO 'rm'), 'type nul >' (NO 'touch'), 'dir' (NO 'ls'), 'copy' (NO 'cp'), 'move' (NO 'mv'), 'rd /s /q' (NO 'rm -rf'). \n\
        PROHIBIDO usar: rm, touch, chmod, ls, cat, cp, mv, mkdir -p, grep, sudo. \n\
        3. \"AGENTIC_TASK\": El usuario quiere buscar información EN TIEMPO REAL, crear código ESPECÍFICO, modificar archivos físicos, analizar sistemas, solucionar bugs, o VERIFICAR/DEMOSTRAR que algo funciona correctamente.\n\
        ¡OBLIGATORIO AGENTIC_TASK! Las siguientes frases SIEMPRE son AGENTIC_TASK, NUNCA FAST_TRACK_OS:\n\
        - 'prueba que [algo] funcione', 'verifica que [algo] funcione', 'demuestra que funciona', 'comprueba que funciona', 'prueba el sistema', 'verifica el sistema'\n\
        - Cualquier petición que requiera ANALIZAR resultados, no solo ejecutar un comando.\n\
        - Si el mensaje contiene 'investiga', 'investigue', 'busca', 'buscar', 'busque', 'investigar', 'prueba que', 'verifica que', 'demuestra que', 'comprueba que'.\n\
        [NOTA CLAVE]: Para comandos de terminal de 1 sola acción (mkdir, del, dir, ping) usa FAST_TRACK_OS. Para verificar/probar/demostrar sistemas usa AGENTIC_TASK. Para charla general usa CONVERSATION.\n\
        ⚠️ REGLA FAST_TRACK_OS: El 'os_command' debe ser el comando mínimo posible. NUNCA inventes nombres de proyectos, títulos de ventanas, ni rutas. Usa siempre rutas relativas simples o solo el nombre del archivo.\n\n\\
        ESTRUCTURA DEL JSON A DEVOLVER (OBLIGATORIA):\n\
        {{\n\
          \"intent_type\": \"CONVERSATION\" | \"FAST_TRACK_OS\" | \"AGENTIC_TASK\",\n\
          \"technical_translation\": \"<Si es AGENTIC_TASK, escribe una instrucción técnica súper clara para el Agente Programador. Si no, deja en null>\",\n\
          \"os_command\": \"<Si es FAST_TRACK_OS, escribe el comando de consola exacto (ej: 'mkdir fenix'). Si no, deja en null>\",\n\
          \"direct_response\": \"<Si es CONVERSATION, responde al usuario aquí mismo de manera fluida y natural. Si no, deja en null>\"\n\
        }}\n\n\
        EJEMPLO 1:\n\
        Usuario: hola como estas\n\
        JSON:\n{{\"intent_type\":\"CONVERSATION\",\"technical_translation\":null,\"os_command\":null,\"direct_response\":\"¡Hola! Todo en orden por aquí. ¿En qué te ayudo hoy?\"}}\n\
        EJEMPLO 2:\n\
        Usuario: hazme una carpeta llamdo prueba pls\n\
        JSON:\n{{\"intent_type\":\"FAST_TRACK_OS\",\"technical_translation\":null,\"os_command\":\"mkdir \\\"prueba\\\"\",\"direct_response\":null}}\n\
        EJEMPLO 2b:\n\
        Usuario: borra el archivo test.bat\n\
        JSON:\n{{\"intent_type\":\"FAST_TRACK_OS\",\"technical_translation\":null,\"os_command\":\"del /f \\\"test.bat\\\"\",\"direct_response\":null}}\n\
        EJEMPLO 2c:\n\
        Usuario: lista los archivos\n\
        JSON:\n{{\"intent_type\":\"FAST_TRACK_OS\",\"technical_translation\":null,\"os_command\":\"dir\",\"direct_response\":null}}\n\
        EJEMPLO 3:\n\
        Usuario: metele un hello world al main viejo\n\
        JSON:\n{{\"intent_type\":\"AGENTIC_TASK\",\"technical_translation\":\"Modificar el archivo main existente para que imprima 'Hello World' o su equivalente en el lenguaje del proyecto usando TOOL_PROGRAMMER.\",\"os_command\":null,\"direct_response\":null}}\n\
        EJEMPLO 4 (CRITICO - no confundir):\n\
        Usuario: prueba que el sistema funcione correctamente\n\
        JSON:\n{{\"intent_type\":\"AGENTIC_TASK\",\"technical_translation\":\"Verificar que el proyecto actual funciona correctamente: usar TOOL_AUDITOR para leer el estado actual, ejecutar el script principal con TOOL_TERMINAL y confirmar que no hay errores.\",\"os_command\":null,\"direct_response\":null}}\n\
        EJEMPLO 5 (CRITICO - no confundir):\n\
        Usuario: verifica que todo esté bien\n\
        JSON:\n{{\"intent_type\":\"AGENTIC_TASK\",\"technical_translation\":\"Auditar el estado del workspace y verificar que los archivos están completos y funcionales.\",\"os_command\":null,\"direct_response\":null}}\n\n\
        {}Usuario: {}\n",
        context_str,
        user_input
    );
    
    emit_event(app_handle, 0, &format!("Analizando intención con {}...", model), "PLANNING");
    
    match call_ollama_text(&model, &system_prompt).await {
        Ok(mut res) => {
            res = res.trim().to_string();
            // Limpiar markdown json tags
            if res.starts_with("```json") { res = res.trim_start_matches("```json").to_string(); }
            else if res.starts_with("```") { res = res.trim_start_matches("```").to_string(); }
            if res.ends_with("```") { res = res.trim_end_matches("```").to_string(); }
            res = res.trim().to_string();

            if res.is_empty() {
                emit_event(app_handle, 0, "Fallo cognitivo en NLU (Vacío). Forzando tarea agente.", "WARNING");
                format!("{{\"intent_type\":\"AGENTIC_TASK\",\"technical_translation\":\"{}\",\"os_command\":null,\"direct_response\":null}}", user_input.replace('"', "\\\""))
            } else {
                emit_event(app_handle, 0, "Intención clasificada exitosamente.", "SUCCESS");
                res
            }
        },
        Err(e) => {
            emit_event(app_handle, 0, &format!("NLU Falló: {}. Forzando tarea agente.", e), "ERROR");
            format!("{{\"intent_type\":\"AGENTIC_TASK\",\"technical_translation\":\"{}\",\"os_command\":null,\"direct_response\":null}}", user_input.replace('"', "\\\""))
        }
    }
}
