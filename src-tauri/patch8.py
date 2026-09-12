import sys

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\core\mission_runtime.rs', 'r', encoding='utf-8') as f:
    text = f.read()

replacement = '''
    pub fn format_anchor(&self, journal: &crate::core::session_journal::SessionJournal, current_role: &str, last_error: &str) -> String {
        let pending_metas: Vec<String> = journal.micro_metas.iter()
            .filter(|m| m.estado != "VERIFICADA")
            .map(|m| m.descripcion.clone())
            .collect();
        let current_meta = if journal.micro_meta_actual < journal.micro_metas.len() {
            journal.micro_metas[journal.micro_meta_actual].descripcion.clone()
        } else {
            "Ninguna".to_string()
        };

        format!(
            "[MISSION_ANCHOR]\\nWorkspace: {}\\nFase: {:?}\\nRol: {:?}\\nArchivos pendientes: {}\\nMeta actual: {}\\nÚltimo error crítico: {}\\n[/MISSION_ANCHOR]",
            self.workspace_path,
            journal.ultimo_estado,
            current_role,
            if pending_metas.is_empty() { "Ninguno".to_string() } else { pending_metas.join(", ") },
            current_meta,
            if last_error.is_empty() { "Ninguno".to_string() } else { last_error.to_string() }
        )
    }
}
'''

last_brace_idx = text.rfind('}')
if last_brace_idx == -1:
    print('NO BRACE')
    sys.exit(1)

text = text[:last_brace_idx] + replacement

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\core\mission_runtime.rs', 'w', encoding='utf-8') as f:
    f.write(text)

print('SUCCESS')
