"""Un .bat debe funcionar al abrirlo con doble clic, con el PATH del Explorador: en las misiones funcionaba dentro de Aura y
al usuario le decía «"python" no se reconoce». Se ejecutan de verdad, solo con system32 en el PATH."""
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_agente import borrar

from agente.herramientas import Herramientas

SYSTEM32 = os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32")
PY_LANZADOR = shutil.which("py", path=os.environ.get("SystemRoot", r"C:\Windows"))
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
        parche = mock.patch("agente.herramientas.carpetas_sistema", lambda: [SYSTEM32])
        parche.start()
        self.addCleanup(parche.stop)

    def tearDown(self):
        self.h.detener_todos()
        borrar(self.dir)

    def test_un_bat_con_python_sin_buscarlo_falla_como_al_usuario(self):
        (self.dir / "run.bat").write_bytes(b"@echo off\r\npython main.py\r\nexit /b %errorlevel%\r\n")
        salida = self.h.ejecutar("run.bat", segundos=30)
        self.assertNotIn("exit=0", salida)
        self.assertNotIn("funciona", salida)

    @unittest.skipUnless(python_del_usuario(), "sin Python en el PATH guardado del usuario")
    def test_crear_bat_recupera_el_path_del_usuario(self):
        self.h.crear_bat("run", ["python main.py"])
        salida = self.h.ejecutar("run.bat", segundos=30)
        self.assertIn("exit=0", salida)
        self.assertIn("funciona", salida)

    @unittest.skipUnless(python_del_usuario(), "sin Python en el PATH guardado del usuario")
    def test_tambien_tras_un_cd_y_con_venv_y_con_otra_orden_detras(self):
        self.h.crear_bat("run", ["python main.py && python main.py"], usar_venv=True)
        salida = self.h.ejecutar(f'cd /d "{self.dir}" && run.bat', segundos=180)
        self.assertIn("exit=0", salida)
        self.assertTrue((self.dir / ".venv" / "Scripts" / "python.exe").is_file())
        self.assertEqual(salida.count("funciona"), 2)

    @unittest.skipUnless(PY_LANZADOR, "sin el lanzador py de Windows")
    def test_sin_python_en_ningun_path_usa_el_lanzador_py(self):
        (self.dir / "reg.cmd").write_text(SIN_RESOLVER, encoding="utf-8")  # como si el usuario no lo tuviera en su PATH
        self.h.crear_bat("run", ["python main.py"])
        with mock.patch("agente.herramientas.carpetas_sistema", lambda: [SYSTEM32, str(Path(PY_LANZADOR).parent)]):
            salida = self.h.ejecutar("run.bat", segundos=30)
        self.assertIn("exit=0", salida)
        self.assertIn("funciona", salida)

    def test_sin_ningun_python_lo_dice_claro_y_para(self):
        for falso in ("reg.cmd", "py.cmd"):
            (self.dir / falso).write_text(SIN_RESOLVER, encoding="utf-8")
        self.h.crear_bat("run", ["python main.py", "echo no debe llegar"])
        salida = self.h.ejecutar("run.bat", segundos=30)
        self.assertIn("No encuentro Python", salida)
        self.assertIn("exit=9009", salida)
        self.assertNotIn("no debe llegar\n", salida.split("Falló")[-1])

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

    def test_solo_cuenta_como_abrir_el_bat_si_es_todo_el_comando(self):
        (self.dir / "run.bat").write_bytes(b"@echo off\r\n")
        for si in ("run.bat", ".\\run.bat", "call run.bat", "cmd /c run.bat", f'cd /d "{self.dir}" && run.bat', "run.bat --x 1"):
            self.assertTrue(self.h._abre_bat(si), si)
        for no in ("python main.py", "python run.bat", "run.bat && python main.py", "otro.bat", "python main.py && run.bat"):
            self.assertFalse(self.h._abre_bat(no), no)


if __name__ == "__main__":
    unittest.main()
