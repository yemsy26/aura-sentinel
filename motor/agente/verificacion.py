"""verificar(): control de calidad automático por lenguaje, sin depender del modelo.

Python: sintaxis, ruff (errores reales), vulture (código que nunca se usa = funciones sin conectar), tests,
        ejecución del programa y, si es pygame, una partida sin ventana con teclas aleatorias.
HTML/JS: referencias rotas, ids que busca el JS/TS y no existen, sintaxis JS (node --check) y prueba real en Edge
         sin ventana (errores de consola, 404, canvas vacío, animación, respuesta a teclado/ratón, capturas).
Java:   javac -Xlint y ejecución del main en modo headless (o mvn test si hay pom.xml).
.bat:   archivos llamados que no existen, falta de cd /d "%~dp0" y npm/npx (o un .cmd) sin call.
Rust, Go, .NET, Node/TypeScript, Docker/Compose y YAML: en ecosistemas.py.
"""
import ast
import difflib
import html.parser
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import urllib.parse
import urllib.request
from collections.abc import Callable
from functools import cache, partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from itertools import pairwise
from pathlib import Path

from . import ecosistemas
from .borrados import aviso, peligrosos
from .esquemas import ESQUEMAS
from .idiomas import verificar_idiomas
from .proyecto import IGNORAR, Informe, archivos, cola, correr_texto, ignorado, normalizar, relativa
from .recorrido_web import (
    campos_sin_formulario,
    plantillas_sin_sustituir,
    recorrer,
    recursos_externos,
    url_for_sin_ruta,
    variables_sin_pasar,
)
from .seguridad import pruebas_borran_la_base, seguridad_web

BIN = Path(sys.executable).parent  # ruff.exe y vulture.exe del venv del agente
LIMITE_ORDEN = 30_000  # Windows corta una orden en 32.767 caracteres (WinError 206 si se pasa)
# vulture no acepta un archivo de argumentos: se le llama con las rutas leídas de un archivo (mismos argumentos, misma salida)
LANZAR_VULTURE = ("import sys; from vulture.core import main; rutas = open(sys.argv[1], encoding='utf-8').read().splitlines(); "
                  "sys.argv = ['vulture', *rutas, *sys.argv[2:]]; main()")
SIN_ANALISIS = "No busqué código sin usar: el análisis no pudo ejecutarse o tardó demasiado (proyecto muy grande)."
_LEIDOS: dict[Path, tuple[tuple[int, int], list]] = {}  # borrados_fuera: lo hallado en cada .py mientras no cambie
SONDA_PYGAME = Path(__file__).resolve().parent / "sondas" / "pygame_sonda.py"
SONDA_TK = Path(__file__).resolve().parent / "sondas" / "tk_sonda.py"


# Atributos que leen las librerías (pygame Rect/Sprite, tkinter, unittest...), no el propio código: no son "código muerto".
ATRIBUTOS_DE_LIBRERIA = {
    "args", "kwargs", "image", "rect", "mask", "radius", "layer", "dirty", "visible", "blendmode", "source_rect",
    "x", "y", "top", "left", "bottom", "right", "topleft", "topright", "bottomleft", "bottomright", "midtop",
    "midleft", "midbottom", "midright", "center", "centerx", "centery", "size", "width", "height", "w", "h",
    "maxDiff", "longMessage", "daemon", "title", "geometry", "protocol",
    # los lee la librería al usar el objeto: sqlite3, Flask, Flask-Login, http.server/socketserver
    "row_factory", "text_factory", "isolation_level", "secret_key", "permanent_session_lifetime", "session_cookie_name",
    "json_provider_class", "template_folder", "static_folder", "login_view", "login_message", "login_message_category",
    "refresh_view", "needs_refresh_message", "session_protection", "server_version", "sys_version", "protocol_version",
    "allow_reuse_address", "daemon_threads"}
RUFF_ERRORES = {"F821", "F822", "F823", "F811", "F631", "F632", "F702", "F704", "F706", "F707", "B006", "B012"}
TEXTO_RUFF = {"F841": "variable asignada y NUNCA usada (¿lógica sin terminar?)", "F821": "nombre NO definido",
              "F811": "definición repetida que pisa a la anterior", "F401": "import sin usar",
              "B006": "argumento mutable por defecto", "E722": "except vacío (oculta errores)",
              "B018": "expresión sin efecto", "F632": "comparación 'is' con literal"}




def _traza_limpia(tb: str) -> str:
    """Traceback sin marcas ~~~^^^ ni marcos internos: archivo/línea del proyecto + la excepción final."""
    lineas = [l for l in tb.splitlines() if l.strip() and not re.fullmatch(r"\s*[~^\s]+", l)
              and "<frozen" not in l and "runpy" not in l and "pygame_sonda" not in l and "tk_sonda" not in l]
    return "\n    ".join(l.rstrip() for l in lineas[-10:])


def _resumen_fallo(salida: str, n: int = 18) -> str:
    """De la salida de pytest/unittest se queda con lo útil: líneas del proyecto, asserts y errores (sin trazas internas)."""
    utiles = []
    for l in salida.splitlines():
        l = l.replace("INTERNALERROR> ", "").rstrip()
        if not l.strip() or re.search(r"site-packages|<frozen|\^{3,}|~{3,}|^=+ ", l):
            continue
        if re.match(r"\s*(E\s|FAILED|ERROR|>\s|File \"(?!.*site-packages)|.*(Error|Exception|assert))", l) or re.search(r"\.py:\d+", l):
            utiles.append(l.strip()[:220])
    return "\n    ".join((utiles or salida.strip().splitlines())[-n:])


# ---------------------------------------------------------------- Python
def borrados_fuera(raiz: Path, rutas: list[Path] | None = None) -> list[str]:
    """Cada borrado del código Python fuera de lo suyo (borrados.py), con archivo y línea. Sin rutas, todo el proyecto:
    ejecutar lo mira antes de cada comando, y solo se vuelve a leer un archivo si cambió."""
    if rutas is None:
        rutas = []
        for carpeta, dirs, nombres in os.walk(raiz):
            dirs[:] = [d for d in dirs if d not in IGNORAR]
            rutas += [Path(carpeta) / n for n in nombres if n.endswith(".py")]
    hallados = []
    for p in rutas:
        try:
            st = p.stat()
        except OSError:
            continue
        marca = (st.st_mtime_ns, st.st_size)
        if p not in _LEIDOS or _LEIDOS[p][0] != marca:
            _LEIDOS[p] = (marca, peligrosos(p.read_text(encoding="utf-8", errors="replace")))
        hallados += [aviso(relativa(p, raiz), *h) for h in _LEIDOS[p][1]]
    return hallados


def _sobre_archivos(orden: list[str], rutas: list[Path], despues: list[str], desde_lista, raiz: Path, segundos: int) -> str | None:
    """Corre un analizador sobre muchas rutas y devuelve su salida; None si no se pudo (no arrancó o no acabó a tiempo: nunca
    tira la tarea). Si la orden no cabe en Windows, las rutas van en un archivo y `desde_lista(archivo)` da la orden.
    m4197: un repositorio de 1096 .py necesitaba 122.000 caracteres, CreateProcess daba WinError 206 y ninguna tarea
    podía empezar en esa carpeta."""
    cmd, lista = [*orden, *map(str, rutas), *despues], None
    try:
        if len(subprocess.list2cmdline(cmd)) > LIMITE_ORDEN:
            with tempfile.NamedTemporaryFile("w", encoding="utf-8", suffix=".rutas", delete=False) as f:
                f.write("\n".join(map(str, rutas)))
                lista = Path(f.name)
            cmd = desde_lista(str(lista))
        codigo, salida = correr_texto(cmd, raiz, segundos)
    except OSError:
        return None
    finally:
        if lista:
            lista.unlink(missing_ok=True)
    return None if codigo is None else salida


def lint_python(raiz: Path, rutas: list[Path], inf: Informe, rapido: bool = False) -> None:
    ruff = BIN / "ruff.exe"
    if not ruff.exists() or not rutas:
        return
    sel = "E9,F821,F822,F823,F811,F841,F632,B006" if rapido else "E9,F,B006,B012,B018,E722,PLE"
    orden = [str(ruff), "check", "--no-cache", "--output-format", "concise", "--select", sel, "--ignore", "F403,F405,F541"]
    salida = _sobre_archivos(orden, rutas, [], lambda lista: [*orden, f"@{lista}"], raiz, 60)
    if salida is None:
        inf.avisos.append("No pude pasar ruff por el código Python (no arrancó o tardó demasiado).")
        return
    for linea in salida.splitlines():
        m = re.match(r"(.+?):(\d+):\d+: (\w+) (.*)", linea)
        if not m:
            continue
        archivo, num, codigo, msg = m.groups()
        detalle = f"{relativa(Path(archivo), raiz)}:{num} {TEXTO_RUFF.get(codigo, '')} [{codigo}: {msg}]".replace("  ", " ")
        if codigo.startswith(("E9", "PLE")) or codigo in RUFF_ERRORES:
            inf.errores.append(detalle)
        elif codigo != "F401":  # imports sin usar: inofensivos, y "arreglarlos" rompe código que funciona
            inf.avisos.append(detalle)


