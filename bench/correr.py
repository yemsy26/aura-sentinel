"""Banco de evaluación de Aura: lanza las 12 misiones con un modelo local, las juzga y aplica las reglas Q1/Q2 (ver README.md).

  python bench/correr.py                       las 12 con el motor del repo y Qwen3.6 (unas 2 h de GPU)
  python bench/correr.py bug_py csv_ventas     solo esas
  python bench/correr.py --motor RUTA          otra copia del motor (p. ej. congelada con git archive)
  python bench/correr.py --modelo ID           otro modelo (se descarga el actual y se carga ese)
  python bench/correr.py --seco                fixtures y jueces sobre el estado de partida, sin modelo
  python bench/correr.py --rejuzgar            solo los jueces, sobre carpetas ya hechas
  python bench/correr.py --resumen X.jsonl [--contra Y.jsonl]    Q1/Q2 de un resultado, con la comparación si se da otro
"""
import argparse
import collections
import json
import math
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import misiones as M

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
REPO = Path(__file__).resolve().parent.parent
MODELO = "qwen3.6-35b-a3b"
ERR = re.compile(r"^(exit=[1-9-]|ERROR|TIMEOUT|RECHAZADO|Argumentos|No encontré|No existe|BLOQUEADO|Herramienta desconocida|"
                 r"Sin cambios|'buscar' aparece|VERIFICACIÓN \([^)]*\): [1-9])")
SIN_JUEZ_EN_SECO = ("docker_app", "evm_token", "investigar_web", "tienda_roles", "escritorio_gastos")  # piden red, docker, foundry o una app
HERRAMIENTAS = ["plan", "paso_hecho", "listar", "leer", "buscar", "escribir", "editar", "ejecutar", "procesos", "crear_bat", "web_buscar", "web_leer", "investigar", "calcular", "resolver_sat", "guia", "version", "verificar", "ver_pantalla", "revisar", "restaurar", "aprender", "experimentar", "abrir_android_studio", "git", "preguntar", "terminar"]


def veredicto(termina: bool, juez_ok: bool, traceback: bool = False) -> str:
    """Q1: FUNCIONA = termina (FIN) y el juez pasa; PARCIAL = una de las dos; FALLA = ninguna, o proceso caído."""
    if traceback or (not termina and not juez_ok):
        return "FALLA"
    return "FUNCIONA" if termina and juez_ok else "PARCIAL"


def q2(funcionan: int, total: int = 12) -> str:
    """Q2: ≥ 10 de 12 → base sólida; 7-9 → corregir antes de añadir nada; ≤ 6 → no pasar a mejoras nuevas (proporcional si son menos)."""
    if funcionan >= math.ceil(total * 10 / 12):
        return "base sólida: se puede pasar a lo que le falta al agente"
    if funcionan >= math.ceil(total * 7 / 12):
        return "hay fallos concretos que corregir antes, listados por frecuencia"
    return "no pasar a mejoras nuevas: la base falla"


def metricas(c: Path, salida: str) -> dict:
    reg, met = c / ".agente" / "registro.log", c / ".agente" / "metricas.jsonl"
    log = reg.read_text(encoding="utf-8", errors="replace") if reg.is_file() else ""
    turnos = []
    for ln in (met.read_text(encoding="utf-8").splitlines() if met.is_file() else []):
        try:
            r = json.loads(ln)
        except ValueError:
            continue
        if r.get("tipo") == "turno":
            turnos.append(r)
    llam = collections.defaultdict(lambda: [0, 0])
    for m in re.finditer(r"^\[[\d:]+\] (\w+) .*? -> (.*)$", log, re.MULTILINE):
        llam[m.group(1)][0] += 1
        llam[m.group(1)][1] += bool(ERR.match(m.group(2)))
    det = re.search(r"Detenido: el modelo repitió '(\w+)'", salida + log)
    ult = (re.findall(r"VERIFICACIÓN \([^)]*\): (\d+) ERRORES, (\d+) AVISOS", log) or [("?", "?")])[-1]
    return {"pasos": max((t["paso"] for t in turnos), default=0), "termina": "FIN:" in log,
            "maximo": re.findall(r"Se alcanzó el máximo de (\d+)", salida), "repitio": det.group(1) if det else "",
            "traceback": "Traceback (most recent call last)" in salida, "llamadas": dict(sorted(llam.items())),
            "ultimo_verificar": list(ult)}


