# Agente Local (motor de Aura Sentinel)

Agente de programación 100 % local para Bionic / LM Studio: planifica, escribe, ejecuta, verifica (tests, web en
Edge sin ventana, juegos pygame, capturas de pantalla) y corrige hasta que funciona. Es el motor de las misiones de
Aura Sentinel (carpeta `motor/` del repositorio) y también se usa solo, en consola o como MCP. Funciona con cualquier
modelo con tool-calling; el de `agente.json` es **Qwen3.6-35B-A3B UD-Q3_K_XL** (MoE) con visión, probado en una
RTX 4060 de 8 GB con 32 GB de RAM.

## Instalación (en este o en otro equipo)

Lo más sencillo es `setup.ps1` en la raíz del repositorio: hace todo lo de abajo. A mano, desde esta carpeta:

1. `instalar.bat`: crea `.venv` (con `uv` si está; si no, con `python -m venv`), instala `mcp`, `ruff`, `vulture`
   y `playwright` (usa el Edge o Chrome del equipo: no descarga navegadores), pasa las pruebas y registra el MCP.
2. `descargar_modelo.bat`: baja de Hugging Face (`unsloth/Qwen3.6-35B-A3B-GGUF`) el modelo (16,8 GB) y su visión
   (`mmproj-F16.gguf`, 0,9 GB) a la carpeta de modelos de LM Studio. Reanuda si se corta y comprueba el SHA-256
   publicado; si ya están y cuadran, no baja nada. `--sin-vision` omite la visión.
3. Abre Bionic (o LM Studio con su servidor local activo) para que vea el modelo.
4. `cargar_modelo.bat`: mide tu VRAM y deja el modelo cargado con el mayor contexto que cabe (ver más abajo).
5. `.venv\Scripts\python.exe -m agente.instalar --configurar-modelo auto`: ajustes de chat del modelo en Bionic.

Requisitos: Windows 10/11, Python 3.11+ o `uv`, Bionic o LM Studio, Edge o Chrome. Con GPU NVIDIA va mucho más
rápido; el MoE necesita ~10 GB de RAM libre para sus expertos.

## Qué cambia en Bionic / LM Studio (y cómo deshacerlo)

Todo lo que se toca guarda antes una copia `.bak` junto al original (y no la pisa si ya existe).

| Qué | Dónde (`%USERPROFILE%\.lmstudio\...`) | Lo hace |
|---|---|---|
| MCP `agente-local` (este motor como herramienta del chat) | `mcp.json` y `apps\bionic\.internal\ng-mcp.json` | `instalar.bat` |
| `bionic-core` desactivado (choca con las herramientas del agente) | `apps\bionic\.internal\ng-mcp.json` | `instalar.bat` |
| Carga del modelo: contexto medido, caché KV q8_0, flash attention, 1 sesión, sin mlock, 8 puntos de caché y, en MoE, cuántas capas de expertos van a la RAM/CPU | `apps\bionic\.internal\user-concrete-model-default-config\<repo>\<gguf>.json` | `cargar_modelo.bat` (en cada carga) |
| Chat: prompt de sistema del MCP, muestreo recomendado, razonamiento máx. 2048 tokens | el mismo archivo | `agente.instalar --configurar-modelo` |
| Visión apagada/encendida (aparta o devuelve el `mmproj`) | `models\...` ↔ `mmproj-desactivados\...` | `vision.bat off` / `on` |

Deshacer: restaura los `.bak` (con Bionic cerrado) y borra la entrada `agente-local` de los dos JSON del MCP. Las
medidas de VRAM se guardan aquí en `.cache_modelos.json` (de cada equipo; `cargar_modelo.bat --remedir` las rehace).

## Uso rápido

