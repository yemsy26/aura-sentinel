"""m4182 (Gemma): 16 de 17 editar de una pasada dieron «OK editado. Sintaxis OK.» y no cambiaban nada (buscar y reemplazar
del mismo tamaño; el archivo final sin tocar); Gemma los repetía hasta la 5.ª llamada idéntica y la tarea se detenía.
Ahora un editar cuyo resultado es el mismo texto no se da por bueno: no escribe y lo dice."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.herramientas import Herramientas


class SinCambios(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_sincambios_"))
        self.h = Herramientas(self.dir)
        (self.dir / "app.py").write_text("def f():\n    x = 1\n    y = 2\n", encoding="utf-8", newline="")

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def leer(self) -> str:
        return (self.dir / "app.py").read_text(encoding="utf-8")

    def test_buscar_igual_que_reemplazar_no_se_da_por_editado(self):
        r = self.h.editar("app.py", "    x = 1", "    x = 1")
        self.assertTrue(r.startswith("Sin cambios en app.py: 'reemplazar' deja el archivo exactamente igual"), r)
        self.assertIn("'reemplazar' tiene que ser distinto de 'buscar'", r)
        self.assertEqual(self.leer(), "def f():\n    x = 1\n    y = 2\n")
        self.assertEqual(self.h.paso_escritura, -1)  # el bucle no cree que se haya modificado nada

    def test_la_reindentacion_que_deja_el_mismo_texto_tambien_es_sin_cambios(self):
        """Rama tolerante: buscar con 2 espacios sobre código a 4; el reemplazo, reindentado, es lo que ya hay."""
        r = self.h.editar("app.py", "  x = 1\n  y = 2", "  x = 1\n  y = 2")
        self.assertTrue(r.startswith("Sin cambios en app.py"), r)
        self.assertEqual(self.leer(), "def f():\n    x = 1\n    y = 2\n")

    def test_un_cambio_real_devuelve_lo_de_siempre(self):
        self.h.paso = 7
        r = self.h.editar("app.py", "    x = 1", "    x = 5")
        self.assertTrue(r.startswith("OK editado: app.py."), r)
        self.assertEqual(self.leer(), "def f():\n    x = 5\n    y = 2\n")
        self.assertEqual(self.h.paso_escritura, 7)

    def test_un_archivo_con_crlf_no_se_reescribe(self):
        p = self.dir / "crlf.py"
        p.write_bytes(b"a = 1\r\nb = 2\r\n")
        r = self.h.editar("crlf.py", "a = 1", "a = 1")
        self.assertTrue(r.startswith("Sin cambios"), r)
        self.assertEqual(p.read_bytes(), b"a = 1\r\nb = 2\r\n")

    def test_cuenta_para_la_racha_de_fallos_seguidos(self):
        r1, r2, r3 = (self.h.editar("app.py", "    x = 1", "    x = 1") for _ in range(3))
        self.assertNotIn("Llevas", r1)
        self.assertNotIn("Llevas", r2)
        self.assertIn("[Llevas 3 editar seguidos sin acierto en app.py.", r3)

    def test_un_cambio_real_reinicia_la_cuenta(self):
        self.h.editar("app.py", "    x = 1", "    x = 1")
        self.h.editar("app.py", "    x = 1", "    x = 1")
        self.assertTrue(self.h.editar("app.py", "    y = 2", "    y = 3").startswith("OK editado"))
        self.assertNotIn("Llevas", self.h.editar("app.py", "    y = 3", "    y = 3"))


if __name__ == "__main__":
    unittest.main()
