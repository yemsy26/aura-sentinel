import sys, re

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'r', encoding='utf-8') as f:
    text = f.read()

pattern = r'if f_ext == \*ext_match \|\| requested_files\.len\(\) == 1 \{'
replacement = r'if f_ext == *ext_match {'

if not re.search(pattern, text):
    print("NOT FOUND")
    sys.exit(1)

text = re.sub(pattern, replacement, text)

with open(r'C:\Users\yemsy\.gemini\antigravity\scratch\aura sentinel\src-tauri\src\llm\agent.rs', 'w', encoding='utf-8') as f:
    f.write(text)

print("SUCCESS")
