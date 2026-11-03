"""Un .bat de crear_bat debe funcionar al abrirlo con doble clic, con el PATH del Explorador: en las misiones funcionaba dentro
de Aura y al usuario le decía «"python" no se reconoce». Se ejecutan de verdad, solo con system32 en el PATH."""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_agente import borrar

from agente.herramientas import Herramientas
from agente.proyecto import Informe
from agente.verificacion import bats

SYSTEM32 = os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32")
SIN_RESOLVER = "@exit /b 1\r\n"


def python_del_usuario() -> bool:
    try:
        import winreg
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, "Environment") as k:
            return bool(shutil.which("python", path=winreg.ExpandEnvironmentStrings(winreg.QueryValueEx(k, "Path")[0])))
    except OSError:
        return False


class DobleClic(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_clic_"))
        self.h = Herramientas(self.dir)
        (self.dir / "main.py").write_text("print('funciona')\n", encoding="utf-8")

    def tearDown(self):
        self.h.detener_todos()
        borrar(self.dir)

    def abrir(self, nombre="run.bat", path=SYSTEM32) -> subprocess.CompletedProcess:
        quitar = ("PATH", "VIRTUAL_ENV", "NODEFAULTCURRENTDIRECTORYINEXEPATH")
        entorno = {k: v for k, v in os.environ.items() if k.upper() not in quitar}
        return subprocess.run(["cmd", "/c", str(self.dir / nombre)], cwd=self.dir, env={**entorno, "PATH": path, "AGENTE_NOPAUSE": "1"},
                              stdin=subprocess.DEVNULL, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=180,
                              check=False)

    def test_un_bat_con_python_sin_buscarlo_falla_como_al_usuario(self):
        (self.dir / "run.bat").write_bytes(b"@echo off\r\npython main.py\r\nexit /b %errorlevel%\r\n")
        self.assertNotIn("funciona", self.abrir().stdout)

    @unittest.skipUnless(python_del_usuario(), "sin Python en el PATH guardado del usuario")
    def test_crear_bat_recupera_el_path_del_usuario(self):
        self.h.crear_bat("run", ["python main.py"])
        r = self.abrir()
        self.assertEqual(r.returncode, 0, r.stdout)
        self.assertIn("funciona", r.stdout)

    @unittest.skipUnless(python_del_usuario(), "sin Python en el PATH guardado del usuario")
    def test_tambien_crea_el_venv_y_con_otra_orden_detras(self):
        self.h.crear_bat("run", ["python main.py && python main.py"], usar_venv=True)
        r = self.abrir()
        self.assertEqual(r.returncode, 0, r.stdout)
        self.assertTrue((self.dir / ".venv" / "Scripts" / "python.exe").is_file())
        self.assertEqual(r.stdout.count("funciona"), 2)

    def test_sin_ningun_python_lo_dice_claro_y_para(self):
        for falso in ("reg.cmd", "py.cmd"):
            (self.dir / falso).write_text(SIN_RESOLVER, encoding="utf-8")
        self.h.crear_bat("run", ["python main.py", "echo no debe llegar"])
        r = self.abrir()
        self.assertIn("No encuentro Python", r.stdout)
        self.assertEqual(r.returncode, 9009)
        self.assertNotIn("no debe llegar\n", r.stdout.split("Falló")[-1])

    def test_lo_que_lanza_start_o_cmd_tambien_se_busca(self):
        """El caso real: «start cmd /k python app.py web». La ventana nueva hereda el PATH de este .bat."""
        self.h.crear_bat("run", ['start "" /min cmd /k python main.py web'])
        texto = (self.dir / "run.bat").read_text(encoding="utf-8")
        self.assertIn('set "FALTA=1"', texto)
        self.assertIn('start "" /min cmd /k %PY% main.py web', texto)

    @unittest.skipUnless(python_del_usuario(), "sin Python en el PATH guardado del usuario")
    def test_cmd_c_con_python_funciona(self):
        self.h.crear_bat("run", ['cmd /c "python main.py"'])
        r = self.abrir()
        self.assertEqual(r.returncode, 0, r.stdout)
        self.assertIn("funciona", r.stdout)

    def test_lo_de_cmd_y_del_proyecto_no_se_busca(self):
        (self.dir / "otro.bat").write_bytes(b"@echo otro\r\n")
        self.h.crear_bat("run", ["start http://localhost:8000", "echo hola", "cd sub && otro", "otro.bat"])
        texto = (self.dir / "run.bat").read_text(encoding="utf-8")
        self.assertNotIn("FALTA", texto)
        self.assertNotIn('set "PY=', texto)

    def test_el_mensaje_de_cada_paso_no_cambia(self):
        self.h.crear_bat("run", ["python main.py"])
        texto = (self.dir / "run.bat").read_text(encoding="utf-8")
        self.assertIn("echo [1/1] python main.py", texto)
        self.assertIn("%PY% main.py", texto)

    def test_codigos_de_html_en_un_bat_son_error(self):
        """Una misión escribió «python app.py &gt;nul»: cmd lanza el servidor y setup.bat se queda colgado."""
        (self.dir / "setup.bat").write_text('@echo off\ncd /d "%~dp0"\npython main.py &gt;nul 2&gt;&amp;1\n', encoding="utf-8")
        inf = Informe()
        bats(self.dir, [self.dir / "setup.bat"], inf)
        self.assertTrue(any("&gt;" in e for e in inf.errores), inf.errores)


if __name__ == "__main__":
    unittest.main()
