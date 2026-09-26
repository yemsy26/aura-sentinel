use crate::core::session_journal::Fase;
/// PESP v2 — Phase Planner
///
/// Called once at mission start (before the main agent loop) when the mission
/// type is Construction or Refactor. Generates a structured list of Fases
/// that the agent will execute sequentially and autonomously.
///
/// If Qwen fails to produce valid JSON, falls back to a single-phase plan
/// that is equivalent to the pre-PESP behavior.
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Raw JSON structure returned by Qwen's phase architect prompt
#[derive(Serialize, Deserialize, Debug, Default)]
struct PhasePlanRaw {
    #[serde(alias = "fases", alias = "phases")]
    fases: Vec<FaseRaw>,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
struct FaseRaw {
    #[serde(alias = "numero", alias = "number", default)]
    numero: u32,
    #[serde(alias = "descripcion", alias = "description", default)]
    descripcion: String,
    #[serde(alias = "archivos", alias = "files", default)]
    archivos: Vec<String>,
    #[serde(alias = "criterio_de_exito", alias = "success_criterion", default)]
    criterio_de_exito: String,
}

/// The prompt sent to Qwen to generate the phase plan.
fn build_architect_prompt(user_message: &str) -> String {
    format!(
        "Eres un arquitecto de software. Analiza la siguiente tarea y descomponla en fases secuenciales.\n\
        Tarea: {}\n\n\
        REGLAS:\n\
        1. RESPONDE ÚNICAMENTE CON JSON VÁLIDO. NADA más fuera del JSON.\n\
        2. Si la tarea es simple (1-2 archivos), crea solo 1 o 2 fases.\n\
        3. Máximo 4 fases. Cada fase debe producir algo verificable.\n\
        4. El criterio_de_exito debe ejecutar pruebas que verifiquen comportamientos solicitados. PROHIBIDO usar pruebas vacías como print('OK'), echo, compilar únicamente o comprobar solo que un archivo existe.\n\
        5. Para una aplicación web, cubre la interfaz real y las funciones pedidas; no inventes un dashboard ni sustituyas el producto por un script de consola.\n\
        6. Separa estructura, estilos, lógica y pruebas cuando el alcance lo justifique; elige los archivos según la solicitud, no según el ejemplo.\n\
        7. Respeta los nombres exactos que pidió el usuario y no agregues scripts de verificación salvo que el alcance los necesite.\n\
        8. Si se pide probar localmente y luego desplegar, las pruebas funcionales locales deben preceder al despliegue. No declares probado el producto solo porque un servidor arranque.\n\n\
        Formato exacto:\n\
        {{\"fases\": [{{\"numero\": 1, \"descripcion\": \"<fase ligada al producto solicitado>\", \"archivos\": [\"<archivos realmente necesarios>\"], \"criterio_de_exito\": \"<comando de pruebas funcionales reales>\"}}]}}",
        user_message
    )
}

fn phase_plan_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "fases": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "numero": { "type": "integer" },
                        "descripcion": { "type": "string" },
                        "archivos": { "type": "array", "items": { "type": "string" } },
                        "criterio_de_exito": { "type": "string" }
                    },
                    "required": ["numero", "descripcion", "archivos", "criterio_de_exito"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["fases"],
        "additionalProperties": false
    })
}

fn normalize_plan_text(text: &str) -> String {
    text.to_lowercase()
        .replace('á', "a")
        .replace('é', "e")
        .replace('í', "i")
        .replace('ó', "o")
        .replace('ú', "u")
        .replace('ñ', "n")
}

pub(crate) fn is_approved_consultation_web_task(user_message: &str) -> bool {
    let text = normalize_plan_text(user_message);
    let approved = text.contains("[modo implementacion de plan aprobado]")
        || text.contains("hoja de ruta que el usuario acaba de aprobar")
        || text.contains("implementa esta solicitud siguiendo la hoja de ruta aprobada")
        || (text.contains("plan inicial") && text.contains("procede con la fase 1"));
    let web_app = text.contains("aplicacion web")
        || text.contains("aplicacion tipo web")
        || text.contains("mvp web")
        || text.contains("aplicacion sera de consulta");
    let consultation = text.contains("consulta") || text.contains("agenda");
    let firebase = text.contains("firebase");
    let implementation_intent =
        approved || text.contains("constru") || text.contains("contru") || text.contains("mvp");
    let business_modules = text.contains("factura") && text.contains("contabilidad");
    implementation_intent && web_app && consultation && firebase && business_modules
}

/// A filename-only phase check can mistake leftover placeholder files for a
/// finished application. This small, deterministic audit is scoped to the
/// approved local consultation MVP and checks its real source and test harness
/// before Aura spends a step running the acceptance command.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ConsultationMvpAudit {
    pub issues: Vec<String>,
    pub repair_files: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ConsultationFirebaseAudit {
    pub issues: Vec<String>,
    pub repair_files: Vec<String>,
}

