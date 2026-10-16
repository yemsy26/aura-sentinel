# Aura Sentinel

Agente de desarrollo de software con interfaz Tauri, núcleo Rust, editor Monaco y modelos de Ollama.
Desarrollado por Ramón Antonio Burgos Jerez.

## Ejecución

Requisitos: Node.js 24, Rust con herramientas de compilación de Windows y WebView2. Según el motor:
Bionic o LM Studio con Agente Local, u Ollama (`127.0.0.1:11434`) para el motor original.

```sh
git submodule update --init   # src-tauri/memory_vfs (Chronos-VFS)
npm install
npm run tauri dev
```

## Motores de las misiones

El selector junto al modelo elige quién ejecuta las misiones:

- **Agente Local (Bionic)**, por defecto si está instalado. Aura lanza `python -m agente <carpeta> --json`
  desde la instalación de Agente Local y muestra sus pasos, archivos y preguntas en la interfaz
  (`src-tauri/src/llm/local_agent.rs`). El agente planifica, escribe, ejecuta, verifica y revisa con el modelo
  cargado en Bionic/LM Studio. El selector de modelos lista los de Bionic; elegir otro lo carga ajustado a la VRAM.
  Conserva la conversación mientras no cambies de carpeta ni de modelo. Cancelar detiene el agente y sus procesos.
- **Aura (Ollama)**: el motor original de este repositorio.

La elección se guarda en `motor.json` (carpeta de configuración de la app). `AURA_MOTOR=aura|agente-local` y
`AGENTE_LOCAL_DIR=<carpeta>` la sustituyen. Sin configuración se busca la instalación en
`%USERPROFILE%\.gemini\antigravity\scratch\lm studio` y `%USERPROFILE%\AgenteLocal`.
El registro del agente queda en `%TEMP%\aura_agente_local.log`.

El paso previo de Tauri copia Monaco y DOMPurify desde `node_modules` a `src/vendor`.
El editor y el filtro HTML quedan incluidos en la aplicación; no necesitan un CDN durante el uso.
La instalación inicial de dependencias y las herramientas de búsqueda o descarga sí pueden necesitar internet.
Selecciona una carpeta de trabajo y un modelo instalado en Ollama antes de iniciar una misión.

## Verificación

```sh
npm test
```

Este comando prepara los recursos de la interfaz, comprueba JavaScript, ejecuta las pruebas de interfaz y las pruebas Rust de todos los objetivos de la aplicación.
Las pruebas de interfaz simulan el puente de Tauri: no sustituyen una prueba visual en WebView2 ni una misión con un modelo real.

Si el lanzador `npm` de Windows falla buscando `npm-cli.js` en otra instalación, puede utilizarse el ejecutable existente:

```powershell
node "C:\Program Files\nodejs\node_modules\npm\bin\npm-cli.js" test
```

No copies carpetas `target` entre ubicaciones del proyecto. Pueden conservar rutas absolutas de compilaciones anteriores.

## Estructura

- `src/`: interfaz, chat y editor.
- `scripts/`: preparación de dependencias locales y pruebas de interfaz.
- `src-tauri/src/main.rs`: entrada de escritorio; llama a `app_lib::run()`.
- `src-tauri/src/lib.rs`: inicialización única de Tauri, comandos y servicios.
- `src-tauri/src/llm/`: clasificación de instrucciones y bucle del agente.
- `src-tauri/src/core/mission_runtime.rs`: ejecución gobernada por herramientas, observaciones y presupuesto.
- `src-tauri/src/core/completion_gate.rs`: comprobación del contrato antes del cierre del bucle.
- `src-tauri/src/core/evidence.rs`: registro de evidencia y asociación con el estado observado.
- `src-tauri/memory_vfs/` y `src-tauri/spectrasat/`: componentes Rust independientes.

## Comportamiento de las misiones

Las tareas de la interfaz se ejecutan de forma secuencial y conservan la carpeta asignada.
La continuación explícita (`continua`, `continúa`, `retoma`, `resume`) utiliza el diario de esa carpeta.
El botón de recuperación consulta el estado persistido al cargar la interfaz, evitando depender exclusivamente de un evento de inicio.
Descartar una misión elimina su estado pendiente, sin marcarla como completada.

El cierre exige los criterios del contrato y evidencia compatible con el estado observado.
Si el contrato exige revisión manual, se solicita al usuario después de superar las comprobaciones técnicas.
Una respuesta del modelo no sustituye esa revisión. Las tarjetas de certificados y tests basadas únicamente en palabras del chat se han retirado.

## Límites actuales

### Diagnóstico de una misión real

Compila con `cargo build --manifest-path src-tauri/Cargo.toml --bin aura-diagnostics`.
Prepara una carpeta de prueba vacía y un JSON con `workspace` (ruta absoluta), `prompt` y `model` (nombre instalado en Ollama).
Ejecuta `src-tauri\target\debug\aura-diagnostics.exe --diagnose-mission ruta-absoluta-al-job.json`.
Este modo usa el motor real, ejecuta herramientas y modifica la carpeta indicada. Omite el programador de tareas y la recuperación automática de otras misiones.
Guarda eventos con tiempos en `.aura/diagnostic-events.jsonl` y el resultado en `.aura/diagnostic-result.json` dentro de esa carpeta. El proceso termina con código 0 solo si el motor devuelve `FINISH`.

La escritura del programador valida sintaxis; no equivale a una prueba funcional. Un verificador solicitado debe imprimir una línea JSON con `passed` y `total` enteros, `percentage` calculado y `failed_criteria` como lista de textos. Debe ejecutar comprobaciones sobre los archivos reales y devolver código 1 al fallar. Una ejecución sin resultados se devuelve a reparación.
La reparación sintáctica conserva el borrador rechazado en `.aura/programmer_failure.json` y revierte únicamente los archivos de esa propuesta. Los scripts internos preparados no se presentan como procesos en ejecución.

### Cobertura pendiente

- El programador usa UTC y necesita la aplicación abierta. La entrega de sus eventos no tiene confirmación persistente; un cierre durante el disparo puede perder una ejecución. Su interpretación de cron es parcial.
- La recuperación conserva contexto, rol y paso, pero no serializa y restaura íntegramente el contrato y el grafo de evidencia del runtime. Las comprobaciones deben repetirse.
- Las huellas de contenido se obtienen de SHA-256 truncado a 64 bits. No constituyen una garantía criptográfica de ausencia de errores ni de alucinaciones.
- El rendimiento depende del modelo, hardware y herramientas. No se garantiza latencia cero ni equivalencia con servicios externos.
- Las herramientas web pueden transmitir las consultas o los datos que se les proporcionen. El uso de un modelo local no implica que toda actividad del agente sea exclusivamente local.

La revisión de septiembre de 2026 corrige conexiones y contradicciones verificadas. No certifica todas las herramientas, todos los lenguajes ni el comportamiento de cualquier modelo.
