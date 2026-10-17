"""Servidor MCP 'agente-local': las mismas herramientas del agente para usarlas desde Bionic / LM Studio.

Carpeta inicial: variable AGENTE_RAIZ. El modelo puede cambiarla con la herramienta proyecto(ruta).
"""
import atexit
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
try:  # mcp 2.x renombró FastMCP a MCPServer (misma API de decoradores)
    from mcp.server.mcpserver import MCPServer as FastMCP
except ImportError:
    from mcp.server.fastmcp import FastMCP

from agente.herramientas import ESQUEMAS, Herramientas

D = {e["function"]["name"]: e["function"]["description"] for e in ESQUEMAS}
MAX_CHARS = int(os.environ.get("AGENTE_MAX_CHARS", "6000"))
H = Herramientas(os.environ.get("AGENTE_RAIZ") or Path.home() / "Proyectos", MAX_CHARS)
mcp = FastMCP("agente-local")
atexit.register(lambda: H.detener_todos())


@mcp.tool(description="Muestra la carpeta del proyecto actual o la cambia (ruta absoluta; se crea si no existe). "
                      "Llámala primero con la carpeta que indique el usuario.")
def proyecto(ruta: str = "") -> str:
    global H
    if ruta:
        H.detener_todos()
        H = Herramientas(ruta, MAX_CHARS)
    plan = H.plan_texto()
    return f"Proyecto: {H.raiz}\n{H.listar('.', 1)}" + (f"\nPlan:\n{plan}" if plan else "")


@mcp.tool(description=D["plan"])
def plan(pasos: list[str]) -> str:
    return H.plan(pasos)


@mcp.tool(description=D["paso_hecho"])
def paso_hecho(numero: int, nota: str = "") -> str:
    return H.paso_hecho(numero, nota)


@mcp.tool(description=D["listar"])
def listar(ruta: str = ".", profundidad: int = 2) -> str:
    return H.listar(ruta, profundidad)


@mcp.tool(description=D["leer"])
def leer(ruta: str, desde: int = 1, lineas: int = 200) -> str:
    return H.leer(ruta, desde, lineas)


@mcp.tool(description=D["buscar"])
def buscar(patron: str, ruta: str = ".", extension: str = "") -> str:
    return H.buscar(patron, ruta, extension)


@mcp.tool(description=D["escribir"])
def escribir(ruta: str, contenido: str) -> str:
    return H.escribir(ruta, contenido)


@mcp.tool(description=D["editar"])
def editar(ruta: str, buscar: str, reemplazar: str) -> str:
    return H.editar(ruta, buscar, reemplazar)


@mcp.tool(description=D["ejecutar"])
def ejecutar(comando: str, segundos: int = 120, fondo: bool = False) -> str:
    return H.ejecutar(comando, segundos, fondo)


@mcp.tool(description=D["procesos"])
def procesos(accion: str = "listar", id: int = 0) -> str:
    return H.procesos_cmd(accion, id)


@mcp.tool(description=D["crear_bat"])
def crear_bat(nombre: str, comandos: list[str], descripcion: str = "", usar_venv: bool = False) -> str:
    return H.crear_bat(nombre, comandos, descripcion, usar_venv)


@mcp.tool(description=D["web_buscar"])
def web_buscar(consulta: str) -> str:
    return H.web_buscar(consulta)


@mcp.tool(description=D["web_leer"])
def web_leer(url: str, buscar: str = "") -> str:
    return H.web_leer(url, buscar)


@mcp.tool(description=D["investigar"])
def investigar(pregunta: str) -> str:
    return H.investigar(pregunta)


@mcp.tool(description=D["guia"])
def guia(tema: str) -> str:
    return H.guia(tema)


@mcp.tool(description=D["version"])
def version(paquete: str, ecosistema: str = "pypi") -> str:
    return H.version(paquete, ecosistema)


@mcp.tool(description=D["verificar"])
def verificar(ruta: str = ".", segundos: int = 8) -> str:
    return H.verificar_cmd(ruta, segundos)


def _revisor(requisitos: str, enfoque: str) -> tuple[str, bool]:
    from agente import config, llm
    from agente.revision import revisar as revisar_codigo
    cfg = config.cargar()
    base, modelo, ctx = llm.detectar(cfg)
    if not modelo:
        return "Revisión no disponible: no hay modelo cargado.", True
    return revisar_codigo(llm.LLM(base, modelo, ctx, cfg), H.raiz, requisitos, H.verificacion["informe"], enfoque,
                          H.evidencia_texto())


@mcp.tool(description=D["revisar"] + " En 'requisitos' copia lo que pidió el usuario.")
def revisar(requisitos: str, enfoque: str = "") -> str:
    H.revisar_cb = _revisor
    return H.revisar_cmd(requisitos, enfoque)


@mcp.tool(description=D["restaurar"])
def restaurar(checkpoint: int = 0) -> str:
    return H.restaurar(checkpoint)


@mcp.tool(description=D["calcular"])
def calcular(codigo: str, segundos: int = 30) -> str:
    return H.calcular(codigo, segundos)


@mcp.tool(description=D["resolver_sat"])
def resolver_sat(archivo: str, segundos: int = 60, modelo: str = "") -> str:
    return H.resolver_sat(archivo, segundos, modelo)


@mcp.tool(description=D["aprender"])
def aprender(leccion: str) -> str:
    return H.aprender(leccion)


@mcp.tool(description=D["ver_pantalla"])
def ver_pantalla(ruta: str = "", pregunta: str = "", segundos: int = 2) -> str:
    return H.ver_pantalla(ruta, pregunta, segundos)


@mcp.tool(description=D["git"] + " Sin consola para confirmar: lo destructivo se bloquea.")
def git(accion: str = "estado", mensaje: str = "", nombre: str = "", ruta: str = "", comando: str = "") -> str:
    return H.git(accion, mensaje, nombre, ruta, comando)


if __name__ == "__main__":
    mcp.run()
