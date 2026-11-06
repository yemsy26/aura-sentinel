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
import sys
import tempfile
import threading
import urllib.parse
import urllib.request
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from . import ecosistemas
from .idiomas import verificar_idiomas
from .proyecto import Informe, archivos, cola, correr_texto, normalizar, relativa

BIN = Path(sys.executable).parent  # ruff.exe y vulture.exe del venv del agente
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
def lint_python(raiz: Path, rutas: list[Path], inf: Informe, rapido: bool = False) -> None:
    ruff = BIN / "ruff.exe"
    if not ruff.exists() or not rutas:
        return
    sel = "E9,F821,F822,F823,F811,F841,F632,B006" if rapido else "E9,F,B006,B012,B018,E722,PLE"
    _, salida = correr_texto([str(ruff), "check", "--no-cache", "--output-format", "concise", "--select", sel,
                         "--ignore", "F403,F405,F541", *map(str, rutas)], raiz, 60)
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
    _, salida = correr_texto([str(vulture), *map(str, rutas), "--min-confidence", "60"], raiz, 60)
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
    return {normalizar(e) for e in inf.errores + inf.avisos}


def _modulos_proyecto(rutas: list[Path]) -> set[str]:
    return {p.stem for p in rutas if not p.stem.startswith("test")} | {p.parent.name for p in rutas}


