# Aura Sentinel

Entorno de desarrollo con agente: interfaz Tauri (React + TypeScript), editor Monaco y **Agente Local** como
motor de las misiones, con el modelo servido por Bionic (LM Studio) en tu equipo.
Desarrollado por Ramón Antonio Burgos Jerez.

## Ejecución

Requisitos: Node.js 24, Rust con herramientas de compilación de Windows, WebView2, Bionic (o LM Studio) y
Agente Local instalado (`instalar.bat` en su carpeta). `setup.ps1` comprueba todo e instala las dependencias.

```sh
npm install
npm run tauri dev      # o start.bat (Vite en :1420 + ventana de Tauri)
```

Vite empaqueta Monaco con sus workers: el editor funciona sin conexión.

## La interfaz

- **Barra superior**: carpeta del proyecto (con recientes), paleta `Ctrl+K` (archivos y comandos), modelo de
  Bionic con su contexto, uso de VRAM y configuración.
- **Lateral**: explorador con marcas **M/A** de lo que cambió o creó el agente en la misión, búsqueda en el
  proyecto, git (rama, cambios, commits), puntos de restauración del agente y sus reglas y lecciones.
- **Editor**: pestañas Monaco; si el agente cambió el archivo, un aviso con `+N −M` líneas, **Ver cambios**
  (editor de diferencias frente a cómo estaba al empezar la misión) y **Deshacer**.
- **Panel inferior** (`Ctrl+J`): terminal (comandos en la carpeta del proyecto), problemas de la última
  verificación y de la revisión visual, procesos que dejó el agente (servidores) y vista previa de
  `http://localhost:PUERTO`.
- **Agente**: estado, paso y tiempo, contexto usado, plan con su avance, una tarjeta por paso (con la salida
  y la captura de `ver_pantalla`), confirmación de comandos en línea, misión pendiente, acciones rápidas
  (Verificar, Revisar código, Continuar, Nueva conversación) y `@archivo` en el mensaje.

## Cómo funcionan las misiones

Cada mensaje del chat lo ejecuta Agente Local (`src-tauri/src/agent.rs`). Aura lanza
`python -m agente <carpeta> --json` y muestra sus pasos, archivos y preguntas en la interfaz:

- Cada evento llega a la interfaz tal cual (`agent-event`): `paso` → tarjeta y plan; `archivos` → el editor
  recarga las pestañas abiertas; `confirmar` → tarjeta para permitir o bloquear comandos destructivos.
- Las acciones rápidas usan órdenes del protocolo en la misma sesión (`agent_order`): verificar, revisar,
  procesos, detener, restaurar, regla y nuevo.
- El agente planifica, escribe, ejecuta, verifica (Python, web en Edge sin ventana, Java, .bat), revisa el
  código y no termina con errores. El selector de modelos lista los de Bionic; elegir otro lo carga ajustado
  a la VRAM.
- La conversación se conserva mientras no cambies de carpeta ni de modelo. Cancelar detiene el agente y todo
  lo que lanzó.
- Si una misión queda a medias (plan con pasos sin marcar en `.agente/plan.md`), al abrir la app aparece
  **Reanudar**; descartarla aparta ese plan.
- El perfil del usuario (nombre, país, idioma) se envía al agente en el primer mensaje de cada sesión.

La carpeta de Agente Local se busca en `AGENTE_LOCAL_DIR`, en `motor.json` (configuración de la app) y en
`%USERPROFILE%\.gemini\antigravity\scratch\lm studio` o `%USERPROFILE%\AgenteLocal`.
El registro del proceso queda en `%TEMP%\aura_agente_local.log`.

## Verificación

```sh
npm test
```

Comprueba los tipos (TypeScript), ejecuta las pruebas de interfaz con Vitest (simulan el puente de Tauri) y
las pruebas Rust. No sustituyen una prueba visual en WebView2 ni una misión con un modelo real.

Si el lanzador `npm` de Windows falla buscando `npm-cli.js` en otra instalación:

```powershell
node "C:\Program Files\nodejs\node_modules\npm\bin\npm-cli.js" test
```

No copies carpetas `target` entre ubicaciones del proyecto: conservan rutas absolutas.

### Misión real sin ventana

`cargo build --manifest-path src-tauri/Cargo.toml --bin aura-diagnostics` y después
`aura-diagnostics.exe --diagnose-mission job.json`, con `workspace` (ruta absoluta), `prompt` y, si quieres,
`model` (clave de Bionic). Ejecuta el motor real y modifica esa carpeta. Guarda los eventos en
`.aura/diagnostic-events.jsonl` y el resultado en `.aura/diagnostic-result.json`; termina con código 0 solo
si la misión acaba en `FINISH`.

## Estructura

- `src/`: interfaz React. `App.tsx` (estado y conexiones), `components/` (barras, lateral, editor, panel
  inferior, agente, diálogos) y `lib/` (comandos de Tauri tipados, estado del agente, Monaco y utilidades).
- `src-tauri/src/lib.rs`: inicialización de Tauri y comandos.
- `src-tauri/src/agent.rs`: motor (proceso de Agente Local, eventos, modelos, misiones pendientes).
- `src-tauri/src/workspace.rs`: árbol de archivos, editor e historial del chat.
- `src-tauri/src/ide.rs`: CPU/RAM/VRAM, git, puntos de restauración, capturas, búsqueda, terminal y la copia
  del proyecto al empezar cada misión (base de "Ver cambios").
- `src-tauri/src/ask_user.rs` y `user_profile.rs`: preguntas al usuario y perfil local.
- `src-tauri/src/diagnostics.rs`: misión real sin ventana (`aura-diagnostics`).
