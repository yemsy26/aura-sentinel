"""La sonda de tkinter: una peluquería pasó verificar y fallaba nada más entrar (ttk.Notebook.add de un frame nieto),
porque la app solo se ejecutaba hasta su bucle de eventos."""
import shutil
import sys
import tempfile
import textwrap
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.proyecto import Informe
from agente.verificacion import entrada_bat, probar_tk, usuarios_del_proyecto, verificar

try:
    import tkinter  # noqa: F401
    TK = True
except ImportError:
    TK = False

LOGIN = '''
import hashlib, tkinter as tk
from tkinter import ttk, messagebox
USUARIOS = {"duena": hashlib.sha256("clave2026".encode()).hexdigest()}

class App:
    def __init__(self, root):
        self.root = root
        self.u = ttk.Entry(root); self.u.pack()
        self.c = ttk.Entry(root, show="*"); self.c.pack()
        ttk.Button(root, text="Entrar", command=self.entrar).pack()

    def entrar(self):
        if USUARIOS.get(self.u.get()) != hashlib.sha256(self.c.get().encode()).hexdigest():
            messagebox.showerror("Error", "Usuario o contraseña incorrectos")
            return
        for w in self.root.winfo_children():
            w.destroy()
        self.principal()

{principal}

if __name__ == "__main__":
    root = tk.Tk()
    App(root)
    root.mainloop()
'''
BIEN = '''    def principal(self):
        nb = ttk.Notebook(self.root); nb.pack()
        for nombre in ("Agenda", "Clientes"):
            f = ttk.Frame(nb); nb.add(f, text=nombre)
            ttk.Button(f, text="Ver " + nombre, command=lambda n=nombre: messagebox.showinfo(n, "ok")).pack()
        ttk.Button(self.root, text="Salir", command=self.root.destroy).pack()'''
NIETO = '''    def principal(self):
        nb = ttk.Notebook(self.root); nb.pack()
        ttk.Frame(nb, name="tab_agenda")
        nb.add(tk.Frame(nb.nametowidget("tab_agenda")), text="Agenda")'''


