"""CLI: python -m agente [carpeta] [--tarea "..."] [--json]"""
import argparse
import json
import os
import sys
from pathlib import Path

from . import __version__, config
from . import modelo as cargador
from .bucle import INCOMPLETA, Agente, color
from .llm import ErrorLLM, detectar
from .prompts import LECCIONES, REGLAS_USUARIO

AYUDA = """Comandos: /plan  /contexto  /verificar  /nuevo (olvida el historial)  /salir
  /regla <texto>  corrige una conducta del agente para siempre (p. ej. /regla usa siempre pytest)
  /reglas         muestra las reglas y lecciones guardadas
  /grande <texto> cambio grande por partes (plan por módulos, contexto limpio y commit local por parte)
Texto de varias líneas: empieza y termina con una línea que tenga solo \"\"\" """


def leer_entrada() -> str:
    linea = input(color("\ntú> ", "verde"))
    if linea.strip() != '"""':
        return linea.strip()
    partes = []
    while (l := input("... ")).strip() != '"""':
        partes.append(l)
    return "\n".join(partes).strip()


def modo_json(cfg: dict, carpeta: str) -> int:
    """Protocolo para IDEs: órdenes JSON por stdin y eventos JSON por stdout, uno por línea.

    Órdenes: {"tipo": "tarea"|"regla"|"revisar", "texto": ...}, {"tipo": "nuevo"|"verificar"|"plan"|
    "procesos"|"salir"}, {"tipo": "detener", "id": N}, {"tipo": "restaurar", "numero": N} y
    {"tipo": "respuesta", "si": bool} tras un evento "confirmar" (trae "comando" y "motivo") y
    {"tipo": "respuesta", "texto": "..."} tras un evento "pregunta" (trae "pregunta" y "opciones"). Cada orden acaba con un evento "fin"
    (con "datos" estructurados en "procesos"); mientras tanto llegan "paso", "archivos", "progreso",
    "aviso" y "nota".
    """
    canal, sys.stdout = sys.stdout, sys.stderr  # cualquier print suelto va a stderr: el canal solo lleva eventos
    try:
        return _protocolo(cfg, carpeta, canal)
    finally:
        sys.stdout = canal


def _protocolo(cfg: dict, carpeta: str, canal) -> int:
    sys.stdin.reconfigure(encoding="utf-8", errors="replace")

    def emitir(tipo: str, **datos) -> None:
        canal.write(json.dumps({"tipo": tipo, **datos}, ensure_ascii=False) + "\n")
        canal.flush()

    def leer_ordenes():
        for linea in sys.stdin:
            if linea.strip():
                try:
                    yield json.loads(linea)
                except ValueError:
                    emitir("error", texto=f"Orden JSON inválida: {linea[:200]}")

    ordenes = leer_ordenes()

    def responder(tipo: str, **datos) -> dict:
        emitir(tipo, **datos)
        for orden in ordenes:
            if orden.get("tipo") == "respuesta":
                return orden
            emitir("error", texto=f"Esperaba la respuesta a '{tipo}' y llegó '{orden.get('tipo')}'.")
        return {}

    try:
        if not detectar(cfg)[1]:
            emitir("aviso", texto="No hay modelo cargado: lo cargo con el ajuste de VRAM (puede tardar un minuto) ...")
            cargador.ajustar(cfg, None, None)
        agente = Agente(cfg, Path(carpeta), interactivo=True)
    except ErrorLLM as e:
        emitir("error", texto=str(e))
        return 2
    agente.emitir, agente.responder = emitir, responder
    emitir("listo", version=__version__, modelo=agente.llm.modelo, contexto=agente.llm.contexto,
           servidor=agente.llm.base, proyecto=str(agente.h.raiz), entorno=agente.entorno)
    try:
        for orden in ordenes:
            tipo, texto, estado, datos = orden.get("tipo"), str(orden.get("texto", "")), "ok", None
            if tipo == "salir":
                break
            try:
                if tipo == "tarea":
                    respuesta = agente.tarea(texto)
                    estado = "incompleta" if INCOMPLETA.match(respuesta) else "ok"
                elif tipo == "nuevo":
                    agente.ctx.mensajes = agente.ctx.mensajes[:1]
                    respuesta = "Historial borrado (los archivos y el plan se conservan)."
                elif tipo == "regla":
                    respuesta = agente.nueva_regla(texto)
                elif tipo == "verificar":
                    respuesta = agente.h.verificar_cmd()
                elif tipo == "plan":
                    respuesta = agente.h.plan_texto() or "(sin plan)"
                elif tipo == "revisar":
                    respuesta = agente.h.revisar_cmd(texto)
                elif tipo == "procesos":  # los que el agente lanzó con fondo=true (servidores, juegos...)
                    respuesta = agente.h.procesos_cmd("listar")
                    datos = [{"id": n, "vivo": p["proc"].poll() is None, "comando": p["comando"]}
                             for n, p in agente.h.procesos.items()]
                elif tipo == "detener":
                    respuesta = agente.h.procesos_cmd("detener", int(orden.get("id") or 0))
                elif tipo == "restaurar":  # vuelve a un punto de restauración (0 = el último que funcionaba)
                    respuesta = agente.h.restaurar(int(orden.get("numero") or 0))
                    emitir("archivos", rutas=[])
                else:
                    respuesta, estado = f"Orden desconocida: {tipo}", "error"
            except Exception as e:
                agente.ctx.reparar()
                respuesta, estado = f"Error interno: {type(e).__name__}: {e}", "error"
            emitir("fin", orden=tipo, estado=estado, respuesta=respuesta, uso=agente.ctx.uso(), datos=datos)
    finally:
        agente.cerrar()
    return 0


