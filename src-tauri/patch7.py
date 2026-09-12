import sys

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'r', encoding='utf-8') as f:
    text = f.read()

start_idx = text.find('if is_continuation_command && !journal.objetivo.is_empty() {')
if start_idx == -1:
    print('NOT FOUND START')
    sys.exit(1)

end_idx = text.find('    }\n', start_idx) + 6

replacement = '''    if is_continuation_command && !journal.objetivo.is_empty() {
        // Retain original mission objective and existing phases!
        journal = crate::core::session_journal::resume_existing_mission(&workspace_path);
        user_message = journal.objetivo.clone();
        original_prompt_parsed = journal.objetivo.clone();
        journal.interrupted = true; // Signals restoration block below
    } else {
        // Any new user prompt (not an explicit continuation) starts a completely clean session
        journal = crate::core::session_journal::start_new_mission(&workspace_path, &user_message);
    }\n'''

text = text[:start_idx] + replacement + text[end_idx:]

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'w', encoding='utf-8') as f:
    f.write(text)

print('SUCCESS')