@unittest.skipUnless(TK, "sin tkinter")
class SondaTk(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_sonda_tk_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)

    def probar(self, codigo: str, nombre: str = "main.py") -> Informe:
        (self.dir / nombre).write_text(textwrap.dedent(codigo), encoding="utf-8")
        inf, t0 = Informe(), time.time()
        probar_tk(self.dir, self.dir / nombre, sys.executable, inf, 10)
        self.assertLess(time.time() - t0, 40)
        return inf

    def test_falla_al_entrar(self):
        inf = self.probar(LOGIN.replace("{principal}", NIETO))
        self.assertEqual(len(inf.errores), 1, inf.errores)
        self.assertIn("EXCEPCIÓN al entrar", inf.errores[0])
        self.assertIn('File "main.py"', inf.errores[0])
        self.assertIn("as slave of", inf.errores[0])
        self.assertNotIn("agente_tk_", inf.errores[0])  # ni la carpeta de la copia ni los marcos de tkinter
        self.assertNotIn("__init__.py", inf.errores[0])

    def test_entra_y_recorre(self):
        inf = self.probar(LOGIN.replace("{principal}", BIEN))
        self.assertEqual(inf.errores, [])
        self.assertIn("entró como «duena»", inf.ok[0])
        self.assertIn("2 pestañas", inf.ok[0])
        self.assertFalse((self.dir / ".agente").exists())  # se usa sobre una copia

    def test_usuarios_cerca_de_la_contrasena(self):
        (self.dir / "db.py").write_text('USUARIOS = [\n    ("duena", sha256("clave2026"), "admin"),\n]\nX = "lejos"\n',
                                        encoding="utf-8")
        self.assertEqual(usuarios_del_proyecto(self.dir, ["clave2026"])[:2], ["duena", "admin"])

    def test_boton_que_falla_con_campos_vacios_es_aviso(self):
        inf = self.probar('''
            import tkinter as tk
            root = tk.Tk()
            e = tk.Entry(root); e.pack()
            tk.Button(root, text="Guardar", command=lambda: int(e.get())).pack()
            root.mainloop()
        ''')
        self.assertEqual(inf.errores, [])
        self.assertTrue(any("al pulsar un botón" in a and "ValueError" in a for a in inf.avisos), inf.avisos)

    def test_dialogo_que_espera_no_cuelga(self):
        inf = self.probar('''
            import tkinter as tk
            root = tk.Tk()
            def dialogo():
                d = tk.Toplevel(root); tk.Label(d, text="hola").pack(); d.grab_set(); root.wait_window(d)
            tk.Button(root, text="Nueva cita", command=dialogo).pack()
            root.mainloop()
        ''')
        self.assertEqual(inf.errores, [])
        self.assertIn("1 botones pulsados", inf.ok[0])

    def test_otro_tk_al_entrar(self):
        inf = self.probar(LOGIN.replace("{principal}", '''    def principal(self):
        self.root.destroy()
        r = tk.Tk(); nb = ttk.Notebook(r); nb.pack()
        for n in ("A", "B", "C"):
            nb.add(ttk.Frame(nb), text=n)'''))
        self.assertEqual(inf.errores, [])
        self.assertIn("3 pestañas", inf.ok[0])

    def test_ventana_oculta_al_entrar(self):
        # 4.12.6: la principal se construía en la misma raíz que la de entrar y luego root.withdraw(): no se veía nada
        inf = self.probar('''
            import tkinter as tk
            from tkinter import ttk
            USUARIO = "duena"
            PASSWORD = "clave2026"
            root = tk.Tk()
            u = ttk.Entry(root); u.pack(); c = ttk.Entry(root, show="*"); c.pack()
            def entrar():
                if (u.get(), c.get()) == (USUARIO, PASSWORD):
                    nb = ttk.Notebook(root); nb.pack()
                    for n in ("Agenda", "Clientes", "Servicios"):
                        nb.add(ttk.Frame(nb), text=n)
                    root.withdraw()  # ocultar el login
            ttk.Button(root, text="Entrar", command=entrar).pack()
            root.mainloop()
        ''')
        self.assertEqual(len(inf.errores), 1, inf.errores)
        self.assertIn("después de entrar (como «duena») no queda ninguna ventana a la vista", inf.errores[0])

    def test_entrar_en_toplevel_con_la_raiz_oculta(self):
        inf = self.probar('''
            import tkinter as tk
            from tkinter import ttk
            USUARIO = "duena"
            PASSWORD = "clave2026"
            root = tk.Tk(); root.withdraw()
            top = tk.Toplevel(root)
            u = ttk.Entry(top); u.pack(); c = ttk.Entry(top, show="*"); c.pack()
            def entrar():
                if (u.get(), c.get()) == (USUARIO, PASSWORD):
                    top.destroy(); root.deiconify()
                    nb = ttk.Notebook(root); nb.pack(); nb.add(ttk.Frame(nb), text="Agenda")
            ttk.Button(top, text="Entrar", command=entrar).pack()
            root.mainloop()
        ''')
        self.assertEqual(inf.errores, [])
        self.assertIn("entró como «duena»", inf.ok[0])

    def test_no_abre_nada_fuera_de_la_app(self):
        # un «Exportar» que llama a os.startfile abría Excel en el escritorio; si no se interceptara, este daría error
        inf = self.probar('''
            import os, tkinter as tk
            root = tk.Tk()
            tk.Button(root, text="Exportar", command=lambda: os.startfile("no_existe_sonda.xyz")).pack()
            root.mainloop()
        ''')
        self.assertEqual((inf.errores, inf.avisos), ([], []))
        self.assertIn("1 archivos o programas que abriría fuera de la app (no se abrieron)", inf.ok[0])

    def test_falla_al_abrir_la_ventana(self):
        inf = self.probar('''
            import tkinter as tk
            root = tk.Tk()
            tk.Label(root, text="hola").pack()
            tk.Label(root, text="mal").grid()
            root.mainloop()
        ''')
        self.assertIn("EXCEPCIÓN al abrir la ventana", inf.errores[0])

    def test_verificar_mira_la_ventana_aunque_main_solo_la_importe(self):
        (self.dir / "app.py").write_text(textwrap.dedent(LOGIN.replace("{principal}", NIETO)), encoding="utf-8")
        (self.dir / "main.py").write_text("from app import App\nimport tkinter as tk\n\nif __name__ == '__main__':\n"
                                          "    r = tk.Tk(); App(r); r.mainloop()\n", encoding="utf-8")
        texto, errores = verificar(self.dir, sys.executable, segundos=10)
        self.assertGreaterEqual(errores, 1)
        self.assertIn("EXCEPCIÓN al entrar", texto)

    def test_bat_que_abre_un_archivo_sin_ventana(self):
        # m4127_pelu2: run.bat lanzaba «python -m app» y la ventana se abría en __main__.py; con doble clic, nada.
        (self.dir / "app.py").write_text("import tkinter as tk\n\ndef pantalla(root):\n    tk.Label(root, text='Agenda')"
                                         ".pack()\n", encoding="utf-8")
        (self.dir / "__main__.py").write_text("import tkinter as tk\nfrom app import pantalla\n\nroot = tk.Tk()\n"
                                              "pantalla(root)\nroot.mainloop()\n", encoding="utf-8")
        bat = '@echo off\r\ncd /d "%~dp0"\r\nset "PY=python"\r\necho [1/1] python -m app\r\n{}\r\n'
        (self.dir / "run.bat").write_text(bat.format("%PY% -m app"), encoding="utf-8")
        texto, errores = verificar(self.dir, sys.executable, segundos=10)
        self.assertGreaterEqual(errores, 1, texto)
        self.assertIn("run.bat abre app.py, que termina sin abrir ninguna ventana", texto)
        self.assertIn("La ventana se abre en __main__.py", texto)
        (self.dir / "run.bat").write_text(bat.format("%PY% ."), encoding="utf-8")
        texto, _ = verificar(self.dir, sys.executable, segundos=10)
        self.assertNotIn("sin abrir ninguna ventana", texto)
        self.assertIn("__main__.py: ventana usada sin enseñarla", texto)


