"""Usa una app tkinter sin enseñar ventanas. Uso: python tk_sonda.py main.py SEGUNDOS
Se lanza sobre una copia del proyecto. Las ventanas quedan ocultas y messagebox/filedialog/simpledialog contestan solos.
Cuando la app entra en su bucle de eventos: si hay pantalla de entrar (un Entry con show=), prueba los usuarios y
contraseñas de SONDA_USUARIOS / SONDA_CLAVES (JSON); después elige cada pestaña de cada Notebook y pulsa cada botón que
no sea de salir o borrar. Imprime una línea 'SONDA_TK {json}' con las excepciones de cada fase y lo que hizo; en
'sin_ventana', si al abrir o tras entrar la app había ocultado o cerrado todas sus ventanas (withdraw/iconify/destroy).
Una ventana que espera (wait_window) se cierra sola a los 1,5 s."""
import atexit
import json
import os
import re
import runpy
import subprocess
import sys
import tempfile
import time
import tkinter as tk
import traceback
import webbrowser
from tkinter import filedialog, messagebox, simpledialog, ttk

destino, duracion = os.path.abspath(sys.argv[1]), float(sys.argv[2])
os.chdir(os.path.dirname(destino))
sys.path.insert(0, os.path.dirname(destino))
sys.argv = [destino]
CLAVES = json.loads(os.environ.get("SONDA_CLAVES", "[]"))
USUARIOS = json.loads(os.environ.get("SONDA_USUARIOS", "[]"))
EXCLUIR = set(json.loads(os.environ.get("SONDA_EXCLUIR", "[]")))  # usuarios ya probados: se entra como otro
# «Entrar como Dueña» / «Entrar como Empleada» sin contraseña: cualquiera elige el rol.
COMO_ROL = re.compile(r"(?i)\b(?:entrar|ingresar|acceder|iniciar(?: sesi[oó]n)?|conectar(?:se)?|soy)\s+como\b")
NO_PULSAR = re.compile(r"(?i)salir|cerrar|exit|quit|logout|eliminar|borrar|delete|remove|apagar|desconectar|reiniciar|"
                       r"restablecer|vaciar|limpiar todo")
ENTRAR = re.compile(r"(?i)entrar|ingresar|acceder|iniciar|login|log in|sign in|aceptar|continuar|^ok$")
estado = {"fase": "arranque", "errores": [], "mensajes": [], "ventanas": 0, "entrar": None, "pestanas": 0,
          "botones": 0, "bucle": False, "sin_ventana": None, "impreso": False, "inicio": time.time(),
          "vista": {"pestanas": [], "botones": [], "menus": []}, "como_rol": []}
nuevas: list = []  # Toplevel creadas desde la última mirada
raices: list = []  # una app puede destruir su Tk al entrar y crear otro
todas: list = []  # Tk y Toplevel creadas, para saber si al usuario le queda alguna a la vista


def _registrar(exc, val, tb):
    estado["errores"].append({"fase": estado["fase"], "traza": "".join(traceback.format_exception(exc, val, tb))[-2500:]})


tk.Tk.report_callback_exception = lambda self, exc, val, tb: _registrar(exc, val, tb)
_tk_init, _top_init, _retirar = tk.Tk.__init__, tk.Toplevel.__init__, tk.Wm.wm_withdraw


def _oculta(w):
    try:
        _retirar(w)
        w.attributes("-alpha", 0.0)
    except tk.TclError:
        pass


# Las ventanas siempre están ocultas para la sonda; aparte se anota si la app quiere que se vean. Una peluquería
# construía la ventana principal en la misma raíz que la de entrar y luego la ocultaba con withdraw(): no se veía nada.
def _marca(visible):
    def f(self, *a):
        self._sonda_visible = visible
    return f


def _wm_state(self, newstate=None):
    if newstate is None:
        return "normal" if getattr(self, "_sonda_visible", True) else "withdrawn"
    self._sonda_visible = newstate in ("normal", "zoomed")
    return None


tk.Wm.withdraw = tk.Wm.wm_withdraw = tk.Wm.iconify = tk.Wm.wm_iconify = _marca(False)
tk.Wm.deiconify = tk.Wm.wm_deiconify = _marca(True)
tk.Wm.state = tk.Wm.wm_state = _wm_state


def _a_la_vista() -> list:
    return [w for w in todas if _existe(w) and getattr(w, "_sonda_visible", True)]


def _tk_init_oculto(self, *a, **k):
    _tk_init(self, *a, **k)
    _oculta(self)
    raices.append(self)
    todas.append(self)
    estado["ventanas"] += 1
    if estado["ventanas"] == 1:
        print("SONDA_TK_VENTANA", flush=True)  # si luego se pasa del tiempo, se sabe que fue con la ventana abierta
        self.after(300, lambda: _explorar(self))


def _top_init_oculto(self, *a, **k):
    _top_init(self, *a, **k)
    _oculta(self)
    todas.append(self)
    estado["ventanas"] += 1
    nuevas.append(self)


tk.Tk.__init__, tk.Toplevel.__init__ = _tk_init_oculto, _top_init_oculto
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
# Nada sale de la app: un botón «Exportar» llamaba a os.startfile y abría Excel en el escritorio del usuario.
ABRE = re.compile(r"(?i)^\s*(?:start\b|explorer(?:\.exe)?\b|notepad(?:\.exe)?\b|excel(?:\.exe)?\b|"
                  r"cmd(?:\.exe)?\s+/[ck]\s+start\b|rundll32\b)")


def _no_abrir(que, respuesta=None):
    def f(*a, **k):
        estado["abiertos"].append(f"{que} {str(a[0])[:100] if a else ''}".strip())
        return respuesta
    return f


estado["abiertos"] = []
if hasattr(os, "startfile"):
    os.startfile = _no_abrir("os.startfile")
