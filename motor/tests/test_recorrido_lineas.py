"""Una venta cuyas líneas arma JavaScript («+ Agregar Producto»): sin JavaScript, el recorrido solo enviaba el cliente y
recibía «Agrega al menos un producto», así que la venta nunca se probaba. Una entrega salió con un fallo en esa ruta
(«'sqlite3.Connection' object has no attribute 'last_insert_rowid'») que la web enseñaba con un 200."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.recorrido_web import recorrer
from agente.verificacion import canal_navegador

WEB = r'''import traceback
import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer

MODO = "__MODO__"
FILAS = """<div id="filas"></div><button type="button" onclick="agregar()">__BOTON__</button>"""
SCRIPT = """<script>function agregar(){var d=document.createElement('div');
d.innerHTML='<select name="items"><option value="">--</option><option value="7">Pan (Stock: 0)</option>' +
'<option value="8">Leche</option></select><input type="number" name="items" value="1" min="1">';
document.getElementById('filas').appendChild(d);}</script>"""
VENTA = ('<html><body><form method="post" action="/ventas"><select name="cliente"><option value="1">Ana</option></select>'
         + FILAS + '<button type="submit">Registrar Venta</button></form>' + SCRIPT + '</body></html>')


class Web(BaseHTTPRequestHandler):
    def responder(self, estado, cuerpo):
        self.send_response(estado)
        self.end_headers()
        self.wfile.write(cuerpo.encode())

    def do_GET(self):
        if self.path == "/ventas/nueva":
            return self.responder(200, VENTA)
        self.responder(200, '<a href="/ventas/nueva">Nueva venta</a>')

    def do_POST(self):
        datos = urllib.parse.parse_qs(self.rfile.read(int(self.headers["Content-Length"])).decode())
        items = datos.get("items", [])
        if not items:
            return self.responder(200, "<p>Agrega al menos un producto.</p>" + VENTA)
        if MODO == "flash":
            return self.responder(200, "<p>Error al registrar venta: 'Connection' object has no attribute 'last_insert_rowid'</p>")
        if MODO == "unique":
            return self.responder(200, "<p>Error: UNIQUE constraint failed: ventas.numero</p>")
        try:
            assert items == ["8", "1"], items  # la línea con un producto con stock y una cantidad
            if MODO == "500":
                {}["total"]
        except Exception:
            traceback.print_exc()
            return self.responder(500, "Internal Server Error")
        self.responder(200, "<p>Venta registrada</p>")


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 5000), Web).serve_forever()
'''


def hay_navegador() -> bool:
    try:
        from playwright.sync_api import sync_playwright
        with sync_playwright() as p:
            nav = canal_navegador(p)
            if nav is not None:
                nav.close()
            return nav is not None
    except Exception:
        return False


@unittest.skipUnless(hay_navegador(), "sin navegador")
class VentaConLineasJS(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_lineas_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)

    def recorrer(self, modo: str = "ok", boton: str = "+ Agregar Producto") -> dict:
        (self.dir / "app.py").write_text(WEB.replace("__MODO__", modo).replace("__BOTON__", boton), encoding="utf-8")
        return recorrer(self.dir / "app.py", sys.executable, [], abrir_navegador=canal_navegador)

    def test_la_venta_que_funciona_no_da_errores(self):
        r = self.recorrer()
        self.assertEqual(r["errores"], [])
        self.assertEqual(r["lineas"]["guardados"], 1)  # eligió Leche (Pan está sin stock) y una unidad

    def test_la_venta_que_da_500_se_ve(self):
        r = self.recorrer("500")
        [error] = r["errores"]
        self.assertEqual(error["estado"], 500)
        self.assertIn("guardar el formulario de /ventas/nueva (tras «+ Agregar Producto»)", error["que"])
        self.assertIn("items=8", error["datos"])
        self.assertIn("KeyError", error["traza"])

    def test_el_error_de_programa_que_la_web_enseña_con_un_200_se_ve(self):
        r = self.recorrer("flash")
        [error] = r["errores"]
        self.assertEqual(error["estado"], 200)
        self.assertIn("object has no attribute 'last_insert_rowid'", error["traza"])

    def test_una_restriccion_de_la_base_no_es_un_error_de_programa(self):
        self.assertEqual(self.recorrer("unique")["errores"], [])

    def test_sin_boton_de_anadir_linea_no_se_abre_el_navegador(self):
        r = self.recorrer("ok", boton="Calcular")
        self.assertIsNone(r["lineas"])
        self.assertEqual(r["errores"], [])


if __name__ == "__main__":
    unittest.main()
