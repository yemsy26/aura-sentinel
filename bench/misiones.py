"""Las 12 misiones del banco de evaluación de Aura: mandato, tope de tiempo, fixture (la carpeta de partida) y juez automático.

Salieron de las pruebas m4185 y m4192 (ver README.md). Un juez devuelve (ok, detalle) mirando solo lo que el agente dejó en la
carpeta; no lee su resumen. iniciar() fija el motor, el python con el que se juzga y el entorno aislado (TEMP propio) antes de usarlas.
"""
import collections
import csv
import hashlib
import json
import os
import random
import re
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
PY = REPO / "motor" / ".venv" / "Scripts" / "python.exe"
MOTOR = REPO / "motor"
ENV: dict = dict(os.environ)
OMITIR = {".git", ".agente", "node_modules", "target", ".venv", "venv", "__pycache__", "dist", "build"}


def iniciar(motor: Path, tmp: Path, python: Path | None = None) -> None:
    """motor: la carpeta motor/ que ejecuta las misiones (el árbol del repo o una copia congelada con git archive); tmp: TEMP
    propio de las misiones y de los jueces; python: el que corre los jueces de Python (por defecto el venv del motor del repo)."""
    global PY, MOTOR, ENV
    MOTOR = Path(motor).resolve()
    PY = Path(python) if python else (REPO / "motor" / ".venv" / "Scripts" / "python.exe")
    if not PY.exists():
        PY = Path(sys.executable)
    tmp.mkdir(parents=True, exist_ok=True)
    ENV = {k: v for k, v in os.environ.items() if k.upper() != "NODEFAULTCURRENTDIRECTORYINEXEPATH"}
    ENV.update(PYTHONUTF8="1", TEMP=str(tmp), TMP=str(tmp), TMPDIR=str(tmp), AGENTE_SIN_ESCRITORIO="1", AGENTE_NOPAUSE="1", CI="1")


# ------------------------------------------------------------------ utilidades
def ejecutar(cmd, cwd, timeout=120, shell=False) -> tuple[int, str]:
    try:
        r = subprocess.run(cmd, cwd=cwd, env=ENV, capture_output=True, text=True, encoding="utf-8", errors="replace", check=False,
                           timeout=timeout, stdin=subprocess.DEVNULL, shell=shell)
        return r.returncode, (r.stdout or "") + (r.stderr or "")
    except subprocess.TimeoutExpired:
        return -9, "TIMEOUT"
    except OSError as e:
        return -1, f"{type(e).__name__}: {e}"


