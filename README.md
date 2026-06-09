# Aura-Sentinel 🛡️🧠
**Agente DevSecOps Autónomo y Resiliente — por Ramon Antonio Burgos Jerez**

Aura-Sentinel es un agente de Inteligencia Artificial diseñado para desarrollo de software continuo, control de calidad automatizado y resiliencia arquitectónica. Construido con **Tauri (Rust + Vanilla JS)**, orquesta múltiples modelos de lenguaje locales (Ollama) para programar, auditar y proteger tu código **sin depender de APIs de terceros en la nube**.

Diseñado para un flujo de trabajo real y diverso: sistemas Android, facturación, lotería, Firebase, inteligencia artificial con redes neuronales, trading autónomo en Rust/Python, sistemas MEV en Polygon/BNB, y aprendizaje diario de nuevos lenguajes.

---

## 🏗️ Arquitectura de DevSecOps Autónoma

Aura-Sentinel no solo escribe código; lo entiende, lo valida, asegura sus dependencias y se autoprotege a través de sistemas modulares críticos:

### 1. Ingeniería de Entornos (Módulo DevOps Autónomo) 🌍
- **Auto-Instalación Silenciosa**: Detecta qué programa falta (`go`, `node`, `python`, `gcc`, `mvn`, etc.) y lo instala mediante `scoop` o `winget`, sin permisos de administrador invasivos.
- **Hot-Reloading del PATH**: Tras instalar una dependencia, Rust recarga `PATH` desde el registro de Windows en memoria. Aura instala un compilador en el Paso 1 y lo usa en el Paso 2, *sin reiniciar la aplicación*.
- **Pre-Flight Check**: Antes de cada misión, valida que el entorno mínimo esté operativo (Ollama respondiendo, workspace válido).

### 2. Máquina de Estados y Kill-Switches (Protección Anti-Bucles) 🛑
- **Cooldown de Programador**: Si Aura edita el mismo archivo 3 veces seguidas sin probar, Rust aborta la operación.
- **Terminal Estricta**: Intercepta comandos vacíos y comandos repetitivos (detecta `go mod init` ejecutándose dos veces y lo bloquea).
- **Escalada de 3 Niveles (NoTests)**: Si el agente intenta ejecutar tests en un proyecto vacío: 1) Advierte, 2) Fuerza crear archivos de test, 3) Fuerza instalación del framework de test.
- **Detección Inteligente de Errores de Dependencias**: Distingue entre `go.mod` ausente (→ `go mod init`) y errores de sintaxis en el código (→ Auto-Debugger). Evita que el agente ejecute `go mod init` repetidamente cuando el problema es lógico.

### 3. Git-Shield (Protección de Integridad) 🛡️
- Antes de cada cambio propuesto por el programador, realiza un `git commit` silencioso.
- Si el código falla, revierte con `git restore .` preservando el último estado funcional.
- **Snapshot de Terminal**: Cuando el terminal ejecuta un comando exitoso (ej. `go mod init`), Git-Shield toma un snapshot inmediato para que archivos de configuración como `go.mod` no se pierdan en un revert posterior.

### 4. QA Autónomo (TOOL_TESTER & Auto-Debugger) 🧪
- Detecta el lenguaje del proyecto automáticamente y ejecuta la suite de tests nativa.
- **Detección de Errores**: Distingue errores de dependencias (→ `TOOL_TERMINAL` automático) de errores de lógica (→ `TOOL_PROGRAMMER` obligatorio).
- **Auto-Debugger**: Si los tests fallan por lógica, el sistema obliga al agente a arreglar el código antes de poder volver a testear.

### 5. Razonamiento Lógico Matemático (TOOL_LOGIC_SOLVER) 🧮
- **Z3 Theorem Prover**: Evalúa matemáticamente la validez del código para detectar bucles infinitos y condiciones inalcanzables.
- Capacidad de razonamiento avanzado para asegurar la completitud lógica del código antes de compilarlo.

### 6. Autonomía de Entorno (TOOL_WORKSPACE_MANAGER) 🧹
- **Mantenimiento Autónomo**: El agente limpia activamente su propio workspace, eliminando archivos basura o pruebas temporales tras completar sus tareas.

