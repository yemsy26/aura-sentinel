import os

path = "src-tauri/src/core/architect.rs"
content = open(path, "r", encoding="utf-8").read()

content = content.replace(
    'let js_import_re = Regex::new(r"import\\s+.*?from\\s+[\'\\""](.+?)[\'\\""]").unwrap();',
    'let js_import_re = Regex::new(r#"import\\s+.*?from\\s+[\'"](.+?)[\'"]"#).unwrap();'
)
content = content.replace(
    'let js_require_re = Regex::new(r"require\\([\'\\""](.+?)[\'\\""]\\)").unwrap();',
    'let js_require_re = Regex::new(r#"require\\([\'"](.+?)[\'"]\\)"#).unwrap();'
)
content = content.replace(
    'let js_dynamic_re = Regex::new(r"import\\([^\\'\\""]").unwrap();',
    'let js_dynamic_re = Regex::new(r#"import\\([^\\\'"]"#).unwrap();'
)

# wait, I should just replace them explicitly

lines = content.splitlines()
for i, line in enumerate(lines):
    if "let js_import_re =" in line:
        lines[i] = '    let js_import_re = Regex::new(r#"import\\s+.*?from\\s+[\'"](.+?)[\'"]"#).unwrap();'
    elif "let js_require_re =" in line:
        lines[i] = '    let js_require_re = Regex::new(r#"require\\([\'"](.+?)[\'"]\\)"#).unwrap();'
    elif "let js_dynamic_re =" in line:
        lines[i] = '    let js_dynamic_re = Regex::new(r#"import\\([^\\\'"]"#).unwrap();'

open(path, "w", encoding="utf-8").write("\\n".join(lines))
