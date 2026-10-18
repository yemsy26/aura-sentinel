"""mapa(): mapa compacto del repositorio sin el modelo (módulos, archivos, líneas y símbolos de primer nivel).

Caché en .agente/mapa.json: solo se vuelve a analizar lo que cambió (mtime o tamaño). Pensado para repos grandes, donde
listar y leer archivo a archivo gasta pasos y contexto.
"""
import ast
import json
import os
import re
import threading
from pathlib import Path

from .verificacion import IGNORAR, es_codigo

PATRONES_MAPA = {
    ".js": r"(?:^|\s)(?:class|function|interface|record)\s+(\w+)", ".mjs": r"(?:^|\s)(?:class|function)\s+(\w+)",
    ".ts": r"(?:^|\s)(?:class|function|interface|type|enum)\s+(\w+)", ".tsx": r"(?:^|\s)(?:class|function|interface)\s+(\w+)",
    ".jsx": r"(?:^|\s)(?:class|function)\s+(\w+)", ".java": r"(?:^|\s)(?:class|interface|record|enum)\s+(\w+)",
    ".kt": r"(?:^|\s)(?:class|interface|object|fun)\s+(\w+)", ".cs": r"(?:^|\s)(?:class|interface|record|struct|enum)\s+(\w+)",
    ".rs": r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:fn|struct|enum|trait|mod)\s+(\w+)",
    ".go": r"^(?:func\s+(?:\([^)]*\)\s*)?|type\s+)(\w+)",
    ".sol": r"^\s*(?:abstract\s+)?(?:contract|interface|library|function|event|error)\s+(\w+)",
    ".dart": r"^\s*(?:abstract\s+)?(?:class|mixin|enum|extension)\s+(\w+)",
}
EXTRA = {".php": r"^\s*(?:abstract\s+|final\s+)?(?:class|interface|trait|function)\s+(\w+)",
         ".rb": r"^\s*(?:class|module|def)\s+([\w.]+)", ".swift": r"^\s*(?:public\s+|final\s+)*(?:class|struct|enum|protocol|func)\s+(\w+)"}
BLOQUEOS = {"package-lock.json", "yarn.lock", "pnpm-lock.yaml", "cargo.lock", "go.sum", "packages.lock.json",
            "composer.lock", "poetry.lock"}
MAX_ARCHIVO = 2_000_000  # bundles minificados y datos enormes no son módulos que leer
GRANDE_ARCHIVOS, GRANDE_BYTES = 25, 120_000  # desde aquí se ofrece mapa y puede entrar el modo grande
_CERROJO = threading.Lock()


def _recorrer(raiz: Path):
    """(ruta relativa con '/', DirEntry) de los archivos de código, podando entornos, dependencias y compilados."""
    pila = [(raiz, "")]
    while pila:
        carpeta, prefijo = pila.pop()
        try:
            entradas = list(os.scandir(carpeta))
        except OSError:
            continue
        for e in entradas:
            if e.is_dir(follow_symlinks=False):
                if e.name not in IGNORAR and not e.name.startswith("."):
                    pila.append((Path(e.path), f"{prefijo}{e.name}/"))
            elif e.is_file(follow_symlinks=False) and e.name.lower() not in BLOQUEOS and es_codigo(Path(e.name)):
                yield f"{prefijo}{e.name}", e


def es_grande(raiz: Path) -> bool:
    """Repo grande (≥ 25 archivos de código o ≥ 120 KB). Para en cuanto lo sabe: en un proyecto nuevo cuesta milisegundos."""
    n = total = 0
    for _, e in _recorrer(raiz):
        n += 1
        try:
            total += e.stat().st_size
        except OSError:
            continue
        if n >= GRANDE_ARCHIVOS or total >= GRANDE_BYTES:
            return True
    return False


def _simbolos(nombre: str, texto: str) -> list[str]:
    suf = Path(nombre).suffix.lower()
    if suf == ".py":
        try:
            arbol = ast.parse(texto)
        except (SyntaxError, ValueError):
            return ["(error de sintaxis)"]
        out = []
        for n in arbol.body:
            if isinstance(n, ast.ClassDef):
                metodos = [m.name for m in n.body if isinstance(m, (ast.FunctionDef, ast.AsyncFunctionDef))
                           and not m.name.startswith("__")]
                out.append(f"class {n.name}({', '.join(metodos[:6])}{', …' if len(metodos) > 6 else ''})")
            elif isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)):
                out.append(f"{n.name}()")
            elif isinstance(n, ast.Assign) and isinstance(n.targets[0], ast.Name) and n.targets[0].id.isupper():
                out.append(n.targets[0].id)
        return out
    patron = PATRONES_MAPA.get(suf) or EXTRA.get(suf)
    return list(dict.fromkeys(re.findall(patron, texto, re.MULTILINE)))[:12] if patron else []


