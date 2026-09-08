#![allow(dead_code)]
use serde::{Deserialize, Serialize};

/// Classification of error types to enable targeted recovery strategies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorClass {
    Syntax,
    Compile,
    Dependency,
    Test,
    Runtime,
    Environment,
    Permission,
    Network,
    Tool,
    Model,
    Unknown,
}

/// The decision the runtime should take in response to a failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryDecision {
    /// Retry the same operation with a fix suggestion.
    Retry { advice: String },
    /// Change the approach / strategy entirely.
    ChangeStrategy { advice: String },
    /// Switch to a different tool.
    ChangeTool { recommended_tool: String, rationale: String },
    /// Trigger a full re-plan of remaining steps.
    Replan { reason: String },
    /// Attempt to repair the environment (install deps, check PATH, etc.).
    RepairEnvironment { advice: String },
    /// Escalate to the user for input.
    AskUser { prompt: String },
    /// No recovery possible, abort mission.
    Abort { reason: String },
}

/// Legacy action alias kept for backward compatibility.
pub type RecoveryAction = RecoveryDecision;

/// Classifies an error message string into an ErrorClass.
pub fn classify_error(error_msg: &str) -> ErrorClass {
    let e = error_msg.to_lowercase();
    // Windows-specific: 'touch' not recognized, or "El nombre del directorio no es válido"
    if e.contains("el nombre del directorio no es") || e.contains("nombre del directorio")
        || e.contains("is not recognized") && e.contains("touch")
        || e.contains("no se reconoce el comando interno") && e.contains("touch")
    {
        return ErrorClass::Environment;
    }
    if e.contains("syntax") || e.contains("expected") || e.contains("unexpected token") || e.contains("parse error") {
        ErrorClass::Syntax
    } else if e.contains("error[e") || e.contains("cannot find") || e.contains("undeclared") || e.contains("does not exist") {
        ErrorClass::Compile
    } else if e.contains("no such crate") || e.contains("unresolved import") || e.contains("could not find") || e.contains("package not found") {
        ErrorClass::Dependency
    } else if e.contains("test failed") || e.contains("panicked at") || e.contains("assertion failed") {
        ErrorClass::Test
    } else if e.contains("permission denied") || e.contains("access is denied") {
        ErrorClass::Permission
    } else if e.contains("network") || e.contains("connection refused") || e.contains("timeout") || e.contains("tls") {
        ErrorClass::Network
    } else if e.contains("not recognized") || e.contains("not found") || e.contains("no se reconoce") || e.contains("command not found") {
        ErrorClass::Environment
    } else if e.contains("tool") {
        ErrorClass::Tool
    } else {
        ErrorClass::Unknown
    }
}

#[derive(Debug, Default)]
pub struct RecoveryEngine {
    failure_counts: std::collections::HashMap<String, usize>,
}

impl RecoveryEngine {
    pub fn new() -> Self { Self::default() }

    /// Records a failure and returns the best recovery decision based on
    /// error classification and failure history.
    pub fn recover(&mut self, tool_name: &str, error_msg: &str, error_class: ErrorClass) -> RecoveryDecision {
        let count = self.failure_counts.entry(tool_name.to_string()).or_insert(0);
        *count += 1;
        let failures = *count;

        // Hard cap: abort after 5 consecutive failures on same tool
        if failures >= 5 {
            return RecoveryDecision::Abort {
                reason: format!("'{}' ha fallado {} veces consecutivas — misión inviable.", tool_name, failures),
            };
        }

        // Escalate to user after 3 consecutive failures
        if failures >= 3 {
            return RecoveryDecision::AskUser {
                prompt: format!(
                    "'{}' ha fallado {} veces con error: '{}'. ¿Cómo deseas proceder?",
                    tool_name, failures, error_msg
                ),
            };
        }

        match error_class {
            ErrorClass::Syntax | ErrorClass::Compile => {
                if tool_name == "TOOL_PROGRAMMER" && failures >= 2 {
                    RecoveryDecision::ChangeStrategy {
                        advice: "Reescribe el módulo completo en lugar de hacer un patch parcial.".to_string(),
                    }
                } else {
                    RecoveryDecision::Retry {
                        advice: format!("Corrige el error de compilación/sintaxis antes de reintentar: {}", error_msg),
                    }
                }
            }
            ErrorClass::Dependency => RecoveryDecision::RepairEnvironment {
                advice: format!("Instala las dependencias faltantes o ajusta Cargo.toml/package.json: {}", error_msg),
            },
            ErrorClass::Test => RecoveryDecision::Retry {
                advice: format!("Un test falló. Analiza el output del test y corrige la lógica: {}", error_msg),
            },
            ErrorClass::Environment => {
                let windows_hint = if error_msg.to_lowercase().contains("touch")
                    || error_msg.to_lowercase().contains("nombre del directorio")
                {
                    "En Windows 'touch' no existe. Usa: 'New-Item -Type File <nombre>' o 'echo $null > <nombre>'".to_string()
                } else {
                    format!("El comando no está disponible en este entorno. Verifica la disponibilidad o usa una alternativa compatible: {}", error_msg)
                };
                RecoveryDecision::RepairEnvironment { advice: windows_hint }
            }
            ErrorClass::Permission => RecoveryDecision::AskUser {
                prompt: format!("Permiso denegado al ejecutar '{}'. ¿Requieres elevar privilegios?", tool_name),
            },
            ErrorClass::Network => RecoveryDecision::Retry {
                advice: "Error de red transitorio. Reintenta en unos segundos.".to_string(),
            },
            ErrorClass::Model | ErrorClass::Tool | ErrorClass::Runtime | ErrorClass::Unknown => {
                RecoveryDecision::Replan {
                    reason: format!("Error no clasificable en '{}': {}. Se recomienda replanning.", tool_name, error_msg),
                }
            }
        }
    }

    /// Legacy method — wraps recover() with auto-classification.
    pub fn plan_recovery(&mut self, tool_name: &str, error_msg: &str) -> RecoveryDecision {
        let class = classify_error(error_msg);
        self.recover(tool_name, error_msg, class)
    }

    pub fn record_success(&mut self, tool_name: &str) {
        self.failure_counts.remove(tool_name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_error() {
        assert_eq!(classify_error("error[E0412]: cannot find type"), ErrorClass::Compile);
        assert_eq!(classify_error("panicked at 'assertion failed'"), ErrorClass::Test);
        assert_eq!(classify_error("permission denied (os error 13)"), ErrorClass::Permission);
        assert_eq!(classify_error("node is not recognized as a command"), ErrorClass::Environment);
    }

    #[test]
    fn test_recovery_planning() {
        let mut engine = RecoveryEngine::new();
        let dec = engine.recover("TOOL_TERMINAL", "package not found", ErrorClass::Dependency);
        assert!(matches!(dec, RecoveryDecision::RepairEnvironment { .. }));
    }

    #[test]
    fn test_escalation_on_repeated_failure() {
        let mut engine = RecoveryEngine::new();
        engine.recover("TOOL_PROGRAMMER", "syntax error", ErrorClass::Syntax);
        engine.recover("TOOL_PROGRAMMER", "syntax error", ErrorClass::Syntax);
        let dec = engine.recover("TOOL_PROGRAMMER", "syntax error", ErrorClass::Syntax);
        assert!(matches!(dec, RecoveryDecision::AskUser { .. }));
    }
}
