/// Only standalone continuation requests may restore an existing mission.
pub fn is_resume_command(message: &str) -> bool {
    let message = strip_injected_context(message).trim();
    matches!(
        message.to_lowercase().as_str(),
        "continua"
            | "continúa"
            | "continuar"
            | "continue"
            | "sigue"
            | "adelante"
            | "retoma"
            | "retoma la tarea"
            | "resume"
            | "donde me quede"
            | "donde me quedé"
    )
}

/// Detect an interface review that asks to inspect/test the existing page but
/// does not ask Aura to modify it. This is a deterministic scope boundary: an
/// imperfect NLU response must not turn a review request into construction.
pub fn is_scoped_visual_review(message: &str) -> bool {
    let normalized = normalize_approval_phrase(message);
    let asks_for_review = [
        "revisa",
        "revisar",
        "inspecciona",
        "inspeccionar",
        "analiza",
        "analizar",
        "evalua",
        "evaluar",
        "comprueba",
        "comprobar",
        "verifica",
        "verificar",
        "audita",
    ]
    .iter()
    .any(|cue| normalized.contains(cue));
    let targets_interface = [
        "pagina",
        "interfaz",
        "pantalla",
        "login",
        "diseno",
        "estructura visual",
        "modulos",
        "modulo",
        "navegador",
        "web",
        "sitio",
        "frontend",
    ]
    .iter()
    .any(|cue| normalized.contains(cue));
    let asks_for_changes = [
        "corrige",
        "corregir",
        "arregla",
        "arreglar",
        "repara",
        "reparar",
        "modifica",
        "modificar",
        "cambia",
        "cambiar",
        "agrega",
        "agregar",
        "anade",
        "anadir",
        "redisenia",
        "construye",
        "construir",
        "implementa",
        "implementar",
        "actualiza",
        "actualizar",
        "aplica los cambios",
        "haz los cambios",
    ]
    .iter()
    .any(|cue| normalized.contains(cue));

    asks_for_review && targets_interface && !asks_for_changes
}

/// Profile and translation context are appended after the user's message. They
/// must not turn a one-word continuation into a new mission objective.
fn strip_injected_context(message: &str) -> &str {
    let lower = message.to_lowercase();
    let markers = [
        "\n\n[perfil local del usuario y contexto]",
        "\n\nguía de traducción técnica",
        "\n\nguia de traduccion tecnica",
        "\n\n[modo plan inicial]",
    ];
    let end = markers.iter().filter_map(|marker| lower.find(marker)).min();
    let message = end.map_or(message, |index| &message[..index]).trim();
    let lower_message = message.to_lowercase();
    [
        "petición original del usuario:",
        "peticion original del usuario:",
        "usuario:",
    ]
    .iter()
    .find_map(|prefix| {
        lower_message
            .starts_with(prefix)
            .then(|| message[prefix.len()..].trim())
    })
    .unwrap_or(message)
}

/// Approval words are deliberately exact: a short "procede" should continue
/// the immediately preceding approved plan, not become a new empty objective.
pub fn is_plan_approval_command(message: &str) -> bool {
    let normalized = normalize_approval_phrase(message);
    matches!(
        normalized.as_str(),
        "procede"
            | "proceder"
            | "procede con el plan"
            | "procede con la mejora"
            | "procede con la fase 1"
            | "procede con fase 1"
            | "continua con la fase 1"
            | "continua con fase 1"
            | "empecemos con la fase 1"
            | "empecemos con fase 1"
            | "empecemos fase 1"
            | "comencemos con la fase 1"
            | "comencemos con fase 1"
            | "comencemos fase 1"
            | "iniciemos con la fase 1"
            | "iniciemos con fase 1"
            | "iniciemos fase 1"
            | "empieza con la fase 1"
            | "empieza con fase 1"
            | "inicia la fase 1"
            | "inicia fase 1"
            | "adelante"
            | "hazlo"
            | "inicia"
            | "empieza"
            | "implementa el plan"
    )
}

