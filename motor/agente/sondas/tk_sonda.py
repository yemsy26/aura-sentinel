"""Usa una app tkinter sin enseñar ventanas. Uso: python tk_sonda.py main.py SEGUNDOS
Se lanza sobre una copia del proyecto. Las ventanas quedan ocultas y messagebox/filedialog/simpledialog contestan solos.
Cuando la app entra en su bucle de eventos: si hay pantalla de entrar (un Entry con show=), prueba los usuarios y
contraseñas de SONDA_USUARIOS / SONDA_CLAVES (JSON); después elige cada pestaña de cada Notebook y pulsa cada botón que
no sea de salir o borrar. Imprime una línea 'SONDA_TK {json}' con las excepciones de cada fase y lo que hizo.
Una ventana que espera (wait_window) se cierra sola a los 1,5 s."""
import atexit
import json
import os
import re
import runpy
import sys
import tempfile
import time
import tkinter as tk
import traceback
from tkinter import filedialog, messagebox, simpledialog, ttk

destino, duracion = os.path.abspath(sys.argv[1]), float(sys.argv[2])
os.chdir(os.path.dirname(destino))
sys.path.insert(0, os.path.dirname(destino))
sys.argv = [destino]
CLAVES = json.loads(os.environ.get("SONDA_CLAVES", "[]"))
USUARIOS = json.loads(os.environ.get("SONDA_USUARIOS", "[]"))
NO_PULSAR = re.compile(r"(?i)salir|cerrar|exit|quit|logout|eliminar|borrar|delete|remove|apagar|desconectar|reiniciar|"
                       r"restablecer|vaciar|limpiar todo")
ENTRAR = re.compile(r"(?i)entrar|ingresar|acceder|iniciar|login|log in|sign in|aceptar|continuar|^ok$")
estado = {"fase": "arranque", "errores": [], "mensajes": [], "ventanas": 0, "entrar": None, "pestanas": 0,
          "botones": 0, "bucle": False, "impreso": False, "inicio": time.time()}
nuevas: list = []  # Toplevel creadas desde la última mirada
raices: list = []  # una app puede destruir su Tk al entrar y crear otro


def _registrar(exc, val, tb):
    estado["errores"].append({"fase": estado["fase"], "traza": "".join(traceback.format_exception(exc, val, tb))[-2500:]})


tk.Tk.report_callback_exception = lambda self, exc, val, tb: _registrar(exc, val, tb)
_tk_init, _top_init = tk.Tk.__init__, tk.Toplevel.__init__


def _oculta(w):
    try:
        w.withdraw()
        w.attributes("-alpha", 0.0)
    except tk.TclError:
        pass


def _tk_init_oculto(self, *a, **k):
    _tk_init(self, *a, **k)
    _oculta(self)
    raices.append(self)
    estado["ventanas"] += 1
    if estado["ventanas"] == 1:
        print("SONDA_TK_VENTANA", flush=True)  # si luego se pasa del tiempo, se sabe que fue con la ventana abierta
        self.after(300, lambda: _explorar(self))


def _top_init_oculto(self, *a, **k):
    _top_init(self, *a, **k)
    _oculta(self)
    estado["ventanas"] += 1
    nuevas.append(self)


tk.Tk.__init__, tk.Toplevel.__init__ = _tk_init_oculto, _top_init_oculto
tk.Wm.deiconify = tk.Wm.wm_deiconify = lambda self: None
_esperar = tk.Misc.wait_window


def _esperar_con_limite(self, ventana=None):
    v = ventana or self
    try:
        v.after(1500, lambda: v.winfo_exists() and v.destroy())
    except tk.TclError:
        return None
    return _esperar(self, ventana)


tk.Misc.wait_window = _esperar_con_limite


def _mensaje(tipo, respuesta):
    def f(title=None, message=None, **k):
        estado["mensajes"].append({"tipo": tipo, "fase": estado["fase"], "texto": f"{title or ''}: {message or ''}"[:200]})
        return respuesta
    return f


for nombre, resp in (("showinfo", "ok"), ("showwarning", "ok"), ("showerror", "ok"), ("askyesno", True),
                     ("askokcancel", True), ("askquestion", "yes"), ("askretrycancel", False), ("askyesnocancel", True)):
    setattr(messagebox, nombre, _mensaje(nombre, resp))
_salida = tempfile.mkdtemp(prefix="tk_sonda_")
filedialog.asksaveasfilename = lambda *a, **k: os.path.join(_salida, "informe" + (k.get("defaultextension") or ".txt"))
filedialog.askopenfilename = lambda *a, **k: ""
filedialog.askdirectory = lambda *a, **k: _salida
simpledialog.askstring = lambda *a, **k: "prueba"
simpledialog.askinteger = lambda *a, **k: 1
simpledialog.askfloat = lambda *a, **k: 1.0