def tests_python(raiz: Path, rutas: list[Path], python: str, inf: Informe) -> None:
    tests = [p for p in rutas if p.name.startswith("test_") or p.name.endswith("_test.py") or "tests" in p.parts]
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
    cmd = ([python, "-m", "pytest", "-q", "-x", "--no-header", "-p", "no:cacheprovider"] if codigo == 0
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
    codigo, salida = correr_texto([python, str(entrada)], entrada.parent, segundos, env)
    if codigo is None:
        inf.ok.append(f"{rel} sigue en ejecución tras {segundos} s (servidor o bucle): arrancó sin errores")
    elif "EOFError" in salida:
        inf.avisos.append(f"{rel} pide datos con input(): pruébalo con tests o con 'echo datos| python {entrada.name}'.")
    elif codigo != 0:
        inf.errores.append(f"{rel} termina con error (exit {codigo}):\n    {cola(salida)}")
    else:
        inf.ok.append(f"{rel} se ejecuta sin errores")


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


def entrada_bat(raiz: Path) -> tuple[Path, Path] | None:
    """(.bat, .py que abre) para el .bat de doble clic: python app.py, "%PY%" -m app, python . (su __main__.py)."""
    lotes = sorted((b for b in raiz.glob("*.bat") if not NO_ABRE.match(b.stem)),
                   key=lambda b: (not re.match(r"(?i)(run|abrir|iniciar|ejecutar|start|app|main)", b.stem), b.name.lower()))
    for b in lotes:
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
                for c in candidatos:
                    if "%" not in str(c) and c.is_file() and not c.name.startswith("test"):
                        return b, c
    return None


def probar_tk(raiz: Path, entrada: Path, python: str, inf: Informe, segundos: int, bat: Path | None = None) -> None:
    rel = relativa(entrada, raiz)
    tramo = min(20, max(5, segundos))
    copia = Path(tempfile.mkdtemp(prefix="agente_tk_")) / "p"
    try:
        shutil.copytree(raiz, copia, ignore=shutil.ignore_patterns(*COPIA_FUERA))
        claves = claves_del_proyecto(raiz)
        env = dict(os.environ, PYTHONUTF8="1", PYTHONIOENCODING="utf-8", SONDA_CLAVES=json.dumps(claves),
                   SONDA_USUARIOS=json.dumps(usuarios_del_proyecto(raiz, claves)))
        destino = copia / entrada.relative_to(raiz)
        codigo, salida = correr_texto([python, str(SONDA_TK), str(destino), str(tramo)], destino.parent, tramo + 25, env)
    finally:
        shutil.rmtree(copia.parent, ignore_errors=True)
    salida = salida.replace(str(copia) + os.sep, "")
    m = re.search(r"^SONDA_TK (\{.*\})$", salida, re.MULTILINE)
    if not m:
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
    d = json.loads(m.group(1))
    for e in d["errores"]:  # sin la carpeta de la copia ni los marcos de dentro de tkinter (su línea de código va detrás)
        e["traza"] = re.sub(r'\n\s*File "[^"\n]*[\\/]tkinter[\\/][^\n]*\n[^\n]*', "",
                            _traza_limpia(e["traza"]).replace(str(copia) + os.sep, ""))
    graves = [e for e in d["errores"] if e["fase"] in FASE_TK]
    if graves:
        e = graves[0]
        inf.errores.append(f"{rel}: EXCEPCIÓN al {FASE_TK[e['fase']]}: el usuario se queda ahí sin poder seguir "
                           f"(verificar usa la app sin enseñar la ventana):\n    {e['traza']}")
        return
    if d.get("sin_ventana"):
        cuando = ("al abrir el programa" if d["sin_ventana"] == "arranque" else
                  f"después de entrar (como «{d['entrar']['usuario']}»)")
        inf.errores.append(f"{rel}: {cuando} no queda ninguna ventana a la vista: la app oculta o cierra la que el "
                           "usuario tiene que ver (withdraw(), iconify() o destroy()) y no ve nada. Si la ventana "
                           "principal se construye en la misma raíz que la de entrar, no la ocultes: quita solo los "
                           "widgets del formulario de entrar.")
        return
    for e in [e for e in d["errores"] if e["fase"] == "botones"][:2]:
        inf.avisos.append(f"{rel}: EXCEPCIÓN al pulsar un botón con los campos vacíos (la sonda pulsa cada botón una vez):"
                          f"\n    {e['traza']}")
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
    if (ent := d["entrar"]) is not None:
        hecho.append(f"entró como «{ent['usuario']}»" if ent["usuario"] is not None else
                     f"pantalla de entrar: no dio con usuario y contraseña del código (probó {ent['intentos']}); lo de "
                     "después no se probó")
    hecho.append(f"{d['pestanas']} pestañas y {d['botones']} botones pulsados, {len(d['mensajes'])} mensajes")
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
    for j in jss:
        _node_check(raiz, j, relativa(j, raiz), inf)


# getElementById("x") / querySelector("#x ...") con texto literal: una plantilla (${...}), una variable o una suma
# no encajan (no se sabe qué id será) y no se miran.
BUSCA_ID = re.compile(r"""getElementById\(\s*['"`]([\w-]+)['"`]\s*\)|querySelector(?:All)?\s*(?:<[^<>()]*>)?"""
                      r"""\(\s*['"`]#([\w-]+)(?:[\s>+~.:\[,][^'"`$]*)?['"`]\s*\)""")
# Ids que crea el código: el.id = "x", setAttribute("id", "x"), id="x" en innerHTML o JSX, id={"x"}, {id: "x"}.
CREA_ID = re.compile(r"""\.id\s*=\s*['"`]([\w-]+)['"`]|setAttribute\(\s*['"`]id['"`]\s*,\s*['"`]([\w-]+)['"`]|"""
                     r"""(?<![\w.-])id\s*[=:]\s*\{?\s*\\?['"`]([\w-]+)\\?['"`]""")
SCRIPT_EN_LINEA = re.compile(r"<script\b[^>]*>(.*?)</script>", re.IGNORECASE | re.DOTALL)
FUENTES_DOM = (".js", ".mjs", ".cjs", ".ts", ".tsx", ".jsx")


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


def _node_check(raiz: Path, codigo_o_ruta, nombre: str, inf: Informe, modulo: bool = False) -> None:
    if not shutil.which("node"):
        return
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
        lineas = [l for l in salida.splitlines() if l.strip()]
        detalle = next((l for l in lineas if "Error" in l), lineas[-1] if lineas else "")
        donde = next((l.rsplit(":", 1)[-1] for l in lineas if re.search(r":\d+$", l)), "?")
        inf.errores.append(f"{nombre}:{donde} ERROR DE SINTAXIS JS: {detalle[:160]}")


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
    """¿El .java es de un proyecto Android o de la carpeta android/ de Flutter? (javac solo no tiene sus librerías)."""
    for d in (a for a in p.parents if a == base or base in a.parents):
        if (d / "pubspec.yaml").exists():
            return True
        for nombre in ("build.gradle", "build.gradle.kts"):
            if (d / nombre).is_file() and "com.android" in (d / nombre).read_text("utf-8", "replace"):
                return True
    return False


def java(raiz: Path, fuentes: list[Path], inf: Informe, segundos: int) -> None:
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
        patron = r'(?:\b(?:python|py(?: -3\.\d+)?|node|java -jar)|%PY%)\s+(?:"([^"]+\.(?:py|js|mjs|jar))"|(?!-)([^\s"&|<>]+\.(?:py|js|mjs|jar))\b)'
        for m in re.finditer(patron, texto, re.IGNORECASE):
            destino = (m.group(1) or m.group(2)).strip()
            if "%" not in destino and not (b.parent / destino).exists():
                inf.errores.append(f"{rel} ejecuta '{destino}' y ese archivo NO existe junto al .bat.")


# ---------------------------------------------------------------- resumen
# Una misión terminó diciendo «usuario admin, contraseña admin» y el programa creaba admin con admin123 (así lo decían
# el código y el README): la dueña no pudo entrar. La contraseña que da el resumen tiene que estar en el proyecto.
CLAVE_RESUMEN = re.compile(r"(?i)\b(?:contrase[ñn]a|clave|password|pass|pwd)\b[^`'\"\n]{0,15}?"
                           r"(?:`([^`\s]{1,40})`|\*\*([^*\s]{1,40})\*\*|\"([^\"\s]{1,40})\"|'([^'\s]{1,40})')")
# La 4.12.5 rechazó «empleada123», que estaba en hashlib.sha256("empleada123".encode()): también vale lo que se pasa a
# una función de hash (sha256(, md5(, hashpw(b"...").
CLAVE_CODIGO = re.compile(r"""(?i)(?:hash\w*|passw\w*|pass|pwd|contrase\w*|clave\w*|sha\d{0,3}|md5|blake2\w?|\w*crypt\w*|"""
                          r"""pbkdf2\w*)['"]?\s*(?:\(|[:=,])\s*b?['"]([^'"\s]{1,40})['"]""")
NO_ES_CLAVE = re.compile(r"^\.?[\w-]+\.\w{1,8}$|[/\\()=<>{}]|^[A-Z][A-Z0-9_]+$")  # archivo, ruta, llamada, variable


def _textos_proyecto(raiz: Path):
    for p in archivos(raiz, ".py", ".js", ".ts", ".mjs", ".cjs", ".php", ".rb", ".go", ".java", ".cs", ".rs", ".sql", ".json",
                      ".toml", ".ini", ".cfg", ".yml", ".yaml", ".env", ".md", ".txt", ".html", ".bat"):
        if p.name.startswith(".") and not p.name.startswith(".env"):  # .fenix_chat.json guarda el propio resumen
            continue
        try:
            yield p.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue


def claves_del_proyecto(raiz: Path) -> list[str]:
    """Contraseñas que da el proyecto (código, README...), en el orden en que aparecen."""
    conocidas: dict[str, None] = {}
    for texto in _textos_proyecto(raiz):
        conocidas.update(dict.fromkeys(CLAVE_CODIGO.findall(texto)))
        conocidas.update(dict.fromkeys(next(g for g in m.groups() if g) for m in CLAVE_RESUMEN.finditer(texto)))
    return [c for c in conocidas if not NO_ES_CLAVE.search(c)]  # text="Contraseña:").pack(anchor= no es una


LITERAL = re.compile(r"""[`'"]([A-Za-z][\w.@-]{2,30})[`'"]""")


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
    return usuarios if "admin" in usuarios else [*usuarios[:11], "admin"]


def claves_inventadas(raiz: Path, resumen: str) -> str:
    """Contraseñas que el resumen da al usuario y no aparecen en el proyecto; '' si están o si no se sabe cuáles son."""
    dichas = {next(g for g in m.groups() if g) for m in CLAVE_RESUMEN.finditer(resumen)}
    dichas = {d for d in dichas if not NO_ES_CLAVE.search(d)}
    if not dichas:
        return ""
    conocidas = set(claves_del_proyecto(raiz))
    malas = sorted(dichas - conocidas)
    if not conocidas or not malas:
        return ""
    return (f"el resumen da la contraseña {', '.join(f'«{c}»' for c in malas)}, pero no aparece en el proyecto (allí "
            f"están: {', '.join(f'«{c}»' for c in sorted(conocidas)[:6])}). Con la del resumen el usuario no podrá entrar. "
            "Busca en el código cuál crea el programa y corrige el resumen.")


# ---------------------------------------------------------------- entrada


def verificar(raiz: Path, python: str, ruta: str = ".", segundos: int = 8, maximo: int = 6000,
              previos: set[str] | None = None, idiomas_parcial: list[str] | None = None) -> tuple[str, int]:
    """Devuelve (informe, número de errores). previos = errores y avisos normalizados que ya estaban: no cuentan."""
    inf = informe_proyecto(raiz, python, ruta, segundos, idiomas_parcial, previos)
    if isinstance(inf, str):
        return inf, 1
    if previos:
        inf.previos = [re.sub(r"(NUNCA se usa).*", r"\1", e) for e in inf.errores + inf.avisos if normalizar(e) in previos]
        inf.errores = [e for e in inf.errores if normalizar(e) not in previos]
        inf.avisos = [a for a in inf.avisos if normalizar(a) not in previos]
    return inf.texto(maximo), len(inf.errores)


def informe_proyecto(raiz: Path, python: str, ruta: str = ".", segundos: int = 8,
                     idiomas_parcial: list[str] | None = None, previos: set[str] | None = None) -> "Informe | str":
    """El informe estructurado de verificar (o un texto si la ruta no existe)."""
    base = (raiz / ruta).resolve() if ruta not in ("", ".") else raiz
    if not base.exists():
        return f"No existe: {ruta}"
    if base.is_file():  # verificar("index.html") debe probar el proyecto, no devolver "sin código"
        base = raiz
    inf = Informe()
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
        lint_python(raiz, pys, inf)
        if base == raiz:
            codigo_muerto(raiz, pys, inf, previos=previos)
        else:  # una subcarpeta: quien usa sus funciones puede estar fuera
            codigo_muerto(raiz, archivos(raiz, ".py"), inf, solo=base, previos=previos)
        if sintaxis_ok:  # con errores de sintaxis no tiene sentido ejecutar
            tests_python(raiz, pys, python, inf)
            ejecutar_python(raiz, pys, python, inf, segundos)
    if htmls or jss:
        inf.tipos.append("web")
        sueltas = [h for h in htmls if not es_plantilla(h, raiz)]
        html_estatico(raiz, sueltas, jss, inf)
        ids_dom(raiz, base, inf)
        librerias_cdn(raiz, htmls, inf)
        directas = [h for h in sueltas if not fuente_vite(h)]
        if directas:
            navegador(raiz, directas, inf)
    javas = [p for p in javas if not _de_android(p, base)]  # esos los compila Gradle (android_gradle, flutter)
    if javas:
        inf.tipos.append("java")
        java(raiz, javas, inf, segundos)
    if bat:
        bats(raiz, bat, inf)
    ecosistemas.verificar_todos(base, raiz, inf)
    if (vite := [h for h in htmls if fuente_vite(h)]):  # después de node(): su npm run build deja dist/
        paginas_vite(raiz, vite, inf)
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
