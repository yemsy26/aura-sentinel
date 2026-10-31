"""Métricas de cada turno en .agente/metricas.jsonl: lo que tarda el modelo y cada herramienta y cuánto escribe el
modelo (razonamiento, notas, argumentos de escribir/editar). Solo se anotan: no cambian nada de lo que hace el agente.

    python -m agente.metricas <carpeta del proyecto>...   resume las misiones de esas carpetas
"""
import json
import sys
import time
from collections import Counter
from pathlib import Path

ARCHIVO = "metricas.jsonl"


def anotar(dir_agente: Path, **datos) -> None:
    try:
        with open(dir_agente / ARCHIVO, "a", encoding="utf-8") as f:
            f.write(json.dumps({"t": round(time.time(), 1), **datos}, ensure_ascii=False) + "\n")
    except OSError:
        pass  # medir nunca para una misión


def detalle(nombre: str, args: dict) -> dict:
    """Lo que el modelo tuvo que generar en las llamadas que más escriben."""
    if nombre == "escribir":
        return {"contenido_car": len(str(args.get("contenido", "")))}
    if nombre == "editar":
        return {"buscar_car": len(str(args.get("buscar", ""))), "reemplazar_car": len(str(args.get("reemplazar", "")))}
    return {}


def leer(carpetas: list[str]) -> list[dict]:
    filas = []
    for c in carpetas:
        p = Path(c) / ".agente" / ARCHIVO
        if p.is_file():
            filas += [json.loads(l) for l in p.read_text(encoding="utf-8").splitlines() if l.strip()]
    return filas


def resumen(filas: list[dict]) -> str:
    turnos = [f for f in filas if f.get("tipo") == "turno"]
    llamadas = [f for f in filas if f.get("tipo") == "llamada"]
    compactar = [f for f in filas if f.get("tipo") == "compactar"]
    if not turnos:
        return "Sin métricas (las anotan las misiones desde esta versión)."
    modelo = sum(f["modelo_s"] for f in turnos)
    espera = sum(f.get("espera_s") or 0 for f in turnos)
    herr = sum(f["s"] for f in llamadas)
    comp = sum(f["s"] for f in compactar)
    total = modelo + herr + comp
    salida = sum(f.get("tokens_salida") or 0 for f in turnos)
    car = Counter()
    for f in turnos:
        car["razonamiento"] += f.get("razonamiento_car", 0)
        car["notas"] += f.get("nota_car", 0)
    for f in llamadas:
        car["escribir (contenido)"] += f.get("contenido_car", 0)
        car["editar (texto viejo)"] += f.get("buscar_car", 0)
        car["editar (texto nuevo)"] += f.get("reemplazar_car", 0)
        car["otros argumentos"] += f["args_car"] - f.get("contenido_car", 0) - f.get("buscar_car", 0) - f.get(
            "reemplazar_car", 0)
    suma = sum(car.values()) or 1
    lineas = [f"{len(turnos)} turnos, {len(llamadas)} llamadas: {total / 60:.1f} min en total",
              (f"  modelo {modelo / 60:.1f} min ({100 * modelo / total:.0f} %; esperando el primer token "
               f"{espera / 60:.1f} min), herramientas {herr / 60:.1f} min ({100 * herr / total:.0f} %), "
               f"compactar {len(compactar)} veces {comp / 60:.1f} min"),
              f"  tokens generados: {salida} ({salida / max(modelo - espera, 1):.1f} por segundo generando)",
              "  lo que escribe el modelo (caracteres):"]
    lineas += [f"    {k:22} {v:8} ({100 * v / suma:4.1f} %)" for k, v in car.most_common() if v]
    por = Counter()
    n = Counter()
    for f in llamadas:
        por[f["nombre"]] += f["s"]
        n[f["nombre"]] += 1
    lineas.append("  tiempo de cada herramienta:")
    lineas += [f"    {k:14} {n[k]:4} veces {s / 60:6.1f} min" for k, s in por.most_common(10)]
    return "\n".join(lineas)


if __name__ == "__main__":
    for flujo in (sys.stdout, sys.stderr):
        flujo.reconfigure(encoding="utf-8", errors="replace")
    print(resumen(leer(sys.argv[1:] or ["."])))
