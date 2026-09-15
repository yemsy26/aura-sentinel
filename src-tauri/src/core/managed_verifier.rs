use crate::core::mission_contract::{MissionContract, VerificationMethod};
use std::path::Path;

const TACTICAL_DASHBOARD_VERIFIER: &str = r###"# AURA_MANAGED_VERIFIER_V2
import json
import re
from pathlib import Path

def read_project_source():
    parts = []
    for pattern in ("*.html", "*.css", "*.js", "*.ts"):
        for path in sorted(Path(".").glob(pattern)):
            parts.append(path.read_text(encoding="utf-8", errors="ignore").lower())
    return "\n".join(parts)

if __name__ == "__main__":
    source = read_project_source()
    checks = [
        (
            "Estructura HTML5 y elemento Canvas presentes",
            "<!doctype html" in source and "<canvas" in source,
        ),
        (
            "Radar animado con barrido y nodos en coordenadas polares",
            "requestanimationframe" in source
            and "math.cos" in source
            and "math.sin" in source
            and ("threat" in source or "amenaza" in source or "node" in source),
        ),
        (
            "Telemetría de paquetes con IP, protocolo, latencia y riesgo",
            ("packet" in source or "paquete" in source)
            and "ip" in source
            and ("protocol" in source or "protocolo" in source)
            and "latenc" in source
            and ("risk" in source or "riesgo" in source)
            and re.search(r"\b(?:const|let|var|function)\s+\w*(?:packet|paquete)\w*", source) is not None
            and ("textcontent" in source or "innerhtml" in source),
        ),
        (
            "Gráfico dinámico de tráfico y barra de alerta de ataques",
            ("traffic" in source or "trafico" in source or "tráfico" in source)
            and (
                source.count("<canvas")
                + source.count("createelement('canvas')")
                + source.count('createelement("canvas")')
            ) >= 2
            and source.count("getcontext") >= 2
            and ("trafficctx" in source or "trafficcontext" in source)
            and ("alert" in source or "alerta" in source)
            and ("attack" in source or "ataque" in source),
        ),
        (
            "Diseño oscuro glassmorphism, monospace y neón",
            "background" in source
            and "monospace" in source
            and ("glass" in source or "backdrop-filter" in source)
            and ("neon" in source or "box-shadow" in source or "text-shadow" in source),
        ),
    ]
    failed_criteria = [description for description, passed_check in checks if not passed_check]
    passed = sum(1 for _, passed_check in checks if passed_check)
    total = len(checks)
    percentage = 100 * passed / total
    print(json.dumps({
        "passed": passed,
        "total": total,
        "percentage": percentage,
        "failed_criteria": failed_criteria,
    }, ensure_ascii=False))
    exit(0 if passed == total else 1)
"###;

pub fn ensure_for_mission(
    workspace_path: &str,
    objective: &str,
    contract: &MissionContract,
) -> Result<Option<String>, String> {
    let lower = objective.to_lowercase();
    let supported = lower.contains("dashboard")
        && lower.contains("radar")
        && lower.contains("canvas")
        && lower.contains("telemetr")
        && (lower.contains("tráfico") || lower.contains("trafico"));
    if !supported {
        return Ok(None);
    }

    let command = contract.acceptance_criteria.iter().find_map(|criterion| {
        if let VerificationMethod::SemanticVerification { command } = &criterion.verification {
            Some(command.as_str())
        } else {
            None
        }
    });
    let Some(command) = command else {
        return Ok(None);
    };
    let Some(requested) = command.split_whitespace().last() else {
        return Ok(None);
    };
    let requested_path = Path::new(requested);
    let safe_name = requested_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| name.starts_with("verify_") && name.ends_with(".py"));
    let Some(file_name) = safe_name else {
        return Ok(None);
    };
    if requested_path.components().count() != 1 {
        return Err(format!(
            "MANAGED_VERIFIER_PATH_REJECTED: la ruta debe ser un nombre relativo simple: {}",
            requested
        ));
    }
    let destination = Path::new(workspace_path).join(file_name);
    if destination.exists() {
        return Ok(None);
    }
    std::fs::write(&destination, TACTICAL_DASHBOARD_VERIFIER)
        .map_err(|error| format!("MANAGED_VERIFIER_WRITE_FAILED: {}", error))?;
    Ok(Some(file_name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_a_structured_dashboard_verifier_once() {
        let root =
            std::env::temp_dir().join(format!("aura-managed-verifier-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let objective = "Dashboard con radar Canvas, telemetría y tráfico";
        let mut contract = MissionContract::new(objective);
        contract.add_criterion(
            "verify",
            "Verificación semántica",
            VerificationMethod::SemanticVerification {
                command: "python verify_dashboard.py".into(),
            },
            true,
        );
        let first = ensure_for_mission(root.to_str().unwrap(), objective, &contract).unwrap();
        let second = ensure_for_mission(root.to_str().unwrap(), objective, &contract).unwrap();
        let source = std::fs::read_to_string(root.join("verify_dashboard.py")).unwrap();
        assert_eq!(first.as_deref(), Some("verify_dashboard.py"));
        assert!(second.is_none());
        assert!(source.contains("failed_criteria"));
        assert!(source.contains("requestanimationframe"));
        assert!(source.contains("import re"));
        assert!(source.contains("trafficctx"));
        let _ = std::fs::remove_dir_all(root);
    }
}
