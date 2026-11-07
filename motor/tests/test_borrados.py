"""Código generado que borra fuera de lo suyo (2026-11-07): un test de una misión hacía
shutil.rmtree(os.path.dirname(self.tmp)) con self.tmp = tempfile.mkdtemp() y cada ejecución vaciaba el %TEMP% del
usuario. Ahora: los comandos del proyecto tienen su propia carpeta temporal, y verificar, ejecutar y experimentar ven
esos borrados antes de ejecutar nada.

Ningún test de aquí ejecuta un borrado peligroso: el código que se analiza no se ejecuta, o el borrado está en un método
que nadie llama; el hijo que sí borra «la carpeta temporal» solo ve carpetas de la prueba."""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import entorno
from agente.borrados import peligrosos
from agente.bucle import Agente
from agente.herramientas import Herramientas

# El test de la misión, con el borrado en un método que nadie llama (el detector no mira quién llama).
TEST_MISION = '''import os
import shutil
import tempfile
import unittest
from pathlib import Path

Path(__file__).with_name("SE_EJECUTO").write_text("si")


class TestApp(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()

    def limpiar_mal(self):
        shutil.rmtree(os.path.dirname(self.tmp), ignore_errors=True)

    def test_suma(self):
        self.assertEqual(1 + 1, 2)
'''

PELIGROSOS = {
    "dirname de mkdtemp en otra función (el de la misión)": TEST_MISION,
    "gettempdir": "import shutil, tempfile\nshutil.rmtree(tempfile.gettempdir())",
    "tempfile.tempdir": "import shutil, tempfile\nshutil.rmtree(tempfile.tempdir)",
    ".parent de Path(mkdtemp())": "import shutil, tempfile\nfrom pathlib import Path\nd = Path(tempfile.mkdtemp())\nshutil.rmtree(d.parent)",
    "dirname de TemporaryDirectory": "import os, shutil, tempfile\nwith tempfile.TemporaryDirectory() as d:\n    shutil.rmtree(os.path.dirname(d))",
    "dirname de mkstemp": "import os, shutil, tempfile\nfd, ruta = tempfile.mkstemp()\nshutil.rmtree(os.path.dirname(ruta))",
    "join con ..": "import os, shutil, tempfile\nd = tempfile.mkdtemp()\nshutil.rmtree(os.path.join(d, '..'))",
    "variable TEMP": "import os, shutil\nshutil.rmtree(os.environ['TEMP'])",
    "getenv USERPROFILE": "import os, shutil\nshutil.rmtree(os.getenv('USERPROFILE'))",
    "Path.home": "import shutil\nfrom pathlib import Path\nshutil.rmtree(Path.home())",
    "expanduser ~": "import os, shutil\nshutil.rmtree(os.path.expanduser('~'))",
    "carpeta del proyecto": "import os, shutil\nBASE = os.path.dirname(os.path.abspath(__file__))\nshutil.rmtree(BASE)",
    "Path(__file__).parent": "import shutil\nfrom pathlib import Path\nshutil.rmtree(Path(__file__).resolve().parent)",
    "la carpeta actual": "import shutil\nshutil.rmtree('.')",
    "raíz del disco": "import shutil\nshutil.rmtree('C:\\\\')",
    "unlink de Path en %TEMP%": "import os\nfrom pathlib import Path\nPath(os.environ['TMP']).rmdir()",
}

