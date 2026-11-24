"""Gemma (carpeta (25)): editar con un `buscar` de 8 espacios sobre código a 12 casaba como subcadena y dejaba la 1.ª línea del
reemplazo a 12 y las demás a 8 → «expected an indented block after 'except'»; después repetía la misma llamada ya aplicada
5 veces y se detenía. Ahora el reemplazo se reindenta por la diferencia y un editar fallido sobre un .py roto enseña dónde rompe."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.herramientas import Herramientas, reindentar

BASE = '''class A:
    def f(self):
        def vender():
            try:
                x = 1
            except Exception as err: print("Error", err)
        vender()
'''
REEMPLAZO_A_8 = '        except ValueError as err:\n            print("Entrada", err)\n        except Exception as err:\n            print("Error", err)'


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_sangria_"))
        self.h = Herramientas(self.dir)

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def escribir(self, texto: str, nombre: str = "main.py") -> Path:
        p = self.dir / nombre
        p.write_text(texto, encoding="utf-8", newline="")
        return p

    def leer(self, nombre: str = "main.py") -> str:
        return (self.dir / nombre).read_text(encoding="utf-8")


class ReindentaAlEditar(Base):
    def test_el_caso_de_gemma_buscar_a_8_sobre_codigo_a_12(self):
        p = self.escribir(BASE)
        r = self.h.editar("main.py", '        except Exception as err: print("Error", err)', REEMPLAZO_A_8)
        self.assertNotIn("ERROR DE SINTAXIS", r)
        self.assertIn("            except ValueError as err:\n                print(\"Entrada\", err)\n"
                      "            except Exception as err:\n                print(\"Error\", err)\n        vender()", p.read_text())

    def test_buscar_con_la_misma_sangria_no_cambia_nada(self):
        self.escribir(BASE)
        self.h.editar("main.py", '            except Exception as err: print("Error", err)',
                      '            except ValueError as err:\n                print(1)\n            except Exception as err:\n                print(2)')
        self.assertIn("            except ValueError as err:\n                print(1)\n            except Exception as err:\n"
                      "                print(2)\n        vender()", self.leer())

    def test_buscar_sin_espacios_al_inicio_no_se_toca(self):
        """El modelo apuntó a mitad de línea: no se adivina qué sangría quería para el resto."""
        self.escribir("def f():\n    x = 1\n    return x\n")
        self.h.editar("main.py", "x = 1", "x = 1\ny = 2")
        self.assertEqual(self.leer(), "def f():\n    x = 1\ny = 2\n    return x\n")

    def test_lineas_en_blanco_del_reemplazo_no_ganan_espacios(self):
        self.escribir("def f():\n    if 1:\n        a = 1\n")
        self.h.editar("main.py", "    a = 1", "    a = 1\n\n    b = 2")
        self.assertEqual(self.leer(), "def f():\n    if 1:\n        a = 1\n\n        b = 2\n")

    def test_tolerante_buscar_con_mas_sangria_que_el_archivo(self):
        """16 frente a 12: por subcadena no casa; por líneas sin espacios sí, y el reemplazo pierde los 4 de más."""
        self.escribir("def f():\n    if 1:\n        if 2:\n            a = 1\n            b = 2\n")
        self.h.editar("main.py", "                a = 1\n                b = 2", "                a = 3\n                b = 4")
        self.assertEqual(self.leer(), "def f():\n    if 1:\n        if 2:\n            a = 3\n            b = 4\n")

    def test_tolerante_tabs_frente_a_espacios_no_se_toca(self):
        self.escribir("def f():\n    if 1:\n        a = 1\n        b = 2\n")
        self.h.editar("main.py", "\t\ta = 1\n\t\tb = 2", "\t\ta = 3\n\t\tb = 4")
        self.assertEqual(self.leer(), "def f():\n    if 1:\n\t\ta = 3\n\t\tb = 4\n")

    def test_reindentar_suelto(self):
        self.assertEqual(reindentar("a\n  b\n\nc", extra="    ", saltar_primera=True), "a\n      b\n\n    c")
        self.assertEqual(reindentar("    a\n      b", quitar="    "), "a\n  b")


class FalloSobreArchivoRoto(Base):
    ROTO = "def f():\n    try:\n        x = 1\n    except ValueError:\n    print(x)\n    return x\n"

    def test_editar_fallido_sobre_un_py_roto_dice_donde_rompe(self):
        self.escribir(self.ROTO)
        r = self.h.editar("main.py", "esto no está en el archivo", "x")
        self.assertTrue(r.startswith("No encontré el texto"), r)
        self.assertIn("[OJO: main.py tiene ahora un ERROR DE SINTAXIS en la línea 5", r)
        self.assertIn("4|     except ValueError:\n5|     print(x)", r)

    def test_con_zona_parecida_tambien_lo_dice(self):
        self.escribir(self.ROTO)
        r = self.h.editar("main.py", "    except ValueError as e:\n        pass", "x")
        self.assertIn("Lo más parecido", r)
        self.assertIn("ERROR DE SINTAXIS en la línea 5", r)

    def test_archivo_sano_sin_ruido(self):
        self.escribir("def f():\n    return 1\n")
        r = self.h.editar("main.py", "esto no está en el archivo", "x")
        self.assertNotIn("OJO", r)

    def test_un_json_o_un_txt_sin_ruido(self):
        self.escribir("{ esto no es json", "datos.json")
        self.escribir("def (:", "notas.txt")
        for nombre in ("datos.json", "notas.txt"):
            self.assertNotIn("OJO", self.h.editar(nombre, "no está", "x"))


if __name__ == "__main__":
    unittest.main()
