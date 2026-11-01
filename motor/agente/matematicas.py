"""Matemáticas exactas para el agente: cálculo simbólico con SymPy, y SAT y optimización con motor (complemento
Sat-hibrido-rust).

El modelo no calcula de memoria: escribe la fórmula (CNF u OPB) o el código SymPy y recibe un resultado comprobado.
"""
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

from . import complementos

SONDA_CALCULO = Path(__file__).with_name("sondas") / "calculo.py"
MAX_CNF = 50 * 1024 * 1024  # bytes: más no cabe cómodo en memoria para comprobar el modelo
MAX_SAT = 600  # segundos
MAX_CALCULO = 300
SIN_SYMPY = 3  # código de salida de la sonda


def ruta_sat() -> str | None:
    """El resolvedor del complemento Sat-hibrido-rust si está activado (AGENTE_SAT lo sustituye)."""
    exe = complementos.ruta("sat-hibrido")
    return os.environ.get("AGENTE_SAT") or (str(exe) if exe else None)


def leer_dimacs(texto: str) -> tuple[int, list[list[int]], list[str]]:
    """(variables, cláusulas, avisos) de un DIMACS CNF. ValueError con el número de línea si el formato no vale."""
    n = m = None
    clausulas, actual, avisos = [], [], []
    for i, linea in enumerate(texto.splitlines(), 1):
        linea = linea.strip()
        if not linea or linea[0] == "c":
            continue
        if linea[0] == "%":  # fin de fórmula en los archivos de SATLIB
            break
        if linea[0] == "p":
            partes = linea.split()
            if len(partes) != 4 or partes[1] != "cnf" or not (partes[2].isdigit() and partes[3].isdigit()):
                raise ValueError(f"línea {i}: la cabecera debe ser 'p cnf VARIABLES CLAUSULAS' y es {linea[:60]!r}")
            n, m = int(partes[2]), int(partes[3])
            continue
        if n is None:
            raise ValueError(f"línea {i}: falta la cabecera 'p cnf VARIABLES CLAUSULAS' antes de las cláusulas")
        for token in linea.split():
            try:
                literal = int(token)
            except ValueError:
                raise ValueError(f"línea {i}: {token[:20]!r} no es un literal (3 = x3, -3 = no x3, 0 = fin de "
                                 "cláusula)") from None
            if literal:
                actual.append(literal)
            else:
                if not actual:  # haría la fórmula insatisfacible por sí sola: un resultado falso con buena pinta
                    raise ValueError(f"línea {i}: cláusula vacía. 0 no es una variable, marca el fin de cláusula: un script "
                                     "que usa 0 como constante falsa corrompe la fórmula. Una constante es una variable "
                                     "nueva con su cláusula de una sola variable ([-f] para falso)")
                clausulas.append(actual)
                actual = []
    if n is None:
        raise ValueError("falta la cabecera 'p cnf VARIABLES CLAUSULAS'")
    if actual:
        clausulas.append(actual)
        avisos.append("la última cláusula no acaba en 0")
    usada = max((abs(lit) for c in clausulas for lit in c), default=0)
    if usada > n:
        avisos.append(f"la cabecera dice {n} variables pero se usa la {usada}")
    if m != len(clausulas):
        avisos.append(f"la cabecera dice {m} cláusulas y hay {len(clausulas)}")
    return max(n, usada), clausulas, avisos


def _incumplida(clausulas: list[list[int]], modelo: set[int]) -> int | None:
    """Número (desde 1) de la primera cláusula que el modelo no cumple; None si las cumple todas."""
    return next((i for i, c in enumerate(clausulas, 1) if not any(lit in modelo for lit in c)), None)


