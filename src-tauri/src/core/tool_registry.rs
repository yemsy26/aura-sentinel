/// FINAL-6: ToolRegistry — Real dispatch authority for tool execution.
///
/// Upgraded from FINAL-2 (name validator) to FINAL-6 (executor registry).
/// The LLM delivers an ActionProposal. The Runtime resolves and dispatches
/// the executor via this registry. agent.rs NEVER decides which code runs
/// for a given tool name — ToolRegistry is the sole dispatch authority.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// The concrete return type of any tool execution.
pub type ToolResult = Result<String, String>;

/// A boxed async future that produces a ToolResult.
pub type BoxFuture = Pin<Box<dyn Future<Output = ToolResult> + Send>>;

/// The type of a registered executor function.
/// Takes the tool arguments (from ActionProposal.arguments) and returns a BoxFuture.
pub type ExecutorFn = Arc<dyn Fn(serde_json::Value) -> BoxFuture + Send + Sync>;

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
    "TOOL_THINK",
    "TOOL_AUDITOR",
    "TOOL_VISION_EVALUATOR",
    "TOOL_ASK_USER",
    "TOOL_READ_FILE",
    "TOOL_BACKGROUND_READ",
    "TOOL_BACKGROUND_KILL",
    "TOOL_ASSET_MANAGER",
    "TOOL_WEB_SCRAPER",
    "TOOL_LEARN",
];

/// ToolRegistry owns the dispatch table: tool_name → ExecutorFn.
/// Instantiated inside MissionRuntime. agent.rs registers executors once,
/// before the mission loop — not inline inside match arms.
pub struct ToolRegistry {
    executors: HashMap<String, ExecutorFn>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { executors: HashMap::new() }
    }

    /// Returns true if the tool name is in the static KNOWN_TOOLS list.
    pub fn is_known(tool: &str) -> bool {
        KNOWN_TOOLS.contains(&tool)
    }

    /// Validates that the tool name is known.
    /// Used by authorize_action() as the first gate.
    pub fn validate_name(tool: &str) -> Result<(), String> {
        if Self::is_known(tool) {
            Ok(())
        } else {
            Err(format!(
                "TOOL_UNKNOWN: '{}' no está registrado en ToolRegistry. \
                 El LLM intentó usar una herramienta inexistente.",
                tool
            ))
        }
    }

    /// Registers the real executor for a known tool.
    /// Called ONCE by agent.rs before the mission loop starts.
    /// Returns Err if the tool name is not in KNOWN_TOOLS.
    pub fn register(&mut self, tool: &str, executor: ExecutorFn) -> Result<(), String> {
        Self::validate_name(tool)?;
        self.executors.insert(tool.to_string(), executor);
        Ok(())
    }

    /// Resolves a tool name to its registered executor.
    /// Returns None if the tool is known but has no executor registered yet.
    pub fn resolve(&self, tool: &str) -> Option<&ExecutorFn> {
        self.executors.get(tool)
    }

    /// Dispatches the registered executor for the given tool.
    /// Returns TOOL_UNREGISTERED if the tool has no executor (known but not registered).
    pub async fn dispatch(&self, tool: &str, args: serde_json::Value) -> ToolResult {
        match self.resolve(tool) {
            Some(executor) => executor(args).await,
            None => Err(format!(
                "TOOL_UNREGISTERED: '{}' is known but has no executor registered. \
                 Call runtime.tool_registry.register() before starting the mission loop.",
                tool
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_tools_are_valid() {
        for tool in KNOWN_TOOLS {
            assert!(ToolRegistry::is_known(tool),
                "KNOWN_TOOLS entry '{}' must be recognized by is_known()", tool);
        }
    }

    #[test]
    fn test_unknown_tool_rejected() {
        let result = ToolRegistry::validate_name("TOOL_FAKE_EXPLOIT");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("TOOL_UNKNOWN"));
    }

    #[test]
    fn test_empty_tool_name_rejected() {
        let result = ToolRegistry::validate_name("");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("TOOL_UNKNOWN"));
    }

    #[test]
    fn test_register_known_tool_succeeds() {
        let mut registry = ToolRegistry::new();
        let result = registry.register("TOOL_TERMINAL", Arc::new(|_args| {
            Box::pin(async { Ok("ok".to_string()) })
        }));
        assert!(result.is_ok());
    }

    #[test]
    fn test_register_unknown_tool_fails() {
        let mut registry = ToolRegistry::new();
        let result = registry.register("TOOL_INVENTED", Arc::new(|_args| {
            Box::pin(async { Ok("ok".to_string()) })
        }));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("TOOL_UNKNOWN"));
    }

    #[tokio::test]
    async fn test_dispatch_registered_tool() {
        let mut registry = ToolRegistry::new();
        registry.register("TOOL_TERMINAL", Arc::new(|_args| {
            Box::pin(async { Ok("dispatched".to_string()) })
        })).unwrap();
        let result = registry.dispatch("TOOL_TERMINAL", serde_json::Value::Null).await;
        assert_eq!(result, Ok("dispatched".to_string()));
    }

    #[tokio::test]
    async fn test_dispatch_unregistered_known_tool_fails() {
        let registry = ToolRegistry::new(); // nothing registered
        let result = registry.dispatch("TOOL_TERMINAL", serde_json::Value::Null).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("TOOL_UNREGISTERED"));
    }
}