fn firebase_config_value(source: &str, key: &str) -> Option<String> {
    let pattern = regex::Regex::new(&format!(
        r#"(?i)\b{}\s*:\s*['\"]([^'\"]+)['\"]"#,
        regex::escape(key)
    ))
    .ok()?;
    pattern
        .captures(source)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Checks that Phase 2 contains a Firebase integration exercised against the
/// local Auth and Firestore emulators. A successful Phase 1 `npm test` is not
/// evidence for this phase because it only runs the localStorage test suite.
pub(crate) fn audit_consultation_firebase(workspace: &Path) -> ConsultationFirebaseAudit {
    let mut audit = ConsultationFirebaseAudit::default();
    let read =
        |relative: &str| std::fs::read_to_string(workspace.join(relative)).unwrap_or_default();
    let config = read("firebase-config.js");
    let app = read("app.js");
    let tests = read("tests/firebase-emulator.test.js");
    let package_source = read("package.json");
    let rules = read("firestore.rules");
    let firebase_json_source = read("firebase.json");

    let mut issue = |message: String, files: &[&str]| {
        audit.issues.push(message);
        for file in files {
            let path = file.to_string();
            if !audit.repair_files.contains(&path) {
                audit.repair_files.push(path);
            }
        }
    };

    let config_project = firebase_config_value(&config, "projectId");
    let config_project_is_placeholder = config_project.as_deref().map_or(true, |project| {
        let lower = project.to_ascii_lowercase();
        lower.contains("your-") || lower.contains("placeholder") || lower.contains("<")
    });
    if config_project_is_placeholder {
        issue(
            "firebase-config.js necesita un projectId de demostración real para conectar los emuladores; no uses YOUR_PROJECT_ID".into(),
            &["firebase-config.js"],
        );
    }
    if !config.contains("apiKey") || !config.contains("authDomain") || !config.contains("appId") {
        issue(
            "firebase-config.js debe exportar la configuración completa que consume initializeApp"
                .into(),
            &["firebase-config.js"],
        );
    }

    let imports_auth = regex::Regex::new(r#"(?s)from\s*['\"]firebase/auth['\"]"#)
        .unwrap()
        .is_match(&app);
    let imports_firestore = regex::Regex::new(r#"(?s)from\s*['\"]firebase/firestore['\"]"#)
        .unwrap()
        .is_match(&app);
    let imports_app_and_config = regex::Regex::new(
        r#"(?s)from\s*['\"]firebase/app['\"].{0,500}from\s*['\"][^'\"]*firebase-config\.js['\"]"#,
    )
    .unwrap()
    .is_match(&app);
    let firebase_calls = [
        "initializeApp(",
        "getAuth(",
        "getFirestore(",
        "createUserWithEmailAndPassword(",
        "signInWithEmailAndPassword(",
        "connectAuthEmulator(",
        "connectFirestoreEmulator(",
        "addDoc(",
        "getDocs(",
    ];
    let missing_calls: Vec<&str> = firebase_calls
        .iter()
        .copied()
        .filter(|call| !app.contains(call))
        .collect();
    if !imports_auth || !imports_firestore || !imports_app_and_config || !missing_calls.is_empty() {
        issue(
            format!(
                "app.js no integra Firebase Auth y Firestore de forma ejecutable; imports Auth={}, Firestore={}, app/config={}, llamadas faltantes={:?}",
                imports_auth, imports_firestore, imports_app_and_config, missing_calls
            ),
            &["app.js"],
        );
    }

    let package = serde_json::from_str::<serde_json::Value>(&package_source).ok();
    let firebase_test_script = package
        .as_ref()
        .and_then(|value| value.pointer("/scripts/test:firebase"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let dependencies = package.as_ref().and_then(|value| value.get("dependencies"));
    let dev_dependencies = package
        .as_ref()
        .and_then(|value| value.get("devDependencies"));
    let firebase_dependency = dependencies
        .and_then(|value| value.get("firebase"))
        .is_some();
    let firebase_tools_dependency = dev_dependencies
        .and_then(|value| value.get("firebase-tools"))
        .is_some();
    let runs_emulator_suite = firebase_test_script.contains("emulators:exec")
        && firebase_test_script.contains("tests/firebase-emulator.test.js")
        && config_project.as_deref().is_some_and(|project| {
            firebase_test_script.contains(&format!("--project {project}"))
                || firebase_test_script.contains(&format!("--project={project}"))
        });
    if !firebase_dependency || !firebase_tools_dependency || !runs_emulator_suite {
        issue(
            "package.json debe declarar firebase y firebase-tools, y `test:firebase` debe ejecutar tests/firebase-emulator.test.js dentro de firebase emulators:exec con el mismo proyecto demo".into(),
            &["package.json"],
        );
    }

    let test_case_count = regex::Regex::new(r#"\b(?:test|it)\s*\("#)
        .unwrap()
        .find_iter(&tests)
        .count();
    if !tests.contains("node:test")
        || test_case_count < 2
        || !tests.contains("connectAuthEmulator")
        || !tests.contains("connectFirestoreEmulator")
        || !tests.contains("registerUser(")
        || !tests.contains("createClient(")
        || !tests.contains("assert.")
    {
        issue(
            format!(
                "tests/firebase-emulator.test.js debe ejecutar al menos 2 pruebas con node:test, conectar Auth/Firestore a emuladores y comprobar registro y escritura de un cliente (casos encontrados: {test_case_count})"
            ),
            &["tests/firebase-emulator.test.js"],
        );
    }

    let open_rules = rules
        .to_ascii_lowercase()
        .contains("allow read, write: if true")
        || rules.to_ascii_lowercase().contains("allow read: if true")
        || rules.to_ascii_lowercase().contains("allow write: if true");
    if !rules.contains("request.auth.uid") || open_rules {
        issue(
            "firestore.rules debe limitar las operaciones al usuario autenticado y nunca permitir lectura/escritura pública".into(),
            &["firestore.rules"],
        );
    }

    let firebase_json = serde_json::from_str::<serde_json::Value>(&firebase_json_source).ok();
    let rules_path = firebase_json
        .as_ref()
        .and_then(|value| value.pointer("/firestore/rules"))
        .and_then(serde_json::Value::as_str);
    let indexes_path = firebase_json
        .as_ref()
        .and_then(|value| value.pointer("/firestore/indexes"))
        .and_then(serde_json::Value::as_str);
    if rules_path != Some("firestore.rules") || indexes_path != Some("firestore.indexes.json") {
        issue(
            "firebase.json debe enlazar firestore.rules y firestore.indexes.json mediante rutas a archivos separados".into(),
            &["firebase.json"],
        );
    }

    audit.repair_files.sort();
    audit
}

/// Prevents deploys from inheriting an unrelated global Firebase project and
/// checks the static Hosting payload before invoking the Firebase CLI.
pub(crate) fn firebase_deploy_preflight(workspace: &Path, command: &str) -> Result<(), String> {
    let lower = command.to_ascii_lowercase();
    let invokes_deploy =
        regex::Regex::new(r#"(?i)(?:^|\s)(?:npx\s+)?firebase(?:\.cmd)?\s+deploy(?:\s|$)"#)
            .unwrap()
            .is_match(command)
            || (lower.contains("firebase") && lower.contains("deploy"));
    if !invokes_deploy {
        return Ok(());
    }

    let firebaserc_source = std::fs::read_to_string(workspace.join(".firebaserc"))
        .map_err(|_| "FIREBASE_DEPLOY_BLOCKED: falta .firebaserc con un proyecto elegido explícitamente en este workspace.".to_string())?;
    let firebaserc = serde_json::from_str::<serde_json::Value>(&firebaserc_source)
        .map_err(|_| "FIREBASE_DEPLOY_BLOCKED: .firebaserc no contiene JSON válido.".to_string())?;
    let default_project = firebaserc
        .pointer("/projects/default")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let explicit_project = regex::Regex::new(r#"(?i)--project(?:=|\s+)([a-z0-9-]+)"#)
        .unwrap()
        .captures(command)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str());
    let project = explicit_project.unwrap_or(default_project);
    let valid_project = regex::Regex::new(r"^[a-z][a-z0-9-]{4,28}[a-z0-9]$")
        .unwrap()
        .is_match(project);
    let lower_project = project.to_ascii_lowercase();
    if !valid_project
        || lower_project.starts_with("demo-")
        || lower_project.contains("your-")
        || lower_project.contains("placeholder")
    {
        return Err("FIREBASE_DEPLOY_BLOCKED: falta un projectId Firebase explícito y válido (6–30 letras minúsculas, números o guiones) en .firebaserc o en --project. Aura no usará el proyecto global de Firebase.".into());
    }
    let config = std::fs::read_to_string(workspace.join("firebase-config.js"))
        .map_err(|_| "FIREBASE_DEPLOY_BLOCKED: falta firebase-config.js con la configuración web del proyecto.".to_string())?;
    let config_project = firebase_config_value(&config, "projectId").unwrap_or_default();
    if config_project != project {
        return Err(format!(
            "FIREBASE_DEPLOY_BLOCKED: firebase-config.js usa projectId '{config_project}', pero el destino explícito es '{project}'. Completa la configuración web del mismo proyecto antes de desplegar."
        ));
    }
    let lower_config = config.to_ascii_lowercase();
    if ["your_", "your-", "placeholder", "replace_me", "<your"]
        .iter()
        .any(|marker| lower_config.contains(marker))
    {
        return Err("FIREBASE_DEPLOY_BLOCKED: firebase-config.js todavía contiene valores de ejemplo/placeholder; completa la configuración de la app web Firebase antes del despliegue.".into());
    }

    for key in ["apiKey", "authDomain", "appId"] {
        if firebase_config_value(&config, key).is_none() {
            return Err(format!(
                "FIREBASE_DEPLOY_BLOCKED: firebase-config.js no contiene un valor para '{key}'. Completa la configuración web antes del despliegue."
            ));
        }
    }

    let firebase_json_source = std::fs::read_to_string(workspace.join("firebase.json"))
        .map_err(|_| "FIREBASE_DEPLOY_BLOCKED: falta firebase.json.".to_string())?;
    let firebase_json =
        serde_json::from_str::<serde_json::Value>(&firebase_json_source).map_err(|_| {
            "FIREBASE_DEPLOY_BLOCKED: firebase.json no contiene JSON válido.".to_string()
        })?;
    let hosting = firebase_json
        .get("hosting")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "FIREBASE_DEPLOY_BLOCKED: firebase.json no define hosting.".to_string())?;
    let public_dir = hosting
        .get("public")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(".");
    let ignore = hosting
        .get("ignore")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "FIREBASE_DEPLOY_BLOCKED: hosting.ignore debe enumerar archivos internos que no se publican.".to_string())?;
    let has_ignore = |pattern: &str| ignore.iter().any(|item| item.as_str() == Some(pattern));
    let mut required_ignores = vec![
        "**/*.bat", "**/*.cmd", "**/*.exe", "**/*.dll", "**/*.apk", "**/*.ipa",
    ];
    if public_dir == "." {
        required_ignores.extend([".aura/**", ".firebase/**"]);
    }
    {
        let missing: Vec<&str> = required_ignores
            .iter()
            .copied()
            .filter(|pattern| !has_ignore(pattern))
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "FIREBASE_DEPLOY_BLOCKED: hosting.ignore no excluye todos los recursos internos/ejecutables; agrega estos patrones: {:?}.",
                missing
            ));
        }
    }
    Ok(())
}

pub(crate) fn audit_consultation_mvp(workspace: &Path) -> ConsultationMvpAudit {
    let mut audit = ConsultationMvpAudit::default();
    let app_path = workspace.join("app.js");
    let tests_path = workspace.join("tests").join("app.test.js");
    let html_path = workspace.join("index.html");

    let app = std::fs::read_to_string(&app_path).unwrap_or_default();
    let comment_pattern = regex::Regex::new(r"(?s)/\*.*?\*/|//[^\r\n]*").unwrap();
    let executable_app = comment_pattern.replace_all(&app, " ");
    if app.trim().is_empty() {
        audit.issues.push("app.js está vacío o no existe".into());
        audit.repair_files.push("app.js".into());
    } else {
        let stub_report = crate::core::stub_enforcer::detect_stubs(&app, "app.js");
        if stub_report.has_stubs {
            audit.issues.push(format!(
                "app.js contiene una implementación incompleta: {}",
                stub_report.warnings.join("; ")
            ));
            audit.repair_files.push("app.js".into());
        }

        // Verify the documented public API rather than searching for word
        // combinations. The previous heuristic rejected valid implementations
        // when names or data flow differed, sending the model into blind rewrites.
        let required_exports = [
            ("registerUser", "registro"),
            ("loginUser", "inicio de sesión"),
            ("createClient", "gestión de clientes"),
            ("createAppointment", "agenda de consultas o citas"),
            ("createInvoice", "facturación"),
            ("addAccountingTransaction", "contabilidad"),
        ];
        for (name, label) in required_exports {
            let export_pattern = regex::Regex::new(&format!(
                r"(?m)\bexport\s+(?:(?:async\s+)?function|const|let|var)\s+{}\b",
                regex::escape(name)
            ))
            .unwrap();
            if !export_pattern.is_match(&app) {
                audit.issues.push(format!(
                    "app.js no exporta la operación requerida `{name}` ({label})"
                ));
                if !audit.repair_files.iter().any(|file| file == "app.js") {
                    audit.repair_files.push("app.js".into());
                }
            }
            let use_pattern =
                regex::Regex::new(&format!(r"\b{}\s*\(", regex::escape(name))).unwrap();
            if use_pattern.find_iter(&executable_app).count() < 2 {
                audit.issues.push(format!(
                    "app.js exporta `{name}` ({label}), pero no la conecta con una acción de la interfaz"
                ));
                if !audit.repair_files.iter().any(|file| file == "app.js") {
                    audit.repair_files.push("app.js".into());
                }
            }
        }
        let lower_app = app.to_ascii_lowercase();
        if !lower_app.contains("localstorage")
            || !lower_app.contains("getitem")
            || !lower_app.contains("setitem")
        {
            audit.issues.push(
                "app.js debe leer y guardar el estado local mediante localStorage (getItem/setItem)".into(),
            );
            if !audit.repair_files.iter().any(|file| file == "app.js") {
                audit.repair_files.push("app.js".into());
            }
        }
        let has_dom_submit_binding = lower_app.contains("addeventlistener")
            && lower_app.contains("submit")
            && (lower_app.contains("getelementbyid") || lower_app.contains("queryselector"));
        if !has_dom_submit_binding {
            audit.issues.push(
                "app.js no conecta los formularios HTML con sus operaciones mediante manejadores submit".into(),
            );
            if !audit.repair_files.iter().any(|file| file == "app.js") {
                audit.repair_files.push("app.js".into());
            }
        }
        let renders_feedback = [
            "innerhtml",
            "innertext",
            "textcontent",
            "insertadjacenthtml",
            "appendchild",
            "replacechildren",
        ]
        .iter()
        .any(|marker| lower_app.contains(marker));
        if !renders_feedback {
            audit.issues.push(
                "app.js no muestra en la interfaz los resultados de las operaciones ni sus estados"
                    .into(),
            );
            if !audit.repair_files.iter().any(|file| file == "app.js") {
                audit.repair_files.push("app.js".into());
            }
        }
    }

    let tests = std::fs::read_to_string(&tests_path).unwrap_or_default();
    let comment_pattern = regex::Regex::new(r"(?s)/\*.*?\*/|//[^\r\n]*").unwrap();
    let executable_tests = comment_pattern.replace_all(&tests, " ");
    let test_cases = regex::Regex::new(r"\b(?:test|it)\s*\(")
        .unwrap()
        .find_iter(&executable_tests)
        .count();
    if !executable_tests.contains("node:test") {
        audit
            .issues
            .push("tests/app.test.js no usa el runner node:test".into());
        audit.repair_files.push("tests/app.test.js".into());
    }
    if test_cases < 6 {
        audit.issues.push(format!(
            "tests/app.test.js solo registra {test_cases} casos; se requieren casos ejecutables para acceso, clientes, consultas, facturación y contabilidad"
        ));
        audit.repair_files.push("tests/app.test.js".into());
    }
    let assertions = regex::Regex::new(r"\bassert\s*\.|\bexpect\s*\([^\n]*\)\s*\.")
        .unwrap()
        .find_iter(&executable_tests)
        .count();
    if assertions < 6 {
        audit.issues.push(format!(
            "tests/app.test.js contiene {assertions} aserciones ejecutables; cada flujo necesita comprobar resultados reales"
        ));
        audit.repair_files.push("tests/app.test.js".into());
    }
    let imports_app = executable_tests.contains("app.js")
        && regex::Regex::new(r#"(?s)\bimport\b.{0,500}?['"][^'"]*app\.js['"]"#)
            .unwrap()
            .is_match(&executable_tests);
    if !imports_app {
        audit
            .issues
            .push("tests/app.test.js no importa la lógica real de app.js".into());
        audit.repair_files.push("tests/app.test.js".into());
    }
    let required_test_calls = [
        ("registerUser", "registro"),
        ("loginUser", "inicio de sesión"),
        ("createClient", "clientes"),
        ("createAppointment", "consultas o agenda"),
        ("createInvoice", "facturación"),
        ("addAccountingTransaction", "contabilidad"),
    ];
    for (name, label) in required_test_calls {
        let invoked = regex::Regex::new(&format!(r"\b{}\s*\(", regex::escape(name)))
            .unwrap()
            .is_match(&executable_tests);
        if !invoked {
            audit.issues.push(format!(
                "tests/app.test.js no invoca la operación `{name}` para probar {label}"
            ));
            if !audit
                .repair_files
                .iter()
                .any(|file| file == "tests/app.test.js")
            {
                audit.repair_files.push("tests/app.test.js".into());
            }
        }
    }

    let package_path = workspace.join("package.json");
    let package_source = std::fs::read_to_string(&package_path).unwrap_or_default();
    let package = serde_json::from_str::<serde_json::Value>(&package_source).ok();
    let is_module_package = package
        .as_ref()
        .and_then(|value| value.get("type"))
        .and_then(serde_json::Value::as_str)
        == Some("module");
    let has_node_test_script = package
        .as_ref()
        .and_then(|value| value.pointer("/scripts/test"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|script| script.trim() == "node --test tests/app.test.js");
    if !is_module_package || !has_node_test_script {
        audit.issues.push(
            "package.json debe declarar `type: module` y `test: node --test tests/app.test.js`"
                .into(),
        );
        audit.repair_files.push("package.json".into());
    }

    let html = std::fs::read_to_string(html_path).unwrap_or_default();
    let document = scraper::Html::parse_document(&html);
    for (form_id, label) in [
        ("login-form", "inicio de sesión"),
        ("register-form", "registro"),
        ("client-form", "gestión de clientes"),
        ("appointment-form", "agenda de consultas o citas"),
        ("invoice-form", "facturación"),
        ("account-form", "movimientos de contabilidad"),
    ] {
        let selector = scraper::Selector::parse(&format!("form#{form_id}"))
            .expect("fixed form id selector is valid");
        if !document.select(&selector).next().is_some() {
            audit.issues.push(format!(
                "index.html no ofrece un formulario para {label}; agrega `<form id=\"{form_id}\">`"
            ));
            if !audit.repair_files.iter().any(|file| file == "index.html") {
                audit.repair_files.push("index.html".into());
            }
        }
    }
    let module_scripts = scraper::Selector::parse("script[type='module'][src]").unwrap();
    let stylesheet_links = scraper::Selector::parse("link[rel='stylesheet'][href]").unwrap();
    let connects_module = document.select(&module_scripts).any(|element| {
        element.value().attr("src").is_some_and(|src| {
            src.rsplit('/')
                .next()
                .unwrap_or(src)
                .eq_ignore_ascii_case("app.js")
        })
    });
    let connects_styles = document.select(&stylesheet_links).any(|element| {
        element.value().attr("href").is_some_and(|href| {
            href.rsplit('/')
                .next()
                .unwrap_or(href)
                .eq_ignore_ascii_case("styles.css")
        })
    });
    if !connects_module || !connects_styles {
        audit.issues.push(
            "index.html debe enlazar styles.css y cargar app.js con `<script type=\"module\">`"
                .into(),
        );
        audit.repair_files.push("index.html".into());
    }

    audit.repair_files.sort();
    audit.repair_files.dedup();
    audit
}

fn approved_consultation_web_plan(user_message: &str) -> Option<Vec<Fase>> {
    if !is_approved_consultation_web_task(user_message) {
        return None;
    }

    Some(vec![
        Fase {
            numero: 1,
            descripcion: "Construir el MVP web real de consultas con login y registro, gestión de clientes, agenda, facturación y contabilidad; ejecutarlo y probar sus flujos localmente antes de publicar".into(),
            archivos: vec![
                "index.html".into(), "styles.css".into(), "app.js".into(),
                "package.json".into(), "tests/app.test.js".into(),
            ],
            criterio_de_exito: "npm test".into(),
            estado: "PENDIENTE".into(),
        },
        Fase {
            numero: 2,
            descripcion: "Integrar Firebase Authentication y Firestore con reglas de seguridad y emuladores; repetir las pruebas locales antes de cualquier publicación".into(),
            archivos: vec![
                "firebase-config.js".into(), "firebase.json".into(),
                "app.js".into(), "package.json".into(),
                "firestore.rules".into(), "firestore.indexes.json".into(),
                "tests/firebase-emulator.test.js".into(),
            ],
            criterio_de_exito: "npm run test:firebase".into(),
            estado: "PENDIENTE".into(),
        },
        Fase {
            numero: 3,
            descripcion: "Desplegar el MVP ya probado en Firebase Hosting y ejecutar pruebas de humo del sitio publicado; detenerse si falta el proyecto o la autenticación de Firebase".into(),
            archivos: vec![".firebaserc".into(), "firebase.json".into()],
            criterio_de_exito: "firebase deploy --only hosting".into(),
            estado: "PENDIENTE".into(),
        },
    ])
}

pub(crate) fn is_local_consultation_agenda_task(user_message: &str) -> bool {
    let text = normalize_plan_text(user_message);
    let consultation_agenda =
        text.contains("consulta") && (text.contains("cita") || text.contains("agenda"));
    let local_web_prototype = text.contains("local")
        && (text.contains("html") || text.contains("javascript"))
        && (text.contains("sin dependencias") || text.contains("no instales"));
    let demo_scope =
        text.contains("demostracion") || text.contains("demo") || text.contains("prueba local");
    consultation_agenda && local_web_prototype && demo_scope
}

fn local_consultation_agenda_plan(user_message: &str) -> Option<Vec<Fase>> {
    if !is_local_consultation_agenda_task(user_message) {
        return None;
    }

    Some(vec![Fase {
        numero: 1,
        descripcion: "Construir y probar Consulta Clara en una sola fase local: interfaz profesional en español, adaptable a escritorio y móvil; registro, inicio y cierre de sesión de demostración claramente no reales, sin pedir credenciales reales ni guardar contraseñas; gestión de citas con cliente de prueba, fecha, hora, motivo y estado, incluyendo crear, editar, cancelar y completar; búsqueda, filtro por estado y estado vacío; persistencia de citas de demostración en localStorage y control visible para restablecerlas; formularios etiquetados, campos obligatorios validados, errores comprensibles, navegación por teclado y buen contraste. Inspeccionar los archivos antes de escribir y modificar solo lo necesario. Iniciar un servidor HTTP local sin instalar paquetes; probar en navegador los flujos de sesión, citas, validación, búsqueda, estados y persistencia tras recargar; revisar consola y vista de escritorio/móvil, corregir defectos una vez y reportar URL, pruebas ejecutadas y pendientes. No desplegar ni usar Firebase.".into(),
        archivos: vec!["index.html".into(), "styles.css".into(), "script.js".into()],
        criterio_de_exito: "Servidor local con URL real; flujos principales demostrados en navegador; persistencia confirmada tras recarga; consola revisada; vista de escritorio y móvil evaluada; defectos corregidos o pendientes descritos sin declarar éxito no probado.".into(),
        estado: "PENDIENTE".into(),
    }])
}

pub(crate) fn local_consultation_agenda_plan_is_complete(phases: &[Fase]) -> bool {
    let Some(phase) = phases.first() else {
        return false;
    };
    let coverage = normalize_plan_text(&format!(
        "{} {}",
        phase.descripcion, phase.criterio_de_exito
    ));
    [
        "interfaz profesional en espanol",
        "inicio de sesion",
        "registro",
        "cierre de sesion",
        "sin pedir credenciales reales",
        "guardar contrasenas",
        "cliente de prueba",
        "fecha",
        "hora",
        "motivo",
        "estado",
        "crear",
        "editar",
        "cancelar",
        "completar",
        "busqueda",
        "filtro por estado",
        "estado vacio",
        "localstorage",
        "restablecer",
        "formularios etiquetados",
        "campos obligatorios",
        "errores comprensibles",
        "navegacion por teclado",
        "buen contraste",
        "servidor http local",
        "navegador",
        "persistencia",
        "escritorio y movil",
        "vista de escritorio/movil",
        "consola",
    ]
    .iter()
    .all(|requirement| coverage.contains(requirement))
}

/// Generate a fallback single-phase plan when Qwen fails.
/// This makes the system behave exactly like before PESP v2.
fn single_phase_fallback(user_message: &str) -> Vec<Fase> {
    vec![Fase {
        numero: 1,
        descripcion: format!(
            "Ejecutar tarea completa: {}",
            user_message.chars().take(60).collect::<String>()
        ),
        archivos: vec![],
        criterio_de_exito: String::new(),
        estado: "PENDIENTE".to_string(),
    }]
}

/// Strip markdown code fences and <think> tags from Qwen's response
fn clean_response(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    // Remove <think>...</think> blocks
    while let (Some(start), Some(end)) = (s.find("<think>"), s.find("</think>")) {
        if start < end {
            s = format!("{}{}", &s[..start], &s[end + 8..]);
        } else {
            break;
        }
    }
    crate::core::structured_json::parse_json_object(&s)
        .and_then(|value| serde_json::to_string(&value).map_err(|error| error.to_string()))
        .unwrap_or_else(|_| s.trim().to_string())
}

/// Main entry point. Calls Qwen to generate a phase plan.
/// Returns a Vec<Fase> ready to store in SessionJournal.
/// Never panics — always returns at least 1 phase.
pub async fn generate_phase_plan(user_message: &str, model: &str) -> Vec<Fase> {
    if let Some(phases) = approved_consultation_web_plan(user_message) {
        return phases;
    }
    if let Some(phases) = local_consultation_agenda_plan(user_message) {
        return phases;
    }

    // Small, explicitly named deliverables do not need several speculative LLM phases.
    let contract = crate::core::mission_contract::MissionContract::from_objective(user_message);
    let files: Vec<String> = contract
        .acceptance_criteria
        .iter()
        .filter_map(|criterion| {
            if let crate::core::mission_contract::VerificationMethod::FileExistence(file) =
                &criterion.verification
            {
                Some(file.clone())
            } else {
                None
            }
        })
        .collect();
    if !files.is_empty() && files.len() <= 3 {
        let verifier = contract
            .acceptance_criteria
            .iter()
            .find_map(|criterion| {
                if let crate::core::mission_contract::VerificationMethod::SemanticVerification {
                    command,
                } = &criterion.verification
                {
                    Some(command.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default();
        return vec![Fase {
            numero: 1,
            descripcion: "Implementar los entregables solicitados y verificar su funcionamiento"
                .into(),
            archivos: files,
            criterio_de_exito: verifier,
            estado: "PENDIENTE".into(),
        }];
    }

    let prompt = build_architect_prompt(user_message);

    // Call Qwen using the same infrastructure as the programmer
    let result = crate::llm::call_ollama_with_schema_options(
        model,
        &prompt,
        phase_plan_schema(),
        2048,
        0.05,
    )
    .await;

    match result {
        Err(_) => {
            // Network/model error — fall back silently
            return single_phase_fallback(user_message);
        }
        Ok(raw) => {
            let cleaned = clean_response(&raw);
            match serde_json::from_str::<PhasePlanRaw>(&cleaned) {
                Err(_) => {
                    // JSON parse error — fall back silently
                    single_phase_fallback(user_message)
                }
                Ok(plan) if plan.fases.is_empty() => single_phase_fallback(user_message),
                Ok(plan) => plan
                    .fases
                    .into_iter()
                    .enumerate()
                    .map(|(i, f)| Fase {
                        numero: if f.numero == 0 {
                            (i + 1) as u32
                        } else {
                            f.numero
                        },
                        descripcion: if f.descripcion.is_empty() {
                            format!("Fase {}", i + 1)
                        } else {
                            f.descripcion
                        },
                        archivos: f.archivos,
                        criterio_de_exito: f.criterio_de_exito,
                        estado: "PENDIENTE".to_string(),
                    })
                    .collect(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    fn temp_workspace() -> PathBuf {
        std::env::temp_dir().join(format!("aura-consultation-audit-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn approved_web_consultation_plan_uses_product_phases_and_real_tests() {
        let request = "[MODO IMPLEMENTACION DE PLAN APROBADO] construir una aplicación web de consultas con login, registro, facturación, contabilidad y despliegue en Firebase";
        let phases = super::approved_consultation_web_plan(request).unwrap();
        assert_eq!(phases.len(), 3);
        assert!(phases[0].archivos.contains(&"index.html".into()));
        assert!(phases[0].archivos.contains(&"tests/app.test.js".into()));
        assert_eq!(phases[0].criterio_de_exito, "npm test");
        assert!(phases[0].descripcion.contains("facturación"));
        assert!(phases[1].descripcion.contains("emuladores"));
        assert_eq!(phases[1].criterio_de_exito, "npm run test:firebase");
        assert!(phases[1].archivos.contains(&"app.js".into()));
        assert!(phases[1].archivos.contains(&"package.json".into()));
        assert!(phases[2].descripcion.contains("Firebase Hosting"));
    }

    #[test]
    fn approved_plan_survives_resume_context_and_misspellings() {
        let request = "[MODO IMPLEMENTACION DE PLAN APROBADO] Solicitud original: vamos a contruir una aplicacion tipo web probamos local y luego desplegamos en firebase la aplicacion sera de consulta. Hoja de ruta que el usuario acaba de aprobar: Objetivo: construir una aplicación web de gestión de consultas, facturación y contabilidad. Instrucción actual: implementa esta solicitud siguiendo la hoja de ruta aprobada. Para el MVP web usa HTML, CSS y JavaScript; no introduzcas Python/Flask.";
        assert!(super::is_approved_consultation_web_task(request));
        let phases = super::approved_consultation_web_plan(request).unwrap();
        assert_eq!(
            phases[0].archivos,
            vec![
                "index.html",
                "styles.css",
                "app.js",
                "package.json",
                "tests/app.test.js"
            ]
        );
        assert_eq!(phases[0].criterio_de_exito, "npm test");
    }

    #[test]
    fn local_consultation_agenda_plan_preserves_all_requested_flows() {
        let request = "Construye y prueba en este workspace una aplicación web local llamada Consulta Clara, una agenda profesional para gestionar consultas y citas. Es una prueba local, de una sola fase. Usa HTML, CSS y JavaScript sin frameworks ni dependencias externas. No despliegues, no uses Firebase, no instales paquetes. Registro de demostración no real, CRUD de citas, búsqueda, filtros, localStorage, accesibilidad y pruebas reales en navegador de escritorio y móvil.";
        assert!(super::is_local_consultation_agenda_task(request));
        assert!(!super::is_approved_consultation_web_task(request));
        let phases = super::local_consultation_agenda_plan(request).unwrap();
        assert_eq!(phases.len(), 1);
        assert_eq!(
            phases[0].archivos,
            vec!["index.html", "styles.css", "script.js"]
        );
        assert!(super::local_consultation_agenda_plan_is_complete(&phases));
        assert!(phases[0]
            .descripcion
            .contains("crear, editar, cancelar y completar"));
        assert!(phases[0].descripcion.contains("persistencia de citas"));
        assert!(phases[0]
            .descripcion
            .contains("No desplegar ni usar Firebase"));
    }

    #[test]
    fn previous_login_only_phase_is_rejected_as_incomplete_for_local_agenda() {
        let shallow = vec![super::Fase {
            numero: 1,
            descripcion: "Login y registro, formularios etiquetados, validación y servidor local"
                .into(),
            archivos: vec!["index.html".into(), "styles.css".into(), "script.js".into()],
            criterio_de_exito: "Abrir en navegador".into(),
            estado: "EN_PROGRESO".into(),
        }];
        assert!(!super::local_consultation_agenda_plan_is_complete(&shallow));
    }

    #[test]
    fn consultation_audit_rejects_existing_stub_and_a_test_file_with_no_cases() {
        let root = temp_workspace();
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::write(
            root.join("index.html"),
            "<link rel=\"stylesheet\" href=\"styles.css\"><script src=\"app.js\"></script>",
        )
        .unwrap();
        std::fs::write(root.join("styles.css"), "body { color: black; }").unwrap();
        std::fs::write(
            root.join("app.js"),
            "function register(username, password) { return true; }",
        )
        .unwrap();
        std::fs::write(
            root.join("tests/app.test.js"),
            "const { test } = require('node:test'); async function testLogin() {};",
        )
        .unwrap();
        std::fs::write(root.join("package.json"), "{}").unwrap();

        let audit = super::audit_consultation_mvp(&root);
        assert!(audit
            .issues
            .iter()
            .any(|issue| issue.contains("implementación incompleta")));
        assert!(audit
            .issues
            .iter()
            .any(|issue| issue.contains("registra 0 casos")));
        assert!(audit.repair_files.contains(&"app.js".into()));
        assert!(audit.repair_files.contains(&"tests/app.test.js".into()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn consultation_audit_accepts_real_module_wiring_and_behavior_tests() {
        let root = temp_workspace();
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::write(root.join("index.html"), "<link rel=\"stylesheet\" href=\"styles.css\"><script type=\"module\" src=\"app.js\"></script><form id=\"login-form\"></form><form id=\"register-form\"></form><form id=\"client-form\"></form><form id=\"appointment-form\"></form><form id=\"invoice-form\"></form><form id=\"account-form\"></form>").unwrap();
        std::fs::write(root.join("styles.css"), "body { color: black; }").unwrap();
        std::fs::write(root.join("app.js"), "export function registerUser() { return { saved: true }; } export function loginUser() { return { user: true }; } export function createClient() { return { client: true }; } export function createAppointment() { return { appointment: true }; } export function createInvoice() { return { invoice: true }; } export function addAccountingTransaction() { return { transaction: true }; } export const localStorageAdapter = { getItem() {}, setItem() {} }; const storageName = 'localStorage'; const submitEvent = 'submit'; function bindForms() { document.getElementById('login-form').addEventListener(submitEvent, () => loginUser()); document.getElementById('register-form').addEventListener(submitEvent, () => registerUser()); document.getElementById('client-form').addEventListener(submitEvent, () => createClient()); document.getElementById('appointment-form').addEventListener(submitEvent, () => createAppointment()); document.getElementById('invoice-form').addEventListener(submitEvent, () => createInvoice()); document.getElementById('account-form').addEventListener(submitEvent, () => addAccountingTransaction()); document.body.innerHTML = 'rendered'; }").unwrap();
        std::fs::write(root.join("tests/app.test.js"), "import { test } from 'node:test'; import assert from 'node:assert/strict'; import { registerUser, loginUser, createClient, createAppointment, createInvoice, addAccountingTransaction } from '../app.js'; test('registration', () => assert.ok(registerUser())); test('login', () => assert.ok(loginUser())); test('clients', () => assert.ok(createClient())); test('appointments', () => assert.ok(createAppointment())); test('invoices', () => assert.ok(createInvoice())); test('accounting', () => assert.ok(addAccountingTransaction()));").unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"type":"module","scripts":{"test":"node --test tests/app.test.js"}}"#,
        )
        .unwrap();

        let audit = super::audit_consultation_mvp(&root);
        assert!(
            audit.issues.is_empty(),
            "unexpected audit issues: {:?}",
            audit.issues
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn consultation_audit_rejects_exported_services_without_a_wired_web_interface() {
        let root = temp_workspace();
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::write(
            root.join("index.html"),
            "<link rel=\"stylesheet\" href=\"styles.css\"><script type=\"module\" src=\"app.js\"></script><form id=\"login-form\"></form><form id=\"register-form\"></form><form id=\"account-form\"></form>",
        )
        .unwrap();
        std::fs::write(root.join("styles.css"), "body { color: black; }").unwrap();
        std::fs::write(
            root.join("app.js"),
            "export function registerUser() {} export function loginUser() {} export function createClient() {} export function createAppointment() {} export function createInvoice() {} export function addAccountingTransaction() {} const localStorageAdapter = { getItem() {}, setItem() {} };",
        )
        .unwrap();
        std::fs::write(
            root.join("tests/app.test.js"),
            "import { test } from 'node:test'; import assert from 'node:assert/strict'; import { registerUser, loginUser, createClient, createAppointment, createInvoice, addAccountingTransaction } from '../app.js'; test('registration', () => assert.ok(registerUser())); test('login', () => assert.ok(loginUser())); test('clients', () => assert.ok(createClient())); test('appointments', () => assert.ok(createAppointment())); test('invoices', () => assert.ok(createInvoice())); test('accounting', () => assert.ok(addAccountingTransaction()));",
        )
        .unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"type":"module","scripts":{"test":"node --test tests/app.test.js"}}"#,
        )
        .unwrap();

        let audit = super::audit_consultation_mvp(&root);
        assert!(audit
            .issues
            .iter()
            .any(|issue| issue.contains("manejadores submit")));
        assert!(audit
            .issues
            .iter()
            .any(|issue| issue.contains("no la conecta con una acción de la interfaz")));
        assert!(audit
            .issues
            .iter()
            .any(|issue| issue.contains("appointment-form")));
        assert!(audit.repair_files.contains(&"app.js".into()));
        assert!(audit.repair_files.contains(&"index.html".into()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn architect_prompt_no_longer_suggests_noop_success_checks() {
        let prompt = super::build_architect_prompt("Construye una aplicación web");
        assert!(!prompt.contains("import os; print('OK')"));
        assert!(prompt.contains("pruebas vacías"));
        assert!(prompt.contains("comportamientos solicitados"));
    }

    #[test]
    fn firebase_phase_audit_requires_integrated_services_and_emulator_tests() {
        let root = temp_workspace();
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::write(root.join("firebase-config.js"), r#"export default { apiKey: "demo-key", authDomain: "demo-aura.firebaseapp.com", projectId: "demo-aura", appId: "demo-app" };"#).unwrap();
        std::fs::write(root.join("app.js"), r#"
            import { initializeApp } from 'firebase/app';
            import config from './firebase-config.js';
            import { getAuth, createUserWithEmailAndPassword, signInWithEmailAndPassword, connectAuthEmulator } from 'firebase/auth';
            import { getFirestore, connectFirestoreEmulator, addDoc, getDocs } from 'firebase/firestore';
            initializeApp(config); getAuth(); getFirestore(); createUserWithEmailAndPassword(); signInWithEmailAndPassword();
            connectAuthEmulator(); connectFirestoreEmulator(); addDoc(); getDocs();
        "#).unwrap();
        std::fs::write(root.join("package.json"), r#"{"scripts":{"test:firebase":"firebase emulators:exec --only auth,firestore --project demo-aura \"node --test tests/firebase-emulator.test.js\""},"dependencies":{"firebase":"12"},"devDependencies":{"firebase-tools":"14"}}"#).unwrap();
        std::fs::write(root.join("tests/firebase-emulator.test.js"), r#"import { test } from 'node:test'; import assert from 'node:assert/strict'; import { connectAuthEmulator, connectFirestoreEmulator } from '../app.js'; import { registerUser, createClient } from '../app.js'; test('auth emulator', async () => { connectAuthEmulator(); assert.ok(await registerUser()); }); test('firestore emulator', async () => { connectFirestoreEmulator(); assert.ok(await createClient()); });"#).unwrap();
        std::fs::write(root.join("firestore.rules"), "match /users/{userId} { allow read, write: if request.auth != null && request.auth.uid == userId; }").unwrap();
        std::fs::write(
            root.join("firebase.json"),
            r#"{"firestore":{"rules":"firestore.rules","indexes":"firestore.indexes.json"}}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("firestore.indexes.json"),
            r#"{"indexes":[],"fieldOverrides":[]}"#,
        )
        .unwrap();

        let audit = super::audit_consultation_firebase(&root);
        assert!(
            audit.issues.is_empty(),
            "unexpected Firebase audit issues: {:?}",
            audit.issues
        );

        std::fs::write(
            root.join("tests/firebase-emulator.test.js"),
            "import { test } from 'node:test'; test('noop', () => {});",
        )
        .unwrap();
        let incomplete = super::audit_consultation_firebase(&root);
        assert!(incomplete
            .issues
            .iter()
            .any(|issue| issue.contains("firebase-emulator.test.js")));
        assert!(incomplete
            .repair_files
            .contains(&"tests/firebase-emulator.test.js".into()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn firebase_deploy_preflight_blocks_ambient_or_mismatched_project_and_internal_payloads() {
        let root = temp_workspace();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join(".firebaserc"),
            r#"{"projects":{"default":"your-firebase-project-id"}}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("firebase-config.js"),
            r#"export default { projectId: "YOUR_PROJECT_ID" };"#,
        )
        .unwrap();
        std::fs::write(
            root.join("firebase.json"),
            r#"{"hosting":{"public":".","ignore":[]}}"#,
        )
        .unwrap();
        let blocked =
            super::firebase_deploy_preflight(&root, "firebase deploy --only hosting").unwrap_err();
        assert!(blocked.contains("FIREBASE_DEPLOY_BLOCKED"));

        std::fs::write(
            root.join(".firebaserc"),
            r#"{"projects":{"default":"aura-test-123456"}}"#,
        )
        .unwrap();
        std::fs::write(root.join("firebase-config.js"), r#"export default { apiKey: "public-web-key", authDomain: "aura-test.firebaseapp.com", projectId: "aura-test-123456", appId: "aura-test-app" };"#).unwrap();
        let ignores = [
            ".aura/**",
            ".firebase/**",
            "**/*.bat",
            "**/*.cmd",
            "**/*.exe",
            "**/*.dll",
            "**/*.apk",
            "**/*.ipa",
        ];
        let hosting = serde_json::json!({"hosting":{"public":".","ignore":ignores}});
        std::fs::write(
            root.join("firebase.json"),
            serde_json::to_string(&hosting).unwrap(),
        )
        .unwrap();
        assert!(super::firebase_deploy_preflight(&root, "firebase deploy --only hosting").is_ok());

        std::fs::write(
            root.join("firebase-config.js"),
            r#"export default { projectId: "other-project-654321" };"#,
        )
        .unwrap();
        let mismatch =
            super::firebase_deploy_preflight(&root, "firebase deploy --only hosting").unwrap_err();
        assert!(mismatch.contains("projectId"));

        std::fs::write(root.join("firebase-config.js"), r#"export default { apiKey: "public-web-key", authDomain: "other-project.firebaseapp.com", projectId: "other-project-654321", appId: "other-project-app" };"#).unwrap();
        assert!(super::firebase_deploy_preflight(
            &root,
            "firebase deploy --only hosting --project other-project-654321"
        )
        .is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn explicit_dashboard_and_verifier_use_one_grounded_phase_without_a_model_call() {
        let phases = super::generate_phase_plan(
            "Construye cyber_sentinel.html y verify_dashboard.py; ejecuta el verificador.",
            "unavailable-model",
        )
        .await;
        assert_eq!(phases.len(), 1);
        assert_eq!(
            phases[0].archivos,
            vec!["cyber_sentinel.html", "verify_dashboard.py"]
        );
        assert_eq!(phases[0].criterio_de_exito, "python verify_dashboard.py");
    }
}
