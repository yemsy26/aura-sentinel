"""Un .bat que abre en el navegador un puerto distinto del que escucha el programa: con doble clic el usuario ve «no se
puede acceder» aunque el sistema funcione. Lo encontró una revisión, como MEDIO, en una misión del negocio."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.proyecto import Informe
from agente.verificacion import bats, puertos_escucha

RUN = '@echo off\ncd /d "%~dp0"\nstart "" http://localhost:{puerto}\n{arranque}\n'


class PuertoDelBat(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_puerto_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)

    def errores(self, app: str, puerto: int, arranque: str = "python app.py", nombre: str = "app.py") -> list[str]:
        (self.dir / nombre).write_text(app, encoding="utf-8")
        (self.dir / "run.bat").write_text(RUN.format(puerto=puerto, arranque=arranque), encoding="utf-8")
        inf = Informe()
        bats(self.dir, [self.dir / "run.bat"], inf)
        return [e for e in inf.errores if "abre http://localhost" in e]

    def test_puerto_distinto_del_de_la_app(self):
        errores = self.errores('app.run(host="0.0.0.0", port=5055, debug=True)\n', 5000)
        self.assertEqual(len(errores), 1)
        self.assertIn("run.bat:3 abre http://localhost:5000, pero el programa escucha en 5055", errores[0])
        self.assertEqual(self.errores('app.run(host="0.0.0.0", port=5055, debug=True)\n', 5055), [])

    def test_puerto_en_constante_o_variable_de_entorno(self):
        self.assertEqual(self.errores("PORT = 8080\napp.run(port=PORT)\n", 8080), [])
        self.assertTrue(self.errores("PORT = 8080\napp.run(port=PORT)\n", 5000))
        entorno = 'port = int(os.environ.get("PORT", 5055))\napp.run(port=port)\n'
        self.assertEqual(self.errores(entorno, 5055), [])
        self.assertEqual(self.errores(entorno, 5000, 'set PORT=5000\npython app.py'), [])  # el .bat lo cambia

    def test_puertos_por_defecto(self):
        self.assertEqual(self.errores("app.run(debug=True)\n", 5000), [])  # Flask sin puerto: 5000
        self.assertTrue(self.errores("app.run(debug=True)\n", 5055))
        self.assertEqual(self.errores("print('api')\n", 8000, "uvicorn main:app"), [])
        self.assertEqual(self.errores("print('api')\n", 8001, "uvicorn main:app --port 8001"), [])
        self.assertEqual(self.errores("", 8000, "python -m http.server"), [])
        self.assertEqual(self.errores("", 9000, "python -m http.server 9000"), [])

    def test_nombres_y_formas_del_puerto(self):
        """Los sistemas de gastos: WEB_PORT, un parámetro con tipo, PUERTO y argparse; importar http.server no es
        arrancarlo (antes le daba 8000 por defecto y avisaba en falso)."""
        servidor = "from http.server import HTTPServer, BaseHTTPRequestHandler\n"
        self.assertEqual(self.errores(servidor + "WEB_PORT = 8765\nHTTPServer(('127.0.0.1', WEB_PORT), H)\n", 8765), [])
        self.assertEqual(self.errores(servidor + "def iniciar_servidor(port: int = 8765):\n    pass\n", 8765), [])
        self.assertEqual(self.errores(servidor + "PUERTO = 8765\n", 8765), [])
        self.assertEqual(self.errores('p.add_argument("--port", type=int, default=8765)\n', 8765), [])
        self.assertTrue(self.errores(servidor + "WEB_PORT = 8765\n", 8000))

    def test_otros_servidores(self):
        self.assertEqual(self.errores('HTTPServer(("", 8765), Manejador).serve_forever()\n', 8765), [])
        self.assertTrue(self.errores('HTTPServer(("", 8765), Manejador).serve_forever()\n', 8000))
        self.assertEqual(self.errores("app.listen(3000);\n", 3000, "node server.js", "server.js"), [])
        self.assertEqual(self.errores("export default {}\n", 5173, "call npm run dev", "vite.config.js"), [])

    def test_sin_servidor_conocido_o_con_docker_no_se_mira(self):
        self.assertEqual(self.errores("print('hola')\n", 5000), [])  # no se sabe dónde escucha: no se adivina
        self.assertEqual(self.errores("app.run(port=5055)\n", 5000, "docker compose up -d"), [])

    def test_lo_comentado_no_cuenta(self):
        (self.dir / "app.py").write_text("app.run(port=5055)\n", encoding="utf-8")
        (self.dir / "run.bat").write_text('@echo off\nrem antes: http://localhost:5000\n:: http://localhost:5000\n'
                                          'start "" http://localhost:5055\npython app.py\n', encoding="utf-8")
        inf = Informe()
        bats(self.dir, [self.dir / "run.bat"], inf)
        self.assertFalse([e for e in inf.errores if "abre http://localhost" in e])
        self.assertEqual(puertos_escucha(self.dir), {5055})


if __name__ == "__main__":
    unittest.main()
