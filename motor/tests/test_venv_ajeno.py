"""Un .venv que llega con el proyecto (zip, clonado) trae sus propios programas: Aura solo usa el que se creó en esa
carpeta. Los testigos son un .pth (corre al arrancar su python) y un .bat en sus Scripts."""
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from agente import conocimiento
from agente.herramientas import Herramientas

PYPI = {"info": {"name": "x", "version": "1.0", "summary": "x"}, "urls": [{"filename": "x-1.0-py3-none-any.whl"}]}


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_venv_"))
        self.proyecto = self.dir / "Mi Peluquería ñ"
        self.proyecto.mkdir()
        self.arranco = self.dir / "arranco.txt"  # lo crea cualquier python de ese entorno
        self.corrio = self.dir / "corrio.txt"    # lo crea testigo.bat

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def entorno(self, carpeta: Path, orden=(sys.executable, "-m", "venv", "--without-pip")):
        venv = carpeta / ".venv"
        subprocess.run([*orden, str(venv)], check=True, capture_output=True)
        (venv / "Lib" / "site-packages" / "testigo.pth").write_text(
            f"import pathlib; pathlib.Path({str(self.arranco)!a}).touch()\n", encoding="ascii")
        (venv / "Scripts" / "testigo_aura.bat").write_text(f'@type nul > "{self.corrio}"\n', encoding="ascii")
        return venv

    def herramientas(self):
        h = Herramientas(self.proyecto)
        h._descargar = lambda url, datos=None, **kw: (json.dumps(PYPI), "application/json")
        return h


class Ajeno(Base):
    def setUp(self):
        super().setUp()
        shutil.copytree(self.entorno(self.dir / "origen"), self.proyecto / ".venv")  # creado en otra carpeta
        self.arranco.unlink(missing_ok=True)

    def test_consultar_un_paquete_no_ejecuta_su_python(self):
        r = conocimiento.version(self.herramientas(), "x")
        self.assertIn("PyPI x 1.0", r)
        self.assertIn("Wheel para tu Python", r)
        self.assertFalse(self.arranco.exists())

    def test_ejecutar_no_lo_pone_en_el_path_y_lo_dice(self):
        h = self.herramientas()
        r = h.ejecutar("testigo_aura")
        h.ejecutar('python -c "print(1)"')
        self.assertFalse(self.corrio.exists())
        self.assertFalse(self.arranco.exists())
        self.assertIn(".venv de esta carpeta no se creó aquí", r)
        self.assertEqual(h.python_proyecto(), "python")
        env = h._entorno()
        self.assertNotIn(str(self.proyecto), env["PATH"])
        self.assertNotIn(str(self.proyecto), env.get("VIRTUAL_ENV", ""))

    def test_rehacerlo_como_dice_el_aviso_lo_recupera(self):
        h = self.herramientas()
        r = h.ejecutar("python -m venv --clear .venv")
        self.assertEqual(Path(h.python_proyecto()), h.raiz / ".venv" / "Scripts" / "python.exe", r)
        self.assertNotIn("no se creó aquí", r)
        self.assertFalse((self.proyecto / ".venv" / "Lib" / "site-packages" / "testigo.pth").exists())
        self.assertFalse(self.arranco.exists())

    def test_sin_activate_bat_tampoco_se_usa(self):
        (self.proyecto / ".venv" / "Scripts" / "activate.bat").unlink()
        self.assertEqual(self.herramientas().python_proyecto(), "python")

    def test_ruta_de_red_en_su_activate_no_cuenta(self):
        (self.proyecto / ".venv" / "Scripts" / "activate.bat").write_text(
            'set "VIRTUAL_ENV=\\\\servidor.invalido\\x\\.venv"\n', encoding="utf-8")
        self.assertEqual(self.herramientas().python_proyecto(), "python")


class Propio(Base):
    def comprobar(self):
        h = self.herramientas()
        self.assertEqual(Path(h.python_proyecto()), h.raiz / ".venv" / "Scripts" / "python.exe")
        r = h.ejecutar("testigo_aura")
        self.assertTrue(self.corrio.exists(), r)
        self.assertNotIn("no se creó aquí", r)
        conocimiento.version(h, "x")
        self.assertTrue(self.arranco.exists())

    def test_creado_con_python_m_venv(self):
        self.entorno(self.proyecto)
        self.comprobar()

    @unittest.skipUnless(shutil.which("uv"), "sin uv")
    def test_creado_con_uv(self):
        self.entorno(self.proyecto, ("uv", "venv", "--python", sys.executable))
        self.comprobar()


if __name__ == "__main__":
    unittest.main()
