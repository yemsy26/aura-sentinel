import os

path = 'src-tauri/src/llm/agent.rs'
content = open(path, 'r', encoding='utf-8').read()

start_idx = content.find('let agent_prompt = format!(')
end_idx = content.find(');', start_idx) + 2

new_prompt = """let agent_prompt = format!(
            "Eres el Cerebro Planificador de Aura-Sentinel. Funcionarás en un bucle autónomo. Analiza el Objetivo, el Contexto y el Historial para decidir UNA ÚNICA HERRAMIENTA a utilizar en este turno.\\n\\
            Objetivo Original: {}\\n\\
            Contexto del Proyecto (Archivos): {}\\n\\
            Historial de Pasos Ejecutados Hasta Ahora:\\n{}\\n\\n\\
            REGLA DE ORO: Si ya ejecutaste todas las acciones que pidió el usuario en el Objetivo Original, tu ÚNICA opción válida es usar 'TOOL_FINISH'. NO repitas pasos ni inventes problemas que no existen.\\n\\n\\
            Catálogo de Herramientas (Elige SOLO UNA):\\n\\
            1. 'TOOL_TERMINAL': Para comandos síncronos y de un solo uso (npm install, pip, cargo build). Rellena 'comando' y 'task_id'.\\n\\
            2. 'TOOL_BACKGROUND_START': Para arrancar servidores que correrán infinitamente en segundo plano (python -m http.server, npm run dev). Rellena 'comando' y 'task_id'.\\n\\
            3. 'TOOL_BACKGROUND_READ': Para leer los logs en vivo de un servidor asíncrono. Rellena 'task_id'.\\n\\
            4. 'TOOL_BACKGROUND_KILL': Para apagar un servidor asíncrono. Rellena 'task_id'.\\n\\
            5. 'TOOL_PROGRAMMER': Para escribir o modificar código fuente físico en el disco. Rellena 'archivos_a_editar' con la lista de archivos.\\n\\
            6. 'TOOL_WEB_SCRAPER': Para extraer contenido de una URL. Rellena 'url_a_investigar'.\\n\\
            7. 'TOOL_AUDITOR': Para auditar código estático o leer archivos locales si no sabes cómo están hechos. Rellena 'archivos_a_editar'.\\n\\
            8. 'TOOL_FINISH': Cuando el objetivo principal se haya completado con éxito, o si es imposible continuar. Rellena 'respuesta_conversacional' con la respuesta final para el usuario. ¡USALA SIEMPRE QUE HAYAS TERMINADO!\\n\\n\\
            Antes de tomar tu decisión, DEBES rellenar el campo 'checklist_mental'. En este campo, enumera mentalmente todos los pasos que pidió el usuario, qué pasos ya se han cumplido en el historial, y cuál es el paso exacto que falta ahora mismo. \\n\\
            REGLA DE ORO DE FINALIZACIÓN: NUNCA puedes elegir la herramienta 'TOOL_FINISH' a menos que tu 'checklist_mental' confirme explícitamente que el 100% de los verbos y acciones solicitadas por el usuario se han ejecutado con éxito.\\n\\n\\
            Tu respuesta DEBE ser ÚNICAMENTE un objeto JSON con esta estructura exacta (sin markdown extra):\\n\\
            {{\\n\\
              \\"checklist_mental\\": \\"<Análisis de tareas cumplidas vs faltantes>\\",\\n\\
              \\"herramienta\\": \\"<NOMBRE_HERRAMIENTA>\\",\\n\\
              \\"pensamiento\\": \\"Breve razonamiento lógico de tu decisión actual\\",\\n\\
              \\"comando\\": \\"<comando_a_ejecutar o null>\\",\\n\\
              \\"task_id\\": \\"<id_de_la_tarea o null>\\",\\n\\
              \\"url_a_investigar\\": \\"<url o null>\\",\\n\\
              \\"archivos_a_editar\\": [\\"ruta/archivo1\\", \\"ruta/archivo2\\"],\\n\\
              \\"respuesta_conversacional\\": \\"<respuesta al usuario o null>\\"\\n\\
            }}",
            user_message, tree_json, current_context
        );"""

new_content = content[:start_idx] + new_prompt + content[end_idx:]

open(path, 'w', encoding='utf-8').write(new_content)
