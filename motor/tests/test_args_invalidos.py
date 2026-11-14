"""Mensaje de «Argumentos inválidos»: nombra qué falta o sobra y da la firma (sin modelo): python -m unittest discover -s tests -v"""
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from agente.herramientas import Herramientas, despachar


class ArgumentosInvalidos(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.h = Herramientas(Path(self._tmp.name))

    def test_falta_la_ruta_de_escribir(self):
        """Gemma mandó escribir con solo «contenido» cinco veces seguidas y el mensaje de Python no la sacó del bucle."""
        r = despachar(self.h, "escribir", {"contenido": "body { color: red }"})
        self.assertTrue(r.startswith("Argumentos inválidos para escribir:"), r)
        self.assertIn("falta «ruta»", r)
        self.assertIn("Recibí: contenido", r)
        self.assertIn("Uso: escribir(ruta, contenido, anadir=…)", r)
        self.assertNotIn("Herramientas.escribir()", r)  # ya no es el texto de Python

    def test_sobra_un_argumento(self):
        r = despachar(self.h, "leer", {"ruta": "x.py", "pagina": 3})
        self.assertIn("sobra «pagina»", r)
        self.assertNotIn("falta", r)
        self.assertIn("Uso: leer(ruta, desde=…, lineas=…)", r)

    def test_falta_y_sobra_a_la_vez(self):
        r = despachar(self.h, "escribir", {"path": "a.css", "contenido": "x"})
        self.assertIn("falta «ruta» y sobra «path»", r)

    def test_sin_argumentos(self):
        r = despachar(self.h, "escribir", {})
        self.assertIn("falta «ruta», «contenido»", r)
        self.assertIn("Recibí: ningún argumento", r)

    def test_un_typeerror_de_dentro_conserva_el_mensaje_de_python(self):
        with mock.patch.object(self.h, "listar", side_effect=TypeError("unsupported operand")):
            r = despachar(self.h, "listar", {})
        self.assertEqual(r, "Argumentos inválidos para listar: unsupported operand")

    def test_una_llamada_valida_no_cambia(self):
        (Path(self._tmp.name) / "a.txt").write_text("hola", encoding="utf-8")
        self.assertIn("a.txt", despachar(self.h, "listar", {}))
        self.assertIn("OK", despachar(self.h, "escribir", {"ruta": "b.txt", "contenido": "x"}))
        self.assertEqual((Path(self._tmp.name) / "b.txt").read_text(encoding="utf-8"), "x")

    def test_herramienta_desconocida_sigue_igual(self):
        self.assertTrue(despachar(self.h, "volar", {}).startswith("Herramienta desconocida: volar."))


if __name__ == "__main__":
    unittest.main()
