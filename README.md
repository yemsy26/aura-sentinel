# Aura-Sentinel 🛡️🧠
**Agente DevSecOps Autónomo y Resiliente**

Aura-Sentinel es un agente de Inteligencia Artificial diseñado para desarrollo de software continuo, control de calidad automatizado y resiliencia arquitectónica. Construido con **Tauri (Rust + Vanilla JS)**, Aura-Sentinel orquesta múltiples modelos de lenguaje locales (Ollama) para programar, auditar y proteger tu código sin depender de APIs de terceros en la nube.

---

## 🏗️ Arquitectura de DevSecOps Autónoma

Aura-Sentinel no solo escribe código; lo entiende, lo valida y lo protege a través de sistemas modulares críticos:

### 1. Auto-Healing & Pre-Flight Check (Escudo Ambiental) ✈️
Antes de que Aura gaste un solo token, el módulo de validación en Rust analiza el entorno. Verifica que Git, Node.js, Python, y Cargo estén en el PATH, confirma conectividad de red y asegura espacio en disco (>500MB). 
**Auto-Sanación**: Si durante el desarrollo falla una compilación porque falta una dependencia (`[ENV_FAILURE]`), Aura usará `TOOL_TERMINAL` para intentar instalar silenciosamente lo que necesita (ej. `winget install`, `npm install`) antes de pedir ayuda humana.

### 2. Git-Shield (Protección de Integridad) 🛡️
Cada vez que el programador de Aura propone un cambio físico, el sistema realiza automáticamente un `git commit` silencioso creando un punto de retorno seguro. Si el código falla estrepitosamente, Aura revierte los archivos (`git restore .`) impidiendo que tu proyecto se rompa.

### 3. QA Autónomo & Máquina de Estados (TOOL_TESTER) 🧪
*"Un código no probado es un código incompleto."* Aura invoca sus propias suites de pruebas (PyTest, Jest, go test). Si los tests fallan, Aura aplica el Rollback de Git-Shield y es obligada por una **Máquina de Estados Inquebrantable** a re-escribir el código usando `TOOL_PROGRAMMER` antes de poder testear de nuevo, previniendo bucles de alucinación. El límite máximo de intentos de rescate es 3.

### 4. Inteligencia Híbrida (Router Dinámico) 🧠☁️
Aura cuenta con un Router de Inteligencia que clasifica la complejidad de las tareas. Si el código falla las pruebas automatizadas, Aura escala la complejidad y pide ayuda a modelos pesados. Si tu PC no soporta modelos locales de gran tamaño (ej. 14B o 32B), Aura redirige el tráfico automáticamente a modelos expertos en la nube (ej. `qwen3.5:cloud`, `nemotron-3-super:cloud`) integrados a través de Ollama.

### 5. Memoria Vectorial Permanente (RAG) 🐘
Aura aprende de sus éxitos. Cuando un proyecto pasa todos los tests, el código fuente es vectorizado usando `nomic-embed-text` y almacenado permanentemente mediante un motor de Similitud de Coseno nativo en Rust puro. En futuras tareas, Aura escanea esta memoria e inyecta fragmentos relevantes en su contexto antes de programar, evitando reinventar la rueda.

### 6. Ingeniería Políglota 🌍
El sistema detecta automáticamente el contexto de tu entorno basándose en manifiestos (`Cargo.toml`, `go.mod`, `package.json`, etc.). Soporta de forma nativa flujos de prueba y compilación en **Rust, Go, TypeScript, C++, y Python**, adaptando sus comandos de terminal de forma dinámica sin hardcoding.

### 7. Path Jail (Seguridad Estricta) 🔒
Para evitar daños en el sistema operativo del usuario, el motor de Rust intercepta y audita cada intento de lectura/escritura (`is_path_allowed`). Si Aura intenta tocar cualquier archivo fuera de tu directorio de proyecto, la operación se bloquea inmediatamente bajo un error de `[SECURITY_VIOLATION]`.

---

## 🛠️ Guía de Instalación

### Requisitos Previos
Aura es completamente local. Necesitas instalar:
1. [Ollama](https://ollama.com/) (Para la inteligencia local).
2. [Rust / Cargo](https://rustup.rs/) (Para el backend resiliente).
3. [Node.js](https://nodejs.org/) (Para el frontend Tauri).
4. [Git](https://git-scm.com/) (Para Git-Shield).

### Modelos de Ollama Requeridos
Abre una terminal y descarga los cerebros de Aura:
```bash
ollama pull llama3.1:8b        # El Orquestador y Arquitecto
ollama pull qwen2.5-coder:7b   # El Ingeniero de Software
ollama pull nomic-embed-text   # El Motor de Memoria Vectorial
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

Aura está diseñada para conversaciones naturales basadas en objetivos.

1. **Pídele tareas completas:** *"Aura, crea un servidor backend con FastAPI y añade pruebas unitarias."*
2. **Deja que actúe:** Verás en la consola técnica cómo Aura planifica (LLaMA), escribe código (Qwen), hace backups (Git), prueba el código (Tester), y guarda el aprendizaje en su base de datos (RAG).
3. **Intervención Cero:** El sistema maneja errores de compilación, fallos lógicos y dependencias por ti.

### Scripts de Configuración (Portabilidad)
Si estás instalando Aura en un entorno nuevo, simplemente ejecuta los scripts de preparación:
- **Windows:** Ejecuta `.\setup.ps1` en PowerShell.
- **Mac/Linux:** Ejecuta `bash setup.sh` en tu terminal.

¡Bienvenido al futuro del desarrollo automatizado! 🚀

---

## ⚖️ Legal & Licensing

Este proyecto está distribuido bajo la [Licencia MIT](LICENSE).
Copyright (c) 2026 Ramon Antonio Burgos Jerez.
