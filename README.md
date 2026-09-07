# Aura-Sentinel 🛡️🧠
**Agente Autónomo de Ingeniería de Software DevSecOps de Nivel Industrial**  
*Desarrollado por Ramón Antonio Burgos Jerez*

> 🏆 **Nivel Alcanzado: Agente Autónomo Completo — Tier 0 (v3.0.0 — Septiembre 2026)**  
> Arquitectura de autonomía completa y ejecución determinista. Estándar de la industria comparable a SWE-agent y Devin 2.0.

Aura-Sentinel es un agente de Inteligencia Artificial de alto rendimiento diseñado para el ciclo de vida completo de desarrollo de software: planificación de arquitecturas, generación modular de código, verificación estricta de compilación y ejecución de suites de pruebas con auto-reparación en bucle cerrado. Construido sobre un núcleo nativo en **Rust (Tauri)** acoplado a una interfaz moderna con **Monaco Editor**, orquesta modelos locales (Ollama) sin depender de APIs de terceros en la nube, garantizando máxima privacidad y latencia cero.

---

## 🏆 Estado del Sistema (v3.0.0 Enterprise)

| Módulo Subyacente | Estado | Nivel / Tier |
|---|---|---|
| **FSM Multi-Agente (Planificador → Ejecutor → Crítico)** | 🟢 100% Operativo | Tier 0 (Industrial) |
| **Unified Brain Selector & Turbo Mode (`⚡ MÁXIMA POTENCIA`)** | 🟢 100% Operativo | Tier 0 (Industrial) |
| **Dynamic Phase Gatekeeper (PESP v2 & Smart Alias)** | 🟢 100% Operativo | Tier 0 (Industrial) |
| **AutoValidator Subsystem (`core::auto_validator`)** | 🟢 100% Conectado | Tier 0 (Industrial) |
| **SpectraSAT FFI — Motor Lógico Booleano en RAM** | 🟢 100% Operativo | Tier 0 (Determinista) |
| **Zero-Hint Router (Enrutamiento Semántico Autónomo)** | 🟢 100% Operativo | Tier 0 (Industrial) |
| **Mission Persistence (Persistencia Cross-Restart)** | 🟢 100% Operativo | Tier 0 (Resiliente) |
| **Sanity Monitor (Anti-Stall & Recuperación Activa)** | 🟢 100% Operativo | Tier 0 (Industrial) |
| **Monaco Editor & Tarjetas de Artefactos Interactivas** | 🟢 100% Operativo | Tier 0 (UI/UX) |
| **TOOL_CONTAINER (Docker & Podman Nativo)** | 🟢 100% Operativo | Tier 0 (DevOps) |
| **Scheduler Autónomo (Cron Tokio Asíncrono)** | 🟢 100% Operativo | Tier 0 (Automatización) |
| **Memoria Episódica Multi-Sesión (JSONL Vectorial)** | 🟢 100% Operativo | Tier 0 (Cognición) |
| **Git-Shield & Auto-Sanación sin Amnesia** | 🟢 100% Operativo | Tier 0 (Resiliente) |
| **Compresión Dinámica de Contexto & Sanitizer** | 🟢 100% Operativo | Tier 0 (Optimizado) |

---

## 🏛️ Arquitectura del Motor Autónomo

### 1. FSM Multi-Agente Cognitiva (Planificador → Ejecutor → Crítico)
- **Planificador (Planner / Zero-Hint Router)**: Inicia la misión abstrayendo el objetivo en un contrato de aceptación estricto y un desglose modular de micro-metas. Deduce implícitamente herramientas matemáticas o de desarrollo sin necesidad de sintaxis forzada.
- **Ejecutor (Executor)**: Genera y edita archivos de código uno a uno con validación de sintaxis inmediata, ejecuta comandos de terminal, orquesta contenedores e instala dependencias.
- **Crítico (Critic)**: Evalúa el cumplimiento del 100% de los criterios del contrato. Ejecuta pruebas automatizadas y prohíbe la finalización (`TOOL_FINISH`) si existen fallos de compilación, enlaces de assets rotos o errores en tests.

### 2. Unified Brain Selector & Turbo Mode (`⚡ MÁXIMA POTENCIA`)
- **Control Centralizado**: Un selector único e intuitivo en la barra superior unifica el modelo activo tanto para el frontend como para el backend.
- **Modo Turbo**: Permite activar la aceleración directa para omitir pasos intermedios repetitivos y maximizar el rendimiento en tareas guiadas por scripts.
- **HUD de Rendimiento**: Telemetría visual en vivo con temperatura, tiempo de respuesta y estado de GPU offload.

### 3. Dynamic Phase Gatekeeper (PESP v2) & Smart Alias Matching
- **Detección Flexible de Entregables**: Resuelve dinámicamente alias de archivos solicitados por el usuario (ej. `cyber_sentinel.html`, `dashboard.html`, `index.html`) evitando atascos rígidos por nombres de archivo predeterminados.
- **Auto-Convergencia Determinista**: Cuando un script de verificación (ej. `python verify_dashboard.py`) pasa con código de salida 0 y 100% de aserciones válidas, el sistema intercepta el resultado e invoca inmediatamente `TOOL_FINISH`, erradicando bucles infinitos de re-testeo.

### 4. AutoValidator Proactivo Conectado
- Integrado directamente en `validate_workspace(&workspace_path)`.
- Escanea de forma recursiva archivos HTML y JS en busca de dependencias locales faltantes (`<script src="...">`, `<link href="...">`, imágenes, audio).
- Reporta advertencias y errores accionables al agente para que subsane enlaces rotos antes de entregar el proyecto.
- Protegido contra inyecciones de código incompatibles en proyectos web convencionales.