def _widgets(w):
    try:
        hijos = w.winfo_children()
    except tk.TclError:
        return
    for h in hijos:
        yield h
        yield from _widgets(h)


def _existe(w) -> bool:
    try:
        return bool(w.winfo_exists())
    except tk.TclError:
        return False


def _vive(w) -> bool:
    try:
        return bool(w.winfo_exists()) and (w.winfo_manager() != "" or isinstance(w, (tk.Tk, tk.Toplevel)))
    except tk.TclError:
        return False


def _texto(w) -> str:
    try:
        return str(w.cget("text"))
    except (tk.TclError, AttributeError):
        return ""


def _poner(e, valor):
    try:
        e.delete(0, "end")
        e.insert(0, valor)
    except tk.TclError:
        pass


def _actualizar(raiz):
    try:
        raiz.update()
    except tk.TclError:
        pass


def _cerrar_nuevas(raiz):
    _actualizar(raiz)
    while nuevas:
        v = nuevas.pop()
        try:
            if v.winfo_exists():
                v.destroy()
        except tk.TclError:
            pass


def _entrar(raiz) -> None:
    clave = next((w for w in _widgets(raiz) if isinstance(w, (tk.Entry, ttk.Entry)) and _vive(w)
                  and str(w.cget("show")).strip()), None)
    if clave is None:
        return
    estado["fase"] = "entrar"
    ventana = clave.winfo_toplevel()
    textos = [w for w in _widgets(ventana) if isinstance(w, (tk.Entry, ttk.Entry)) and w is not clave
              and not str(w.cget("show")).strip()]
    boton = next((b for b in _widgets(ventana) if hasattr(b, "invoke") and ENTRAR.search(_texto(b))), None)
    boton = boton or next((b for b in _widgets(ventana) if isinstance(b, (tk.Button, ttk.Button))), None)
    intentos = [(u, c) for c in CLAVES[:6] for u in (USUARIOS[:12] or [""])][:40]
    estado["entrar"] = {"intentos": 0, "usuario": None}
    for usuario, contrasena in intentos:
        if not _vive(clave):
            break
        estado["entrar"]["intentos"] += 1
        if textos:
            _poner(textos[0], usuario)
        _poner(clave, contrasena)
        errores, ventanas = len(estado["errores"]), estado["ventanas"]
        try:
            boton.invoke() if boton is not None else clave.event_generate("<Return>")
        except tk.TclError:
            pass
        _actualizar(raiz)
        if len(estado["errores"]) > errores or not _vive(clave) or estado["ventanas"] > ventanas:
            estado["entrar"]["usuario"] = usuario
            return


def _explorar(raiz) -> None:
    estado["bucle"] = True
    try:
        _entrar(raiz)
        raiz = next((r for r in reversed(raices) if _existe(r)), raiz)
        nuevas.clear()  # la ventana principal puede ser una Toplevel que abrió el entrar
        estado["fase"] = "pestanas"
        for nb in [w for w in _widgets(raiz) if isinstance(w, ttk.Notebook)]:
            for pestana in list(nb.tabs()):
                try:
                    nb.select(pestana)
                    estado["pestanas"] += 1
                except tk.TclError:
                    pass
                _actualizar(raiz)
        estado["fase"] = "botones"
        vistos = set()
        for _ in range(3):  # los botones de pestañas o paneles que aparecen tras pulsar otros
            pendientes = [b for b in _widgets(raiz) if hasattr(b, "invoke") and isinstance(b, (tk.Button, ttk.Button))
                          and str(b) not in vistos and not NO_PULSAR.search(_texto(b))]
            if not pendientes:
                break
            for b in pendientes:
                if estado["botones"] >= 60 or time.time() - estado["inicio"] > duracion:
                    break
                vistos.add(str(b))
                try:
                    if b.winfo_exists() and str(b.cget("state")) != "disabled":
                        b.invoke()
                        estado["botones"] += 1
                except tk.TclError:
                    pass
                _cerrar_nuevas(raiz)
    except Exception:
        _registrar(*sys.exc_info())
    _imprimir()
    os._exit(0)


def _imprimir():
    if estado["impreso"]:
        return
    estado["impreso"] = True
    d = {k: v for k, v in estado.items() if k not in ("impreso", "inicio")}
    print("SONDA_TK " + json.dumps(d, ensure_ascii=False), flush=True)


atexit.register(_imprimir)
try:
    runpy.run_path(destino, run_name="__main__")
except SystemExit:
    pass
except Exception:
    _registrar(*sys.exc_info())
