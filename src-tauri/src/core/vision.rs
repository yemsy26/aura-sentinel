use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::json;
use std::path::{Path, PathBuf};
use xcap::Monitor;

fn choose_vision_model(body: &str, prefer_advanced: bool) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let models = value.get("models")?.as_array()?;
    let installed = models
        .iter()
        .filter_map(|model| model.get("name").or_else(|| model.get("model")))
        .filter_map(serde_json::Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();

    let candidates: &[&str] = if prefer_advanced {
        &[
            "qwen3-vl",
            "qwen2.5vl",
            "llama3.2-vision",
            "llava",
            "llava-phi3",
            "minicpm-v",
            "gemma3",
            "bakllava",
        ]
    } else {
        &[
            "moondream",
            "qwen3-vl",
            "qwen2.5vl",
            "llava",
            "llama3.2-vision",
            "llava-phi3",
            "minicpm-v",
            "gemma3",
            "bakllava",
        ]
    };

    candidates.iter().find_map(|candidate| {
        installed.iter().find_map(|name| {
            let family = name.split(':').next().unwrap_or(name).to_ascii_lowercase();
            (family == **candidate).then(|| name.clone())
        })
    })
}

/// Find a real static entry page when no development-server URL was reported.
pub fn workspace_static_entry_url(workspace: &Path) -> Option<String> {
    let package_path = workspace.join("package.json");
    if let Ok(package) = std::fs::read_to_string(package_path) {
        if let Ok(package) = serde_json::from_str::<serde_json::Value>(&package) {
            let has_runtime_script = package
                .get("scripts")
                .and_then(serde_json::Value::as_object)
                .is_some_and(|scripts| {
                    ["dev", "start", "serve"]
                        .iter()
                        .any(|name| scripts.contains_key(*name))
                });
            let has_bundler = ["vite", "next", "react-scripts", "webpack", "parcel", "nuxt"]
                .iter()
                .any(|marker| package.to_string().contains(marker));
            if has_runtime_script || has_bundler {
                return None;
            }
        }
    }

    ["index.html", "src/index.html", "public/index.html"]
        .iter()
        .map(|relative| workspace.join(relative))
        .find(|path| path.is_file())
        .and_then(|path| file_url(&path))
}

fn file_url(path: &Path) -> Option<String> {
    let absolute = path.canonicalize().ok()?;
    let normalized = absolute.to_string_lossy().replace('\\', "/");
    let mut encoded = String::with_capacity(normalized.len());
    for byte in normalized.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b':' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    if !encoded.starts_with('/') {
        encoded.insert(0, '/');
    }
    Some(format!("file://{encoded}"))
}

async fn find_vision_model(prefer_advanced: bool) -> Option<String> {
    let client = reqwest::Client::new();
    let response = client
        .get("http://127.0.0.1:11434/api/tags")
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
        .ok()?;
    let body = response.text().await.ok()?;
    choose_vision_model(&body, prefer_advanced)
}

