import sys, re

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'r', encoding='utf-8') as f:
    text = f.read()

# For checking
pattern_check = r'comandos_ejecutados_historico\.contains\(&comando\)'
replacement_check = r'comandos_ejecutados_historico.contains(&format!("{}|{}", comando.trim().to_lowercase(), runtime.current_world_hash()))'

if pattern_check not in text and not re.search(pattern_check, text):
    print("pattern_check NOT FOUND")
text = re.sub(pattern_check, replacement_check, text)

# For inserting
pattern_insert = r'comandos_ejecutados_historico\.insert\(comando\.clone\(\)\);'
replacement_insert = r'comandos_ejecutados_historico.insert(format!("{}|{}", comando.trim().to_lowercase(), runtime.current_world_hash()));'
if pattern_insert not in text and not re.search(pattern_insert, text):
    print("pattern_insert NOT FOUND")
text = re.sub(pattern_insert, replacement_insert, text)

# For checking bg task empty
pattern_bg_empty_check = r'comandos_ejecutados_historico\.contains\("__EMPTY_BG_CMD__"\)'
replacement_bg_empty_check = r'comandos_ejecutados_historico.contains(&format!("__EMPTY_BG_CMD__|{}", runtime.current_world_hash()))'
text = re.sub(pattern_bg_empty_check, replacement_bg_empty_check, text)

# For inserting bg empty
pattern_bg_empty_insert = r'comandos_ejecutados_historico\.insert\("__EMPTY_BG_CMD__"\.to_string\(\)\);'
replacement_bg_empty_insert = r'comandos_ejecutados_historico.insert(format!("__EMPTY_BG_CMD__|{}", runtime.current_world_hash()));'
text = re.sub(pattern_bg_empty_insert, replacement_bg_empty_insert, text)


with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'w', encoding='utf-8') as f:
    f.write(text)

print("SUCCESS")