def codigo_muerto(raiz: Path, rutas: list[Path], inf: Informe, solo: Path | None = None, tope: int = 12,
                  previos: set[str] | None = None) -> None:
    """solo = carpeta verificada: el análisis mira todo el proyecto (los usos pueden estar fuera de ella) pero solo
    informa de lo que hay dentro. Lo que ya estaba (previos) no gasta el tope: taparía lo nuevo."""
    vulture = BIN / "vulture.exe"
    if not vulture.exists() or not rutas:
        return
    # vulture busca usos entre archivos: la lista NO se parte en lotes (inventaría «NUNCA se usa»)
    opciones = ["--min-confidence", "60"]
    salida = _sobre_archivos([str(vulture)], rutas, opciones,
                             lambda lista: [str(BIN / "python.exe"), "-c", LANZAR_VULTURE, lista, *opciones], raiz, max(60, len(rutas) // 5))
    if salida is None:
        inf.avisos.append(SIN_ANALISIS)
        return
    decorados: dict[str, set[int]] = {}  # funciones/clases con decorador las registra un framework (rutas, eventos)
    for r in rutas:
        try:
            arbol = ast.parse(r.read_text(encoding="utf-8", errors="replace"))
        except SyntaxError:
            continue
        lineas = set()
        for nodo in ast.walk(arbol):
            if isinstance(nodo, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)) and nodo.decorator_list:
                lineas |= {nodo.lineno} | {d.lineno for d in nodo.decorator_list}
        decorados[str(r.resolve()).lower()] = lineas
    n = 0
    for linea in salida.splitlines():
        m = re.match(r"(.+?):(\d+): unused (\w+(?: \w+)?) '([^']+)'", linea)
        if not m or re.search(r"(^|[\\/])(test_|tests[\\/])", m.group(1)):
            continue
        archivo, num, tipo, nombre = m.groups()
        if (tipo == "import" or nombre.startswith("_") or nombre in ATRIBUTOS_DE_LIBRERIA or nombre.isupper()
                or (tipo == "method" and re.match(r"(setUp|tearDown|do_|on_|handle_|visit_)", nombre))):
            continue
        ruta_abs = Path(archivo) if Path(archivo).is_absolute() else raiz / archivo  # vulture da rutas relativas a raiz
        if int(num) in decorados.get(str(ruta_abs.resolve()).lower(), set()):
            continue
        if solo is not None and solo not in ruta_abs.resolve().parents:
            continue
        traduccion = {"function": "función", "method": "método", "class": "clase", "variable": "variable",
                      "attribute": "atributo", "property": "propiedad"}.get(tipo, tipo)
        if tipo in ("class", "function"):  # en código generado casi siempre es una pieza que falta conectar
            lista, texto = inf.errores, (f"{relativa(Path(archivo), raiz)}:{num} {traduccion} '{nombre}' NUNCA se usa: el "
                                         f"programa no la crea ni la llama. Conéctala al flujo principal (o bórrala si sobra).")
        else:
            lista, texto = inf.avisos, (f"{relativa(Path(archivo), raiz)}:{num} {traduccion} '{nombre}' se define pero NUNCA "
                                        f"se usa (¿falta llamarla/conectarla al flujo principal?)")
        lista.append(texto)
        n += normalizar(texto) not in (previos or ())
        if n >= tope:
            break


def muertos_ahora(raiz: Path) -> set[str]:
    """El código sin usar que ya hay (normalizado). Un módulo existente puede ofrecer funciones que nadie llama dentro
    del proyecto: al empezar una tarea no son piezas por conectar, y exigir corregirlas hacía borrar código ajeno."""
    inf = Informe()
    codigo_muerto(raiz, archivos(raiz, ".py"), inf, tope=10_000)
    return {normalizar(e) for e in inf.errores + inf.avisos if e != SIN_ANALISIS}


def _modulos_proyecto(rutas: list[Path]) -> set[str]:
    return {p.stem for p in rutas if not p.stem.startswith("test")} | {p.parent.name for p in rutas}


NOMBRES_DE_PRUEBAS_SUELTOS = ("test.py", "tests.py")


def _con_pruebas(p: Path) -> bool:
    """¿Define pruebas (una clase TestCase o funciones test_*)? Un test.py que solo lanza otro archivo no cuenta."""
    try:
        arbol = ast.parse(p.read_text(encoding="utf-8", errors="replace"))
    except (SyntaxError, ValueError):
        return False
    return any((isinstance(n, ast.ClassDef) and any(ast.unparse(b).endswith("TestCase") for b in n.bases))
               or (isinstance(n, ast.FunctionDef) and n.name.startswith("test_")) for n in ast.walk(arbol))


def tests_python(raiz: Path, rutas: list[Path], python: str, inf: Informe) -> None:
    # test.py y tests.py también, si definen pruebas: el nombre que eligió Aura en una entrega con 17 pruebas, que así nunca se
    # corrieron. Un test.py que solo lanza test_app.py (m4133_neg3, p3_neg3) no es una prueba: pytest lo importaría y fallaría.
    tests = [p for p in rutas if p.name.startswith("test_") or p.name.endswith("_test.py") or "tests" in p.parts
             or (p.name in NOMBRES_DE_PRUEBAS_SUELTOS and _con_pruebas(p))]
    if not tests:
        if len([p for p in rutas if p.stem not in ("setup", "conftest")]) >= 1:
            inf.avisos.append("No hay tests (test_*.py). Crea tests que importen tu código y comprueben los requisitos.")
        return
    propios = _modulos_proyecto(rutas)
    for t in tests:
        try:
            arbol = ast.parse(t.read_text(encoding="utf-8", errors="replace"))
        except SyntaxError:
            continue
        importados = {n.module.split(".")[0] for n in ast.walk(arbol) if isinstance(n, ast.ImportFrom) and n.module}
        importados |= {a.name.split(".")[0] for n in ast.walk(arbol) if isinstance(n, ast.Import) for a in n.names}
        asserts = sum(isinstance(n, ast.Assert) for n in ast.walk(arbol)) + len(re.findall(r"\.assert\w+\(|pytest\.raises", t.read_text(encoding="utf-8", errors="replace")))
        if not (importados & propios):
            inf.avisos.append(f"{relativa(t, raiz)} no importa ningún módulo del proyecto: NO prueba tu código.")
        elif asserts == 0:
            inf.avisos.append(f"{relativa(t, raiz)} no tiene ningún assert: no comprueba nada.")
    env = dict(os.environ, SDL_VIDEODRIVER="dummy", SDL_AUDIODRIVER="dummy", PYTHONUTF8="1", MPLBACKEND="Agg")
    codigo, _ = correr_texto([python, "-c", "import pytest"], raiz, 30, env)
    # pytest solo recoge test_*.py y *_test.py: un test.py hay que dárselo por su nombre (y entonces, todos).
    suelto = [relativa(t, raiz) for t in tests] if any(t.name in NOMBRES_DE_PRUEBAS_SUELTOS for t in tests) else []
    cmd = ([python, "-m", "pytest", "-q", "-x", "--no-header", "-p", "no:cacheprovider", *suelto] if codigo == 0
           else [python, "-m", "unittest", "discover", "-p", "test*.py"])
    codigo, salida = correr_texto(cmd, raiz, 180, env)
    if codigo is None:
        inf.errores.append("Los tests no terminan en 180 s (¿bucle infinito o esperan input()?).")
    elif codigo == 5 and "pytest" in cmd:
        inf.avisos.append("pytest no encontró tests (las funciones deben llamarse test_*).")
    elif codigo == 5 or "Ran 0 tests" in salida:
        inf.errores.append("unittest no encontró tests: si son funciones sueltas (estilo pytest), añade pytest a "
                           "requirements.txt e instálalo en el .venv.")
    elif codigo != 0:
        inf.errores.append(f"Tests FALLAN ({' '.join(cmd[2:4])}):\n    {_resumen_fallo(salida)}")
    else:
        resumen = next((l for l in reversed(salida.splitlines()) if re.search(r"passed|OK|Ran \d+", l)), "OK")
        inf.ok.append(f"tests: {resumen.strip()}")


def entrada_python(raiz: Path, rutas: list[Path]) -> Path | None:
    for nombre in ("main.py", "app.py", "juego.py", "game.py", "run.py", "__main__.py"):
        c = [p for p in rutas if p.name == nombre]
        if c:
            return min(c, key=lambda p: len(p.parts))
    con_main = [p for p in rutas if not p.name.startswith("test")
                and "__main__" in p.read_text(encoding="utf-8", errors="replace")]
    return con_main[0] if len(con_main) == 1 else None


def _pantalla_fija(d: dict) -> str:
    """Por qué la pantalla no cambia, con lo que la sonda pulsó y vio: el modelo debe mirar su lógica, no el driver."""
    if d["un_color"]:
        return (f"la pantalla NUNCA cambia y está VACÍA (un solo color en {d['segundos_de_juego']} s de juego simulado): "
                "no dibuja nada, lo tapa con fill() después de dibujar o dibuja en una Surface que nunca copia (blit) a "
                "la de set_mode().")
    if not d["teclas"]:
        return ("la pantalla NUNCA cambia y no se pudo pulsar ninguna tecla: el bucle principal no llama a "
                "pygame.event.get() en cada frame (usa 'for evento in pygame.event.get():').")
    teclas =", ".join(f"{k}×{n}" for k, n in d["teclas"].items())
    textos = f" Textos en pantalla: {', '.join(repr(t[:60]) for t in d['textos'])}." if d["textos"] else ""
    return (f"la pantalla NUNCA cambia: en {d['segundos_de_juego']} s de juego simulado se pulsaron {teclas} y "
            f"{d['clics']} clics, y la imagen fue siempre la misma.{textos} El juego se queda ATASCADO en la primera "
            "pantalla: revisa el código que trata esas teclas (KEYDOWN) y que el cambio de estado no se deshaga en el "
            "mismo frame (p. ej. llamar a reiniciar()/reset() DESPUÉS de cambiar el estado, si ese método vuelve a "
            "poner el estado inicial). No es cosa del driver de vídeo: no toques SDL_VIDEODRIVER ni set_mode().")


def ejecutar_python(raiz: Path, rutas: list[Path], python: str, inf: Informe, segundos: int) -> None:
    entrada = entrada_python(raiz, rutas)
    if entrada is None:
        return
    fuente = entrada.read_text(encoding="utf-8", errors="replace")
    usa_pygame = any("pygame" in p.read_text(encoding="utf-8", errors="replace") for p in rutas
                     if not p.name.startswith("test"))
    rel = relativa(entrada, raiz)
    env = dict(os.environ, PYTHONUTF8="1", PYTHONIOENCODING="utf-8", SDL_VIDEODRIVER="dummy", SDL_AUDIODRIVER="dummy")
    if usa_pygame:
        if "pygame" not in inf.tipos:
            inf.tipos.append("pygame")
        tramo = max(2.0, segundos / 3)
        total = {"frames": 0, "juego": 0, "pantallas": 0}
        for semilla in (1, 2, 3):  # tres partidas distintas pero reproducibles
            codigo, salida = correr_texto([python, str(SONDA_PYGAME), str(entrada), str(tramo)], entrada.parent, tramo + 25,
                                     dict(env, SONDA_SEMILLA=str(semilla)))
            m = re.search(r"^SONDA (\{.*\})$", salida, re.MULTILINE)
            if codigo is None or not m:
                if "No module named 'pygame'" in salida:
                    inf.errores.append("pygame no está instalado en el entorno del proyecto: añade pygame-ce a "
                                       "requirements.txt e instálalo en .venv.")
                elif codigo is None:
                    inf.errores.append(f"{rel}: el juego NO se cierra al recibir QUIT (o se bloquea). Maneja pygame.QUIT "
                                       "en el bucle de eventos.")
                else:
                    inf.errores.append(f"{rel}: falló al arrancar:\n    {cola(salida)}")
                return
            d = json.loads(m.group(1))
            if d["error"]:
                inf.errores.append(f"{rel}: EXCEPCIÓN en la partida simulada nº {semilla} (tras {d['segundos_de_juego']} s "
                                   f"de juego con teclas aleatorias):\n    " + _traza_limpia(d["error"]))
                return
            if not d["termino_por_quit"] and d["segundos"] < tramo - 1:
                inf.avisos.append(f"{rel}: el juego se cerró solo a los {d['segundos_de_juego']} s de juego sin que nadie "
                                  "lo cerrara.")
            if d["driver"] != "dummy" and semilla == 1:
                inf.avisos.append(f"{rel}: el juego cambia SDL_VIDEODRIVER y abre una ventana de verdad al verificar. "
                                  "Quita ese código: verificar ya simula la pantalla.")
            if d["pantallas_distintas"] <= 1:
                inf.errores.append(f"{rel}: {_pantalla_fija(d)}")
                return
            if d["frames"] > 200 and d["segundos_de_juego"] == 0:
                inf.errores.append(f"{rel}: el bucle principal NUNCA llama a reloj.tick(60): el juego irá a velocidad máxima "
                                   "(injugable) y usará el 100 % de la CPU. Llama a self.reloj.tick(60) en cada vuelta.")
                return
            if d["ms_por_frame"] > 33 and d["frames"] > 0:
                inf.avisos.append(f"{rel}: cada frame tarda {d['ms_por_frame']} ms (>33 ms = menos de 30 FPS). No recrees "
                                  "superficies/degradados/fuentes en cada frame.")
            total["frames"] += d["frames"]
            total["juego"] += d["segundos_de_juego"]
            total["pantallas"] += d["pantallas_distintas"]
        inf.ok.append(f"3 partidas simuladas: {total['juego']} s de juego ({total['frames']} frames, teclas aleatorias "
                      f"reproducibles), {total['pantallas']} pantallas distintas, sin excepciones")
        return
    if any(re.search(r"^\s*(?:import tkinter|from tkinter|import customtkinter|from customtkinter)\b",
                     p.read_text(encoding="utf-8", errors="replace"), re.MULTILINE) for p in rutas if not p.name.startswith("test")):
        lanzado = entrada_bat(raiz)
        probar_tk(raiz, lanzado[1] if lanzado else entrada, python, inf, segundos, lanzado[0] if lanzado else None)
        return
    if re.search(r"\b(PyQt|PySide|wx)\b", fuente):
        inf.avisos.append(f"{rel}: app con ventana (GUI): no se abre aquí; prueba su lógica con tests.")
        return
    web, lanzado = False, None
    if any(WEB_PY.search(p.read_text(encoding="utf-8", errors="replace")) for p in rutas if not p.name.startswith("test")):
        web, lanzado = True, entrada_bat(raiz)
        if probar_web(raiz, lanzado[1] if lanzado else entrada, python, inf):
            return
    codigo, salida = correr_texto([python, str(entrada)], entrada.parent, segundos, env)
    if codigo is None:
        inf.ok.append(f"{rel} sigue en ejecución tras {segundos} s (servidor o bucle): arrancó sin errores")
    elif "EOFError" in salida:
        inf.avisos.append(f"{rel} pide datos con input(): pruébalo con tests o con 'echo datos| python {entrada.name}'.")
    elif codigo != 0:
        inf.errores.append(f"{rel} termina con error (exit {codigo}):\n    {cola(salida)}")
    elif not (web and WEB_PY.search(fuente) and (lanzado is None or lanzado[1] == entrada)
              and servidor_ausente(raiz, entrada, lanzado, inf)):
        inf.ok.append(f"{rel} se ejecuta sin errores")


# Una web de negocio daba 500 al registrar una venta y verificar solo veía que el servidor arrancaba: se usa de verdad.
WEB_PY = re.compile(r"\bFlask\s*\(|\bFastAPI\s*\(|\b(?:Threading)?HTTPServer\s*\(|\bsocketserver\.\w*TCPServer\s*\(")
POR_ESTADO = {0: "nada (el servidor se cayó)", 404: "404: esa ruta no existe", 405: "405: esa ruta no acepta ese método",
              200: "200 (parece que fue bien), pero la página enseña un error de programa"}


def probar_web(raiz: Path, entrada: Path, python: str, inf: Informe) -> bool:
    """Recorre la web de verdad sobre una copia (recorrido_web.py). False si no llegó a escuchar: entonces se prueba
    como un programa más."""
    claves = claves_del_proyecto(raiz)
    usuarios = usuarios_del_proyecto(raiz, claves)
    accesos = list(dict.fromkeys(pares_del_proyecto(raiz, claves, usuarios) + [(u, c) for c in claves for u in usuarios]))
    copia = Path(tempfile.mkdtemp(prefix="agente_web_")) / "p"
    try:
        shutil.copytree(raiz, copia, ignore=shutil.ignore_patterns(*COPIA_FUERA))
        r = recorrer(copia / entrada.relative_to(raiz), python, accesos, abrir_navegador=canal_navegador)
    finally:
        shutil.rmtree(copia.parent, ignore_errors=True)
    if r is None:
        return False
    rel = relativa(entrada, raiz)
    nav, antes = r["navegador"], len(inf.errores)
    for p in r["plantillas"][:4]:
        pista = (" En Jinja se escribe {{ '%.2f' % x }} o {{ '{:.2f}'.format(x) }}, no {{ x:.2f }}."
                 if "got ':'" in p["error"] else "")
        inf.errores.append(f"la plantilla {p['plantilla']} no compila (línea {p['linea']}: {p['error']}): toda página que "
                           f"la use da error 500.{pista}")
    for p in plantillas_sin_sustituir(entrada.parent, raiz)[:2]:
        inf.avisos.append(f"la plantilla {p['plantilla']} (línea {p['linea']}) enseña «{p['texto']}» tal cual: Jinja no lo sustituye (es sintaxis "
                          "de Python o de JavaScript) y quien abre la página ve el marcador escrito, no el valor. En una plantilla Jinja se escribe "
                          "{{ '%.2f'|format(x) }} (con el $ delante si es dinero).")
    externos = recursos_externos(entrada.parent, raiz)
    if externos:  # Bootstrap o jQuery de un CDN en un sistema de doble clic (ferretería 22): sin red la web pierde el estilo
        hosts = ", ".join(sorted({e["host"] for e in externos}))
        donde = ", ".join(f"{e['recurso']} ({e['archivo']})" for e in externos[:4]) + (" y más" if len(externos) > 4 else "")
        cuantos = "1 archivo se carga" if len(externos) == 1 else f"{len(externos)} archivos se cargan"
        inf.avisos.append(f"{cuantos} de internet ({hosts}): {donde}. Un sistema que se abre con doble clic en una computadora de negocio no debería depender de la "
                          "red: sin conexión la web pierde el estilo, o los menús y los scripts dejan de funcionar. Copia esos archivos a static/ (bájalos una vez con curl) "
                          "y enlázalos con url_for('static', filename=...), o escribe tu propio CSS; si la web necesita internet por otra razón, ignora este aviso.")
    for p in url_for_sin_ruta(entrada.parent, raiz)[:3]:
        sugerencia = f" (¿quisiste decir «{p['sugerencia']}»?)" if p["sugerencia"] else ""
        inf.errores.append(f"la plantilla {p['plantilla']} (línea {p['linea']}) usa url_for('{p['endpoint']}'), pero ninguna ruta de la web se llama así"
                           f"{sugerencia}: cuando esa parte de la página se muestre, Flask da un error 500 (BuildError); si está dentro de un «if» que no se "
                           "cumple, el botón o el enlace no se verá nunca.")
    faltan: dict[tuple, list[dict]] = {}
    for p in campos_sin_formulario(entrada.parent, raiz):
        faltan.setdefault((p["plantilla"], p["linea"], p["ruta"], p["funcion"]), []).append(p)
    for (plantilla, linea, ruta, funcion), hallados in list(faltan.items())[:3]:
        campos = [p["campo"] for p in hallados if p["tipo"] == "indexa"]
        if campos:
            leidos = " y ".join('request.form["' + c + '"]' for c in campos)
            inf.errores.append(f"el formulario de {plantilla} (línea {linea}) envía a {ruta}, pero la ruta «{funcion}» lee {leidos} y el formulario no tiene "
                               f"{'ese campo' if len(campos) == 1 else 'esos campos'}: Flask responde 400 Bad Request en cada envío y esa acción no se puede "
                               f"hacer desde la pantalla. Añade el campo al formulario (<input name=\"{campos[0]}\">) o lee el valor con "
                               f"request.form.get(\"{campos[0]}\").")
        validados = [p for p in hallados if p["tipo"] == "valida"]
        if validados:  # lo lee con .get y rechaza el envío si viene vacío: el formulario nunca lo trae y cada envío acaba en «campo obligatorio»
            campos = [p["campo"] for p in validados]
            leidos = " y ".join('request.form.get("' + c + '")' for c in campos)
            suyos = ", ".join(validados[0]["formulario"])
            inf.errores.append(f"el formulario de {plantilla} (línea {linea}) envía a {ruta}, pero la ruta «{funcion}» lee {leidos} y rechaza el envío si "
                               f"viene vacío, y el formulario no tiene {'ese campo' if len(campos) == 1 else 'esos campos'}"
                               f"{f' (sus campos son: {suyos})' if suyos else ''}: cada envío acaba en el aviso de campo obligatorio y esa acción no se "
                               f"puede hacer desde la pantalla. Usa el mismo nombre en el formulario y en la ruta (<input name=\"{campos[0]}\"> o "
                               f"request.form.get(\"<el nombre del formulario>\")).")
    efecto = {"for": "el bucle sale siempre vacío", "if": "esa parte nunca se muestra", "salida": "sale en blanco",
              "atributo": "da error 500 al pintar esa parte"}
    sin_pasar: dict[str, list[dict]] = {}
    for v in variables_sin_pasar(r.get("variables") or [], entrada.parent, raiz):
        sin_pasar.setdefault(v["plantilla"], []).append(v)
    for plantilla, vs in list(sin_pasar.items())[:3]:
        detalle = ", ".join(f"«{v['nombre']}» (línea {v['linea']}: {efecto[v['uso']]})" for v in vs[:4])
        quisiste = "; ".join(f"en vez de «{v['nombre']}» ¿«{v['sugerencia']}»?" for v in vs[:4] if v["sugerencia"])
        pasan = ", ".join(vs[0]["pasan"]) or "nada"
        texto = (f"la plantilla {plantilla} lee {detalle}, pero ninguna ruta se lo pasa: render_template solo le pasa {pasan}"
                 f"{' (' + quisiste + ')' if quisiste else ''}. Jinja no da error por una variable sin definir: la lista sale vacía, el «if» es falso "
                 "y el texto sale en blanco.")
        # un bucle sobre algo sin definir en una página que rellena JavaScript puede ser código sobrante: solo aviso
        (inf.errores if any(v["uso"] == "atributo" or v["uso"] == "for" and not v["con_js"] for v in vs) else inf.avisos).append(texto)
    por_texto: dict[str, list[str]] = {}
    for x in r.get("etiquetas") or []:
        por_texto.setdefault(x["texto"], []).append(f"{x['que']} de {x['pagina']}")
    for texto, donde in list(por_texto.items())[:2]:
        otros = f" (y {len(donde) - 1} más: {', '.join(donde[1:3])})" if len(donde) > 1 else ""
        inf.avisos.append(f"{rel}: un {donde[0]} dice «{texto}»{otros}: la plantilla pinta el valor de una variable de Python en vez de un texto, "
                          "y quien lo ve no sabe qué hace. Suele ser {{ form.submit.data }} (el valor del campo, False hasta que se envía): "
                          "usa {{ form.submit.label.text }} o escribe el texto del botón.")
    for e in r["errores"][:4]:
        traza = e["traza"].replace(str(copia) + os.sep, "").replace("\n", "\n    ")
        inf.errores.append(f"{rel}: al {e['que']}" + (f" con {e['datos']}" if e["datos"] else "") +
                           f", la web responde {POR_ESTADO.get(e['estado'], e['estado'])}" + (f":\n    {traza}" if traza else "."))
    # Una entrega no dejaba entrar a nadie: sus tests escribían en la base real un admin de prueba y, como ya había un
    # admin, el del README no se creaba. Era un aviso y el agente terminó así.
    otro_puerto = nav and nav.get("otro_puerto")
    if (r["sin_entrar"] or (nav and nav["sin_entrar"] and accesos)) and otro_puerto:
        inf.avisos.append(f"{rel}: no pude entrar: la página llama a {otro_puerto}, con el puerto escrito, y aquí la web "
                          "escuchaba en otro. Si ese puerto está ocupado o cambia, la web deja de funcionar: usa rutas "
                          "relativas (/api/...).")
    elif (r["sin_entrar"] or (nav and nav["sin_entrar"] and accesos)) and (r.get("registro") or nav and nav.get("registro")):
        inf.avisos.append(f"{rel}: no pude entrar con los usuarios y contraseñas que da el proyecto; la web tiene "
                          "registro. Si la primera cuenta se crea registrándose, dilo en el README.")
    elif r["sin_entrar"] or (nav and nav["sin_entrar"] and accesos):
        inf.errores.append(f"{rel}: nadie puede entrar en la web con los usuarios y contraseñas que da el proyecto "
                           f"(probé {min(len(accesos), 12)}, entre ellos {', '.join(u for u, _ in accesos[:3])})."
                           + (' El formulario de entrar se envía por GET: le falta method="post".' if r.get("entrar_get")
                              else " Comprueba que la base de datos que se entrega tiene ese usuario con esa contraseña "
                              "(si los tests escriben en la base real, dejan sus usuarios y el admin no se crea) y que "
                              "la pantalla de entrar funciona."))
    elif r["entrar_js"] and not nav:
        inf.avisos.append(f"{rel}: la pantalla de entrar se envía con JavaScript y no hay navegador para usarla: solo "
                          "recorrí lo que se ve sin entrar. Prueba lo de dentro con tests que inicien sesión.")
    elif nav and nav["sin_entrar"]:
        inf.avisos.append(f"{rel}: no pude entrar (con el navegador): el proyecto no da usuarios ni contraseñas.")
    for s in r.get("sin_formulario", [])[:3]:
        inf.errores.append(f"{rel}: el enlace «{s['texto']}» de {s['desde']} no abre ningún formulario: {s['a']} vuelve a "
                           f"{s['desde']}, también con datos ya creados. Nadie puede hacer eso desde la web: {s['a']} "
                           "tiene que enseñar su formulario (GET) y guardarlo (POST).")
    if r.get("login_vacio"):
        inf.errores.append(f"{rel}: la web manda a {r['login_vacio']} para entrar, pero esa página sale vacía (sin formulario, "
                           "campos, botones ni enlaces): nadie puede entrar. Mira que la plantilla de entrar rellene el bloque "
                           "que pide la plantilla base (el nombre del {% block %} tiene que ser el mismo en las dos).")
    for s in r.get("sin_permiso", [])[:3]:
        cierre = "la web le responde 403" if s["a"] is None else f"la web lo manda a {s['a']} con «{s['frase']}»"
        inf.errores.append(f"{rel}: el administrador «{s['usuario']}» entra, pero al abrir {s['ruta']} {cierre}: no puede usar "
                           "su propia sección (crear, vender...). Revisa cómo se carga el usuario en cada petición (¿el rol "
                           "llega bien al construirlo?) y que el administrador tenga el rol que piden esas rutas.")
    emp = r.get("empleado") or {}
    if emp.get("costo"):
        inf.errores.append(f"{rel}: un empleado (creado con rol de empleado desde la pantalla de usuarios) ve el costo de un producto "
                           f"(el precio de compra 7.77 que puso la prueba) en {', '.join(emp['costo'][:4])}. El costo es del "
                           "administrador: quítalo de lo que recibe el empleado en el servidor (que la plantilla o el JSON no lo "
                           "incluyan, también en atributos data-* o scripts; ocultarlo con CSS o JavaScript no sirve, el dato ya viajó).")
    for s in emp.get("abre", [])[:3]:
        que = {"ganancias": "y enseña ganancias, utilidades o costos", "alta de usuarios": "y puede dar de alta usuarios"}.get(
            s["motivo"], f"y la ve igual que el administrador (parecido {s.get('parecido')})")
        inf.errores.append(f"{rel}: un empleado (creado con rol de empleado desde la pantalla de usuarios) abre {s['ruta']} {que}: "
                           "es una sección de gestión del administrador. Ciérrala al rol de empleado en la propia ruta (403, o "
                           "redirigir con un aviso), no solo quitando el enlace del menú: el empleado escribe la URL.")
    if emp.get("error"):
        inf.errores.append(f"{rel}: un empleado (creado con rol de empleado desde la pantalla de usuarios) recibe un error de programa "
                           f"(500) al abrir {', '.join(emp['error'][:4])}, donde el administrador la abre bien. Revisa lo que esa ruta "
                           "hace con un usuario que no es administrador: ¿lee una columna o una clave que no se carga para ese rol, o "
                           "una variable que solo existe para el administrador? Mira la traza en la consola de la web.")
    abiertas = [a for a in emp.get("ajenas") or [] if a["motivo"] not in ("vendedor", "tabla")]
    if abiertas:
        inf.avisos.append(f"{rel}: un empleado (creado con rol de empleado desde la pantalla de usuarios) abre "
                          f"{', '.join(a['ruta'] for a in abiertas[:4])}: ventas o facturas de otros usuarios (él no ha vendido "
                          "nada). Si cada empleado solo debe ver lo suyo, filtra por el vendedor en la lista y comprueba de quién es "
                          "en el detalle (403 o 404 si no es suyo); si en este negocio los empleados ven todas las ventas, ignora "
                          "este aviso.")
    en_tabla = [a for a in emp.get("ajenas") or [] if a["motivo"] == "vendedor" and all(a["ruta"] != x["ruta"] for x in abiertas)]
    if en_tabla:  # el panel de inicio con «Últimas ventas» de todos aunque /ventas ya filtre por vendedor
        donde = ", ".join(f"{a['ruta']} (vendedor {', '.join(f'«{v}»' for v in a['vendedores'])})" for a in en_tabla[:4])
        inf.avisos.append(f"{rel}: un empleado (creado con rol de empleado desde la pantalla de usuarios) ve en {donde} una lista de "
                          "ventas de otros vendedores (él no ha vendido nada). Si cada empleado solo debe ver lo suyo, filtra también "
                          "esa lista por el vendedor (el panel de inicio suele enseñar «Últimas ventas» de todos aunque /ventas ya "
                          "filtre); si en este negocio los empleados ven todas las ventas, ignora este aviso.")
    sin_columna = [a for a in emp.get("ajenas") or [] if a["motivo"] == "tabla" and all(a["ruta"] != x["ruta"] for x in abiertas + en_tabla)]
    if sin_columna:  # la pantalla de venta con «ventas recientes» de todos (ferretería 22): sin columna de vendedor ni enlaces de detalle
        donde = ", ".join(f"{a['ruta']} ({a['filas']} {'fila' if a['filas'] == 1 else 'filas'}, por ejemplo con importe {a['importe']})"
                          for a in sin_columna[:4])
        inf.avisos.append(f"{rel}: un empleado (creado con rol de empleado desde la pantalla de usuarios) ve en {donde} una tabla con ventas "
                          "de otros usuarios (él no ha vendido nada). Si cada empleado solo debe ver lo suyo, filtra esa lista por el "
                          "vendedor en la consulta (la pantalla de venta suele enseñar «ventas recientes» de todos); si en este negocio "
                          "los empleados ven todas las ventas, ignora este aviso.")
    modulo = (' Esta pantalla carga un <script type="module">: las funciones de un módulo no son globales y un onclick="función()" no las ve; '
              "engancha los botones con addEventListener dentro del módulo o asigna window.función = función.")
    for e in (r.get("venta") or {}).get("errores", [])[:2]:
        if e["tipo"] == "vacio":
            inf.errores.append(f"{rel}: en la pantalla de venta {e['pagina']}, el desplegable de productos queda vacío aunque hay productos "
                               f"creados (error de JavaScript: «{e['js']}»): nadie puede vender desde esta pantalla. Mira la consola del "
                               "navegador: ¿la lista de productos que usa el script llega a la página (una variable que se define, "
                               "un fetch a la API)?")
        elif e["tipo"] == "no_registrada":
            fallo = f" (error de JavaScript: «{e['js']}»)" if e["js"] else ""
            inf.errores.append(f"{rel}: en la pantalla de venta {e['pagina']}, tras elegir un producto y una cantidad y pulsar guardar, no se "
                               f"registra la venta ({e['mensaje']}): la pantalla se queda igual sin decir nada y ni la lista de ventas ni "
                               f"las existencias cambian{fallo}. ¿El formulario no valida por un campo obligatorio que la plantilla no pinta "
                               "(el vendedor, un id), o el servidor no guarda? Una venta guardada redirige a otra pantalla o dice que se registró.")
        elif e["tipo"] == "por_get":
            inf.errores.append(f"{rel}: en la pantalla de venta {e['pagina']}, al pulsar guardar el navegador envía los datos por GET (la dirección "
                               f"acaba en «{e['mensaje']}») en vez de por POST: el <form> no lleva method=\"post\" (o el script lo envía con "
                               "this.submit() sin cambiarlo) y la ruta solo guarda con POST, así que la venta no se guarda nunca. Añade "
                               "method=\"post\" al <form>.")
        elif e["tipo"] == "sin_desplegable":
            inf.errores.append(f"{rel}: en la pantalla de venta {e['pagina']} no aparece ningún desplegable de productos aunque hay productos "
                               f"creados (error de JavaScript al cargar: «{e['js']}»): nadie puede vender desde esta pantalla. Mira la consola "
                               "del navegador: ¿la lista de productos que usa el script llega bien a la página (datos mal envueltos, una "
                               "variable sin definir)?" + (modulo if e.get("modulo") else ""))
        elif e["tipo"] == "js_roto":
            inf.errores.append(f"{rel}: la pantalla de venta {e['pagina']} tiene un error de JavaScript («{e['js']}») y ningún desplegable de "
                               "productos: lo que el script debía pintar no aparece y es probable que no se pueda armar la venta desde ahí "
                               "(la prueba no pudo crear productos para verlo con datos). Mira la consola del navegador: ¿esa función o "
                               "variable existe y está donde la usa el script?" + (modulo if e.get("modulo") else ""))
        elif e["tipo"] == "rechazada":
            inf.errores.append(f"{rel}: en la pantalla de venta {e['pagina']}, tras elegir un producto y una cantidad y pulsar guardar, la web "
                               f"responde «{e['mensaje']}» y la venta no se registra (ni la lista de ventas ni las existencias cambian). "
                               "Quien vende no puede vender desde esta pantalla: busca ese error en el código que guarda la venta.")
        elif e["tipo"] == "defecto":
            inf.errores.append(f"{rel}: en la pantalla de venta {e['pagina']}, la venta se registra eligiendo otra opción, pero con {e['defecto']} tal como "
                               f"viene (lo que hace quien no toca el desplegable) la web responde «{e['mensaje']}» y no se registra. ¿Un id vacío llega a "
                               "la base como texto ('') en vez de NULL (usa `valor or None`), o la ruta exige el campo y no avisa? Vende una vez sin tocar "
                               "ese desplegable.")
        else:
            inf.errores.append(f"{rel}: en la pantalla de venta {e['pagina']}, tras elegir un producto y una cantidad y pulsar guardar, la "
                               f"web responde «{e['mensaje']}»: la venta no se guarda desde la pantalla. ¿El campo con las líneas va fuera "
                               "del <form> (un formulario solo envía sus propios campos), o el JavaScript no lo rellena antes de enviar?")
    neg = r.get("negocio") or {}
    for ruta in neg.get("informe_cero", [])[:2]:
        inf.avisos.append(f"{rel}: el informe {ruta} enseña todos sus importes en 0 aunque hay ventas ({neg['ventas'][1]} en "
                          f"{neg['ventas'][0]}). ¿El filtro por defecto deja fuera las ventas de hoy (una fecha de fin sin la hora, el "
                          "mes mal calculado, UTC frente a la hora local)? Quien abre el informe quiere ver cuánto ha ganado, sin tocar "
                          "ningún filtro.")
    for ruta in neg.get("facturas_vacias", [])[:2]:
        inf.avisos.append(f"{rel}: hay ventas ({neg['ventas'][1]} en {neg['ventas'][0]}), pero la lista de facturas {ruta} está vacía y "
                          "no hay ningún botón para generarlas: ¿se crea la factura y no se guarda (falta el commit) o no se crea?")
    if nav and nav.get("login_invisible"):
        etiqueta, ident, clases, como = nav["login_invisible"]
        quien = etiqueta + (f"#{ident}" if ident else "") + "".join("." + c for c in clases.split())
        inf.errores.append(f"{rel}: la pantalla de entrar sale en blanco: el formulario está en la página, pero {quien} lo "
                           f"oculta ({como}) y no se ve ningún campo, botón ni texto. Nadie puede entrar. Revisa el CSS de "
                           "esa clase (¿existe la regla que la enseña, como .active o .visible?).")
    if nav and nav.get("sin_autorizacion"):
        s = nav["sin_autorizacion"]
        rutas = ", ".join(f"GET {x['ruta']} ({x['estado']})" for x in s[:4])
        inf.errores.append(f"{rel}: después de entrar como {s[0]['usuario']}, la pantalla pide {rutas}"
                           + (f" y {len(s) - 4} más" if len(s) > 4 else "") + " y la API le responde que no está autorizada, "
                           "recién entrado: o la petición sale sin su credencial (la cabecera Authorization o la cookie), o "
                           "el servidor no acepta la que le llega (¿se compara con el valor que se guardó al entrar?). De "
                           "las dos formas las listas salen vacías. Llama a la API con una sola función que ponga siempre la "
                           "credencial, en vez de un fetch suelto en cada pantalla.")
    if nav and nav.get("clave_a_la_vista"):
        inf.errores.append(f"{rel}: después de entrar como {nav['usuario']}, la pantalla de entrar sigue a la vista y la "
                           "aplicación aparece debajo: quien la usa cree que no ha entrado. Revisa cómo se oculta (una "
                           "regla CSS con display puede ganar a la clase que la oculta).")
    if nav and nav.get("menu_quieto"):
        inf.errores.append(f"{rel}: al pulsar en el menú {', '.join('«' + t + '»' for t in nav['menu_quieto'][:6])}, la "
                           "pantalla no cambia: no carga otra página ni enseña otra sección. Revisa que cada entrada "
                           "lleve a una página que existe o enseñe una sección que está en esa misma página.")
    for b in (nav or {}).get("nuevo_sin_campos", [])[:3]:
        inf.errores.append(f"{rel}: en «{b['seccion']}», el botón «{b['texto']}» no abre nada para rellenar"
                           + (f" (error de JavaScript: {b['js'][0]})" if b["js"] else "") + ".")
    if nav and nav["avisos_js"]:
        inf.avisos.append(f"{rel}: errores de JavaScript en la página: " + " | ".join(nav["avisos_js"][:3]))
    if r["rotos"]:
        inf.avisos.append(f"{rel}: enlaces que dan 404: {', '.join(r['rotos'][:5])}")
    if len(inf.errores) == antes:
        hecho = (f"{nav['secciones']} secciones y {nav['formularios']} formularios guardados con el navegador" if nav
                 else f"{r['paginas']} páginas y {r['formularios']} formularios enviados")
        inf.ok.append(f"{rel}: usé la web de verdad: {hecho}" + (f" entrando como {r['usuario']}" if r["usuario"] else "")
                      + (" y como un empleado creado desde la pantalla de usuarios (no ve el costo ni las secciones de gestión)"
                         if emp.get("entro") else "") + ", sin errores")
    return True


# Una entrega acababa su run.bat en «python app.py», pero app.py solo definía create_app(): nadie la llamaba ni hacía
# app.run, no se abría ningún servidor y verificar decía «se ejecuta sin errores». Sin .bat puede arrancarse con
# `flask run` o un servidor de producción: entonces solo se avisa, y nada si el proyecto lo nombra.
OTRO_ARRANQUE = re.compile(r"(?i)\bflask\s+(?:--app\b|run\b)|\b(?:uvicorn|gunicorn|waitress|hypercorn)\b|\bfastapi\s+(?:dev|run)\b")
# Un gestor de consola con un modo web opcional también crea un HTTPServer: si el programa pone algo a servir en algún sitio
# (aunque sea en un modo), no se marca.
SIRVE = re.compile(r"\.run\s*\(|\bserve_forever\b|\brun_simple\s*\(|\bwaitress\.serve\b|\.serve\s*\(")


def servidor_ausente(raiz: Path, entrada: Path, lanzado: tuple[Path, Path] | None, inf: Informe) -> bool:
    """Marca una web cuyo programa termina sin servir nada. False si el proyecto se arranca de otra forma."""
    if SIRVE.search(entrada.read_text(encoding="utf-8", errors="replace")):
        return False
    rel = relativa(entrada, raiz)
    consejo = 'Llama a app.run(...) (Flask) o uvicorn.run(...) (FastAPI) al final, dentro de `if __name__ == "__main__":`.'
    if lanzado:
        inf.errores.append(f"{rel}: crea la aplicación web pero nunca la pone a servir: `python {entrada.name}` (lo que abre "
                           f"{lanzado[0].name}) termina sin abrir ningún servidor, así que con doble clic no se ve nada. {consejo}")
        return True
    for p in (*raiz.glob("*.bat"), *raiz.glob("*.cmd"), *raiz.glob("*.sh"), *raiz.glob("*.md"), raiz / "Procfile"):
        try:
            if OTRO_ARRANQUE.search(p.read_text(encoding="utf-8", errors="replace")[:20000]):
                return False
        except OSError:
            continue
    inf.avisos.append(f"{rel}: crea la aplicación web pero al ejecutarlo termina sin abrir ningún servidor. Si se arranca con "
                      f"`flask run` o un servidor de producción, ignóralo; si no, {consejo[0].lower() + consejo[1:]}")
    return True


# Una peluquería en tkinter terminó con verificar en verde y fallaba nada más entrar (ttk.Notebook.add de un frame
# nieto): la app solo se ejecutaba hasta su bucle de eventos. La sonda la usa sin enseñar ventanas, sobre una copia.
FASE_TK = {"arranque": "abrir la ventana", "entrar": "entrar (usuario y contraseña)", "pestanas": "cambiar de pestaña"}
COPIA_FUERA = (".venv", "venv", "node_modules", ".git", "__pycache__", ".agente", "dist", "build", "target", ".pytest_cache")


# Una peluquería terminó con run.bat lanzando «python -m app»: app.py solo definía funciones y la ventana se abría en
# __main__.py. Con doble clic no se veía nada y verificar, que había elegido app.py por su nombre, decía «se ejecuta sin
# errores». La app se prueba por donde la abre el .bat de doble clic.
NO_ABRE = re.compile(r"(?i)^(test|prueba|instal|setup|build|compil)")
OBJETIVO_PY = re.compile(r'(?i)(?:\b(?:python[w3]?|pyw?)(?:\.exe)?(?:\s+-3(?:\.\d+)?)?|%PY%)"?\s+'
                         r'(?:-m\s+([\w.]+)|"([^"]+\.pyw?)"|([^\s"&|<>]+\.pyw?)\b|(\.)(?=[\s"]|$))')


# Lo que deja la app abierta: un servidor o una ventana. Un run.bat hacía «python seed.py» y luego «python app.py»;
# verificar probaba seed.py (siembra datos y termina) y la web, con el login roto, nunca se usó.
ARRANCA = re.compile(r"\bFlask\s*\(|\bFastAPI\s*\(|\.run\s*\(|uvicorn\.run|serve_forever|mainloop\s*\(|\.exec_?\s*\(")


def entrada_bat(raiz: Path) -> tuple[Path, Path] | None:
    """(.bat, .py que abre) para el .bat de doble clic: python app.py, "%PY%" -m app, python . (su __main__.py). Si el
    .bat lanza varios (preparar datos y después la app), el que arranca algo; si no se distingue, el último."""
    lotes = sorted((b for b in raiz.glob("*.bat") if not NO_ABRE.match(b.stem)),
                   key=lambda b: (not re.match(r"(?i)(run|abrir|iniciar|ejecutar|start|app|main)", b.stem), b.name.lower()))
    for b in lotes:
        encontrados: list[Path] = []
        for linea in b.read_text(encoding="utf-8", errors="replace").splitlines():
            if re.match(r"(?i)\s*@?(rem\b|::|echo\b)", linea):
                continue
            for m in OBJETIVO_PY.finditer(linea):
                modulo, citado, suelto, punto = m.groups()
                if modulo:
                    base = raiz / modulo.replace(".", "/")
                    candidatos = [base.with_name(base.name + ".py"), base / "__main__.py"]
                elif punto:
                    candidatos = [raiz / "__main__.py"]
                else:
                    candidatos = [raiz / (citado or suelto).replace("%~dp0", "")]
                encontrados += [c for c in candidatos if "%" not in str(c) and c.is_file() and not c.name.startswith("test")
                                and c not in encontrados][:1]
        if encontrados:
            arrancan = [c for c in encontrados if ARRANCA.search(c.read_text(encoding="utf-8", errors="replace"))]
            return b, (arrancan or encontrados)[-1]
    return None


def _sonda_tk(raiz: Path, entrada: Path, python: str, tramo: int, claves: list[str], usuarios: list[str],
              excluir: list[str]) -> tuple[int | None, str, dict | None]:
    """Una pasada de la sonda sobre una copia nueva del proyecto: (código, salida, lo que imprime la sonda)."""
    copia = Path(tempfile.mkdtemp(prefix="agente_tk_")) / "p"
    try:
        shutil.copytree(raiz, copia, ignore=shutil.ignore_patterns(*COPIA_FUERA))
        env = dict(os.environ, PYTHONUTF8="1", PYTHONIOENCODING="utf-8", SONDA_CLAVES=json.dumps(claves),
                   SONDA_USUARIOS=json.dumps(usuarios), SONDA_EXCLUIR=json.dumps(excluir))
        destino = copia / entrada.relative_to(raiz)
        codigo, salida = correr_texto([python, str(SONDA_TK), str(destino), str(tramo)], destino.parent, tramo + 25, env)
    finally:
        shutil.rmtree(copia.parent, ignore_errors=True)
    salida = salida.replace(str(copia) + os.sep, "")
    m = re.search(r"^SONDA_TK (\{.*\})$", salida, re.MULTILINE)
    if not m:
        return codigo, salida, None
    d = json.loads(m.group(1))
    for e in d["errores"]:  # sin la carpeta de la copia ni los marcos de dentro de tkinter (su línea de código va detrás)
        e["traza"] = re.sub(r'\n\s*File "[^"\n]*[\\/]tkinter[\\/][^\n]*\n[^\n]*', "",
                            _traza_limpia(e["traza"]).replace(str(copia) + os.sep, ""))
    return codigo, salida, d


def _fallo_grave(rel: str, d: dict, inf: Informe) -> bool:
    """Excepción al abrir, entrar o cambiar de pestaña, o ninguna ventana a la vista: ERROR."""
    como = f" (como «{d['entrar']['usuario']}»)" if d.get("entrar") and d["entrar"]["usuario"] is not None else ""
    graves = [e for e in d["errores"] if e["fase"] in FASE_TK]
    if graves:
        e = graves[0]
        inf.errores.append(f"{rel}: EXCEPCIÓN al {FASE_TK[e['fase']]}{como if e['fase'] != 'arranque' else ''}: el "
                           "usuario se queda ahí sin poder seguir (verificar usa la app sin enseñar la ventana):\n    "
                           f"{e['traza']}")
        return True
    if d.get("sin_ventana"):
        cuando = "al abrir el programa" if d["sin_ventana"] == "arranque" else f"después de entrar{como}"
        inf.errores.append(f"{rel}: {cuando} no queda ninguna ventana a la vista: la app oculta o cierra la que el "
                           "usuario tiene que ver (withdraw(), iconify() o destroy()) y no ve nada. Si la ventana "
                           "principal se construye en la misma raíz que la de entrar, no la ocultes: quita solo los "
                           "widgets del formulario de entrar.")
        return True
    return False


# Dos peluquerías con dueña y empleada terminaron con roles que no limitaban nada: en una, la empleada veía las
# ganancias; en otra, cualquiera pulsaba «Entrar como Dueña» sin contraseña. La sonda entra como cada usuario del código
# y compara lo que ve y puede hacer cada uno (pestañas, botones, menús y mensajes al usarlos).
ROLES = re.compile(r"(?i)\b(?:rol|role|roles|perfil|permisos?|es_admin|is_admin)\b")


def _vista(d: dict, usuarios: list[str]) -> tuple:
    # Rutas (cada pasada usa otra carpeta temporal), usuarios y números no distinguen un rol de otro.
    quitar = re.compile("|".join([r"[A-Za-z]:[\\/][^\s'\"]*", *map(re.escape, sorted(usuarios, key=len, reverse=True)),
                                  r"\d+"]), re.IGNORECASE)
    v = d.get("vista") or {}
    limpio = [{quitar.sub("", str(t)).strip() for t in v.get(k, [])} - {""} for k in ("pestanas", "botones", "menus")]
    limpio.append({quitar.sub("", m["texto"]).strip() for m in d["mensajes"] if m["fase"] in ("pestanas", "botones")})
    return tuple(tuple(sorted(s)) for s in limpio)


def probar_tk(raiz: Path, entrada: Path, python: str, inf: Informe, segundos: int, bat: Path | None = None) -> None:
    rel = relativa(entrada, raiz)
    tramo = min(20, max(5, segundos))
    claves = claves_del_proyecto(raiz)
    usuarios = usuarios_del_proyecto(raiz, claves)
    codigo, salida, d = _sonda_tk(raiz, entrada, python, tramo, claves, usuarios, [])
    if d is None:
        if codigo is None and "SONDA_TK_VENTANA" in salida:
            inf.avisos.append(f"{rel}: la ventana no se pudo recorrer en {tramo} s (¿espera algo sin volver al bucle de "
                              "eventos?).")
        elif codigo is None:
            inf.ok.append(f"{rel} sigue en ejecución tras {tramo} s (servidor o bucle): arrancó sin errores")
        elif "No module named 'tkinter'" in salida or "No module named '_tkinter'" in salida:
            inf.avisos.append(f"{rel}: el Python del proyecto no tiene tkinter: la ventana no se ha probado.")
        else:
            inf.errores.append(f"{rel}: falló al arrancar:\n    {cola(salida)}")
        return
    if _fallo_grave(rel, d, inf):
        return
    for e in [e for e in d["errores"] if e["fase"] == "botones"][:2]:
        inf.avisos.append(f"{rel}: EXCEPCIÓN al pulsar un botón o una opción del menú con los campos vacíos (la sonda "
                          f"pulsa cada uno una vez):\n    {e['traza']}")
    if not d["bucle"]:
        ventanas = [p for p in archivos(raiz, ".py", ".pyw") if not p.name.startswith("test")
                    and re.search(r"\bmainloop\s*\(", p.read_text(encoding="utf-8", errors="replace"))]
        if bat and ventanas:
            otros = [relativa(p, raiz) for p in ventanas if p.resolve() != entrada.resolve()]
            inf.errores.append(f"{relativa(bat, raiz)} abre {rel}, que termina sin abrir ninguna ventana: con doble clic el "
                               "usuario no ve nada. " + (f"La ventana se abre en {', '.join(otros[:2])}: haz que {bat.name} "
                               "ejecute ese archivo." if otros else "Su mainloop() nunca se ejecuta: llama a lo que crea "
                               "la ventana."))
            return
        inf.ok.append(f"{rel} se ejecuta sin errores (no llega a abrir el bucle de la ventana)")
        return
    hecho = []
    # Una peluquería creaba la base de datos antes de mirar si era la primera vez y nunca creaba admin/admin123: con lo
    # que decían el código y el resumen no entraba nadie. Si el proyecto da un usuario y su contraseña juntos y la
    # pantalla de entrar los rechaza, el usuario no podrá usar la app.
    if (ent := d["entrar"]) is not None and ent["usuario"] is None and ent.get("campo_usuario"):
        pares = {}  # el primer usuario de cada contraseña
        for u, c in pares_del_proyecto(raiz, claves, usuarios):
            if u in usuarios[:12] and c in claves[:6]:
                pares.setdefault(c, u)
        pares = [(u, c) for c, u in pares.items()]
        if pares:
            dice = next((m["texto"] for m in d["mensajes"] if m["fase"] == "entrar"), "")
            inf.errores.append(f"{rel}: con los usuarios y contraseñas que da el proyecto ("
                               + ", ".join(f"«{u}» / «{c}»" for u, c in pares[:2]) + ") no se puede entrar"
                               + (f" (la app dice «{dice[:120]}»)" if dice else "") + ": nadie podrá usar la app. "
                               "Comprueba que esos usuarios existen de verdad al abrirla, también cuando la base de "
                               "datos ya estaba creada.")
            return
    if ent is not None:
        hecho.append(f"entró como «{ent['usuario']}»" if ent["usuario"] is not None else
                     f"pantalla de entrar: no dio con usuario y contraseña del código (probó {ent['intentos']}); lo de "
                     "después no se probó")
    hecho.append(f"{d['pestanas']} pestañas y {d['botones']} botones pulsados, {len(d['mensajes'])} mensajes")
    con_roles = any(ROLES.search(p.read_text(encoding="utf-8", errors="replace"))
                    for p in archivos(raiz, ".py", ".pyw") if not p.name.startswith("test"))
    if con_roles and ent is not None and ent["usuario"] is not None:
        vistas = [(ent["usuario"], _vista(d, usuarios))]
        for _ in range(2):  # hasta 3 usuarios distintos
            _, _, d2 = _sonda_tk(raiz, entrada, python, tramo, claves, usuarios, [u for u, _ in vistas])
            if d2 is None or not d2.get("entrar") or d2["entrar"]["usuario"] is None:
                break
            if _fallo_grave(rel, d2, inf):
                return
            vistas.append((d2["entrar"]["usuario"], _vista(d2, usuarios)))
        nombres = " y ".join(f"«{u}»" for u, _ in vistas)
        if len(vistas) >= 2 and all(v == vistas[0][1] for _, v in vistas[1:]):
            p, b, m, _ = vistas[0][1]
            inf.errores.append(f"{rel}: como {nombres} se ve y se puede hacer exactamente lo mismo ({len(p)} pestañas, "
                               f"{len(b)} botones y {len(m)} órdenes de menú, con los mismos mensajes): los roles no "
                               "limitan nada. Haz que cada rol vea y use solo lo suyo.")
            return
        if len(vistas) >= 2:
            hecho.append(f"entró también como {' y '.join(f'«{u}»' for u, _ in vistas[1:])}: cada uno ve distinto")
    elif con_roles and ent is None and len(set(d.get("como_rol", []))) >= 2:
        botones = ", ".join(f"«{t}»" for t in sorted(set(d["como_rol"]))[:3])
        inf.errores.append(f"{rel}: deja elegir el rol con botones ({botones}) sin pedir contraseña: cualquiera entra con "
                           "el rol que quiera y ve lo que ese rol tiene prohibido. Pide usuario y contraseña al abrir y "
                           "aplica el rol de quien entra.")
        return
    if d.get("abiertos"):
        hecho.append(f"{len(d['abiertos'])} archivos o programas que abriría fuera de la app (no se abrieron)")
    inf.ok.append(f"{rel}: ventana usada sin enseñarla, sin excepciones ({'; '.join(hecho)})")


# ---------------------------------------------------------------- HTML / JS
class _HtmlInfo(html.parser.HTMLParser):
    VACIOS = frozenset({"area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source",
                        "track", "wbr", "param", "!doctype"})

    def __init__(self):
        super().__init__()
        self.ids, self.refs, self.scripts, self.pila, self.mal_cerradas = [], [], [], [], []
        self.modulo, self._en_script = False, False

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if a.get("id"):
            self.ids.append(a["id"])
        for k in ("src", "href"):
            v = (a.get(k) or "").strip()
            if v and not re.match(r"(https?:|//|data:|#|mailto:|javascript:|tel:|\{|\$)", v) and not (tag == "a" and not v.endswith((".html", ".htm"))):
                self.refs.append(v.split("?")[0].split("#")[0])
        if tag == "script":
            self._en_script = not a.get("src")
            self.modulo |= a.get("type") == "module"
        if tag not in self.VACIOS and not tag.startswith("!"):
            self.pila.append(tag)

    def handle_endtag(self, tag):
        if tag == "script":
            self._en_script = False
        if tag in self.pila:
            while self.pila and self.pila[-1] != tag:
                self.mal_cerradas.append(self.pila.pop())
            self.pila.pop()

    def handle_data(self, data):
        if self._en_script and data.strip():
            self.scripts.append(data)


CDN = re.compile(r"cdnjs\.cloudflare\.com/ajax/libs/([\w.-]+)/(r?\d[\d.]*)|"
                 r"(?:cdn\.jsdelivr\.net/npm|unpkg\.com)/(@?[\w.-]+(?:/[\w.-]+)?)@(\d[\d.]*)")
_ULTIMAS: dict[str, str | None] = {}


def ultima_npm(paquete: str) -> str | None:
    """Versión actual en npm (None sin red o si no existe); se recuerda durante la sesión."""
    if paquete not in _ULTIMAS:
        url = f"https://registry.npmjs.org/{urllib.parse.quote(paquete, safe='@/')}/latest"
        try:
            with urllib.request.urlopen(url, timeout=5) as r:
                _ULTIMAS[paquete] = json.loads(r.read())["version"]
        except Exception:
            _ULTIMAS[paquete] = None
    return _ULTIMAS[paquete]


def _atrasada(usada: str, actual: str) -> bool:
    u, a = ([int(x) for x in re.findall(r"\d+", v)[:2]] + [0, 0] for v in (usada, actual))
    return a[0] - u[0] >= 2 if a[0] >= 1 else a[1] - u[1] >= 30  # 0.x (three): 30 versiones menores


def librerias_cdn(raiz: Path, htmls: list[Path], inf: Informe) -> None:
    """Librerías cargadas desde un CDN con una versión muy antigua (p. ej. three.js r128 de 2021)."""
    vistas = set()
    for h in htmls:
        for m in CDN.finditer(h.read_text(encoding="utf-8", errors="replace")):
            nombre, usada = (m.group(1), m.group(2)) if m.group(1) else (m.group(3), m.group(4))
            if usada.startswith("r"):  # cdnjs publica three.js como r128 = 0.128.0
                usada = f"0.{usada[1:]}.0"
            if (nombre, usada) in vistas:
                continue
            vistas.add((nombre, usada))
            # cdnjs "three.js" es npm "three" (en npm hay un "three.js" abandonado): gana la versión más alta
            candidatos = [(p, v) for p in dict.fromkeys((nombre, re.sub(r"\.js$", "", nombre, flags=re.IGNORECASE).lower()))
                          if (v := ultima_npm(p))]
            paquete, actual = max(candidatos, key=lambda c: [int(x) for x in re.findall(r"\d+", c[1])],
                                  default=(None, None))
            if actual and _atrasada(usada, actual):
                consejo = (' como módulo ES: <script type="importmap"> + import * as THREE from "three" (guia web)'
                           if paquete == "three" else "")
                inf.errores.append(f"{relativa(h, raiz)}: {nombre} {usada} está DESACTUALIZADA (la actual es {actual}). "
                                   f"Usa la versión actual{consejo}.")


def es_plantilla(h: Path, raiz: Path) -> bool:
    """Plantilla de servidor (Jinja, Django): {{ url_for(...) }} y {% %} no son HTML ni JS hasta que el servidor la entrega."""
    return "templates" in h.relative_to(raiz).parts or "{%" in h.read_text(encoding="utf-8", errors="replace")


def html_estatico(raiz: Path, htmls: list[Path], jss: list[Path], inf: Informe) -> None:
    for h in htmls:
        p = _HtmlInfo()
        p.feed(h.read_text(encoding="utf-8", errors="replace"))
        for i in {x for x in p.ids if p.ids.count(x) > 1}:
            inf.avisos.append(f"{relativa(h, raiz)}: id '{i}' repetido.")
        for ref in p.refs:
            if not ((raiz / ref.lstrip("/")) if ref.startswith("/") else (h.parent / ref)).exists():
                inf.errores.append(f"{relativa(h, raiz)} referencia '{ref}' y NO existe.")
        abiertas = [t for t in p.mal_cerradas + p.pila if t not in ("html", "body", "head", "p", "li", "td", "tr", "option")]
        if abiertas:
            inf.avisos.append(f"{relativa(h, raiz)}: etiquetas sin cerrar: {', '.join(sorted(set(abiertas)))[:120]}")
        for i, s in enumerate(p.scripts):
            _node_check(raiz, s, f"{relativa(h, raiz)} <script> #{i + 1}", inf, p.modulo)
    cargas = cache(partial(_cargas_html, raiz))  # solo se miran los .html si algún .js tiene JSX, y una vez
    jsx = [relativa(j, raiz) for j in jss if _node_check(raiz, j, relativa(j, raiz), inf, cargas=cargas)]
    if jsx:
        inf.avisos.append(f"{len(jsx)} archivo(s) .js con JSX ({', '.join(jsx[:3])}{', …' if len(jsx) > 3 else ''}): Node no "
                          "entiende JSX y no se comprobó su sintaxis. Es normal en React o Docusaurus: no los cambies por esto.")


# getElementById("x") / querySelector("#x ...") con texto literal: una plantilla (${...}), una variable o una suma
# no encajan (no se sabe qué id será) y no se miran.
BUSCA_ID = re.compile(r"""getElementById\(\s*['"`]([\w-]+)['"`]\s*\)|querySelector(?:All)?\s*(?:<[^<>()]*>)?"""
                      r"""\(\s*['"`]#([\w-]+)(?:[\s>+~.:\[,][^'"`$]*)?['"`]\s*\)""")
# Ids que crea el código: el.id = "x", setAttribute("id", "x"), id="x" en innerHTML o JSX, id={"x"}, {id: "x"}.
CREA_ID = re.compile(r"""\.id\s*=\s*['"`]([\w-]+)['"`]|setAttribute\(\s*['"`]id['"`]\s*,\s*['"`]([\w-]+)['"`]|"""
                     r"""(?<![\w.-])id\s*[=:]\s*\{?\s*\\?['"`]([\w-]+)\\?['"`]""")
SCRIPT_EN_LINEA = re.compile(r"<script\b[^>]*>(.*?)</script>", re.IGNORECASE | re.DOTALL)
FUENTES_DOM = (".js", ".mjs", ".cjs", ".ts", ".tsx", ".jsx")
# Dos webs tenían «+ Nuevo producto» con onclick="abrirModalProducto()" y esa función no existía: al pulsar no pasaba
# nada (el navegador lo dice en una consola que nadie mira). También vale el HTML que construyen los scripts.
MANEJADOR = re.compile(r"""\bon(?:click|submit|change|input|keyup|keydown|keypress|blur|focus|dblclick|load)\s*=\s*"""
                       r"""\\?(["'])(.*?)\\?\1""", re.IGNORECASE | re.DOTALL)
LLAMADA_SUELTA = re.compile(r"(?<![\w$.])([A-Za-z_$][\w$]*)\s*\(")
DEFINE_FUNCION = re.compile(r"""\bfunction\s*\*?\s*([A-Za-z_$][\w$]*)\s*\(|\b(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*="""
                            r"""|\b(?:window|globalThis|self)\s*(?:\.\s*([A-Za-z_$][\w$]*)|\[\s*['"]([\w$]+)['"]\s*\])\s*="""
                            r"""|^\s*([A-Za-z_$][\w$]*)\s*=\s*(?:async\b|function\b|\()""", re.MULTILINE)
A_WINDOW = re.compile(r"Object\.assign\(\s*(?:window|globalThis|self)\s*,\s*\{([^}]*)\}")
SCRIPT_SRC = re.compile(r"""<script\b([^>]*?)\bsrc\s*=\s*(?:"([^"]*)"|'([^']*)')([^>]*)>""", re.IGNORECASE)
SCRIPT_CON_ATRIBUTOS = re.compile(r"<script\b([^>]*)>(.*?)</script>", re.IGNORECASE | re.DOTALL)
PLANTILLA_EN_CODIGO = re.compile(r"\{\{.*?\}\}|\{%.*?%\}|\$\{[^}]*\}|'(?:\\.|[^'\\])*'|\"(?:\\.|[^\"\\])*\"|"
                                 r"`(?:\\.|[^`\\])*`|&\w+;", re.DOTALL)  # plantillas, textos entre comillas y &quot;
GLOBALES_JS = {"if", "for", "while", "switch", "return", "function", "typeof", "new", "catch", "await", "void", "delete",
               "alert", "confirm", "prompt", "setTimeout", "setInterval", "clearTimeout", "clearInterval", "fetch", "open",
               "close", "print", "eval", "parseInt", "parseFloat", "Number", "String", "Boolean", "Array", "Object",
               "Date", "JSON", "Math", "isNaN", "isFinite", "encodeURIComponent", "decodeURIComponent", "encodeURI",
               "decodeURI", "requestAnimationFrame", "scrollTo", "scrollBy", "structuredClone", "queueMicrotask",
               "Promise", "Error", "RegExp", "Symbol", "BigInt", "Intl", "URL", "URLSearchParams", "FormData", "Blob",
               "Event", "CustomEvent", "Audio", "Image", "atob", "btoa", "require", "$", "jQuery", "_"}


def _id_parecido(i: str, ids: set[str]) -> str:
    """El id existente más parecido: primero entre los que empiezan igual (cpu-percent -> cpu-usage), si no, entre
    todos los muy parecidos."""
    inicio = re.split(r"[-_]", i)[0]
    mismos = [x for x in sorted(ids) if re.split(r"[-_]", x)[0] == inicio]
    return next(iter(difflib.get_close_matches(i, mismos, 1, 0.4) or difflib.get_close_matches(i, sorted(ids), 1, 0.6)),
                "")


def ids_dom(raiz: Path, base: Path, inf: Informe) -> None:
    """El JS/TS de base busca con texto literal un id que no tiene ningún HTML del proyecto ni crea ningún script:
    getElementById devolverá null (fuera de la app, p. ej. en Tauri, init() vuelve antes y la prueba no lo ve)."""
    todos = archivos(raiz, *FUENTES_DOM, ".vue", ".svelte", ".html", ".htm")
    if not any(p.suffix.lower() in (".html", ".htm") for p in todos):
        return  # sin páginas (una librería) no hay DOM con el que comparar
    definidos, usos = set(), {}  # usos: id -> ["archivo:línea", ...]
    for p in todos:
        texto, suf = p.read_text(encoding="utf-8", errors="replace"), p.suffix.lower()
        definidos |= {m[m.lastindex] for m in CREA_ID.finditer(texto)}
        if suf in (".html", ".htm"):
            info = _HtmlInfo()
            info.feed(texto)
            definidos |= set(info.ids)
            trozos = [(m.start(1), m[1]) for m in SCRIPT_EN_LINEA.finditer(texto)]
        else:
            trozos = [(0, texto)]
        if suf in (".vue", ".svelte") or p.name.endswith(".min.js") or base not in (p, *p.parents):
            continue
        for desde, codigo in trozos:
            for m in BUSCA_ID.finditer(codigo):
                antes = codigo[codigo.rfind("\n", 0, m.start()) + 1:m.start()]
                if "//" in antes or antes.lstrip().startswith(("*", "/*")):
                    continue  # comentado
                linea = texto.count("\n", 0, desde + m.start()) + 1
                usos.setdefault(m[m.lastindex], []).append(f"{relativa(p, raiz)}:{linea}")
    for i, sitios in usos.items():
        if i in definidos:
            continue
        parecido = _id_parecido(i, definidos)
        inf.errores.append(f"{sitios[0]} busca el elemento id='{i}' pero no existe en ningún HTML ni lo crea ningún "
                           "script (devolverá null)." + (f" ¿Querías '{parecido}', que sí existe?" if parecido else "")
                           + (f" También en {', '.join(sitios[1:4])}." if len(sitios) > 1 else ""))


def _globales(codigo: str, modulo: bool) -> set[str]:
    """Las funciones que un script deja a la vista de onclick: en un módulo (type="module") solo las de window, también
    las de Object.assign(window, {abrir, cerrar: cerrarModal})."""
    nombres = {next(g for g in m.groups() if g) for m in DEFINE_FUNCION.finditer(codigo) if not modulo or m[3] or m[4]}
    for m in A_WINDOW.finditer(codigo):
        nombres |= {re.split(r"\s*:", n.strip())[0] for n in m[1].split(",") if n.strip()}
    return nombres


def funciones_de_botones(raiz: Path, base: Path, inf: Informe) -> None:
    """Cada manejador (onclick="abrir()"...) de base llama a una función suelta que existe en algún script del proyecto
    (archivos o <script> de las páginas y plantillas) o en el navegador. Dos webs cargaban app.js con type="module":
    sus funciones no se ven desde onclick y «+ Nuevo producto» no hacía nada."""
    todos = archivos(raiz, *FUENTES_DOM, ".html", ".htm")
    if not any(p.suffix.lower() in (".html", ".htm") for p in todos):
        return
    clasicos, modulos = set(), set()  # scripts que alguna página carga como script normal / como módulo
    for p in todos:
        if p.suffix.lower() in (".html", ".htm"):
            for m in SCRIPT_SRC.finditer(p.read_text(encoding="utf-8", errors="replace")):
                src = m[2] if m[2] is not None else m[3]
                if (flask := re.search(r"""filename\s*=\s*['"]([^'"]+)""", src)):  # url_for('static', filename=...)
                    src = "static/" + flask[1]
                src = src.split("?")[0]
                destino = next((c for c in (p.parent / src, raiz / src.lstrip("/")) if c.is_file()), p.parent / src)
                es_modulo = re.search(r"""type\s*=\s*["']?module""", m[1] + m[4], re.IGNORECASE)
                (modulos if es_modulo else clasicos).add(destino.resolve())
    definidas, en_modulo, usos = set(GLOBALES_JS), {}, {}  # usos: función -> ["archivo:línea", ...]
    for p in todos:
        texto = p.read_text(encoding="utf-8", errors="replace")
        if p.suffix.lower() in FUENTES_DOM:
            trozos = [(p.resolve() in modulos and p.resolve() not in clasicos, texto)]
        else:
            trozos = [(bool(re.search(r"""type\s*=\s*["']?module""", m[1], re.IGNORECASE)), m[2])
                      for m in SCRIPT_CON_ATRIBUTOS.finditer(texto)]
        for modulo, codigo in trozos:
            definidas |= _globales(codigo, modulo)
            if modulo:
                en_modulo.update(dict.fromkeys(_globales(codigo, False) - _globales(codigo, True), relativa(p, raiz)))
        if p.name.endswith(".min.js") or base not in (p, *p.parents):
            continue
        for m in MANEJADOR.finditer(texto):
            linea = texto.count("\n", 0, m.start()) + 1
            for llamada in LLAMADA_SUELTA.finditer(PLANTILLA_EN_CODIGO.sub("0", m[2])):
                usos.setdefault(llamada[1], []).append(f"{relativa(p, raiz)}:{linea}")
    for nombre, sitios in sorted(usos.items()):
        if nombre in definidas:
            continue
        if nombre in en_modulo:
            motivo = (f"está en {en_modulo[nombre]}, que se carga como módulo (type=\"module\"), y las funciones de un "
                      f"módulo no se ven desde onclick. Quita type=\"module\" o añade window.{nombre} = {nombre};")
        else:
            parecida = difflib.get_close_matches(nombre, sorted(definidas - GLOBALES_JS), 1, 0.7)
            motivo = "no está en ningún script del proyecto." + (f" ¿Querías {parecida[0]}()?" if parecida else "")
        inf.errores.append(f"{sitios[0]} un botón llama a {nombre}() y al pulsarlo no pasa nada (el navegador dice "
                           f"«{nombre} is not defined»): {motivo}"
                           + (f" También en {', '.join(dict.fromkeys(sitios[1:4]))}." if len(sitios) > 1 else ""))


# React, Docusaurus o Next escriben JSX dentro de .js y node no lo entiende: en la documentación de un repo ajeno daba «ERROR DE SINTAXIS JS» en
# cinco archivos correctos, y el modelo habría «arreglado» código ajeno (m4198).
LIBRERIAS_JSX = {"react", "react-dom", "react-native", "preact", "solid-js", "next", "ink", "gatsby", "expo", "react-scripts",
                 "@docusaurus/core", "@babel/preset-react"}
IMPORTA_JSX = re.compile(r"""(?:\bfrom|\brequire\(|^[ \t]*import)\s*['"](?:(?:react|react-dom|react-native|preact|solid-js|next|"""
                         r"""ink|gatsby|expo)(?:/[^'"\n]*)?|(?:@docusaurus|@theme|@theme-original|@site)/[^'"\n]*)['"]""",
                         re.MULTILINE)
ABRE_JSX = re.compile(r"<(?:>|[A-Za-z_$][\w$.:-]*(?=[\s/>{]|$))")  # <p, <Foo.Bar, <>; no <!DOCTYPE, «< 1» ni <?xml


def _senala_jsx(salida: str) -> bool:
    """¿El «Unexpected token '<'» de node apunta al principio de una etiqueta o de un fragmento?"""
    if "SyntaxError: Unexpected token '<'" not in salida:
        return False
    for fuente, marca in pairwise(salida.splitlines()):
        if marca.strip() and not marca.strip().strip("^"):
            # node cuenta la columna en UTF-16: un emoji antes del «<» ocupa dos
            antes = fuente.encode("utf-16-le", "surrogatepass")[:2 * marca.index("^")]
            return bool(ABRE_JSX.match(fuente, len(antes.decode("utf-16-le", "replace"))))
    return False


def _cargas_html(raiz: Path) -> dict[Path, set[str]]:
    """Con qué type carga cada .html del proyecto cada script ('' = sin type)."""
    cargas: dict[Path, set[str]] = {}
    for h in archivos(raiz, ".html", ".htm"):
        for m in SCRIPT_SRC.finditer(h.read_text(encoding="utf-8", errors="replace")):
            antes, doble, simple, despues = m.groups()
            src = (doble or simple or "").split("?")[0].split("#")[0].strip()
            if not src or re.match(r"(?i)(https?:|//|data:)", src):
                continue
            tipo = re.search(r"""\btype\s*=\s*["']?([\w/+.-]+)""", f"{antes} {despues}", re.IGNORECASE)
            destino = (raiz / src.lstrip("/")) if src.startswith("/") else h.parent / src
            cargas.setdefault(destino.resolve(), set()).add(tipo.group(1).lower() if tipo else "")
    return cargas


def _dependencias_cerca(carpeta: Path, raiz: Path) -> set[str]:
    """Lo que declaran los package.json desde la carpeta del archivo hasta la raíz del proyecto."""
    deps: set[str] = set()
    for d in (carpeta, *carpeta.parents):
        try:
            pkg = json.loads((d / "package.json").read_text(encoding="utf-8", errors="replace"))
            deps |= {k for clave in ("dependencies", "devDependencies", "peerDependencies") for k in pkg.get(clave) or ()}
        except (OSError, ValueError, AttributeError, TypeError):
            pass
        if d == raiz or raiz not in d.parents:
            break
    return deps


def _es_jsx_legitimo(raiz: Path, ruta: Path, salida: str, cargas) -> bool:
    """Un .js que node rechaza por JSX y que algo transforma antes de ejecutarlo. Ante la duda, no (se acusa)."""
    if not _senala_jsx(salida):
        return False
    texto = ruta.read_text(encoding="utf-8", errors="replace")
    if texto.lstrip("﻿ \t\r\n").startswith("<"):  # una página HTML guardada como .js
        return False
    tipos = cargas().get(ruta.resolve(), set())
    if tipos & {"text/babel", "text/jsx"}:
        return True
    deps = _dependencias_cerca(ruta.parent, raiz)
    # script corriente en un .html: el navegador lo recibe tal cual (salvo que Parcel empaquete desde ese .html)
    if tipos and not deps & {"parcel", "parcel-bundler"}:
        return False
    return bool(deps & LIBRERIAS_JSX or IMPORTA_JSX.search(texto))


def _node_check(raiz: Path, codigo_o_ruta, nombre: str, inf: Informe, modulo: bool = False, cargas=None) -> bool:
    """Devuelve True si era JSX en un .js: node no puede comprobar su sintaxis y no se acusa."""
    if not shutil.which("node"):
        return False
    if isinstance(codigo_o_ruta, Path):
        ruta, temporal = codigo_o_ruta, None
    else:
        fd, temporal = tempfile.mkstemp(suffix=".mjs" if modulo else ".js")
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(codigo_o_ruta)
        ruta = Path(temporal)
    try:
        codigo, salida = correr_texto(["node", "--check", str(ruta)], raiz, 30)
    finally:
        if temporal:
            os.unlink(temporal)
    if codigo not in (0, None):
        if temporal is None and _es_jsx_legitimo(raiz, ruta, salida, cargas or partial(_cargas_html, raiz)):
            return True
        lineas = [l for l in salida.splitlines() if l.strip()]
        detalle = next((l for l in lineas if "Error" in l), lineas[-1] if lineas else "")
        donde = next((l.rsplit(":", 1)[-1] for l in lineas if re.search(r":\d+$", l)), "?")
        inf.errores.append(f"{nombre}:{donde} ERROR DE SINTAXIS JS: {detalle[:160]}")
    return False


class Silencioso(SimpleHTTPRequestHandler):
    def log_message(self, *a):
        pass


def canal_navegador(p):
    for canal in ("msedge", "chrome", None):
        try:
            return p.chromium.launch(channel=canal, headless=True) if canal else p.chromium.launch(headless=True)
        except Exception:
            continue
    return None


JS_CANVAS = """() => [...document.querySelectorAll('canvas')].map(c => {
  let colores = -1;
  try { const ctx = c.getContext('2d'); if (ctx) { const d = ctx.getImageData(0, 0, c.width, c.height).data; const s = new Set();
    for (let i = 0; i < d.length; i += 4 * 97) s.add(d[i] + ',' + d[i+1] + ',' + d[i+2] + ',' + d[i+3]); colores = s.size; } } catch (e) {}
  return {id: c.id, w: c.width, h: c.height, colores};
})"""


VITE_FUENTE = re.compile(r"""<script\b[^>]*\bsrc=["']([^"']+\.(?:ts|tsx|jsx|vue|svelte))["']""", re.IGNORECASE)


def fuente_vite(h: Path) -> str:
    """El script de la página es .ts/.tsx/.jsx/.vue: solo funciona servida por Vite o compilada (dist/)."""
    try:
        m = VITE_FUENTE.search(h.read_text(encoding="utf-8", errors="replace"))
    except OSError:
        return ""
    return m.group(1) if m else ""


def paginas_vite(raiz: Path, fuentes: list[Path], inf: Informe) -> None:
    """Prueba en el navegador la versión compilada de cada página fuente de Vite (la deja npm run build en dist/)."""
    for h in fuentes:
        paquete = next((d for d in (h.parent, *h.parent.parents) if (d / "package.json").exists()), None)
        compilada = paquete / "dist" / h.relative_to(paquete) if paquete and raiz in (paquete, *paquete.parents) else None
        if compilada and compilada.exists():
            antes = len(inf.ok)
            navegador(raiz, [compilada], inf, raiz_web=paquete / "dist")
            if (paquete / "src-tauri" / "tauri.conf.json").exists():  # el agente tomaba los «--» por un fallo
                inf.ok[antes:] = [o + " (en un navegador normal, FUERA de Tauri: invoke no existe y sin sus datos es lo "
                                  "esperado; la app de verdad se prueba aparte)" for o in inf.ok[antes:]]
        else:
            inf.avisos.append(f"{relativa(h, raiz)}: es la página fuente de Vite (carga {fuente_vite(h)}) y no se abre tal cual; "
                              "se prueba la compilada de dist/ cuando `npm run build` funciona.")


# Rutas que define el servidor del proyecto (@app.route("/api/me"), @router.get("/x/{id}"), url_prefix, o
# `if path == "/api/x"` en http.server). Abierta como archivo, sin el servidor, la página no puede pedirlas: daban 404
# falsos que una misión persiguió hasta quedarse sin pasos.
RUTA_PY = re.compile(r"""\.(?:route|get|post|put|patch|delete|api_route)\(\s*[rRuU]?['"](/[^'"]*)['"]"""
                     r"""|add_url_rule\(\s*['"](/[^'"]*)['"]|\b(?:path|ruta|url)\s*==\s*['"](/[^'"]*)['"]""")
EMPIEZA_PY = re.compile(r"""\b(?:path|ruta|url)\.startswith\(\s*['"](/[^'"]+)['"]""")
PREFIJO_PY = re.compile(r"""(?:url_prefix|prefix)\s*=\s*['"](/[^'"]*)['"]""")
JSON_ROTO = re.compile(r"not valid JSON|Unexpected token '<'|Unexpected end of JSON|Failed to fetch|NetworkError")


def rutas_servidor(raiz: Path) -> list[re.Pattern]:
    rutas, empiezan, prefijos = set(), set(), {""}
    for p in archivos(raiz, ".py"):
        texto = p.read_text(encoding="utf-8", errors="replace")
        rutas |= {next(x for x in grupos if x) for grupos in RUTA_PY.findall(texto)}
        empiezan |= set(EMPIEZA_PY.findall(texto))
        prefijos |= set(PREFIJO_PY.findall(texto))
    return ([re.compile("[^/]+".join(map(re.escape, re.split(r"<[^>]*>|\{[^}]*\}", (pre + r).strip("/")))))
             for r in rutas for pre in prefijos]
            + [re.compile(re.escape(e.strip("/")) + ".*") for e in empiezan])


def del_servidor(ruta: str, rutas: list[re.Pattern]) -> bool:
    """¿La pide la página a su servidor? Un {{ url_for }} sin procesar, o una ruta que define el servidor del proyecto."""
    ruta = urllib.parse.unquote(ruta.split("?")[0].split("#")[0])
    return "{{" in ruta or "{%" in ruta or any(p.fullmatch(ruta.strip("/")) for p in rutas)


def navegador(raiz: Path, htmls: list[Path], inf: Informe, raiz_web: Path | None = None) -> None:
    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        inf.avisos.append("Prueba en navegador no disponible (falta playwright en el agente: ejecuta instalar.bat).")
        return
    paginas = sorted(htmls, key=lambda p: (p.name != "index.html", len(p.parts)))[:2]
    rutas = rutas_servidor(raiz) if raiz_web is None else []
    web = raiz_web or raiz  # dist/ de Vite se sirve desde su carpeta: sus rutas /assets/... son absolutas
    servidor = ThreadingHTTPServer(("127.0.0.1", 0), partial(Silencioso, directory=str(web)))
    threading.Thread(target=servidor.serve_forever, daemon=True).start()
    capturas = raiz / ".agente" / "capturas"
    capturas.mkdir(parents=True, exist_ok=True)
    try:
        with sync_playwright() as p:
            nav = canal_navegador(p)
            if nav is None:
                inf.avisos.append("No se pudo abrir Edge/Chrome sin ventana para probar la página.")
                return
            for h in paginas:
                rel, url = relativa(h, raiz).replace("\\", "/"), h.relative_to(web).as_posix()
                pag = nav.new_page(viewport={"width": 1024, "height": 768})
                problemas = []
                pag.on("pageerror", lambda e, problemas=problemas: problemas.append(("error", f"EXCEPCIÓN JS: {e}")))
                pag.on("console", lambda m, problemas=problemas: problemas.append(("error" if m.type == "error" else "aviso", f"consola.{m.type}: {m.text[:200]}"))
                       if m.type in ("error", "warning") else None)
                pag.on("requestfailed", lambda r, problemas=problemas: problemas.append(("error", f"no cargó {r.url.split('/', 3)[-1]}")))
                pag.on("response", lambda r, problemas=problemas: problemas.append(("error", f"HTTP {r.status}: {r.url.split('/', 3)[-1]}"))
                       if r.status >= 400 and not r.url.endswith("favicon.ico") else None)
                try:
                    pag.goto(f"http://127.0.0.1:{servidor.server_port}/{url}", wait_until="load", timeout=20000)
                except Exception as e:
                    inf.errores.append(f"{rel}: no se pudo abrir: {e}")
                    continue
                pag.wait_for_timeout(1200)
                c1 = pag.screenshot()
                pag.wait_for_timeout(1300)
                c2 = pag.screenshot(path=str(capturas / f"{h.stem}_1_inicio.png"))
                canvas = pag.evaluate(JS_CANVAS)
                objetivo = pag.query_selector("canvas") or pag.query_selector("button") or pag.query_selector("body")
                try:
                    objetivo.click(timeout=2000)
                except Exception:
                    pass
                for tecla in ["Enter", "Space"] + ["ArrowLeft"] * 4 + ["ArrowRight"] * 4 + ["ArrowUp", "ArrowDown", "Space", "KeyA", "KeyD"]:
                    pag.keyboard.down(tecla)
                    pag.wait_for_timeout(60)
                    pag.keyboard.up(tecla)
                pag.wait_for_timeout(800)
                c3 = pag.screenshot(path=str(capturas / f"{h.stem}_2_tras_teclas.png"))
                pag.wait_for_timeout(2500)  # ya en juego: ¿se mueve algo sin tocar nada?
                c4 = pag.screenshot(path=str(capturas / f"{h.stem}_3_jugando.png"))
                try:
                    texto = re.sub(r"\s+", " ", pag.inner_text("body"))
                except Exception:
                    texto = ""
                juego = bool(canvas) and len(texto) < 600  # mucho texto = web con canvas decorativo, no un juego
                texto = texto[:160]
                vistos = set()
                con_http = {m.split(": ", 1)[1] for _, m in problemas if m.startswith("HTTP ")}
                pedidas = sorted({m.split(": ", 1)[1] if m.startswith("HTTP ") else m[9:] for _, m in problemas
                                  if (m.startswith("HTTP ") and del_servidor(m.split(": ", 1)[1], rutas))
                                  or (m.startswith("no cargó ") and del_servidor(m[9:], rutas))})
                for nivel, msg in problemas:
                    if (msg in vistos or "Failed to load resource" in msg
                            or (msg.startswith("no cargó ") and msg[9:] in con_http)
                            or (msg.startswith(("HTTP ", "no cargó ")) and (msg.split(": ", 1)[-1] in pedidas or msg[9:] in pedidas))):
                        continue
                    vistos.add(msg)
                    if pedidas and JSON_ROTO.search(msg):
                        nivel = "aviso"  # le llegó la página de error de una ruta que sí sirve el servidor
                    (inf.errores if nivel == "error" else inf.avisos).append(f"{rel}: {msg}")
                if pedidas:
                    inf.avisos.append(f"{rel}: abierta como archivo, sin tu servidor, no pudo pedir "
                                      f"{', '.join(urllib.parse.unquote(x) for x in pedidas[:4])}: esas rutas las sirve tu servidor. "
                                      "Pruébalas con el servidor en marcha (ejecutar en segundo plano y ver_pantalla).")
                for c in canvas:
                    if c["colores"] == 1:
                        inf.errores.append(f"{rel}: el canvas '{c['id']}' está VACÍO (no se dibuja nada).")
                if juego and c1 == c2 == c3:
                    inf.errores.append(f"{rel}: la página no se anima ni reacciona a teclado/ratón (¿falla el bucle "
                                       "requestAnimationFrame o los eventos?).")
                elif juego and c2 == c3:
                    inf.avisos.append(f"{rel}: nada cambia al pulsar teclas/clic (¿listeners de teclado o estado de inicio?).")
                if juego and c3 == c4:
                    inf.avisos.append(f"{rel}: tras empezar (Enter/Espacio/clic) nada se mueve en 2,5 s: ¿aparecen y se "
                                      "mueven los objetos del juego?")
                anima = "sí" if c1 != c2 or c3 != c4 else "no"
                inf.ok.append(f"{rel} abierta en el navegador (anima: {anima}, cambia con entrada: "
                              f"{'sí' if c2 != c3 else 'no'}). Texto: \"{texto}\". Capturas en .agente/capturas/")
                pag.close()
            nav.close()
    finally:
        servidor.shutdown()
        servidor.server_close()


# ---------------------------------------------------------------- Java
def _de_android(p: Path, base: Path) -> bool:
    """¿El .java es de un proyecto Android o de la carpeta android/ de Flutter? (javac solo no tiene sus librerías).
    También el Java que saca jadx de un APK (carpeta sources/ con resources/ al lado): es para leer, no compila."""
    for d in (a for a in p.parents if a == base or base in a.parents):
        if (d / "pubspec.yaml").exists() or (d.name == "sources" and (d.parent / "resources").is_dir()):
            return True
        for nombre in ("build.gradle", "build.gradle.kts"):
            if (d / nombre).is_file() and "com.android" in (d / nombre).read_text("utf-8", "replace"):
                return True
    return False


def java(raiz: Path, fuentes: list[Path], inf: Informe, segundos: int) -> None:
    if not inf.ejecutar:  # sin sus dependencias, javac solo daría errores falsos; mvn y java ejecutan
        inf.omitido.append("compilar y ejecutar Java")
        return
    if (raiz / "pom.xml").exists():
        if shutil.which("mvn"):
            codigo, salida = correr_texto("mvn -q -B test", raiz, 600)
            (inf.ok.append("mvn test OK") if codigo == 0 else inf.errores.append(f"mvn test falla:\n    {cola(salida, 25)}"))
            return
        inf.avisos.append("Hay pom.xml pero Maven no está instalado: compilo con javac (sin dependencias ni JUnit).")
    if not shutil.which("javac"):
        inf.errores.append("No hay JDK (javac). Instala uno: scoop install openjdk  (o choco install temurin).")
        return
    salida_dir = raiz / ".agente" / "java_out"
    shutil.rmtree(salida_dir, ignore_errors=True)
    salida_dir.mkdir(parents=True, exist_ok=True)
    principales = [f for f in fuentes if "src\\test" not in str(f) and "/test/" not in f.as_posix()]
    lista = raiz / ".agente" / "java_fuentes.txt"
    lista.write_text("\n".join(f'"{f.as_posix()}"' for f in principales), encoding="utf-8")
    codigo, salida = correr_texto(["javac", "-encoding", "UTF-8", "-Xlint:all", "-d", str(salida_dir), f"@{lista}"], raiz, 180)
    for m in re.finditer(r"^(.+?\.java):(\d+): (error|warning): (.+)$", salida, re.MULTILINE):
        detalle = f"{relativa(Path(m.group(1)), raiz)}:{m.group(2)} {m.group(4)}"
        (inf.errores if m.group(3) == "error" else inf.avisos).append(detalle)
    if codigo != 0:
        if not inf.errores:
            inf.errores.append(f"javac falla:\n    {cola(salida)}")
        return
    inf.ok.append(f"javac: {len(principales)} archivos compilan")
    for f in principales:
        texto = f.read_text(encoding="utf-8", errors="replace")
        if re.search(r"static\s+void\s+main\s*\(", texto):
            paquete = re.search(r"^\s*package\s+([\w.]+)\s*;", texto, re.MULTILINE)
            clase = f"{paquete.group(1)}.{f.stem}" if paquete else f.stem
            codigo, salida = correr_texto(["java", "-Djava.awt.headless=true", "-cp", str(salida_dir), clase], raiz, segundos)
            if codigo is None:
                inf.ok.append(f"{clase} sigue en ejecución tras {segundos} s (bucle/servidor): arrancó sin errores")
            elif "NoSuchElementException" in salida:
                inf.avisos.append(f"{clase} pide datos por teclado (Scanner): pruébalo con 'echo datos| java -cp out {clase}'.")
            elif "HeadlessException" in salida:
                inf.avisos.append(f"{clase} abre ventana (Swing/AWT): no se prueba aquí; separa y prueba la lógica.")
            elif codigo != 0:
                inf.errores.append(f"{clase} termina con excepción:\n    {cola(salida)}")
            else:
                inf.ok.append(f"{clase} se ejecuta sin errores")
            break


# ---------------------------------------------------------------- .bat
# Programas que en Windows son un .cmd o un .bat: lanzados sin "call", el .bat termina con ellos y no ejecuta nada
# más (run.bat hacía npm install y nunca llegaba a npx tauri dev ni a su pause).
LOTES = {"npm", "npx", "yarn", "pnpm", "ng", "vite", "tauri", "tsc", "expo", "eas", "ionic", "cordova", "flutter",
         "gradle", "gradlew", "mvn", "mvnw", "conda", "az", "gcloud", "firebase", "vercel", "netlify", "code"}
SEPARADOR_BAT = re.compile(r"&&|\|\||&")
# «start "" /min cmd /k python app.py» lanza python app.py en otra ventana, que hereda el PATH de este .bat: hay que
# buscar ese programa (una misión lo escribió así y con doble clic la ventana nueva no encontraba Python).
LANZADOR = r'(?:start\s+(?:"[^"]*"\s+)?(?:/\w+\s+)*)?(?:cmd(?:\.exe)?\s+/[ck]\s+"?)?'
PYTHON_SUELTO = re.compile(r"(?i)(python3?|pip3?)(?=[\s\"]|$)")


def ordenes_bat(linea: str) -> list[str]:
    """La línea partida por &&, || y & fuera de comillas; los separadores quedan como elementos sueltos."""
    partes, actual, comillas, i = [], "", False, 0
    while i < len(linea):
        c = linea[i]
        if c == '"':
            comillas = not comillas
        elif not comillas and (m := SEPARADOR_BAT.match(linea, i)) and linea[i - 1:i] != "^":
            partes += [actual, m.group(0)]
            actual, i = "", i + len(m.group(0))
            continue
        actual += c
        i += 1
    return partes + [actual]


def es_lote(orden: str, carpeta: Path | None = None) -> bool:
    """El programa de la orden es un .cmd o un .bat: npm, npx, flutter, gradlew, otro .bat del proyecto..."""
    orden = orden.strip().lstrip("@(").strip()
    programa = orden[1:].split('"')[0] if orden.startswith('"') else (orden.split() or [""])[0]
    nombre = programa.replace("\\", "/").rsplit("/", 1)[-1].lower()
    if nombre.endswith((".bat", ".cmd")) or nombre in LOTES:
        return True
    return bool(carpeta and nombre and "." not in nombre
                and any((carpeta / f"{programa}{ext}").is_file() for ext in (".bat", ".cmd")))


def con_call(linea: str, carpeta: Path | None = None) -> str:
    """Pone call delante de cada orden de la línea que lanza un .cmd o un .bat (cd x && call npm install)."""
    partes = ordenes_bat(linea)
    for i in range(0, len(partes), 2):
        orden = partes[i].lstrip()
        if orden and not re.match(r"(?i)call\s", orden) and es_lote(orden, carpeta):
            partes[i] = partes[i][:len(partes[i]) - len(orden)] + "call " + orden
    return "".join(partes)


# Dónde escucha el programa: port=5055, WEB_PORT = 8765, PUERTO = 8765, def servir(port: int = 8765),
# os.environ.get("PORT", 5055), "port": 3000, add_argument("--port", default=8765), app.listen(3000),
# HTTPServer(("", 8000), ...), --port 8000, python -m http.server 8000, set PORT=5000. Un número de más solo calla el aviso.
ESCUCHA = re.compile(r"""\b\w*(?:port|puerto)\w*['"]?\s*(?::\s*int\s*=|[:=])\s*(?:int\(\s*)?"""
                     r"""(?:os\.(?:environ\.get|getenv)\(\s*['"]\w+['"]\s*,\s*)?['"]?(\d{2,5})"""
                     r"""|add_argument\(\s*['"]-[-\w]*port\w*['"][^)]*?\bdefault\s*=\s*(\d{2,5})"""
                     r"""|\.listen\(\s*(\d{2,5})|Server\(\s*\(\s*['"][^'"]*['"]\s*,\s*(\d{2,5})|--port[= ](\d{2,5})"""
                     r"""|-m\s+http\.server\s+(\d{2,5})|\bset\s+"?PORT=(\d{2,5})""", re.IGNORECASE)
# Sin puerto escrito: Flask 5000, uvicorn y python -m http.server 8000; las herramientas de frontend usan los suyos.
POR_DEFECTO = ((r"\bapp\.run\((?![^)]*\bport\b)", {5000}), (r"\bflask\s+run\b(?!.*--port)", {5000}),
               (r"\buvicorn\b(?!.*--port)", {8000}), (r"-m\s+http\.server(?!\s+\d)", {8000}),
               (r"\b(?:npm|npx|yarn|pnpm|vite|ng)\b", {3000, 4173, 4200, 5173, 8080}))
URL_LOCAL = re.compile(r"https?://(?:localhost|127\.0\.0\.1):(\d{2,5})", re.IGNORECASE)


def puertos_escucha(raiz: Path) -> set[int]:
    """Los puertos en los que puede escuchar el programa, según su código, su configuración y sus .bat."""
    rutas = archivos(raiz, ".py", ".js", ".mjs", ".cjs", ".ts", ".json", ".toml", ".ini", ".cfg", ".yml", ".yaml", ".bat", ".cmd")
    texto = "\n".join(p.read_text(encoding="utf-8", errors="replace") for p in rutas + [raiz / ".env"] if p.is_file())
    puertos = {int(next(g for g in m.groups() if g)) for m in ESCUCHA.finditer(texto)}
    return puertos.union(*(p for patron, p in POR_DEFECTO if re.search(patron, texto, re.IGNORECASE)))


# Un run.bat hacía «python -c "...db.drop_all(); db.create_all()"» antes de seed.py y de app.py: con cada doble clic se
# borraba todo lo que la dueña había dado de alta (m4135_neg4). Se mira el .bat que abre el usuario, no los que existen para
# borrar (reset, limpiar...) ni los de instalar/probar, y solo lo que se ejecuta siempre (sin un «if exist» que lo guarde).
BAT_QUE_BORRA = re.compile(r"(?i)^(test|prueba|instal|setup|build|compil|reset|reinici|limpi|borr|clean|demo)")
BORRA_DATOS = re.compile(r"(?i)\bdrop_all\s*\(|\bdrop\s+(?:table|database)\b|\b(?:os\.remove|\.unlink)\s*\([^)]*\.(?:db|sqlite3?)\b"
                         r"|^(?:del|erase)\b[^\n]*\.(?:db|sqlite3?)\b|^(?:rmdir|rd)\b[^\n]*\b(?:instance|data)\b")


CONSEJO_BORRAR = ("con doble clic se pierde lo que el usuario haya dado de alta. No borres nada al arrancar: usa db.create_all() "
                  "(no toca lo que ya existe), haz que seed.py solo cargue datos si la tabla está vacía y deja el borrado "
                  "para un reset.bat aparte.")
# Un seed.py o el propio app.py que borran las tablas al ejecutarse, lanzados desde el .bat: no dentro de un reset/test (se
# llama a propósito) ni tras un «if» que lo condicione (--reset, TESTING, un archivo que ya existe).
SE_BORRA_A_PROPOSITO = re.compile(r"(?i)reset|reinici|limpi|borr|clear|clean|wipe|drop|test|demo")
CONDICION_DE_BORRAR = re.compile(r"(?i)argv|args|testing|reset|confirm|input|environ|getenv|exists")
SQL_BORRA = re.compile(r"(?i)\bdrop\s+(?:table|database)\b")
ARCHIVO_BASE = re.compile(r"(?i)\.(?:db|sqlite3?)$")


def borra_al_arrancar(ruta: Path) -> tuple[str, int] | None:
    """(la llamada, su línea) si el script borra tablas o el archivo de la base cuando se ejecuta."""
    try:
        arbol = ast.parse(ruta.read_text(encoding="utf-8", errors="replace"))
    except (SyntaxError, ValueError):
        return None

    def buscar(nodo: ast.AST) -> tuple[str, int] | None:
        if isinstance(nodo, (ast.FunctionDef, ast.AsyncFunctionDef)) and SE_BORRA_A_PROPOSITO.search(nodo.name):
            return None
        if isinstance(nodo, ast.If) and CONDICION_DE_BORRAR.search(ast.unparse(nodo.test)):
            return None
        if isinstance(nodo, ast.Call):
            f = nodo.func
            nombre = f.attr if isinstance(f, ast.Attribute) else f.id if isinstance(f, ast.Name) else ""
            cadenas = [a.value for a in nodo.args if isinstance(a, ast.Constant) and isinstance(a.value, str)]
            if nombre == "drop_all":
                return "drop_all(", nodo.lineno
            if nombre in ("execute", "executescript") and any(SQL_BORRA.search(c) for c in cadenas):
                return "DROP TABLE", nodo.lineno
            if nombre in ("remove", "unlink") and any(ARCHIVO_BASE.search(c) for c in cadenas):
                return f"{nombre}({next(c for c in cadenas if ARCHIVO_BASE.search(c))!r})", nodo.lineno
        return next(filter(None, map(buscar, ast.iter_child_nodes(nodo))), None)

    return buscar(arbol)


def scripts_de_linea(carpeta: Path, linea: str) -> list[Path]:
    """Los .py que lanza la línea del .bat (python seed.py, "%PY%" -m app...) y existen junto a él."""
    rutas: list[Path] = []
    for m in OBJETIVO_PY.finditer(linea):
        modulo, citado, suelto, punto = m.groups()
        if punto:
            continue
        base = carpeta / (modulo.replace(".", "/") if modulo else (citado or suelto).replace("%~dp0", ""))
        candidatos = [base.with_name(base.name + ".py"), base / "__main__.py"] if modulo else [base]
        rutas += [c for c in candidatos if "%" not in str(c) and c.is_file() and not c.name.startswith("test")][:1]
    return rutas


def borra_datos_bat(b: Path, lineas: list[str], raiz: Path, inf: Informe) -> None:
    if BAT_QUE_BORRA.match(b.stem):
        return
    guardado, vistos, rel = 0, set(), relativa(b, raiz)
    for i, l in enumerate(lineas):
        if re.match(r"(?i)if\b.*\(\s*$", l):
            guardado += 1
        elif l.startswith(")") and not l.endswith("("):  # «) else (» sigue dentro del if
            guardado = max(0, guardado - 1)
        if guardado or not l or re.match(r"(?i)(rem\b|::|@?echo\b|if\b)", l):
            continue
        if (m := BORRA_DATOS.search(l)):
            inf.errores.append(f"{rel}:{i + 1} borra la base de datos en cada arranque (`{m[0]}`): {CONSEJO_BORRAR}")
            return
        for script in scripts_de_linea(b.parent, l):
            if script not in vistos and (hay := borra_al_arrancar(script)):
                inf.errores.append(f"{rel}:{i + 1} ejecuta {script.name}, que borra la base de datos en cada arranque "
                                   f"(`{hay[0]}` en {relativa(script, raiz)}:{hay[1]}): {CONSEJO_BORRAR}")
                return
            vistos.add(script)


# El README de una entrega mandaba ejecutar install.bat y run.bat, y ninguno existía (m4137_neg1): quien lo seguía no podía usar el
# sistema. Solo se miran los nombres sueltos o con carpetas del proyecto: no los que llevan variables (%USERPROFILE%\x.bat), ni los
# de un entorno virtual (Scripts\activate.bat), ni los de un comando conocido (npm.cmd), ni los que el README dice que se creen.
BAT_CITADO = re.compile(r"(?<![\w%$<>{}.\\/-])((?:[\w.-]+[\\/])*[\w.-]+\.(?:bat|cmd))\b", re.IGNORECASE)
BAT_AJENO = re.compile(r"(?i)(?:^|[\\/])(?:scripts|venv|\.venv|node_modules)[\\/]|^(?:de)?activate\.bat$|^(?:npm|npx|yarn|pip|pnpm)\.cmd$")
SE_CREA = re.compile(r"(?i)\b(?:crea|creas|crear|cree|creal[oa]|genera|generar|escribe|escribir|guarda|guardar|nuevo archivo)\b")


def bat_citado_que_no_existe(raiz: Path, inf: Informe) -> None:
    """Error si un README de la raíz cita un .bat o .cmd que no está en el proyecto."""
    existentes = {p.name.lower() for p in archivos(raiz, ".bat", ".cmd")}
    for readme in sorted(raiz.glob("README*")):
        if not readme.is_file() or readme.suffix.lower() not in ("", ".md", ".txt", ".rst"):
            continue
        faltan: list[str] = []
        for linea in readme.read_text(encoding="utf-8", errors="replace").splitlines():
            for m in BAT_CITADO.finditer(linea):
                nombre = m.group(1)
                if (BAT_AJENO.search(nombre) or SE_CREA.search(linea[:m.start()]) or nombre in faltan
                        or re.split(r"[\\/]", nombre)[-1].lower() in existentes):
                    continue
                faltan.append(nombre)
        if faltan:
            inf.errores.append(f"{readme.name} cita {', '.join(faltan[:4])}, pero no existe en el proyecto: quien siga el README "
                               "no puede usarlo. Crea ese archivo o cambia el README por lo que sí hay.")
            return


# El run.bat de una misión llamaba «resolver_sat problema.opb»: es una herramienta de Aura (como escribir o ejecutar), no
# un comando de Windows, y con doble clic salía «"resolver_sat" no se reconoce». `git` sí es un programa.
HERRAMIENTAS_AURA = frozenset(e["function"]["name"] for e in ESQUEMAS) - {"git"}


def herramienta_de_aura_como_orden(linea: str, carpeta: Path) -> str | None:
    """El nombre de la herramienta de Aura que una línea de .bat lanza como si fuera un comando; None si no hay."""
    for orden in ordenes_bat(linea)[::2]:
        orden = re.sub(f"(?i)^(?:call\\s+)?{LANZADOR}", "", orden.strip().lstrip("@(").strip())
        palabra = (orden.split() or [""])[0].strip('"').lower()
        if palabra in HERRAMIENTAS_AURA and not any((carpeta / f"{palabra}{ext}").is_file() for ext in (".bat", ".cmd", ".exe")):
            return palabra
    return None


# `pip install Flask-SQLAlchemy>=3.1` en cmd: el `>` sin comillas dobles es una redirección. La salida de pip va a un archivo llamado «3.1» (el modelo ve exit=0 y
# nada más) y el paquete se instala sin la versión pedida. Las comillas simples no protegen en cmd. Ferretería (23): 11 archivos sueltos en 9 de 145 carpetas.
PIP_INSTALA = re.compile(r"(?i)\bpip3?(?:\.exe)?\s+install\b|\buv\s+(?:pip\s+install|add)\b")
PIP_ESPEC = re.compile(r"^[A-Za-z0-9][\w.\-]*(?:\[[\w,.\-]+\])?[<>]=?\s*\d")
PIP_SALIDA = re.compile(r"(?m)^(?:Collecting |Requirement already satisfied|Using cached |Successfully installed |Installing collected packages"
                        r"|Defaulting to user installation)")
NOMBRE_DE_VERSION = re.compile(r"^\d+(?:\.\d+)+$")


def pip_sin_comillas(linea: str) -> str | None:
    """El requisito con < o > sin comillas dobles de una orden pip install (cmd lo toma por una redirección); None si no hay."""
    for orden in ordenes_bat(linea)[::2]:
        if PIP_INSTALA.search(orden):
            for token in re.findall(r'"[^"]*"|\S+', orden):
                if not token.startswith('"') and PIP_ESPEC.match(token.lstrip("'")):  # con comillas simples cmd tampoco protege el > ni el <
                    return token
    return None


def archivos_de_pip(raiz: Path) -> list[Path]:
    """Archivos sueltos de la raíz con nombre de versión (3.1) y contenido de salida de pip: los dejó un pip install paquete>=3.1 sin comillas."""
    hallados = []
    for f in sorted(raiz.iterdir()) if raiz.is_dir() else []:
        if f.is_file() and NOMBRE_DE_VERSION.match(f.name) and f.stat().st_size < 200_000:
            try:
                texto = f.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            if PIP_SALIDA.search(texto):
                hallados.append(f)
    return hallados


def pip_suelto(raiz: Path, inf: Informe) -> None:
    """Aviso por cada archivo suelto de la raíz que es la salida de un pip install paquete>=3.1 sin comillas (máximo 3)."""
    for f in archivos_de_pip(raiz)[:3]:
        inf.avisos.append(f"{f.name}: archivo suelto en la raíz del proyecto con la salida de un pip install: lo creó un comando como "
                          f"`pip install paquete>={f.name}` sin comillas (en cmd el `>` es una redirección: la salida va a un archivo y el paquete "
                          f"se instala sin esa versión). Quítalo y escribe la versión entre comillas dobles (pip install \"paquete>={f.name}\") "
                          "o en requirements.txt.")


# El resolver.bat de una misión decía «regenera el .opb, lo resuelve y muestra el resultado», pero solo mostraba el .modelo
# que Aura había calculado con resolver_sat: con otros datos seguía enseñando la solución vieja.
PROMETE_RESOLVER = re.compile(r"(?i)\b(?:resuelve|resolver|resolviendo|resuelto|calcula|calculando|optimiza|optimizando)\b")
SOLVER_LIBRERIA = re.compile(r"(?i)\b(?:scipy|pysat|python-sat|z3|ortools|pulp|mip|cvxpy|highspy|pyomo|gurobipy|cplex|docplex)\b"
                             r"|sat-hibrido|motor\.exe")
ESCRIBE_ARCHIVO = re.compile(r"""open\([^)]*['"]w|write_text\(""")


def bat_sin_calculo(raiz: Path, lista: list[Path], inf: Informe) -> None:
    """Error si un .bat promete resolver, hay una solución guardada por resolver_sat (*.modelo) y nada la calcula."""
    modelos = sorted(raiz.glob("*.modelo"))
    if not modelos or not lista:
        return
    for p in archivos(raiz, ".py"):
        if re.match(r"(?i)(test_|tests?\.py|conftest)", p.name):
            continue
        texto = p.read_text(encoding="utf-8", errors="replace")
        if SOLVER_LIBRERIA.search(texto) or (".modelo" in texto and ESCRIBE_ARCHIVO.search(texto)):
            return
    for b in lista:
        promesa = PROMETE_RESOLVER.search(b.stem) or PROMETE_RESOLVER.search(b.read_text(encoding="utf-8", errors="replace"))
        if promesa:
            inf.errores.append(
                f"{relativa(b, raiz)} dice que {promesa[0].lower()}, pero nada del proyecto calcula la solución: solo muestra "
                f"{modelos[0].name}, que calculó Aura con resolver_sat. Si el usuario cambia los datos, el .bat le seguirá "
                "enseñando la solución vieja. O haces que el proyecto calcule (python-sat, z3 o scipy.optimize.milp, con su "
                "requirements.txt), o cambias el .bat y tu resumen a «muestra la solución calculada».")
            return


# Un run.bat de Rust estaba en la raíz y el proyecto en palabras\ (cargo new palabras): con doble clic, «could not find Cargo.toml»
# (m4185 rust_palabras). El agente probó el .exe a mano y verificar(ruta="palabras") no miraba el .bat de la raíz. Cada orden
# de construcción lee su manifiesto de la carpeta donde corre: aquí se sigue el cd del .bat y se comprueba que está.
MANIFIESTOS = (
    (re.compile(r"(?i)^cargo(?:\.exe)?\s+(?:\+\S+\s+)?(?:run|r|test|t|build|b|check|clippy|fmt|bench|doc)\b"), ("Cargo.toml",),
     re.compile(r"--manifest-path")),
    (re.compile(r"(?i)^go(?:\.exe)?\s+(?:run|test|build|vet|mod\s+(?:tidy|download|vendor))\b"), ("go.mod",),
     re.compile(r"\S\.go\b|(?:^|\s)-C\s")),
    (re.compile(r"(?i)^(?:npm|pnpm|yarn)(?:\.cmd)?\s+(?:run(?:-script)?|start|test|install|i|ci|build|dev)\b"), ("package.json",),
     re.compile(r"--prefix|--cwd|--workspace|(?:^|\s)-[Cw]\s")),
    (re.compile(r"(?i)^dotnet(?:\.exe)?\s+(?:run|build|test|publish|restore|watch|clean)\b"),
     ("*.csproj", "*.sln", "*.slnx", "*.fsproj", "*.vbproj"), re.compile(r"--project|\.(?:cs|fs|vb)proj\b|\.slnx?\b|(?:^|\s)-p\s")),
    (re.compile(r"(?i)^(?:mvn|mvnw)(?:\.cmd)?\s+(?:-\S+\s+)*(?:clean|compile|package|install|test|verify|spring-boot:\S+|exec:\S+)\b"),
     ("pom.xml",), re.compile(r"(?:^|\s)(?:-f|--file)\s")),
    (re.compile(r"(?i)^(?:gradle|gradlew)(?:\.bat)?\s+(?:-\S+\s+)*(?:run|build|test|assemble\w*|install\w*|bootRun|clean|check|jar)\b"),
     ("build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts"),
     re.compile(r"(?:^|\s)(?:-p|--project-dir|-c|--settings-file)\s")),
    (re.compile(r"(?i)^flutter(?:\.bat)?\s+(?:run|build|test|pub|analyze)\b"), ("pubspec.yaml",), re.compile(r"$^")),
    (re.compile(r"(?i)^docker(?:\s+|-)compose\s+(?:-\S+\s+)*(?:up|down|build|run|start|stop|logs|ps|exec|pull)\b"),
     ("compose.yaml", "compose.yml", "docker-compose.yml", "docker-compose.yaml"), re.compile(r"(?:^|\s)(?:-f|--file)\s")),
)
CREA_MANIFIESTO = re.compile(r"(?i)\b(?:cargo\s+(?:init|new)|npm\s+init|go\s+mod\s+init|dotnet\s+new|flutter\s+create|"
                             r"pnpm\s+(?:init|create)|yarn\s+init)\b")
CD_BAT = re.compile(r"(?i)^(cd|chdir|pushd)(?:\s+/d)?(?:\s+(.*))?$")
SIN_MIRAR = re.compile(r"(?i)^(?:rem\b|::|:|@?echo\b|set\b|title\b|exit\b|goto\b|chcp\b|setlocal\b)")
NO_BUSCAR = {".git", ".venv", "venv", "node_modules", "target", ".agente", "__pycache__", "dist", "build"}


def _destino_cd(destino: str, cwd: Path | None, carpeta: Path) -> Path | None:
    """La carpeta a la que lleva el cd, o None si no se puede saber (variable) o no existe todavía."""
    t = destino.strip().strip('"').strip().replace("%~dp0", str(carpeta) + os.sep)
    if "%" in t or "!" in t or not t:
        return None
    p = Path(t)
    if not p.is_absolute():
        if cwd is None:
            return None
        p = cwd / p
    p = Path(os.path.normpath(p))
    return p if p.is_dir() else None


def _manifiesto_cerca(cwd: Path, nombres: tuple[str, ...]) -> list[str]:
    """Subcarpetas (hasta 2 niveles) de cwd donde está el manifiesto."""
    hallados = []
    for n in nombres:
        for patron in (f"*/{n}", f"*/*/{n}"):
            for f in sorted(cwd.glob(patron)):
                rel = f.parent.relative_to(cwd)
                if not (set(rel.parts) & NO_BUSCAR) and rel.as_posix() not in hallados:
                    hallados.append(rel.as_posix())
    return hallados[:3]


def manifiestos_bat(carpeta: Path, lineas: list[str]) -> list[tuple[int, str, str, list[str]]]:
    """(línea, orden, manifiesto que falta, subcarpetas donde sí está) de las órdenes del .bat (que vive en carpeta) cuyo
    manifiesto no está en la carpeta donde corren, siguiendo cd, pushd y popd."""
    if CREA_MANIFIESTO.search("\n".join(lineas)):
        return []
    cwd: Path | None = carpeta
    pila: list[Path | None] = []
    fallos = []
    for i, linea in enumerate(lineas):
        l = linea.strip()
        if not l or SIN_MIRAR.match(l):
            continue
        if re.match(r"(?i)(?:if|for)\b", l):
            if re.search(r"(?i)\b(?:cd|chdir|pushd|popd)\b", l):
                cwd = None  # un cd dentro de un if/for: a partir de ahí no se sabe dónde corre
            continue
        for o in ordenes_bat(l)[::2]:
            o = re.sub(f"(?i)^(?:call\\s+)?{LANZADOR}", "", o.strip().lstrip("@(").strip()).strip()
            if (m := CD_BAT.match(o)):
                if m[1].lower() == "pushd":
                    pila.append(cwd)
                if m[2]:
                    cwd = _destino_cd(m[2], cwd, carpeta)
                continue
            if re.match(r"(?i)popd\b", o):
                cwd = pila.pop() if pila else cwd
                continue
            if cwd is None:
                continue
            for patron, nombres, aparte in MANIFIESTOS:
                if patron.match(o) and not aparte.search(o):
                    if not any(next(cwd.glob(n), None) for n in nombres):
                        fallos.append((i + 1, o[:50], nombres[0], _manifiesto_cerca(cwd, nombres)))
                    break
    return fallos


def mensaje_manifiesto(orden: str, manifiesto: str, cerca: list[str]) -> str:
    base = (f"'{orden}' necesita {manifiesto} en la carpeta donde corre y no está: con doble clic fallará "
            f"(«could not find {manifiesto}» o similar). ")
    if cerca:
        sub = cerca[0].replace("/", "\\")
        return base + f"Está en {', '.join(cerca)}: pon antes de esa orden 'cd /d \"%~dp0{sub}\"' o crea el .bat dentro de esa carpeta."
    return base + "Créalo, o corrige el cd que hay antes."


def manifiesto_bat(raiz: Path, b: Path, inf: Informe, lineas: list[str] | None = None) -> None:
    if lineas is None:
        lineas = b.read_text(encoding="utf-8", errors="replace").splitlines()
    for n, orden, manifiesto, cerca in manifiestos_bat(b.parent, lineas):
        inf.errores.append(f"{relativa(b, raiz)}:{n} " + mensaje_manifiesto(orden, manifiesto, cerca))


def bats(raiz: Path, lista: list[Path], inf: Informe) -> None:
    escucha = puertos_escucha(raiz) if lista else set()
    for b in lista:
        texto = b.read_text(encoding="utf-8", errors="replace")
        rel = relativa(b, raiz)
        if not re.search(r'cd\s+/d\s+"?%~dp0', texto, re.IGNORECASE) and re.search(r"\b(python|py|node|java|javac)\b", texto, re.IGNORECASE):
            inf.avisos.append(f"{rel}: no hace cd /d \"%~dp0\": fallará si se abre desde otra carpeta.")
        if (m := re.search(r"&(?:gt|lt|amp|quot);", texto)):
            inf.errores.append(f"{rel}: tiene '{m[0]}', que es código de HTML, y cmd lo ejecuta tal cual. Escribe > < & \" sin más.")
        lineas = [l.strip() for l in texto.splitlines()]
        manifiesto_bat(raiz, b, inf, lineas)
        for i, l in enumerate(lineas):
            if not l or re.match(r"(?i)(echo|rem|::|:|set|title|start|if|for|goto|exit)\b|@?echo\b", l):
                continue
            sigue = next((x for x in lineas[i + 1:] if x and not re.match(r"(?i)(rem\b|::)", x)), "")
            sin_call = [o.strip() for o in ordenes_bat(l)[::2] if o.strip() and not re.match(r"(?i)call\s", o.strip())
                        and es_lote(o, b.parent)]
            if sin_call and sigue:
                inf.errores.append(f"{rel}:{i + 1} '{sin_call[0][:50]}' sin call: es un .cmd y el .bat termina ahí, sin "
                                   f"ejecutar lo que sigue ('{sigue[:40]}'). Escribe 'call {sin_call[0][:50]}'.")
        for i, l in enumerate(lineas):
            if re.search(r"(http\.server|flask\s+run|uvicorn|npm\s+(run\s+)?(start|dev)|npx\s+serve|live-server)", l, re.IGNORECASE) \
                    and not re.match(r"(?i)(echo|rem|start)\b", l):
                despues = [x for x in lineas[i + 1:] if x and not re.match(r"(?i)(echo|rem|::|if |goto|exit|pause|:|\))", x)]
                if despues:
                    inf.errores.append(f"{rel}: '{l[:60]}' bloquea (el servidor no termina) y las líneas siguientes "
                                       f"('{despues[0][:40]}') nunca se ejecutan. Pon 'start \"\" http://...' ANTES del servidor.")
        # Un run.bat abría http://localhost:5000 y la app escuchaba en 5055: con doble clic, «no se puede acceder». La
        # revisión lo vio como MEDIO en 1 de 6 revisiones. Con Docker el puerto de fuera lo decide el mapeo: no se mira.
        if escucha and not re.search(r"\bdocker\b", texto, re.IGNORECASE):
            for i, l in enumerate(lineas):
                if re.match(r"(?i)(rem\b|::)", l):
                    continue
                for p in sorted({int(x) for x in URL_LOCAL.findall(l)} - escucha):
                    inf.errores.append(f"{rel}:{i + 1} abre http://localhost:{p}, pero el programa escucha en "
                                       f"{', '.join(map(str, sorted(escucha)))}: con doble clic el usuario no verá el "
                                       "sistema. Pon el mismo puerto en los dos sitios.")
        # Con doble clic el .bat recibe el PATH del Explorador, que puede no traer Python: crear_bat lo busca, pero una
        # misión cambió su run.bat con editar por «python main.py» y al usuario le salió «"python" no se reconoce».
        if not re.search(r"reg query HKCU\\Environment|%PY%|\.venv\\Scripts\\", texto, re.IGNORECASE):
            for i, l in enumerate(lineas):
                if re.match(r"(?i)(rem\b|::|@?echo\b)", l):
                    continue
                ordenes = [re.sub(f"(?i)^(?:call\\s+)?{LANZADOR}", "", o.strip().lstrip("@(").strip()) for o in ordenes_bat(l)[::2]]
                if (m := next(filter(None, map(PYTHON_SUELTO.match, ordenes)), None)):
                    inf.errores.append(f"{rel}:{i + 1} lanza '{m[1]}' sin buscarlo: con doble clic Windows puede no "
                                       f"tener Python en el PATH y el usuario verá «\"{m[1]}\" no se reconoce», aunque en "
                                       "tus pruebas funcione. Rehaz el .bat con crear_bat, que busca Python y recupera el PATH.")
                    break
        for i, l in enumerate(lineas):
            if re.match(r"(?i)(rem\b|::|@?echo\b)", l):
                continue
            if (t := herramienta_de_aura_como_orden(l, b.parent)):
                inf.errores.append(f"{rel}:{i + 1} lanza '{t}', que es una herramienta de Aura (como escribir o ejecutar) y no un "
                                   f"comando de Windows: con doble clic el usuario verá «\"{t}\" no se reconoce». Llámala tú con "
                                   "la herramienta y haz que el .bat lance un script del proyecto (por ejemplo python solver.py).")
                break
        patron = r'(?:\b(?:python|py(?: -3\.\d+)?|node|java -jar)|%PY%)\s+(?:"([^"]+\.(?:py|js|mjs|jar))"|(?!-)([^\s"&|<>]+\.(?:py|js|mjs|jar))\b)'
        for m in re.finditer(patron, texto, re.IGNORECASE):
            destino = (m.group(1) or m.group(2)).strip()
            if "%" not in destino and not (b.parent / destino).exists():
                inf.errores.append(f"{rel} ejecuta '{destino}' y ese archivo NO existe junto al .bat.")
        for i, l in enumerate(lineas):
            if not re.match(r"(?i)\s*(?:rem\b|::|@?echo\b)", l) and (t := pip_sin_comillas(l)):
                inf.avisos.append(f"{rel}:{i + 1} tiene `{t}` sin comillas: en un .bat el `>` (o el `<`) es una redirección, así que cada ejecución "
                                  "deja un archivo con la salida de pip y el paquete se instala sin esa versión. Ponlo entre comillas dobles "
                                  f"(pip install \"{t}\") o déjalo en requirements.txt.")
                break
        borra_datos_bat(b, lineas, raiz, inf)


# ---------------------------------------------------------------- resumen
# Una misión terminó diciendo «usuario admin, contraseña admin» y el programa creaba admin con admin123 (así lo decían
# el código y el README): la dueña no pudo entrar. La contraseña que da el resumen tiene que estar en el proyecto.
# «password,» o «password)» es un nombre de columna (INSERT INTO usuarios (nombre, username, password, rol) VALUES
# ('Administrador', ...)): el texto que viene después no es su contraseña (m4134_neg1 probaba admin/Administrador).
CLAVE_RESUMEN = re.compile(r"(?i)\b(?:contrase[ñn]a|clave|password|pass|pwd)\b(?!\s*[,)])[^`'\"\n]{0,15}?"
                           r"(?:`([^`\s]{1,40})`|\*\*([^*\s]{1,40})\*\*|\"([^\"\s]{1,40})\"|'([^'\s]{1,40})')")
# La 4.12.5 rechazó «empleada123», que estaba en hashlib.sha256("empleada123".encode()): también vale lo que se pasa a
# una función de hash (sha256(, md5(, hashpw(b"...").
CLAVE_CODIGO = re.compile(r"""(?i)(?:hash\w*|passw\w*|pass|pwd|contrase\w*|clave\w*|sha\d{0,3}|md5|blake2\w?|\w*crypt\w*|"""
                          r"""pbkdf2\w*)['"]?\s*(?:\(|[:=,])\s*b?['"]([^'"\s]{1,40})['"]""")
NO_ES_CLAVE = re.compile(r"^\.?[\w-]+\.\w{1,8}$|[/\\()=<>{}]|^[A-Z][A-Z0-9_]+$")  # archivo, ruta, llamada, variable
# crear_usuario("Administrador", "admin", "admin123", "admin"): la contraseña va sin nombre; se toma la que lleva números.
CLAVE_LLAMADA = re.compile(r"""(?i)\b\w*(?:usuario|user|empleado|cuenta|account)\w*\s*\(([^()\n]*)\)""")
CON_NUMERO = re.compile(r"""['"]([A-Za-z][^'"\s]{2,38}\d[^'"\s]{0,10}|\d[^'"\s]{3,39})['"]""")
# En un INSERT con lista de columnas cada valor va con su columna: (username, password) -> ('admin', 'admin123').
INSERT_SQL = re.compile(r"(?is)\bINSERT\s+(?:OR\s+\w+\s+)?INTO\s+[\w.`\"\[\]]+\s*\(([^()]*)\)\s*VALUES\s*\(([^()]*)\)")
COL_USUARIO = re.compile(r"(?i)^[`\"\[]?(?:user_?name|usuario|user|login|email|correo)[`\"\]]?$")
COL_CLAVE = re.compile(r"(?i)^[`\"\[]?(?:password|passwd|pass|pwd|contrase[ñn]a|clave)[`\"\]]?$")
ES_HASH = re.compile(r"(?i)^(?:\$2[aby]?\$|pbkdf2|scrypt|argon2|sha\d*\$)|^[0-9a-f]{32,}$")
# «Usuario por defecto: admin / admin123», «Credenciales: admin / 1234», «**Usuario administrador:** `admin` / `admin123`».
PAREJA_BARRA = re.compile(r"(?i)\b(?:usuario|user|login|credenciales|acceso)\b[^:\n]{0,25}:\s*\**\s*[`'\"]?"
                          r"([A-Za-z][\w.@-]{1,38})[`'\"]?\s*/\s*[`'\"]?([^\s`'\"*/<>()\[\]{},;|:]{3,39})(?![\w.]*\s*:)")


def _valores_sql(texto: str) -> list[str]:
    """'a', 'b,c', ? -> ["'a'", "'b,c'", "?"]: separa por comas que no van entre comillas."""
    valores, actual, comilla = [], "", ""
    for ch in texto:
        if comilla:
            comilla = "" if ch == comilla else comilla
        elif ch in "'\"":
            comilla = ch
        elif ch == ",":
            valores.append(actual.strip())
            actual = ""
            continue
        actual += ch
    return [*valores, actual.strip()]


def parejas_con_etiqueta_fija(texto: str) -> list[tuple[str, str]]:
    """Parejas que el texto da con su etiqueta sin dudas: columnas de un INSERT y «Usuario …: X / Y»."""
    parejas = []
    for m in INSERT_SQL.finditer(texto):
        columnas, valores = [c.strip() for c in m[1].split(",")], _valores_sql(m[2])
        iu = next((i for i, c in enumerate(columnas) if COL_USUARIO.match(c)), None)
        ic = next((i for i, c in enumerate(columnas) if COL_CLAVE.match(c)), None)
        if iu is None or ic is None or len(columnas) != len(valores):
            continue
        u, c = valores[iu], valores[ic]
        if all(len(v) > 2 and v[0] == v[-1] and v[0] in "'\"" for v in (u, c)):
            parejas.append((u[1:-1], c[1:-1]))
    parejas += PAREJA_BARRA.findall(texto)
    return [(u, c) for u, c in parejas if u != c and len(c) >= 3 and not ES_HASH.search(c)
            and _valor_etiquetado(u) and _valor_etiquetado(c)]


def _archivos_texto(raiz: Path, con_env: bool = False):
    """(archivo, texto) de los archivos de texto del proyecto. Un `.env` no tiene extensión y `archivos` no lo lista: con_env
    los añade (el recorrido sigue leyendo lo de siempre; solo claves_inventadas los mira)."""
    lista = archivos(raiz, ".py", ".js", ".ts", ".mjs", ".cjs", ".php", ".rb", ".go", ".java", ".cs", ".rs", ".sql", ".json",
                     ".toml", ".ini", ".cfg", ".yml", ".yaml", ".env", ".md", ".txt", ".html", ".bat")
    if con_env:
        lista += [p for p in sorted(raiz.rglob(".env*")) if p.is_file() and not ignorado(p, raiz) and p not in lista]
    for p in lista:
        if p.name.startswith(".") and not p.name.startswith(".env"):  # .fenix_chat.json guarda el propio resumen
            continue
        try:
            yield p, p.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue


def _textos_proyecto(raiz: Path):
    for _, texto in _archivos_texto(raiz):
        yield texto


# La ferretería de una misión daba «empleado / empleado123» en el resumen: solo estaba en test_app.py, donde el test crea ese
# usuario; el programa solo creaba admin. Una contraseña que solo existe en los tests no sirve para entrar.
ES_DE_TEST = re.compile(r"(?i)(?:^|[\\/])(?:tests?|__tests__)[\\/]|(?:^|[\\/])(?:test_[^\\/]*|[^\\/]*_test\.[^\\/]+|"
                        r"[^\\/]*\.(?:test|spec)\.[^\\/]+|conftest\.py|tests?\.py)$")


def claves_del_proyecto(raiz: Path, sin_tests: bool = False, con_env: bool = False) -> list[str]:
    """Contraseñas que da el proyecto (código, README...), en el orden en que aparecen; sin_tests las deja fuera de los tests."""
    conocidas: dict[str, None] = {}
    for p, texto in _archivos_texto(raiz, con_env):
        if sin_tests and ES_DE_TEST.search(relativa(p, raiz)):
            continue
        conocidas.update(dict.fromkeys(c for _, c in parejas_con_etiqueta_fija(texto)))
        conocidas.update(dict.fromkeys(CLAVE_CODIGO.findall(texto)))
        conocidas.update(dict.fromkeys(next(g for g in m.groups() if g) for m in CLAVE_RESUMEN.finditer(texto)))
        conocidas.update(dict.fromkeys(c for m in CLAVE_LLAMADA.finditer(texto) for c in CON_NUMERO.findall(m[1])))
    return [c for c in conocidas if not NO_ES_CLAVE.search(c)]  # text="Contraseña:").pack(anchor= no es una


LITERAL = re.compile(r"""[`'"]([A-Za-z][\w.@-]{2,30})[`'"]""")
# «usuario: admin / contraseña: admin123», «Usuario=`admin`», «login es admin»: el valor va tras la etiqueta, con o sin
# comillas. Un comentario así daba admin/admin123 y, sin comillas, no se emparejaba: la pareja caía fuera de las primeras
# que se prueban y «nadie puede entrar» saltaba en falso.
# Tras «:» (también «:** `x`» de un README) o tras «=» con comillas: «password = request.form.get(...)» es código.
_TRAS_ETIQUETA = r"\s*(?::\s*\**\s*[`'\"]?|=\s*[`'\"]|\s+es\s+[`'\"]?)"
USUARIO_ETIQ = re.compile(r"(?i)\b(?:usuario|username|user|login)\b" + _TRAS_ETIQUETA + r"([A-Za-z][\w.@-]{1,38})(?!\()")
CLAVE_ETIQ = re.compile(r"(?i)\b(?:contrase[ñn]a|clave|password|passwd|pwd)\b" + _TRAS_ETIQUETA +
                        r"([^\s`'\"<>*()\[\]{},;|]{3,39})(?!\()")
NO_ES_VALOR = {"str", "int", "bool", "float", "bytes", "dict", "list", "any", "none", "null", "true", "false", "string",
               "text", "optional", "self", "required"}  # def entrar(usuario: str, password: str)


def _valor_etiquetado(v: str) -> bool:
    """Un usuario o una contraseña de verdad, no un tipo, un campo ni un atributo (request.form)."""
    return (not NO_ES_CLAVE.search(v) and v.lower() not in CAMPOS and v.lower() not in NO_ES_VALOR
            and not (re.search(r"[A-Za-z_]\.[A-Za-z_]", v) and "@" not in v))


def pares_del_proyecto(raiz: Path, claves: list[str], usuarios: list[str]) -> list[tuple[str, str]]:
    """(usuario, contraseña) que el proyecto da juntos: en la misma línea o en la de al lado. Primero las parejas con
    etiqueta («usuario: X ... contraseña: Y»), que son las más fiables."""
    etiquetados: dict[tuple[str, str], None] = {}
    pares: dict[tuple[str, str], None] = {}
    candidatos = sorted(usuarios, key=lambda u: u.lower() in CAMPOS)  # {"usuario": "duena", ...}: primero «duena»
    for texto in _textos_proyecto(raiz):
        etiquetados.update(dict.fromkeys(parejas_con_etiqueta_fija(texto)))
        lineas = texto.splitlines()
        for i, linea in enumerate(lineas):
            zona2 = " ".join(lineas[max(0, i - 1):i + 2])
            for zona in (linea, zona2):  # etiquetados: usuario y contraseña con su etiqueta, en la zona
                us = [u.rstrip(".") for u in USUARIO_ETIQ.findall(zona)]
                cs = [c.rstrip(".") for c in CLAVE_ETIQ.findall(zona)]
                us = [u for u in us if len(u) >= 2 and _valor_etiquetado(u)]
                cs = [c for c in cs if len(c) >= 3 and _valor_etiquetado(c)]
                etiquetados.update(dict.fromkeys((u, c) for u in us for c in cs if u != c))
            for c in [c for c in claves if re.search(rf"[`'\"]{re.escape(c)}[`'\"]", linea)]:
                for zona in (linea, zona2):  # primero los de la misma línea
                    pares.update(dict.fromkeys((u, c) for u in candidatos
                                               if u != c and re.search(rf"[`'\"]{re.escape(u)}[`'\"]", zona)))
    # Las parejas sueltas con contraseña con número («duena»/«duena2026») son fiables; las de contraseña sin número son
    # coincidencias del código (('Administrador','admin'), ('rol','admin') de los INSERT). admin/admin123 es lo que dan
    # casi todas las webs de negocio: detrás de las etiquetadas y de las sueltas fiables (adivinada, no debe tapar a
    # «duena»/«duena2026» ni salir antes que ellas en los mensajes), pero DELANTE de las sueltas sin número: esas, sin
    # README, empujaban (admin,admin123) más allá de las 12 que se prueban -> «nadie puede entrar» en falso (m4133 neg1/neg3).
    fuertes = [p for p in pares if re.search(r"\d", p[1])]
    debiles = [p for p in pares if not re.search(r"\d", p[1])]
    por_defecto = ([("admin", c) for c in claves if re.search(r"\d", c) and c != "admin"][:2]
                   if "admin" in usuarios else [])
    return list(dict.fromkeys([*etiquetados, *fuertes, *por_defecto, *debiles]))


CAMPOS = {"usuario", "usuarios", "user", "users", "username", "nombre", "password", "contraseña", "clave", "rol", "role",
          "email", "correo"}


def usuarios_del_proyecto(raiz: Path, claves: list[str]) -> list[str]:
    """Posibles usuarios: los textos entre comillas cerca (6 líneas) de donde el proyecto da una contraseña."""
    cerca: dict[str, int] = {}
    for texto in _textos_proyecto(raiz):
        lineas = texto.splitlines()
        for i, linea in enumerate(lineas):
            if not any(re.search(rf"[`'\"]{re.escape(c)}[`'\"]", linea) for c in claves):
                continue
            for j in range(max(0, i - 6), min(len(lineas), i + 7)):
                for u in LITERAL.findall(lineas[j]):
                    if u not in claves and not NO_ES_CLAVE.search(u) and u.lower() not in ("utf-8", "utf8", "ascii"):
                        cerca[u] = min(cerca.get(u, 99), abs(i - j))
    usuarios = sorted(cerca, key=cerca.get)[:12]
    # «duena123» suele ser la contraseña de «duena»: también se prueba ese usuario, detrás de los de cerca.
    usuarios += [b for b in dict.fromkeys(re.sub(r"\d+$", "", c) for c in claves if re.search(r"\d$", c))
                 if len(b) >= 3 and b not in usuarios]
    usuarios = usuarios[:12]
    return usuarios if "admin" in usuarios else [*usuarios[:11], "admin"]


def claves_inventadas(raiz: Path, resumen: str) -> str:
    """Contraseñas que el resumen da al usuario y no aparecen en el proyecto; '' si están o si no se sabe cuáles son."""
    dichas = {next(g for g in m.groups() if g) for m in CLAVE_RESUMEN.finditer(resumen)}
    dichas = {d for d in dichas if not NO_ES_CLAVE.search(d)}
    if not dichas:
        return ""
    conocidas = set(claves_del_proyecto(raiz, con_env=True))
    if not conocidas:
        return ""
    mensajes = []
    if (malas := sorted(dichas - conocidas)):
        mensajes.append(f"el resumen da la contraseña {', '.join(f'«{c}»' for c in malas)}, pero no aparece en el proyecto (allí "
                        f"están: {', '.join(f'«{c}»' for c in sorted(conocidas)[:6])}). Con la del resumen el usuario no podrá entrar. "
                        "Busca en el código cuál crea el programa y corrige el resumen.")
    if (solo_tests := sorted((dichas & conocidas) - set(claves_del_proyecto(raiz, sin_tests=True, con_env=True)))):
        donde = sorted({relativa(p, raiz) for p, texto in _archivos_texto(raiz) if ES_DE_TEST.search(relativa(p, raiz))
                        and any(c in texto for c in solo_tests)})
        mensajes.append(f"el resumen da la contraseña {', '.join(f'«{c}»' for c in solo_tests)}, pero solo aparece en los tests "
                        f"({', '.join(donde[:3])}): el programa no la crea, así que con ella el usuario no podrá entrar. Haz que "
                        "el programa cree ese usuario con esa contraseña, o quita la credencial del resumen y explica cómo crearlo "
                        "(por ejemplo, desde la pantalla de usuarios).")
    return " Además, ".join(mensajes) if len(mensajes) > 1 else "".join(mensajes)


# ---------------------------------------------------------------- entrada


def verificar(raiz: Path, python: str, ruta: str = ".", segundos: int = 8, maximo: int = 6000,
              previos: set[str] | None = None, idiomas_parcial: list[str] | None = None,
              ejecutar: bool = True, ya_estaba: Callable[[str], bool] | None = None) -> tuple[str, int]:
    """Devuelve (informe, número de errores). previos = errores y avisos normalizados que ya estaban: no cuentan.
    ejecutar=False: no lanza nada del proyecto, solo lo analiza (código ajeno que la tarea no ha cambiado).
    ya_estaba: en una auditoría, dice si un error lo tenía ya el código: se enseña, pero no cuenta."""
    inf = informe_proyecto(raiz, python, ruta, segundos, idiomas_parcial, previos, ejecutar)
    if isinstance(inf, str):
        return inf, 1
    if previos:
        inf.previos = [re.sub(r"(NUNCA se usa).*", r"\1", e) for e in inf.errores + inf.avisos if normalizar(e) in previos]
        inf.errores = [e for e in inf.errores if normalizar(e) not in previos]
        inf.avisos = [a for a in inf.avisos if normalizar(a) not in previos]
    if ya_estaba:
        inf.viejos = [e for e in inf.errores if ya_estaba(e)]
        inf.errores = [e for e in inf.errores if e not in inf.viejos]
    return inf.texto(maximo), len(inf.errores)


def informe_proyecto(raiz: Path, python: str, ruta: str = ".", segundos: int = 8,
                     idiomas_parcial: list[str] | None = None, previos: set[str] | None = None,
                     ejecutar: bool = True) -> "Informe | str":
    """El informe estructurado de verificar (o un texto si la ruta no existe)."""
    base = (raiz / ruta).resolve() if ruta not in ("", ".") else raiz
    if not base.exists():
        return f"No existe: {ruta}"
    if base.is_file():  # verificar("index.html") debe probar el proyecto, no devolver "sin código"
        base = raiz
    inf = Informe()
    inf.ejecutar = ejecutar
    pys, htmls = archivos(base, ".py"), archivos(base, ".html", ".htm")
    jss, javas, bat = archivos(base, ".js", ".mjs"), archivos(base, ".java"), archivos(base, ".bat", ".cmd")
    if pys:
        inf.tipos.append("python")
        errores_antes = len(inf.errores)
        for p in pys:
            try:
                compile(p.read_text(encoding="utf-8", errors="replace"), str(p), "exec")
            except SyntaxError as e:
                inf.errores.append(f"{relativa(p, raiz)}:{e.lineno} ERROR DE SINTAXIS: {e.msg}")
        sintaxis_ok = len(inf.errores) == errores_antes
        peligro = borrados_fuera(raiz, pys)
        inf.errores += peligro
        lint_python(raiz, pys, inf)
        pruebas_borran_la_base(raiz, pys, inf)
        seguridad_web(raiz, pys, inf)
        if base == raiz:
            codigo_muerto(raiz, pys, inf, previos=previos)
        else:  # una subcarpeta: quien usa sus funciones puede estar fuera
            codigo_muerto(raiz, archivos(raiz, ".py"), inf, solo=base, previos=previos)
        if peligro:
            inf.avisos.append("No ejecuté los tests ni el programa: borrarían fuera del proyecto. Corrige antes esos borrados.")
        elif not inf.ejecutar:
            inf.omitido.append("pruebas y programa de Python")
        elif sintaxis_ok:  # con errores de sintaxis no tiene sentido ejecutar
            tests_python(raiz, pys, python, inf)
            ejecutar_python(raiz, pys, python, inf, segundos)
    if htmls or jss:
        inf.tipos.append("web")
        sueltas = [h for h in htmls if not es_plantilla(h, raiz)]
        html_estatico(raiz, sueltas, jss, inf)
        ids_dom(raiz, base, inf)
        funciones_de_botones(raiz, base, inf)
        librerias_cdn(raiz, htmls, inf)
        directas = [h for h in sueltas if not fuente_vite(h)]
        if directas and inf.ejecutar:
            navegador(raiz, directas, inf)
        elif directas:
            inf.omitido.append("la página en el navegador")
    javas = [p for p in javas if not _de_android(p, base)]  # esos los compila Gradle (android_gradle, flutter)
    if javas:
        inf.tipos.append("java")
        java(raiz, javas, inf, segundos)
    if bat:
        bats(raiz, bat, inf)
        bat_sin_calculo(raiz, bat, inf)
    if base != raiz:  # verificar("palabras") no miraba el run.bat de la raíz, que es el que abre el usuario (m4185)
        for b in sorted({*raiz.glob("*.bat"), *raiz.glob("*.cmd")} - set(bat)):
            manifiesto_bat(raiz, b, inf)
    if base == raiz:
        bat_citado_que_no_existe(raiz, inf)
        pip_suelto(raiz, inf)
    ecosistemas.verificar_todos(base, raiz, inf)
    if (vite := [h for h in htmls if fuente_vite(h)]):  # después de node(): su npm run build deja dist/
        if inf.ejecutar:
            paginas_vite(raiz, vite, inf)
        else:
            inf.omitido.append("la página en el navegador")
    # solo añade algo si el proyecto tiene archivos de idioma
    verificar_idiomas(raiz, inf, idiomas_parcial)
    if not inf.tipos and not bat:
        inf.avisos.append(f"No encontré código que verificar en '{ruta}': nada se ha comprobado.")
    return inf


def chequeo_rapido(raiz: Path, p: Path) -> str:
    """Tras escribir/editar un archivo: solo lo barato y seguro (lint del archivo, sintaxis JS, rutas de .bat)."""
    inf = Informe()
    suf = p.suffix.lower()
    if suf == ".py":
        inf.errores += borrados_fuera(raiz, [p])
        lint_python(raiz, [p], inf, rapido=True)
    elif suf in (".js", ".mjs"):
        _node_check(raiz, p, relativa(p, raiz), inf)
    elif suf in (".bat", ".cmd"):
        bats(raiz, [p], inf)
    elif suf in (".go", ".rs", ".yml", ".yaml") or p.name.lower() == "dockerfile" or ".dockerfile" in p.name.lower():
            ecosistemas.chequeo_rapido(raiz, p, inf)
    elif suf in (".html", ".htm"):
        html_estatico(raiz, [p], [], inf)
        # Recién escrito el HTML, el JS y el CSS pueden estar aún por crear: se recuerdan, no son errores.
        pendientes = [re.sub(r".* referencia '(.+)' y NO existe\.", r"PENDIENTE: crea '\1'", e) for e in inf.errores
                      if "y NO existe" in e]
        inf.errores = [e for e in inf.errores if "y NO existe" not in e]
        inf.avisos = pendientes + inf.avisos
    lineas = [f"ERROR: {e}" for e in inf.errores[:5]] + [f"AVISO: {a}" for a in inf.avisos[:4]]
    return ("\n" + "\n".join(lineas)) if lineas else ""