### 7. Inteligencia Híbrida (Router Dinámico) 🧠☁️
- **Complejidad Baja**: `deepseek-coder:6.7b` — operaciones rápidas y directas.
- **Complejidad Media**: `qwen2.5-coder:7b` — lógica de negocio estándar.
- **Complejidad Alta / Post-Fallo**: `qwen2.5-coder:14b` — debugging complejo, escala automáticamente tras un fallo de tests.
- **Orquestador**: `llama3.1:8b` — planificación y checklist mental.
- Compatible con modelos en la nube vía Ollama para tareas que superen la capacidad local.

### 8. Memoria Vectorial Permanente (RAG) 🐘
- Código exitoso es vectorizado con `nomic-embed-text` y almacenado en un motor de Similitud de Coseno nativo en Rust puro.
- En futuras tareas, Aura escanea su historial e inyecta fragmentos relevantes en su contexto antes de programar — evita reinventar la rueda.
- Memoria aislada y etiquetada por workspace para no contaminar contextos.

### 9. Path Jail (Seguridad Estricta) 🔒
- Intercepta y audita cada intento de lectura/escritura (`is_path_allowed`).
- Si Aura intenta tocar cualquier archivo fuera del directorio de proyecto, la operación se bloquea bajo `[SECURITY_VIOLATION]`.

### 10. Gestión de Contexto Avanzada 🗜️
- Carpetas masivas como `node_modules`, `.git`, `target`, `build`, `vendor` son invisibles para el LLM.
- En cada turno, el agente recibe un mapa exacto de los archivos que *realmente existen* en el workspace.

---

## 🌐 Lenguajes Soportados por TOOL_TESTER

El sistema detecta el lenguaje automáticamente por los archivos presentes en el workspace:

| Lenguaje | Detección automática | Comando de tests | Caso de uso |
|---|---|---|---|
| **Go** | `*_test.go` / `go.mod` | `go test ./...` | APIs, backends, CLI tools |
| **Rust** | `Cargo.toml` + `#[test]` | `cargo test` | Trading autónomo, sistemas de alto rendimiento |
| **Python** | `test_*.py` / `pytest.ini` / `pyproject.toml` | `python -m pytest` | IA, redes neuronales, trading, bots |
| **JavaScript** | `*.test.js` / `jest.config.js` | `npm test` | Frontend, Firebase, Node.js |
| **TypeScript** | `*.test.ts` / `jest.config.ts` | `npm test` | Firebase, apps web, APIs |
| **Java (Maven)** | `pom.xml` | `mvn test -q` | Sistemas de facturación, backend empresarial |
| **Java (Gradle)** | `build.gradle` / `build.gradle.kts` | `gradlew test` | Proyectos Java modernos |
| **Kotlin (Android)** | `build.gradle` + `AndroidManifest.xml` | `gradlew test` | Apps Android, sistemas móviles |
| **Solidity (Hardhat)** | `hardhat.config.js` / `.ts` | `npx hardhat test` | Contratos MEV en Polygon, BNB, ETH |
| **Solidity (Foundry)** | `foundry.toml` | `forge test -v` | Contratos MEV avanzados, fuzzing |
| **C++** | `CMakeLists.txt` / `Makefile` con `test:` | `make test` | Sistemas embebidos, alto rendimiento |
| **C** | `test_*.c` / `*_test.c` | `gcc + ejecuta` | Algoritmos, sistemas base |
| **PHP** | `phpunit.xml` / `*Test.php` | `php vendor/bin/phpunit` | Sistemas de facturación web |
| **Dart / Flutter** | `pubspec.yaml` | `flutter test` / `dart test` | Apps móviles multiplataforma |
| **Swift** | `Package.swift` | `swift test` | Apps iOS, macOS |

---

## 🛠️ Instalación

