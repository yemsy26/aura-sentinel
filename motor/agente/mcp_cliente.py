"""Cliente MCP por stdio para los servidores que el USUARIO configura en <instalación>/mcp.json.

La configuración nunca sale de la carpeta del proyecto: un repositorio ajeno no puede hacer que Aura ejecute nada. Habla el
protocolo 2026-07-28 (sin saludo: cada petición lleva _meta y `server/discover` sirve de sondeo) y, si el servidor es antiguo
(contesta otro error o calla), `initialize` de 2025-11-25. Lo que devuelve un servidor son datos externos.
"""
import json
import os
import queue
import shutil
import subprocess
import threading
import time
from collections import deque
from pathlib import Path

from . import procesos
from .config import RAIZ_INSTALACION
from .esquemas import esquema_externo

ARCHIVO = RAIZ_INSTALACION / "mcp.json"
MODERNA, ANTIGUA = "2026-07-28", "2025-11-25"
VERSION_NO_ADMITIDA = -32022
PLAZO_SONDEO, PLAZO_ARRANQUE, PLAZO_LISTAR, PLAZO_LLAMADA = 15, 60, 30, 120
CLIENTE = {"name": "aura-sentinel", "version": "1"}
META = {"io.modelcontextprotocol/protocolVersion": MODERNA, "io.modelcontextprotocol/clientInfo": CLIENTE,
        "io.modelcontextprotocol/clientCapabilities": {}}


class ErrorMCP(Exception):
    pass


def configurados(archivo: Path) -> tuple[dict, str]:
    """({nombre: definición}, aviso para el usuario). Formato de siempre: {"mcpServers": {nombre: {command, args, env}}}."""
    if not archivo.is_file():
        return {}, ""
    try:
        servidores = json.loads(archivo.read_text(encoding="utf-8-sig"))["mcpServers"]
        activos = {n: d for n, d in servidores.items() if not d.get("disabled")}
        validos = {n: d for n, d in activos.items() if isinstance(d.get("command"), str) and d["command"]}
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as e:
        return {}, f"mcp.json no se pudo leer ({type(e).__name__}: {e}); sigo sin servidores MCP."
    sobran = sorted(set(activos) - set(validos))
    return validos, (f"mcp.json: sin «command», no se usan (Aura solo habla stdio): {', '.join(sobran)}." if sobran else "")


def texto_de(resultado: dict) -> str:
    partes = []
    for c in resultado.get("content") or []:
        tipo = c.get("type")
        if tipo == "text":
            partes.append(str(c.get("text", "")))
        elif tipo == "resource":
            recurso = c.get("resource") or {}
            partes.append(str(recurso.get("text") or f"[recurso {recurso.get('uri')}]"))
        elif tipo == "resource_link":
            partes.append(f"[enlace {c.get('uri')}]")
        else:
            partes.append(f"[{tipo} {c.get('mimeType', '')}: no se muestra]")
    if not partes and resultado.get("structuredContent") is not None:
        partes.append(json.dumps(resultado["structuredContent"], ensure_ascii=False))
    return "\n".join(partes) or "(sin contenido)"


def _bombear(flujo, destino) -> None:
    with flujo:
        for linea in flujo:
            destino(linea.decode("utf-8", "replace"))
    destino(None)


