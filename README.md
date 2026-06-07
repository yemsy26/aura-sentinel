# Aura-Sentinel 🛡️🧠
**Agente DevSecOps Autónomo y Resiliente**

Aura-Sentinel es un agente de Inteligencia Artificial diseñado para desarrollo de software continuo, control de calidad automatizado y resiliencia arquitectónica. Construido con **Tauri (Rust + Vanilla JS)**, Aura-Sentinel orquesta múltiples modelos de lenguaje locales (Ollama) para programar, auditar y proteger tu código sin depender de APIs de terceros en la nube.

---

## 🏗️ Arquitectura de DevSecOps Autónoma

Aura-Sentinel no solo escribe código; lo entiende, lo valida, asegura sus dependencias y se autoprotege a través de sistemas modulares críticos:

### 1. Ingeniería de Entornos (Módulo DevOps Autónomo) 🌍
Aura no necesita que prepares el entorno por ella. El backend en Rust intercepta fallos por dependencias faltantes (`[ENV_FAILURE]`) y actúa como un DevOps:
- **Auto-Instalación Silenciosa**: Detecta qué programa falta (ej. `go`, `node`, `python`) y lo instala en segundo plano mediante `scoop` o `winget`, sin necesidad de permisos de administrador invasivos.
- **Hot-Reloading (Recarga en Caliente)**: Tras instalar una dependencia, Rust lee el registro de Windows y recarga la variable `PATH` en memoria. Aura puede instalar un compilador en el Paso 1 y usarlo para testear tu código en el Paso 2, *sin que tengas que reiniciar la aplicación*.

### 2. Máquina de Estados y Kill-Switches (Protección Anti-Bucles) 🛑
Los agentes autónomos suelen atascarse en bucles infinitos. Aura posee barreras físicas (Kill-Switches) a nivel del núcleo de Rust:
- **Cooldown de Programador**: Si Aura edita un archivo 3 veces seguidas sin ejecutar las pruebas, Rust aborta la operación para prevenir destrucción de código ciego.
- **Terminal Estricta**: Intercepta comandos vacíos y comandos repetitivos en la terminal (ej. fallos en cascada de `winget`).
- **Escalada de 3 Niveles (NoTests)**: Si el agente intenta ejecutar tests en un proyecto vacío, el sistema escala inteligentemente: 1) Advierte al agente, 2) Fuerza la creación de archivos de test físicos, 3) Fuerza la instalación de frameworks de test en la terminal.
- **Bloqueo de "Éxito Infinito"**: Si una prueba tiene éxito, el sistema inyecta una regla estricta obligando al LLM a finalizar la misión, previniendo bucles donde el agente evalúa código ya exitoso indefinidamente.

### 3. Git-Shield (Protección de Integridad) 🛡️
Cada vez que el programador de Aura propone un cambio físico, el sistema realiza automáticamente un `git commit` silencioso creando un punto de retorno seguro. Si el código introducido falla estrepitosamente, Aura revierte los archivos (`git restore .`) impidiendo que el error corrompa la lógica de tu proyecto.

### 4. QA Autónomo (TOOL_TESTER & Auto-Debugger) 🧪
*"Un código no probado es un código incompleto."* Aura invoca de forma nativa sus propias suites de pruebas (PyTest, Jest, go test). 
- **Detección Inteligente de Errores**: El sistema distingue entre *errores de dependencias* (ej. falta `jest` o módulo no encontrado) y *errores de lógica* (tests fallidos). Si faltan dependencias, Aura ejecuta `npm install` / `pip install` automáticamente sin culpar al código. Si es un error de lógica, activa el Auto-Debugger.
- **Auto-Debugger**: Si las pruebas automatizadas fallan por lógica, la Máquina de Estados obliga a Aura a usar la herramienta `TOOL_PROGRAMMER` para arreglar el código; le está prohibido ignorar el error o volver a testear código roto sin antes editarlo.

### 5. Inteligencia Híbrida (Router Dinámico) 🧠☁️
Aura cuenta con un Router que clasifica la complejidad de las tareas en tiempo real. Si el código falla las pruebas automatizadas, Aura escala la complejidad y pide ayuda a modelos más pesados. Si tu hardware local se queda corto frente a bugs complejos, Aura puede redirigir el tráfico automáticamente a modelos expertos en la nube (ej. `qwen3.5:cloud`) integrados a través de Ollama.

