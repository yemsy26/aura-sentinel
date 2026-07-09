import sys
import re

with open('src/llm/agent.rs', 'r', encoding='utf-8') as f:
    content = f.read()

# 1. Inject AgentContext right before `fn classify_mission`
agent_context_code = """
#[derive(Clone, Debug)]
pub struct AgentContext {
    pub workspace_state: String,
    pub critical_alerts: Vec<String>,
    pub current_step: String,
    pub history: std::collections::VecDeque<String>,
}

impl AgentContext {
    pub fn new() -> Self {
        Self {
            workspace_state: String::new(),
            critical_alerts: Vec::new(),
            current_step: String::new(),
            history: std::collections::VecDeque::new(),
        }
    }
    pub fn push_str(&mut self, s: &str) {
        self.current_step.push_str(s);
    }
    pub fn push_alert(&mut self, s: &str) {
        self.critical_alerts.push(s.to_string());
    }
    pub fn set_workspace_state(&mut self, s: String) {
        self.workspace_state = s;
    }
    pub fn commit_step(&mut self) {
        if !self.current_step.trim().is_empty() {
            if self.history.len() >= 5 {
                self.history.pop_front();
            }
            self.history.push_back(self.current_step.clone());
            self.current_step.clear();
        }
    }
    pub fn render_to_prompt(&self) -> String {
        let mut out = String::new();
        if !self.workspace_state.is_empty() {
            out.push_str(&self.workspace_state);
            out.push_str("\\n\\n");
        }
        if !self.critical_alerts.is_empty() {
            out.push_str("--- ALERTAS CRÍTICAS DEL SISTEMA (PERSISTENTES) ---\\n");
            for a in &self.critical_alerts {
                out.push_str(a);
                out.push_str("\\n");
            }
            out.push_str("\\n");
        }
        out.push_str("--- HISTORIAL RECIENTE ---\\n");
        for h in &self.history {
            out.push_str(h);
            out.push_str("\\n");
        }
        out.push_str(&self.current_step);
        out
    }
    pub fn sanitize_stale_markers(&mut self) {
        let stale_markers = [
            "proxy-stack-windows",
            "proxy-stack",
            "\\\\proxy-",
            "/proxy-",
        ];
        for h in self.history.iter_mut() {
            for marker in &stale_markers {
                if h.contains(marker) {
                    *h = h.lines().filter(|line| !line.contains(marker)).collect::<Vec<_>>().join("\\n");
                }
            }
        }
        for marker in &stale_markers {
            if self.current_step.contains(marker) {
                self.current_step = self.current_step.lines().filter(|line| !line.contains(marker)).collect::<Vec<_>>().join("\\n");
            }
        }
    }
}
"""
content = content.replace("fn classify_mission(msg: &str) -> MissionType {", agent_context_code + "\nfn classify_mission(msg: &str) -> MissionType {")

# 2. Change initialization
content = content.replace("let mut current_context = String::new();", "let mut current_context = AgentContext::new();")

# 3. Handle workspace state push instead of push_str
ws_state_push = """current_context.push_str(&format!(
                "[ESTADO ACTUAL DEL WORKSPACE] Los siguientes archivos YA EXISTEN en el proyecto. \\
                Antes de crear nada, verifica si estos archivos ya cumplen el objetivo:\\n{}\\n\\n",
                existing_files.join("\\n")
            ));"""
ws_state_new = """current_context.set_workspace_state(format!(
                "[ESTADO ACTUAL DEL WORKSPACE] Los siguientes archivos YA EXISTEN en el proyecto. \\
                Antes de crear nada, verifica si estos archivos ya cumplen el objetivo:\\n{}",
                existing_files.join("\\n")
            ));"""
content = content.replace(ws_state_push, ws_state_new)

# 4. Handle pesp_status formatting safely
pesp_start = content.find("// Prepend to context so it's always at the top")
if pesp_start != -1:
    pesp_end = content.find("}", pesp_start) + 1
    # Replace the chunk
    old_pesp = content[pesp_start:pesp_end]
    new_pesp = 'current_context.set_workspace_state(format!("{}\\n{}", pesp_status, current_context.workspace_state));\n        }'
    content = content.replace(old_pesp, new_pesp)

# 5. Remove Memory Compression Block
mem_comp_start = content.find("// 🧠 Context Compression: every 10 steps")
if mem_comp_start == -1:
    # Try finding without emoji in case of encoding issues
    mem_comp_start = content.find("Context Compression: every 10 steps")
    if mem_comp_start != -1:
        mem_comp_start = content.rfind("//", 0, mem_comp_start)

if mem_comp_start != -1:
    mem_comp_end = content.find("let mut forced_override", mem_comp_start)
    if mem_comp_end != -1:
        content = content[:mem_comp_start] + content[mem_comp_end:]

# 6. Remove offset truncation block
truncate_start = content.find("// --- EVITAR DESBORDAMIENTO DE CONTEXTO ---")
if truncate_start != -1:
    truncate_end = content.find("}", content.find("}", content.find("}", truncate_start) + 1) + 1) + 1
    content = content[:truncate_start] + content[truncate_end:]

# 7. agent_prompt format to use render_to_prompt()
content = content.replace("extra_prompt, current_context,", "extra_prompt, current_context.render_to_prompt(),")

# 8. Update the Sanitizer
sanitizer_start = content.find("current_context = {")
if sanitizer_start != -1:
    sanitizer_end = content.find("};", sanitizer_start) + 2
    content = content[:sanitizer_start] + "current_context.sanitize_stale_markers();" + content[sanitizer_end:]

# 9. Add commit_step at the start of the loop
loop_start = "while step_count <= max_steps {"
loop_start_new = "while step_count <= max_steps {\n        current_context.commit_step();"
content = content.replace(loop_start, loop_start_new)

with open('src/llm/agent.rs', 'w', encoding='utf-8') as f:
    f.write(content)

print("AgentContext injected successfully!")