def sha(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


def numeros(texto: str) -> list[float]:
    out = []
    for tok in re.findall(r"\d[\d.,]*", texto):
        t = tok.rstrip(".,")
        m = re.fullmatch(r"(\d{1,3}(?:[.,]\d{3})+|\d+)(?:[.,](\d{1,2}))?", t)
        if m:
            out.append(float(f"{re.sub(r'[.,]', '', m.group(1))}.{m.group(2) or '0'}"))
        try:
            out.append(float(t.replace(",", ".")))
        except ValueError:
            pass
    return out


def hay(valor: float, lista: list[float], tol: float = 0.011) -> bool:
    return any(abs(valor - x) <= tol for x in lista)


def bat_de(carpeta: Path) -> Path | None:
    r"""El .bat de doble clic: el de la carpeta o, si no hay, el de la carpeta del proyecto (un nivel más abajo). m4192: tras el
    rechazo de crear_bat (C15) el agente puso run.bat y test.bat dentro de palabras_repeticas\ y el juez, que solo miraba la raíz,
    marcaba PARCIAL un entregable que funcionaba."""
    subcarpetas = sorted(p for p in carpeta.iterdir() if p.is_dir() and p.name not in OMITIR)
    for base in (carpeta, *subcarpetas):
        for n in ("run.bat", "iniciar.bat", "abrir.bat", "ejecutar.bat"):
            if (base / n).is_file():
                return base / n
        otros = [p for p in sorted(base.glob("*.bat")) if "test" not in p.name.lower()]
        if otros:
            return otros[0]
    return None


def correr_bat(carpeta: Path, timeout: int = 90) -> tuple[int, str]:
    b = bat_de(carpeta)
    if not b:
        return -1, "no hay .bat de arranque"
    return ejecutar(["cmd", "/c", b.name], b.parent, timeout=timeout)


def verificar_motor(carpeta: Path) -> tuple[int, int, str]:
    """La verificación del propio motor sobre la carpeta (errores, avisos, informe)."""
    code = f"from agente.herramientas import Herramientas; print(Herramientas(r'{carpeta}').verificar_cmd())"
    _rc, out = ejecutar([str(PY), "-c", code], MOTOR, timeout=900)
    m = re.search(r"VERIFICACIÓN \([^)]*\): (\d+) ERRORES, (\d+) AVISOS", out)
    return (int(m.group(1)), int(m.group(2)), out[-1500:]) if m else (-1, -1, out[-1500:])


# ------------------------------------------------------------------ 1 git_conflicto
def fx_git(c: Path) -> dict:
    g = lambda *a: ejecutar(["git", *a], c, 60)
    g("init", "-b", "main")
    g("config", "user.name", "Prueba")
    g("config", "user.email", "prueba@example.com")
    (c / "README.md").write_text("# Demo\nLinea base\n", encoding="utf-8")
    (c / "config.json").write_text('{\n  "nombre": "demo",\n  "color": "azul",\n  "version": 1\n}\n', encoding="utf-8")
    (c / "app.py").write_text('print("hola")\n', encoding="utf-8")
    g("add", "-A"); g("commit", "-m", "base")
    g("checkout", "-b", "ajustes")
    (c / "config.json").write_text('{\n  "nombre": "demo",\n  "color": "verde",\n  "version": 2\n}\n', encoding="utf-8")
    (c / "README.md").write_text("# Demo\nLinea base\nLinea de ajustes\n", encoding="utf-8")
    (c / "extra.txt").write_text("extra\n", encoding="utf-8")
    g("add", "-A"); g("commit", "-m", "ajustes")
    g("checkout", "main")
    (c / "config.json").write_text('{\n  "nombre": "demo-main",\n  "color": "rojo",\n  "version": 1\n}\n', encoding="utf-8")
    (c / "README.md").write_text("# Demo\nLinea base\nLinea de main\n", encoding="utf-8")
    (c / "app.py").write_text('print("hola")\nprint("main")\n', encoding="utf-8")
    g("add", "-A"); g("commit", "-m", "cambios de main")
    return {}


def juez_git(c: Path, ctx: dict) -> tuple[bool, str]:
    g = lambda *a: ejecutar(["git", *a], c, 60)
    fallos = []
    if g("merge-base", "--is-ancestor", "ajustes", "main")[0] != 0:
        fallos.append("ajustes no está en main")
    if [l for l in g("status", "--porcelain")[1].splitlines() if l.strip() and not l[3:].startswith(".agente")]:
        fallos.append("árbol sin limpiar")  # .agente/ es la carpeta de trabajo del propio motor, no del proyecto
    if g("rev-parse", "--abbrev-ref", "HEAD")[1].strip() != "main":
        fallos.append("no está en main")
    marcas = [p.name for p in c.rglob("*") if p.is_file() and ".git" not in p.parts and ".agente" not in p.parts
              and re.search(r"^(<<<<<<<|=======|>>>>>>>)", p.read_text(encoding="utf-8", errors="replace"), re.MULTILINE)]
    if marcas:
        fallos.append(f"marcas de conflicto en {marcas}")
    try:
        cfg = json.loads((c / "config.json").read_text(encoding="utf-8"))
        if cfg.get("nombre") != "demo-main" or cfg.get("version") != 2 or cfg.get("color") not in ("rojo", "verde"):
            fallos.append(f"config.json mal fusionado: {cfg}")
    except (ValueError, OSError) as e:
        fallos.append(f"config.json no es JSON: {e}")
    leeme = (c / "README.md").read_text(encoding="utf-8", errors="replace") if (c / "README.md").exists() else ""
    if "Linea de ajustes" not in leeme or "Linea de main" not in leeme:
        fallos.append("README sin las dos líneas")
    if not (c / "extra.txt").exists():
        fallos.append("falta extra.txt")
    if 'print("main")' not in ((c / "app.py").read_text(encoding="utf-8", errors="replace") if (c / "app.py").exists() else ""):
        fallos.append("app.py perdió el cambio de main")
    return not fallos, "; ".join(fallos) or "fusión correcta"


# ------------------------------------------------------------------ 2 bug_py
INVENTARIO = '''class Inventario:
    def __init__(self):
        self.items = {}

    def agregar(self, nombre, cantidad, precio):
        if cantidad <= 0:
            raise ValueError("cantidad debe ser positiva")
        if nombre in self.items:
            self.items[nombre]["cantidad"] += cantidad
            self.items[nombre]["precio"] = precio
        else:
            self.items[nombre] = {"cantidad": cantidad, "precio": precio}

    def retirar(self, nombre, cantidad):
        if nombre not in self.items:
            raise KeyError(nombre)
        self.items[nombre]["cantidad"] -= cantidad
        if self.items[nombre]["cantidad"] == 0:
            del self.items[nombre]

    def valor_total(self):
        return sum(i["cantidad"] * i["precio"] // 1 for i in self.items.values())

    def buscar(self, texto):
        return sorted(n for n in self.items if texto in n)
'''
TEST_INVENTARIO = '''import unittest

from inventario import Inventario


class PruebasInventario(unittest.TestCase):
    def setUp(self):
        self.inv = Inventario()
        self.inv.agregar("Tornillo", 100, 0.25)
        self.inv.agregar("Martillo", 3, 12.5)

    def test_valor_total(self):
        self.assertAlmostEqual(self.inv.valor_total(), 62.5)

    def test_retirar_valida_la_existencia(self):
        with self.assertRaises(ValueError):
            self.inv.retirar("Martillo", 5)

    def test_retirar_descuenta(self):
        self.inv.retirar("Martillo", 1)
        self.assertEqual(self.inv.items["Martillo"]["cantidad"], 2)

    def test_retirar_todo_lo_quita(self):
        self.inv.retirar("Martillo", 3)
        self.assertNotIn("Martillo", self.inv.items)

    def test_buscar_sin_distinguir_mayusculas(self):
        self.assertEqual(self.inv.buscar("mart"), ["Martillo"])

    def test_agregar_suma(self):
        self.inv.agregar("Tornillo", 50, 0.3)
        self.assertEqual(self.inv.items["Tornillo"]["cantidad"], 150)


if __name__ == "__main__":
    unittest.main()
'''


def fx_bug(c: Path) -> dict:
    (c / "inventario.py").write_text(INVENTARIO, encoding="utf-8")
    (c / "test_inventario.py").write_text(TEST_INVENTARIO, encoding="utf-8")
    return {"sha_test": sha(c / "test_inventario.py")}


def juez_bug(c: Path, ctx: dict) -> tuple[bool, str]:
    t = c / "test_inventario.py"
    if not t.exists() or sha(t) != ctx["sha_test"]:
        return False, "las pruebas se han tocado o borrado"
    rc, out = ejecutar([str(PY), "-m", "unittest", "-q"], c, 120)
    return rc == 0, "pruebas pasan" if rc == 0 else out[-300:]


# ------------------------------------------------------------------ 3 csv_ventas
def fx_csv(c: Path) -> dict:
    rng = random.Random(4185)
    precios = {"tornillo": 0.25, "martillo": 12.5, "clavo": 0.1, "cinta": 3.75, "pintura": 18.9}
    while True:
        filas = [(f"2026-10-{rng.randint(1, 12):02d}", p, rng.randint(1, 30), precios[p]) for p in rng.choices(list(precios), k=60)]
        por_dia = collections.Counter()
        for f, p, q, pr in filas:
            por_dia[f] += q * pr
        top = por_dia.most_common(2)
        if top[0][1] - top[1][1] > 5:
            break
    with (c / "ventas.csv").open("w", encoding="utf-8", newline="") as fh:
        w = csv.writer(fh)
        w.writerow(["fecha", "producto", "cantidad", "precio"])
        w.writerows(filas)
    totales = collections.Counter()
    for f, p, q, pr in filas:
        totales[p] += q * pr
    return {"totales": {k: round(v, 2) for k, v in totales.items()}, "mejor_dia": top[0][0]}


def pruebas_proyecto(c: Path) -> tuple[bool | None, str]:
    """True/False si hay pruebas de Python y pasan o no; None si no hay."""
    if not list(c.glob("test*.py")) and not list(c.glob("tests/*.py")):
        return None, "sin pruebas"
    rc, out = ejecutar([str(PY), "-m", "unittest", "discover", "-q"], c, 180)
    if rc != 0 and ("No module named" in out and "pytest" in out or "NO TESTS RAN" in out or "Ran 0 tests" in out):
        rc, out = ejecutar([str(PY), "-m", "pytest", "-q"], c, 180)
    return rc == 0, out[-200:]


def juez_csv(c: Path, ctx: dict) -> tuple[bool, str]:
    _rc, out = correr_bat(c)
    nums = numeros(out)
    faltan = [f"{k}={v}" for k, v in ctx["totales"].items() if not hay(v, nums)]
    dia = ctx["mejor_dia"] in out or ctx["mejor_dia"][-2:].lstrip("0") in re.findall(r"\b\d{1,2}\b", out)
    pr, det = pruebas_proyecto(c)
    fallos = ([f"totales que no salen: {faltan}"] if faltan else []) + ([] if dia else ["no dice el mejor día"]) + \
        ([f"pruebas: {det}"] if pr is not True else [])
    return not fallos, "; ".join(fallos) or "totales, mejor día y pruebas correctos"


# ------------------------------------------------------------------ 4 mochila_sat
def fx_mochila(c: Path) -> dict:
    rng = random.Random(41850)
    objs = [(f"item_{i:02d}", rng.randint(5, 40), rng.randint(10, 100)) for i in range(1, 29)]
    with (c / "objetos.csv").open("w", encoding="utf-8", newline="") as fh:
        w = csv.writer(fh)
        w.writerow(["nombre", "peso", "valor"])
        w.writerows(objs)
    cap = 120
    mejor = [0] * (cap + 1)
    for _, p, v in objs:
        for k in range(cap, p - 1, -1):
            mejor[k] = max(mejor[k], mejor[k - p] + v)
    return {"optimo": mejor[cap], "cap": cap, "objs": {n: (p, v) for n, p, v in objs}}


def juez_mochila(c: Path, ctx: dict) -> tuple[bool, str]:
    r = c / "resultado.txt"
    if not r.exists():
        return False, "no hay resultado.txt"
    nombres = set(re.findall(r"item_\d{2}", r.read_text(encoding="utf-8", errors="replace")))
    nombres &= set(ctx["objs"])
    peso = sum(ctx["objs"][n][0] for n in nombres)
    valor = sum(ctx["objs"][n][1] for n in nombres)
    ok = peso <= ctx["cap"] and valor == ctx["optimo"]
    return ok, f"valor {valor} (óptimo {ctx['optimo']}), peso {peso}/{ctx['cap']}, {len(nombres)} objetos"


# ------------------------------------------------------------------ 5 investigar_web
def _json(url: str):
    with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "aura-prueba"}), timeout=20) as r:
        return json.loads(r.read().decode("utf-8"))


