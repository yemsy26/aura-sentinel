/// FINAL-2: ToolRegistry — Authority for valid tool names.
///
/// The LLM can only request tools whose names appear in this registry.
/// This prevents spoofed or hallucinated tool names from entering the
/// execution gateway and becoming an implicit authorization bypass.
///
/// The registry does NOT own the executor logic — that lives in agent.rs.
/// It only validates that the tool name the LLM requested is a known,
/// registered capability of the system.

/// All tool names the agent is allowed to request.
/// Adding a new tool requires updating this list — by design.
pub static KNOWN_TOOLS: &[&str] = &[
    "TOOL_TERMINAL",
    "TOOL_PROGRAMMER",
    "TOOL_TESTER",
    "TOOL_FINISH",
    "TOOL_ENV_MANAGER",
    "TOOL_MAPPER",
    "TOOL_AST_INJECT",
    "TOOL_CONTAINER",
    "TOOL_WORKSPACE_MANAGER",
    "TOOL_BACKGROUND_START",
    "TOOL_BACKGROUND_QUERY",
    "TOOL_BROWSE",
    "TOOL_GIT",
];

pub struct ToolRegistry;

impl ToolRegistry {
    /// Returns true if the tool name is a registered, known capability.
    pub fn is_known(tool: &str) -> bool {
        KNOWN_TOOLS.contains(&tool)
    }

    /// Validates that the tool is registered.
    /// Returns Err with TOOL_UNKNOWN prefix so callers can surface it as a FATAL audit event.
    pub fn validate(tool: &str) -> Result<(), String> {
        if Self::is_known(tool) {
            Ok(())
        } else {
            Err(format!(
                "TOOL_UNKNOWN: '{}' no está registrado en ToolRegistry. El LLM intentó usar una herramienta inexistente.",
                tool
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_tools_are_valid() {
        for tool in KNOWN_TOOLS {
            assert!(ToolRegistry::is_known(tool), "KNOWN_TOOLS entry '{}' must be recognized", tool);
        }
    }

    #[test]
    fn test_unknown_tool_rejected() {
        let result = ToolRegistry::validate("TOOL_FAKE_EXPLOIT");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("TOOL_UNKNOWN"));
    }

    #[test]
    fn test_empty_tool_name_rejected() {
        let result = ToolRegistry::validate("");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("TOOL_UNKNOWN"));
    }
}
