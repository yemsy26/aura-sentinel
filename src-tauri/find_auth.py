import sys

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'r', encoding='utf-8') as f:
    for i, line in enumerate(f):
        if 'authorize_action' in line:
            print(f"Line {i+1}: {line.strip().encode('ascii', 'ignore').decode('ascii')}")
