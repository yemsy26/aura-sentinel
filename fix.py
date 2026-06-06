import os

path = 'src-tauri/src/llm/agent.rs'
content = open(path, 'r', encoding='utf-8').read()

content = content.replace("""            },
                }
            "TOOL_FINISH\"""", """                }
            },
            "TOOL_FINISH\"""")

open(path, 'w', encoding='utf-8').write(content)
