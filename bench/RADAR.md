# Radar mensual de Aura

Para que Aura no se quede atrás: una vez al mes se mira qué ha cambiado fuera (llama.cpp, los agentes de referencia, la
especificación MCP, modelos nuevos) y qué hay instalado en este equipo, y se escribe una página con lo que le importa a Aura y lo que
convendría probar. **El radar solo propone**: no cambia código ni mediciones. Cada propuesta sigue el método de siempre (condiciones
escritas antes, medición con el banco, commit con OK y push con otro OK).

## Cada mes

```powershell
python bench\radar.py                       # informe en ..\aura-radar\AAAA-MM.md y ultimo.json (fuera del repo: lleva el estado del equipo)
python bench\radar.py --desde 2026-10-25    # mirar desde otra fecha
python bench\radar.py --sin-red             # solo el estado local
python bench\radar.py --seco                # no avanza la ventana (la próxima vez mira desde donde miraba)
```

El script hace la parte mecánica:

1. **Estado local**: versión de Aura y commit del repo, motor SAT fijado, llama.cpp de LM Studio (versión del backend y commit de
   `llama-server --version` con CUDA oculto: no carga modelo ni usa VRAM), modelos instalados y versiones de python, node, go, cargo,
   docker y forge.
2. **Cambios desde la última revisión** en las fuentes con API (`radar_fuentes.json`): releases estables de los agentes y de la
   especificación MCP (las prereleases solo se cuentan), modelos nuevos de Qwen y, para llama.cpp, cuántos commits separan tu build
   de `master` y de la última release. Una fuente que no responde sale con su motivo y no tumba el informe. Solo hace peticiones GET
   públicas a `api.github.com` y `huggingface.co`, sin credenciales y sin datos del equipo en la URL.
3. **Fuentes «a mano»** (sin API estable): se listan con qué buscar en cada una.

Quien revisa (una persona o un agente) completa las secciones 4 y 5 del informe:

- **Qué le importa a Aura**: máximo 5 puntos, cada uno con su fuente primaria (changelog, release, especificación). Sin cifras de
  blogs ni de fabricantes: no son comparables; lo que importa se mide con el banco.
- **Qué probar**: una prueba por punto con su regla de decisión escrita antes. Una cosa a la vez.

## La puerta de entrada de modelos y motores

Un modelo nuevo, otra versión de llama.cpp o una mejora del motor no se adoptan por una tabla de internet: se miden con el banco y
pasan por la puerta.

```powershell
python bench\correr.py --prefijo m5001 --modelo OTRO_ID                    # la batería con el candidato (unas 2 h de GPU: pide OK)
python bench\correr.py --puerta ..\aura-bench\m5001.jsonl --contra ..\aura-mediciones\m4192.jsonl
```

Veredicto, por orden de precedencia (hacen falta las 12 misiones en las dos tandas):

| Veredicto | Cuándo |
|---|---|
| RECHAZAR | FUNCIONA baja, o hay traceback en alguna misión |
| REPETIR | alguna misión cae de FUNCIONA a PARCIAL o FALLA: con n = 1 es ruido probable. Se repiten esas misiones (`--rep 2` y `--rep 3`), se añaden las filas al `.jsonl` y decide la mayoría |
| ADOPTAR | no hay caídas y FUNCIONA sube, o con el mismo FUNCIONA los minutos de las misiones FUNCIONA en las dos bajan a 80 % o menos |
| NO ADOPTAR | sin mejora: lo nuevo no compensa el cambio |
| SIN VEREDICTO | falta alguna de las 12 misiones |

El veredicto no ejecuta nada: decide el usuario con la evidencia delante. Ejemplo real: m4192 frente a m4185 da REPETIR
(`tienda_roles` cayó de FUNCIONA a FALLA con 200 pasos de tope) aunque FUNCIONA sube de 6 a 10.

## Tarea programada (opcional, pide OK)

Programarlo es configuración persistente: solo con OK del usuario. Texto para una tarea el primer lunes de cada mes:

> En el repo aura-sentinel ejecuta `python bench\radar.py`. Lee el informe que escribe en `..\aura-radar`, abre las fuentes de la
> sección 3 y completa las secciones 4 y 5 (máximo 5 puntos, con fuente primaria; sin cifras de blogs). No cambies código ni
> mediciones, no uses la GPU y no hagas commit. Avisa con un resumen de tres líneas y la ruta del informe.

## Límites

- Detecta cambios en fuentes con API y recuerda las demás; juzgar si algo importa lo hace quien revisa.
- Los repos de agentes se mueven (`anomalyco/opencode` antes era `sst/opencode`; GitHub avisa con «Moved Permanently»): si una fuente
  deja de responder, corregir su `repo` en `radar_fuentes.json`.
- La puerta con n = 1 por misión detecta regresiones grandes, no diferencias finas; por eso el escalón REPETIR.