### Requisitos Previos
1. [**Ollama**](https://ollama.com/) — Motor de IA local
2. [**Rust / Cargo**](https://rustup.rs/) — Para compilar el backend
3. [**Node.js**](https://nodejs.org/) — Para el frontend
4. [**Git**](https://git-scm.com/) — Para Git-Shield

### Modelos de Ollama Requeridos
```bash
ollama pull llama3.1:8b          # Orquestador y Planificador
ollama pull qwen2.5-coder:7b     # Ingeniero Base
ollama pull deepseek-coder:6.7b  # Ingeniero Rápido
ollama pull qwen2.5-coder:14b    # Ingeniero Experto (Auto-Debugger)
ollama pull nomic-embed-text     # Motor de Memoria Vectorial (RAG)
```

### Ejecución
```bash
npm install
npm run tauri dev
```

### Scripts de Configuración Automática
- **Windows**: `.\setup.ps1` en PowerShell
- **Mac/Linux**: `bash setup.sh`

---

## 📋 Formato del Comando
```
[USER] Aura, en un entorno de [LENGUAJE], [DESCRIPCIÓN DE LA TAREA].
Ejecuta TOOL_TESTER. Cuando el test pase, usa TOOL_FINISH para avisarme.
```

### Ejemplos por dominio
```
# Android
Aura, en un entorno Kotlin/Android, crea una clase Calculadora con suma y resta. 
Crea su test unitario. Ejecuta TOOL_TESTER y usa TOOL_FINISH al terminar.

# MEV / Blockchain
Aura, en un entorno Solidity con Hardhat, crea un contrato ERC20 básico con
función de mint. Crea el test con ethers.js. Ejecuta TOOL_TESTER.

# Trading (Rust)
Aura, en un entorno Rust, crea una función que calcula el RSI dado un vector
de precios. Crea su test con datos reales de BTC. Ejecuta TOOL_TESTER.

# Redes Neuronales (Python)
Aura, en un entorno Python, crea una red neuronal simple con NumPy para
clasificar XOR. Crea su test de accuracy. Ejecuta TOOL_TESTER.

# Facturación (Java/Maven)
Aura, en un entorno Java con Maven, crea una clase Factura con cálculo de IVA.
Crea su test con JUnit. Ejecuta TOOL_TESTER.
```

---

## 🔧 Historial de Correcciones Críticas (Changelog)

### v0.1.0 — 2026-06-07 (Sesión de Estabilización)
Sesión de ingeniería intensiva que resolvió la cadena de fallos que impedía completar tareas en Go:

| Fix | Problema resuelto |
|-----|-------------------|
| **Ollama via HTTP** | `ollama list` fallaba en Windows por alias de ejecución. Reemplazado por llamada HTTP a `http://127.0.0.1:11434/api/tags` |
| **Quitar `forced_next_tool` de TOOL_TESTER** | El sistema forzaba una herramienta sin pasar el JSON correcto, generando comandos vacíos en cascada |
| **Prompt post-TOOL_TERMINAL** | El LLM volvía a usar el terminal tras un éxito en vez de ir a TOOL_TESTER. Se añadió instrucción imperativa |
| **Git-Shield tras TOOL_TERMINAL** | `go.mod` creado por el terminal era eliminado en el siguiente revert por no estar trackeado en Git |
| **Quitar `setup failed` de dep detection** | Go usa `[setup failed]` tanto para módulo faltante como para error de sintaxis. El sistema los confundía, enviando al LLM a `go mod init` infinitamente cuando el problema era un bug de código |
| **Soporte Java (Maven/Gradle)** | Añadido a `languages.rs` |
| **Soporte Kotlin/Android** | Detección por `AndroidManifest.xml` + Gradle |
| **Soporte Solidity** | Hardhat (`npx hardhat test`) y Foundry (`forge test`) |
| **Soporte PHP** | PHPUnit via `phpunit.xml` o `*Test.php` |
| **Soporte Dart/Flutter** | Detección por `pubspec.yaml` |
| **Soporte Swift** | Detección por `Package.swift` |
| **Soporte C** | Compilación y ejecución con `gcc` de archivos `test_*.c` |

---

## 🤝 Filosofía de Uso
1. **Pídele tareas completas** con tests incluidos.
2. **Deja que actúe** — verás en la consola técnica cómo planifica, asegura el entorno, escribe, hace backup, prueba y aprende.
3. **Intervención Cero** — dependencias, errores de compilación y fallos lógicos son manejados automáticamente.

---

## ⚖️ Legal & Licensing
Distribuido bajo la [Licencia MIT](LICENSE).  
Copyright (c) 2026 Ramon Antonio Burgos Jerez.
