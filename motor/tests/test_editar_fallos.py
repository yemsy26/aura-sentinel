"""m4179 (Gemma): dos bucles de editar que el motor no ayudaba a romper. (C7) «'buscar' aparece 3 veces… incluye más líneas de
contexto» y Gemma repetía LA MISMA llamada 4 veces: ahora el mensaje trae, para cada aparición, las líneas anteriores que la
distinguen. (C9) Una racha de 17 «No encontré» seguidos sobre un archivo: del 3.º fallo seguido el mensaje recuerda la salida
(buscar de una línea o escribir)."""
import re
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.herramientas import Herramientas

TRES_BOTONES = '''class A:
    def a(self):
        tk.Label(self.root, text="Menú").pack()
        tk.Button(self.root, text="Volver", command=self.main_menu).pack()

    def b(self):
        tk.Label(self.root, text="Productos").pack()
        tk.Button(self.root, text="Volver", command=self.main_menu).pack()

    def c(self):
        tk.Label(self.root, text="Clientes").pack()
        tk.Button(self.root, text="Volver", command=self.main_menu).pack()
'''
BOTON = 'tk.Button(self.root, text="Volver", command=self.main_menu).pack()'


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_fallos_"))
        self.h = Herramientas(self.dir)

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def escribir(self, texto: str, nombre: str = "app.py") -> None:
        (self.dir / nombre).write_text(texto, encoding="utf-8", newline="")

    def leer(self, nombre: str = "app.py") -> str:
        return (self.dir / nombre).read_text(encoding="utf-8")


def buscar_de(mensaje: str, linea: int) -> str:
    """Lo que copiaría el modelo: las líneas numeradas del trozo de esa aparición, sin su «N| »."""
    cola = mensaje.split(f"· La de la línea {linea}:", 1)[1].split("\n")[1:]
    copiadas = []
    for l in cola:
        m = re.match(r"\d+\| (.*)$", l)
        if not m:
            break
        copiadas.append(m.group(1))
    return "\n".join(copiadas)


class VariasApariciones(Base):
    def test_cada_aparicion_trae_la_linea_que_la_distingue(self):
        self.escribir(TRES_BOTONES)
        r = self.h.editar("app.py", BOTON, "x")
        self.assertIn("aparece 3 veces en app.py (empieza en las líneas 4, 8, 12)", r)
        self.assertIn("Incluye más líneas de contexto para que sea único.", r)
        for linea, previa in ((4, "Menú"), (8, "Productos"), (12, "Clientes")):
            self.assertIn(f"· La de la línea {linea}: añade al principio de buscar la línea anterior:", r)
            self.assertIn(f'{linea - 1}|         tk.Label(self.root, text="{previa}").pack()', r)

    def test_copiar_el_trozo_edita_solo_esa_aparicion(self):
        """K1: con el buscar que sale de cada trozo la llamada ya acierta, y solo cambia esa aparición."""
        for linea in (4, 8, 12):
            self.escribir(TRES_BOTONES)
            mensaje = self.h.editar("app.py", BOTON, "x")
            buscar = buscar_de(mensaje, linea)
            self.assertEqual(buscar.count("\n"), 1, buscar)
            nuevo = buscar.replace(BOTON, "HECHO")
            self.assertTrue(self.h.editar("app.py", buscar, nuevo).startswith("OK editado"))
            texto = self.leer()
            self.assertEqual(texto.count("HECHO"), 1)
            self.assertEqual(texto.splitlines()[linea - 1].strip(), "HECHO")
            self.assertEqual(texto.count(BOTON), 2)

    def test_apariciones_identicas_aunque_se_anada_contexto(self):
        bloque = "a = 1\nb = 2\nc = 3\nd = 4\nx = 0\n"
        self.escribir(bloque + "# aparte\n" + bloque)
        r = self.h.editar("app.py", "x = 0", "x = 9")
        self.assertIn("aparece 2 veces", r)
        self.assertNotIn("· La de la línea", r)
        self.assertIn("Las apariciones de las líneas 5, 11 son idénticas aunque se añadan 4 líneas de contexto", r)

    def test_la_primera_linea_del_archivo_no_tiene_linea_anterior(self):
        self.escribir("x = 0\ny = 1\nx = 0\n")
        r = self.h.editar("app.py", "x = 0", "x = 9")
        self.assertIn("· La de la línea 3: añade al principio de buscar la línea anterior:\n2| y = 1\n3| x = 0", r)
        self.assertIn("Las apariciones de las líneas 1 son idénticas", r)

    def test_un_buscar_largo_no_hace_un_mensaje_enorme(self):
        largo = "\n".join(f"        paso_{i} = valor_{i} + calcular_algo_bastante_largo(argumento_{i}, otro_{i})" for i in range(12))
        self.escribir(f"def f():\n    uno = 1\n{largo}\n\ndef g():\n    dos = 2\n{largo}\n")
        r = self.h.editar("app.py", largo, "x")
        self.assertIn("aparece 2 veces", r)
        self.assertLess(len(r), 2500)
        self.assertIn("\n…", r)  # solo se enseñan las 3 primeras líneas del buscar
        self.assertNotIn("paso_5", r)

    def test_una_sola_aparicion_no_cambia(self):
        self.escribir("a = 1\nb = 2\n")
        self.assertTrue(self.h.editar("app.py", "a = 1", "a = 5").startswith("OK editado: app.py."))
        self.assertEqual(self.leer(), "a = 5\nb = 2\n")