def main() -> int:
    for flujo in (sys.stdout, sys.stderr):
        flujo.reconfigure(encoding="utf-8", errors="replace")
    os.system("")  # activa colores ANSI en la consola de Windows
    ap = argparse.ArgumentParser(prog="agente", description="Agente de programación local para LM Studio / Bionic")
    ap.add_argument("carpeta", nargs="?", default=".", help="carpeta del proyecto (se crea si no existe)")
    ap.add_argument("--tarea", help="ejecuta una sola tarea y sale")
    ap.add_argument("--razonamiento", choices=["auto", "on", "off"])
    ap.add_argument("--max-pasos", type=int)
    ap.add_argument("--json", action="store_true", help="protocolo de eventos JSON por stdin/stdout (para IDEs)")
    ap.add_argument("--modelo", help="clave del modelo (en vez de la de agente.json); con --json lo carga si falta")
    a = ap.parse_args()

    cfg = config.cargar()
    if a.razonamiento:
        cfg["razonamiento"] = a.razonamiento
    if a.max_pasos:
        cfg["max_pasos"] = a.max_pasos
    if a.modelo:
        cfg["modelo"] = a.modelo
    if a.json:
        return modo_json(cfg, a.carpeta)
    try:
        agente = Agente(cfg, Path(a.carpeta), interactivo=a.tarea is None)
    except ErrorLLM as e:
        print(color(f"[ERROR] {e}", "rojo"))
        return 2

    print(color(f"Agente Local {__version__}", "negrita") +
          color(f"  modelo={agente.llm.modelo}  contexto={agente.llm.contexto}  servidor={agente.llm.base}", "gris"))
    print(color(f"Proyecto: {agente.h.raiz}\nEntorno: {agente.entorno}", "gris"))
    try:
        if a.tarea:
            agente.tarea(a.tarea)
            return 0
        print(color(AYUDA, "gris"))
        while True:
            try:
                texto = leer_entrada()
            except EOFError:
                break
            if not texto:
                continue
            if texto in ("/salir", "/exit", "salir"):
                break
            if texto == "/plan":
                print(agente.h.plan_texto() or "(sin plan)")
            elif texto == "/contexto":
                print(agente.ctx.uso())
            elif texto.startswith("/regla "):
                print(agente.nueva_regla(texto[7:]))
            elif texto == "/reglas":
                for f in (REGLAS_USUARIO, LECCIONES):
                    print(f.read_text(encoding="utf-8") if f.exists() else f"({f.name} vacío)")
            elif texto == "/verificar":
                print(agente.h.verificar_cmd())
            elif texto == "/nuevo":
                agente.ctx.mensajes = agente.ctx.mensajes[:1]
                print("Historial borrado (los archivos y el plan se conservan).")
            elif texto.startswith("/") and not texto.startswith("/grande "):
                print(AYUDA)
            else:
                try:
                    agente.tarea(texto)
                except KeyboardInterrupt:
                    agente.ctx.reparar()
                    print(color("\n  (interrumpido; escribe 'continúa' para seguir)", "amarillo"))
                print(color(f"  [{agente.ctx.uso()}]", "gris"))
    except KeyboardInterrupt:
        pass
    finally:
        agente.cerrar()
    return 0


if __name__ == "__main__":
    sys.exit(main())
