"""Un .venv que llega con el proyecto (zip, clonado) trae sus propios programas: Aura solo usa el que se creó en esa
carpeta. Los testigos son un .pth (corre al arrancar su python) y un .bat en sus Scripts."""
import json
import os
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


class Bat(Base):
    """El .bat de crear_bat corre también con doble clic: la comprobación va dentro de él (m4208). Se ejecuta de verdad."""

    def setUp(self):
        super().setUp()
        self.proyecto = self.dir / "Mi Peluquería ñ (copia) & más"
        self.proyecto.mkdir()
        (self.proyecto / "main.py").write_text("import sys; print('entorno', sys.prefix.endswith('.venv'))", encoding="utf-8")

    def abrir(self, usar_venv=True) -> subprocess.CompletedProcess:
        Herramientas(self.proyecto).crear_bat("run", ["python main.py"], usar_venv=usar_venv)
        entorno = {k: v for k, v in os.environ.items() if k.upper() not in ("PATH", "VIRTUAL_ENV", "NODEFAULTCURRENTDIRECTORYINEXEPATH")}
        entorno["PATH"] = os.pathsep.join([sys.base_prefix, os.path.join(os.environ["SystemRoot"], "System32")])
        return subprocess.run(["cmd", "/c", "run.bat"], cwd=self.proyecto, env={**entorno, "AGENTE_NOPAUSE": "1"}, check=False,
                              stdin=subprocess.DEVNULL, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=180)

    def ajeno(self) -> Path:
        venv = self.proyecto / ".venv"
        shutil.copytree(self.entorno(self.dir / "origen"), venv)
        with (venv / "Scripts" / "activate.bat").open("a", encoding="utf-8") as f:
            print(f'type nul > "{self.corrio}"', file=f)
        self.arranco.unlink(missing_ok=True)
        return venv

    def se_detiene_sin_tocarlo(self, venv: Path):
        antes = sorted(venv.rglob("*"))
        r = self.abrir()
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("La carpeta .venv no se creó aquí", r.stdout)
        self.assertNotIn("entorno", r.stdout)
        self.assertFalse(self.corrio.exists(), r.stdout)
        self.assertFalse(self.arranco.exists(), r.stdout)
        self.assertEqual(sorted(venv.rglob("*")), antes)

    def test_ajeno_no_se_activa(self):
        self.se_detiene_sin_tocarlo(self.ajeno())

    def test_sin_activate_bat_no_se_usa(self):
        venv = self.ajeno()
        (venv / "Scripts" / "activate.bat").unlink()
        self.se_detiene_sin_tocarlo(venv)

    def test_sin_python_exe_no_se_crea_encima(self):
        venv = self.ajeno()
        (venv / "Scripts" / "python.exe").unlink()
        self.se_detiene_sin_tocarlo(venv)

    def usa_el_suyo(self):
        for _ in range(2):
            r = self.abrir()
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            self.assertIn("entorno True", r.stdout)

    def test_sin_venv_lo_crea_y_despues_lo_usa(self):
        self.usa_el_suyo()

    def test_creado_con_python_m_venv(self):
        self.entorno(self.proyecto)
        self.usa_el_suyo()

    @unittest.skipUnless(shutil.which("uv"), "sin uv")
    def test_creado_con_uv(self):
        self.entorno(self.proyecto, ("uv", "venv", "--python", sys.executable))
        self.usa_el_suyo()

    def test_sin_usar_venv_no_cambia(self):
        self.abrir(usar_venv=False)
        self.assertNotIn(".venv", (self.proyecto / "run.bat").read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
