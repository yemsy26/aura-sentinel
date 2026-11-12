"""Un run.bat de doble clic que borra la base de datos en cada arranque: la dueña pierde lo que dio de alta. Lo hizo
una misión del negocio (m4135_neg4: «db.drop_all(); db.create_all()» antes de seed.py y de app.py)."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.proyecto import Informe
from agente.verificacion import bats

CABECERA = '@echo off\ncd /d "%~dp0"\n'
DROP = ('python -c "from app import create_app; from models import db; app=create_app(); ctx=app.app_context(); '
        'ctx.push(); db.drop_all(); db.create_all()"\n')


class BatQueBorraDatos(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_borra_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)

    def errores(self, cuerpo: str, nombre: str = "run.bat") -> list[str]:
        (self.dir / nombre).write_text(CABECERA + cuerpo, encoding="utf-8")
        (self.dir / "app.py").write_text("print('hola')\n", encoding="utf-8")
        inf = Informe()
        bats(self.dir, [self.dir / nombre], inf)
        return [e for e in inf.errores if "borra la base de datos" in e]

    def test_drop_all_en_cada_arranque_es_error(self):
        errores = self.errores(DROP + "python app.py\n")
        self.assertEqual(len(errores), 1)
        self.assertIn("run.bat:3 borra la base de datos en cada arranque (`drop_all(`)", errores[0])
        self.assertIn("db.create_all()", errores[0])

    def test_borrar_el_archivo_de_la_base_tambien(self):
        self.assertTrue(self.errores("del negocio.db\npython app.py\n"))
        self.assertTrue(self.errores("del /q data\\negocio.sqlite\npython app.py\n"))
        self.assertTrue(self.errores("rmdir /s /q instance\npython app.py\n"))
        self.assertTrue(self.errores('python -c "import os; os.remove(\'ventas.db\')"\npython app.py\n'))

    def test_con_create_all_no_se_marca(self):
        self.assertEqual(self.errores(DROP.replace("db.drop_all(); ", "") + "python app.py\n"), [])
        self.assertEqual(self.errores("python app.py\n"), [])

    def test_si_esta_guardado_por_un_if_no_se_marca(self):
        self.assertEqual(self.errores("if not exist negocio.db " + DROP + "python app.py\n"), [])
        self.assertEqual(self.errores("if exist negocio.db del negocio.db\npython app.py\n"), [])
        bloque = "if not exist negocio.db (\n  " + DROP + "  python seed.py\n) else (\n  echo ya existe\n)\npython app.py\n"
        self.assertEqual(self.errores(bloque), [])
        # lo que viene después del bloque ya no está guardado
        self.assertEqual(len(self.errores("if not exist x.txt (\n  echo hola\n)\n" + DROP + "python app.py\n")), 1)

    def test_los_bat_que_existen_para_borrar_o_probar_no_se_marcan(self):
        for nombre in ("reset_db.bat", "limpiar.bat", "borrar_datos.bat", "test.bat", "instalar.bat", "demo.bat"):
            self.assertEqual(self.errores(DROP, nombre), [], nombre)

    def test_un_echo_con_la_orden_no_cuenta(self):
        self.assertEqual(self.errores("echo [2/5] " + DROP + "python app.py\n"), [])


if __name__ == "__main__":
    unittest.main()
