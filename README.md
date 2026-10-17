# Aura Sentinel

Entorno de desarrollo con agente: interfaz Tauri (React + TypeScript), editor Monaco y **Agente Local** como
motor de las misiones (carpeta `motor/`), con el modelo servido por Bionic (LM Studio) en tu equipo. Todo es
local: el código, el modelo y tus proyectos no salen del ordenador.
Desarrollado por Ramón Antonio Burgos Jerez.

## Instalación en un equipo nuevo

### 1. Requisitos

| Qué | Para qué | Dónde |
|---|---|---|
| Windows 10/11 x64 | | |
| Node.js 24 LTS o superior | interfaz (Vite, React) | https://nodejs.org/ |
| Rust (toolchain `x86_64-pc-windows-msvc`) | backend de Tauri | https://rustup.rs/ |
| Build Tools de Visual Studio con «Desarrollo para el escritorio con C++» | compilar Rust en Windows | https://visualstudio.microsoft.com/visual-cpp-build-tools/ |
| WebView2 Runtime (viene con Windows 11) | ventana de la app | https://developer.microsoft.com/microsoft-edge/webview2/ |
| `uv` (recomendado) o Python 3.11+ | entorno del motor | https://docs.astral.sh/uv/ |
| Bionic o LM Studio | sirve el modelo (puertos 1235 / 1234) | https://lmstudio.ai/ |
| Edge o Chrome | el agente prueba ahí las páginas web | ya instalado en Windows |
| GPU NVIDIA (recomendado) y ~32 GB de RAM | velocidad del modelo | |
| ~20 GB libres en disco | el modelo y su visión | |

### 2. Instalar

```powershell
git clone https://github.com/yemsy26/aura-sentinel.git
cd aura-sentinel
powershell -ExecutionPolicy Bypass -File setup.ps1
```

`setup.ps1` se puede repetir (salta lo ya hecho) y no pide permisos de administrador:

1. Comprueba los requisitos de arriba y dice dónde conseguir lo que falte.
2. `npm install` de la interfaz.
3. Instala el motor (`motor\instalar.bat`): crea `motor\.venv`, instala sus dependencias, pasa sus pruebas y
   registra el MCP `agente-local` en Bionic y LM Studio.
4. Descarga y verifica el modelo de `motor\agente.json` (`motor\descargar_modelo.bat`: Qwen3.6-35B-A3B
   UD-Q3_K_XL, 16,8 GB, y su visión, 0,9 GB, desde Hugging Face; reanuda si se corta y comprueba el SHA-256).
5. Con Bionic abierto, lo ajusta a tu equipo (`motor\cargar_modelo.bat`): mide la VRAM, elige el contexto que
   cabe y cuántas capas de expertos del modelo van a la RAM/CPU, y aplica los ajustes de chat en Bionic.

Opciones: `-Si` (sin preguntas), `-SinModelo` (deja la descarga para después), `-SinVision`.
Qué cambia exactamente en Bionic/LM Studio y cómo deshacerlo: `motor\LEEME.md`.

### 3. Ajustes de Bionic recomendados

- Activa su servidor local (puerto 1235; LM Studio usa 1234). Aura usa el que responda.
- Desactiva la voz (Voxtral) si no la usas: ocupa 2-5 GB de VRAM que el modelo aprovecha para el contexto.
- Si acabas de descargar el modelo y Bionic no lo ve, reinícialo y ejecuta `motor\cargar_modelo.bat`.

### 4. Arrancar

`start.bat`: comprueba Bionic, pone al día las dependencias, libera el puerto 1420 si quedó ocupado por una
ejecución anterior, compila (la primera vez unos minutos) y abre Aura. En **Configuración → Motor** debe salir
«Motor instalado y listo».

### Actualizar

`git pull` y `start.bat` (instala solo lo que cambió). Si cambiaron las dependencias del motor, `motor\instalar.bat`.

### Instalador de Windows (.exe)

```sh
npm run tauri build
```

