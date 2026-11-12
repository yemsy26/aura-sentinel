"""Tres fallos de entregas reales que el recorrido de verificar no veía (juicio de m4136):
- el administrador entra, pero la web le cierra sus propias secciones («No tienes permiso»: el rol llegaba mal al usuario);
- la página de entrar a la que manda «/» sale vacía (login.html no rellenaba el bloque de la plantilla base);
- las altas son un modal que guarda con fetch (sin <form>): el recorrido sin JavaScript no creaba nada, y la página de
  productos, vacía abría bien pero daba 500 en cuanto había uno."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.recorrido_web import recorrer
from agente.verificacion import canal_navegador

WEB_PERMISO = r'''import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer

CIERRA = "__CIERRA__"  # "redirige", "403" o "nada"


class Web(BaseHTTPRequestHandler):
    def responder(self, estado, cuerpo, cabeceras=()):
        self.send_response(estado)
        for k, v in cabeceras:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(cuerpo.encode())

    def do_GET(self):
        dentro = "sesion=ok" in (self.headers.get("Cookie") or "")
        if not dentro and self.path != "/entrar":
            return self.responder(302, "", [("Location", "/entrar")])
        if self.path == "/entrar":
            return self.responder(200, '<form method="post" action="/entrar"><input name="usuario">'
                                       '<input type="password" name="clave"><button>Entrar</button></form>')
        if self.path == "/":
            return self.responder(200, '<a href="/productos/nuevo">Nuevo producto</a> <a href="/ventas">Ventas</a>')
        if self.path == "/productos/nuevo" and CIERRA == "redirige":
            return self.responder(302, "", [("Location", "/inicio")])
        if self.path == "/productos/nuevo" and CIERRA == "403":
            return self.responder(403, "Forbidden")
        if self.path == "/inicio":
            return self.responder(200, "<p>No tienes permiso para acceder a esta sección.</p>")
        self.responder(200, "<p>pagina</p>")

    def do_POST(self):
        datos = dict(urllib.parse.parse_qsl(self.rfile.read(int(self.headers["Content-Length"])).decode()))
        if datos.get("clave") == "pan123":
            return self.responder(302, "", [("Location", "/"), ("Set-Cookie", "sesion=ok")])
        self.responder(200, "<p>no</p>")


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 5000), Web).serve_forever()
'''

WEB_LOGIN = r'''from http.server import BaseHTTPRequestHandler, HTTPServer

LOGIN = """__LOGIN__"""


class Web(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/":
            self.send_response(302)
            self.send_header("Location", "/login?next=%2F")
            self.end_headers()
            return
        self.send_response(200)
        self.end_headers()
        self.wfile.write(LOGIN.encode())


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 5000), Web).serve_forever()
'''

WEB_MODAL = r'''import json
import traceback
from http.server import BaseHTTPRequestHandler, HTTPServer

PRODUCTOS = []
FALLA = "__FALLA__"
NAV = '<nav><a href="/">Inicio</a> <a href="/productos">Productos</a></nav>'
PAGINA = NAV + """<h1>Productos</h1><ul>__ITEMS__</ul>
<button type="button" onclick="document.getElementById('modal').style.display='block'">Nuevo producto</button>
<div id="modal" style="display:none"><input name="nombre" type="text"><input name="precio" type="number">
<button type="button" onclick="guardar()">Guardar</button></div>
<script>function guardar(){fetch('/api/productos',{method:'POST',headers:{'Content-Type':'application/json'},
body:JSON.stringify({nombre:document.querySelector('[name=nombre]').value,
precio:document.querySelector('[name=precio]').value})}).then(()=>location.reload());}</script>"""


class Web(BaseHTTPRequestHandler):
    def responder(self, estado, cuerpo):
        self.send_response(estado)
        self.end_headers()
        self.wfile.write(cuerpo.encode())

    def do_GET(self):
        if self.path == "/productos":
            try:
                if PRODUCTOS and FALLA == "si":
                    json.dumps(object())  # el fallo de la entrega: un objeto que no se puede pasar a JSON
                items = "".join(f"<li>{p}</li>" for p in PRODUCTOS)
                return self.responder(200, "<html><body>" + PAGINA.replace("__ITEMS__", items) + "</body></html>")
            except Exception:
                traceback.print_exc()
                return self.responder(500, "Internal Server Error")
        self.responder(200, "<html><body>" + NAV + '<p>Inicio</p><a href="/productos">Ir a productos</a></body></html>')

    def do_POST(self):
        PRODUCTOS.append(json.loads(self.rfile.read(int(self.headers["Content-Length"])))["nombre"])
        self.responder(200, "{}")


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


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_permiso_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)

    def web(self, fuente: str, **cambios: str) -> Path:
        for k, v in cambios.items():
            fuente = fuente.replace(f"__{k}__", v)
        (self.dir / "app.py").write_text(fuente, encoding="utf-8")
        return self.dir / "app.py"


class AdminSinPermiso(Base):
    def recorrido(self, cierra: str, usuario: str = "admin") -> dict:
        return recorrer(self.web(WEB_PERMISO, CIERRA=cierra), sys.executable, [(usuario, "pan123")])

    def test_el_admin_al_que_la_web_manda_a_otra_pagina_con_no_tienes_permiso(self):
        r = self.recorrido("redirige")
        [s] = r["sin_permiso"]
        self.assertEqual((s["usuario"], s["ruta"], s["a"]), ("admin", "/productos/nuevo", "/inicio"))
        self.assertIn("No tienes permiso", s["frase"])

    def test_el_admin_con_un_403(self):
        [s] = self.recorrido("403")["sin_permiso"]
        self.assertEqual((s["ruta"], s["a"], s["frase"]), ("/productos/nuevo", None, "403"))

    def test_un_vendedor_sin_permiso_es_normal(self):
        self.assertEqual(self.recorrido("redirige", usuario="vendedor1")["sin_permiso"], [])

    def test_si_la_web_lo_deja_pasar_no_hay_nada(self):
        self.assertEqual(self.recorrido("nada")["sin_permiso"], [])


class LoginVacio(Base):
    def login(self, cuerpo: str) -> dict:
        return recorrer(self.web(WEB_LOGIN, LOGIN=cuerpo), sys.executable, [("admin", "x")])

    def test_la_pagina_de_entrar_sin_nada(self):
        vacia = "<html><head><title>Iniciar sesión</title></head><body><main><div><!-- Flash messages --></div></main></body></html>"
        self.assertEqual(self.login(vacia)["login_vacio"], "/login?next=%2F")

    def test_con_formulario_no_es_vacia(self):
        form = '<form method="post"><input name="u"><input type="password" name="p"><button>Entrar</button></form>'
        self.assertIsNone(self.login(f"<html><body>{form}</body></html>")["login_vacio"])

    def test_una_web_que_pinta_con_javascript_no_es_vacia(self):
        self.assertIsNone(self.login('<html><body><div id="root"></div><script src="/app.js"></script></body></html>')["login_vacio"])

    def test_un_enlace_para_entrar_con_otro_servicio_no_es_vacia(self):
        self.assertIsNone(self.login('<html><body><a href="https://accounts.example.com/oauth">Entrar con Google</a></body></html>')["login_vacio"])


@unittest.skipUnless(hay_navegador(), "sin navegador")
class AltaPorModal(Base):
    def recorrido(self, falla: str) -> dict:
        return recorrer(self.web(WEB_MODAL, FALLA=falla), sys.executable, [], abrir_navegador=canal_navegador)

    def test_la_pagina_que_da_500_cuando_hay_un_producto_se_ve(self):
        r = self.recorrido("si")
        self.assertIsNotNone(r["navegador"])
        self.assertTrue(any(e["estado"] == 500 and "Productos" in e["que"] for e in r["errores"]), r["errores"])

    def test_si_no_falla_con_datos_no_hay_errores(self):
        r = self.recorrido("no")
        self.assertIsNotNone(r["navegador"])
        self.assertEqual(r["errores"], [])

    def test_sin_navegador_no_hace_nada(self):
        r = recorrer(self.web(WEB_MODAL, FALLA="si"), sys.executable, [])
        self.assertIsNone(r["navegador"])
        self.assertEqual(r["errores"], [])


if __name__ == "__main__":
    unittest.main()
