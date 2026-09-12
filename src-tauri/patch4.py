import sys, re

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\core\world_state.rs', 'r', encoding='utf-8') as f:
    text = f.read()

pattern = r'if path\.is_dir\(\) \{.*?if name == "node_modules" \|\| name == "\.git" \|\| name == "target" \|\| name == "__pycache__" \|\| name == "\.venv" \{.*?continue;.*?\}'

replacement = '''if path.is_dir() {
                if name == "node_modules" || name == ".git" || name == "target" || name == "__pycache__" || name == ".venv" || name == ".aura" {
                    continue;
                }'''

if not re.search(pattern, text, flags=re.DOTALL):
    print("NOT FOUND DIR")
    sys.exit(1)

text = re.sub(pattern, replacement, text, flags=re.DOTALL)


pattern2 = r'\} else if path\.is_file\(\) \{'
replacement2 = '''} else if path.is_file() {
                if name == ".aura_session.json" || name == ".aura_session.json.tmp" || name == ".aura_graph.json" || name == ".fenix_index.json" || name.starts_with(".aura") {
                    continue;
                }'''

if not re.search(pattern2, text):
    print("NOT FOUND FILE")
    sys.exit(1)

text = re.sub(pattern2, replacement2, text)


with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\core\world_state.rs', 'w', encoding='utf-8') as f:
    f.write(text)

print("SUCCESS")
