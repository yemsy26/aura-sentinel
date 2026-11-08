"""Arranca la web de un proyecto para recorrerla. Uso: python web_arranque.py app.py PUERTO
Se lanza sobre una copia del proyecto. Escucha solo en 127.0.0.1:PUERTO aunque el código diga otro puerto (Flask.run,
uvicorn.run y los servidores de socketserver/http.server), sin recarga automática ni depurador, y nada sale de la app:
ni el navegador (webbrowser, os.startfile) ni programas lanzados con start. Las trazas de los fallos salen por stderr."""
import os
import re
import runpy
import socketserver
import subprocess
import sys
import webbrowser

destino, PUERTO = os.path.abspath(sys.argv[1]), int(sys.argv[2])
os.chdir(os.path.dirname(destino))
sys.path.insert(0, os.path.dirname(destino))
sys.argv = [destino]
os.environ["PORT"] = str(PUERTO)
ABRE = re.compile(r"(?i)^\s*(?:start\b|explorer(?:\.exe)?\b|notepad(?:\.exe)?\b|excel(?:\.exe)?\b|"
                  r"cmd(?:\.exe)?\s+/[ck]\s+start\b|rundll32\b)")


def _nada(*a, **k):
    return True


if hasattr(os, "startfile"):
    os.startfile = _nada
webbrowser.open = webbrowser.open_new = webbrowser.open_new_tab = _nada
_sistema, _popen_init = os.system, subprocess.Popen.__init__
os.system = lambda orden: 0 if ABRE.search(str(orden)) else _sistema(orden)


def _popen_sin_abrir(self, args, *a, **k):
    if ABRE.search(args if isinstance(args, str) else " ".join(map(str, args))):
        args, k["shell"] = [sys.executable, "-c", "pass"], False
    _popen_init(self, args, *a, **k)


subprocess.Popen.__init__ = _popen_sin_abrir
_tcp_init = socketserver.TCPServer.__init__


def _tcp_en_el_puerto(self, direccion, *a, **k):
    _tcp_init(self, ("127.0.0.1", PUERTO), *a, **k)


socketserver.TCPServer.__init__ = _tcp_en_el_puerto
try:
    import flask

    def _flask_run(self, host=None, port=None, debug=None, load_dotenv=True, **opciones):
        opciones.pop("use_reloader", None)
        opciones.pop("use_debugger", None)
        return _run(self, "127.0.0.1", PUERTO, False, load_dotenv, use_reloader=False, use_debugger=False, **opciones)

    _run, flask.Flask.run = flask.Flask.run, _flask_run
except ImportError:
    pass
try:
    import uvicorn

    def _uvicorn_run(app, *a, **k):
        k.update(host="127.0.0.1", port=PUERTO, reload=False, workers=1)
        return _uvicorn(app, **k)

    _uvicorn, uvicorn.run = uvicorn.run, _uvicorn_run
except ImportError:
    pass
print(f"WEB_ARRANQUE {PUERTO}", flush=True)
runpy.run_path(destino, run_name="__main__")
