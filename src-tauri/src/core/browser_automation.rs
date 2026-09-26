//! Dependency-free browser interaction checks using the Chrome DevTools Protocol.
//!
//! The test plan is declarative: the model may choose selectors and expected
//! text, but it cannot submit arbitrary JavaScript to the browser.

use serde::Deserialize;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const PLAN_PREFIX: &str = "BROWSER_TEST:";
const NODE_CDP_RUNNER: &str = r#"
import fs from 'node:fs';
const input = JSON.parse(fs.readFileSync(0, 'utf8'));
if (typeof WebSocket !== 'function') throw new Error('NODE_WEBSOCKET_UNAVAILABLE');
const origin = `http://127.0.0.1:${input.port}`;
const browserVersion = await fetch(`${origin}/json/version`).then(response => response.json());
const targetResponse = await fetch(`${origin}/json/new?about:blank`, { method: 'PUT' });
if (!targetResponse.ok) throw new Error(`CDP_TARGET_CREATE_FAILED: ${targetResponse.status}`);
const target = await targetResponse.json();
const socket = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  socket.addEventListener('open', resolve, { once: true });
  socket.addEventListener('error', () => reject(new Error('CDP_WEBSOCKET_OPEN_FAILED')), { once: true });
});
let sequence = 0;
const pending = new Map();
const consoleErrors = [];
socket.addEventListener('message', event => {
  let message;
  try { message = JSON.parse(event.data); } catch { return; }
  if (message.method === 'Runtime.exceptionThrown') {
    consoleErrors.push(message.params?.exceptionDetails?.text || 'JavaScript exception');
  }
  if (message.method === 'Runtime.consoleAPICalled' && message.params?.type === 'error') {
    const text = (message.params.args || []).map(arg => arg.value ?? arg.description ?? '').join(' ');
    consoleErrors.push(text || 'console.error called');
  }
  if (message.id && pending.has(message.id)) {
    const { resolve, reject } = pending.get(message.id);
    pending.delete(message.id);
    if (message.error) reject(new Error(message.error.message));
    else resolve(message.result || {});
  }
});
function cdp(method, params = {}) {
  const id = ++sequence;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    socket.send(JSON.stringify({ id, method, params }));
    setTimeout(() => {
      if (pending.delete(id)) reject(new Error(`CDP_TIMEOUT: ${method}`));
    }, 10000).unref?.();
  });
}
async function evaluate(expression) {
  const result = await cdp('Runtime.evaluate', {
    expression, awaitPromise: true, returnByValue: true, userGesture: true,
  });
  if (result.exceptionDetails) {
    throw new Error(result.exceptionDetails.exception?.description || result.exceptionDetails.text);
  }
  return result.result?.value;
}
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
async function waitForPage() {
  const deadline = Date.now() + 12000;
  while (Date.now() < deadline) {
    try {
      if (await evaluate('document.readyState') === 'complete') return;
    } catch { /* navigation is still in progress */ }
    await pause(100);
  }
  throw new Error('PAGE_LOAD_TIMEOUT');
}
const q = value => JSON.stringify(value);
function requiredElement(selector) {
  return `(() => { const el = document.querySelector(${q(selector)}); if (!el) throw new Error('SELECTOR_NOT_FOUND: ' + ${q(selector)}); return el; })()`;
}
async function runStep(step) {
  const selector = step.selector;
  switch (step.action) {
    case 'click':
      await evaluate(`(() => { const el = ${requiredElement(selector)}; el.click(); return true; })()`);
      await pause(100);
      break;
    case 'fill':
      await evaluate(`(() => { const el = ${requiredElement(selector)}; el.focus(); el.value = ${q(step.value)}; el.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText', data: ${q(step.value)} })); el.dispatchEvent(new Event('change', { bubbles: true })); return true; })()`);
      break;
    case 'select':
      await evaluate(`(() => { const el = ${requiredElement(selector)}; el.value = ${q(step.value)}; el.dispatchEvent(new Event('change', { bubbles: true })); return true; })()`);
      break;
    case 'assert_visible':
      if (!await evaluate(`(() => { const el = document.querySelector(${q(selector)}); if (!el) return false; const s = getComputedStyle(el); const r = el.getBoundingClientRect(); return s.display !== 'none' && s.visibility !== 'hidden' && Number(s.opacity) !== 0 && r.width > 0 && r.height > 0; })()`)) throw new Error(`ASSERT_VISIBLE_FAILED: ${selector}`);
      break;
    case 'assert_hidden':
      if (await evaluate(`(() => { const el = document.querySelector(${q(selector)}); if (!el) return true; const s = getComputedStyle(el); const r = el.getBoundingClientRect(); return s.display === 'none' || s.visibility === 'hidden' || Number(s.opacity) === 0 || r.width === 0 || r.height === 0; })()`)) break;
      throw new Error(`ASSERT_HIDDEN_FAILED: ${selector}`);
    case 'assert_text':
      if (!await evaluate(`(() => { const el = document.querySelector(${q(selector)}); return !!el && (el.innerText || el.textContent || '').includes(${q(step.text)}); })()`)) throw new Error(`ASSERT_TEXT_FAILED: ${selector} should contain ${JSON.stringify(step.text)}`);
      break;
    case 'assert_value':
      if (await evaluate(`(() => { const el = document.querySelector(${q(selector)}); return el ? el.value : null; })()`) !== step.value) throw new Error(`ASSERT_VALUE_FAILED: ${selector}`);
      break;
    case 'assert_count':
      if (await evaluate(`document.querySelectorAll(${q(selector)}).length`) !== step.count) throw new Error(`ASSERT_COUNT_FAILED: ${selector} expected ${step.count}`);
      break;
    case 'wait_for_visible': {
      const deadline = Date.now() + (step.timeout_ms || 5000);
      while (Date.now() < deadline) {
        if (await evaluate(`(() => { const el = document.querySelector(${q(selector)}); if (!el) return false; const s = getComputedStyle(el); const r = el.getBoundingClientRect(); return s.display !== 'none' && s.visibility !== 'hidden' && r.width > 0 && r.height > 0; })()`)) return;
        await pause(100);
      }
      throw new Error(`WAIT_FOR_VISIBLE_TIMEOUT: ${selector}`);
    }
    case 'wait':
      await pause(step.milliseconds);
      break;
    case 'reload':
      await cdp('Page.reload', { ignoreCache: true });
      await waitForPage();
      break;
    case 'set_viewport':
      await cdp('Emulation.setDeviceMetricsOverride', {
        width: step.width, height: step.height, deviceScaleFactor: 1, mobile: step.width < 600,
      });
      break;
    case 'assert_no_console_errors':
      if (consoleErrors.length) throw new Error(`BROWSER_CONSOLE_ERRORS: ${consoleErrors.join(' | ')}`);
      break;
    default:
      throw new Error(`UNSUPPORTED_BROWSER_ACTION: ${step.action}`);
  }
}
let outcome = 0;
try {
  await cdp('Page.enable');
  await cdp('Runtime.enable');
  await cdp('Page.navigate', { url: input.plan.url });
  await waitForPage();
  console.log(`URL ${input.plan.url}`);
  for (let index = 0; index < input.plan.steps.length; index++) {
    const step = input.plan.steps[index];
    await runStep(step);
    console.log(`PASS ${index + 1}/${input.plan.steps.length} ${step.action}${step.selector ? ` ${step.selector}` : ''}`);
  }
  if (!input.plan.steps.some(step => step.action === 'assert_no_console_errors')) {
    if (consoleErrors.length) throw new Error(`BROWSER_CONSOLE_ERRORS: ${consoleErrors.join(' | ')}`);
    console.log('PASS console: no JavaScript or browser console errors');
  }
  console.log(`BROWSER_TEST_PASS: ${input.plan.steps.length} interaction checks`);
} catch (error) {
  outcome = 1;
  console.error(`BROWSER_TEST_FAIL: ${error?.stack || error}`);
} finally {
  try { socket.close(); } catch { }
  try {
    const control = new WebSocket(browserVersion.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('CDP_SHUTDOWN_TIMEOUT')), 1500);
      control.addEventListener('open', () => {
        control.send(JSON.stringify({ id: 1, method: 'Browser.close', params: {} }));
      }, { once: true });
      control.addEventListener('close', () => { clearTimeout(timer); resolve(); }, { once: true });
      control.addEventListener('error', () => { clearTimeout(timer); resolve(); }, { once: true });
    });
  } catch { }
}
process.exitCode = outcome;
"#;

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct BrowserTestPlan {
    url: String,
    steps: Vec<BrowserStep>,
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum BrowserStep {
    Click {
        selector: String,
    },
    Fill {
        selector: String,
        value: String,
    },
    Select {
        selector: String,
        value: String,
    },
    AssertVisible {
        selector: String,
    },
    AssertHidden {
        selector: String,
    },
    AssertText {
        selector: String,
        text: String,
    },
    AssertValue {
        selector: String,
        value: String,
    },
    AssertCount {
        selector: String,
        count: usize,
    },
    WaitForVisible {
        selector: String,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    Wait {
        milliseconds: u64,
    },
    Reload,
    SetViewport {
        width: u32,
        height: u32,
    },
    AssertNoConsoleErrors,
}

impl BrowserStep {
    fn validate(&self) -> Result<(), String> {
        let selector = match self {
            Self::Click { selector }
            | Self::Fill { selector, .. }
            | Self::Select { selector, .. }
            | Self::AssertVisible { selector }
            | Self::AssertHidden { selector }
            | Self::AssertText { selector, .. }
            | Self::AssertValue { selector, .. }
            | Self::AssertCount { selector, .. }
            | Self::WaitForVisible { selector, .. } => Some(selector),
            _ => None,
        };
        if selector.is_some_and(|value| value.trim().is_empty() || value.len() > 1000) {
            return Err(
                "BROWSER_TEST_INVALID: los selectores deben tener entre 1 y 1000 caracteres".into(),
            );
        }
        match self {
            Self::Wait { milliseconds } if *milliseconds > 5000 => {
                Err("BROWSER_TEST_INVALID: una espera no puede superar 5000 ms".into())
            }
            Self::WaitForVisible {
                timeout_ms: Some(timeout),
                ..
            } if *timeout > 10000 => {
                Err("BROWSER_TEST_INVALID: timeout_ms no puede superar 10000 ms".into())
            }
            Self::SetViewport { width, height }
                if !(240..=3000).contains(width) || !(320..=3000).contains(height) =>
            {
                Err(
                    "BROWSER_TEST_INVALID: dimensiones de viewport fuera del rango permitido"
                        .into(),
                )
            }
            Self::Fill { value, .. }
            | Self::Select { value, .. }
            | Self::AssertValue { value, .. }
                if value.len() > 5000 =>
            {
                Err("BROWSER_TEST_INVALID: el valor excede 5000 caracteres".into())
            }
            Self::AssertText { text, .. } if text.len() > 5000 => {
                Err("BROWSER_TEST_INVALID: el texto esperado excede 5000 caracteres".into())
            }
            _ => Ok(()),
        }
    }
}

pub fn is_browser_test_command(command: &str) -> bool {
    command.trim_start().starts_with(PLAN_PREFIX)
}

pub fn browser_test_plan_url(command: &str) -> Result<String, String> {
    Ok(parse_plan(command)?.url)
}

pub async fn execute_browser_test_command(
    workspace: &str,
    command: &str,
) -> Result<crate::core::tool_registry::ExecutionResult, String> {
    let plan = parse_plan(command)?;
    validate_local_url(&plan.url)?;
    let workspace = workspace.to_string();
    tokio::task::spawn_blocking(move || run_browser_test(&workspace, plan))
        .await
        .map_err(|error| format!("BROWSER_TEST_FAILED: no se pudo esperar el navegador: {error}"))?
}

pub fn validate_local_url(value: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value)
        .map_err(|error| format!("BROWSER_TEST_INVALID: URL inválida: {error}"))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url
            .host_str()
            .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"))
    {
        return Err(
            "BROWSER_TEST_DENIED: solo se permiten páginas HTTP locales en localhost/127.0.0.1"
                .into(),
        );
    }
    Ok(url)
}

fn parse_plan(command: &str) -> Result<BrowserTestPlan, String> {
    let json = command
        .trim_start()
        .strip_prefix(PLAN_PREFIX)
        .ok_or_else(|| "BROWSER_TEST_INVALID: falta el prefijo BROWSER_TEST:".to_string())?
        .trim();
    let plan: BrowserTestPlan = serde_json::from_str(json)
        .map_err(|error| format!("BROWSER_TEST_INVALID: plan JSON no válido: {error}"))?;
    if plan.steps.is_empty() || plan.steps.len() > 80 {
        return Err("BROWSER_TEST_INVALID: el plan debe incluir de 1 a 80 pasos".into());
    }
    for step in &plan.steps {
        step.validate()?;
    }
    Ok(plan)
}

fn run_browser_test(
    workspace: &str,
    plan: BrowserTestPlan,
) -> Result<crate::core::tool_registry::ExecutionResult, String> {
    let browser_path = crate::core::vision::browser_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            "BROWSER_AUTOMATION_UNAVAILABLE: no se encontró Chrome ni Edge".to_string()
        })?;
    let node_check = Command::new("node")
        .args(["-p", "typeof WebSocket"])
        .output()
        .map_err(|error| {
            format!("BROWSER_AUTOMATION_UNAVAILABLE: no se puede ejecutar Node.js: {error}")
        })?;
    if !node_check.status.success()
        || !String::from_utf8_lossy(&node_check.stdout).contains("function")
    {
        return Err("BROWSER_AUTOMATION_UNAVAILABLE: se requiere Node.js 21 o superior con WebSocket integrado".into());
    }

    let profile = std::env::temp_dir().join(format!("aura_browser_test_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&profile).map_err(|error| {
        format!("BROWSER_TEST_FAILED: no se pudo preparar el perfil temporal: {error}")
    })?;
    let profile_arg = format!("--user-data-dir={}", profile.to_string_lossy());
    let mut child = Command::new(browser_path)
        .args([
            "--headless=new",
            "--no-sandbox",
            "--disable-gpu",
            "--disable-extensions",
            "--disable-background-networking",
            "--no-first-run",
            "--no-default-browser-check",
            "--remote-debugging-port=0",
            "--remote-allow-origins=*",
            "about:blank",
        ])
        .arg(profile_arg)
        .current_dir(workspace)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            format!("BROWSER_AUTOMATION_UNAVAILABLE: no se pudo abrir Chrome/Edge: {error}")
        })?;

    let port_file = profile.join("DevToolsActivePort");
    let deadline = Instant::now() + Duration::from_secs(12);
    let port = loop {
        if let Ok(contents) = std::fs::read_to_string(&port_file) {
            if let Some(port) = contents
                .lines()
                .next()
                .and_then(|line| line.trim().parse::<u16>().ok())
            {
                break port;
            }
        }
        if child.try_wait().ok().flatten().is_some() {
            let _ = std::fs::remove_dir_all(&profile);
            return Err(
                "BROWSER_AUTOMATION_UNAVAILABLE: Chrome/Edge se cerró antes de habilitar DevTools"
                    .into(),
            );
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&profile);
            return Err(
                "BROWSER_AUTOMATION_UNAVAILABLE: Chrome/Edge no habilitó DevTools en 12 segundos"
                    .into(),
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    let input = serde_json::json!({ "port": port, "plan": plan });
    let mut runner = match Command::new("node")
        .args(["--input-type=module", "-e", NODE_CDP_RUNNER])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&profile);
            return Err(format!(
                "BROWSER_AUTOMATION_UNAVAILABLE: no se pudo iniciar el runner Node: {error}"
            ));
        }
    };
    if let Some(mut stdin) = runner.stdin.take() {
        if let Err(error) = stdin.write_all(input.to_string().as_bytes()) {
            let _ = runner.kill();
            let _ = runner.wait();
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&profile);
            return Err(format!(
                "BROWSER_TEST_FAILED: no se pudo enviar el plan: {error}"
            ));
        }
    }
    let timeout = Instant::now() + Duration::from_secs(90);
    loop {
        match runner.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < timeout => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = runner.kill();
                let _ = runner.wait();
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_dir_all(&profile);
                return Err("BROWSER_TEST_TIMEOUT: la prueba superó 90 segundos".into());
            }
        }
    }
    let output = runner.wait_with_output().map_err(|error| {
        format!("BROWSER_TEST_FAILED: no se pudo leer la salida de Node: {error}")
    })?;
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&profile);
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if output.status.success() && stdout.contains("BROWSER_TEST_PASS:") {
        Ok(crate::core::tool_registry::ExecutionResult::success(stdout))
    } else {
        Err(format!(
            "{}{}{}",
            stderr.lines().next().unwrap_or("BROWSER_TEST_FAIL"),
            if stdout.is_empty() { "" } else { "\n" },
            if stdout.is_empty() { &stderr } else { &stdout }
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_local_browser_test_plans_with_real_steps() {
        let valid = r##"BROWSER_TEST: {"url":"http://127.0.0.1:8000/","steps":[{"action":"assert_visible","selector":"main"},{"action":"click","selector":"#save"}]}"##;
        assert!(is_browser_test_command(valid));
        assert_eq!(
            browser_test_plan_url(valid).unwrap(),
            "http://127.0.0.1:8000/"
        );
        assert!(validate_local_url("http://localhost:8000/").is_ok());
        assert!(validate_local_url("file:///index.html").is_err());
        assert!(validate_local_url("https://example.com/").is_err());
        assert!(parse_plan("BROWSER_TEST: {\"url\":\"http://localhost/\",\"steps\":[]}").is_err());
        assert!(parse_plan("BROWSER_TEST: {\"url\":\"http://localhost/\",\"steps\":[{\"action\":\"evaluate\",\"script\":\"alert(1)\"}]}").is_err());
    }

    #[test]
    fn bounds_user_supplied_browser_actions() {
        let too_long_wait = r#"BROWSER_TEST: {"url":"http://localhost/","steps":[{"action":"wait","milliseconds":6000}]}"#;
        assert!(parse_plan(too_long_wait).is_err());
        let bad_viewport = r#"BROWSER_TEST: {"url":"http://localhost/","steps":[{"action":"set_viewport","width":100,"height":400}]}"#;
        assert!(parse_plan(bad_viewport).is_err());
    }

    #[tokio::test]
    async fn local_browser_runner_executes_click_fill_reload_and_console_checks() {
        if crate::core::vision::browser_candidates()
            .into_iter()
            .all(|path| !path.is_file())
            || Command::new("node")
                .args(["-p", "typeof WebSocket"])
                .output()
                .ok()
                .is_none_or(|output| !String::from_utf8_lossy(&output.stdout).contains("function"))
        {
            return;
        }

        let workspace = std::env::temp_dir().join(format!(
            "aura-browser-automation-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("index.html"),
            r#"<!doctype html><html lang="es"><meta charset="utf-8"><main><label for="client">Cliente</label><input id="client"><button id="save" type="button">Guardar cita</button><p id="status">Sin citas</p></main><script>const status = document.querySelector('#status'); if (localStorage.getItem('client')) status.textContent = `Persistida: ${localStorage.getItem('client')}`; document.querySelector('#save').addEventListener('click', () => { const value = document.querySelector('#client').value; localStorage.setItem('client', value); status.textContent = `Guardada: ${value}`; });</script></html>"#,
        )
        .unwrap();

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let port_text = port.to_string();
        let mut server = Command::new("python")
            .args([
                "-m",
                "http.server",
                &port_text,
                "--bind",
                "127.0.0.1",
                "--directory",
            ])
            .arg(&workspace)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Python is required to run the local browser fixture");
        let url = format!("http://127.0.0.1:{port}/");
        let ready_deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < ready_deadline {
            if reqwest::get(&url).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let command = format!(
            "BROWSER_TEST: {}",
            serde_json::json!({
                "url": url,
                "steps": [
                    {"action":"assert_visible","selector":"#client"},
                    {"action":"fill","selector":"#client","value":"Ana Demo"},
                    {"action":"click","selector":"#save"},
                    {"action":"assert_text","selector":"#status","text":"Guardada: Ana Demo"},
                    {"action":"reload"},
                    {"action":"assert_text","selector":"#status","text":"Persistida: Ana Demo"},
                    {"action":"set_viewport","width":390,"height":844},
                    {"action":"assert_no_console_errors"}
                ]
            })
        );
        let result = execute_browser_test_command(&workspace.to_string_lossy(), &command).await;
        let _ = server.kill();
        let _ = server.wait();
        let _ = std::fs::remove_dir_all(&workspace);
        let result = result.expect("browser interactions should pass against the local fixture");
        assert!(result
            .stdout
            .contains("BROWSER_TEST_PASS: 8 interaction checks"));
        assert!(result.stdout.contains("PASS 6/8 assert_text"));
    }
}