NORMALES = {
    "borra su mkdtemp": "import shutil, tempfile\nclass T:\n    def setUp(self):\n        self.tmp = tempfile.mkdtemp()\n"
                        "    def tearDown(self):\n        shutil.rmtree(self.tmp, ignore_errors=True)",
    "borra su TemporaryDirectory": "import shutil, tempfile\nt = tempfile.TemporaryDirectory()\nshutil.rmtree(t.name)",
    "subcarpeta propia de %TEMP%": "import os, shutil, tempfile\nshutil.rmtree(os.path.join(tempfile.gettempdir(), 'miapp'))",
    "subcarpeta con /": "import shutil\nfrom pathlib import Path\nshutil.rmtree(Path.home() / '.miapp')",
    "archivo del proyecto": "import os\nBASE = os.path.dirname(__file__)\nos.remove(os.path.join(BASE, 'datos.db'))",
    "list.remove": "datos = [1, 2]\ndatos.remove(1)",
    "su mkstemp": "import os, tempfile\nfd, ruta = tempfile.mkstemp()\nos.close(fd)\nos.remove(ruta)",
    "dirname de un archivo dentro de su temporal": "import os, shutil, tempfile\nd = tempfile.mkdtemp()\n"
                                                   "shutil.rmtree(os.path.dirname(os.path.join(d, 'x.txt')))",
    "parámetro con el mismo nombre": "import shutil, tempfile\nd = tempfile.gettempdir()\ndef limpiar(d):\n    shutil.rmtree(d)",
    "reasignada a una subcarpeta": "import os, shutil, tempfile\nd = tempfile.gettempdir()\nd = os.path.join(d, 'app')\nshutil.rmtree(d)",
    "self.parent de tkinter": "class V:\n    def __init__(self, parent):\n        self.parent = parent\n    def cerrar(self):\n        self.parent.destroy()",
    "sintaxis rota": "def (:",
}


class Detector(unittest.TestCase):
    def test_peligrosos(self):
        for nombre, codigo in PELIGROSOS.items():
            with self.subTest(nombre):
                self.assertEqual(len(peligrosos(codigo)), 1, codigo)

    def test_normales(self):
        for nombre, codigo in NORMALES.items():
            with self.subTest(nombre):
                self.assertEqual(peligrosos(codigo), [], codigo)

    def test_dice_linea_y_que_borra(self):
        [(linea, llamada, que)] = peligrosos(TEST_MISION)
        self.assertEqual(linea, 15)
        self.assertIn("os.path.dirname(self.tmp)", llamada)
        self.assertIn("%TEMP%", que)


