"""Matemáticas exactas para el agente: cálculo simbólico con SymPy y SAT con hunt (complemento Sat-hibrido-rust).

El modelo no calcula de memoria: escribe la fórmula (CNF) o el código SymPy y recibe un resultado comprobado.
"""
import os
import subprocess
import sys
import time
from pathlib import Path

from . import complementos

SONDA_CALCULO = Path(__file__).with_name("sondas") / "calculo.py"
MAX_CNF = 50 * 1024 * 1024  # bytes: más no cabe cómodo en memoria para comprobar el modelo
MAX_SAT = 600  # segundos
MAX_CALCULO = 300
CLAUSULAS_ENJAMBRE = 2000  # con menos, un hilo es más rápido que repartir el trabajo
SIN_SYMPY = 3  # código de salida de la sonda


def ruta_hunt() -> str | None:
    """El resolvedor del complemento Sat-hibrido-rust si está activado (AGENTE_HUNT lo sustituye)."""
    exe = complementos.ruta("sat-hibrido")
    return os.environ.get("AGENTE_HUNT") or (str(exe) if exe else None)


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


def resolver_sat(h, archivo: str, segundos: int = 60, modelo: str = "") -> str:
    exe = ruta_hunt()
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
    try:
        n, clausulas, avisos = leer_dimacs(p.read_text(encoding="utf-8", errors="replace"))
    except ValueError as e:
        return f"DIMACS no válido en {h.rel(p)}: {e}"
    destino = h.ruta(modelo) if modelo else p.with_suffix(".modelo")
    destino.parent.mkdir(parents=True, exist_ok=True)
    destino.unlink(missing_ok=True)
    limite = max(1, min(int(segundos or 60), MAX_SAT))
    orden = [exe, str(p), "--timeout", str(limite), "--model", str(destino)]
    hilos = min(4, (os.cpu_count() or 2) // 2)
    if len(clausulas) >= CLAUSULAS_ENJAMBRE and hilos > 1:
        orden += ["--workers", str(hilos)]
    t0 = time.time()
    try:
        r = subprocess.run(orden, capture_output=True, text=True, encoding="utf-8", errors="replace",
                           timeout=limite + 15, creationflags=subprocess.CREATE_NO_WINDOW, check=False)
        codigo, salida = r.returncode, (r.stdout + r.stderr).strip()
    except subprocess.TimeoutExpired:
        codigo, salida = 124, ""
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