def fx_nada(c: Path) -> dict:
    return {}


def juez_investigar(c: Path, ctx: dict) -> tuple[bool, str]:
    p = c / "informe.md"
    if not p.exists():
        return False, "no hay informe.md"
    t = p.read_text(encoding="utf-8", errors="replace")
    fallos, notas = [], []
    if len(set(re.findall(r"https?://\S+", t))) < 3:
        fallos.append("menos de 3 URLs")
    try:
        py = _json("https://endoflife.date/api/python.json")[0]["latest"]
        mm = ".".join(py.split(".")[:2])
        if not re.search(rf"\b{re.escape(mm)}(\.\d+)?\b", t):
            fallos.append(f"Python {mm} no aparece")
        notas.append(f"python {py}")
    except Exception as e:  # sin red o formato nuevo: no se puede juzgar esta parte
        notas.append(f"python no verificable ({type(e).__name__})")
    try:
        idx = _json("https://nodejs.org/dist/index.json")
        actual = idx[0]["version"].lstrip("v").split(".")[0]
        lts = next(i["version"].lstrip("v").split(".")[0] for i in idx if i.get("lts"))
        if not re.search(rf"\b(v)?({actual}|{lts})\.\d+", t):
            fallos.append(f"Node {actual} (actual) o {lts} (LTS) no aparece")
        notas.append(f"node {actual}/{lts}")
    except Exception as e:
        notas.append(f"node no verificable ({type(e).__name__})")
    try:
        with urllib.request.urlopen("https://static.rust-lang.org/dist/channel-rust-stable.toml", timeout=20) as r:
            rv = re.search(r'\[pkg\.rust\]\s*version = "(\d+\.\d+)', r.read().decode("utf-8")).group(1)
        if not re.search(rf"\b{re.escape(rv)}(\.\d+)?\b", t):
            fallos.append(f"Rust {rv} no aparece")
        notas.append(f"rust {rv}")
    except Exception as e:
        notas.append(f"rust no verificable ({type(e).__name__})")
    return not fallos, "; ".join(fallos + notas)


