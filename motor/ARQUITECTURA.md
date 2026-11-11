# Arquitectura del motor

Cómo está organizado `motor/agente`. Las reglas de abajo no son solo texto: las comprueba `tests/test_estructura.py`.

## Reglas

1. **Sin ciclos.** Cada módulo solo importa de capas inferiores.
2. **Las conexiones se leen en la cabecera.** Nada de importar dentro de una función, salvo dos comandos sueltos que no deben cargar lo demás al arrancar (`__main__` → `prueba_modelo`, `instalar` → `descargar`).
3. **Cada nombre se importa de donde vive,** nunca de otro módulo que solo lo importa a su vez.
4. **Lo privado no cruza de módulo.** Un nombre con `_` delante solo se usa en su archivo; si otro lo necesita, deja de llevarlo.
5. **Lo que el modelo ve no cambia sin querer.** `esquemas.py` y `prompts.py` afectan a todas las misiones; un cambio ahí se mide en misiones reales antes de integrarlo.

## Capas

De abajo arriba; cada capa usa solo las anteriores.

| Capa | Módulos | Qué hacen |
| --- | --- | --- |
| Base | `config`, `entorno`, `procesos`, `llm`, `metricas`, `android`, `inversa` | Configuración, entorno de los comandos, control de procesos, cliente del modelo, métricas; SDK de Android y herramientas de ingeniería inversa (apktool, jadx) |
| Contratos y reglas | `esquemas`, `permisos`, `borrados`, `proyecto` | Qué puede pedir el modelo; qué se puede hacer sin preguntar; qué código borra fuera de lo suyo; qué es código, el `Informe` y cómo correr un comando |
| Modelo | `compat`, `modelo`, `descargar`, `prueba_modelo`, `llamadas` | Fichas de modelos, carga en la VRAM, descargas, prueba de un modelo, lectura de las llamadas que escribe |
| Conocimiento | `pasajes`, `conocimiento`, `lecciones`, `mapa`, `idiomas`, `complementos`, `matematicas` | Lo investigado, guías, lecciones, mapa del repositorio, idiomas, motores opcionales y cálculo exacto |
| Comprobación | `app_tauri`, `ecosistemas`, `recorrido_web`, `verificacion`, `revision`, `pantalla` | Control de calidad por lenguaje, usar la web del proyecto de verdad, revisión independiente, ver la interfaz de verdad |
| Prompt | `prompts`, `contexto` | Instrucciones del modelo y presupuesto de contexto |
| Acción | `herramientas` | Ejecuta lo que pide el modelo y aplica los permisos |
| Dirección | `grande`, `bucle` | Tareas por partes; el bucle con las puertas para terminar |
| Entrada | `__main__`, `instalar`, `../agente_mcp.py` | Línea de comandos y protocolo `--json` de la app; instalación; servidor MCP |

## Por dónde entra cada cosa

| Quiero… | Toco |
| --- | --- |
| Añadir una herramienta | Esquema en `esquemas.py`, método en `Herramientas` y entrada en `despachar` (`herramientas.py`), icono en `bucle.ICONOS` |
| Permitir o bloquear un comando | `permisos.py` |
| Comprobar un lenguaje nuevo | Una función en `ecosistemas.py`, llamada desde `verificar_todos` |
| Cambiar cuándo puede terminar una misión | `bucle._puede_terminar` |
| Entender otro formato de llamadas de un modelo | `llamadas.py` |
| Añadir conocimiento sobre una tecnología | Una guía en `guias/` |
| Añadir un motor opcional | `CATALOGO` de `complementos.py` |

## Puntos que dependen de nombres

Quien renombre uno de estos debe actualizar también a quien lo llama:

- `agente_mcp.py`: `agente.esquemas.ESQUEMAS`, `agente.herramientas.Herramientas`, `agente.revision.revisar`.
- `instalar.bat`: `-m agente.complementos asegurar`, `-m agente.instalar`.
- `instalar_herramientas.bat`: `-m agente.inversa instalar` (y `inversa.resumen()` lo nombra al modelo cuando faltan).
- `setup.ps1`: `-m agente.descargar`, `-m agente.modelo cargar`, `-m agente.instalar`.
- `src-tauri/tauri.conf.json` (`resources`): copia `agente/*.py`, `guias/*.md`, `sondas/*.py`, `tests/*.py` y `tests/datos/*`. Una subcarpeta nueva dentro de `agente/` hay que añadirla ahí o no llegará al instalador.

## Antes de dar un cambio por bueno

```bash
python -m unittest discover -s tests
```

Si cambia el comportamiento del agente, además una misión real con una orden corriente, sin decirle qué usar ni cómo, y usando luego lo que construyó. La suite no ve lo que hace el modelo.