class Servidor:
    def __init__(self, definicion: dict, proyecto: Path):
        self.d, self.proyecto = definicion, proyecto
        self.proc, self.trabajo = None, None
        self.moderno = True
        self._id = 0
        self._hilos: list = []
        self._entrada: queue.Queue = queue.Queue()
        self._errores: deque = deque(maxlen=8)

    def _valor(self, texto) -> str:
        return str(texto).replace("${proyecto}", str(self.proyecto))

    def _pista(self) -> str:
        ultimo = " | ".join(l for l in self._errores if l)[-300:]
        return f" (stderr: {ultimo})" if ultimo else ""

    def _arrancar(self) -> None:
        self.cerrar()
        entorno = {**os.environ, **{k: self._valor(v) for k, v in (self.d.get("env") or {}).items()}}
        orden = self._valor(self.d["command"])
        exe = shutil.which(orden, path=entorno.get("PATH"))
        if not exe:
            raise ErrorMCP(f"no encuentro el programa «{orden}»")
        exe = Path(exe).resolve()
        if exe.is_relative_to(self.proyecto):  # en Windows which mira antes la carpeta actual
            raise ErrorMCP(f"«{orden}» está dentro del proyecto ({exe}): no lo ejecuto como servidor")
        args = [self._valor(a) for a in self.d.get("args") or []]
        if exe.suffix.lower() in (".bat", ".cmd") and any(set(a) & set('&|<>^%"') for a in args):
            raise ErrorMCP("un argumento tiene caracteres que cmd interpretaría como órdenes")
        try:
            self.proc, self.trabajo = procesos.lanzar(
                [str(exe), *args], cwd=self._valor(self.d.get("cwd") or RAIZ_INSTALACION), env=entorno,
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        except OSError as e:
            raise ErrorMCP(f"no pude arrancar «{orden}»: {e}") from e
        self._entrada, self._errores = queue.Queue(), deque(maxlen=8)
        errores = self._errores
        self._hilos = [threading.Thread(target=_bombear, args=(flujo, destino), daemon=True)
                       for flujo, destino in ((self.proc.stdout, self._entrada.put),
                                              (self.proc.stderr, lambda l: errores.append((l or "").strip())))]
        for hilo in self._hilos:
            hilo.start()

    def _enviar(self, mensaje: dict) -> None:
        try:
            self.proc.stdin.write((json.dumps({"jsonrpc": "2.0", **mensaje}, ensure_ascii=False) + "\n").encode("utf-8"))
            self.proc.stdin.flush()
        except (OSError, ValueError) as e:
            raise ErrorMCP(f"el servidor cerró la entrada{self._pista()}") from e

    def _pedir(self, metodo: str, params: dict, plazo: float) -> dict:
        """La respuesta JSON-RPC completa (con result o error). ErrorMCP si el servidor muere o no contesta a tiempo."""
        self._id += 1
        rid = self._id
        self._enviar({"id": rid, "method": metodo, "params": {**params, "_meta": META} if self.moderno else params})
        fin = time.monotonic() + plazo
        while True:
            try:
                linea = self._entrada.get(timeout=max(0.0, fin - time.monotonic()))
            except queue.Empty:
                if metodo != "server/discover":
                    self._enviar({"method": "notifications/cancelled", "params": {"requestId": rid}})
                raise ErrorMCP(f"sin respuesta a {metodo} en {plazo:g} s{self._pista()}") from None
            if linea is None:  # cerró su salida (haya salido o no): la siguiente llamada arranca uno nuevo
                self.cerrar()
                raise ErrorMCP(f"el servidor terminó{self._pista()}")
            try:
                m = json.loads(linea)
            except ValueError:
                continue  # registros escritos por error en stdout
            if not isinstance(m, dict):
                continue
            if "method" in m:
                if "id" in m:  # un servidor antiguo puede pedir cosas al cliente; no ofrecemos ninguna
                    respuesta = {"result": {}} if m["method"] == "ping" else {"error": {"code": -32601, "message": "no"}}
                    self._enviar({"id": m["id"], **respuesta})
                continue
            if m.get("id") == rid:
                return m

    def _resultado(self, metodo: str, params: dict, plazo: float) -> dict:
        r = self._pedir(metodo, params, plazo)
        if "error" in r:
            error = r["error"] or {}
            raise ErrorMCP(f"{metodo}: {error.get('message')} (código {error.get('code')})")
        return r.get("result") or {}

    def conectar(self) -> None:
        if self.proc and self.proc.poll() is None:
            return
        self._arrancar()
        self.moderno = True
        try:
            r = self._pedir("server/discover", {}, PLAZO_SONDEO)
        except ErrorMCP:
            if not self.proc:  # algún servidor antiguo se cae ante lo que no conoce
                self._arrancar()
        else:
            if "result" in r:
                versiones = (r["result"] or {}).get("supportedVersions") or []
                if MODERNA in versiones:
                    return
                if not any(v <= ANTIGUA for v in versiones):
                    raise ErrorMCP(f"solo habla versiones que Aura no conoce: {versiones}")
            elif (r["error"] or {}).get("code") == VERSION_NO_ADMITIDA:
                raise ErrorMCP(f"no admite la versión {MODERNA}: {(r['error'].get('data') or {}).get('supported')}")
        self.moderno = False
        self._resultado("initialize", {"protocolVersion": ANTIGUA, "capabilities": {}, "clientInfo": CLIENTE}, PLAZO_ARRANQUE)
        self._enviar({"method": "notifications/initialized"})

    def herramientas(self) -> list[dict]:
        self.conectar()
        todas, cursor = [], None
        for _ in range(20):
            r = self._resultado("tools/list", {"cursor": cursor} if cursor else {}, PLAZO_LISTAR)
            todas += [t for t in r.get("tools") or [] if isinstance(t, dict) and t.get("name")]
            cursor = r.get("nextCursor")
            if not cursor:
                break
        return todas

    def llamar(self, herramienta: str, argumentos: dict) -> tuple[bool, str]:
        """(es un error, texto)."""
        self.conectar()
        r = self._resultado("tools/call", {"name": herramienta, "arguments": argumentos}, PLAZO_LLAMADA)
        if r.get("resultType") == "input_required":
            return True, "el servidor pide datos al usuario a mitad de la llamada, y Aura todavía no lo admite."
        return bool(r.get("isError")), texto_de(r)

    def cerrar(self) -> None:
        """Como pide el protocolo: cerrar la entrada, esperar y, si no sale, terminar su árbol de procesos."""
        if not self.proc:
            return
        try:
            self.proc.stdin.close()
            self.proc.wait(timeout=3)
        except (OSError, subprocess.TimeoutExpired):
            pass
        procesos.detener(self.proc, self.trabajo)
        if self.trabajo:
            self.trabajo.cerrar()
        for hilo in self._hilos:
            hilo.join(timeout=1)  # que llegue lo último que escribió en stderr
        self.proc, self.trabajo = None, None


class Gestor:
    def __init__(self, proyecto: Path, archivo: Path | None = None):
        self.definiciones, self.aviso = configurados(archivo or ARCHIVO)
        self.proyecto = proyecto
        self._abiertos: dict[str, Servidor] = {}
        self.propias: dict[str, tuple[str, str]] = {}  # herramienta ofrecida al modelo -> (servidor, nombre en él)

    def ofrecer(self, presupuesto: int) -> tuple[list, list[str]]:
        """Arranca los servidores y da sus herramientas como esquemas propios, más lo que no se pudo ofrecer así.
        presupuesto = caracteres que pueden ocupar los esquemas."""
        esquemas, problemas = [], []
        for servidor in self.nombres:
            try:
                herramientas = self.servidor(servidor).herramientas()
            except Exception as e:  # un servidor raro no puede tumbar la tarea
                problemas.append(f"{servidor}: {e}")
                continue
            fuera = []
            for h in herramientas:
                esquema = esquema_externo(servidor, h["name"], h.get("description"), h.get("inputSchema"))
                nombre, ocupa = esquema["function"]["name"], len(json.dumps(esquema, ensure_ascii=False))
                if nombre in self.propias or ocupa > presupuesto:
                    fuera.append(str(h["name"]))
                    continue
                presupuesto -= ocupa
                self.propias[nombre] = (servidor, h["name"])
                esquemas.append(esquema)
            if fuera:
                problemas.append(f"{servidor}: no caben o repiten nombre: {', '.join(fuera)}"[:200])
        return esquemas, problemas

    @property
    def nombres(self) -> list[str]:
        return list(self.definiciones)

    def servidor(self, nombre: str) -> Servidor:
        if nombre not in self.definiciones:
            raise ErrorMCP(f"no existe el servidor «{nombre}»; configurados: {', '.join(self.nombres) or 'ninguno'}")
        if nombre not in self._abiertos:
            self._abiertos[nombre] = Servidor(self.definiciones[nombre], self.proyecto)
        return self._abiertos[nombre]

    def pide_permiso(self, nombre: str) -> bool:
        return self.definiciones[nombre].get("confirmar", True) is not False

    def cerrar(self) -> None:
        for servidor in self._abiertos.values():
            servidor.cerrar()
        self._abiertos.clear()
