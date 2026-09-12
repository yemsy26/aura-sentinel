import sys

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'r', encoding='utf-8') as f:
    text = f.read()

start_idx = text.find('if let Err(auth_err) = runtime.authorize_action(&action_proposal) {')
if start_idx == -1:
    print('NOT FOUND START')
    sys.exit(1)

end_idx = text.find('        }\n\n        match tool.as_str() {', start_idx)

if end_idx == -1:
    print('NOT FOUND END')
    sys.exit(1)

replacement = '// authorize_action removed (handled by execute_action)'

text = text[:start_idx] + replacement + text[end_idx:]

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'w', encoding='utf-8') as f:
    f.write(text)

print('SUCCESS')
