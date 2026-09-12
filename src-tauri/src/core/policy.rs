#![allow(dead_code)]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RiskLevel {
    Safe,
    Moderate,
    Dangerous,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyDecision {
    Allow,
    Deny(String),
    RequireUser(String),
    Sandbox(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionProposal {
    pub tool: String,
    pub arguments: serde_json::Value,
    pub expected_effect: String,
    pub risk: RiskLevel,
    pub world_hash: Option<u64>,
}

pub struct PolicyEngine;

impl PolicyEngine {
    /// Evalúa una propuesta de acción de una herramienta y determina si se autoriza la ejecución.
    pub fn authorize(proposal: &ActionProposal) -> PolicyDecision {
        let tool = proposal.tool.to_uppercase();

        // 1. Reglas estrictas para TOOL_TERMINAL
        if tool == "TOOL_TERMINAL" {
            let cmd = proposal.arguments.get("comando")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();

            let cmd_lower = cmd.to_lowercase();

            // Bloquear comandos destructivos y operaciones críticas del sistema operativo
            let dangerous_patterns = [
                "format ", "del /f /s /q c:\\", "rmdir /s /q c:\\", "rd /s /q c:\\",
                "drop database", "shutdown", "reg delete", "diskpart",
                "powershell remove-item -recurse -force c:\\",
                ":(){ :|:& };:", // forkbomb
            ];

            for pattern in &dangerous_patterns {
                if cmd_lower.contains(pattern) {
                    return PolicyDecision::Deny(format!(
                        "[POLÍTICA DE SEGURIDAD] Comando prohibido por alto riesgo destructivo: '{}'",
                        pattern
                    ));
                }
            }

            // Operaciones que requieren confirmación explícita del usuario
            let user_required_patterns = [
                "git push --force", "git reset --hard", "drop table", "truncate table"
            ];

            for pattern in &user_required_patterns {
                if cmd_lower.contains(pattern) {
                    return PolicyDecision::RequireUser(format!(
                        "La acción involucra '{}' y requiere confirmación explícita del usuario.",
                        cmd
                    ));
                }
            }
        }

        // 2. Comprobación general por nivel de riesgo declarado
        match proposal.risk {
            RiskLevel::Safe => PolicyDecision::Allow,
            RiskLevel::Moderate => PolicyDecision::Allow,
            RiskLevel::Dangerous => PolicyDecision::Deny(
                "Acción clasificada como Dangerous sin autorización específica.".to_string(),
            ),
        }
    }

    /// Clasifica el nivel de riesgo de un comando terminal
    pub fn classify_terminal_command(command: &str) -> RiskLevel {
        let cmd_lower = command.to_lowercase();
        if cmd_lower.contains("rm ") || cmd_lower.contains("del ") || cmd_lower.contains("rd ") || cmd_lower.contains("remove-item") {
            RiskLevel::Moderate
        } else if cmd_lower.contains("format") || cmd_lower.contains("diskpart") || cmd_lower.contains("reg delete") {
            RiskLevel::Dangerous
        } else {
            RiskLevel::Safe
        }
    }
}