def actualizar(raiz: Path) -> dict:
    """{ruta: [mtime_ns, tamaño, líneas, símbolos]} al día. Solo analiza los archivos nuevos o cambiados."""
    raiz = Path(raiz)
    cache = raiz / ".agente" / "mapa.json"
    with _CERROJO:
        try:
            previo = json.loads(cache.read_text(encoding="utf-8")).get("archivos", {})
        except (OSError, ValueError, AttributeError):
            previo = {}
        actual, cambios = {}, 0
        for rel, e in _recorrer(raiz):
            try:
                st = e.stat()
            except OSError:
                continue
            antes = previo.get(rel)
            if antes and antes[0] == st.st_mtime_ns and antes[1] == st.st_size:
                actual[rel] = antes
                continue
            cambios += 1
            if st.st_size > MAX_ARCHIVO:
                actual[rel] = [st.st_mtime_ns, st.st_size, 0, ["(archivo enorme: no se analiza)"]]
                continue
            try:
                texto = Path(e.path).read_bytes().decode("utf-8", "replace")
            except OSError:
                continue
            lineas = texto.count("\n") + (bool(texto) and not texto.endswith("\n"))
            actual[rel] = [st.st_mtime_ns, st.st_size, lineas, _simbolos(rel, texto)]
        if cambios or len(actual) != len(previo):
            try:
                cache.parent.mkdir(exist_ok=True)
                cache.write_text(json.dumps({"version": 1, "archivos": actual}, ensure_ascii=False), encoding="utf-8")
            except OSError:
                pass
        return actual


def precalentar(raiz: Path) -> threading.Thread:
    """Pone la caché al día en segundo plano: cuando el modelo pida el mapa ya estará hecho."""
    hilo = threading.Thread(target=actualizar, args=(Path(raiz),), daemon=True, name="mapa")
    hilo.start()
    return hilo


def modulos(rutas: list[str], maximo: int = 30) -> list[tuple[str, list[str]]]:
    """Agrupa por carpeta de primer nivel; una carpeta con más de `maximo` archivos se parte en sus subcarpetas."""
    def partir(prefijo: str, lista: list[str], nivel: int) -> list[tuple[str, list[str]]]:
        if nivel and (len(lista) <= maximo or nivel >= 4):
            return [(prefijo, lista)]
        sub: dict[str, list[str]] = {}
        directos = []
        for r in lista:
            partes = r.split("/")
            (sub.setdefault("/".join(partes[:nivel + 1]) + "/", []) if len(partes) > nivel + 1 else directos).append(r)
        out = [(prefijo or "./", directos)] if directos else []
        for k in sorted(sub):
            out += partir(k, sub[k], nivel + 1)
        return out
    return partir("", sorted(rutas), 0)


def _miles(n: int) -> str:
    return f"{n:,}".replace(",", ".")


def texto(raiz: Path, ruta: str = "", maximo: int = 6000, idiomas: str = "") -> str:
    archivos = actualizar(raiz)
    if not archivos:
        return "El proyecto no tiene archivos de código todavía."
    ruta = ruta.strip().strip("\"'").replace("\\", "/").strip("/").removeprefix("./")
    if not ruta or ruta == ".":
        lineas = [(f"MAPA: {_miles(len(archivos))} archivos de código, {_miles(sum(a[2] for a in archivos.values()))} "
                   "líneas (por módulo; detalle con mapa(ruta='carpeta')):")]
        if idiomas:
            lineas.append(f"Idiomas: {idiomas}")
        for nombre, lista in modulos(list(archivos)):
            mayores = sorted(lista, key=lambda r: -archivos[r][2])[:6]
            nombres = ", ".join(r.removeprefix(nombre if nombre != "./" else "") for r in mayores)
            resto = f" (+{len(lista) - len(mayores)})" if len(lista) > len(mayores) else ""
            lineas.append(f"- {nombre} ({len(lista)} arch., {_miles(sum(archivos[r][2] for r in lista))} lín.): "
                          f"{nombres}{resto}")
        out = "\n".join(lineas)
        return out if len(out) <= maximo else out[:maximo - 60] + "\n[... recortado: pide mapa(ruta) de un módulo]"
    elegidos = sorted(r for r in archivos if r == ruta or r.startswith(ruta + "/"))
    if not elegidos:
        return f"No hay archivos de código en '{ruta}'. Mira mapa() para ver los módulos."
    lineas, usado = [f"{ruta}: {len(elegidos)} archivos"], 0
    for r in elegidos:
        _, _, n, simbolos = archivos[r]
        linea = f"- {r} ({n} lín.)" + (f": {', '.join(simbolos)}"[:220] if simbolos else "")
        usado += len(linea) + 1
        if usado > maximo - 80:
            lineas.append(f"[... {len(elegidos) - len(lineas) + 1} archivos más: pide una subcarpeta]")
            break
        lineas.append(linea)
    return "\n".join(lineas)
