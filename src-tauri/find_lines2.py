import sys
with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'r', encoding='utf-8') as f:
    for i, line in enumerate(f):
        if 'record_tool_call' in line:
            print(f"{i+1}: {line.strip().encode('ascii', 'ignore').decode('ascii')}")