| Archivo | Para qué |
|---|---|
| `instalar.bat` | Crea `.venv`, pasa las pruebas y registra el MCP `agente-local` en Bionic y LM Studio |
| `descargar_modelo.bat` | Descarga y verifica el modelo de `agente.json` y su visión |
| `cargar_modelo.bat` | Carga el modelo con el **mayor contexto que cabe en tu VRAM libre** (sin desbordar a RAM compartida) |
| `agente.bat "C:\ruta\proyecto"` | Abre el agente en esa carpeta (consola interactiva) |
| `probar.bat` | Pruebas internas + prueba real: el modelo crea un proyecto y se verifica que sus tests pasan |
| `vision.bat off` / `on` | Quita/restaura el proyector de visión (mmproj): libera ~0,9 GB de VRAM |

Dentro del agente: escribe la tarea. Comandos: `/plan`, `/contexto`, `/nuevo`, `/salir`. Texto largo: entre `"""`.
Tarea única sin consola: `agente.bat "C:\proyecto" --tarea "crea ..."`.

## Cómo reparte el modelo entre VRAM y RAM (medido en una RTX 4060 de 8 GB)

| Qué ocupa VRAM | MiB |
|---|---|
| Bionic con la voz Voxtral cargada | **2069** (crece hasta ~5600): desactívala en Bionic si no la usas |
| Qwen3.5 9B con visión (mmproj) | 5741 fijos + 22 por cada 1K de contexto |
| Qwen3.5 9B sin visión | 4500 fijos + 21 por cada 1K de contexto |
| Qwen3.6-35B-A3B (sin expertos) con visión | 3685 fijos + 18,5 por cada 1K de contexto |
| Techo útil en Windows (WDDM) | total − ~300 |

Si se pasa del techo, Windows desborda a RAM compartida y todo va muy lento; por eso `cargar_modelo.bat` mide antes
y elige el contexto que cabe (entre 8K y 64K, `agente.json` → `carga`).

Ajustes aplicados al cargar: GPU completa, caché KV **q8_0**, flash attention, **1 sola sesión paralela**
(las sesiones extra reservan memoria que el agente no usa), **sin mlock** (no bloquea en RAM una copia entera
del modelo) y **8 puntos de restauración** de caché en vez de 32 (cada uno guarda estado en RAM).
Con el 9B el servidor pasa de ~14 GB de RAM a ~7 GB recién cargado.

### Modelos MoE (p. ej. Qwen3.6-35B-A3B)

`cargar_modelo.bat` los detecta leyendo el GGUF: pone atención y caché en la GPU, elige el contexto como con
cualquier modelo y llena la VRAM que sobra con los expertos de las últimas capas; el resto va a la CPU
(`--n-cpu-moe`). Usa lotes de 2048 (el prompt se procesa ~2× más rápido con expertos en CPU).
Medido con voz OFF:

| Modelo | Contexto | Colocación | VRAM | Prompt | Generación |
|---|---|---|---|---|---|
| Qwen3.5 9B Q3_K_XL | 64K | todo en GPU | 5893 MiB | 1982 tok/s | 39,7 tok/s |
| Qwen3.6-35B-A3B UD-Q3_K_XL | 64K | expertos de 30/40 capas en CPU | 7071 MiB | 911 tok/s | 29,0 tok/s |
| Qwen3.6-35B-A3B UD-Q3_K_XL **con visión** | 64K | expertos de 33/40 capas en CPU | 7183 MiB | 800 tok/s | 27,4 tok/s |

El MoE terminó el juego de naves de prueba (verificado y 16/16 comprobaciones funcionales); el 9B no lo logró
en 7 intentos. Es el modelo de `agente.json`: `cargar_modelo.bat` lo carga así (16,8 GB en disco; sus expertos usan ~10 GB de RAM).
El GGUF se abre con mmap: la primera vez los expertos que van a la CPU se leen del disco (las primeras respuestas
van más lentas: se midió 13 tok/s en frío y 29 tok/s después) y luego quedan en la caché de RAM de Windows. Sin mlock,
Windows puede liberar esa RAM si otro programa la necesita, en vez de bloquear una copia entera del modelo.

## Cómo evita gastar tokens y perderse (no depende del modelo)

