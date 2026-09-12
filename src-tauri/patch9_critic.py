import sys
with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'r', encoding='utf-8') as f:
    lines = f.readlines()

for i, line in enumerate(lines):
    if 'let mut critic_fsm_lock_consecutive = 0u32;' in line:
        lines[i] = '// let mut critic_fsm_lock_consecutive = 0u32; removed for Commit 9\n'
    elif 'critic_fsm_lock_consecutive = 0;' in line:
        lines[i] = '// critic_fsm_lock_consecutive = 0;\n'
    elif 'critic_fsm_lock_consecutive += 1;' in line:
        lines[i] = '// critic_fsm_lock_consecutive += 1;\n'

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'w', encoding='utf-8') as f:
    f.writelines(lines)

print('SUCCESS')