# ------------------------------------------------------------------ 6-8 palabras (node, go, rust)
def fx_palabras(c: Path) -> dict:
    rng = random.Random(41856)
    top = [("de", 40), ("la", 36), ("que", 32), ("el", 28), ("en", 25), ("los", 22), ("más", 19), ("para", 16), ("con", 13),
           ("también", 10)]
    relleno = ["casa", "perro", "libro", "mesa", "luz", "mar", "sol", "agua", "tierra", "viento", "camino", "puerta", "río", "nube"]
    palabras = [w for w, n in top for _ in range(n)] + [w for w in relleno for _ in range(rng.randint(2, 8))]
    rng.shuffle(palabras)
    partes, nueva = [], True
    for w in palabras:
        partes.append(w.capitalize() if nueva else w)
        nueva = False
        r = rng.random()
        if r < 0.08:
            partes[-1] += "."
            nueva = True
        elif r < 0.2:
            partes[-1] += ","
    (c / "texto.txt").write_text(" ".join(partes) + ".\n", encoding="utf-8")
    return {"top": top}


def juez_palabras(lenguaje: str):
    def juez(c: Path, ctx: dict) -> tuple[bool, str]:
        _rc, out = correr_bat(c, 600 if lenguaje == "rust" else 180)
        proyecto = (bat_de(c) or c / "run.bat").parent  # las pruebas se corren donde está el proyecto
        bien = 0
        for w, n in ctx["top"]:
            if any(re.search(rf"\b{re.escape(w)}\b", ln, re.IGNORECASE) and re.search(rf"(?<!\d){n}(?!\d)", ln) for ln in out.splitlines()):
                bien += 1
        if lenguaje == "node":
            cmd = ["cmd", "/c", "npm test --silent"] if (proyecto / "package.json").exists() and '"test"' in (proyecto / "package.json").read_text(encoding="utf-8", errors="replace") else ["node", "--test"]
        elif lenguaje == "go":
            cmd = ["go", "test", "./..."]
        else:
            cmd = ["cargo", "test", "--quiet"]
        rc_t, out_t = ejecutar(cmd, proyecto, 900)
        sin_pruebas = lenguaje == "go" and "no test files" in out_t or "no tests" in out_t.lower() and rc_t == 0 and lenguaje == "node"
        pruebas_ok = rc_t == 0 and not sin_pruebas
        fallos = ([f"palabras correctas {bien}/10"] if bien < 10 else []) + ([] if pruebas_ok else [f"pruebas: rc={rc_t} {out_t[-160:]!r}"])
        return not fallos, "; ".join(fallos) or "10/10 palabras y pruebas pasan"
    return juez


