"""H19: `pip install paquete>=3.1` sin comillas en cmd es una redirección (ferretería 23): la salida va a un archivo «3.1» y el paquete se instala sin la versión.
ejecutar y crear_bat lo rechazan, verificar avisa del .bat que ya lo tiene y del archivo suelto que dejó."""
import shutil
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from agente import verificacion
from agente.herramientas import Herramientas
from agente.proyecto import Informe
from agente.verificacion import archivos_de_pip, pip_sin_comillas

# Los comandos reales de las misiones guardadas: cada uno dejó un archivo «3.1», «9.2», «8.0» o «2.0» en la carpeta.
REALES = [
    r'cd "C:\Users\yemsy\Music\programa1\Nueva carpeta (23)" && .venv\Scripts\python -m pip install Flask-SQLAlchemy>=3.1',
    "pip install flask-sqlalchemy>=3.1",
    "pip install Flask-SQLAlchemy>=3.1",
    "python -m pip install pytest>=9.2",
    "pip install pytest>=8.0",
    "pip install SQLAlchemy>=2.0",
]
SALIDA_PIP = ("Collecting Flask-SQLAlchemy\n  Using cached flask_sqlalchemy-3.1.1-py3-none-any.whl.metadata (3.4 kB)\n"
              "Requirement already satisfied: flask>=2.2.5 in .\\.venv\\Lib\\site-packages\nSuccessfully installed Flask-SQLAlchemy-3.1.1\n")


class QueSeDetecta(unittest.TestCase):
    def test_los_comandos_reales_de_las_misiones(self):
        for c in REALES:
            self.assertIsNotNone(pip_sin_comillas(c), c)
        self.assertEqual(pip_sin_comillas(REALES[0]), "Flask-SQLAlchemy>=3.1")

    def test_otras_formas_que_cmd_toma_por_redireccion(self):
        self.assertEqual(pip_sin_comillas("uv add flask>=3"), "flask>=3")
        self.assertEqual(pip_sin_comillas("uv pip install requests>2"), "requests>2")
        self.assertEqual(pip_sin_comillas("pip3 install x<2"), "x<2")
        self.assertEqual(pip_sin_comillas("pip install 'x>=1'"), "'x>=1'")  # las comillas simples no protegen en cmd
        self.assertEqual(pip_sin_comillas("pip install flask[async]>=3"), "flask[async]>=3")
        self.assertEqual(pip_sin_comillas("pip install a==1 b>=2"), "b>=2")
        self.assertEqual(pip_sin_comillas("cd /d x && pip install -q rich>=13 && python app.py"), "rich>=13")

    def test_con_comillas_dobles_o_sin_version_no(self):
        for c in ('pip install "Flask-SQLAlchemy>=3.1"', 'python -m pip install "pytest>=9.2" "x<2"', "pip install flask",
                  "pip install flask==3.1", "pip install flask~=3.1", "pip install -r requirements.txt", "pip install ./paquete",
                  "pip install flask 2>nul", "pip install flask 2>&1", 'pip install "x>=1" > log.txt', "pip install -r requirements.txt > log.txt",
                  "pip freeze > req.txt", "echo flask>=3 > r.txt", 'python -c "print(1>=0)"', 'uv add "x>=1"', "python -m venv .venv"):
            self.assertIsNone(pip_sin_comillas(c), c)


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_pip_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)


class EjecutarYCrearBat(Base):
    def setUp(self):
        super().setUp()
        self.h = Herramientas(self.dir, max_chars=30000, timeout=30)
        self.addCleanup(self.h.detener_todos)

    def test_ejecutar_rechaza_sin_ejecutar_y_da_el_comando_corregido(self):
        with mock.patch("agente.herramientas.correr", return_value=(0, b"ok")) as correr:
            for c in REALES:
                r = self.h.ejecutar(c)
                self.assertTrue(r.startswith("RECHAZADO"), c)
                self.assertIn("No he ejecutado nada", r)
            correr.assert_not_called()
        self.assertEqual([p.name for p in self.dir.iterdir() if p.is_file()], [])
        r = self.h.ejecutar("python -m pip install Flask-SQLAlchemy>=3.1")
        self.assertIn("`Flask-SQLAlchemy>=3.1` sin comillas es una redirección", r)
        self.assertIn('python -m pip install "Flask-SQLAlchemy>=3.1"', r)

    def test_con_comillas_se_ejecuta(self):
        with mock.patch("agente.herramientas.correr", return_value=(0, b"ok")) as correr:
            r = self.h.ejecutar('python -m pip install "Flask-SQLAlchemy>=3.1"')
            self.assertTrue(r.startswith("exit=0"), r)
            self.assertEqual(correr.call_count, 1)
            self.h.ejecutar("python -m pip install -r requirements.txt 2>&1")
            self.assertEqual(correr.call_count, 2)

    def test_crear_bat_rechaza_y_no_crea_el_archivo(self):
        r = self.h.crear_bat("run.bat", ["pip install flask>=3", "python app.py"])
        self.assertTrue(r.startswith("ERROR"), r)
        self.assertIn("`flask>=3` sin comillas", r)
        self.assertIn('pip install \\"flask>=3\\"'.replace("\\", ""), r)
        self.assertIn("No he creado run.bat", r)
        self.assertFalse((self.dir / "run.bat").exists())

    def test_crear_bat_con_comillas_crea_el_bat(self):
        r = self.h.crear_bat("run.bat", ['pip install "flask>=3"', "python app.py"])
        self.assertTrue(r.startswith("OK creado"), r)
        self.assertTrue((self.dir / "run.bat").is_file())


