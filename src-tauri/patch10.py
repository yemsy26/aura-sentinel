import sys

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'r', encoding='utf-8') as f:
    lines = f.readlines()

for idx in [1954, 2134, 2166, 2186, 2268, 3059, 3361]:
    # 0-indexed
    i = idx - 1
    if 'runtime.record_tool_call();' in lines[i]:
        lines[i] = lines[i].replace('runtime.record_tool_call();', '// runtime.record_tool_call(); removed (handled by execute_action)')

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'w', encoding='utf-8') as f:
    f.writelines(lines)

print('SUCCESS')
