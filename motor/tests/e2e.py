"""Prueba real con el modelo cargado: el agente debe crear, probar y entregar un proyecto pequeño.
Uso: python tests\\e2e.py [carpeta_destino]
"""
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

RAIZ = Path(__file__).resolve().parent.parent
TAREA = ("Crea un módulo Python calculadora.py con funciones sumar, restar, multiplicar y dividir "
         "(dividir entre cero debe lanzar ValueError con un mensaje claro), tests con unittest en "
         "test_calculadora.py y un test.bat que ejecute los tests. Instala lo que haga falta, prueba todo y entrega.")


def main() -> int:
    destino = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(tempfile.mkdtemp(prefix="agente_e2e_"))
    print(f"Carpeta de prueba: {destino}")
    t0 = time.time()
    rc = subprocess.run([sys.executable, "-m", "agente", str(destino), "--tarea", TAREA], cwd=RAIZ).returncode
    print(f"\nAgente terminó (rc={rc}) en {time.time() - t0:.0f}s. Verificando de forma independiente...")
    fallos = [f for f in ("calculadora.py", "test_calculadora.py", "test.bat") if not (destino / f).exists()]
    if fallos:
        print(f"[FALLO] faltan archivos: {fallos}")
        return 1
    r = subprocess.run("test.bat", shell=True, cwd=destino, capture_output=True, text=True,
                       env=dict(os.environ, AGENTE_NOPAUSE="1"), stdin=subprocess.DEVNULL, timeout=180)
    salida = (r.stdout + r.stderr)[-1500:]
    print(salida)
    if r.returncode != 0:
        print(f"[FALLO] test.bat devolvió {r.returncode}")
        return 1
    print("[OK] El agente creó un proyecto cuyos tests pasan.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
