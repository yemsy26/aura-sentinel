import sys
with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'r', encoding='utf-8') as f:
    text = f.read()
idx = text.find('"TOOL_PROGRAMMER" => {')
if idx != -1:
    print('record_tool_call count:', text[idx:idx+15000].count('record_tool_call'))
    print('execute_action count:', text[idx:idx+15000].count('execute_action'))