class CarpetaTemporal(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_borrados_"))
        self.del_usuario = self.dir / "temp_del_usuario"  # hace de %TEMP% del usuario
        self.del_usuario.mkdir()
        (self.del_usuario / "de_otro_programa.txt").write_text("no se toca")
        self.propia = self.dir / "proyecto" / ".agente" / "tmp"
        self.tempdir = tempfile.tempdir

    def tearDown(self):
        tempfile.tempdir = self.tempdir
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_los_comandos_solo_pueden_vaciar_la_suya(self):
        fuera = dict.fromkeys(entorno.TEMPORALES, str(self.del_usuario))
        with mock.patch.dict(os.environ, fuera):
            tempfile.tempdir = None  # el motor la toma ahora: la del usuario
            otro = {"TEMP": "x"}
            entorno.aislar_temporales(self.propia, otro)
            try:
                self.assertEqual(os.environ["TEMP"], str(self.propia))
                self.assertEqual(os.environ["TMPDIR"], str(self.propia))
                self.assertEqual(otro["TMP"], str(self.propia))
                self.assertEqual(tempfile.gettempdir(), str(self.del_usuario))  # el motor sigue con la suya
                # Lo del test de la misión, sin salir nunca de la carpeta de la prueba aunque la protección fallara.
                borra = ("import os, shutil, tempfile; d = os.path.dirname(tempfile.mkdtemp()); "
                         f"assert os.path.normcase(d).startswith(os.path.normcase({str(self.dir)!r})), d; "
                         "shutil.rmtree(d, ignore_errors=True)")
                for _ in range(2):  # la segunda vez, la carpeta propia tiene que seguir ahí
                    subprocess.run([sys.executable, "-c", borra], env=dict(os.environ), check=True)
                donde = subprocess.run([sys.executable, "-c", "import tempfile; print(tempfile.gettempdir())"],
                                       env=dict(os.environ), capture_output=True, text=True, check=True).stdout.strip()
            finally:
                entorno.soltar_temporales()  # dentro del patch: si no, dejaría el TEMP de la prueba a toda la suite
            self.assertEqual((os.environ["TEMP"], otro["TEMP"]), (str(self.del_usuario), "x"))
            self.assertNotIn("TMP", otro)
        self.assertTrue((self.del_usuario / "de_otro_programa.txt").exists())
        self.assertTrue(self.propia.is_dir())
        self.assertEqual(Path(donde), self.propia)


class AntesDeEjecutar(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_borrados_"))
        self.h = Herramientas(self.dir)

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_escribir_lo_avisa(self):
        r = self.h.escribir("test_app.py", TEST_MISION)
        self.assertIn("ERROR: test_app.py:15 BORRA FUERA DE LO SUYO", r)

    def test_verificar_no_ejecuta_los_tests(self):
        (self.dir / "test_app.py").write_text(TEST_MISION, encoding="utf-8")
        r = self.h.verificar_cmd()
        self.assertIn("test_app.py:15 BORRA FUERA DE LO SUYO", r)
        self.assertIn("No ejecuté los tests", r)
        self.assertFalse((self.dir / "SE_EJECUTO").exists())

    def test_ejecutar_y_experimentar_se_bloquean(self):
        (self.dir / "test_app.py").write_text(TEST_MISION, encoding="utf-8")
        for r in (self.h.ejecutar(f'"{sys.executable}" -m unittest'), self.h.experimentar("print(1)"),
                  self.h.ver_pantalla("test_app.py")):
            self.assertIn("BLOQUEADO", r)
            self.assertIn("test_app.py:15", r)
        self.assertFalse((self.dir / "SE_EJECUTO").exists())
        (self.dir / "test_app.py").write_text(TEST_MISION.replace("os.path.dirname(self.tmp)", "self.tmp"),
                                              encoding="utf-8")
        self.assertIn("exit=0", self.h.ejecutar(f'"{sys.executable}" -c "print(1)"'))

    def test_experimento_que_borra_fuera(self):
        r = self.h.experimentar("import shutil, tempfile\ndef nunca():\n    shutil.rmtree(tempfile.gettempdir())")
        self.assertIn("BLOQUEADO", r)
        self.assertIn("experimento:3", r)

    def test_el_resumen_avisa_si_queda_un_borrado(self):
        agente = SimpleNamespace(h=self.h)
        self.assertEqual(Agente._peligro_borrados(agente), "")
        (self.dir / "test_app.py").write_text(TEST_MISION, encoding="utf-8")
        aviso = Agente._peligro_borrados(agente)
        self.assertTrue(aviso.startswith("[PELIGRO: el código aún borra fuera del proyecto"))
        self.assertIn("test_app.py:15", aviso)


class CambioDeProyecto(unittest.TestCase):
    """El servidor MCP aísla cada proyecto que elige; al cerrar, TEMP vuelve a ser el de antes del primero."""

    def test_suelta_el_anterior_y_recuerda_el_original(self):
        base = Path(tempfile.mkdtemp(prefix="test_borrados_"))
        self.addCleanup(shutil.rmtree, base, True)
        uno, dos = base / "uno" / ".agente" / "tmp", base / "dos" / ".agente" / "tmp"
        with mock.patch.dict(os.environ, dict.fromkeys(entorno.TEMPORALES, str(base))):
            try:
                entorno.aislar_temporales(uno)
                entorno.aislar_temporales(dos)
                self.assertEqual(os.environ["TMP"], str(dos))
                (uno / "aura_en_uso").unlink()  # el archivo del primero ya no está abierto
            finally:
                entorno.soltar_temporales()
            self.assertEqual(os.environ["TEMP"], str(base))


if __name__ == "__main__":
    unittest.main()