class RachaDeFallos(Base):
    def falla(self, nombre: str = "app.py") -> str:
        return self.h.editar(nombre, "esto no está en el archivo", "x")

    def test_avisa_del_tercer_fallo_seguido_en_adelante(self):
        self.escribir("a = 1\n")
        r1, r2, r3, r4 = (self.falla() for _ in range(4))
        self.assertNotIn("Llevas", r1)
        self.assertNotIn("Llevas", r2)
        self.assertIn("[Llevas 3 editar seguidos sin acierto en app.py. Para salir: usa como buscar UNA sola línea", r3)
        self.assertIn("o reescribe el archivo con escribir.]", r3)
        self.assertIn("Llevas 4 editar seguidos", r4)

    def test_un_editar_correcto_reinicia_la_cuenta(self):
        self.escribir("a = 1\nb = 2\n")
        self.falla()
        self.falla()
        self.assertTrue(self.h.editar("app.py", "a = 1", "a = 5").startswith("OK editado"))
        self.assertNotIn("Llevas", self.falla())
        self.assertNotIn("Llevas", self.falla())
        self.assertIn("Llevas 3", self.falla())

    def test_escribir_el_archivo_reinicia_la_cuenta(self):
        self.escribir("a = 1\n")
        self.falla()
        self.falla()
        self.h.escribir("app.py", "a = 2\n")
        self.assertNotIn("Llevas", self.falla())

    def test_cada_archivo_lleva_su_cuenta(self):
        self.escribir("a = 1\n", "app.py")
        self.escribir("b = 1\n", "otro.py")
        self.falla("app.py")
        self.falla("app.py")
        self.assertNotIn("Llevas", self.falla("otro.py"))
        self.assertIn("Llevas 3", self.falla("app.py"))

    def test_aparece_varias_veces_y_no_encontre_cuentan_juntos(self):
        self.escribir("x = 0\nx = 0\n")
        self.assertNotIn("Llevas", self.h.editar("app.py", "x = 0", "x = 1"))
        self.assertNotIn("Llevas", self.h.editar("app.py", "x = 0", "x = 1"))
        self.assertIn("Llevas 3", self.falla())

    def test_un_archivo_que_no_existe_no_cuenta(self):
        for _ in range(4):
            self.assertEqual(self.h.editar("no_existe.py", "a", "b"), "No existe el archivo: no_existe.py.")
        self.assertEqual(self.h.editar_fallos, {})


if __name__ == "__main__":
    unittest.main()