fn normalize_approval_phrase(message: &str) -> String {
    message
        .trim()
        .to_lowercase()
        .chars()
        .map(|character| match character {
            'á' => 'a',
            'é' => 'e',
            'í' => 'i',
            'ó' => 'o',
            'ú' => 'u',
            'ñ' => 'n',
            other if other.is_alphanumeric() => other,
            _ => ' ',
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Rebuilds the implementation request from the last user objective and the
/// immediately preceding assistant plan in the visible project conversation.
/// Returns None for unrelated approvals, stale plans, or malformed history.
pub fn recover_approved_plan_request(chat_json: &str, current_message: &str) -> Option<String> {
    if !is_plan_approval_command(current_message) {
        return None;
    }
    recover_plan_request_from_history(chat_json, current_message, false)
}

/// Restores the last user-approved plan after a failed attempt when the user
/// says "continua". Every intervening user turn must still refer to that plan.
pub fn recover_pending_approved_plan_request(
    chat_json: &str,
    current_message: &str,
) -> Option<String> {
    if !is_resume_command(current_message) {
        return None;
    }
    recover_plan_request_from_history(chat_json, current_message, true)
}

/// Restores the user's full task when the previous assistant turn asked for
/// clarification. Small local models sometimes treat the answer as a new task
/// and discard the request that prompted the question.
pub fn recover_pending_clarification_request(
    chat_json: &str,
    current_message: &str,
) -> Option<String> {
    let current = strip_injected_context(current_message).trim();
    if current.is_empty() || clearly_starts_new_task(current) {
        return None;
    }

    let messages: Vec<serde_json::Value> = serde_json::from_str(chat_json).ok()?;
    let current_index = messages
        .iter()
        .rposition(|message| {
            message.get("sender").and_then(|value| value.as_str()) == Some("user")
                && message
                    .get("text")
                    .and_then(|value| value.as_str())
                    .is_some_and(|text| text.trim().eq_ignore_ascii_case(current))
        })
        .unwrap_or(messages.len());

    // The most recent assistant question must be unanswered until this turn.
    let question_index = messages[..current_index].iter().rposition(|message| {
        let sender = message.get("sender").and_then(|value| value.as_str());
        if sender == Some("user") {
            return false;
        }
        let Some(text) = message.get("text").and_then(|value| value.as_str()) else {
            return false;
        };
        text.contains(PENDING_CLARIFICATION_MARKER) || (text.contains('?') || text.contains('¿'))
    })?;

    // Do not reuse a stale question after any intervening user turn.
    if messages[question_index + 1..current_index]
        .iter()
        .any(|message| message.get("sender").and_then(|value| value.as_str()) == Some("user"))
    {
        return None;
    }

    let original = messages[..question_index]
        .iter()
        .rev()
        .find(|message| message.get("sender").and_then(|value| value.as_str()) == Some("user"))
        .and_then(|message| message.get("text"))
        .and_then(|value| value.as_str())
        .map(strip_injected_context)?
        .trim();
    if original.chars().count() < 30
        || is_plan_approval_command(original)
        || is_resume_command(original)
        || clearly_starts_new_task(original)
    {
        return None;
    }

    Some(format!(
        "[RECUPERACION DE ACLARACION PENDIENTE]\n\
         Mandato original del usuario:\n{}\n\n\
         Respuesta del usuario a la aclaración:\n{}\n\n\
         Instrucción: conserva el mandato original completo. Incorpora la respuesta como contexto o preferencia; si el usuario confirma que los datos ya están claros, procede con el mandato sin volver a preguntar lo mismo. No sustituyas la tarea por una distinta.",
        original, current
    ))
}

const PENDING_CLARIFICATION_MARKER: &str = "[ACLARACION NLU PENDIENTE]";

pub fn pending_clarification_marker() -> &'static str {
    PENDING_CLARIFICATION_MARKER
}

fn clearly_starts_new_task(message: &str) -> bool {
    let normalized = normalize_approval_phrase(message);
    [
        "nuevo proyecto",
        "nueva tarea",
        "otra tarea",
        "otro proyecto",
        "otra aplicacion",
        "cambiemos de tema",
        "olvida lo anterior",
        "ignora lo anterior",
    ]
    .iter()
    .any(|cue| normalized.contains(cue))
}

fn recover_plan_request_from_history(
    chat_json: &str,
    current_message: &str,
    allow_resume_commands: bool,
) -> Option<String> {
    let messages: Vec<serde_json::Value> = serde_json::from_str(chat_json).ok()?;
    let current_index = messages
        .iter()
        .rposition(|message| {
            message.get("sender").and_then(|value| value.as_str()) == Some("user")
                && message
                    .get("text")
                    .and_then(|value| value.as_str())
                    .is_some_and(|text| text.trim().eq_ignore_ascii_case(current_message.trim()))
        })
        .unwrap_or(messages.len());
    let plan_index = messages[..current_index].iter().rposition(|message| {
        if message.get("sender").and_then(|value| value.as_str()) == Some("user") {
            return false;
        }
        let Some(text) = message.get("text").and_then(|value| value.as_str()) else {
            return false;
        };
        is_approved_plan_text(text)
    })?;
    // A failed implementation may have appended an error after the approved
    // plan. Reuse it only when every later user message was another approval,
    // so an unrelated newer request can never inherit a stale plan.
    let approvals_only = messages[plan_index + 1..current_index]
        .iter()
        .all(|message| {
            message.get("sender").and_then(|value| value.as_str()) != Some("user")
                || message
                    .get("text")
                    .and_then(|value| value.as_str())
                    .is_some_and(|text| {
                        is_plan_approval_command(text)
                            || (allow_resume_commands && is_resume_command(text))
                    })
        });
    if !approvals_only {
        return None;
    }
    if allow_resume_commands
        && !messages[plan_index + 1..current_index]
            .iter()
            .any(|message| {
                message.get("sender").and_then(|value| value.as_str()) == Some("user")
                    && message
                        .get("text")
                        .and_then(|value| value.as_str())
                        .is_some_and(is_plan_approval_command)
            })
    {
        return None;
    }
    let plan = messages[plan_index]
        .get("text")
        .and_then(|value| value.as_str())?
        .trim();
    let original = messages[..plan_index]
        .iter()
        .rev()
        .find(|message| message.get("sender").and_then(|value| value.as_str()) == Some("user"))
        .and_then(|message| message.get("text"))
        .and_then(|value| value.as_str())?
        .trim();
    if original.len() < 20 || is_plan_approval_command(original) || is_resume_command(original) {
        return None;
    }

    Some(format!(
        "[MODO IMPLEMENTACION DE PLAN APROBADO]\n\
         Solicitud original del usuario:\n{}\n\n\
         Hoja de ruta que el usuario acaba de aprobar:\n{}\n\n\
         Instrucción actual: implementa esta solicitud siguiendo la hoja de ruta aprobada. Empieza por la fase local, construye la aplicación web funcional y ejecuta pruebas reales de sus flujos solicitados. Para el MVP web usa HTML, CSS y JavaScript salvo que el usuario haya solicitado otro stack; no introduzcas Python/Flask ni dependencias sin necesidad, y verifica las dependencias declaradas antes de ejecutar. No reduzcas la entrega a un archivo que solo imprima OK. Deja el despliegue remoto para después de que las pruebas locales pasen.",
        original, plan
    ))
}

fn is_approved_plan_text(text: &str) -> bool {
    let lower = text.to_lowercase();
    (lower.contains("plan inicial") || lower.contains("hoja de ruta"))
        && lower.contains("fase 1")
        && lower.contains("fase 2")
        && lower.contains("entregable")
}

pub const ACTIVE_MISSION_FOLLOW_UP_MARKER: &str = "[SEGUIMIENTO DE MISION ACTIVA]";

/// A durable journal snapshot gives the NLU the context needed to resolve
/// references such as "that part", short answers, and phase edits.
pub fn active_mission_context(
    journal: &crate::core::session_journal::SessionJournal,
) -> Option<String> {
    if journal.objetivo.trim().is_empty()
        || !matches!(
            journal.status.as_str(),
            "EN_PROGRESO" | "ESPERANDO" | "FALLIDO"
        )
    {
        return None;
    }
    let phases = journal
        .fases
        .iter()
        .enumerate()
        .map(|(index, phase)| {
            format!(
                "- Fase {}{} [{}]: {} | archivos: {} | verificación: {}",
                phase.numero,
                if index == journal.fase_actual {
                    " (actual)"
                } else {
                    ""
                },
                phase.estado,
                phase.descripcion,
                if phase.archivos.is_empty() {
                    "sin especificar".to_string()
                } else {
                    phase.archivos.join(", ")
                },
                if phase.criterio_de_exito.is_empty() {
                    "sin especificar"
                } else {
                    &phase.criterio_de_exito
                }
            )
        })
        .collect::<Vec<_>>();
    let recent = journal
        .chat_history
        .iter()
        .rev()
        .take(6)
        .cloned()
        .collect::<Vec<_>>();
    let phase_text = if phases.is_empty() {
        "(sin fases)".to_string()
    } else {
        phases.join("\n")
    };
    let recent_text = if recent.is_empty() {
        "(sin mensajes recientes)".to_string()
    } else {
        recent.into_iter().rev().collect::<Vec<_>>().join("\n")
    };
    Some(format!(
        "Objetivo original: {}\nEstado: {}\nFase actual: {}/{}\nPlan:\n{}\nConversación reciente:\n{}",
        journal.objetivo,
        journal.status,
        journal.fase_actual + 1,
        journal.fases.len().max(1),
        phase_text,
        recent_text
    ))
}

pub fn is_contextual_follow_up(message: &str) -> bool {
    message.starts_with(ACTIVE_MISSION_FOLLOW_UP_MARKER)
}

/// Conservative fallback for older/underspecified NLU responses. Clear new
/// project cues win; short references and edits stay attached to the active task.
pub fn fallback_turn_relation(user_message: &str, mission_context: Option<&str>) -> &'static str {
    let Some(context) = mission_context else {
        return "NEW_TASK";
    };
    let lower = user_message.trim().to_lowercase();
    let clearly_new = [
        "nuevo proyecto",
        "otra tarea",
        "otra aplicación",
        "otra aplicacion",
        "vamos a construir",
        "quiero crear",
        "empecemos otro",
        "en otra carpeta",
    ];
    if clearly_new.iter().any(|cue| lower.contains(cue)) {
        return "NEW_TASK";
    }
    let contextual = [
        "eso",
        "esa",
        "ese",
        "lo anterior",
        "la anterior",
        "el anterior",
        "opción",
        "opcion",
        "fase",
        "procede",
        "continúa",
        "continua",
        "hazlo",
        "cambia",
        "modifica",
        "agrega",
        "añade",
        "quita",
        "pon",
        "me refiero",
        "como te dije",
    ];
    let short_yes_no = lower.chars().count() <= 24
        && lower
            .split(|character: char| !character.is_alphanumeric())
            .any(|word| matches!(word, "si" | "sí" | "no"));
    let last_assistant_question = context
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with("Aura:"))
        .is_some_and(|line| line.contains('?') || line.contains('¿'));
    let short_answer = last_assistant_question && lower.chars().count() <= 72;
    if short_answer || short_yes_no || contextual.iter().any(|cue| lower.contains(cue)) {
        "FOLLOW_UP"
    } else {
        "NEW_TASK"
    }
}

