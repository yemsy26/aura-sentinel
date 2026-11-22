"""La ferretería de una misión (Aura 4.13.14) terminó diciendo «Empleado: usuario `empleado`, contraseña `empleado123`», pero el
programa solo creaba `admin`: `empleado123` solo estaba en test_app.py, donde el test crea ese usuario. claves_inventadas la
aceptaba porque aparecía en un archivo del proyecto; ahora una contraseña que solo existe en los tests se rechaza."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.verificacion import ES_DE_TEST, claves_del_proyecto, claves_inventadas

APP = ('def init_db():\n    conn.execute("INSERT INTO usuario (username, password_hash, rol) VALUES (?, ?, ?)",\n'
       '                 ("admin", generate_password_hash("admin123"), "admin"))\n')
TEST = ('def test_empleado(client):\n    client.post("/usuarios/nuevo", data={"username": "empleado", "password": "empleado123"})\n'
        '    login(client, "empleado", "empleado123")\n    login(client, "admin", "admin123")\n')
RESUMEN = "Credenciales por defecto:\n- Admin: usuario `admin`, contraseña `admin123`\n- Empleado: usuario `empleado`, contraseña `empleado123`\n"


class ClavesEnTests(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_claves_tests_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)

    def escribir(self, ruta: str, texto: str):
        p = self.dir / ruta
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(texto, encoding="utf-8")

    def test_la_contrasena_que_solo_esta_en_los_tests_se_rechaza(self):
        self.escribir("app.py", APP)
        self.escribir("test_app.py", TEST)
        malas = claves_inventadas(self.dir, RESUMEN)
        self.assertIn("«empleado123»", malas)
        self.assertNotIn("«admin123»", malas)  # esa sí la crea app.py
        self.assertIn("solo aparece en los tests (test_app.py)", malas)
        self.assertIn("el programa no la crea", malas)
        self.assertIn("cree ese usuario con esa contraseña", malas)
        self.assertIn("quita la credencial del resumen", malas)

    def test_carpetas_y_nombres_de_tests(self):
        for ruta in ("test_app.py", "app_test.py", "tests/test_x.py", "tests/datos.json", "src/__tests__/a.js", "conftest.py",
                     "web/app.test.js", "web/app.spec.ts", "test.py", "tests.py", "Tests/Leeme.md"):
            with self.subTest(ruta):
                self.assertTrue(ES_DE_TEST.search(ruta), ruta)
        for ruta in ("app.py", "contest.py", "latest.py", "attestation.py", "src/testing_tools.py", "README.md", "seed.py",
                     "protest/app.py", "tests_de_carga.txt"):
            with self.subTest(ruta):
                self.assertFalse(ES_DE_TEST.search(ruta), ruta)

    def test_si_tambien_esta_en_el_codigo_o_en_otro_sitio_se_acepta(self):
        self.escribir("test_app.py", TEST)
        for ruta, texto in (("app.py", APP + 'crear_usuario("empleado", "empleado123", "empleado")\n'),
                            ("seed.py", 'db.add(User("empleado", password="empleado123"))\n'),
                            ("README.md", "Empleado: usuario `empleado`, contraseña: `empleado123`\n"),
                            (".env", "EMPLEADO_PASSWORD=empleado123\n" + 'password = "empleado123"\n'),
                            ("config.json", '{"usuarios": [{"user": "empleado", "password": "empleado123"}]}\n')):
            with self.subTest(ruta):
                self.escribir("app.py", APP)
                self.escribir(ruta, texto)
                self.assertEqual(claves_inventadas(self.dir, RESUMEN), "")
                (self.dir / ruta).unlink()
                self.escribir("app.py", APP)

    def test_lo_que_no_cambia(self):
        self.escribir("app.py", APP)
        self.escribir("test_app.py", TEST)
        self.assertEqual(claves_inventadas(self.dir, "Todo listo. Entra con el usuario que creaste."), "")  # sin contraseña
        self.assertEqual(claves_inventadas(self.dir, "Admin: contraseña `admin123`"), "")  # la crea app.py
        vacio = Path(tempfile.mkdtemp(prefix="agente_claves_vacio_"))
        self.addCleanup(shutil.rmtree, vacio, ignore_errors=True)
        (vacio / "app.py").write_text("print('hola')\n", encoding="utf-8")
        self.assertEqual(claves_inventadas(vacio, "contraseña `secreta99`"), "")  # el proyecto no da ninguna: no se sabe

    def test_la_inventada_que_no_esta_en_ningun_sitio_da_el_mensaje_de_antes(self):
        self.escribir("app.py", APP)
        self.escribir("test_app.py", TEST)
        malas = claves_inventadas(self.dir, "Contraseña `inventada77`")
        self.assertIn("«inventada77», pero no aparece en el proyecto", malas)
        self.assertNotIn("solo aparece en los tests", malas)

    def test_dos_problemas_a_la_vez_dan_los_dos_mensajes(self):
        self.escribir("app.py", APP)
        self.escribir("test_app.py", TEST)
        malas = claves_inventadas(self.dir, RESUMEN + "Contraseña `inventada77`\n")
        self.assertIn("«inventada77», pero no aparece en el proyecto", malas)
        self.assertIn("«empleado123», pero solo aparece en los tests", malas)
        self.assertIn("Además,", malas)

    def test_claves_del_proyecto_sigue_dando_todo_por_defecto(self):
        self.escribir("app.py", APP)
        self.escribir("test_app.py", TEST)
        self.assertIn("empleado123", claves_del_proyecto(self.dir))
        self.assertNotIn("empleado123", claves_del_proyecto(self.dir, sin_tests=True))
        self.assertIn("admin123", claves_del_proyecto(self.dir, sin_tests=True))


if __name__ == "__main__":
    unittest.main()
