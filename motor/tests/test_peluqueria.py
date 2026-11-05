"""Tres fallos de una misión real (sistema de citas de una peluquería, Aura 4.12.2 instalada):
- run.bat cambiado con editar por «python main.py»: con doble clic, «"python" no se reconoce»;
- el resumen final decía «contraseña admin» y el programa creaba admin con admin123: no se podía entrar;
- el modelo pedía leer(lineas=86) para ver la línea 86, recibía otra vez las líneas 1-86 y se detenía sin contestar."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.herramientas import Herramientas
from agente.proyecto import Informe
from agente.verificacion import bats, claves_inventadas

CABECERA = '@echo off\r\nsetlocal EnableExtensions\r\nchcp 65001 >nul\r\ncd /d "%~dp0"\r\n'


class Peluqueria(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_peluqueria_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)
        (self.dir / "main.py").write_text("print('hola')\n", encoding="utf-8")

    def errores_bat(self, cuerpo: str) -> list[str]:
        (self.dir / "run.bat").write_bytes((CABECERA + cuerpo).encode("utf-8"))
        inf = Informe()
        bats(self.dir, [self.dir / "run.bat"], inf)
        return [e for e in inf.errores if "sin buscarlo" in e]

    def test_bat_con_python_sin_buscar(self):
        errores = self.errores_bat('echo [1/1] python main.py\r\npython main.py\r\npause\r\n')
        self.assertEqual(len(errores), 1)
        self.assertIn("run.bat:6 lanza 'python' sin buscarlo", errores[0])
        self.assertTrue(self.errores_bat('start "" /min cmd /k python main.py\r\n'))
        self.assertTrue(self.errores_bat('pip install -r requirements.txt && python main.py\r\n'))

    def test_bat_que_busca_python_o_usa_venv(self):
        self.assertEqual(self.errores_bat('set "PY=python"\r\ncall %PY% main.py\r\n'), [])
        self.assertEqual(self.errores_bat('call .venv\\Scripts\\activate.bat\r\npython main.py\r\n'), [])
        self.assertEqual(self.errores_bat('.venv\\Scripts\\python.exe main.py\r\n'), [])
        self.assertEqual(self.errores_bat('py main.py\r\nrem python main.py\r\n'), [])
        recupera = ('call python --version >nul 2>&1 || set "FALTA=1"\r\nif defined FALTA for /f "skip=2 tokens=2,*" '
                    '%%A in (\'reg query HKCU\\Environment /v Path 2^>nul\') do call set "PATH=%%PATH%%;%%B"\r\n'
                    'python main.py\r\n')
        self.assertEqual(self.errores_bat(recupera), [])

    def test_bat_de_crear_bat_pasa(self):
        h = Herramientas(self.dir, max_chars=3000, timeout=30)
        self.addCleanup(h.detener_todos)
        h.crear_bat("run.bat", ["python main.py"], "Abre el programa.")
        inf = Informe()
        bats(self.dir, [self.dir / "run.bat"], inf)
        self.assertFalse([e for e in inf.errores if "sin buscarlo" in e])

    def test_contrasena_inventada_en_el_resumen(self):
        (self.dir / "gui.py").write_text(
            'db.crear_usuario("admin", hash_password("admin123"), "admin")\n'
            'db.crear_usuario("empleado", hash_password("emple123"), "empleado")\n', encoding="utf-8")
        (self.dir / ".fenix_chat.json").write_text('[{"text": "contraseña `admin`"}]', encoding="utf-8")
        mal = claves_inventadas(self.dir, "2. **Login:** usuario `admin`, contraseña `admin`.")
        self.assertIn("«admin»", mal)
        self.assertIn("«admin123»", mal)
        self.assertEqual(claves_inventadas(self.dir, "usuario `admin`, contraseña `admin123`; empleada: clave **emple123**"), "")
        self.assertEqual(claves_inventadas(self.dir, "**Contraseña:** `admin123`"), "")
        self.assertIn("«1234»", claves_inventadas(self.dir, "**Contraseña:** `1234`"))

    def test_contrasena_sin_saber_cual_no_se_juzga(self):
        self.assertEqual(claves_inventadas(self.dir, "contraseña `admin`"), "")  # el proyecto no dice ninguna
        (self.dir / "app.py").write_text('ADMIN_PASSWORD = os.environ.get("ADMIN_PASS", "s3creta")\n', encoding="utf-8")
        self.assertEqual(claves_inventadas(self.dir, "La contraseña se guarda en `db.sqlite3` y en `ADMIN_PASSWORD`."), "")
        self.assertEqual(claves_inventadas(self.dir, "contraseña `s3creta`"), "")
        self.assertIn("«secreta»", claves_inventadas(self.dir, "contraseña `secreta`"))

    def test_leer_lineas_como_numero_de_linea(self):
        h = Herramientas(self.dir, max_chars=30000, timeout=30)
        self.addCleanup(h.detener_todos)
        h.escribir("gui.py", "\n".join(f"linea {i}" for i in range(1, 701)))
        self.assertIn("50| linea 50", h.leer("gui.py", desde=1, lineas=50))
        aviso = h.leer("gui.py", lineas=86)
        self.assertNotIn("| linea", aviso)
        self.assertIn("'lineas' es CUÁNTAS líneas", aviso)
        self.assertIn("desde=51", aviso)
        self.assertIn("desde=86", aviso)
        self.assertIn("86| linea 86", h.leer("gui.py", desde=86, lineas=20))
        self.assertIn("1| linea 1", h.leer("gui.py", desde=1, lineas=5))  # pedir el principio a propósito sí vale
        h.escribir("gui.py", "\n".join(f"otra {i}" for i in range(1, 701)))
        self.assertIn("86| otra 86", h.leer("gui.py", lineas=86))  # el archivo cambió: se lee
        h.nueva_tarea()
        self.assertIn("1| otra 1", h.leer("gui.py", lineas=10))  # tarea nueva: se lee

    def test_leer_sin_argumentos_como_siempre(self):
        h = Herramientas(self.dir, max_chars=30000, timeout=30)
        self.addCleanup(h.detener_todos)
        h.escribir("a.py", "\n".join(f"x{i} = {i}" for i in range(1, 301)))
        self.assertIn("200| x200", h.leer("a.py"))
        self.assertIn("usa desde=201", h.leer("a.py"))


if __name__ == "__main__":
    unittest.main()
