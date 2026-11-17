"""Arranca la web de un proyecto para recorrerla. Uso: python web_arranque.py app.py PUERTO
Se lanza sobre una copia del proyecto. Escucha solo en 127.0.0.1:PUERTO aunque el código diga otro puerto (Flask.run,
uvicorn.run y los servidores de socketserver/http.server), sin recarga automática ni depurador, y nada sale de la app:
ni el navegador (webbrowser, os.startfile) ni programas lanzados con start. Las trazas de los fallos salen por stderr.
Con Flask, antes de servir compila las plantillas del proyecto y avisa de las que no compilan (WEB_PLANTILLA)."""
import json
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
def _plantillas(app) -> None:
    """Compila cada plantilla del proyecto: una que no compila da 500 en cuanto una página la usa, y el recorrido puede
    no llegar a esa página (el detalle de una venta que aún no existe). Imprime 'WEB_PLANTILLA {json}' por cada una."""
    import jinja2
    env = app.jinja_env
    try:
        nombres = env.list_templates()
    except Exception:  # un cargador que no sabe listar
        return
    for nombre in nombres:
        try:
            fuente, archivo, _ = env.loader.get_source(env, nombre)
        except Exception:
            continue
        if not archivo or re.search(r"[\\/](?:site-packages|Lib)[\\/]", archivo):
            continue  # las de las librerías (Flask-Admin, Bootstrap...) no son del proyecto
        try:
            env.compile(fuente, nombre, archivo)
        except jinja2.TemplateSyntaxError as e:  # también TemplateAssertionError (filtro que no existe)
            print("WEB_PLANTILLA " + json.dumps({"plantilla": nombre, "linea": e.lineno, "error": e.message},
                                                ensure_ascii=False), flush=True)


def _variables(app) -> None:
    """Nombres que una plantilla lee del contexto y que ni el entorno ni un procesador de contexto definen: Jinja no da error por
    una variable sin definir (la lista sale vacía, el «if» es falso, el texto sale en blanco). Imprime 'WEB_VARIABLE {json}' por plantilla,
    con {nombre: {linea, uso}}; si no se puede saber qué da el contexto (un procesador falla), no imprime nada. Los nombres que la plantilla
    comprueba a propósito (`is defined`, `|default`) no salen. Qué rutas pasan esos nombres lo ve el motor leyendo las render_template."""
    from jinja2 import meta, nodes
    env = app.jinja_env
    try:
        nombres = env.list_templates()
    except Exception:
        return
    proveen = set(env.globals)
    try:
        with app.test_request_context():
            for lista in app.template_context_processors.values():
                for funcion in lista:
                    datos = funcion()
                    if not isinstance(datos, dict):
                        return
                    proveen.update(datos)
    except Exception:
        return
    subir = (nodes.Not, nodes.And, nodes.Or, nodes.Compare, nodes.Operand, nodes.Filter, nodes.Test, nodes.Concat, nodes.Neg)
    for nombre in nombres:
        try:
            fuente, archivo, _ = env.loader.get_source(env, nombre)
            if not archivo or re.search(r"[\\/](?:site-packages|Lib)[\\/]", archivo):
                continue
            arbol = env.parse(fuente, nombre, archivo)
            sin = meta.find_undeclared_variables(arbol) - proveen
        except Exception:  # una plantilla que no compila ya sale por su cuenta
            continue
        if not sin:
            continue
        padres = {}
        pila = [arbol]
        while pila:
            n = pila.pop()
            for hijo in n.iter_child_nodes():
                padres[id(hijo)] = n
                pila.append(hijo)
        a_proposito, usos, en_if = set(), {}, set()
        for n in arbol.find_all(nodes.Name):
            if n.name not in sin or n.ctx != "load":
                continue
            p = padres.get(id(n))
            if isinstance(p, nodes.Test) and p.name in ("defined", "undefined") or isinstance(p, nodes.Filter) and p.name in ("default", "d"):
                a_proposito.add(n.name)
                continue
            uso, hijo, p = "salida", n, padres.get(id(n))
            if isinstance(p, (nodes.Getattr, nodes.Getitem, nodes.Call)) and p.node is n:
                uso = "atributo"
            else:
                while p is not None and isinstance(p, subir):
                    hijo, p = p, padres.get(id(p))
                if isinstance(p, nodes.For) and p.iter is hijo:
                    uso = "for"
                elif isinstance(p, (nodes.If, nodes.CondExpr)) and p.test is hijo:
                    uso = "if"
                    en_if.add(n.name)
            orden = ("salida", "if", "for", "atributo")
            previo = usos.get(n.name)
            if previo is None or orden.index(uso) > orden.index(previo["uso"]):
                usos[n.name] = {"linea": previo["linea"] if previo else n.lineno, "uso": uso}
        for k in en_if:  # `x.algo if x else ''`: el atributo solo se lee si x existe
            if usos.get(k, {}).get("uso") == "atributo":
                usos[k]["uso"] = "if"
        usos = {k: v for k, v in usos.items() if k not in a_proposito}
        if usos:
            print("WEB_VARIABLE " + json.dumps({"plantilla": nombre, "nombres": usos}, ensure_ascii=False), flush=True)


try:
    import flask

    def _flask_run(self, host=None, port=None, debug=None, load_dotenv=True, **opciones):
        opciones.pop("use_reloader", None)
        opciones.pop("use_debugger", None)
        try:
            _plantillas(self)
        except Exception:  # comprobar no debe impedir que arranque
            pass
        try:
            _variables(self)
        except Exception:
            pass
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
