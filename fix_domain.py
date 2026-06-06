import os

path = 'src-tauri/src/llm/agent.rs'
content = open(path, 'r', encoding='utf-8').read()

target = "REGLA DE ORO DE FINALIZACIÓN: NUNCA puedes elegir la herramienta 'TOOL_FINISH' a menos que tu 'checklist_mental' confirme explícitamente que el 100% de los verbos y acciones solicitadas por el usuario se han ejecutado con éxito.\\n\\n\\"

injection = "MANUAL DE OPERACIONES ANTIGRAVITY (DOMAIN KNOWLEDGE):\\n\\\n            - Scaffolding Frontend: Si el usuario pide crear una web desde cero, usa 'TOOL_TERMINAL' con 'npx -y create-vite@latest . --template vanilla' (o similar) en lugar de intentar escribir archivos manualmente.\\n\\\n            - Backend Rápidos: Si piden un servidor, crea el código físico con 'TOOL_PROGRAMMER' y luego levántalo con 'TOOL_BACKGROUND_START'.\\n\\\n            - Firebase Deploy: Si piden desplegar a producción/Firebase, asume que 'firebase-tools' está instalado y usa 'TOOL_TERMINAL' con 'firebase init hosting' o 'firebase deploy --only hosting'. Asegúrate de compilar antes si es necesario (ej. 'npm run build').\\n\\\n            - Resolución de Errores: Si un comando falla, lee los logs o la consola, usa 'TOOL_PROGRAMMER' para arreglar el código, y vuelve a intentar.\\n\\n\\"

if target in content:
    new_content = content.replace(target, target + "\n            " + injection)
    open(path, 'w', encoding='utf-8').write(new_content)
    print("Injected Domain Knowledge.")
else:
    print("Target not found.")
