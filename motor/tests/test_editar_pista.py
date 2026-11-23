"""m4171 M2: editar da la zona parecida (con números de línea) cuando no encuentra el texto y las líneas de cada aparición cuando hay varias.
m4170: 17 editar sin encontrar el texto en 8 misiones, casi todos seguidos de un leer del mismo archivo."""
import shutil
import tempfile
import unittest
from pathlib import Path

from agente.herramientas import Herramientas, zona_parecida

ARCHIVO = """import os

def cargar(ruta):
    if not os.path.exists(ruta):
        return None
    with open(ruta) as f:
        return f.read()

def guardar(ruta, texto):
    if not texto:
        return None
    with open(ruta, 'w') as f:
        f.write(texto)

def total(items):
    suma = 0
    for it in items:
        suma += it.precio * it.cantidad
    return suma
"""


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_editar_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)
        self.h = Herramientas(self.dir, max_chars=60000, timeout=30)
        self.addCleanup(self.h.detener_todos)
        (self.dir / "util.py").write_text(ARCHIVO, encoding="utf-8")


class SinEncontrarElTexto(Base):
    def test_una_linea_cambiada_da_las_lineas_reales_con_su_numero(self):
        r = self.h.editar("util.py", "def total(items):\n    suma = 0\n    for item in items:\n        suma += item.precio\n",
                          "def total(items):\n    return 0\n")
        self.assertTrue(r.startswith("No encontré el texto a reemplazar en util.py"), r)
        self.assertIn("15| def total(items):", r)
        self.assertIn("17|     for it in items:", r)
        self.assertNotIn("Usa leer", r)

    def test_con_la_zona_se_corrige_y_el_segundo_intento_funciona(self):
        self.h.editar("util.py", "    for item in items:\n        suma += item.precio\n", "    pass\n")
        r = self.h.editar("util.py", "    for it in items:\n        suma += it.precio * it.cantidad\n", "    suma = sum(i.precio for i in items)\n")
        self.assertTrue(r.startswith("OK editado"), r)

    def test_con_dos_zonas_iguales_elige_la_que_sigue_pareciendose(self):
        """cargar y guardar tienen un 'return None' parecido: el bloque de guardar (con 'with open(ruta, w)') debe señalar guardar."""
        r = self.h.editar("util.py", "    if not texto:\n        return None\n    with open(ruta, 'a') as f:\n", "    pass\n")
        self.assertIn("10|     if not texto:", r)
        self.assertNotIn("4|     if not os.path.exists", r)

    def test_la_zona_cabe_en_poco_texto_aunque_el_archivo_sea_grande(self):
        grande = "\n".join(f"def f{i}(x):\n    return x + {i}\n" for i in range(400))
        (self.dir / "grande.py").write_text(grande, encoding="utf-8")
        r = self.h.editar("grande.py", "def f200(x):\n    return x + 999\n", "pass")
        self.assertIn("def f200(x):", r)
        self.assertLess(len(r), 1700)

    def test_si_nada_se_parece_no_inventa_una_zona(self):
        r = self.h.editar("util.py", "class Cliente:\n    nombre = ''\n    telefono = ''\n", "pass")
        self.assertTrue(r.startswith("No encontré el texto a reemplazar en util.py. Usa leer"), r)
        self.assertNotIn("| ", r)

    def test_zona_parecida_vacia_sin_lineas_utiles(self):
        self.assertEqual(zona_parecida(ARCHIVO, "\n\n  \n"), "")
        self.assertEqual(zona_parecida("", "def x():\n    pass\n"), "")


class VariasApariciones(Base):
    def test_dice_en_que_lineas_aparece(self):
        r = self.h.editar("util.py", "        return None\n", "        return 0\n")
        self.assertIn("aparece 2 veces en util.py", r)
        self.assertIn("líneas 5, 11", r)
        self.assertIn("Incluye más líneas de contexto", r)


class LoQueYaFuncionabaSigueIgual(Base):
    def test_coincidencia_exacta(self):
        self.assertTrue(self.h.editar("util.py", "    suma = 0\n", "    suma = 1\n").startswith("OK editado"))
        self.assertIn("suma = 1", (self.dir / "util.py").read_text(encoding="utf-8"))

    def test_tolerancia_por_espacios(self):
        r = self.h.editar("util.py", "  suma = 0  \n for it in items:\n", "    suma = 5\n    for it in items:\n")
        self.assertTrue(r.startswith("OK editado"), r)

    def test_archivo_inexistente(self):
        self.assertEqual(self.h.editar("no_existe.py", "a", "b"), "No existe el archivo: no_existe.py.")


if __name__ == "__main__":
    unittest.main()
