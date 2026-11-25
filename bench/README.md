# Banco de evaluación de Aura

Doce misiones con juez automático para saber, con números y sin fiarse de lo que dice el agente, si un cambio ayuda o empeora:
una mejora del motor, un modelo nuevo o una versión nueva de llama.cpp. Salió de las baterías m4185 y m4192 (la segunda, con el
motor ya corregido, dio 10 de 12). Solo añade esta carpeta: el motor no cambia y el instalador no la lleva.

## Requisitos

- Windows, el venv del motor (`motor\.venv`; si no existe se usa el Python que lanza el banco) y `git`.
- El modelo en LM Studio o Bionic (por defecto `qwen3.6-35b-a3b`, el cerebro de referencia). **Ocupa la GPU unas 2 h**: no usar Aura ni
  la GPU mientras corre. `correr.py` descarga el modelo cargado, comprueba que la GPU quedó libre y carga el que se pide; si otro
  proceso la ocupa, para.
- Para que no salgan «no pasa» por falta de herramientas: `node`, `go`, `cargo`, `docker` y `forge` (Foundry) en el PATH.

## Uso

```powershell
python bench\correr.py                                  # las 12 con el motor del repo y Qwen3.6
python bench\correr.py bug_py csv_ventas                # solo esas
python bench\correr.py --motor C:\ruta\motor_congelado  # otra copia del motor
python bench\correr.py --modelo OTRO_ID                 # otro modelo
python bench\correr.py --salida D:\medidas --prefijo m5001 --rep 2
python bench\correr.py --resume                         # salta las ya anotadas en <prefijo>.jsonl
python bench\correr.py --seco                           # fixtures y jueces sobre el estado de partida, sin modelo
python bench\correr.py --rejuzgar --salida C:\Proyectos --prefijo m4192   # solo los jueces, sobre carpetas ya hechas
python bench\correr.py --resumen m4192.jsonl            # Q1/Q2/Q3 de un resultado
python bench\correr.py --resumen nuevo.jsonl --contra anterior.jsonl     # y el veredicto de la otra tanda al lado
```

Cada misión crea su carpeta `<salida>\<prefijo>_<misión>` (por defecto `..\aura-bench\bench_<misión>`, que **debe estar vacía**),
guarda lo que imprimió el agente en `<carpeta>_salida.txt` y añade una línea a `<salida>\<prefijo>.jsonl`: pasos, minutos, si
terminó (`FIN:`), «Detenido: repitió X», traceback, llamadas por herramienta con sus errores, último `verificar`, resultado del juez
y veredicto. TEMP y TMP apuntan a `<salida>\tmp` mientras corre (un test del motor ya borró `%TEMP%` una vez: nunca medir con el
real). Cada misión tiene su tope de tiempo; la que lo pasa se corta y cuenta como que no termina.

## Reglas

- **Q1** FUNCIONA = termina (`FIN:`) **y** el juez pasa. PARCIAL = una de las dos. FALLA = ninguna, o el proceso cae (traceback).
- **Q2** con 12 misiones: 10 o más FUNCIONA → base sólida; 7 a 9 → corregir los fallos concretos antes de añadir nada, por frecuencia;
  6 o menos → no pasar a mejoras nuevas. Q2 solo vale con las 12: con una tanda parcial (p. ej. `bug_py csv_ventas`, que sirve de
  humo para comprobar el cableado) el resumen dice «sin veredicto» y no lista las herramientas nunca llamadas.
- **Q3** herramienta con 8 o más llamadas y 25 % o más de errores, o nunca llamada (se dice si es por diseño de la batería).
- Un fallo con n = 1 es un indicio; uno que se repite en varias misiones o mata el proceso pesa más. El motor no se toca durante una
  batería. Las imágenes de Docker que deja el agente se listan y se preguntan antes de quitarlas.

Además de la letra, conviene auditar: que ninguna misión toque `.agente/` (líneas `BLOQUEADO` y `MANIPULACIÓN` en `registro.log`),
que el resumen final diga lo hecho, y abrir a mano los `run.bat`/`test.bat` de las misiones de lenguaje.

