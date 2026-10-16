# Aura Sentinel

Entorno de desarrollo con agente: interfaz Tauri, editor Monaco y **Agente Local** como motor de las misiones,
con el modelo servido por Bionic (LM Studio) en tu equipo.
Desarrollado por Ramón Antonio Burgos Jerez.

## Ejecución

Requisitos: Node.js 24, Rust con herramientas de compilación de Windows, WebView2, Bionic (o LM Studio) y
Agente Local instalado (`instalar.bat` en su carpeta). `setup.ps1` comprueba todo e instala las dependencias.

```sh
npm install
npm run tauri dev      # o start.bat
```

El paso previo de Tauri copia Monaco y DOMPurify desde `node_modules` a `src/vendor`: el editor no necesita CDN.

## Cómo funcionan las misiones

Cada mensaje del chat lo ejecuta Agente Local (`src-tauri/src/agent.rs`). Aura lanza
`python -m agente <carpeta> --json` y muestra sus pasos, archivos y preguntas en la interfaz:

- `paso` → registro y telemetría (plan por fases, verificación, revisión); `archivos` → el editor recarga
  las pestañas abiertas; `confirmar` → ventana para permitir o bloquear comandos destructivos.
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

Prepara los recursos de la interfaz, comprueba JavaScript, ejecuta las pruebas de interfaz (simulan el puente
de Tauri) y las pruebas Rust. No sustituyen una prueba visual en WebView2 ni una misión con un modelo real.

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

- `src/`: interfaz, chat y editor.
- `scripts/`: preparación de dependencias locales y pruebas de interfaz.
- `src-tauri/src/lib.rs`: inicialización de Tauri y comandos.
- `src-tauri/src/agent.rs`: motor (proceso de Agente Local, eventos, modelos, misiones pendientes).
- `src-tauri/src/workspace.rs`: árbol de archivos, editor e historial del chat.
- `src-tauri/src/ask_user.rs` y `user_profile.rs`: preguntas al usuario y perfil local.
- `src-tauri/src/diagnostics.rs`: misión real sin ventana (`aura-diagnostics`).
