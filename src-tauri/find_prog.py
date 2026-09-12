import sys
with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'r', encoding='utf-8') as f:
    text = f.read()
idx = text.find('"TOOL_PROGRAMMER" => {')
if idx != -1:
    print(text[idx:idx+1500].encode('ascii', 'ignore').decode('ascii'))