pub fn contextual_follow_up_objective(message: &str) -> Option<&str> {
    if !is_contextual_follow_up(message) {
        return None;
    }
    message
        .lines()
        .find_map(|line| line.strip_prefix("Objetivo original de la misión: "))
}

pub fn contextual_follow_up_instruction(message: &str) -> Option<&str> {
    if !is_contextual_follow_up(message) {
        return None;
    }
    message
        .lines()
        .find_map(|line| line.strip_prefix("Interpretación corregida: "))
}

pub fn contextual_follow_up_request(
    context: &str,
    user_message: &str,
    corrected_instruction: &str,
) -> Option<String> {
    let objective = context
        .lines()
        .find_map(|line| line.strip_prefix("Objetivo original: "))?;
    Some(format!(
        "{ACTIVE_MISSION_FOLLOW_UP_MARKER}\nObjetivo original de la misión: {objective}\nInstrucción nueva del usuario: {user_message}\nInterpretación corregida: {corrected_instruction}\n\nContexto persistido de la misión:\n{context}\n\nRegla: continúa el diario y la fase existentes. Aplica la instrucción nueva al trabajo pendiente y conserva los requisitos anteriores que no contradiga. No reinicies el proyecto."
    ))
}

/// Applies only model-resolved updates to explicitly numbered, existing phases.
/// An empty or malformed update leaves the saved plan untouched.
pub fn apply_phase_plan_updates(
    journal: &mut crate::core::session_journal::SessionJournal,
    nlu: &serde_json::Value,
) -> usize {
    let Some(updates) = nlu.get("phase_updates").and_then(|value| value.as_array()) else {
        return 0;
    };
    let mut applied = 0;
    for update in updates {
        let Some(number) = update
            .get("numero")
            .and_then(|value| value.as_u64())
            .and_then(|value| u32::try_from(value).ok())
        else {
            continue;
        };
        let Some(description) = update
            .get("descripcion")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(index) = journal
            .fases
            .iter()
            .position(|phase| phase.numero == number)
        else {
            continue;
        };
        if let Some(files) = update.get("archivos").and_then(|value| value.as_array()) {
            let safe_relative_files = files.iter().all(|file| {
                let Some(file) = file.as_str().map(str::trim).filter(|file| !file.is_empty())
                else {
                    return false;
                };
                let path = std::path::Path::new(file);
                !path.is_absolute()
                    && path.components().all(|component| {
                        !matches!(
                            component,
                            std::path::Component::ParentDir
                                | std::path::Component::RootDir
                                | std::path::Component::Prefix(_)
                        )
                    })
            });
            if !safe_relative_files {
                continue;
            }
        }
        let phase = &mut journal.fases[index];
        phase.descripcion = description.to_string();
        if let Some(files) = update.get("archivos").and_then(|value| value.as_array()) {
            phase.archivos = files
                .iter()
                .filter_map(|file| file.as_str().map(str::to_string))
                .collect();
        }
        if let Some(criterion) = update
            .get("criterio_de_exito")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            phase.criterio_de_exito = criterion.to_string();
        }
        phase.estado = "PENDIENTE".to_string();
        journal.fase_actual = journal.fase_actual.min(index);
        applied += 1;
    }
    applied
}

