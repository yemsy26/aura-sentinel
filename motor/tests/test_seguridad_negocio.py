"""Dos fallos de la entrega de un negocio que funcionaba (la prueba del usuario con la 4.13.4) que verificar no veía:
- test.py borraba instance/negocio.db, la base real de la app: quien pulsa test.bat pierde todos sus datos;
- /register abierto a cualquiera en un sistema con roles: cualquiera se crea una cuenta y ve lo de dentro."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.proyecto import Informe, archivos
from agente.seguridad import pruebas_borran_la_base, seguridad_web
from agente.verificacion import informe_proyecto

MODELS = '''import os
DB_PATH = os.path.join(os.path.dirname(__file__), "instance", "negocio.db")
def init_db():
    os.makedirs(os.path.dirname(DB_PATH), exist_ok=True)
'''
TEST_REAL = '''import os, unittest
from models import init_db
class T(unittest.TestCase):
    def setUp(self):
        db_path = os.path.join(os.path.dirname(__file__), "instance", "negocio.db")
        if os.path.exists(db_path):
            os.remove(db_path)
        init_db()
    def test_algo(self):
        self.assertTrue(True)
'''
TEST_MIXTO = '''import tempfile, unittest
from pathlib import Path
from models import init_db
class T(unittest.TestCase):
    def test_con_base_propia(self):
        carpeta = tempfile.mkdtemp()
        self.assertTrue(carpeta)
    def test_cli(self):
        db_real = Path(__file__).parent / "instance" / "negocio.db"
        if db_real.exists():
            db_real.unlink()
'''
TEST_TEMP = '''import os, tempfile, unittest
class T(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.db = os.path.join(self.dir, "negocio.db")
    def tearDown(self):
        if os.path.exists(self.db):
            os.remove(self.db)
    def test_algo(self):
        self.assertTrue(True)
'''
TEST_AJENA = TEST_REAL.replace('"negocio.db"', '"test_negocio.db"')
TEST_CHDIR = '''import os, tempfile, unittest
class T(unittest.TestCase):
    def setUp(self):
        os.chdir(tempfile.mkdtemp())
        if os.path.exists("negocio.db"):
            os.remove("negocio.db")
    def test_algo(self):
        self.assertTrue(True)
'''
TEST_CSV = '''import os, unittest
class T(unittest.TestCase):
    def tearDown(self):
        os.remove(os.path.join(os.path.dirname(__file__), "salida.csv"))
    def test_algo(self):
        self.assertTrue(True)
'''

REGISTRO = '''from flask import Flask, request, render_template
from werkzeug.security import generate_password_hash
app = Flask(__name__)

def rol_de(u):
    return u.rol  # el sistema tiene roles

@app.route("/register", methods=["GET", "POST"])
__DECORADOR__
def register():
    if request.method == "POST":
        __CUERPO__
    return render_template("register.html")
'''
CREA = 'h = generate_password_hash(request.form["password"])'
CREA_GUARDADO = 'if current_user.is_authenticated and current_user.rol == "admin":\n            h = generate_password_hash(request.form["password"])'


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_segneg_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)

    def proyecto(self, **archivos_: str) -> list[Path]:
        for nombre, texto in archivos_.items():
            (self.dir / f"{nombre}.py").write_text(texto, encoding="utf-8")
        return archivos(self.dir, ".py")


class TestsBorranLaBase(Base):
    def errores(self, **py: str) -> list[str]:
        inf = Informe()
        pruebas_borran_la_base(self.dir, self.proyecto(**py), inf)
        return inf.errores

    def test_el_setup_que_borra_la_base_real(self):
        [e] = self.errores(models=MODELS, test_app=TEST_REAL)
        self.assertIn("test_app.py:7 borra negocio.db", e)
        self.assertIn("pierde todo lo que haya guardado", e)

    def test_aunque_el_archivo_use_tempfile_en_otras_pruebas(self):
        # prueba_aura_4111_cli1: una prueba de CLI «limpia la DB real» en un archivo que en otras usa bases temporales.
        [e] = self.errores(models=MODELS, test_app=TEST_MIXTO)
        self.assertIn("borra negocio.db", e)

    def test_con_una_base_temporal_no_se_marca(self):
        self.assertEqual(self.errores(models=MODELS, test_app=TEST_TEMP), [])

    def test_si_borra_otra_base_que_la_app_no_usa_no_se_marca(self):
        self.assertEqual(self.errores(models=MODELS, test_app=TEST_AJENA), [])

    def test_si_cambia_de_carpeta_y_borra_un_nombre_suelto_no_se_marca(self):
        self.assertEqual(self.errores(models=MODELS, test_app=TEST_CHDIR), [])

    def test_borrar_un_csv_no_es_borrar_la_base(self):
        self.assertEqual(self.errores(models=MODELS, test_app=TEST_CSV), [])

    def test_sin_tests_no_hay_nada(self):
        self.assertEqual(self.errores(models=MODELS), [])

    def test_va_en_el_informe_de_verificar(self):
        self.proyecto(models=MODELS, test_app=TEST_REAL)
        inf = informe_proyecto(self.dir, sys.executable, segundos=2)
        self.assertTrue(any("borra negocio.db" in e for e in inf.errores), inf.errores)


class RegistroAbierto(Base):
    def avisos(self, decorador: str = "", cuerpo: str = CREA, **otros: str) -> list[str]:
        fuente = REGISTRO.replace("__DECORADOR__", decorador).replace("__CUERPO__", cuerpo)
        inf = Informe()
        seguridad_web(self.dir, self.proyecto(app=fuente, **otros), inf)
        return inf.avisos

    def test_registro_abierto_en_un_sistema_con_roles(self):
        [a] = self.avisos()
        self.assertIn("app.py:", a)
        self.assertIn("/register deja que cualquiera se cree una cuenta sin entrar", a)

    def test_va_en_el_informe_de_verificar(self):
        fuente = REGISTRO.replace("__DECORADOR__", "").replace("__CUERPO__", CREA)
        self.proyecto(app=fuente)
        inf = informe_proyecto(self.dir, sys.executable, segundos=2)
        self.assertTrue(any("cualquiera se cree una cuenta" in a for a in inf.avisos), inf.avisos)

    def test_con_un_decorador_de_rol_no_se_avisa(self):
        self.assertEqual(self.avisos(decorador="@requerir_admin"), [])

    def test_si_crear_la_cuenta_va_dentro_de_un_if_de_admin_no_se_avisa(self):
        self.assertEqual(self.avisos(cuerpo=CREA_GUARDADO), [])

    def test_sin_roles_no_se_avisa(self):
        fuente = REGISTRO.replace("__DECORADOR__", "").replace("__CUERPO__", CREA).replace("u.rol", "u.nombre").replace("roles", "nombres")
        inf = Informe()
        seguridad_web(self.dir, self.proyecto(app=fuente), inf)
        self.assertEqual(inf.avisos, [])

    def test_otra_ruta_no_se_confunde(self):
        fuente = REGISTRO.replace("/register", "/ventas").replace("__DECORADOR__", "").replace("__CUERPO__", CREA)
        inf = Informe()
        seguridad_web(self.dir, self.proyecto(app=fuente), inf)
        self.assertEqual(inf.avisos, [])


if __name__ == "__main__":
    unittest.main()
