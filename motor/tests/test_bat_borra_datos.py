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
SEED_BORRA = "from app import create_app\nfrom models import db\n\nwith create_app().app_context():\n    db.drop_all()\n    db.create_all()\n"
SQL_BORRA = 'import sqlite3\nc = sqlite3.connect("x.db")\nc.execute("DROP TABLE IF EXISTS ventas")\n'


class BatQueBorraDatos(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_borra_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)

    def errores(self, cuerpo: str, nombre: str = "run.bat", **scripts: str) -> list[str]:
        (self.dir / nombre).write_text(CABECERA + cuerpo, encoding="utf-8")
        (self.dir / "app.py").write_text("print('hola')\n", encoding="utf-8")
        for archivo, fuente in scripts.items():
            (self.dir / (archivo + ".py")).write_text(fuente, encoding="utf-8")
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

    def test_un_script_que_lanza_el_bat_y_borra_las_tablas(self):
        errores = self.errores("python seed.py\npython app.py\n", seed=SEED_BORRA)
        self.assertEqual(len(errores), 1)
        self.assertIn("run.bat:3 ejecuta seed.py, que borra la base de datos en cada arranque (`drop_all(` en seed.py:5)", errores[0])
        self.assertTrue(self.errores("python init_db.py\npython app.py\n", init_db=SQL_BORRA))
        self.assertTrue(self.errores("python init_db.py\n", init_db='import os\nos.remove("negocio.db")\n'))
        self.assertTrue(self.errores("python app.py\n", app=SQL_BORRA))  # el propio programa que abre el .bat

    def test_un_script_que_borra_a_proposito_o_con_condicion_no_se_marca(self):
        self.assertEqual(self.errores("python seed.py\n", seed="def reset_db():\n    db.drop_all()\n"), [])
        self.assertEqual(self.errores("python seed.py\n", seed='import sys\nif "--reset" in sys.argv:\n    db.drop_all()\n'), [])
        existe = "import os\nif not os.path.exists('x.db'):\n    db.drop_all()\n    db.create_all()\n"
        self.assertEqual(self.errores("python seed.py\n", seed=existe), [])
        self.assertEqual(self.errores("python seed.py\n", seed="db.create_all()\n"), [])
        self.assertEqual(self.errores("python seed.py\n", "reset.bat", seed=SEED_BORRA), [])
        self.assertEqual(self.errores("python test_seed.py\n", test_seed="db.drop_all()\n"), [])
        self.assertEqual(self.errores("python seed.py\n", seed="def (:\n"), [])  # no es Python válido


if __name__ == "__main__":
    unittest.main()