def leer_opb(texto: str) -> tuple[list[tuple[int, int]], list[tuple[list[tuple[int, int]], str, int]]]:
    """(objetivo, restricciones) de un OPB; un término es (coeficiente, literal) y ~x3 es el literal -3.
    ValueError con el número de restricción si el formato no vale."""
    limpio = "\n".join(l for l in texto.splitlines() if not l.lstrip().startswith("*"))
    partes = limpio.split(";")
    if partes[-1].strip():
        raise ValueError("falta el ';' al final de la última restricción")
    objetivo, restricciones = [], []

    def terminos(trozo: str, donde: str) -> list[tuple[int, int]]:
        if re.sub(r"\s*[+-]?\d+\s+~?x\d+\s*", "", trozo).strip():
            raise ValueError(f"{donde}: cada término es un coeficiente y una variable, como +2 x1 o -1 ~x3: {trozo.strip()[:60]!r}")
        return [(int(c), -int(v) if n else int(v)) for c, n, v in re.findall(r"([+-]?\d+)\s+(~?)x(\d+)", trozo)]

    for parte in partes[:-1]:
        if parte.strip().startswith("min:"):
            objetivo = terminos(parte.strip()[4:], "objetivo")
            continue
        donde = f"restricción {len(restricciones) + 1}"
        m = re.fullmatch(r"(.*?)(>=|<=|=)\s*([+-]?\d+)\s*", parte, re.DOTALL)
        if not m:
            raise ValueError(f"{donde}: debe acabar en >=, <= o = y un número: {parte.strip()[:60]!r}")
        restricciones.append((terminos(m[1], donde), m[2], int(m[3])))
    return objetivo, restricciones


def _valor(terminos: list[tuple[int, int]], modelo: set[int]) -> int:
    """Suma de los términos: ~x3 vale 1 si x3 es falsa, aunque el modelo no nombre x3."""
    return sum(c for c, lit in terminos if (lit in modelo if lit > 0 else -lit not in modelo))


def _ejecutar(orden: list[str], limite: int) -> tuple[int, str, str]:
    try:
        r = subprocess.run(orden, capture_output=True, text=True, encoding="utf-8", errors="replace",
                           timeout=limite + 15, creationflags=subprocess.CREATE_NO_WINDOW, check=False)
        return r.returncode, r.stdout, r.stderr
    except subprocess.TimeoutExpired:
        return 124, "", ""