- **Método fijo**: entender → `plan` → construir → instalar (`.venv`) → probar → `crear_bat` → `terminar`.
- **Puerta de verificación**: `terminar` se rechaza si modificó archivos y no ejecutó nada con éxito después,
  o si quedan pasos del plan sin marcar.
- **Razonamiento solo cuando sirve** (`razonamiento: auto`): piensa al planificar y tras un error; el resto de pasos
  van sin `<think>` (en una tarea de 20 pasos ahorra miles de tokens).
- **Presupuesto de contexto**: salidas recortadas (conserva el final, donde están los errores); al 60 % se acortan
  resultados viejos y contenidos ya escritos en disco; si no basta, el modelo resume y sigue con TAREA + RESUMEN + PLAN.
- **Anti-bucles**: avisa a la 3ª llamada idéntica y se detiene a la 5ª. Si responde sin actuar con pasos pendientes, se le empuja a seguir.
- **Robustez**: corrige tipos mal enviados (`"3"`, `"true"`), entiende llamadas escritas como texto (`<tool_call>`),
  valida sintaxis de `.py`/`.json` al escribir, `editar` tolera diferencias de sangría, mata árboles de procesos en timeout.
- Registro de cada paso en `<proyecto>\.agente\registro.log`; plan en `.agente\plan.md`; resumen final en `.agente\resumen.md`.

## Herramientas

`plan`, `paso_hecho`, `listar`, `leer`, `buscar`, `escribir`, `editar`, `ejecutar` (con `fondo=true` para servidores;
crear un venv o `pip`/`npm`/`uv install` tiene siempre al menos 300 s para no dejar el entorno a medias),
`procesos`, `crear_bat`, `web_buscar` (DuckDuckGo, Bing de respaldo), `web_leer`, `investigar`, `guia`, `version`,
`verificar`, `revisar`, `aprender`, `preguntar`, `terminar`.
`preguntar(pregunta, opciones)` pide un dato imprescindible o una decisión al usuario (2-5 opciones y respuesta libre)
y espera la respuesta; como mucho 3 veces por tarea. Sin usuario (MCP, `--tarea`), el agente decide y lo explica.
`ver_pantalla` captura la página (`.html` o `http://localhost:PUERTO`) o el juego pygame y señala texto cortado,
elementos fuera de pantalla o pantalla en negro; si el modelo cargado ve imágenes, la juzga él con su
`mmproj-F16.gguf` (Qwen3.6: 6-7 s por captura). Su juicio es indulgente: puede dar OK a un título tapado por
figuras 3D; una lista de comprobación con razonamiento no lo arregló de forma fiable (probado con varias muestras).
Por eso marca sin modelo el texto sobre fondo cargado (letras que se confunden con lo de detrás, líneas o una figura de
canvas/svg tapando parte de un título; con canvas o vídeo detrás mira 4 instantes y guarda la captura del peor).
Con el mmproj el MoE sigue en 64K de contexto: 7183 MiB, expertos de 33/40 capas en CPU, 800/27,4 tok/s (antes 911/29,0).
`git`: estado, diff, log, commit local y rama; nunca `push`; lo destructivo (`reset --hard`...) pide confirmación.

## Cómo detecta que algo está mal (como los agentes avanzados)

1. **Al escribir cada archivo**: sintaxis + lint inmediato (`.py` con ruff: nombres sin definir, variables sin usar;
   `.js` con `node --check`; `.go` con `gofmt -e`; `.rs` con rustfmt; `.html` referencias rotas; `.bat` archivos que
   no existen; Dockerfile; tabuladores en YAML). Avisa si el archivo lleva una credencial escrita (clave, token).
