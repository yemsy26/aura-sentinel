import sys
with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'r', encoding='utf-8') as f:
    lines = f.readlines()

start_del = -1
end_del = -1

for i, line in enumerate(lines):
    if 'think_programmer_alternation_count += 1;' in line:
        start_del = i - 1
        break

if start_del != -1:
    for i in range(start_del, len(lines)):
        if 'think_programmer_alternation_count = 0; // reset' in line or 'think_programmer_alternation_count = 0; // reset after intervention' in lines[i]:
            end_del = i + 1
            break

if start_del != -1 and end_del != -1:
    lines[start_del:end_del] = ['        // 🛡️ Commit 9: Alternation lock delegated to RecoveryEngine 🛡️\n']

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'w', encoding='utf-8') as f:
    f.writelines(lines)

print('SUCCESS')
