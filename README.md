# Aura-Sentinel 🛡️🧠
**Agente Autónomo de Ingeniería de Software DevSecOps de Nivel Industrial**  
*Desarrollado por Ramón Antonio Burgos Jerez*

> 🏆 **Nivel Alcanzado: Agente Autónomo Cognitivo Completo — Tier 0 (v4.0.0 — Septiembre 2026)**  
> Arquitectura de autonomía completa, determinismo formal, contratos de misión y ejecución gobernada por puertas de finalización verificables. Estándar de la industria equiparable a Devin 2.0 y SWE-agent (Princeton ACI).

Aura-Sentinel es un sistema de ejecución e ingeniería de software autónomo de alto rendimiento. Ejerce control sobre el ciclo de vida de desarrollo de software: deducción de contratos formales de aceptación, particionado de fases, generación modular de código atómico, validación sintáctica determinista, presupuestación matemática de pasos y suites de verificación con auto-reparación en bucle cerrado. Construido sobre un motor nativo en **Rust (Tauri)** con **Monaco Editor**, orquesta modelos locales (Ollama) sin depender de servicios en la nube, garantizando cero latencia y soberanía absoluta de datos.

---

## 🏆 Estado del Sistema (v4.0.0 Enterprise Cognitive Runtime)

| Módulo / Capa Subyacente | Estado | Nivel / Tier | Garantía Arquitectónica |
|---|---|---|---|
| **Contrato Formal de Misión (`mission_contract`)** | 🟢 100% Operativo | Tier 0 (Determinista) | Criterios de aceptación verificables antes de generar código |
| **Puerta de Finalización (`completion_gate`)** | 🟢 100% Operativo | Tier 0 (Gobernado) | Bloquea `TOOL_FINISH` si restan criterios o evidencias pendientes |
| **Grafo de Evidencias Verificables (`evidence`)** | 🟢 100% Operativo | Tier 0 (Auditabilidad) | Registro criptográfico y comprobación de aserciones de prueba |
| **Presupuesto de Pasos 40/30/20/10 (`step_budget`)** | 🟢 100% Operativo | Tier 0 (Confiabilidad) | 40% dev / 30% verificación / 20% auto-reparación / 10% handoff |
| **Validación Previa de Esquema (`schema_validator`)** | 🟢 100% Operativo | Tier 0 (Resiliente) | Rechaza payloads JSON defectuosos antes del despacho al sistema |
| **Typed Error Envelope (`envelope`, `error_classifier`)** | 🟢 100% Operativo | Tier 0 (Industrial) | Señalización `{retryable, requires_human}` anti-bucles |
| **Motor de Políticas y Seguridad (`policy`)** | 🟢 100% Operativo | Tier 0 (Seguridad) | Bloquea comandos destructivos y requiere aprobación en riesgos altos |
| **Detector de Estancamiento (`stall_detector`, `state_delta`)**| 🟢 100% Operativo | Tier 0 (Anti-Loop) | Detección matemática de delta-cero sobre el sistema de archivos |
| **Memoria Cognitiva & Lecciones (`experience`)** | 🟢 100% Operativo | Tier 0 (Cognición) | Huella de tareas y reutilización de estrategias análogas previas |
| **Perfil Multi-Lenguaje (`project_profile`)** | 🟢 100% Operativo | Tier 0 (Multi-Stack) | Detección automática de Rust, Python, TS/JS, Go, C# y frameworks |
| **Orquestador Desacoplado (`mission_runtime`)** | 🟢 100% Operativo | Tier 0 (Modular) | Encapsulamiento del loop y métricas en tiempo real |
| **SpectraSAT FFI — Motor Booleano en RAM** | 🟢 100% Operativo | Tier 0 (Determinista) | Satisfacibilidad lógica garantizada con chips visuales interactivos |
| **Persistencia Atómica & Sesiones (`session_journal`)** | 🟢 100% Operativo | Tier 0 (Tolerancia) | Escrituras seguras vía `.tmp` + rename con UUIDs v4 |
| **FSM Multi-Agente (Planificador → Ejecutor → Crítico)** | 🟢 100% Operativo | Tier 0 (Industrial) | Control de flujo gobernado por el arnés, no por el modelo |

---

## 🏛️ Arquitectura del Motor Autónomo v4

```text
               OBJETIVO DEL USUARIO
                         ↓
               MISSION CONTRACT (ACs)
                         ↓
      POLICY ENGINE & PRE-EXECUTION SCHEMA
                         ↓
             FSM RUNTIME ORCHESTRATOR
    [ Planificador → Ejecutor → Crítico ]
                         ↓
      EVIDENCE GRAPH & STATE DELTA TRACKER
                         ↓
       COMPLETION GATE (Aprobación Formal)
                         ↓
          ENTREGA DETERMINISTA & EXPERIENCIA
```

### 1. Gobernanza por Contratos y Puerta de Cierre
Aura Sentinel v4 no permite que el LLM declare arbitrariamente que terminó una tarea mediante `TOOL_FINISH`. La **Puerta de Finalización** (`CompletionGate`) coteja matemáticamente que cada criterio de aceptación del contrato cuente con evidencia verificada (códigos de retorno 0, pruebas aprobadas, hashes de archivos en disco). Si falta una sola comprobación requerida, el cierre es rechazado con instrucciones exactas para subsanar el faltante.

### 2. Presupuesto Asignado por Fases (40/30/20/10)
Inspirado en el estándar de Devin 2.0 y SWE-agent:
- **40% Ejecución Inicial**: Escritura limpia de componentes y dependencias.
- **30% Verificación Rigurosa**: Ejecución de suites de prueba automatizadas.
- **20% Auto-Reparación Guiada**: Reflexión profunda y resolución de aserciones fallidas sin tocar al usuario.
- **10% Margen de Handoff Seguro**: Garantiza la generación del informe final antes de agotar el límite de pasos.

### 3. Autoprotección y Prevención de Estancamiento
- **StallDetector**: Monitorea la firma de progreso (`ProgressSignature`). Si el modelo ejecuta herramientas repetidamente sin alterar el estado del disco o sin avanzar en el grafo de evidencias, se activan intercepciones duras que rompen el bucle.
- **PolicyEngine**: Impide comandos peligrosos fuera de sandbox (`format`, `diskpart`, borrado de raíces del sistema).

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

El agente detecta automáticamente la tecnología del workspace con `ProjectProfile` y adapta sus comandos de prueba y validación:

- **Rust**: `Cargo.toml` (`cargo check`, `cargo test`, `cargo clippy`)
- **Python**: `pyproject.toml`, `requirements.txt`, `test_*.py` (`pytest`, `ruff`, `compileall`)
- **Web / Frontend**: `*.html`, `*.css`, `*.js`, `*.ts` (AutoValidator desacoplado)
- **Node.js / TypeScript**: `package.json`, `tsconfig.json` (`npm test`, `jest`, `vitest`)
- **Go**: `go.mod`, `*_test.go` (`go test ./...`, `golangci-lint`)
- **Java**: `pom.xml`, `build.gradle` (`mvn test`, `gradlew test`)
- **Kotlin / Android**: `build.gradle.kts`, `AndroidManifest.xml`
- **C / C++ / C#**: `CMakeLists.txt`, `Makefile`, `*.csproj` (`dotnet test`, `make test`)
- **Solidity / Web3**: `hardhat.config.js`, `foundry.toml` (`npx hardhat test`, `forge test`)
- **PHP, Dart / Flutter, Swift**: Detección nativa con suites específicas.

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
