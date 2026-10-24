"""Abre la app Tauri de verdad (el .exe de cargo build) unos segundos y mira su ventana: texto, consola y captura.

Antes solo se compilaba y se probaba el frontend en un navegador, donde invoke no existe: «0 errores» con una app que el
usuario no conseguía abrir, y capturas sin datos que llevaban al agente a «arreglar» lo que funcionaba."""
import importlib.util
import json
import os
import socket
import subprocess
import threading
import time
import urllib.parse
import urllib.request
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import ClassVar

from . import procesos

ESPERA_VENTANA = 30  # s hasta que WebView2 abre el puerto de depuración (la primera vez prepara su perfil)
FUENTES_WEB = (".html", ".htm", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".vue", ".svelte", ".css")
_resultados: dict[str, tuple[tuple, dict]] = {}  # exe -> (huella, resultado): sin cambios no se vuelve a abrir


class _Estaticos(SimpleHTTPRequestHandler):
    # El registro de Windows puede decir que .js es text/plain y el módulo no carga: tipos fijos.
    extensions_map: ClassVar[dict[str, str]] = {
        **SimpleHTTPRequestHandler.extensions_map, ".js": "text/javascript", ".mjs": "text/javascript", ".css": "text/css",
        ".svg": "image/svg+xml", ".json": "application/json", ".wasm": "application/wasm"}

    def log_message(self, *a):
        pass


class _Servidor(ThreadingHTTPServer):
    allow_reuse_address = False  # en Windows, reutilizar dejaría escuchar a dos programas en el mismo puerto


def _resultado(estado: str, motivo: str = "", **datos) -> dict:
    return {"estado": estado, "motivo": motivo, "texto": "", "errores": [], "consola": [], "captura": "", **datos}