class EnVerificar(Base):
    def test_un_bat_que_ya_existe_da_un_aviso_por_linea(self):
        (self.dir / "run.bat").write_bytes(b"@echo off\r\nrem pip install x>=1\r\necho pip install y>=2\r\npip install Flask-SQLAlchemy>=3.1\r\n"
                                           b"python app.py\r\n")
        (self.dir / "app.py").write_text("print('hola')\n", encoding="utf-8")
        inf = Informe()
        verificacion.bats(self.dir, [self.dir / "run.bat"], inf)
        mios = [a for a in inf.avisos if "sin comillas" in a]
        self.assertEqual(len(mios), 1)
        self.assertIn("run.bat:4 tiene `Flask-SQLAlchemy>=3.1` sin comillas", mios[0])
        self.assertIn('pip install "Flask-SQLAlchemy>=3.1"', mios[0])
        self.assertEqual([e for e in inf.errores if "sin comillas" in e], [])

    def test_un_bat_con_comillas_no_da_nada(self):
        (self.dir / "run.bat").write_bytes(b'@echo off\r\npip install "Flask-SQLAlchemy>=3.1"\r\npython app.py\r\n')
        (self.dir / "app.py").write_text("print('hola')\n", encoding="utf-8")
        inf = Informe()
        verificacion.bats(self.dir, [self.dir / "run.bat"], inf)
        self.assertEqual([a for a in inf.avisos if "sin comillas" in a], [])

    def test_archivo_suelto_con_la_salida_de_pip(self):
        (self.dir / "3.1").write_text(SALIDA_PIP, encoding="utf-8")
        (self.dir / "2.0.1").write_text("Successfully installed x-1\n", encoding="utf-8")
        self.assertEqual([f.name for f in archivos_de_pip(self.dir)], ["2.0.1", "3.1"])
        inf = Informe()
        verificacion.pip_suelto(self.dir, inf)
        self.assertEqual(inf.errores, [])
        self.assertEqual(len(inf.avisos), 2)
        self.assertIn("3.1: archivo suelto en la raíz del proyecto con la salida de un pip install", inf.avisos[1])
        self.assertIn('pip install "paquete>=3.1"', inf.avisos[1])

    def test_lo_que_no_es_salida_de_pip_ni_esta_en_la_raiz_no_cuenta(self):
        (self.dir / "1.0").write_text("notas de la versión 1.0\n", encoding="utf-8")
        (self.dir / "notas").write_text(SALIDA_PIP, encoding="utf-8")
        (self.dir / "3.2").mkdir()
        (self.dir / "sub").mkdir()
        (self.dir / "sub" / "3.1").write_text(SALIDA_PIP, encoding="utf-8")
        (self.dir / ".venv").mkdir()
        (self.dir / ".venv" / "3.1").write_text(SALIDA_PIP, encoding="utf-8")
        self.assertEqual(archivos_de_pip(self.dir), [])
        self.assertEqual(archivos_de_pip(self.dir / "no_existe"), [])
        inf = Informe()
        verificacion.pip_suelto(self.dir, inf)
        self.assertEqual((inf.errores, inf.avisos), ([], []))

    def test_a_lo_sumo_tres_avisos(self):
        for n in ("1.1", "1.2", "1.3", "1.4", "1.5"):
            (self.dir / n).write_text(SALIDA_PIP, encoding="utf-8")
        inf = Informe()
        verificacion.pip_suelto(self.dir, inf)
        self.assertEqual(len(inf.avisos), 3)


if __name__ == "__main__":
    unittest.main()