pub(crate) fn browser_candidates() -> Vec<PathBuf> {
    let mut paths = vec![
        PathBuf::from(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"),
        PathBuf::from(r"C:\Program Files\Microsoft\Edge\Application\msedge.exe"),
        PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
        PathBuf::from(r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe"),
    ];
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let root = PathBuf::from(local_app_data);
        paths.push(root.join(r"Microsoft\Edge\Application\msedge.exe"));
        paths.push(root.join(r"Google\Chrome\Application\chrome.exe"));
    }
    paths
}

async fn capture_headless_browser(url: &str, screenshot_path: &Path) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        let browser_path = browser_candidates()
            .into_iter()
            .find(|path| path.is_file())
            .ok_or_else(|| {
                "VISUAL_CAPTURE_FAILED: No se encontró Chrome ni Edge para abrir la página local."
                    .to_string()
            })?;
        if let Some(parent) = screenshot_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                format!("VISUAL_CAPTURE_FAILED: no se pudo crear la carpeta de evidencia: {e}")
            })?;
        }

        let profile_dir =
            std::env::temp_dir().join(format!("aura_vision_profile_{}", uuid::Uuid::new_v4()));
        let screenshot_arg = format!("--screenshot={}", screenshot_path.to_string_lossy());
        let profile_arg = format!("--user-data-dir={}", profile_dir.to_string_lossy());
        let mut browser = tokio::process::Command::new(&browser_path);
        browser.kill_on_drop(true).args([
            "--headless",
            "--no-sandbox",
            "--disable-gpu",
            "--disable-gpu-sandbox",
            "--disable-gpu-rasterization",
            "--disable-gpu-compositing",
            "--disable-accelerated-2d-canvas",
            "--disable-features=UseSkiaRenderer,SkiaGraphite,Vulkan,WebGPU,VizDisplayCompositor",
            "--use-gl=angle",
            "--use-angle=swiftshader",
            "--in-process-gpu",
            "--disable-extensions",
            "--disable-background-networking",
            "--no-first-run",
            "--hide-scrollbars",
            "--window-size=1440,1100",
            "--timeout=10000",
            &profile_arg,
            &screenshot_arg,
            url,
        ]);
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(35), browser.output()).await;
        let _ = tokio::fs::remove_dir_all(&profile_dir).await;
        let result = result
            .map_err(|_| {
                "VISUAL_CAPTURE_FAILED: Chrome/Edge agotó el límite de 35 segundos.".to_string()
            })?
            .map_err(|e| format!("VISUAL_CAPTURE_FAILED: no se pudo iniciar el navegador: {e}"))?;
        if !result.status.success() {
            let _ = tokio::fs::remove_file(screenshot_path).await;
            let stderr = String::from_utf8_lossy(&result.stderr);
            return Err(format!(
                "VISUAL_CAPTURE_FAILED: el navegador terminó con {}: {}",
                result.status,
                stderr.chars().take(500).collect::<String>()
            ));
        }

        let bytes = tokio::fs::read(screenshot_path)
            .await
            .map_err(|e| format!("VISUAL_CAPTURE_FAILED: no se pudo leer la captura: {e}"))?;
        if bytes.len() < 128 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            let _ = tokio::fs::remove_file(screenshot_path).await;
            return Err(
                "VISUAL_CAPTURE_FAILED: el navegador no produjo una captura PNG válida."
                    .to_string(),
            );
        }
        Ok(STANDARD.encode(&bytes))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (url, screenshot_path);
        Err(
            "VISUAL_CAPTURE_FAILED: la captura headless solo está implementada en Windows."
                .to_string(),
        )
    }
}

fn visual_verdict(response: &str) -> Option<&'static str> {
    for line in response.lines() {
        let normalized = line
            .trim()
            .trim_matches(|character: char| matches!(character, '*' | '`' | '#' | ' '))
            .to_ascii_uppercase();
        let value = normalized
            .strip_prefix("VEREDICTO:")
            .or_else(|| normalized.strip_prefix("VISUAL VERDICT:"))
            .or_else(|| normalized.strip_prefix("VISUAL_STATUS:"))
            .map(str::trim);
        if let Some(value) = value {
            if value.starts_with("APROBADO") || value.starts_with("PASS") {
                return Some("PASS");
            }
            if value.starts_with("REQUIERE CORRECCIONES") || value.starts_with("FAIL") {
                return Some("FAIL");
            }
            if value.starts_with("INCIERTO") || value.starts_with("UNCERTAIN") {
                return Some("UNCERTAIN");
            }
        }
    }
    let first_line = response
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .trim_matches(|character: char| !character.is_ascii_alphanumeric());
    let normalized = first_line.to_ascii_uppercase();
    if normalized.starts_with("PASS") {
        return Some("PASS");
    }
    if normalized.starts_with("FAIL") {
        return Some("FAIL");
    }
    if normalized.starts_with("UNCERTAIN") {
        return Some("UNCERTAIN");
    }

    // Heuristic fallback for descriptive models (e.g. moondream)
    let lower = response.to_lowercase();
    let is_valid_page = lower.contains("webpage")
        || lower.contains("web page")
        || lower.contains("website")
        || lower.contains("form")
        || lower.contains("button")
        || lower.contains("screen")
        || lower.contains("interface")
        || lower.contains("browser");
    let is_fatal_error = lower.contains("error 404")
        || lower.contains("not found")
        || lower.contains("cannot be reached")
        || lower.contains("connection refused")
        || lower.contains("completely blank")
        || lower.contains("empty white screen");

    if is_valid_page && !is_fatal_error {
        return Some("PASS");
    }

    None
}

