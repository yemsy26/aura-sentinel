"""Ejecuta un cálculo con SymPy leído de la entrada estándar y escribe el resultado (lo lanza matematicas.calcular).

Solo cálculo: rechaza importar módulos fuera de la lista, abrir archivos, eval/exec, atributos internos (_x) y textos
con nombres del sistema. No es una barrera de seguridad: evita que un cálculo toque el disco o la red sin pasar por los
permisos del agente (para eso está ejecutar).
"""
import ast
import re
import sys
import traceback

PERMITIDOS = {"sympy", "mpmath", "math", "cmath", "fractions", "decimal", "itertools", "functools", "statistics",
              "numbers", "random", "collections", "operator", "heapq", "bisect", "numpy", "scipy", "networkx"}
PROHIBIDOS = {"open", "exec", "eval", "compile", "__import__", "globals", "locals", "vars", "getattr", "setattr",
              "delattr", "input", "breakpoint", "exit", "quit", "help", "memoryview"}
# sympify("...") evalúa el texto como código: los textos no pueden nombrar el sistema.
TEXTO_PROHIBIDO = re.compile(r"__|\b(open|exec|eval|compile|import|os|sys|subprocess|shutil|socket|system|popen|"
                             r"remove|unlink|rmdir|urllib|requests|write)\b")
FILA = "<calculo>"


def motivo_rechazo(arbol: ast.AST) -> str | None:
    for nodo in ast.walk(arbol):
        if isinstance(nodo, (ast.Import, ast.ImportFrom)):
            modulos = [a.name for a in nodo.names] if isinstance(nodo, ast.Import) else [nodo.module or ""]
            for m in modulos:
                if m.split(".")[0] not in PERMITIDOS:
                    return f"importar '{m}' (permitidos: {', '.join(sorted(PERMITIDOS))})"
        elif isinstance(nodo, ast.Name) and (nodo.id in PROHIBIDOS or nodo.id.startswith("__")):
            return f"usar '{nodo.id}'"
        elif isinstance(nodo, ast.Attribute) and nodo.attr.startswith("_"):
            return f"usar el atributo interno '{nodo.attr}'"
        elif isinstance(nodo, ast.Constant) and isinstance(nodo.value, str) and TEXTO_PROHIBIDO.search(nodo.value):
            return f"el texto {nodo.value[:40]!r}"
    return None


def main() -> int:
    codigo = sys.stdin.read()
    try:
        arbol = ast.parse(codigo, FILA)
    except SyntaxError as e:
        print(f"ERROR DE SINTAXIS línea {e.lineno}: {e.msg}")
        return 1
    if (motivo := motivo_rechazo(arbol)):
        print(f"RECHAZADO: {motivo}. calcular solo hace cálculo (SymPy, math, fractions...); para otra cosa escribe un "
              "script y lánzalo con ejecutar.")
        return 2
    try:
        import sympy  # noqa: F401  (comprueba que está instalado)
    except ImportError:
        print("Falta SymPy en el entorno del motor: en Aura, Configuración → Motor → Instalar motor (o instalar.bat).")
        return 3
    espacio = {"__name__": "__calculo__"}
    exec("from sympy import *\nx, y, z, t = symbols('x y z t')", espacio)  # noqa: S102  (código fijo)
    ultima = arbol.body.pop() if arbol.body and isinstance(arbol.body[-1], ast.Expr) else None
    try:
        exec(compile(arbol, FILA, "exec"), espacio)  # noqa: S102
        if ultima is not None:
            valor = eval(compile(ast.Expression(ultima.value), FILA, "eval"), espacio)
            if valor is not None:
                print(valor)
    except Exception as e:
        linea = next((f.lineno for f in reversed(traceback.extract_tb(e.__traceback__)) if f.filename == FILA), None)
        print(f"ERROR{f' en la línea {linea}' if linea else ''}: {type(e).__name__}: {e}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