pub enum IntentAction {
    Finish(String),
    Resume {
        objetivo: String,
        resume_msg: String,
    },
}

/// Zero-latency intent router that intercepts meta-queries BEFORE the LLM pipeline.
pub fn try_handle_meta_command(user_message: &str, workspace_path: &str) -> Option<IntentAction> {
    try_handle_meta_command_with_recovery(user_message, workspace_path, None)
}

pub fn try_handle_meta_command_with_recovery(
    user_message: &str,
    workspace_path: &str,
    recovered_objective: Option<&str>,
) -> Option<IntentAction> {
    let msg = user_message.trim().to_lowercase();

    // ── Status / pending task queries ───────────────────────────────────────
    let is_status_query = contains_any(
        &msg,
        &[
            "que tenia pendiente",
            "qué tenia pendiente",
            "que tenía pendiente",
            "qué tenía pendiente",
            "que estaba haciendo",
            "qué estaba haciendo",
            "revisa que tarea",
            "revisa qué tarea",
            "cual era mi tarea",
            "cuál era mi tarea",
            "que tarea tenia",
            "qué tarea tenía",
            "en que estaba",
            "en qué estaba",
            "estado de la mision",
            "estado de la misión",
            "show status",
            "mission status",
            "que me falta",
            "qué me falta",
            "que habia hecho",
            "qué había hecho",
            "resumen de la tarea",
            "check task",
            "pending task",
        ],
    );

    if is_status_query {
        let journal = crate::core::session_journal::load_journal(workspace_path);
        let report = crate::core::session_journal::build_status_report(&journal);
        return Some(IntentAction::Finish(report));
    }

    // ── Resume / continue task ───────────────────────────────────────────────
    let is_resume = is_resume_command(&msg);

    if is_resume {
        let journal = crate::core::session_journal::load_journal(workspace_path);
        let recovered_objective = recovered_objective
            .map(str::trim)
            .filter(|objective| objective.len() >= 20);
        let saved_objective = strip_injected_context(&journal.objetivo);
        let saved_objective_is_valid =
            saved_objective.len() >= 20 && !is_resume_command(saved_objective);
        let objective_needs_repair = saved_objective != journal.objetivo;
        let mission_can_resume = journal.status != "COMPLETADO"
            && (matches!(
                journal.status.as_str(),
                "EN_PROGRESO" | "ESPERANDO" | "FALLIDO"
            ) || journal.interrupted);
        if mission_can_resume && (recovered_objective.is_some() || saved_objective_is_valid) {
            let objetivo = recovered_objective
                .map(str::to_string)
                .unwrap_or_else(|| saved_objective.to_string());
            let resume_step = if recovered_objective.is_some() || objective_needs_repair {
                1
            } else {
                journal.ultimo_paso + 1
            };
            // Return a message that tells the frontend to re-run with the saved objective
            let resume_msg = format!(
                "🔄 **Retomando misión desde el paso {}**\n\n\
                 🎯 Objetivo: {}\n\n\
                 {}Reiniciando el agente con el contexto guardado...",
                resume_step,
                objetivo,
                if recovered_objective.is_some() {
                    "📚 Recuperé el objetivo y el plan aprobados desde el historial.\n\n"
                } else {
                    ""
                }
            );
            return Some(IntentAction::Resume {
                objetivo,
                resume_msg,
            });
        } else if saved_objective_is_valid && journal.status == "COMPLETADO" {
            return Some(IntentAction::Finish(
                format!("✅ La misión previa ya fue completada con éxito:\n\n🎯 *{}*\n\nSi deseas una nueva tarea o auditoría, envíame una nueva instrucción.", journal.objetivo)
            ));
        } else if recovered_objective.is_some() && journal.status != "COMPLETADO" {
            let objetivo = recovered_objective.unwrap().to_string();
            let resume_msg = format!(
                "🔄 **Recuperando la misión aprobada desde el historial**\n\n🎯 Objetivo: {}\n\nReiniciando el agente con el plan guardado...",
                objetivo
            );
            return Some(IntentAction::Resume {
                objetivo,
                resume_msg,
            });
        } else {
            return Some(IntentAction::Finish(
                "📋 No hay ninguna tarea activa que retomar en este workspace. Dame un nuevo objetivo y empezamos.".to_string()
            ));
        }
    }

    // ── Help / capabilities ──────────────────────────────────────────────────
    let is_help = contains_any(
        &msg,
        &[
            "que puedes hacer",
            "qué puedes hacer",
            "ayuda",
            "help",
            "comandos disponibles",
            "herramientas disponibles",
        ],
    );

    if is_help {
        let help_text = r#"🛡️ **Aura-Sentinel — Comandos Disponibles**
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
**Meta-Comandos (instantáneos, sin IA):**
• `"revisa qué tarea tenía pendiente"` — Muestra el estado de la última misión
• `"continúa"` / `"retoma"` — Reanuda la misión donde se quedó
• `"ayuda"` — Este menú

**Herramientas de Desarrollo:**
• TOOL_PROGRAMMER — Escribe/modifica código en disco
• TOOL_TERMINAL — Ejecuta comandos del sistema
• TOOL_TESTER — Corre suites de pruebas automáticamente
• TOOL_ENV_MANAGER — Instala dependencias del sistema (Scoop/winget)
• TOOL_AUDITOR — Audita el código existente
• TOOL_ARCHITECT — Genera mapa de dependencias
• TOOL_BACKGROUND_START/READ/KILL — Servidores en segundo plano
• TOOL_WEB_SCRAPER — Extrae contenido de URLs
• TOOL_LEARN / TOOL_SEARCH — Memoria vectorial persistente
• TOOL_FINISH — Cierra la misión

**Lenguajes Soportados:**
Python, Rust, Go, JavaScript/TypeScript, Solidity (Hardhat/Foundry),
Kotlin/Android, Dart/Flutter, PHP, Swift, C, C++, Java
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"#;
        return Some(IntentAction::Finish(help_text.to_string()));
    }

    // Not a meta-command — let the normal pipeline handle it
    None
}