# ------------------------------------------------------------------ 9 escritorio_gastos
def juez_escritorio(c: Path, ctx: dict) -> tuple[bool, str]:
    fallos = []
    if not bat_de(c):
        fallos.append("sin .bat de arranque")
    err, av, _ = verificar_motor(c)
    if err != 0:
        fallos.append(f"verificación del motor: {err} errores")
    return not fallos, "; ".join(fallos) or f"verificación 0 errores, {av} avisos"


# ------------------------------------------------------------------ 10 docker_app
def fx_docker(c: Path) -> dict:
    (c / "app.py").write_text('from flask import Flask\n\napp = Flask(__name__)\n\n\n@app.get("/")\ndef inicio():\n    return "hola desde la app"\n\n\n'
                              'if __name__ == "__main__":\n    app.run(host="0.0.0.0", port=5000)\n', encoding="utf-8")
    (c / "requirements.txt").write_text("flask\n", encoding="utf-8")
    imgs = ejecutar(["docker", "images", "-q"], c, 60)[1].split()
    return {"imagenes_antes": imgs}


def juez_docker(c: Path, ctx: dict) -> tuple[bool, str]:
    d = c / "Dockerfile"
    if not d.exists():
        return False, "no hay Dockerfile"
    rc, out = ejecutar(["docker", "build", "-t", "aura_juez_bench", "."], c, 1200)
    if rc != 0:
        return False, f"docker build falla: {out[-250:]}"
    puerto = (re.search(r"EXPOSE\s+(\d+)", d.read_text(encoding="utf-8", errors="replace")) or [None, "5000"])[1]
    ejecutar(["docker", "rm", "-f", "aura_juez_bench_c"], c, 60)
    rc, out = ejecutar(["docker", "run", "-d", "--name", "aura_juez_bench_c", "-p", f"5098:{puerto}", "aura_juez_bench"], c, 120)
    cuerpo, estado = "", 0
    if rc == 0:
        for _ in range(30):
            try:
                with urllib.request.urlopen("http://127.0.0.1:5098/", timeout=3) as r:
                    estado, cuerpo = r.status, r.read().decode("utf-8", "replace")
                break
            except Exception:
                time.sleep(1)
    ejecutar(["docker", "rm", "-f", "aura_juez_bench_c"], c, 60)
    ejecutar(["docker", "rmi", "-f", "aura_juez_bench"], c, 120)
    ok = estado == 200 and "hola" in cuerpo.lower()
    return ok, f"contenedor responde {estado}" if ok else f"el contenedor no contesta bien (rc={rc}, estado={estado}) {out[-150:]}"


