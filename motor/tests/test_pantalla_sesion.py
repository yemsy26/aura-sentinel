"""ver_pantalla abría un navegador nuevo en cada captura y la sesión se perdía: una misión capturó /login/admin y luego
/agenda, que salía como la pantalla de entrar, y gastó pasos escribiendo un script para probar con sesión."""
import shutil
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import pantalla
from agente.herramientas import Herramientas


class App(BaseHTTPRequestHandler):
    visitas: list = []  # noqa: RUF012 -- se reinicia en cada prueba

    def do_GET(self):
        dentro = "sesion=duena" in (self.headers.get("Cookie") or "")
        App.visitas.append((self.path, dentro))
        self.send_response(200)
        if self.path == "/login":
            self.send_header("Set-Cookie", "sesion=duena; Path=/")
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.end_headers()
        cuerpo = "<h1>Agenda del día</h1>" if dentro else "<h1>Elige: dueña o empleada</h1>"
        self.wfile.write(cuerpo.encode("utf-8"))

    def log_message(self, *a):
        pass


class SesionDelNavegador(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_sesion_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)
        App.visitas = []
        self.servidor = ThreadingHTTPServer(("127.0.0.1", 0), App)
        threading.Thread(target=self.servidor.serve_forever, daemon=True).start()
        self.addCleanup(self.servidor.server_close)
        self.addCleanup(self.servidor.shutdown)
        self.url = f"http://127.0.0.1:{self.servidor.server_port}"
        self.sesion = self.dir / ".agente" / pantalla.SESION
        self.sesion.parent.mkdir()

    def captura(self, ruta: str) -> None:
        try:
            pantalla.capturar_web(self.dir, self.url + ruta, self.dir / "c.png", 0.5, self.sesion)
        except Exception as e:  # sin Edge/Chrome no se puede probar
            self.skipTest(f"navegador no disponible: {e}")

    def test_la_sesion_pasa_a_la_siguiente_captura(self):
        self.captura("/agenda")
        self.captura("/login")
        self.captura("/agenda")
        agenda = [dentro for ruta, dentro in App.visitas if ruta == "/agenda"]
        self.assertEqual(agenda, [False, True])
        self.assertTrue(pantalla._cookies(self.sesion))

    def test_una_tarea_nueva_empieza_sin_sesion(self):
        self.captura("/login")
        self.assertTrue(self.sesion.is_file())
        h = Herramientas(self.dir, max_chars=3000, timeout=30)
        self.addCleanup(h.detener_todos)
        h.nueva_tarea(1, "continúa")  # el texto no vacío es un mandato nuevo
        self.assertFalse(self.sesion.is_file())


if __name__ == "__main__":
    unittest.main()