def leer(archivo: Path) -> list[dict]:
    return [json.loads(ln) for ln in archivo.read_text(encoding="utf-8").splitlines() if ln.strip()]


def resumen(archivo: Path, contra: Path | None = None) -> None:
    filas = leer(archivo)
    previo = {r["mision"]: r for r in leer(contra)} if contra else {}
    print(f"{'misión':18} {'veredicto':9} {'FIN':4} {'pasos':>5} {'min':>6}  juez" + ("   (contra)" if contra else ""))
    for r in filas:
        v = veredicto(r["termina"], r["juez_ok"], r.get("traceback", False))
        antes = ""
        if contra:
            p = previo.get(r["mision"])
            antes = "   (nueva)" if not p else f"   ({veredicto(p['termina'], p['juez_ok'], p.get('traceback', False))})"
        print(f"{r['mision']:18} {v:9} {'sí' if r['termina'] else 'no':4} {r['pasos']:5} {r['min']:6}  {str(r['juez'])[:70]}{antes}")
    n = sum(veredicto(r["termina"], r["juez_ok"], r.get("traceback", False)) == "FUNCIONA" for r in filas)
    print(f"\nQ2: {n} de {len(filas)} FUNCIONA → {q2(n, len(filas))}")
    tot, err = collections.Counter(), collections.Counter()
    for r in filas:
        for k, (a, e) in (r.get("llamadas") or {}).items():
            tot[k] += a
            err[k] += e
    q3 = [f"{k} ({err[k]}/{tot[k]} errores)" for k in tot if tot[k] >= 8 and err[k] / tot[k] >= 0.25]
    sin_uso = [k for k in HERRAMIENTAS if not tot[k]]
    print(f"Q3: con ≥ 25 % de error: {', '.join(q3) or 'ninguna'}; nunca llamadas: {', '.join(sin_uso) or 'ninguna'}")


