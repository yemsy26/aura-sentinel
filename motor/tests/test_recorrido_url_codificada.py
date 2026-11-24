"""m4174/C6: el recorrido web siguió un enlace de la propia app con un espacio sin codificar (/admin/clientes?q=Juan Perez).
http.client lanza InvalidURL (y UnicodeEncodeError con una ñ), que el except de _Recorrido.pedir no atrapaba: la verificación
entera se perdía por un enlace. Ahora pedir() codifica lo que una URL no puede llevar y deja en paz lo ya codificado."""
import shutil
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.recorrido_web import _Recorrido, recorrer

VISTAS: list[str] = []


class Web(BaseHTTPRequestHandler):
    def do_GET(self):
        VISTAS.append(self.path)
        cuerpo = b'<a href="/admin/clientes?q=Juan Perez">Juan</a> <a href="/buscar?q=\xc3\xb1and\xc3\xba">\xc3\xb1</a>'
        self.send_response(200)
        self.end_headers()
        self.wfile.write(cuerpo)

    def log_message(self, *a):
        pass


class PedirCodificaLaUrl(unittest.TestCase):
    def setUp(self):
        VISTAS.clear()
        self.dir = Path(tempfile.mkdtemp(prefix="test_url_cod_"))
        self.log = self.dir / "servidor.log"
        self.log.write_bytes(b"")
        self.servidor = HTTPServer(("127.0.0.1", 0), Web)
        threading.Thread(target=self.servidor.serve_forever, daemon=True).start()
        self.r = _Recorrido(f"http://127.0.0.1:{self.servidor.server_port}", self.log, [])

    def tearDown(self):
        self.servidor.shutdown()
        self.servidor.server_close()
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_un_espacio_sin_codificar_no_tira_el_recorrido(self):
        estado, _, _ = self.r.pedir("/admin/clientes?q=Juan Perez")
        self.assertEqual(estado, 200)
        self.assertEqual(VISTAS, ["/admin/clientes?q=Juan%20Perez"])

    def test_un_caracter_no_ascii_tampoco(self):
        estado, _, _ = self.r.pedir("/buscar?q=ñandú")
        self.assertEqual(estado, 200)
        self.assertEqual(VISTAS, ["/buscar?q=%C3%B1and%C3%BA"])

    def test_lo_ya_codificado_llega_igual(self):
        estado, _, _ = self.r.pedir("/a%20b/c?x=%C3%B1&y=a+b&z=1,2;3")
        self.assertEqual(estado, 200)
        self.assertEqual(VISTAS, ["/a%20b/c?x=%C3%B1&y=a+b&z=1,2;3"])

    def test_un_espacio_en_la_ruta_y_datos_de_un_formulario_get(self):
        estado, _, _ = self.r.pedir("/mis clientes", [("q", "Juan Perez")])
        self.assertEqual(estado, 200)
        self.assertEqual(VISTAS, ["/mis%20clientes?q=Juan+Perez"])


class RecorridoCompleto(unittest.TestCase):
    def setUp(self):
        VISTAS.clear()
        self.dir = Path(tempfile.mkdtemp(prefix="test_url_rec_"))
        (self.dir / "app.py").write_text(
            "from http.server import BaseHTTPRequestHandler, HTTPServer\n"
            "from pathlib import Path\n"
            "VISITAS = Path(__file__).with_name('visitas.txt')\n"
            "class Web(BaseHTTPRequestHandler):\n"
            "    def do_GET(self):\n"
            "        VISITAS.open('a').write(self.path + '\\n')\n"
            "        self.send_response(200)\n"
            "        self.end_headers()\n"
            "        self.wfile.write('<a href=\"/admin/clientes?q=Juan Perez\">Juan</a> <a href=\"/buscar?q=ñandú\">n</a>'.encode())\n"
            "if __name__ == '__main__':\n"
            "    HTTPServer(('0.0.0.0', 5000), Web).serve_forever()\n", encoding="utf-8")

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_un_enlace_con_espacio_se_visita_y_no_se_pierde_el_recorrido(self):
        r = recorrer(self.dir / "app.py", sys.executable, [])
        self.assertIsNotNone(r)
        visitas = (self.dir / "visitas.txt").read_text(encoding="utf-8").split()
        self.assertIn("/admin/clientes?q=Juan%20Perez", visitas)
        self.assertIn("/buscar?q=%C3%B1and%C3%BA", visitas)
        self.assertEqual(r["errores"], [])


if __name__ == "__main__":
    unittest.main()
