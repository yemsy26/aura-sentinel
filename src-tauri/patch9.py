import sys

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'r', encoding='utf-8') as f:
    text = f.read()

start_idx = text.find('        let pending_metas: Vec<String> = journal.micro_metas')
if start_idx == -1:
    print('NOT FOUND START')
    sys.exit(1)
end_idx = text.find('        current_context = context_monitor.compact_context(&current_context, &mission_state);', start_idx) + len('        current_context = context_monitor.compact_context(&current_context, &mission_state);')

replacement = '''        let mission_state = runtime.format_anchor(&journal, &format!("{:?}", current_role), &last_programmer_error);
        current_context.push_str("\\n\\n");
        current_context.push_str(&mission_state);
        current_context = context_monitor.compact_context(&current_context, &mission_state);'''

text = text[:start_idx] + replacement + text[end_idx:]

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'w', encoding='utf-8') as f:
    f.write(text)

print('SUCCESS')