def _resolver_opb(h, exe: str, p: Path, destino: Path, limite: int) -> str:
    try:
        objetivo, restricciones = leer_opb(p.read_text(encoding="utf-8", errors="replace"))
    except ValueError as e:
        return f"OPB no válido en {h.rel(p)}: {e}"
    t0 = time.time()
    codigo, salida, errores = _ejecutar([exe, "solve", str(p), "--no-verify", "--json", "--timeout", str(limite)], limite)
    tamano = f"{len(restricciones)} restricciones, {time.time() - t0:.1f} s"
    try:
        datos = json.loads(salida.strip().splitlines()[-1])
    except (ValueError, IndexError):
        datos = {}
    if codigo == 20:
        h._anotar(f"resolver_sat {h.rel(p)}", "INSATISFACIBLE", "")
        return (f"INSATISFACIBLE ({tamano}): ninguna asignación cumple todas las restricciones. Si debería tenerla, revisa "
                "la codificación con un caso pequeño de solución conocida.")
    if codigo not in (10, 30, 124):
        h._anotar(f"resolver_sat {h.rel(p)}", f"exit={codigo}", errores)
        return f"ERROR del resolvedor (código {codigo}): {(salida + errores).strip()[-600:] or 'sin salida'}"
    if not datos.get("model"):
        h._anotar(f"resolver_sat {h.rel(p)}", f"TIMEOUT {limite}s", "")
        return (f"SIN RESPUESTA en {limite} s ({tamano}): prueba con más segundos (máx. {MAX_SAT}) o menos variables.")
    literales = set(datos["model"])
    for i, (terminos, op, b) in enumerate(restricciones, 1):
        v = _valor(terminos, literales)
        if not (v >= b if op == ">=" else v <= b if op == "<=" else v == b):
            h._anotar(f"resolver_sat {h.rel(p)}", "ERROR", f"el modelo no cumple la restricción {i}")
            return f"ERROR: el modelo del resolvedor no cumple la restricción {i}. No uses este resultado."
    coste = _valor(objetivo, literales)
    if objetivo and coste != datos.get("cost"):
        h._anotar(f"resolver_sat {h.rel(p)}", "ERROR", f"coste {coste} frente a {datos.get('cost')}")
        return f"ERROR: el modelo tiene coste {coste} y el resolvedor dijo {datos.get('cost')}. No uses este resultado."
    destino.write_text("v " + " ".join(map(str, sorted(literales, key=abs))) + " 0\n", encoding="utf-8")
    verdaderas = sorted(lit for lit in literales if lit > 0)
    lista = " ".join(map(str, verdaderas[:150])) + (f" ... (+{len(verdaderas) - 150})" if len(verdaderas) > 150 else "")
    final = (f"Modelo comprobado contra todas las restricciones y guardado en {h.rel(destino)} (línea 'v': 5 = x5 verdadera, "
             f"-5 = falsa). Variables verdaderas: {lista or 'ninguna'}")
    if codigo == 30:
        h._anotar(f"resolver_sat {h.rel(p)}", "ÓPTIMO", f"coste {coste}, modelo comprobado en {h.rel(destino)}")
        return f"ÓPTIMO: coste {coste}, el menor posible ({tamano}). {final}"
    if codigo == 124 or objetivo:
        h._anotar(f"resolver_sat {h.rel(p)}", f"MEJOR ENCONTRADA {limite}s", f"coste {coste}, sin demostrar")
        return (f"MEJOR ENCONTRADA: coste {coste} en {limite} s, sin demostrar que sea la óptima ({tamano}). Dilo así al "
                f"usuario o prueba con más segundos (máx. {MAX_SAT}). {final}")
    h._anotar(f"resolver_sat {h.rel(p)}", "SATISFACIBLE", f"modelo comprobado en {h.rel(destino)}")
    return f"SATISFACIBLE ({tamano}). {final}"