### 5. SpectraSAT FFI — Motor de Satisfacibilidad en RAM
- Resuelve problemas de lógica proposicional (SAT/UNSAT) mediante una biblioteca nativa en Rust compilada con optimizaciones avanzadas de CPU.
- Procesa restricciones en microsegundos y emite veredictos certificados (`SAT_CERTIFIED` o `UNSAT_*`) con la asignación booleana exacta de cada variable, visualizadas mediante chips interactivos en la interfaz de usuario.

### 6. Sanity Monitor & Anti-Stall Engine
- Monitorea la actividad del agente cada 5 pasos.
- Detecta loops de herramientas, comandos repetitivos sin efecto o patrones de estancamiento.
- Inyecta advertencias correctivas de alta prioridad en el contexto para redirigir la estrategia del modelo.

### 7. Monaco Editor & Tarjetas de Artefactos
- Visualización de código en tiempo real con resaltado de sintaxis profesional.
- Selector de archivos en vivo para explorar los entregables del workspace mientras el agente trabaja.
- Tarjetas de artefactos dedicadas para salidas de consola, reportes de pruebas unitarias y veredictos de satisfacción lógica.

---

## 🧰 Catálogo Completo de Herramientas (Tools)

| Herramienta | Rol Permitido | Función / Propósito |
|---|---|---|
| `TOOL_PROGRAMMER` | Ejecutor | Crea o sobreescribe archivos de código fuente de forma atómica. |
| `TOOL_TERMINAL` | Ejecutor, Crítico | Ejecuta comandos reales en la shell del sistema operativo (PowerShell / CMD / Bash). |
| `TOOL_TESTER` | Crítico | Descubre y ejecuta la suite de pruebas nativa según el lenguaje del proyecto. |
| `TOOL_CONTAINER` | Ejecutor | Administra entornos Docker / Podman (`run`, `exec`, `stop`, `activate_env`). |
| `TOOL_LOGIC_SOLVER` | Planificador | Resuelve problemas de lógica, álgebra booleana y SAT con SpectraSAT FFI. |
| `TOOL_SCHEDULER` | Planificador | Programa tareas periódicas usando expresiones cron nativas en Tokio. |
| `TOOL_VISION_EVALUATOR` | Crítico | Captura la pantalla y evalúa visualmente la interfaz de usuario con visión computacional. |
| `TOOL_AUDITOR` | Planificador | Realiza revisiones de seguridad y calidad generando informes estructurados JSON. |
| `TOOL_MAPPER` | Planificador | Genera el mapa de dependencias y la estructura del árbol de archivos del workspace. |
| `TOOL_BACKGROUND_START` | Ejecutor | Inicia servicios o servidores en segundo plano sin bloquear el terminal. |
| `TOOL_BACKGROUND_STATUS`| Ejecutor | Consulta los logs y el estado de una tarea en segundo plano. |
| `TOOL_BACKGROUND_STOP`  | Ejecutor | Detiene un proceso en segundo plano. |
| `TOOL_ENV_MANAGER` | Ejecutor | Instala paquetes y binarios faltantes de manera transparente mediante Scoop / Winget. |
| `TOOL_ASK_USER` | Todos | Solicita información adicional cuando la intención del usuario es ambigua. |
| `TOOL_FINISH` | Planificador, Crítico | Concluye la misión cuando el 100% de los criterios han sido verificados. |

---

## 🌐 Ecosistemas y Lenguajes Soportados

El agente detecta automáticamente la tecnología del workspace y adapta sus comandos de prueba y validación:

- **Rust**: `Cargo.toml` (`cargo check`, `cargo test`)
- **Python**: `pyproject.toml`, `requirements.txt`, `test_*.py` (`pytest`, `compileall`)
- **Web / Frontend**: `*.html`, `*.css`, `*.js`, `*.ts` (`node --check`, AutoValidator)
- **Node.js / TypeScript**: `package.json`, `tsconfig.json` (`npm test`, `jest`, `vitest`)
- **Go**: `go.mod`, `*_test.go` (`go test ./...`)
- **Java**: `pom.xml`, `build.gradle` (`mvn test`, `gradlew test`)
- **Kotlin / Android**: `build.gradle.kts`, `AndroidManifest.xml`
- **C / C++**: `CMakeLists.txt`, `Makefile` (`gcc`, `make test`)
- **Solidity / Web3**: `hardhat.config.js`, `foundry.toml` (`npx hardhat test`, `forge test`)
- **PHP**: `phpunit.xml`, `*Test.php` (`phpunit`)
- **Dart / Flutter**: `pubspec.yaml` (`flutter test`)
- **Swift**: `Package.swift` (`swift test`)

---

## 🚀 Inicio Rápido

### Requisitos
1. **Rust y Cargo**: `rustup default stable`
2. **Node.js**: v18+ y npm
3. **Ollama**: Motor local de LLMs ([ollama.com](https://ollama.com))

### Modelos Recomendados en Ollama
```bash
ollama pull qwen2.5-coder:7b     # Modelo principal recomendado para ejecución ágil y precisa
ollama pull llama3.1:8b          # Alternativa excelente para orquestación y razonamiento
ollama pull qwen2.5-coder:14b    # Modelo para arquitectura compleja y resolución avanzada
```

### Compilación y Ejecución
```bash
# 1. Instalar dependencias frontend
npm install

# 2. Iniciar en modo desarrollo con Tauri
npm run tauri dev
```

---

## 📜 Licencia
Distribuido bajo la [Licencia MIT](LICENSE).  
Copyright (c) 2026 Ramón Antonio Burgos Jerez.