webbrowser.open = webbrowser.open_new = webbrowser.open_new_tab = _no_abrir("webbrowser.open", True)
_sistema, _popen_init = os.system, subprocess.Popen.__init__
os.system = lambda orden: _no_abrir("os.system", 0)(orden) if ABRE.search(str(orden)) else _sistema(orden)


def _popen_sin_abrir(self, args, *a, **k):
    orden = args if isinstance(args, str) else " ".join(map(str, args))
    if ABRE.search(orden):
        estado["abiertos"].append(f"subprocess {orden[:100]}")
        args, k["shell"] = [sys.executable, "-c", "pass"], False
    _popen_init(self, args, *a, **k)


subprocess.Popen.__init__ = _popen_sin_abrir


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


def _cuantos() -> int:
    return sum(1 for r in raices if _existe(r) for _ in _widgets(r))


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
    # Primero lo del mismo formulario que la contraseña: una peluquería ocultaba sus pantallas con pack_forget() y el
    # usuario se escribía en el campo de fecha de la agenda, que seguía en la ventana.
    def cerca(w):
        return w.master is not clave.master, not _vive(w)

    textos = sorted((w for w in _widgets(ventana) if isinstance(w, (tk.Entry, ttk.Entry)) and w is not clave
                     and not str(w.cget("show")).strip()), key=cerca)
    pulsables = sorted((b for b in _widgets(ventana) if hasattr(b, "invoke")), key=cerca)
    boton = next((b for b in pulsables if ENTRAR.search(_texto(b))), None)
    boton = boton or next((b for b in pulsables if isinstance(b, (tk.Button, ttk.Button))), None)
    otros = [u for u in USUARIOS if u not in EXCLUIR]
    if USUARIOS and not otros:
        return
    intentos = [(u, c) for c in CLAVES[:6] for u in (otros[:12] or [""])][:40]
    estado["entrar"] = {"intentos": 0, "usuario": None, "campo_usuario": bool(textos)}
    for usuario, contrasena in intentos:
        if not _vive(clave):
            break
        estado["entrar"]["intentos"] += 1
        if textos:
            _poner(textos[0], usuario)
        _poner(clave, contrasena)
        errores, ventanas, widgets = len(estado["errores"]), estado["ventanas"], _cuantos()
        try:
            boton.invoke() if boton is not None else clave.event_generate("<Return>")
        except tk.TclError:
            pass
        _actualizar(raiz)
        # entró: falló, quitó el formulario, abrió otra ventana, ocultó la de entrar o la llenó con la principal
        if (len(estado["errores"]) > errores or not _vive(clave) or estado["ventanas"] > ventanas
                or not getattr(ventana, "_sonda_visible", True) or _cuantos() >= widgets + 5):
            estado["entrar"]["usuario"] = usuario
            return


def _explorar(raiz) -> None:
    estado["bucle"] = True
    try:
        if not _a_la_vista():
            estado["sin_ventana"] = "arranque"
        _entrar(raiz)
        raiz = next((r for r in reversed(raices) if _existe(r)), raiz)
        nuevas.clear()  # la ventana principal puede ser una Toplevel que abrió el entrar
        if estado["entrar"] and estado["entrar"]["usuario"] is not None and not _a_la_vista():
            estado["sin_ventana"] = estado["sin_ventana"] or "entrar"
        estado["fase"] = "pestanas"
        for nb in [w for w in _widgets(raiz) if isinstance(w, ttk.Notebook)]:
            estado["vista"]["pestanas"] += [str(nb.tab(p, "text")) for p in nb.tabs() if str(nb.tab(p, "state")) == "normal"]
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
                        estado["vista"]["botones"].append(_texto(b).strip())
                        if estado["entrar"] is None and COMO_ROL.search(_texto(b)):
                            estado["como_rol"].append(_texto(b).strip())
                        b.invoke()
                        estado["botones"] += 1
                except tk.TclError:
                    pass
                _cerrar_nuevas(raiz)
        _menus(raiz)
    except Exception:
        _registrar(*sys.exc_info())
    _imprimir()
    os._exit(0)


def _entradas_menu(menu, vistos):
    """(menú, índice, etiqueta) de cada orden del menú y sus submenús."""
    if str(menu) in vistos:
        return
    vistos.add(str(menu))
    try:
        ultimo = menu.index("end")
    except tk.TclError:
        return
    for i in range(0 if ultimo is None else ultimo + 1):
        try:
            tipo = menu.type(i)
            if tipo == "cascade":
                yield from _entradas_menu(menu.nametowidget(menu.entrycget(i, "menu")), vistos)
            elif tipo == "command":
                yield menu, i, str(menu.entrycget(i, "label"))
        except (tk.TclError, KeyError):
            pass


# Una peluquería con menú «Agenda / Informes» decía «No tienes permiso» a la empleada solo al elegir Informes: cada
# orden del menú se usa como un botón (salvo salir o borrar) y entra en lo que ve cada usuario.
def _menus(raiz) -> None:
    vistos = set()
    for v in [r for r in raices if _existe(r)] + [t for t in todas if _existe(t)]:
        try:
            nombre = str(v.cget("menu"))
        except tk.TclError:
            continue
        if not nombre:
            continue
        for menu, i, etiqueta in list(_entradas_menu(v.nametowidget(nombre), vistos)):
            if estado["botones"] >= 60 or time.time() - estado["inicio"] > duracion or NO_PULSAR.search(etiqueta):
                continue
            try:
                if str(menu.entrycget(i, "state")) == "disabled":
                    continue
                estado["vista"]["menus"].append(etiqueta)
                menu.invoke(i)
                estado["botones"] += 1
            except tk.TclError:
                pass
            _cerrar_nuevas(raiz)


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