def cargar_modelo(modelo: str, motor: Path) -> None:
    M.ejecutar([str(M.PY), "-m", "agente.modelo", "descargar"], motor, 120)
    usada = int(subprocess.run(["nvidia-smi", "--query-gpu=memory.used", "--format=csv,noheader,nounits"],
                               capture_output=True, text=True, check=False).stdout.split()[0])
    if usada > 600:
        sys.exit(f"GPU ocupada ({usada} MiB) tras descargar: paro")
    _, out = M.ejecutar([str(M.PY), "-m", "agente.modelo", "cargar", "--modelo", modelo], motor, 600)
    print(out[-400:], flush=True)
    sys.path.insert(0, str(motor))
    from agente import config, llm
    _, cargado, ctx = llm.detectar(config.cargar())
    if not cargado or modelo.split("-")[0] not in cargado.lower():
        sys.exit(f"No quedó cargado {modelo} (cargado: {cargado})")
    print(f"Modelo cargado: {cargado} contexto={ctx}", flush=True)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("claves", nargs="*", help="misiones a correr (por defecto, las 12)")
    ap.add_argument("--motor", type=Path, default=REPO / "motor")
    ap.add_argument("--modelo", default=MODELO)
    ap.add_argument("--salida", type=Path, default=REPO.parent / "aura-bench", help="carpeta de las misiones y de los resultados")
    ap.add_argument("--prefijo", default="bench", help="las carpetas se llaman <prefijo>_<misión>[_r<N>]")
    ap.add_argument("--rep", help="número de repetición (sufijo _rN de las carpetas)")
    ap.add_argument("--resume", action="store_true", help="salta las misiones ya anotadas en <prefijo>.jsonl")
    ap.add_argument("--seco", action="store_true")
    ap.add_argument("--rejuzgar", action="store_true")
    ap.add_argument("--sin-cargar", action="store_true", help="no cargar el modelo: ya está cargado")
    ap.add_argument("--resumen", type=Path)
    ap.add_argument("--contra", type=Path)
    a = ap.parse_args()
    if a.resumen:
        return resumen(a.resumen, a.contra)
    claves = a.claves or list(M.MISIONES)
    desconocidas = [k for k in claves if k not in M.MISIONES]
    if desconocidas:
        sys.exit(f"Misiones desconocidas: {desconocidas}. Las hay: {', '.join(M.MISIONES)}")
    salida = a.salida.resolve()
    base = Path(tempfile.mkdtemp(prefix="bench_seco_")) if a.seco or a.rejuzgar else None  # sin tocar la carpeta de resultados
    M.iniciar(a.motor, base / "tmp" if base else salida / "tmp")
    resultados = salida / f"{a.prefijo}.jsonl"
    nombre = lambda k: salida / (f"{a.prefijo}_{k}" + (f"_r{a.rep}" if a.rep else ""))
    if a.resume and resultados.exists():
        hechas = {r["mision"] for r in leer(resultados)}
        claves = [k for k in claves if k not in hechas]
    if base:
        for k in claves:
            tope, mandato, fx, juez = M.MISIONES[k]
            nuevo = base / k
            nuevo.mkdir()
            ctx = fx(nuevo)  # el contexto del juez (totales esperados, hashes, imágenes previas) sale del fixture
            if a.rejuzgar:
                c = nombre(k)
                if not c.is_dir():
                    print(f"{k}: no existe {c}", flush=True)
                    continue
            else:
                c = nuevo
                if k in SIN_JUEZ_EN_SECO:
                    print(f"{k}: fixture ok {sorted(p.name for p in c.iterdir())} (el juez no se ejecuta en seco)", flush=True)
                    continue
            try:
                ok, det = juez(c, ctx)
            except Exception as e:
                ok, det = False, f"el juez falló: {type(e).__name__}: {e}"
            print(f"{k}: juez -> {ok}: {det[:140]}", flush=True)
        return None
    assert (M.MOTOR / "agente" / "bucle.py").exists(), M.MOTOR
    for k in claves:
        if nombre(k).exists() and any(nombre(k).iterdir()):
            sys.exit(f"No está vacía: {nombre(k)}")
    salida.mkdir(parents=True, exist_ok=True)
    if not a.sin_cargar:
        cargar_modelo(a.modelo, M.MOTOR)
    for k in claves:
        tope, mandato, fx, juez = M.MISIONES[k]
        c = nombre(k)
        c.mkdir(parents=True, exist_ok=True)
        try:
            ctx = fx(c)
        except Exception as e:
            print(f"{k}: fallo al crear el fixture: {e}", flush=True)
            continue
        print(f"### {k} (tope {tope} min) {time.strftime('%H:%M:%S')}", flush=True)
        t0, parada = time.time(), ""
        try:
            r = subprocess.run([str(M.PY), "-m", "agente", str(c), "--tarea", mandato, "--modelo", a.modelo], cwd=M.MOTOR, env=M.ENV,
                               stdin=subprocess.DEVNULL, capture_output=True, text=True, encoding="utf-8", errors="replace",
                               timeout=tope * 60, check=False)
            texto = r.stdout + "\n" + r.stderr[-3000:]
        except subprocess.TimeoutExpired as e:
            texto = (e.stdout.decode("utf-8", "replace") if isinstance(e.stdout, bytes) else (e.stdout or "")) + f"\n[CORTADA a los {tope} min]"
            parada = f"cortada a los {tope} min"
        (salida / f"{c.name}_salida.txt").write_text(texto, encoding="utf-8")
        res = {"mision": k, "min": round((time.time() - t0) / 60, 1), "parada": parada} | metricas(c, texto)
        try:
            ok, det = juez(c, ctx)
        except Exception as e:
            ok, det = False, f"el juez falló: {type(e).__name__}: {e}"
        res |= {"juez_ok": ok, "juez": det, "veredicto": veredicto(res["termina"], ok, res["traceback"])}
        if k == "docker_app":
            res["imagenes_nuevas"] = len(set(M.ejecutar(["docker", "images", "-q"], c, 60)[1].split()) - set(ctx["imagenes_antes"]))
        print(json.dumps(res, ensure_ascii=False), flush=True)
        with resultados.open("a", encoding="utf-8") as fh:
            fh.write(json.dumps(res, ensure_ascii=False) + "\n")
    print(f"HECHO ({a.modelo} queda cargado)", flush=True)
    return resumen(resultados)


if __name__ == "__main__":
    main()