pub async fn evaluate_vision(
    prompt: &str,
    use_advanced_model: bool,
    target_url: Option<&str>,
    workspace_path: &Path,
    require_target: bool,
) -> Result<String, String> {
    let vision_model = find_vision_model(use_advanced_model).await;

    let target_url = target_url.map(str::trim).filter(|url| {
        url.starts_with("http://") || url.starts_with("https://") || url.starts_with("file:///")
    });
    let inferred_url = if require_target && target_url.is_none() {
        workspace_static_entry_url(workspace_path)
    } else {
        None
    };
    let capture_url = target_url.or(inferred_url.as_deref());
    if require_target && capture_url.is_none() {
        return Err("VISUAL_TARGET_MISSING: No se encontró un index.html ni una URL local confirmada. Inicia la aplicación y lee los logs del servidor para obtener la dirección localhost antes de evaluar.".to_string());
    }

    let evidence_path = if require_target {
        let dir = workspace_path.join(".aura").join("evidence").join("visual");
        dir.join(format!("ui-{}.png", uuid::Uuid::new_v4()))
    } else {
        std::env::temp_dir().join(format!("aura_vision_{}.png", uuid::Uuid::new_v4()))
    };

    let base64_img = if let Some(url) = capture_url {
        capture_headless_browser(url, &evidence_path).await?
    } else {
        let monitors = Monitor::all().map_err(|e| format!("Failed to get monitors: {e}"))?;
        let primary = monitors.first().ok_or("No monitors found")?;
        let image = primary
            .capture_image()
            .map_err(|e| format!("Failed to capture screen: {e}"))?;
        let mut buffer = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut buffer);
        image
            .write_to(&mut cursor, image::ImageFormat::Png)
            .map_err(|e| format!("Failed to encode png: {e}"))?;
        STANDARD.encode(&buffer)
    };

    let relative_evidence = evidence_path
        .strip_prefix(workspace_path)
        .unwrap_or(&evidence_path)
        .to_string_lossy()
        .replace('\\', "/");
    let vision_model = vision_model.ok_or_else(|| {
        format!(
            "VISUAL_QA_UNAVAILABLE: Ollama no tiene instalado ni expone un modelo de visión compatible. La captura se guardó en {relative_evidence}; no se marcó la evaluación como exitosa."
        )
    })?;

    let evaluation_prompt = if require_target {
        if vision_model.contains("moondream") {
            "Describe what is shown in this screenshot. Mention any webpage titles, forms, buttons, or text visible.".to_string()
        } else {
            "Inspect this screenshot of a local web application. Is a real webpage visibly rendered with interactive UI elements, rather than a completely blank page, browser error (such as 404 or connection refused), or broken layout? Reply with exactly one word: PASS, FAIL, or UNCERTAIN. Do not guess about behavior that is not visible.".to_string()
        }
    } else {
        prompt.to_string()
    };

    let client = reqwest::Client::new();
    let payload = json!({
        "model": vision_model,
        "prompt": evaluation_prompt,
        "images": [base64_img],
        "stream": false
    });

    let res = client
        .post("http://127.0.0.1:11434/api/generate")
        .timeout(std::time::Duration::from_secs(180))
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("VISUAL_QA_UNAVAILABLE: Ollama no pudo evaluar la captura: {e}"))?;

    if !res.status().is_success() {
        return Err(format!(
            "VISUAL_QA_UNAVAILABLE: Ollama devolvió {} al evaluar la captura.",
            res.status()
        ));
    }
    let response_text = res
        .text()
        .await
        .map_err(|e| format!("VISUAL_QA_UNAVAILABLE: no se pudo leer la evaluación de Ollama: {e}. Captura: {relative_evidence}"))?;
    let json_val: serde_json::Value = serde_json::from_str(&response_text)
        .map_err(|e| format!("VISUAL_QA_INVALID_RESPONSE: respuesta no válida de Ollama: {e}. Captura: {relative_evidence}"))?;
    let response = json_val["response"]
        .as_str()
        .ok_or_else(|| format!("VISUAL_QA_INVALID_RESPONSE: Ollama no devolvió el campo response. Captura: {relative_evidence}"))?;

    if require_target {
        match visual_verdict(response) {
            Some("PASS") => Ok(format!(
                "[VISION QA PASS - {vision_model}]\nURL: {}\nCaptura: {relative_evidence}\n{response}",
                capture_url.unwrap_or_default()
            )),
            Some("FAIL") => Err(format!("VISUAL_QA_REJECTED: La captura presenta defectos o requisitos visibles ausentes. Captura: {relative_evidence}\n{response}")),
            Some("UNCERTAIN") => Err(format!("VISUAL_QA_UNCERTAIN: El modelo no pudo confirmar la interfaz con evidencia suficiente. Captura: {relative_evidence}\n{response}")),
            _ => Err(format!("VISUAL_QA_INVALID_RESPONSE: el modelo no entregó un veredicto reconocible. Captura: {relative_evidence}\n{response}")),
        }
    } else {
        Ok(format!("[VISION QA - {vision_model}]: {response}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vision_model_must_be_installed_and_uses_its_exact_tag() {
        let tags = r#"{"models":[{"name":"qwen2.5-coder:7b"},{"name":"moondream:latest"}]}"#;
        assert_eq!(
            choose_vision_model(tags, false).as_deref(),
            Some("moondream:latest")
        );
        assert_eq!(
            choose_vision_model(tags, true).as_deref(),
            Some("moondream:latest")
        );
        assert_eq!(
            choose_vision_model(
                r#"{"models":[{"name":"my-moondream-helper:latest"}]}"#,
                false
            ),
            None
        );
    }

    #[test]
    fn visual_verdict_requires_an_explicit_supported_label() {
        assert_eq!(
            visual_verdict("VEREDICTO: APROBADO\nLa página se ve completa."),
            Some("PASS")
        );
        assert_eq!(
            visual_verdict("**VEREDICTO: REQUIERE CORRECCIONES**\nFalta el menú."),
            Some("FAIL")
        );
        assert_eq!(visual_verdict("!!!UNCERTAIN!!!"), Some("UNCERTAIN"));
        assert_eq!(visual_verdict("Se ve correcta."), None);
    }

    #[test]
    fn static_entry_is_converted_to_a_file_url_with_escaped_spaces() {
        let workspace = std::env::temp_dir().join(format!("aura vision {}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("index.html"), "<main>App</main>").unwrap();
        let url = workspace_static_entry_url(&workspace).unwrap();
        assert!(url.starts_with("file:///"));
        assert!(url.contains("%20"));
        assert!(url.ends_with("index.html"));
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn dev_server_projects_must_use_the_running_local_url() {
        let workspace =
            std::env::temp_dir().join(format!("aura vision app {}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("index.html"), "<main>App</main>").unwrap();
        std::fs::write(
            workspace.join("package.json"),
            r#"{"scripts":{"dev":"vite"},"devDependencies":{"vite":"*"}}"#,
        )
        .unwrap();
        assert_eq!(workspace_static_entry_url(&workspace), None);
        let _ = std::fs::remove_dir_all(workspace);
    }
}