/// Helper: returns true if `s` is identical to any pattern, or starts with it
/// and the entire message is short (under 40 chars) to prevent hijacking long prompts.
fn contains_any(s: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|&p| {
        if s == p {
            return true;
        }
        if s.starts_with(p) && s.len() < 40 {
            return true;
        }
        false
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resume_requires_an_explicit_command() {
        assert!(is_resume_command("  Continúa  "));
        assert!(is_resume_command("resume"));
        assert!(is_resume_command(
            "continua\n\n[PERFIL LOCAL DEL USUARIO Y CONTEXTO]\nnombre preferido: ramon"
        ));
        assert!(is_resume_command(
            "Petición Original del Usuario: continua\n\nGuía de Traducción Técnica: retoma el diario"
        ));
        assert!(!is_resume_command("implementa integración continua"));
        assert!(!is_resume_command("resume los cambios del proyecto"));
        assert!(!is_resume_command("arregla el siguiente error"));
        assert!(!is_resume_command("continua y cambia el diseño"));
    }

    #[test]
    fn visual_page_review_is_deterministic_and_never_means_editing() {
        assert!(is_scoped_visual_review(
            "revisa si la pagina esta bien estruturada y biseno profesional desde el login a los modulo levanta local y prueba"
        ));
        assert!(is_scoped_visual_review(
            "debe iniciar la pagina local revisa el diseno actual para una pagina profesional"
        ));
        assert!(!is_scoped_visual_review(
            "revisa la pagina y corrige los defectos visuales que encuentres"
        ));
        assert!(!is_scoped_visual_review(
            "construye una aplicacion web con login"
        ));
    }

    #[test]
    fn proceed_after_a_plan_recovers_the_original_objective_and_plan() {
        let history = serde_json::json!([
            { "sender": "user", "text": "Construye una aplicación web de consultas con login, facturación y contabilidad; prueba local y luego Firebase." },
            { "sender": "system", "text": "## Plan inicial\n\n### Fase 1\nEntregables: aplicación y pruebas locales.\n### Fase 2\nEntregables: Firebase Hosting." },
            { "sender": "user", "text": "procede" }
        ]).to_string();

        let request = recover_approved_plan_request(&history, "procede")
            .expect("the immediately preceding message is the user's plan");
        assert!(request.contains("[MODO IMPLEMENTACION DE PLAN APROBADO]"));
        assert!(request.contains("facturación y contabilidad"));
        assert!(request.contains("Fase 1"));
        assert!(request.contains("pruebas reales"));
    }

    #[test]
    fn clarification_answer_restores_the_original_request_instead_of_becoming_a_new_task() {
        let original = "Construye y prueba en este workspace una aplicación web local llamada Consulta Clara, con registro, agenda de consultas, facturación y contabilidad. Usa HTML, CSS y JavaScript; no despliegues ni instales dependencias.";
        let answer = "Todos los datos están claros en el mandato, analiza y procede.";
        let history = serde_json::json!([
            { "sender": "user", "text": original },
            { "sender": "assistant", "text": "¿Puedes darme más detalles sobre lo que necesitas? Quiero entenderte antes de empezar." },
            { "sender": "user", "text": answer }
        ]).to_string();

        let recovered = recover_pending_clarification_request(&history, answer).expect(
            "the answer should remain bound to the request that triggered the clarification",
        );
        assert!(recovered.contains(original));
        assert!(recovered.contains(answer));
        assert!(recovered.contains("No sustituyas la tarea por una distinta"));
    }

    #[test]
    fn clarification_recovery_does_not_capture_a_clear_new_project() {
        let history = serde_json::json!([
            { "sender": "user", "text": "Construye una aplicación web local para administrar citas y facturas." },
            { "sender": "assistant", "text": "¿Qué otro detalle necesitas?" },
            { "sender": "user", "text": "Crea un nuevo proyecto de inventario para una tienda." }
        ]).to_string();

        assert!(recover_pending_clarification_request(
            &history,
            "Crea un nuevo proyecto de inventario para una tienda."
        )
        .is_none());
    }

    #[test]
    fn clarification_recovery_requires_a_preceding_user_task_and_question() {
        let no_question = serde_json::json!([
            { "sender": "user", "text": "Construye una aplicación web local para administrar citas y facturas." },
            { "sender": "assistant", "text": "De acuerdo, empezaré." },
            { "sender": "user", "text": "Procede." }
        ]).to_string();
        assert!(recover_pending_clarification_request(&no_question, "Procede.").is_none());
    }

    #[test]
    fn proceed_does_not_reuse_a_stale_plan_after_an_intervening_response() {
        let history = serde_json::json!([
            { "sender": "user", "text": "Construye una aplicación web con registro de clientes y pruebas locales." },
            { "sender": "system", "text": "## Plan inicial\n### Fase 1\nEntregables: MVP.\n### Fase 2\nEntregables: despliegue." },
            { "sender": "user", "text": "Analiza otro proyecto diferente." },
            { "sender": "system", "text": "No hay nada pendiente en esta carpeta." },
            { "sender": "user", "text": "procede" }
        ]).to_string();
        assert!(recover_approved_plan_request(&history, "procede").is_none());
    }

    #[test]
    fn proceed_can_retry_an_approved_plan_after_a_failed_implementation() {
        let history = serde_json::json!([
            { "sender": "user", "text": "Construye una aplicación web de consulta y contabilidad con pruebas locales y Firebase." },
            { "sender": "system", "text": "## Plan inicial\n### Fase 1\nEntregables: aplicación y pruebas locales.\n### Fase 2\nEntregables: Firebase Hosting." },
            { "sender": "user", "text": "procede" },
            { "sender": "system", "text": "PROGRAMMER_REPAIR_EXHAUSTED: no pudo corregirse un archivo." },
            { "sender": "user", "text": "procede" }
        ]).to_string();
        assert!(recover_approved_plan_request(&history, "procede").is_some());
    }

    #[test]
    fn phase_one_approval_recovers_original_request_after_failed_run() {
        let history = serde_json::json!([
            { "sender": "user", "text": "Construye una aplicación web de consulta con login, facturación y contabilidad; prueba local y luego Firebase." },
            { "sender": "system", "text": "## Plan inicial\n### Fase 1\nEntregables: MVP y pruebas locales.\n### Fase 2\nEntregables: Firebase Hosting." },
            { "sender": "user", "text": "procede" },
            { "sender": "system", "text": "FORCED_TOOL_NOT_OBEYED: el modelo no generó TOOL_THINK." },
            { "sender": "user", "text": "procede con la fase 1" }
        ]).to_string();

        let request = recover_approved_plan_request(&history, "procede con la fase 1")
            .expect("phase approval should resume the approved objective after a failed run");
        assert!(request.contains("[MODO IMPLEMENTACION DE PLAN APROBADO]"));
        assert!(request.contains("contabilidad"));
        assert!(request.contains("pruebas reales"));
        assert!(request.contains("no introduzcas Python/Flask"));
    }

    #[test]
    fn natural_phase_one_start_recovers_the_approved_plan_from_chat_history() {
        let original = "vamos a contruir una aplicacion tipo web probamos local y luego desplegamos en firebase la aplicacion sera de consulta se hacen factura y tendra modulo de contabilidad";
        let plan = "Plan inicial\nObjetivo: aplicación web de consultas, facturación y contabilidad.\nFase 1 — MVP y pruebas locales\nEntregables: aplicación web funcional y pruebas locales.\nFase 2 — Firebase y despliegue\nEntregables: Authentication, Firestore y Hosting.";
        let history = serde_json::json!([
            { "sender": "user", "text": original },
            { "sender": "assistant", "text": plan },
            { "sender": "user", "text": "empecemos con la fase 1" }
        ])
        .to_string();

        let request = recover_approved_plan_request(&history, "empecemos con la fase 1")
            .expect("a natural phase-one approval must restore the preceding approved plan");
        assert!(request.contains("[MODO IMPLEMENTACION DE PLAN APROBADO]"));
        assert!(request.contains(original));
        assert!(request.contains("gestión de consultas") || request.contains("Fase 1"));
        assert!(request.contains("HTML, CSS y JavaScript"));
        assert!(request.contains("no introduzcas Python/Flask"));
    }

    #[test]
    fn phase_approval_normalizes_accents_and_punctuation_without_accepting_new_objectives() {
        assert!(is_plan_approval_command("Empecemos con la fase 1."));
        assert!(is_plan_approval_command("Comencemos con fase 1!"));
        assert!(is_plan_approval_command("Continúa con la fase 1"));
        assert!(!is_plan_approval_command("empecemos una aplicación nueva"));
        assert!(!is_plan_approval_command("empecemos con la fase 2"));
    }

    #[test]
    fn continue_recovers_the_last_approved_plan_after_failed_attempts() {
        let original = "Construye una aplicación web de consulta con login, facturación y contabilidad; prueba local y luego Firebase.";
        let plan = "## Plan inicial\n### Fase 1\nEntregables: MVP y pruebas locales.\n### Fase 2\nEntregables: Firebase Hosting.";
        let history = serde_json::json!([
            { "sender": "user", "text": original },
            { "sender": "system", "text": plan },
            { "sender": "user", "text": "procede con la fase 1" },
            { "sender": "system", "text": "TOOL_PROGRAMMER devolvió argumentos inválidos." },
            { "sender": "user", "text": "continua" }
        ])
        .to_string();

        let request = recover_pending_approved_plan_request(&history, "continua")
            .expect("the approved mission should remain recoverable after an implementation error");
        assert!(request.contains(original));
        assert!(request.contains(plan));
        assert!(request.contains("[MODO IMPLEMENTACION DE PLAN APROBADO]"));
    }

    #[test]
    fn continue_does_not_recover_a_plan_after_a_new_unrelated_objective() {
        let history = serde_json::json!([
            { "sender": "user", "text": "Construye una aplicación web de consulta con facturación y contabilidad; prueba local y luego Firebase." },
            { "sender": "system", "text": "## Plan inicial\n### Fase 1\nEntregables: MVP local.\n### Fase 2\nEntregables: Firebase Hosting." },
            { "sender": "user", "text": "procede" },
            { "sender": "system", "text": "La implementación falló." },
            { "sender": "user", "text": "Construye un juego distinto en esta carpeta." },
            { "sender": "user", "text": "continua" }
        ]).to_string();

        assert!(recover_pending_approved_plan_request(&history, "continua").is_none());
    }

    #[test]
    fn continue_does_not_treat_an_unapproved_plan_as_authorization() {
        let history = serde_json::json!([
            { "sender": "user", "text": "Construye una aplicación web de consulta con facturación y contabilidad; prueba local y luego Firebase." },
            { "sender": "system", "text": "## Plan inicial\n### Fase 1\nEntregables: MVP local.\n### Fase 2\nEntregables: Firebase Hosting." },
            { "sender": "user", "text": "continua" }
        ]).to_string();

        assert!(recover_pending_approved_plan_request(&history, "continua").is_none());
    }

    #[test]
    fn failed_journal_can_resume_with_the_recovered_objective() {
        let root = std::env::temp_dir().join(format!("aura-resume-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let workspace = root.to_string_lossy().to_string();
        let mut journal = crate::core::session_journal::SessionJournal::default();
        journal.objetivo = "continua\n\n[PERFIL LOCAL DEL USUARIO Y CONTEXTO]".into();
        journal.status = "FALLIDO".into();
        crate::core::session_journal::save_journal(&workspace, &journal).unwrap();
        let objective = "[MODO IMPLEMENTACION DE PLAN APROBADO]\nConstruye una aplicación web de consulta, facturación, contabilidad y Firebase.";

        let action = try_handle_meta_command_with_recovery("continua", &workspace, Some(objective))
            .expect("failed mission should route to resume");
        match action {
            IntentAction::Resume {
                objetivo,
                resume_msg,
            } => {
                assert_eq!(objetivo, objective);
                assert!(resume_msg.contains("Recuperé el objetivo y el plan"));
            }
            IntentAction::Finish(message) => panic!("unexpected finish: {message}"),
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn resume_cleans_appended_profile_from_a_valid_saved_objective() {
        let root = std::env::temp_dir().join(format!("aura-resume-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let workspace = root.to_string_lossy().to_string();
        let mut journal = crate::core::session_journal::SessionJournal::default();
        journal.objetivo = "Construye una aplicación web con pruebas locales\n\n[PERFIL LOCAL DEL USUARIO Y CONTEXTO]\nnombre preferido: ramon".into();
        journal.status = "FALLIDO".into();
        journal.ultimo_paso = 41;
        crate::core::session_journal::save_journal(&workspace, &journal).unwrap();

        let action = try_handle_meta_command("continua", &workspace)
            .expect("a failed mission with a valid objective can resume");
        match action {
            IntentAction::Resume {
                objetivo,
                resume_msg,
            } => {
                assert_eq!(objetivo, "Construye una aplicación web con pruebas locales");
                assert!(resume_msg.contains("paso 1"));
            }
            IntentAction::Finish(message) => panic!("unexpected finish: {message}"),
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn active_context_contains_current_phase_and_excludes_completed_missions() {
        let mut journal = crate::core::session_journal::SessionJournal::default();
        journal.objetivo = "Construye una aplicación web de consultas".into();
        journal.status = "EN_PROGRESO".into();
        journal.fase_actual = 1;
        journal.fases = vec![
            crate::core::session_journal::Fase {
                numero: 1,
                descripcion: "MVP local".into(),
                archivos: vec!["index.html".into()],
                criterio_de_exito: "npm test".into(),
                estado: "COMPLETADA".into(),
            },
            crate::core::session_journal::Fase {
                numero: 2,
                descripcion: "Integrar Firebase".into(),
                archivos: vec!["firebase.json".into()],
                criterio_de_exito: "npm test".into(),
                estado: "EN_PROGRESO".into(),
            },
        ];
        let context = active_mission_context(&journal).unwrap();
        assert!(context.contains("Objetivo original: Construye una aplicación web de consultas"));
        assert!(context.contains("Fase 2 (actual)"));
        assert!(context.contains("firebase.json"));
        journal.status = "COMPLETADO".into();
        assert!(active_mission_context(&journal).is_none());
    }

    #[test]
    fn fallback_resolves_short_edits_and_answers_without_capturing_clear_new_tasks() {
        let context = "Aura: ¿Quieres que use Firebase? ";
        assert_eq!(
            fallback_turn_relation("Sí, esa opción", Some(context)),
            "FOLLOW_UP"
        );
        assert_eq!(
            fallback_turn_relation("hazlo más sencillo", Some(context)),
            "FOLLOW_UP"
        );
        assert_eq!(
            fallback_turn_relation("Quiero crear otra aplicación", Some(context)),
            "NEW_TASK"
        );
        assert_eq!(fallback_turn_relation("Sí", None), "NEW_TASK");
    }

    #[test]
    fn contextual_follow_up_keeps_original_and_corrected_user_instruction() {
        let mut journal = crate::core::session_journal::SessionJournal::default();
        journal.objetivo = "Construye una aplicación web de consultas".into();
        journal.status = "EN_PROGRESO".into();
        let context = active_mission_context(&journal).unwrap();
        let request = contextual_follow_up_request(
            &context,
            "ponlo mas elegante",
            "Actualizar el estilo visual de la aplicación web de consultas para que se vea más elegante.",
        ).unwrap();
        assert!(is_contextual_follow_up(&request));
        assert_eq!(
            contextual_follow_up_objective(&request),
            Some(journal.objetivo.as_str())
        );
        assert!(request.contains("ponlo mas elegante"));
        assert!(request.contains("más elegante."));
    }

    #[test]
    fn plan_change_updates_only_target_phase_and_reopens_it() {
        let mut journal = crate::core::session_journal::SessionJournal::default();
        journal.status = "EN_PROGRESO".into();
        journal.fase_actual = 1;
        journal.fases = vec![
            crate::core::session_journal::Fase {
                numero: 1,
                descripcion: "MVP".into(),
                archivos: vec!["index.html".into()],
                criterio_de_exito: "npm test".into(),
                estado: "COMPLETADA".into(),
            },
            crate::core::session_journal::Fase {
                numero: 2,
                descripcion: "Firebase".into(),
                archivos: vec!["firebase.json".into()],
                criterio_de_exito: "npm test".into(),
                estado: "EN_PROGRESO".into(),
            },
        ];
        let nlu = serde_json::json!({
            "phase_updates": [{
                "numero": 2,
                "descripcion": "Integrar autenticación y reglas de acceso",
                "archivos": ["firebase.json", "firestore.rules"],
                "criterio_de_exito": "npm test"
            }]
        });
        assert_eq!(apply_phase_plan_updates(&mut journal, &nlu), 1);
        assert_eq!(journal.fases[0].descripcion, "MVP");
        assert_eq!(journal.fases[0].estado, "COMPLETADA");
        assert_eq!(
            journal.fases[1].descripcion,
            "Integrar autenticación y reglas de acceso"
        );
        assert_eq!(journal.fases[1].estado, "PENDIENTE");
        assert_eq!(journal.fase_actual, 1);
        let unsafe_update = serde_json::json!({
            "phase_updates": [{
                "numero": 1,
                "descripcion": "No debe aplicar una ruta fuera del proyecto",
                "archivos": ["../secreto.txt"]
            }]
        });
        assert_eq!(apply_phase_plan_updates(&mut journal, &unsafe_update), 0);
        assert_eq!(journal.fases[0].descripcion, "MVP");
    }
}
