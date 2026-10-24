"""Prueba rápida de un modelo para el agente: ¿cabe en este equipo?, ¿con qué contexto?, ¿usa bien las herramientas?

Carga el modelo si hace falta (ajustado a la VRAM) y le hace dos peticiones cortas, las mismas que una misión: crear un
archivo con una herramienta y, con el resultado, terminar. No toca ningún proyecto. python -m agente --probar-modelo X"""
import ctypes
import json
import os
import time

from . import compat
from . import modelo as cargador
from .herramientas import ESQUEMAS
from .llm import LLM, ErrorLLM, detectar, info_modelo, listar_modelos

HERRAMIENTAS = [e for e in ESQUEMAS if e["function"]["name"] in ("escribir", "terminar")]
SISTEMA = "Eres un agente de programación. Trabajas con las herramientas: llama a una en cada respuesta."
PEDIDO = "Crea el archivo hola.txt con el texto «hola mundo» usando la herramienta escribir."
LENTO = 5  # tokens/s: por debajo, una misión de 50 pasos se hace eterna


def ram_libre_mib() -> int | None:
    if os.name != "nt":
        return None

    class Estado(ctypes.Structure):
        _fields_ = [("dwLength", ctypes.c_ulong), ("dwMemoryLoad", ctypes.c_ulong), ("ullTotalPhys", ctypes.c_ulonglong),
                    ("ullAvailPhys", ctypes.c_ulonglong), ("ullTotalPageFile", ctypes.c_ulonglong),
                    ("ullAvailPageFile", ctypes.c_ulonglong), ("ullTotalVirtual", ctypes.c_ulonglong),
                    ("ullAvailVirtual", ctypes.c_ulonglong), ("ullAvailExtendedVirtual", ctypes.c_ulonglong)]

    e = Estado(dwLength=ctypes.sizeof(Estado))
    return int(e.ullAvailPhys // 2**20) if ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(e)) else None


def cabida(info: dict, vram: tuple[int, int] | None, ram: int | None, cargado: bool) -> tuple[str, str]:
    """('gpu' | 'mixto' | 'no' | '?', explicación) según el tamaño del modelo y la memoria del equipo."""
    tam = int(info.get("size_bytes") or 0) // 2**20
    if not tam or not vram:
        return "?", "no pude medir la VRAM (sin GPU NVIDIA): no sé si cabe"
    total = vram[1]
    if tam <= total * 0.85:
        return "gpu", f"cabe en la GPU ({tam / 1024:.1f} GB de {total / 1024:.1f} GB)"
    libre = (ram or 0) + (tam if cargado else 0)  # cargado, su parte en RAM ya está ocupada
    if ram is None or tam <= total + libre:
        moe = "; es MoE: sus expertos van a la CPU y aun así va bien" if "moe" in str(info.get("architecture", "")) else ""
        return "mixto", f"no cabe entero en la GPU ({tam / 1024:.1f} GB, {total / 1024:.1f} GB de VRAM): una parte va a la RAM y la CPU, más lento{moe}"
    return "no", (f"pesa {tam / 1024:.1f} GB y entre la VRAM ({total / 1024:.1f} GB) y la RAM libre ({ram / 1024:.1f} GB) "
                  "no hay sitio: puede no cargar o ir lentísimo")


def _args(c: dict) -> dict:
    try:
        return json.loads(c.get("arguments") or "{}")
    except ValueError:
        return {}


def probar(cfg: dict, clave: str, avisar=print) -> dict:
    from .bucle import llamadas_en_texto
    t0 = time.time()
    base = detectar(dict(cfg, modelo="auto"))[0]
    info = next((m for m in listar_modelos(base) if m.get("key") == clave and m.get("type") == "llm"), None)
    if not info:
        return {"modelo": clave, "veredicto": "no apto", "motivos": [f"{clave} no está descargado en Bionic/LM Studio."],
                "limites": [], "detalles": {}}
    cuant = (info.get("quantization") or {}).get("name", "")
    d = {"nombre": info.get("display_name", clave), "tamano_gb": round(int(info.get("size_bytes") or 0) / 2**30, 1),
         "cuantizacion": cuant, "arquitectura": info.get("architecture"), "contexto_max": info.get("max_context_length"),
         **compat.resumen(info)}
    r = {"modelo": clave, "veredicto": "apto", "motivos": [], "limites": [], "detalles": d}
    cfgm = dict(cfg, modelo=clave)
    try:
        cargador.preparar(cfgm, avisar)
        base, instancia, contexto = detectar(cfgm)
        fallo = "" if instancia else "no se pudo cargar (¿memoria insuficiente?)"
    except ErrorLLM as e:
        instancia, fallo = None, f"no se pudo cargar: {e}"
    # después de cargar: antes, el modelo que hubiera cargado ocupaba la RAM y la VRAM que usará este
    sitio, d["memoria"] = cabida(info, cargador.vram(), ram_libre_mib(), bool(instancia))
    if fallo:
        return {**r, "veredicto": "no apto", "motivos": [fallo] + ([d["memoria"]] if sitio == "no" else []),
                "segundos": round(time.time() - t0)}
    d["contexto"] = contexto
    llm = LLM(base, instancia, contexto, cfgm, info_modelo(base, instancia))
    mensajes = [{"role": "system", "content": SISTEMA}, {"role": "user", "content": PEDIDO}]

    def ritmo(resp: dict, t: float) -> None:
        d["respuesta_s"] = round(time.time() - t, 1)
        salida = (resp.get("usage") or {}).get("completion_tokens") or 0
        d["tokens_s"] = round(salida / max(d["respuesta_s"], 0.1), 1) if salida else None

    try:
        t = time.time()
        r1 = llm.chat(mensajes, HERRAMIENTAS, 1024, razonar=False)
        ritmo(r1, t)
        nativas = r1["tool_calls"]
        llamadas = nativas or llamadas_en_texto(r1["content"])
        d["formato"] = "nativo" if nativas else ("texto" if llamadas else "ninguno")
        c = llamadas[0] if llamadas else {}
        args = _args(c)
        d["herramienta_ok"] = c.get("name") == "escribir" and str(args.get("ruta", "")).endswith("hola.txt") \
            and "hola" in str(args.get("contenido", "")).lower()
        if d["herramienta_ok"]:
            c.setdefault("id", "prueba1")
            mensajes += [{"role": "assistant", "content": "", "tool_calls": [
                {"id": c["id"], "type": "function", "function": {"name": c["name"], "arguments": c["arguments"]}}]},
                {"role": "tool", "tool_call_id": c["id"], "content": "OK creado: hola.txt (1 línea)."},
                {"role": "user", "content": "Ya está hecho. Termina la tarea con la herramienta terminar."}]
            t = time.time()
            r2 = llm.chat(mensajes, HERRAMIENTAS, 1024, razonar=False)
            ritmo(r2, t)  # la primera tras cargar aún lee el modelo del disco (Flint: 34 s); esta va al ritmo de una misión
            segunda = r2["tool_calls"] or llamadas_en_texto(r2["content"])
            d["sigue_ok"] = bool(segunda) and segunda[0].get("name") == "terminar"
    except ErrorLLM as e:
        return {**r, "veredicto": "no apto", "motivos": [f"el modelo dio un error al responder: {e}"],
                "segundos": round(time.time() - t0)}
    juzgar(r, contexto, int(cfg["carga"]["contexto_min"]), sitio, info)
    r["segundos"] = round(time.time() - t0)
    return r


def juzgar(r: dict, contexto: int, minimo: int, sitio: str, info: dict) -> None:
    d, motivos, limites = r["detalles"], r["motivos"], r["limites"]
    if not d.get("herramienta_ok"):
        motivos.append("no usó la herramienta como se le pidió (" + {"ninguno": "respondió sin llamarla",
                       "texto": "la escribió como texto y mal", "nativo": "la llamó con datos equivocados"}[d.get("formato", "ninguno")]
                       + "): en una misión no haría nada útil")
    elif not d.get("sigue_ok"):
        limites.append("crea el archivo pero no sigue el hilo con el resultado (no llamó a terminar): se perderá en misiones largas")
    if contexto < minimo:
        motivos.append(f"solo admite {contexto} tokens de contexto y el agente necesita {minimo}")
    elif contexto < 32768:
        limites.append(f"contexto de {contexto} tokens: vale para tareas pequeñas; para proyectos medianos, 32K o más")
    if d.get("formato") == "texto" and d.get("herramienta_ok"):
        limites.append("escribe las llamadas como texto (el servidor no las convierte): el agente las entiende, pero falla más")
    if d.get("tokens_s") is not None and d["tokens_s"] < LENTO:
        limites.append(f"lento en este equipo (~{d['tokens_s']} tokens/s): cada paso de una misión puede tardar minutos")
    if sitio == "no":
        limites.append(d["memoria"])
    limites += [a for a in compat.avisos(info, contexto, minimo) if "contexto" not in a]
    r["veredicto"] = "no apto" if motivos else ("apto con límites" if limites else "apto")
