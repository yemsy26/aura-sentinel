"""Dos arreglos que salieron de las entregas de negocio con la 4.13.4:
- un proyecto con un test.py (el nombre que eligió Aura, con 17 pruebas) recibía «No hay tests» y verificar no las corría;
- un README que manda ejecutar install.bat y run.bat que no existen (m4137_neg1) no daba ningún aviso."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import verificacion
from agente.proyecto import Informe, archivos
from agente.verificacion import bat_citado_que_no_existe, informe_proyecto

APP = "def suma(a, b):\n    return a + b\n"
PRUEBA_BUENA = "import unittest\nfrom app import suma\n\nclass T(unittest.TestCase):\n    def test_suma(self):\n        self.assertEqual(suma(1, 2), 3)\n\nif __name__ == '__main__':\n    unittest.main()\n"
PRUEBA_MALA = PRUEBA_BUENA.replace("3)", "4)")
# m4133_neg3 y p3_neg3: un test.py que solo lanza test_app.py; pytest lo importaría y daría un error falso.
LANZADOR = "import subprocess, sys\nresult = subprocess.run([sys.executable, 'test_app.py'])\nsys.exit(result.returncode)\n"
PRUEBA_CON_FUNCION = "from app import suma\n\n\ndef test_suma():\n    assert suma(1, 2) == 3\n"


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_readme_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)

    def escribir(self, **archivos_: str) -> None:
        for nombre, texto in archivos_.items():
            ruta = self.dir / nombre.replace("__", "/")
            ruta.parent.mkdir(parents=True, exist_ok=True)
            ruta.write_text(texto, encoding="utf-8")


class TestPySuelto(Base):
    def pruebas(self) -> Informe:
        inf = Informe()
        verificacion.tests_python(self.dir, archivos(self.dir, ".py"), sys.executable, inf)
        return inf

    def test_un_test_py_se_corre(self):
        self.escribir(**{"app.py": APP, "test.py": PRUEBA_BUENA})
        inf = self.pruebas()
        self.assertFalse(any("No hay tests" in a for a in inf.avisos), inf.avisos)
        self.assertTrue(any(o.startswith("tests:") and "1 passed" in o for o in inf.ok), inf.ok)

    def test_un_test_py_que_falla_se_ve(self):
        self.escribir(**{"app.py": APP, "test.py": PRUEBA_MALA})
        self.assertTrue(any("Tests FALLAN" in e for e in self.pruebas().errores))

    def test_un_test_py_y_un_test_x_se_corren_los_dos(self):
        self.escribir(**{"app.py": APP, "test.py": PRUEBA_BUENA, "test_otro.py": PRUEBA_BUENA})
        inf = self.pruebas()
        self.assertTrue(any("2 passed" in o for o in inf.ok), inf.ok)

    def test_un_test_py_que_solo_lanza_otro_archivo_no_es_una_prueba(self):
        # m4133_neg3 y p3_neg3: test.py ejecuta test_app.py; pytest lo importaría y daría un error falso.
        self.escribir(**{"app.py": APP, "test_app.py": PRUEBA_BUENA, "test.py": LANZADOR})
        inf = self.pruebas()
        self.assertEqual(inf.errores, [])
        self.assertFalse(any("test.py" in a for a in inf.avisos), inf.avisos)
        self.assertTrue(any("1 passed" in o for o in inf.ok), inf.ok)

    def test_un_test_py_con_funciones_test_se_corre(self):
        self.escribir(**{"app.py": APP, "test.py": PRUEBA_CON_FUNCION})
        self.assertTrue(any("1 passed" in o for o in self.pruebas().ok))

    def test_un_test_x_normal_sigue_igual(self):
        self.escribir(**{"app.py": APP, "test_app.py": PRUEBA_BUENA})
        self.assertTrue(any("1 passed" in o for o in self.pruebas().ok))

    def test_sin_ningun_archivo_de_pruebas_sigue_el_aviso(self):
        self.escribir(**{"app.py": APP})
        self.assertTrue(any("No hay tests" in a for a in self.pruebas().avisos))

    def test_va_en_el_informe_de_verificar(self):
        self.escribir(**{"app.py": APP, "test.py": PRUEBA_BUENA})
        inf = informe_proyecto(self.dir, sys.executable, segundos=2)
        self.assertFalse(any("No hay tests" in a for a in inf.avisos), inf.avisos)


class BatCitado(Base):
    def errores(self, readme: str, **archivos_: str) -> list[str]:
        self.escribir(**{"README.md": readme, **archivos_})
        inf = Informe()
        bat_citado_que_no_existe(self.dir, inf)
        return inf.errores

    def test_el_readme_que_cita_install_y_run_que_no_existen(self):
        [e] = self.errores("# Tienda\n\n1. Ejecuta `install.bat`\n2. Ejecuta `run.bat`\n")
        self.assertIn("README.md cita install.bat, run.bat, pero no existe", e)

    def test_si_el_bat_existe_no_hay_nada(self):
        self.assertEqual(self.errores("Abre `run.bat`.", **{"run.bat": "@echo off\n"}), [])

    def test_un_bat_en_una_subcarpeta_que_existe(self):
        self.assertEqual(self.errores("Abre scripts\\run.bat", **{"herramientas__run.bat": "@echo off\n"}), [])

    def test_activate_bat_del_entorno_virtual_no_cuenta(self):
        self.assertEqual(self.errores("Activa el entorno: `.venv\\Scripts\\activate.bat` o venv\\Scripts\\activate.bat"), [])

    def test_si_el_readme_dice_que_lo_crees_no_cuenta(self):
        self.assertEqual(self.errores("Crea un archivo run.bat con este contenido:"), [])

    def test_con_variables_de_entorno_no_cuenta(self):
        self.assertEqual(self.errores("Copia el acceso a %USERPROFILE%\\Desktop\\x.bat"), [])

    def test_con_una_variable_pegada_al_nombre_no_cuenta(self):
        self.assertEqual(self.errores("Ejecuta %RAIZ%run.bat"), [])

    def test_npm_cmd_no_cuenta(self):
        self.assertEqual(self.errores("Si falla, prueba con npm.cmd run build"), [])

    def test_sin_readme_no_hay_nada(self):
        inf = Informe()
        bat_citado_que_no_existe(self.dir, inf)
        self.assertEqual(inf.errores, [])

    def test_va_en_el_informe_de_verificar(self):
        self.escribir(**{"README.md": "Ejecuta install.bat\n", "app.py": APP})
        inf = informe_proyecto(self.dir, sys.executable, segundos=2)
        self.assertTrue(any("README.md cita install.bat" in e for e in inf.errores), inf.errores)


if __name__ == "__main__":
    unittest.main()