### 6. Memoria Vectorial Permanente (RAG) 🐘
Aura aprende de sus éxitos. Cuando un proyecto pasa todos los tests, el código fuente es vectorizado usando `nomic-embed-text` y almacenado permanentemente mediante un motor de Similitud de Coseno nativo en Rust puro. En futuras tareas, Aura escanea esta memoria e inyecta fragmentos relevantes en su contexto antes de programar, evitando reinventar la rueda. Esta memoria histórica está **aislada y etiquetada** para que el LLM nunca la confunda con la misión actual del usuario.

### 7. Path Jail (Seguridad Estricta) 🔒
Para evitar daños en el sistema operativo del usuario, el motor de Rust intercepta y audita cada intento de lectura/escritura (`is_path_allowed`). Si Aura intenta tocar cualquier archivo fuera de tu directorio de proyecto, la operación se bloquea inmediatamente bajo un error de `[SECURITY_VIOLATION]`.

### 8. Gestión de Contexto Avanzada (Context Window Management) 🗜️
Al igual que los agentes de estado del arte (Cline, Continue), Aura protege a los LLMs de la "saturación de tokens":
- **Filtrado Profundo de Ruido**: Carpetas masivas como `node_modules`, `.git` o `target` son invisibles para el LLM y el motor vectorial, evitando colapsos de memoria.
- **Inyección Dinámica de Workspace**: En cada turno, Aura recibe un mapa exacto de los archivos que *realmente existen* en el proyecto. Solo lee el contenido completo si decide usar su herramienta `TOOL_AUDITOR`, ahorrando miles de tokens y previniendo alucinaciones.

---

## 🛠️ Guía de Instalación

### Requisitos Previos
Aura-Sentinel se ejecuta localmente. Necesitas instalar:
1. [Ollama](https://ollama.com/) (Para la inteligencia local).
2. [Rust / Cargo](https://rustup.rs/) (Para compilar el backend resiliente).
3. [Node.js](https://nodejs.org/) (Para el frontend).
4. [Git](https://git-scm.com/) (Para Git-Shield).

### Modelos de Ollama Requeridos
Abre una terminal y descarga los cerebros de Aura:
```bash
ollama pull llama3.1:8b        # El Orquestador y Planificador
ollama pull qwen2.5-coder:7b   # El Ingeniero de Software Base
ollama pull deepseek-coder:6.7b # Ingeniero Escalado
ollama pull nomic-embed-text   # Motor de Memoria Vectorial
```

### Ejecución
```bash
# Instala las dependencias del frontend
npm install

# Inicia la aplicación en modo desarrollo
npm run tauri dev
```

---

## 🤝 Filosofía de Uso

Aura está diseñada para conversaciones naturales basadas en objetivos y entrega de software robusto.

1. **Pídele tareas completas:** *"Aura, crea un servidor backend en Go, añade tests para las rutas y verifícalo."*
2. **Deja que actúe:** Verás en la consola técnica cómo Aura planifica (LLaMA), asegura el entorno (Rust DevOps), escribe código (Qwen), hace backups (Git), prueba el código (Tester), y guarda el aprendizaje (RAG).
3. **Intervención Cero:** El sistema maneja dependencias faltantes, errores de compilación y fallos lógicos por ti.

### Scripts de Configuración (Portabilidad)
Si estás instalando Aura en un entorno virgen, el sistema se encargará de configurar Scoop y las variables necesarias automáticamente, pero también cuentas con scripts de inicio manual:
- **Windows:** Ejecuta `.\setup.ps1` en PowerShell.
- **Mac/Linux:** Ejecuta `bash setup.sh` en tu terminal.

¡Bienvenido a la era del desarrollo verdaderamente autónomo! 🚀

---

## ⚖️ Legal & Licensing

Este proyecto está distribuido bajo la [Licencia MIT](LICENSE).
Copyright (c) 2026 Ramon Antonio Burgos Jerez.
