"""verificar(): control de calidad automático por lenguaje, sin depender del modelo.

Python: sintaxis, ruff (errores reales), vulture (código que nunca se usa = funciones sin conectar), tests,
        ejecución del programa y, si es pygame, una partida sin ventana con teclas aleatorias.
HTML/JS: referencias rotas, ids que busca el JS/TS y no existen, sintaxis JS (node --check) y prueba real en Edge
         sin ventana (errores de consola, 404, canvas vacío, animación, respuesta a teclado/ratón, capturas).
Java:   javac -Xlint y ejecución del main en modo headless (o mvn test si hay pom.xml).
.bat:   archivos llamados que no existen y falta de cd /d "%~dp0".
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
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

BIN = Path(sys.executable).parent  # ruff.exe y vulture.exe del venv del agente
SONDA_PYGAME = Path(__file__).resolve().parent / "sondas" / "pygame_sonda.py"
IGNORAR = {".git", ".venv", "venv", "env", "node_modules", "__pycache__", "dist", "build", ".next", ".agente",
           "target", "out", "obj", ".idea", ".vscode", ".pytest_cache", ".mypy_cache", ".ruff_cache", "site-packages"}
# Lo que el agente trata como código (revisión, mapa, puntos de restauración, protección contra reescrituras).
CODIGO = {".py", ".js", ".mjs", ".cjs", ".ts", ".tsx", ".jsx", ".vue", ".svelte", ".html", ".htm", ".css", ".scss",
          ".java", ".kt", ".kts", ".gradle", ".rs", ".go", ".cs", ".fs", ".csproj", ".fsproj", ".sln", ".c", ".h",
          ".cpp", ".hpp", ".bat", ".cmd", ".ps1", ".sh", ".sql", ".proto", ".json", ".yml", ".yaml", ".toml", ".xml",
          ".sol", ".dart"}
NOMBRES_CODIGO = {"dockerfile", "makefile", "go.mod", ".dockerignore", ".gitignore", ".env.example", "procfile"}


def es_codigo(p: Path) -> bool:
    nombre = p.name.lower()
    return (p.suffix.lower() in CODIGO or nombre in NOMBRES_CODIGO or nombre.startswith("dockerfile.")
            or nombre.endswith(".dockerfile"))


SIN_VENTANA = 0x08000000  # CREATE_NO_WINDOW
# Atributos que leen las librerías (pygame Rect/Sprite, tkinter, unittest...), no el propio código: no son "código muerto".
ATRIBUTOS_DE_LIBRERIA = {
    "args", "kwargs", "image", "rect", "mask", "radius", "layer", "dirty", "visible", "blendmode", "source_rect",
    "x", "y", "top", "left", "bottom", "right", "topleft", "topright", "bottomleft", "bottomright", "midtop",
    "midleft", "midbottom", "midright", "center", "centerx", "centery", "size", "width", "height", "w", "h",
    "maxDiff", "longMessage", "daemon", "title", "geometry", "protocol"}
RUFF_ERRORES = {"F821", "F822", "F823", "F811", "F631", "F632", "F702", "F704", "F706", "F707", "B006", "B012"}
TEXTO_RUFF = {"F841": "variable asignada y NUNCA usada (¿lógica sin terminar?)", "F821": "nombre NO definido",
              "F811": "definición repetida que pisa a la anterior", "F401": "import sin usar",
              "B006": "argumento mutable por defecto", "E722": "except vacío (oculta errores)",
              "B018": "expresión sin efecto", "F632": "comparación 'is' con literal"}


class Informe:
    def __init__(self):
        self.errores, self.avisos, self.ok, self.tipos = [], [], [], []
        self.previos: list[str] = []  # modo grande: errores que ya estaban antes de empezar (no cuentan)

    def texto(self, maximo: int) -> str:
        cab = (f"VERIFICACIÓN ({', '.join(self.tipos) or 'sin código'}): {len(self.errores)} ERRORES, "
               f"{len(self.avisos)} AVISOS")
        partes = [cab]
        for titulo, lista in (("ERRORES (corrígelos todos)", self.errores), ("AVISOS (revisa cada uno)", self.avisos),
                              ("OK", self.ok)):
            if lista:
                partes.append(f"{titulo}:\n" + "\n".join(f"- {x}" for x in lista[:25]))
        if self.previos:
            partes.append(f"PREEXISTENTES ({len(self.previos)}; ya estaban antes del modo grande y no son de esta parte: "
                          "no los toques):\n" + "\n".join(f"- {x[:160]}" for x in self.previos[:5]))
        txt = "\n".join(partes)
        return txt if len(txt) <= maximo else txt[:maximo - 40] + "\n[... informe recortado]"


def ignorado(p: Path, raiz: Path) -> bool:
    """Carpetas que no son código del proyecto: entornos, compilados y dependencias (también lib/ de Foundry, con
    forge-std y OpenZeppelin, que traen sus propios scripts y package.json, y sus cache/ y broadcast/; y gen/ e icons/
    junto a tauri.conf.json, que Tauri regenera al compilar: sus esquemas, ~300 KB, tapaban el código al revisor)."""
    partes = p.relative_to(raiz).parts[:-1]
    if IGNORAR & set(partes):
        return True
    return any((parte in ("lib", "cache", "broadcast") and (raiz.joinpath(*partes[:i]) / "foundry.toml").exists())
               or (parte in ("gen", "icons") and (raiz.joinpath(*partes[:i]) / "tauri.conf.json").exists())
               for i, parte in enumerate(partes))


def archivos(raiz: Path, *exts: str) -> list[Path]:
    return sorted(p for p in raiz.rglob("*") if p.suffix.lower() in exts and p.is_file() and not ignorado(p, raiz))


def _rel(p: Path, raiz: Path) -> str:
    try:
        return str(p.resolve().relative_to(raiz))
    except ValueError:
        return str(p)


def correr(cmd, cwd, timeout, entorno=None) -> tuple[int | None, bytes]:
    """(código o None si se pasó del tiempo, salida en bruto). Mata el árbol de procesos si se pasa del tiempo.

    La salida va a un archivo temporal y no a una tubería: si el comando deja algo en segundo plano (`start /b`, un
    servidor), ese proceso hereda la salida y con una tubería la espera no acabaría nunca."""
    with tempfile.TemporaryFile() as archivo:
        proc = subprocess.Popen(cmd, cwd=cwd, env=entorno, stdin=subprocess.DEVNULL, stdout=archivo,
                                stderr=subprocess.STDOUT, creationflags=SIN_VENTANA, shell=isinstance(cmd, str))
        try:
            codigo = proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            subprocess.run(["taskkill", "/T", "/F", "/PID", str(proc.pid)], capture_output=True, check=False)
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                pass
            codigo = None
        archivo.seek(0)
        return codigo, archivo.read()


def _correr(cmd, cwd, timeout=60, entorno=None) -> tuple[int | None, str]:
    """(código o None si timeout, salida como texto)."""
    codigo, salida = correr(cmd, cwd, timeout, entorno)
    try:
        texto = salida.decode("utf-8")
    except UnicodeDecodeError:
        texto = salida.decode("cp850", "replace")
    return codigo, texto.strip()


def _cola(texto: str, n: int = 12) -> str:
    return "\n    ".join(texto.strip().splitlines()[-n:])


def _traza_limpia(tb: str) -> str:
    """Traceback sin marcas ~~~^^^ ni marcos internos: archivo/línea del proyecto + la excepción final."""
    lineas = [l for l in tb.splitlines() if l.strip() and not re.fullmatch(r"\s*[~^\s]+", l)
              and "<frozen" not in l and "runpy" not in l and "pygame_sonda" not in l]
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
    _, salida = _correr([str(ruff), "check", "--no-cache", "--output-format", "concise", "--select", sel,
                         "--ignore", "F403,F405,F541", *map(str, rutas)], raiz, 60)
    for linea in salida.splitlines():
        m = re.match(r"(.+?):(\d+):\d+: (\w+) (.*)", linea)
        if not m:
            continue
        archivo, num, codigo, msg = m.groups()
        detalle = f"{_rel(Path(archivo), raiz)}:{num} {TEXTO_RUFF.get(codigo, '')} [{codigo}: {msg}]".replace("  ", " ")
        if codigo.startswith(("E9", "PLE")) or codigo in RUFF_ERRORES:
            inf.errores.append(detalle)
        elif codigo != "F401":  # imports sin usar: inofensivos, y "arreglarlos" rompe código que funciona
            inf.avisos.append(detalle)


def codigo_muerto(raiz: Path, rutas: list[Path], inf: Informe, solo: Path | None = None) -> None:
    """solo = carpeta verificada: el análisis mira todo el proyecto (los usos pueden estar fuera de ella) pero solo
    informa de lo que hay dentro."""
    vulture = BIN / "vulture.exe"
    if not vulture.exists() or not rutas:
        return
    _, salida = _correr([str(vulture), *map(str, rutas), "--min-confidence", "60"], raiz, 60)
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
            inf.errores.append(f"{_rel(Path(archivo), raiz)}:{num} {traduccion} '{nombre}' NUNCA se usa: el programa no "
                               f"la crea ni la llama. Conéctala al flujo principal (o bórrala si sobra).")
        else:
            inf.avisos.append(f"{_rel(Path(archivo), raiz)}:{num} {traduccion} '{nombre}' se define pero NUNCA se usa "
                              f"(¿falta llamarla/conectarla al flujo principal?)")
        n += 1
        if n >= 12:
            break


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
            inf.avisos.append(f"{_rel(t, raiz)} no importa ningún módulo del proyecto: NO prueba tu código.")
        elif asserts == 0:
            inf.avisos.append(f"{_rel(t, raiz)} no tiene ningún assert: no comprueba nada.")
    env = dict(os.environ, SDL_VIDEODRIVER="dummy", SDL_AUDIODRIVER="dummy", PYTHONUTF8="1", MPLBACKEND="Agg")
    codigo, _ = _correr([python, "-c", "import pytest"], raiz, 30, env)
    cmd = ([python, "-m", "pytest", "-q", "-x", "--no-header", "-p", "no:cacheprovider"] if codigo == 0
           else [python, "-m", "unittest", "discover", "-p", "test*.py"])
    codigo, salida = _correr(cmd, raiz, 180, env)
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


def _entrada_python(raiz: Path, rutas: list[Path]) -> Path | None:
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
    entrada = _entrada_python(raiz, rutas)
    if entrada is None:
        return
    fuente = entrada.read_text(encoding="utf-8", errors="replace")
    usa_pygame = any("pygame" in p.read_text(encoding="utf-8", errors="replace") for p in rutas
                     if not p.name.startswith("test"))
    rel = _rel(entrada, raiz)
    env = dict(os.environ, PYTHONUTF8="1", PYTHONIOENCODING="utf-8", SDL_VIDEODRIVER="dummy", SDL_AUDIODRIVER="dummy")
    if usa_pygame:
        if "pygame" not in inf.tipos:
            inf.tipos.append("pygame")
        tramo = max(2.0, segundos / 3)
        total = {"frames": 0, "juego": 0, "pantallas": 0}
        for semilla in (1, 2, 3):  # tres partidas distintas pero reproducibles
            codigo, salida = _correr([python, str(SONDA_PYGAME), str(entrada), str(tramo)], entrada.parent, tramo + 25,
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
                    inf.errores.append(f"{rel}: falló al arrancar:\n    {_cola(salida)}")
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
    if re.search(r"\b(tkinter|PyQt|PySide|customtkinter|wx)\b", fuente):
        inf.avisos.append(f"{rel}: app con ventana (GUI): no se abre aquí; prueba su lógica con tests.")
        return
    codigo, salida = _correr([python, str(entrada)], entrada.parent, segundos, env)
    if codigo is None:
        inf.ok.append(f"{rel} sigue en ejecución tras {segundos} s (servidor o bucle): arrancó sin errores")
    elif "EOFError" in salida:
        inf.avisos.append(f"{rel} pide datos con input(): pruébalo con tests o con 'echo datos| python {entrada.name}'.")
    elif codigo != 0:
        inf.errores.append(f"{rel} termina con error (exit {codigo}):\n    {_cola(salida)}")
    else:
        inf.ok.append(f"{rel} se ejecuta sin errores")


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
                inf.errores.append(f"{_rel(h, raiz)}: {nombre} {usada} está DESACTUALIZADA (la actual es {actual}). "
                                   f"Usa la versión actual{consejo}.")


def html_estatico(raiz: Path, htmls: list[Path], jss: list[Path], inf: Informe) -> None:
    for h in htmls:
        p = _HtmlInfo()
        p.feed(h.read_text(encoding="utf-8", errors="replace"))
        for i in {x for x in p.ids if p.ids.count(x) > 1}:
            inf.avisos.append(f"{_rel(h, raiz)}: id '{i}' repetido.")
        for ref in p.refs:
            if not ((raiz / ref.lstrip("/")) if ref.startswith("/") else (h.parent / ref)).exists():
                inf.errores.append(f"{_rel(h, raiz)} referencia '{ref}' y NO existe.")
        abiertas = [t for t in p.mal_cerradas + p.pila if t not in ("html", "body", "head", "p", "li", "td", "tr", "option")]
        if abiertas:
            inf.avisos.append(f"{_rel(h, raiz)}: etiquetas sin cerrar: {', '.join(sorted(set(abiertas)))[:120]}")
        for i, s in enumerate(p.scripts):
            _node_check(raiz, s, f"{_rel(h, raiz)} <script> #{i + 1}", inf, p.modulo)
    for j in jss:
        _node_check(raiz, j, _rel(j, raiz), inf)


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
                usos.setdefault(m[m.lastindex], []).append(f"{_rel(p, raiz)}:{linea}")
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
        codigo, salida = _correr(["node", "--check", str(ruta)], raiz, 30)
    finally:
        if temporal:
            os.unlink(temporal)
    if codigo not in (0, None):
        lineas = [l for l in salida.splitlines() if l.strip()]
        detalle = next((l for l in lineas if "Error" in l), lineas[-1] if lineas else "")
        donde = next((l.rsplit(":", 1)[-1] for l in lineas if re.search(r":\d+$", l)), "?")
        inf.errores.append(f"{nombre}:{donde} ERROR DE SINTAXIS JS: {detalle[:160]}")


class _Silencioso(SimpleHTTPRequestHandler):
    def log_message(self, *a):
        pass


def _canal_navegador(p):
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
            navegador(raiz, [compilada], inf, raiz_web=paquete / "dist")
        else:
            inf.avisos.append(f"{_rel(h, raiz)}: es la página fuente de Vite (carga {fuente_vite(h)}) y no se abre tal cual; "
                              "se prueba la compilada de dist/ cuando `npm run build` funciona.")


def navegador(raiz: Path, htmls: list[Path], inf: Informe, raiz_web: Path | None = None) -> None:
    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        inf.avisos.append("Prueba en navegador no disponible (falta playwright en el agente: ejecuta instalar.bat).")
        return
    paginas = sorted(htmls, key=lambda p: (p.name != "index.html", len(p.parts)))[:2]
    web = raiz_web or raiz  # dist/ de Vite se sirve desde su carpeta: sus rutas /assets/... son absolutas
    servidor = ThreadingHTTPServer(("127.0.0.1", 0), partial(_Silencioso, directory=str(web)))
    threading.Thread(target=servidor.serve_forever, daemon=True).start()
    capturas = raiz / ".agente" / "capturas"
    capturas.mkdir(parents=True, exist_ok=True)
    try:
        with sync_playwright() as p:
            nav = _canal_navegador(p)
            if nav is None:
                inf.avisos.append("No se pudo abrir Edge/Chrome sin ventana para probar la página.")
                return
            for h in paginas:
                rel, url = _rel(h, raiz).replace("\\", "/"), h.relative_to(web).as_posix()
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
                for nivel, msg in problemas:
                    if (msg in vistos or "Failed to load resource" in msg
                            or (msg.startswith("no cargó ") and msg[9:] in con_http)):
                        continue
                    vistos.add(msg)
                    (inf.errores if nivel == "error" else inf.avisos).append(f"{rel}: {msg}")
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
            codigo, salida = _correr("mvn -q -B test", raiz, 600)
            (inf.ok.append("mvn test OK") if codigo == 0 else inf.errores.append(f"mvn test falla:\n    {_cola(salida, 25)}"))
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
    codigo, salida = _correr(["javac", "-encoding", "UTF-8", "-Xlint:all", "-d", str(salida_dir), f"@{lista}"], raiz, 180)
    for m in re.finditer(r"^(.+?\.java):(\d+): (error|warning): (.+)$", salida, re.MULTILINE):
        detalle = f"{_rel(Path(m.group(1)), raiz)}:{m.group(2)} {m.group(4)}"
        (inf.errores if m.group(3) == "error" else inf.avisos).append(detalle)
    if codigo != 0:
        if not inf.errores:
            inf.errores.append(f"javac falla:\n    {_cola(salida)}")
        return
    inf.ok.append(f"javac: {len(principales)} archivos compilan")
    for f in principales:
        texto = f.read_text(encoding="utf-8", errors="replace")
        if re.search(r"static\s+void\s+main\s*\(", texto):
            paquete = re.search(r"^\s*package\s+([\w.]+)\s*;", texto, re.MULTILINE)
            clase = f"{paquete.group(1)}.{f.stem}" if paquete else f.stem
            codigo, salida = _correr(["java", "-Djava.awt.headless=true", "-cp", str(salida_dir), clase], raiz, segundos)
            if codigo is None:
                inf.ok.append(f"{clase} sigue en ejecución tras {segundos} s (bucle/servidor): arrancó sin errores")
            elif "NoSuchElementException" in salida:
                inf.avisos.append(f"{clase} pide datos por teclado (Scanner): pruébalo con 'echo datos| java -cp out {clase}'.")
            elif "HeadlessException" in salida:
                inf.avisos.append(f"{clase} abre ventana (Swing/AWT): no se prueba aquí; separa y prueba la lógica.")
            elif codigo != 0:
                inf.errores.append(f"{clase} termina con excepción:\n    {_cola(salida)}")
            else:
                inf.ok.append(f"{clase} se ejecuta sin errores")
            break


# ---------------------------------------------------------------- .bat
def bats(raiz: Path, lista: list[Path], inf: Informe) -> None:
    for b in lista:
        texto = b.read_text(encoding="utf-8", errors="replace")
        rel = _rel(b, raiz)
        if not re.search(r'cd\s+/d\s+"?%~dp0', texto, re.IGNORECASE) and re.search(r"\b(python|py|node|java|javac)\b", texto, re.IGNORECASE):
            inf.avisos.append(f"{rel}: no hace cd /d \"%~dp0\": fallará si se abre desde otra carpeta.")
        lineas = [l.strip() for l in texto.splitlines()]
        for i, l in enumerate(lineas):
            if re.search(r"(http\.server|flask\s+run|uvicorn|npm\s+(run\s+)?(start|dev)|npx\s+serve|live-server)", l, re.IGNORECASE) \
                    and not re.match(r"(?i)(echo|rem|start)\b", l):
                despues = [x for x in lineas[i + 1:] if x and not re.match(r"(?i)(echo|rem|::|if |goto|exit|pause|:|\))", x)]
                if despues:
                    inf.errores.append(f"{rel}: '{l[:60]}' bloquea (el servidor no termina) y las líneas siguientes "
                                       f"('{despues[0][:40]}') nunca se ejecutan. Pon 'start \"\" http://...' ANTES del servidor.")
        patron = r'\b(?:python|py(?: -3\.\d+)?|node|java -jar)\s+(?:"([^"]+\.(?:py|js|mjs|jar))"|(?!-)([^\s"&|<>]+\.(?:py|js|mjs|jar))\b)'
        for m in re.finditer(patron, texto, re.IGNORECASE):
            destino = (m.group(1) or m.group(2)).strip()
            if "%" not in destino and not (b.parent / destino).exists():
                inf.errores.append(f"{rel} ejecuta '{destino}' y ese archivo NO existe junto al .bat.")


# ---------------------------------------------------------------- entrada
def normalizar(error: str) -> str:
    """Un error sin números (líneas, recuentos): reconoce los que ya estaban aunque el código se mueva."""
    return re.sub(r"\d+", "N", error)


def verificar(raiz: Path, python: str, ruta: str = ".", segundos: int = 8, maximo: int = 6000,
              previos: set[str] | None = None, idiomas_parcial: list[str] | None = None) -> tuple[str, int]:
    """Devuelve (informe, número de errores). previos = errores normalizados que no cuentan (modo grande)."""
    inf = informe_proyecto(raiz, python, ruta, segundos, idiomas_parcial)
    if isinstance(inf, str):
        return inf, 1
    if previos:
        inf.previos = [e for e in inf.errores if normalizar(e) in previos]
        inf.errores = [e for e in inf.errores if normalizar(e) not in previos]
    return inf.texto(maximo), len(inf.errores)


def informe_proyecto(raiz: Path, python: str, ruta: str = ".", segundos: int = 8,
                     idiomas_parcial: list[str] | None = None) -> "Informe | str":
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
                inf.errores.append(f"{_rel(p, raiz)}:{e.lineno} ERROR DE SINTAXIS: {e.msg}")
        sintaxis_ok = len(inf.errores) == errores_antes
        lint_python(raiz, pys, inf)
        if base == raiz:
            codigo_muerto(raiz, pys, inf)
        else:  # una subcarpeta: quien usa sus funciones puede estar fuera
            codigo_muerto(raiz, archivos(raiz, ".py"), inf, solo=base)
        if sintaxis_ok:  # con errores de sintaxis no tiene sentido ejecutar
            tests_python(raiz, pys, python, inf)
            ejecutar_python(raiz, pys, python, inf, segundos)
    if htmls or jss:
        inf.tipos.append("web")
        html_estatico(raiz, htmls, jss, inf)
        ids_dom(raiz, base, inf)
        librerias_cdn(raiz, htmls, inf)
        directas = [h for h in htmls if not fuente_vite(h)]
        if directas:
            navegador(raiz, directas, inf)
    javas = [p for p in javas if not _de_android(p, base)]  # esos los compila Gradle (android_gradle, flutter)
    if javas:
        inf.tipos.append("java")
        java(raiz, javas, inf, segundos)
    if bat:
        bats(raiz, bat, inf)
    from . import ecosistemas  # aquí para evitar el import circular (ecosistemas usa las utilidades de este módulo)
    ecosistemas.verificar_todos(base, raiz, inf)
    if (vite := [h for h in htmls if fuente_vite(h)]):  # después de node(): su npm run build deja dist/
        paginas_vite(raiz, vite, inf)
    from .idiomas import verificar_idiomas  # solo añade algo si el proyecto tiene archivos de idioma
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
        _node_check(raiz, p, _rel(p, raiz), inf)
    elif suf in (".bat", ".cmd"):
        bats(raiz, [p], inf)
    elif suf in (".go", ".rs", ".yml", ".yaml") or p.name.lower() == "dockerfile" or ".dockerfile" in p.name.lower():
        from . import ecosistemas
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