Deja `AuraSentinel_<versión>_x64-setup.exe` (NSIS, por usuario, sin administrador) y un `.msi` en
`src-tauri\target\release\bundle\` (o en `%CARGO_TARGET_DIR%\release\bundle\`). El instalador incluye el motor:
la primera vez, **Configuración → Motor → Instalar motor** crea su entorno de Python (necesita `uv` o Python).
El modelo se instala aparte (`descargar_modelo.bat` dentro de la carpeta `motor` de la app, o desde LM Studio).
Usa el **`setup.exe`**: instala en `%LOCALAPPDATA%\AuraSentinel`, donde el motor puede crear su `.venv`. El `.msi`
instala en `Program Files` y ahí «Instalar motor» solo funciona ejecutándolo como administrador.
Desinstalar no borra tu perfil ni tus proyectos (tampoco el `.venv` del motor: bórralo a mano si quieres). Identificador de la app: `com.yemsy.aurasentinel`.

### Si algo falla

| Síntoma | Solución |
|---|---|
| «Port 1420 is already in use» | `start.bat` ya cierra el servidor viejo de Aura; si es otro programa, te dice cuál cerrar |
| «Aura ya está abierta» al compilar | cierra la ventana de Aura y vuelve a ejecutar `start.bat` |
| «falta instalarlo (su entorno de Python)» | Configuración → Motor → Instalar motor, o `setup.ps1` |
| «No responde Bionic (:1235) ni LM Studio (:1234)» | abre Bionic y activa su servidor local |
| El modelo no aparece en la barra superior | reinicia Bionic tras la descarga y ejecuta `motor\cargar_modelo.bat` |
| Va muy lento o se queda sin memoria | desactiva la voz de Bionic; `motor\vision.bat off` libera ~0,9 GB; `motor\cargar_modelo.bat --remedir` |

Registro del motor: `%TEMP%\aura_agente_local.log`.

Para depurar la interfaz de la versión instalada, arráncala con
`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333` y abre `edge://inspect`: Aura añade sus
opciones (WebView2 sin GPU, para dejar la VRAM al modelo) a las que pongas.

Vite empaqueta Monaco con sus workers: el editor funciona sin conexión. Monaco (~4 MB) se carga aparte, así que
la ventana pinta en ~0,1 s con un paquete inicial de 358 KB.

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

Aura busca el motor en este orden: `AGENTE_LOCAL_DIR`, la carpeta guardada en Configuración → Motor
(`motor.json`), `motor\` del repositorio, `motor\` dentro de la app instalada y, por compatibilidad, las carpetas
de antes de unificarlo (`%USERPROFILE%\.gemini\antigravity\scratch\lm studio`, `%USERPROFILE%\AgenteLocal`).
Usa la primera que esté instalada; si ninguna lo está, ofrece instalar la primera que encuentre.

## Verificación

```sh
npm test
```

Comprueba los tipos (TypeScript), ejecuta las pruebas de interfaz con Vitest (simulan el puente de Tauri) y
las pruebas Rust. Las del motor: `motor\.venv\Scripts\python.exe -m unittest discover -s motor\tests`
(desde `motor\`). No sustituyen una prueba visual en WebView2 ni una misión con un modelo real.

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

- `setup.ps1`: instalación completa en un equipo nuevo; `start.bat`: arranque diario (con
  `scripts\preparar-inicio.ps1`).
- `motor/`: Agente Local (Python): el agente, sus pruebas, el instalador, la descarga del modelo y el ajuste a la
  VRAM. Su guía: `motor/LEEME.md`.
- `src/`: interfaz React. `App.tsx` (estado y conexiones), `components/` (barras, lateral, editor, panel
  inferior, agente, diálogos) y `lib/` (comandos de Tauri tipados, estado del agente, Monaco y utilidades).
- `src-tauri/src/lib.rs`: inicialización de Tauri y comandos.
- `src-tauri/src/agent.rs`: motor (proceso de Agente Local, eventos, modelos, misiones pendientes).
- `src-tauri/src/workspace.rs`: árbol de archivos, editor e historial del chat.
- `src-tauri/src/ide.rs`: CPU/RAM/VRAM, git, puntos de restauración, capturas, búsqueda, terminal y la copia
  del proyecto al empezar cada misión (base de "Ver cambios").
- `src-tauri/src/ask_user.rs` y `user_profile.rs`: preguntas al usuario y perfil local.
- `src-tauri/src/diagnostics.rs`: misión real sin ventana (`aura-diagnostics`).
