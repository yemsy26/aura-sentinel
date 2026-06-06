# Aura-Sentinel ⚡

**An autonomous, general-purpose software creation agent.**

Aura-Sentinel is a powerful, autonomous AI agent designed to write, compile, run, and self-heal applications entirely from scratch. Unlike traditional coding assistants, Aura acts as a full DevSecOps engineer. It can create complex web architectures, deploy to the cloud, read real-time asynchronous logs, and repair its own code if a build fails.

Built with **Rust** (Tauri) and **Ollama** (Local LLMs), Aura ensures zero privacy leaks while orchestrating local or remote systems.

---

## 🏗️ 4 Core Architectural Pillars

Aura-Sentinel stands on four foundational systems that grant it true autonomy (The "Antigravity" Level):

### 1. ReAct Agentic Loop 🧠
The orchestrator model (e.g., Llama 3) uses a strict Reason + Act (ReAct) loop. Before taking any physical action, it evaluates its environment and updates its **Mental Checklist**. It autonomously decides whether to run a terminal command, edit physical files, start background servers, or wait for logs.

### 2. Asynchronous Background Task Engine ⚙️
Aura doesn't just run linear scripts. It can spawn background jobs (like `npm run dev` or a Python server) without blocking its main thread. It can then poll the STDOUT/STDERR of these processes in real time to verify if the server started correctly or crashed.

### 3. Git-Shield Auto-Rollback 🛡️
Before performing any risky code mutation on your disk, Aura takes a snapshot of your workspace via Git. If a change causes catastrophic failure, the workspace is shielded.

### 4. Self-Healing Compilation Loop 🛠️
When Aura modifies code using its Programmer sub-agent (e.g., Qwen 2.5 Coder), it automatically triggers native validation (like `cargo check` or `python compileall`). If the compiler yells, Aura reads the exact stack trace and rewrites the code to fix the syntax error autonomously.

---

## 🚀 Getting Started

### Prerequisites
To unleash Aura-Sentinel locally, you will need:
- **Git**: Installed and available in PATH.
- **Node.js**: Installed (for web scaffolding).
- **Python 3**: Installed (for backend scripts).
- **Rust & Tauri CLI**: For building the native agent.
- **Ollama**: Running locally with the following models downloaded:
  - `ollama run llama3.1:8b` (The Orchestrator)
  - `ollama run qwen2.5-coder:7b` (The Programmer)

### Installation
1. Clone the repository.
2. Run `npm install` inside the project root to install the Tauri UI dependencies.
3. Start the agent:
   ```bash
   npm run tauri dev
   ```

### Usage
Open the UI and simply command Aura. Examples:
- *"Create a Python backend that listens on port 8080 and returns a JSON response. Start the server."*
- *"Initialize a Vite web project, add a stunning CSS dark-mode dashboard, and deploy it to Firebase."*
- *"Run `npm run build`. If it fails, read the logs and fix the source code."*

---

## 🌎 Español

**Un agente autónomo de creación de software de propósito general.**

Aura-Sentinel es un agente de IA diseñado para escribir, compilar, ejecutar y auto-reparar aplicaciones completamente desde cero. A diferencia de los asistentes tradicionales, Aura actúa como un ingeniero DevSecOps completo. Puede crear arquitecturas web, desplegar en la nube, leer logs asíncronos y reparar su propio código si la compilación falla.

Desarrollado con **Rust** (Tauri) y **Ollama** (LLMs locales), Aura garantiza privacidad absoluta mientras orquesta sistemas locales.

### 🏗️ Los 4 Pilares Arquitectónicos

1. **Bucle ReAct (Agente) 🧠**: Evalúa el entorno, actualiza su *Checklist Mental* y decide qué herramienta nativa usar (Terminal, Programador, Monitor, etc.).
2. **Motor de Tareas Asíncronas ⚙️**: Permite arrancar servidores (`npm run dev`) en segundo plano y monitorear sus logs (STDOUT/STDERR) en tiempo real sin bloquear el sistema.
3. **Escudo Git (Auto-Rollback) 🛡️**: Realiza copias de seguridad instantáneas del código antes de modificar los archivos físicos.
4. **Bucle de Auto-Sanación 🛠️**: Valida el código compilándolo nativamente (`cargo check`, `compileall`). Si hay errores, Aura lee la traza y lo repara por sí mismo.

### Prerrequisitos
- Git, Node.js y Python 3 instalados.
- Entorno de Rust y Tauri.
- **Ollama** instalado con los modelos: `llama3.1:8b` y `qwen2.5-coder:7b`.

*Construido para alcanzar el nivel "Antigravity".*