## Las 12 misiones

| clave | tope | qué pide | qué juzga |
|---|---|---|---|
| `git_conflicto` | 20 min | fusionar la rama `ajustes` en `main` con conflictos | árbol limpio, `ajustes` dentro de `main`, `config.json` y README con lo de las dos ramas |
| `bug_py` | 30 | arreglar el programa para que pasen las pruebas sin tocarlas | las pruebas pasan y el archivo de pruebas sigue igual (sha256) |
| `csv_ventas` | 45 | informe de `ventas.csv` con doble clic y pruebas | totales por producto y mejor día en la salida del `.bat`, y las pruebas pasan |
| `mochila_sat` | 40 | la mejor mochila de 28 objetos (capacidad 120) | `resultado.txt` con el valor óptimo y sin pasarse de peso |
| `investigar_web` | 30 | última versión estable de Python, Node.js y Rust | las versiones salen de las fuentes oficiales (consultadas al juzgar) |
| `node_palabras`, `go_palabras`, `rust_palabras` | 45 / 45 / 60 | contar las 10 palabras más repetidas de un texto, con doble clic y pruebas | las 10 con su cuenta exacta y las pruebas del lenguaje pasan |
| `escritorio_gastos` | 60 | programa de escritorio para anotar gastos | el `verificar` del propio motor da 0 errores y 0 avisos |
| `docker_app` | 45 | poner una app web en un contenedor | la imagen se construye y el contenedor responde 200 |
| `evm_token` | 60 | contrato de un token transferible | `forge test` pasa con las pruebas del agente (solo `src/`; vale heredar de ERC20) |
| `tienda_roles` | 100 | negocio de ferretería con jefe y empleado | el recorrido con navegador del motor: 0 errores y 0 avisos |

El `.bat` de arranque puede estar en la raíz o, si no hay, en la carpeta del proyecto (un nivel): así quedó `rust_palabras` en m4192
tras el rechazo de `crear_bat` (C15) y se ejecuta y prueba desde su carpeta. Un juez que solo miraba la raíz lo marcaba PARCIAL.

## Medir una mejora, un modelo o llama.cpp

1. Escribir antes las reglas de decisión (qué resultado hace que se integre y cuál que no).
2. Congelar el motor: `git archive <commit> motor | tar -x -C D:\medidas\motor_NNNN` y pasar `--motor D:\medidas\motor_NNNN\motor`.
3. Mismo modelo, mismas misiones, `--prefijo` nuevo; para un modelo nuevo o llama.cpp, el mismo motor y `--modelo`.
4. `--resumen nuevo.jsonl --contra anterior.jsonl` y mirar misión por misión, no solo el total.

## Añadir una misión

En `misiones.py`: una función `fx_x(c)` que deja la carpeta de partida y devuelve el contexto del juez (totales esperados, hashes),
una `juez_x(c, ctx)` que devuelve `(ok, detalle)` mirando solo lo que quedó en disco, y una entrada en `MISIONES`:
`clave: (tope_en_minutos, mandato, fx_x, juez_x)`. El mandato se escribe como lo diría el usuario, sin nombrar herramientas de Aura.
Si el juez necesita red, docker o un programa, añadir la clave a `SIN_JUEZ_EN_SECO` en `correr.py`.
Prueba nueva en `motor/tests/test_bench.py` (fixture y juez sobre el estado de partida).

## Límites

- Son 12 misiones con n = 1: detecta regresiones grandes y fallos de integración, no diferencias finas. No es una puntuación
  comparable con SWE-bench u otros bancos públicos.
- Los jueces tienen historia: en m4185 dos eran míos y estaban mal (pytest frente a unittest; `PermissionError` y exigir
  `function transfer` en el contrato) y en m4192 el de rust miraba solo la raíz. Si una misión sale «mal» y el entregable funciona,
  mirar primero el juez.
- Windows solamente (el motor y las misiones lo son). Necesita red para `investigar_web` y deja imágenes de Docker.
