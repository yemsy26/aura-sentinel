"""El recorrido web de verificar (P3): una misión de negocio entregó una web que daba 500 al registrar una venta, con
los tests en verde y la revisión aprobada, porque verificar solo miraba que el servidor arrancara. Ahora la usa de
verdad: entra, abre las páginas y envía los formularios con datos válidos. La web de estas pruebas es de http.server
(sin Flask ni red): escucha en el puerto que le da el arranque aunque su código diga otro."""
import importlib.util
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.herramientas import Herramientas
from agente.recorrido_web import _forma, recorrer

WEB = r'''import traceback
import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

VISITAS = Path(__file__).with_name("visitas.txt")
PAGINAS = {
    "/entrar": '<form method="post" action="/entrar"><input name="usuario"><input type="password" name="clave">'
               '<button>Entrar</button></form>',
    "/": '<a href="/ventas/nueva">Nueva venta</a> <a href="/productos/1">Pan</a> <a href="/productos/2">Leche</a> '
         '<a href="/productos/3">Huevos</a> <a href="/borrar/1">Borrar</a> <a href="/salir">Salir</a> '
         '<a href="/estilo.css">css</a> <a href="/no-existe">Roto</a> <a href="/ajuste">Ajuste</a>',
    "/ajuste": '<form method="post" action="/ajuste"><select name="producto" required><option value="">Elegir</option>'
               '</select><input type="number" name="cantidad" value="0"><button>Ajustar</button></form>',
    "/ventas/nueva": '<form method="post" action="/ventas"><select name="producto"><option value="">Elegir</option>'
                     '<option value="7">Pan</option></select><input type="number" name="cantidad" min="1">'
                     '<input type="date" name="fecha"><input type="hidden" name="csrf" value="abc">'
                     '<button>Vender</button></form>',
}


class Web(BaseHTTPRequestHandler):
    def responder(self, estado, cuerpo, cabeceras=()):
        self.send_response(estado)
        for k, v in cabeceras:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(cuerpo.encode())

    def do_GET(self):
        with open(VISITAS, "a") as f:
            f.write(self.path + "\n")
        dentro = "sesion=ok" in (self.headers.get("Cookie") or "")
        if not dentro and self.path != "/entrar":
            return self.responder(302, "", [("Location", "/entrar")])
        if self.path.startswith("/productos/"):
            return self.responder(200, "<p>producto</p>")
        if self.path in PAGINAS:
            return self.responder(200, PAGINAS[self.path])
        self.responder(404, "no")

    def do_POST(self):
        datos = dict(urllib.parse.parse_qsl(self.rfile.read(int(self.headers["Content-Length"])).decode()))
        if self.path == "/entrar":
            if datos == {"usuario": "ana", "clave": "pan123"}:
                return self.responder(302, "", [("Location", "/"), ("Set-Cookie", "sesion=ok")])
            return self.responder(200, PAGINAS["/entrar"])
        try:
            precio = {"7": 2.5}[datos["producto"]] * int(datos["cantidad"])
            assert datos["csrf"] == "abc" and len(datos["fecha"]) == 10
            precio / 0  # el fallo de la web: vender da 500
        except Exception:
            traceback.print_exc()
            return self.responder(500, "Internal Server Error")


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 5000), Web).serve_forever()
'''