# ------------------------------------------------------------------ 11 evm_token
def juez_evm(c: Path, ctx: dict) -> tuple[bool, str]:
    base = c / "src" if (c / "src").is_dir() else c
    fuentes = [p for p in base.rglob("*.sol") if not {"lib", "node_modules", "out", "cache", "script"} & set(p.parts)
               and "test" not in p.name.lower()]
    if not any(re.search(r"function\s+transfer\b|\bis\s+[^{]*\bERC20\b", p.read_text(encoding="utf-8", errors="replace")) for p in fuentes):
        return False, "ningún contrato con transfer"
    rc, out = ejecutar(["forge", "test"], c, 900)
    pasan = sum(int(n) for n in re.findall(r"(\d+) passed", out))
    return rc == 0 and pasan >= 3, f"forge test rc={rc}, {pasan} pruebas pasan" + ("" if rc == 0 else f" {out[-200:]}")


# ------------------------------------------------------------------ 12 tienda_roles
def juez_tienda(c: Path, ctx: dict) -> tuple[bool, str]:
    err, av, informe = verificar_motor(c)
    return err == 0, f"verificación del motor: {err} errores, {av} avisos" + ("" if err == 0 else f" | {informe[-300:]}")


# ------------------------------------------------------------------ misiones
FERRETERIA = ("Necesito un sistema para mi ferretería. Tengo un administrador y empleados que atienden el mostrador. Quiero registrar "
              "los productos con su costo, su precio de venta y cuántos hay en existencia, y tener una lista de clientes, aunque la "
              "mayoría de las ventas son a público general, sin cliente. Cada venta tiene que descontar las existencias, y quiero ver "
              "el historial de ventas y cuánto se vendió en el día. El empleado solo puede vender: no debe ver costos ni ganancias. "
              "Que funcione en mi computadora con un doble clic.")