def ejecutable(dir_tauri: Path, env: dict) -> Path | None:
    """El .exe de depuración que deja cargo build (cargo metadata respeta CARGO_TARGET_DIR y .cargo/config)."""
    try:
        r = subprocess.run(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=dir_tauri, env=env,
                           capture_output=True, check=False, timeout=60, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        meta = json.loads(r.stdout) if r.returncode == 0 else {}
    except (OSError, ValueError, subprocess.TimeoutExpired):
        return None
    manifiesto = (dir_tauri / "Cargo.toml").resolve()
    for pkg in meta.get("packages", []):
        if Path(pkg["manifest_path"]).resolve() == manifiesto:
            bins = [t["name"] for t in pkg["targets"] if "bin" in t["kind"]]
            if bins:
                nombre = pkg["name"] if pkg["name"] in bins else bins[0]
                exe = Path(meta["target_directory"]) / "debug" / f"{nombre}{'.exe' if os.name == 'nt' else ''}"
                return exe if exe.is_file() else None
    return None


def _escucha(puerto: int) -> bool:
    """Algún programa atiende ya ese puerto (vite escucha en ::1 y no impide ocupar 127.0.0.1)."""
    for host in ("127.0.0.1", "::1"):
        try:
            with socket.create_connection((host, puerto), timeout=0.3):
                return True
        except OSError:
            continue
    return False


def _libre() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _desfasada(dist: Path) -> bool:
    """El frontend cambió después de la última compilación de dist/."""
    from .verificacion import archivos
    raiz = dist.parent
    fuentes = [p for p in archivos(raiz, *FUENTES_WEB) if dist not in p.parents and "src-tauri" not in p.parts]
    return bool(fuentes) and max(p.stat().st_mtime for p in fuentes) > (dist / "index.html").stat().st_mtime + 1


def preparar(dir_tauri: Path, env: dict) -> tuple[dict | None, Path | None, Path | None, int]:
    """(resultado si no se puede abrir, exe, carpeta a servir en devUrl, puerto de devUrl)."""
    if os.name != "nt":
        return _resultado("omitido", "solo se abre en Windows"), None, None, 0
    try:
        cfg = json.loads((dir_tauri / "tauri.conf.json").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return _resultado("omitido", "tauri.conf.json no se puede leer"), None, None, 0
    exe = ejecutable(dir_tauri, env)
    if not exe:
        return _resultado("omitido", "no está compilada (cargo build en src-tauri)"), None, None, 0
    build = cfg.get("build") if isinstance(cfg.get("build"), dict) else {}
    dev = str(build.get("devUrl") or "")
    if not dev:  # sin devUrl, la de depuración lleva el frontend de frontendDist dentro
        return None, exe, None, 0
    url = urllib.parse.urlparse(dev)
    if url.hostname not in ("localhost", "127.0.0.1") or not url.port:
        return _resultado("omitido", f"devUrl {dev} no es un puerto local"), None, None, 0
    dist = (dir_tauri / str(build.get("frontendDist") or "../dist")).resolve()
    if not (dist / "index.html").is_file():
        return _resultado("omitido", f"falta {dist.name}/index.html: ejecuta npm run build"), None, None, 0
    if _desfasada(dist):
        return _resultado("omitido", f"{dist.name}/ es anterior a tus cambios del frontend: ejecuta npm run build"), None, None, 0
    if _escucha(url.port):
        return _resultado("omitido", f"el puerto {url.port} de devUrl ya lo usa otro programa (¿un npx tauri dev o "
                                     "vite que sigue abierto?): detén los tuyos con procesos(accion='detener')"), None, None, 0
    return None, exe, dist, url.port


def probar(dir_tauri: Path, env: dict, png: Path | None = None, espera: float = 4) -> dict:
    """Abre el .exe con el frontend compilado en devUrl y devuelve {estado: ok|error|omitido, motivo, texto, errores,
    consola, captura}. La ventana se ve unos segundos en la pantalla del usuario y se cierra sola."""
    if importlib.util.find_spec("playwright") is None:
        return _resultado("omitido", "falta playwright en el agente (ejecuta instalar.bat)")
    fallo, exe, dist, puerto_dev = preparar(dir_tauri, env)
    if fallo:
        return fallo
    servidor = None
    if dist:
        servidor = _Servidor(("127.0.0.1", puerto_dev), partial(_Estaticos, directory=str(dist)))
        threading.Thread(target=servidor.serve_forever, daemon=True).start()
    puerto = _libre()
    log = (png.parent if png else dir_tauri) / "app_tauri.log"
    log.parent.mkdir(parents=True, exist_ok=True)
    salida = open(log, "wb")  # noqa: SIM115 -- se cierra en el finally, tras matar la app
    proc, trabajo = procesos.lanzar([str(exe)], cwd=dir_tauri, stdout=salida, stderr=subprocess.STDOUT,
                                    stdin=subprocess.DEVNULL,
                                    env=dict(env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={puerto}"),
                                    creationflags=subprocess.CREATE_NO_WINDOW | subprocess.CREATE_NEW_PROCESS_GROUP)
    try:
        return _mirar(proc, puerto, png, espera, log, salida)
    finally:
        procesos.detener(proc, trabajo)  # la app y su WebView2, nada más
        if trabajo:
            trabajo.cerrar()
        salida.close()
        if servidor:
            servidor.shutdown()
            servidor.server_close()


def _cola(log: Path, salida) -> str:
    salida.flush()
    lineas = log.read_bytes().decode("utf-8", "replace").strip().splitlines()
    return " | ".join(lineas[-4:])[:400]


def _mirar(proc, puerto: int, png: Path | None, espera: float, log: Path, salida) -> dict:
    from playwright.sync_api import sync_playwright
    fin = time.time() + ESPERA_VENTANA
    while True:
        if proc.poll() is not None:
            cola = _cola(log, salida) or "sin salida"
            return _resultado("error", errores=[f"la app se cerró al arrancar (código {proc.returncode}): {cola}"])
        try:
            urllib.request.urlopen(f"http://127.0.0.1:{puerto}/json/version", timeout=1).close()
            break
        except OSError:
            if time.time() > fin:
                motivo = (f"la ventana no se abrió en {ESPERA_VENTANA} s (¿ya estaba abierta la app? ciérrala y vuelve "
                          f"a probar): {_cola(log, salida)}")
                return _resultado("error", errores=[motivo])
            time.sleep(0.4)
    consola, errores = [], []
    with sync_playwright() as p:
        nav = p.chromium.connect_over_cdp(f"http://127.0.0.1:{puerto}")
        try:
            pag = None
            for _ in range(50):
                pag = next((x for c in nav.contexts for x in c.pages), None)
                if pag:
                    break
                time.sleep(0.2)
            if pag is None:
                return _resultado("error", errores=["la app arrancó pero su ventana no cargó ninguna página"])
            pag.on("console", lambda m: consola.append(f"{m.type}: {m.text[:200]}"))
            pag.on("pageerror", lambda e: errores.append(f"excepción JS: {str(e)[:200]}"))
            pag.reload(wait_until="load", timeout=20000)  # desde el principio, con la consola ya escuchando
            pag.wait_for_timeout(int(max(1.0, espera) * 1000))
            texto = " ".join((pag.evaluate("document.body ? document.body.innerText : ''") or "").split())
            if png:
                png.parent.mkdir(parents=True, exist_ok=True)
                png.write_bytes(pag.screenshot())
            if pag.url.startswith("chrome-error:"):
                errores.append("la ventana muestra una página de error: no pudo cargar el frontend")
        finally:
            nav.close()
    errores += [c[7:] for c in consola if c.startswith("error: ")]
    return _resultado("error" if errores else "ok", texto=texto[:600], errores=errores[:5],
                      consola=[c for c in consola if not c.startswith("error: ")][:5], captura=str(png or ""))


def _huella(dir_tauri: Path, exe: Path) -> tuple:
    """Lo que cambia la app que se ve: el .exe, la configuración, dist/ y el código del frontend."""
    from .verificacion import archivos
    raiz = dir_tauri.parent
    fuentes = [p for p in archivos(raiz, *FUENTES_WEB) if "src-tauri" not in p.parts]
    return (exe.stat().st_mtime, (dir_tauri / "tauri.conf.json").stat().st_mtime,
            max((p.stat().st_mtime for p in fuentes), default=0), len(fuentes))


def probar_una_vez(dir_tauri: Path, env: dict, png: Path | None = None, espera: float = 4) -> dict:
    """probar(), pero sin volver a abrir la app si nada de lo que se ve cambió desde la última vez."""
    exe = ejecutable(dir_tauri, env)
    huella = _huella(dir_tauri, exe) if exe else None
    previo = _resultados.get(str(exe))
    if huella and previo and previo[0] == huella:
        return previo[1]
    r = probar(dir_tauri, env, png, espera)
    if huella and r["estado"] != "omitido":
        _resultados[str(exe)] = (huella, r)
    return r
