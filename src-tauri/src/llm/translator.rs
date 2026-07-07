use super::call_ollama_text;
use tauri::AppHandle;
use crate::llm::agent::emit_event;

/// NLU Phase 0: Spell-corrects the raw user input, then classifies intent.
/// Uses the Orchestrator-tier model (7b+) for reliable understanding of
/// typo-heavy, implicit or complex instructions.
pub async fn translate_to_technical_intent(user_input: &str, app_handle: &AppHandle, chat_history: &[String]) -> String {
    // Resolve available models via Ollama HTTP API
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

    // ── CAMBIO 1: Usar modelo Orchestrator (7b+) en lugar de FastTrack (1.5b) ──
    // El NLU necesita comprensión semántica real. FastTrack es demasiado pequeño
    // para interpretar mandatos complejos o con errores tipográficos.
    let task_ctx = crate::llm::router::TaskContext {
        task_type: crate::llm::router::TaskType::Orchestrator,
        language: None,
    };

    let model = crate::llm::router::get_best_model(&task_ctx, &available_models, app_handle, 0).await
        .unwrap_or_else(|_| "qwen2.5-coder:7b".to_string());

    // ── CAMBIO 2: Corrector ortográfico explícito (Fase 0 del NLU) ──────────
    // Antes de clasificar el mandato, lo reescribimos limpio. El LLM
    // recibirá siempre texto con gramática correcta aunque el usuario escriba
    // con errores graves.
    emit_event(app_handle, 0, &format!("Analizando intención con {}...", model), "PLANNING");

    let correction_prompt = format!(
        "Eres un corrector de texto especializado en español técnico. \
        Tu ÚNICA función es corregir errores ortográficos, tipográficos y gramaticales del texto dado. \
        REGLAS ABSOLUTAS:\
        \n- Conserva el significado y la intención original. \
        \n- No añadas ni quites información. \
        \n- No respondas con explicaciones, comentarios ni saludos. \
        \n- Devuelve ÚNICAMENTE el texto corregido. Nada más.\
        \n\nTexto a corregir: {}",
        user_input
    );

    let corrected_input = call_ollama_text(&model, &correction_prompt).await
        .map(|r| {
            // Sanitize: strip markdown artifacts the model may add
            let cleaned = r.trim()
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim()
                .to_string();
            if cleaned.is_empty() { user_input.to_string() } else { cleaned }
        })
        .unwrap_or_else(|_| user_input.to_string());

    // Log the correction for debugging (visible in Tauri dev console)
    if corrected_input != user_input {
        println!("[NLU CORRECTOR] '{}' → '{}'", user_input, corrected_input);
    }

    let context_str = if chat_history.is_empty() {
        "".to_string()
    } else {
        format!("CONTEXTO RECIENTE DE LA CONVERSACIÓN:\n{}\n\n", chat_history.join("\n"))
    };

    // ── CAMBIO 3: Añadir NEEDS_CLARIFICATION al clasificador de intención ───
    let system_prompt = format!(
        "Eres el Analista de Intenciones (NLU) de Aura-Sentinel. \
        Tu única función es leer lo que dice el usuario y clasificar la petición en uno de estos cuatro tipos, DEVOLVIENDO UN ÚNICO OBJETO JSON VÁLIDO (sin texto extra):\n\n\
        TIPOS DE INTENCIÓN:\n\
        1. \"CONVERSATION\": El usuario solo está saludando, haciendo charla general o haciendo PREGUNTAS BÁSICAS DE CONOCIMIENTO (ej: \"qué es btc\").\n\
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
        ⚠️ REGLA FAST_TRACK_OS: El 'os_command' debe ser el comando mínimo posible. NUNCA inventes nombres de proyectos, títulos de ventanas, ni rutas. Usa siempre rutas relativas simples o solo el nombre del archivo.\n\
        4. \"NEEDS_CLARIFICATION\": La petición es tan vaga, incompleta o contradictoria que ejecutarla causaría resultados incorrectos o imposibles. Úsalo SOLO cuando genuinamente no hay suficiente información para actuar. Si es RAZONABLEMENTE claro, usa AGENTIC_TASK.\n\
        EJEMPLOS que SÍ requieren clarificación: 'haz una app', 'crea un sistema', 'mejora mi código' (sin especificar qué).\n\
        EJEMPLOS que NO requieren clarificación: 'crea una calculadora en Python', 'haz un servidor web básico en Node.js'.\n\n\
        ESTRUCTURA DEL JSON A DEVOLVER (OBLIGATORIA):\n\
        {{\n\
          \"intent_type\": \"CONVERSATION\" | \"FAST_TRACK_OS\" | \"AGENTIC_TASK\" | \"NEEDS_CLARIFICATION\",\n\
          \"technical_translation\": \"<Si es AGENTIC_TASK, escribe una instrucción técnica súper clara, paso a paso, para el Agente Programador. Si no, deja en null>\",\n\
          \"os_command\": \"<Si es FAST_TRACK_OS, escribe el comando de consola exacto (ej: 'mkdir fenix'). Si no, deja en null>\",\n\
          \"direct_response\": \"<Si es CONVERSATION, responde al usuario aquí mismo de manera fluida y natural. Si no, deja en null>\",\n\
          \"clarification_question\": \"<Si es NEEDS_CLARIFICATION, escribe UNA pregunta clara y específica que le permita al usuario completar su intención. Si no, deja en null>\"\n\
        }}\n\n\
        EJEMPLO 1:\n\
        Usuario: hola como estas\n\
        JSON:\n{{\"intent_type\":\"CONVERSATION\",\"technical_translation\":null,\"os_command\":null,\"direct_response\":\"¡Hola! Todo en orden por aquí. ¿En qué te ayudo hoy?\",\"clarification_question\":null}}\n\
        EJEMPLO 2:\n\
        Usuario: hazme una carpeta llamdo prueba pls\n\
        JSON:\n{{\"intent_type\":\"FAST_TRACK_OS\",\"technical_translation\":null,\"os_command\":\"mkdir \\\"prueba\\\"\",\"direct_response\":null,\"clarification_question\":null}}\n\
        EJEMPLO 3:\n\
        Usuario: metele un hello world al main viejo\n\
        JSON:\n{{\"intent_type\":\"AGENTIC_TASK\",\"technical_translation\":\"Modificar el archivo main existente para que imprima 'Hello World' o su equivalente en el lenguaje del proyecto usando TOOL_PROGRAMMER.\",\"os_command\":null,\"direct_response\":null,\"clarification_question\":null}}\n\
        EJEMPLO 4 (CLARIFICACIÓN):\n\
        Usuario: haz una app\n\
        JSON:\n{{\"intent_type\":\"NEEDS_CLARIFICATION\",\"technical_translation\":null,\"os_command\":null,\"direct_response\":null,\"clarification_question\":\"¿Qué tipo de app quieres que cree? Por ejemplo: ¿web, de escritorio, móvil? ¿En qué lenguaje? ¿Qué funcionalidad debe tener?\"}}\n\
        EJEMPLO 5 (COMPLEJO - NO clarificación):\n\
        Usuario: crea una calculadora en JavaScript con dark mode glassmorphism\n\
        JSON:\n{{\"intent_type\":\"AGENTIC_TASK\",\"technical_translation\":\"Crear 3 archivos en la raíz del workspace: (1) index.html con estructura HTML5 semántica, (2) style.css con tema Dark Mode usando efecto glassmorphism (backdrop-filter: blur, bordes semitransparentes, fondo oscuro), (3) app.js con lógica completa de calculadora (operaciones básicas, teclado numérico, display). Usar TOOL_PROGRAMMER para cada archivo, TOOL_TESTER para validar, y TOOL_VISION_EVALUATOR para verificar visualmente.\",\"os_command\":null,\"direct_response\":null,\"clarification_question\":null}}\n\n\
        {}Usuario: {}\n",
        context_str,
        corrected_input  // ← usamos el texto CORREGIDO, no el original
    );

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
                format!("{{\"intent_type\":\"AGENTIC_TASK\",\"technical_translation\":\"{}\",\"os_command\":null,\"direct_response\":null,\"clarification_question\":null}}", corrected_input.replace('"', "\\\""))
            } else {
                emit_event(app_handle, 0, "Intención clasificada exitosamente.", "SUCCESS");
                res
            }
        },
        Err(e) => {
            emit_event(app_handle, 0, &format!("NLU Falló: {}. Forzando tarea agente.", e), "ERROR");
            format!("{{\"intent_type\":\"AGENTIC_TASK\",\"technical_translation\":\"{}\",\"os_command\":null,\"direct_response\":null,\"clarification_question\":null}}", corrected_input.replace('"', "\\\""))
        }
    }
}
