import re

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\core\mod.rs', 'r', encoding='utf-8') as f:
    lines = f.readlines()

with open('dump_out.txt', 'w', encoding='utf-8') as fw:
    in_func = False
    for line in lines:
        if 'pub async fn execute_terminal_command' in line:
            in_func = True
        if in_func:
            fw.write(line)
        if in_func and 'Err(' in line and 'status.success()' in line:
            pass
        if in_func and '}' in line and line.startswith('}'):
            break