def resolver_sat(h, archivo: str, segundos: int = 60, modelo: str = "") -> str:
    exe = ruta_sat()
    if not exe:
        return ("El resolvedor SAT es un motor complementario y no está activado (el usuario lo activa en Aura: "
                "Configuración → Motor → Motores complementarios; repositorio "
                f"{complementos.CATALOGO['sat-hibrido']['repo']}). Mientras tanto, resuelve el problema con un script "
                "del proyecto que use python-sat (version('python-sat')).")
    p = h.ruta(archivo)
    if not p.is_file():
        return f"No existe {archivo}. Escribe antes la fórmula en DIMACS CNF (guia('matematicas'))."
    if p.stat().st_size > MAX_CNF:
        return f"{h.rel(p)} pasa de {MAX_CNF // 2**20} MB: reduce la codificación (menos variables auxiliares)."
    destino = h.ruta(modelo) if modelo else p.with_suffix(".modelo")
    destino.parent.mkdir(parents=True, exist_ok=True)
    limite = max(1, min(int(segundos or 60), MAX_SAT))
    if p.suffix.lower() == ".opb":
        destino.unlink(missing_ok=True)
        return _resolver_opb(h, exe, p, destino, limite)
    try:
        n, clausulas, avisos = leer_dimacs(p.read_text(encoding="utf-8", errors="replace"))
    except ValueError as e:
        return f"DIMACS no válido en {h.rel(p)}: {e}"
    destino.unlink(missing_ok=True)
    t0 = time.time()
    # Un hilo: medido con v0.2.0, resuelve más que --workers 4 en las fórmulas difíciles (10 frente a 7 de 18).
    codigo, salida, errores = _ejecutar([exe, "solve", str(p), "--no-verify", "--timeout", str(limite), "--model", str(destino)], limite)
    salida = (salida + errores).strip()
    tamano = f"{n} variables, {len(clausulas)} cláusulas, {time.time() - t0:.1f} s"
    aviso = f"\nAVISO: {'; '.join(avisos)}." if avisos else ""
    if codigo == 10:
        try:
            literales = {int(t) for ln in destino.read_text(encoding="utf-8").splitlines() if ln.startswith("v")
                         for t in ln[1:].split() if t != "0"}
        except (OSError, ValueError):
            literales = set()
        if (mala := _incumplida(clausulas, literales)) is not None:
            h._anotar(f"resolver_sat {h.rel(p)}", "ERROR", f"el modelo no cumple la cláusula {mala}")
            return f"ERROR: el modelo del resolvedor no cumple la cláusula {mala}. No uses este resultado."
        verdaderas = sorted(lit for lit in literales if lit > 0)
        lista = " ".join(map(str, verdaderas[:150])) + (f" ... (+{len(verdaderas) - 150})" if len(verdaderas) > 150 else "")
        h._anotar(f"resolver_sat {h.rel(p)}", "SATISFACIBLE", f"modelo comprobado en {h.rel(destino)}")
        return (f"SATISFACIBLE ({tamano}). Modelo comprobado contra todas las cláusulas y guardado en {h.rel(destino)} "
                f"(línea 'v': 5 = x5 verdadera, -5 = falsa). Variables verdaderas: {lista or 'ninguna'}{aviso}")
    if codigo == 20:
        h._anotar(f"resolver_sat {h.rel(p)}", "INSATISFACIBLE", "")
        return (f"INSATISFACIBLE ({tamano}): ninguna asignación cumple todas las cláusulas, así que el problema no "
                f"tiene solución con esas restricciones. Si debería tenerla, revisa la codificación con un caso pequeño "
                f"de solución conocida.{aviso}")
    if codigo == 124:
        h._anotar(f"resolver_sat {h.rel(p)}", f"TIMEOUT {limite}s", "")
        return (f"SIN RESPUESTA en {limite} s ({n} variables, {len(clausulas)} cláusulas): prueba con más segundos "
                f"(máx. {MAX_SAT}) o una codificación más pequeña (menos variables, restricciones más fuertes).{aviso}")
    h._anotar(f"resolver_sat {h.rel(p)}", f"exit={codigo}", salida)
    return f"ERROR del resolvedor (código {codigo}): {salida[-600:] or 'sin salida'}"


def calcular(h, codigo: str, segundos: int = 30) -> str:
    """Ejecuta 'codigo' (Python con SymPy) en un proceso aparte, con .agente/calculo como carpeta de trabajo."""
    limite = max(2, min(int(segundos or 30), MAX_CALCULO))
    tmp = h.dir_agente / "calculo"
    tmp.mkdir(parents=True, exist_ok=True)
    try:
        r = subprocess.run([sys.executable, "-I", "-X", "utf8", str(SONDA_CALCULO)], input=codigo, capture_output=True,
                           text=True, encoding="utf-8", errors="replace", timeout=limite, cwd=tmp,
                           creationflags=subprocess.CREATE_NO_WINDOW, check=False)
    except subprocess.TimeoutExpired:
        h._anotar("calcular", f"TIMEOUT {limite}s", "")
        return (f"SIN RESPUESTA en {limite} s: simplifica el cálculo, pide un resultado numérico con N() o sube "
                f"segundos (máx. {MAX_CALCULO}).")
    salida = (r.stdout + r.stderr).strip()
    if r.returncode == SIN_SYMPY:
        return salida
    h._anotar("calcular", "OK" if r.returncode == 0 else f"exit={r.returncode}", salida)
    if len(salida) > 4000:
        salida = salida[:4000] + f"\n[... {len(salida) - 4000} caracteres omitidos ...]"
    return salida or "(sin salida: deja la expresión en la última línea o usa print)"