2. **`verificar`** (y automático antes de terminar):
   - Python: ruff, **vulture** (funciones/métodos que nunca se llaman = lógica sin conectar), tests (y detecta tests
     falsos que no importan el proyecto), ejecución del programa.
   - **pygame**: juega 3 partidas sin ventana con teclas aleatorias, reloj acelerado (minutos de juego en segundos) y
     semillas fijas (reproducible): excepciones (p. ej. choques que dejan la nave en `None`), coste por frame, si la
     pantalla cambia y si el juego se cierra.
   - Clases y funciones que **nunca se usan** cuentan como ERROR (en código generado casi siempre es una pieza sin
     conectar: p. ej. una clase `Jugador` que nunca se crea deja el juego sin nave). Las decoradas (rutas Flask) no.
   - **HTML/JS**: lo abre en **Edge sin ventana**: errores de consola/JS, 404, ids inexistentes, canvas vacío,
     si anima y si reacciona al teclado/ratón; capturas en `.agente\capturas\`.
   - **Java**: `javac -Xlint` con línea de cada error y ejecución del `main` (o `mvn test` si hay Maven).
   - **Rust**: `cargo build --all-targets` (errores y avisos con archivo:línea), `cargo test` y avisos de clippy.
   - **Go**: `go vet ./...` (compila y detecta fallos típicos), `go test ./...` y aviso si falta `gofmt`.
   - **.NET**: `dotnet build` con archivo:línea de cada error y `dotnet test` en los proyectos de pruebas.
   - **Node/TypeScript**: dependencias instaladas, `tsc --noEmit`, `npm run build` si existe y `npm test`.
   - **Docker**: revisa el Dockerfile (versión fija de la imagen, `COPY` de archivos que no existen, credenciales en
     `ENV`/`ARG`, usuario sin privilegios, `.dockerignore`), valida `compose.yaml` y, con Docker en marcha, construye
     la imagen. Con Docker apagado lo dice para que el agente se lo pida al usuario.
   - **YAML**: tabuladores (no válidos en YAML) en manifiestos y workflows.
   - Si falta la herramienta de un lenguaje (cargo, go, dotnet, npm, docker), el informe dice dónde instalarla.
3. **`revisar`** (automático una vez antes de terminar): el modelo revisa en un contexto limpio cada requisito contra
   el código, con número de línea. Con tu juego de naves detectó: `hits` sin efecto, `show_menu`/`reset` nunca
   llamados, pausa inexistente y test falso.
4. **Puertas de conducta**: no puede terminar con errores de verificación, revisión con puntos CRÍTICOS (la última
   revisión se guarda y bloquea aunque se reanude otro día), archivos pedidos que faltan o pasos sin hacer; no puede
   marcar "probar" sin haber ejecutado; la 4ª reescritura completa de un archivo válido se bloquea (obliga a `editar`);
   rechaza contenido que sea un marcador o esté truncado; avisa si borra tests o si pega código en el chat.
5. **Checkpoints**: cada `verificar` sin errores guarda la versión que funciona. Si después encadena fallos se le
   sugiere `restaurar`; si la tarea se acaba con el proyecto roto, **se restaura sola** la última versión buena.
6. **Mapa del proyecto** al compactar el contexto: archivos con sus clases/funciones reales, para que no invente
   nombres de módulos tras olvidar el historial.

## Orientación actualizada e internet

- `guia(tema)`: prácticas actuales y cómo verificar (python, pygame, html-juego, web, api, java, rust, go, node,
  dotnet, docker, despliegue, tests, bat). Son archivos en `agente\guias\*.md`: puedes editarlos o añadir los tuyos.
- El agente conoce el entorno al empezar: versiones de Python, Node, Java, Rust, Go, .NET, si Docker está en marcha,
  Podman, OpenSSH y kubectl. Las compilaciones e instalaciones largas (`cargo`, `go`, `docker build`, `dotnet`,
  Maven/Gradle, `npm install`...) tienen al menos 5 minutos antes de cortarse.
- `version(paquete, ecosistema)`: versión real en PyPI, npm, Maven, crates.io (Rust), el proxy de Go (avisa si el
  módulo está obsoleto), NuGet (.NET) y Docker Hub (etiquetas para `FROM`, sin `latest`); en PyPI, si hay wheel para tu Python (p. ej. `pygame` → usa `pygame-ce`
  porque `pygame` no tiene wheel para Python 3.14).
- `investigar(pregunta)`: busca, lee las mejores páginas y devuelve solo los párrafos útiles con su fuente, en un paso.

## Corregir conductas del agente

- En la consola del agente: `/regla nunca uses emojis en los textos del juego` → se guarda en `reglas.md` y se aplica
  ya y en todas las tareas (prioridad máxima). `/reglas` las muestra. También puedes editar `reglas.md` a mano.
- El agente guarda sus propias lecciones con `aprender` en `lecciones.md` (máx. 40, las más recientes).
- Por proyecto: crea `AGENTE.md` en la carpeta del proyecto con instrucciones específicas.
Comandos destructivos (`rmdir /s`, `del /s`, `format`, `reg delete`...) piden confirmación. También lo que sale del
equipo: `ssh`, `scp`, `sftp`, `rsync`, `git push`, `docker push`/`login`, `kubectl apply`/`delete`, `helm`,
`terraform apply`, publicar paquetes (`npm`/`cargo publish`), nubes (`az`, `aws`, `gcloud`...) y peticiones
HTTP que escriben (`curl -X POST`, `-d`...) a algo que no sea `localhost`. Sin tu permiso, se bloquea.
**`ejecutar` no es un sandbox**: corre con tus permisos. Úsalo en carpetas de proyecto.

## Usarlo desde Bionic / LM Studio (MCP)

`instalar.bat` registra `agente-local` (mismas herramientas + `proyecto(ruta)`) y desactiva `bionic-core` en Bionic
para no duplicar herramientas (copia en `ng-mcp.json.bak`). Pega `PROMPT_SISTEMA_MCP.txt` como prompt de sistema del modelo.
Para dejar también listos los ajustes de inferencia del modelo en Bionic:
`instalar.bat --configurar-modelo Manojb/Qwen3.5-9B-UD-Q3_K_XL.gguf/Qwen3.5-9B-UD-Q3_K_XL.gguf.json`

El agente de consola (`agente.bat`) es mejor que el chat + MCP para tareas largas: controla el contexto y verifica.

## Usarlo desde un IDE (modo `--json`)

`python -m agente CARPETA --json [--modelo CLAVE]` hace lo mismo que la consola, pero habla por JSON
(una línea por mensaje), para que un IDE (p. ej. Aura Sentinel) muestre los pasos en su interfaz.
Si el modelo no está cargado, lo carga con el ajuste de VRAM.

- Órdenes por la entrada: `{"tipo":"tarea","texto":"..."}`, `{"tipo":"regla","texto":"..."}`,
  `{"tipo":"nuevo"}`, `{"tipo":"verificar"}`, `{"tipo":"plan"}`, `{"tipo":"salir"}`.
- Eventos por la salida: `listo` (modelo, contexto), `paso` (herramienta, detalle, resultado, error, plan),
  `archivos` (rutas cambiadas; vacía = refrescar todo), `nota`, `aviso`, `progreso`, `error`.
- `confirmar` (`comando` y `motivo`: destructivo, o conectarse o publicar fuera del equipo) espera
  `{"tipo":"respuesta","si":true|false}`.
- `pregunta` (`pregunta` y `opciones`) espera `{"tipo":"respuesta","texto":"..."}`.
- Cada orden termina con `fin` (`estado`: ok, incompleta o error; `respuesta`; `uso` del contexto).

El proceso conserva la conversación entre tareas, como la consola. Todo lo que no es un evento va a stderr.

## Configuración (`agente.json`)

`servidor`/`modelo` en `auto` usan Bionic (:1235) o LM Studio (:1234) y el LLM cargado. `muestreo` usa los valores
de Qwen para código (temperature 0.6, top_k 20) con `repeat_penalty` 1.05 contra bucles. `carga.margen_vram_mib`
es la VRAM que se deja libre para Windows y otras apps. Si cambias de modelo, `cargar_modelo.bat --modelo CLAVE`
lo mide la primera vez (2 cargas de prueba) y lo recuerda en `.cache_modelos.json` (`--remedir` para repetir).