class EntradaBat(unittest.TestCase):
    def test_lo_que_abre_el_bat_de_doble_clic(self):
        raiz = Path(tempfile.mkdtemp(prefix="agente_entrada_bat_"))
        self.addCleanup(shutil.rmtree, raiz, ignore_errors=True)
        for nombre in ("app.py", "main.py", "test_app.py", "paquete/__main__.py"):
            (raiz / nombre).parent.mkdir(exist_ok=True)
            (raiz / nombre).write_text("", encoding="utf-8")
        (raiz / "test.bat").write_text("python test_app.py\r\n", encoding="utf-8")
        casos = {"echo python main.py\r\n%PY% -m pip install -r requirements.txt\r\n%PY% -m app": "app.py",
                 '"%PY%" main.py': "main.py", '".venv\\Scripts\\python.exe" "%~dp0main.py"': "main.py",
                 "python -m paquete": "paquete/__main__.py", "python -m venv .venv\r\npython test_app.py": None}
        for texto, esperado in casos.items():
            (raiz / "run.bat").write_text(texto + "\r\n", encoding="utf-8")
            r = entrada_bat(raiz)
            self.assertEqual(r and (r[0].name, r[1].relative_to(raiz).as_posix()), esperado and ("run.bat", esperado), texto)


if __name__ == "__main__":
    unittest.main()
