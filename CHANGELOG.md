# Registro de cambios — Aura Sentinel

Todos los cambios relevantes del proyecto se documentan aquí. Formato: `[versión] — fecha`, con
[versionado semántico](https://semver.org/lang/es/).

---

## [4.2.0] — 2026-10-16 (Motor en el repositorio e instalación completa)

- El motor (Agente Local) vive en `motor/`: clonar el repositorio trae todo lo necesario. No se suben su
  `.venv`, las medidas de VRAM de cada equipo ni las reglas y lecciones del usuario.
- `setup.ps1` instala un equipo nuevo de principio a fin: requisitos, interfaz, motor (entorno de Python,
  pruebas, registro MCP en Bionic/LM Studio), descarga verificada del modelo y ajuste a la VRAM en Bionic
  (contexto, capas de expertos en RAM/CPU, ajustes de chat). Se puede repetir.
- `motor\descargar_modelo.bat`: baja Qwen3.6-35B-A3B y su visión desde Hugging Face, reanuda los cortes y
  comprueba el SHA-256 publicado.
- Aura encuentra el motor del repositorio o el incluido en el instalador; si falta su entorno, Configuración →
  Motor ofrece **Instalar motor**. El instalador de Windows lleva el motor dentro.
- Icono propio (el cubo hexagonal de Aura) en la app, el instalador y la pestaña.
- Guía de instalación, integración con Bionic (qué cambia y cómo deshacerlo) y solución de problemas en
  `README.md` y `motor/LEEME.md`.
- Autoría y licencia MIT en `package.json`, `Cargo.toml` y el instalador (editor y copyright); guía de
  contribución, código de conducta, política de seguridad y plantillas de issues y pull requests en `.github/`.
- Instalador `setup.exe` publicado en GitHub (versiones) y guía para instalarlo sin compilar, con el aviso de
  SmartScreen y la comprobación SHA-256.
- Guía de instalación revisada desde un clon limpio: Git como requisito (`setup.ps1` avisa si falta), espacio
  real en disco (~30 GB) y qué hacer en equipos con menos memoria.
- La app ya no busca el motor en la carpeta de trabajo anterior a la unificación; eliminado `validate_project.rs`,
  que dependía del motor retirado.
- Arreglo: la interfaz se quedaba en blanco al usar la Terminal. En WebView2 reciente `scrollIntoView()`
  devuelve una Promise y un efecto de React la devolvía como si fuera su limpieza. Ningún efecto devuelve ya
  valores, y un límite de errores muestra «Recargar la interfaz» en vez de una ventana vacía (con prueba).
- Arreglo: las confirmaciones no esperaban respuesta. En Tauri `window.confirm` devuelve una Promise, así que
  «Nueva conversación», «Restaurar», «Deshacer cambios del agente», «Borrar perfil» y cerrar una pestaña sin
  guardar se ejecutaban sin preguntar. Ahora usan el diálogo nativo de Windows (`ask` con el permiso
  `dialog:allow-ask`), y Ctrl+W también pregunta si la pestaña tiene cambios (con prueba).

## [4.1.0] — 2026-10-16 (Interfaz nueva)

- Interfaz reescrita en React + TypeScript con Vite (antes un `main.js` de 1 200 líneas). Monaco 0.57,
  marked 18, DOMPurify 3.4.17; pruebas con Vitest.
- Panel del agente con estado, plan, tarjetas por paso, capturas, confirmación en línea y acciones rápidas;
  explorador con marcas de cambios del agente; aviso con diferencias y Deshacer en el editor; terminal,
  problemas, procesos y vista previa; paleta de comandos; búsqueda; git; puntos de restauración; reglas.
- Backend: evento `agent-event`, órdenes rápidas `agent_order`, `ide.rs` (estado del sistema con VRAM, git,
  capturas, búsqueda, terminal, copia del proyecto por misión).
- Instalador de Windows (NSIS y MSI) con identificador propio `com.yemsy.aurasentinel`.
- Rendimiento: Monaco en diferido (paquete inicial de 4,3 MB a 358 KB; primer pintado en ~0,1 s), sin
  consultas de sistema con la ventana oculta, binario con LTO y sin símbolos. Medido en la versión final:
  236 MiB de RAM con WebView2, 0 MiB de VRAM; misión real desde la interfaz en 1,2 min.

## [4.0.0] — 2026-10-16 (Agente Local como único motor)

- Las misiones las ejecuta Agente Local con el modelo de Bionic/LM Studio (`src-tauri/src/agent.rs`).
- Retirado el motor original basado en Ollama (~49 000 líneas: `llm/`, `core/`, `memory/`, `net/`,
  SpectraSAT, el submódulo Chronos-VFS, `Dockerfile`, `setup.sh`, `patch.py`, `data/`). Con él desaparecen
  sus 6 pruebas que fallaban; quedan 14 pruebas Rust y 3 de interfaz, todas en verde.
- Fuera el botón "Máxima potencia" y el selector de motor; el selector de modelos lista los de Bionic.
- Reanudar ahora detecta planes de Agente Local con pasos pendientes; el árbol de archivos ya no se vacía
  si una carpeta (p. ej. de OneDrive) no se puede leer.
- `start.bat` y `setup.ps1` comprueban Bionic y Agente Local en vez de Ollama.

---

## [3.0.0] — 2026-09-06 (Arquitectura v3)

### Selector de modelo unificado y modo turbo
- **Selector unificado**: un único selector de modelo en la barra superior, sincronizado con el backend de Tauri; se
  eliminaron los controles redundantes.
- **Modo turbo** («Máxima potencia»): omite micrometas redundantes y acelera los bucles de ejecución directa.
- **Indicador de estado**: temperatura, descarga en GPU y estado de compilación en tiempo real.

### Control de fases dinámico (PESP v2) y detección de entregables
- Los proyectos web ya no exigen `index.html`: se detectan los archivos HTML (`has_html_files`) y los entregables con
  otros nombres (`cyber_sentinel.html`, `dashboard.html`, etc.).
- El control de fases comprueba que los entregables existen y tienen contenido antes de pasar de fase.
- Si un script de verificación (p. ej. `verify_*.py`) termina con código 0 y todo en verde, la misión termina
  (`TOOL_FINISH`) en lugar de seguir consumiendo pasos.

### Subsistema AutoValidator reconectado
- `core::auto_validator::AutoValidator` vuelve a formar parte de la validación del espacio de trabajo
  (`validate_workspace`).
- Comprueba las etiquetas `<script src="...">`, `<link href="...">` y los recursos locales antes de la verificación
  final.
- Evita insertar código incompatible al generar recursos de aplicaciones web puras.

### Saneador de contexto y limpieza
- Eliminadas las rutas y marcadores obsoletos escritos a mano (`proxy-stack-windows`).
- El contexto de cada sesión filtra las rutas de otros proyectos del disco.
- La compilación de Rust queda con 0 errores y 0 avisos (`cargo check` en menos de 4 s).

---

## [0.5.0] — 2026-08-30 (Sprint 4)

### Nuevo — Arquitectura de Autonomía Completa (Tier 0)

#### SpectraSAT FFI — Motor Lógico Matemático en RAM
- Integración del motor SpectraSAT como librería nativa Rust vía FFI.
- El agente resuelve problemas SAT/UNSAT sin delegar al LLM, eliminando alucinaciones matemáticas.
- Retorna JSON estructurado con veredicto (SAT_CERTIFIED / UNSAT_*) y asignación booleana exacta de variables.
- Pipeline de 4 capas: GF2 Algebraico → SDP Relajado → Branch-and-Bound + Verificador Booleano → DPLL Exhausto.
- Corregido bug crítico de falso-positivo SAT: el motor ahora verifica cada asignación contra todas las cláusulas antes de certificar.

#### Zero-Hint Router
- El Planificador ahora deduce semánticamente qué herramienta usar sin que el usuario la mencione.
- Vocabulario matemático añadido al clasificador de intenciones: auditoría, satisfacibilidad, SAT, lógica.
- Misiones matemáticas ahora clasificadas como ANÁLISIS, evitando el protocolo de fases de construcción.

#### Mission Persistence (core/mission_persist.rs)
- Checkpoint de FSM (contexto, rol, paso) guardado a disco cada 3 pasos.
- auto_resume_if_needed() en setup() detecta misiones interrumpidas al reiniciar la aplicación.
- Evento mission-resumed emitido al frontend para notificar al usuario.

#### TOOL_CONTAINER (core/container.rs)
- Soporte nativo para Docker y Podman mediante auto-detección de runtime.
- Acciones: run, exec, stop, logs, activate_env.
- activate_env detecta tipo de proyecto (Node, Python, Rust) y activa el entorno virtual correcto.
- Handler TOOL_CONTAINER integrado en la FSM del Ejecutor.

#### Memoria Episódica Multi-Sesión (core/episodic_memory.rs)
- Cada misión completada genera un episodio persistente en ~/.aura_episodes.jsonl.
- Al iniciar una nueva tarea, se inyectan los últimos 3 episodios relevantes al contexto del agente.
- Búsqueda semántica simple por coincidencia de texto en el objetivo.
- Comandos Tauri: search_episodes_cmd, get_recent_episodes_cmd.

#### Scheduler Autónomo (core/scheduler.rs)
- Motor de tareas programadas usando cron expressions estándar (5 campos).
- Loop Tokio con tick cada 60 segundos, ejecutado en el runtime de Tauri (`tauri::async_runtime::spawn`).
- Herramienta TOOL_SCHEDULER disponible para el Planificador.
- Comandos Tauri: schedule_task, list_scheduled_tasks, remove_scheduled_task.

#### Sanity Monitor (core/sanity_monitor.rs)
- Evaluación de salud del agente cada 5 pasos de ejecución.
- Detecta: bucles de herramientas, presión de RAM (>90%), fallos JSON consecutivos, estancamiento.
- Emite nivel de alerta (GREEN / YELLOW / RED) con recomendaciones al contexto.
- Evento sanity-report emitido al frontend para el panel de telemetría.

### Corregido
- Bug: `tokio::spawn` en el scheduler causaba pánico al inicio (`no reactor running`). Cambiado a `tauri::async_runtime::spawn`.
- Bug: Extractor de cláusulas usaba `rfind("]]")` frágil. Reemplazado por contador de profundidad de corchetes.
- Bug: Planificador clasificaba auditorías matemáticas como misiones de Construcción, activando protocolo PESP de 4 fases y causando pausas interactivas innecesarias.
- Bug: Variables retry_tracker y agent_workspace eliminadas accidentalmente de scope en refactor anterior.

### Modificado
- session_journal.rs: Campos fsm_context, fsm_role, fsm_step, interrupted añadidos.
- core/mod.rs: Registrados los 5 módulos nuevos del Sprint 4.
- main.rs: start_scheduler() y auto_resume_if_needed() integrados en setup().
- agent.rs: Prompts del Planificador y Ejecutor actualizados con catálogo completo de herramientas.
- .gitignore: Excluidos binarios de prueba, Modelfiles, archivos de sesión runtime.

---

## [0.4.0] — 2026-07-15 (Sprint 3)

### Nuevo
- Vision Evaluator (xcap + moondream) para análisis visual de interfaces.
- Command Trail System para rastrear comandos ejecutados.
- Script Runner Generator automático por lenguaje de proyecto.
- Prompts comprimidos a <200 tokens optimizados para modelos 7B.
- Frontend Anti-Backend Shield.

---

## [0.3.0] — 2026-07-01 (Sprint 2)

### Nuevo
- Transición FSM Ejecutor → Crítico por micrometas.
- TOOL_AUDITOR con salida JSON estructurada.
- Anti-Stub Enforcer para bloquear código incompleto.

---

## [0.2.0] — 2026-06-15 (Sprint 1)

### Nuevo
- FSM Multi-Agente (Planificador, Ejecutor, Crítico).
- Clasificador de Tipo de Misión (NLU heurístico).
- Contrato de Aceptación entre roles.
- Compresión de contexto cada 10 pasos.