class Recorrido(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_recorrido_"))
        (self.dir / "app.py").write_text(WEB, encoding="utf-8")

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_entra_y_envia_el_formulario_que_falla(self):
        r = recorrer(self.dir / "app.py", sys.executable, [("admin", "admin"), ("ana", "pan123")])
        self.assertEqual(r["usuario"], "ana")
        [error] = r["errores"]
        self.assertEqual((error["que"], error["estado"]), ("enviar el formulario de /ventas/nueva (POST /ventas)", 500))
        self.assertIn("producto=7, cantidad=1, fecha=", error["datos"])
        self.assertNotIn("csrf", error["datos"])
        self.assertIn("app.py", error["traza"])
        self.assertIn("ZeroDivisionError", error["traza"])
        self.assertEqual(r["rotos"], ["abrir /no-existe"])
        visitas = (self.dir / "visitas.txt").read_text().split()
        self.assertNotIn("/borrar/1", visitas)  # ni borra ni cierra la sesión
        self.assertNotIn("/salir", visitas)
        self.assertEqual(len({v for v in visitas if v.startswith("/productos/")}), 2)  # dos de la misma forma bastan

    def test_sin_usuarios_validos_lo_dice(self):
        r = recorrer(self.dir / "app.py", sys.executable, [("admin", "admin")])
        self.assertTrue(r["sin_entrar"])
        self.assertEqual(r["errores"], [])

    def test_lo_que_no_es_un_servidor(self):
        (self.dir / "app.py").write_text("print('hola')\n", encoding="utf-8")
        self.assertIsNone(recorrer(self.dir / "app.py", sys.executable, []))

    def test_forma_de_una_ruta(self):
        self.assertEqual(_forma("/productos/7/editar?pagina=2&orden=x"), "/productos/{n}/editar?orden&pagina")
        self.assertEqual(_forma("/"), "/")


WEB_CON_DATOS = r'''import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer

PRODUCTOS = []


class Web(BaseHTTPRequestHandler):
    def responder(self, estado, cuerpo, cabeceras=()):
        self.send_response(estado)
        for k, v in cabeceras:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(cuerpo.encode())

    def do_GET(self):
        if self.path == "/":
            return self.responder(200, '<a href="/ventas/nueva">Vender</a> <a href="/productos/nuevo">Nuevo</a>')
        if self.path == "/productos/nuevo":
            return self.responder(200, '<form method="post" action="/productos"><input name="nombre"><button>Crear</button></form>')
        if self.path == "/ventas/nueva":
            if PRODUCTOS:  # como la web que pasaba filas de la base de datos a JSON: falla en cuanto hay productos
                print("Traceback (most recent call last):\n  File \"app.py\", line 9, in nueva\nTypeError: Row no es JSON", flush=True)
                return self.responder(500, "Internal Server Error")
            return self.responder(200, "<p>No hay productos</p>")
        self.responder(404, "no")

    def do_POST(self):
        PRODUCTOS.append(dict(urllib.parse.parse_qsl(self.rfile.read(int(self.headers["Content-Length"])).decode())))
        self.responder(302, "", [("Location", "/")])


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 5000), Web).serve_forever()
'''

FLASK_PLANTILLA = r'''from flask import Flask, render_template

app = Flask(__name__)


@app.route("/")
def inicio():
    return render_template("inicio.html")


@app.route("/ventas/<int:i>")
def venta(i):
    return render_template("venta.html", total=5)


if __name__ == "__main__":
    app.run(port=5000, debug=True)
'''
# Flask no está en el entorno del motor: la prueba de plantillas usa un Python que lo tenga (si no hay, se salta).
PY_FLASK = os.environ.get("AURA_PRUEBA_PY_FLASK") or (sys.executable if importlib.util.find_spec("flask") else None)


class SegundaVuelta(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_recorrido_"))

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_vuelve_a_las_paginas_cuando_ya_hay_datos(self):
        (self.dir / "app.py").write_text(WEB_CON_DATOS, encoding="utf-8")
        r = recorrer(self.dir / "app.py", sys.executable, [])
        [error] = r["errores"]
        self.assertEqual((error["que"], error["estado"]), ("abrir /ventas/nueva", 500))
        self.assertIn("Row no es JSON", error["traza"])

    @unittest.skipUnless(PY_FLASK, "sin un Python con Flask")
    def test_plantilla_que_no_compila_aunque_nadie_la_abra(self):
        (self.dir / "app.py").write_text(FLASK_PLANTILLA, encoding="utf-8")
        (self.dir / "templates").mkdir()
        (self.dir / "templates" / "inicio.html").write_text("<p>hola</p>", encoding="utf-8")
        (self.dir / "templates" / "venta.html").write_text("<p>{{ total:.2f }}</p>", encoding="utf-8")
        r = recorrer(self.dir / "app.py", PY_FLASK, [])
        [plantilla] = r["plantillas"]
        self.assertEqual((plantilla["plantilla"], plantilla["linea"]), ("venta.html", 1))
        self.assertIn("got ':'", plantilla["error"])
        self.assertEqual(r["errores"], [])


class EnVerificar(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_recorrido_"))
        (self.dir / "app.py").write_text(WEB, encoding="utf-8")
        (self.dir / "README.md").write_text("Entra con usuario `ana` y contraseña `pan123`.\n", encoding="utf-8")

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_verificar_da_el_error_de_la_web(self):
        r = Herramientas(self.dir).verificar_cmd()
        self.assertIn("app.py: al enviar el formulario de /ventas/nueva (POST /ventas) con producto=7", r)
        self.assertIn("la web responde 500", r)
        self.assertIn("ZeroDivisionError", r)
        self.assertNotIn("agente_web_", r)  # la ruta de la copia no sale
        self.assertFalse((self.dir / "visitas.txt").exists())  # se usó una copia


if __name__ == "__main__":
    unittest.main()