PAL = ("En esta carpeta está texto.txt. Quiero un programa en {} que muestre las 10 palabras que más se repiten, sin distinguir "
       "mayúsculas ni signos de puntuación, cada una con cuántas veces aparece. Que lo pueda abrir con doble clic y que tenga sus pruebas.")
MISIONES = {
    "git_conflicto": (20, ("Esta carpeta es un repositorio de git con dos ramas, main y ajustes. Necesito que los cambios de la rama "
                          "ajustes queden también en main, sin perder lo que ya hice en main, y que al final todo esté guardado "
                          "(commit) y limpio."), fx_git, juez_git),
    "bug_py": (30, ("Las pruebas de este proyecto no pasan. Arregla el programa para que pasen, sin tocar las pruebas, y explícame qué "
                   "estaba mal."), fx_bug, juez_bug),
    "csv_ventas": (45, ("En esta carpeta está ventas.csv con mis ventas (fecha, producto, cantidad y precio unitario). Necesito un "
                       "programa que me diga cuánto vendí en total por producto y qué día vendí más dinero. Que lo pueda abrir con "
                       "doble clic y que tenga sus pruebas."), fx_csv, juez_csv),
    "mochila_sat": (40, ("En objetos.csv tengo mis objetos con su peso y su valor. La mochila aguanta 120 de peso en total. Dime "
                        "exactamente cuáles llevar para que el valor sea el más alto posible y asegúrate de que no existe una "
                        "combinación mejor. Escribe el resultado en resultado.txt."), fx_mochila, juez_mochila),
    "investigar_web": (30, ("Necesito saber cuál es la última versión estable de Python, de Node.js y de Rust, y una novedad importante "
                           "de cada una. Escríbelo en informe.md e indica la fuente de cada dato."), fx_nada, juez_investigar),
    "node_palabras": (45, PAL.format("JavaScript con Node"), fx_palabras, juez_palabras("node")),
    "go_palabras": (45, PAL.format("Go"), fx_palabras, juez_palabras("go")),
    "rust_palabras": (60, PAL.format("Rust"), fx_palabras, juez_palabras("rust")),
    "escritorio_gastos": (60, ("Quiero un programita de escritorio para anotar mis gastos del mes con su categoría y ver cuánto llevo "
                              "gastado por categoría. Que lo pueda abrir con doble clic."), fx_nada, juez_escritorio),
    "docker_app": (45, ("Esta aplicación web necesito poder ejecutarla dentro de un contenedor, en cualquier computadora. Prepárala y "
                       "comprueba que arranca bien."), fx_docker, juez_docker),
    "evm_token": (60, ("Necesito un contrato inteligente de un token (como una moneda) con su nombre y símbolo, que se pueda transferir "
                      "entre cuentas, y con pruebas que lo demuestren. Usa Foundry. No lo despliegues en ninguna red."), fx_nada, juez_evm),
    "tienda_roles": (100, FERRETERIA, fx_nada, juez_tienda),
}
