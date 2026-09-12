import sys

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'r', encoding='utf-8') as f:
    text = f.read()

start_idx = text.find(' Context Window Tiered Monitor & Intelligent Compaction (Devin 2.0 / OSS 2025 Pattern) ')
if start_idx == -1:
    print('NOT FOUND START 1')
    sys.exit(1)
start_idx = text.rfind('        // ', 0, start_idx)

end_idx = text.find('} else if ctx_status == crate::core::context_monitor::ContextStatus::ApproachingLimit {', start_idx)
if end_idx == -1:
    print('NOT FOUND END 1')
    sys.exit(1)

replacement = '''        // 🛡️ Context Window Tiered Monitor & Intelligent Compaction (Devin 2.0 / OSS 2025 Pattern) 🛡️
        let (fill_pct, ctx_status) = context_monitor.status(current_context.len());
        if context_monitor.should_compact(current_context.len()) {
            emit_event(&app_handle, runtime.current_step(), &format!("[MEMORIA] Compactando ventana de contexto ({:.0}% uso) preservando Objetivo Inmutable...", fill_pct * 100.0), "INFO");
            let mission_state = runtime.format_anchor(&journal, &format!("{:?}", current_role), &last_programmer_error);
            current_context = context_monitor.compact_context(&current_context, &mission_state);
            emit_event(&app_handle, runtime.current_step(), "[MEMORIA] Contexto compactado exitosamente sin pérdida del objetivo.", "SUCCESS");
        '''

text = text[:start_idx] + replacement + text[end_idx:]


# Now inject final_context
start_idx_prompt = text.find('        let agent_prompt = match current_role {')
if start_idx_prompt == -1:
    print('NOT FOUND PROMPT')
    sys.exit(1)

text = text[:start_idx_prompt] + '''        let final_context = {
            let ms = runtime.format_anchor(&journal, &format!("{:?}", current_role), &last_programmer_error);
            format!("{}\\n\\n{}", current_context, ms)
        };\n\n''' + text[start_idx_prompt:]


# Now replace current_context with final_context in format args
text = text.replace('extra_prompt, current_context,', 'extra_prompt, final_context,')

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'w', encoding='utf-8') as f:
    f.write(text)

print('SUCCESS')
