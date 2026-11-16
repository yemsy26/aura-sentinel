"""Un menú que pinta JavaScript (m4153c_neg1: `{ href: '/productos' }` en static/js/nav.js): el HTML no trae ni un <a> y el recorrido sin
navegador visitó 4 de 17 rutas; nunca abrió /productos, no dio de alta el producto de la prueba y no probó «nueva venta» con productos.
Ahora lee las rutas de navegación que declaran los scripts de la página (inline y del mismo origen)."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.recorrido_web import JS_HREF, recorrer

WEB_MENU = r'''from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

VISITAS = Path(__file__).with_name("visitas.txt")
NAV = """(function () {
  var links = [
    { href: '/productos', label: 'Productos', visible: true },
    { href: "/clientes", label: 'Clientes' },
    { href: '/informe', label: 'Informe' },
    { href: '/no-existe', label: 'Fantasma' },
    { href: '/solo-post', label: 'Solo POST' },
    { href: '/logout', label: 'Salir' },
    { href: '/productos/editar/', label: 'trozo' },
    { href: '/sitio?x=1', label: 'con consulta' },
    { href: `/plantilla/${id}`, label: 'plantilla' }
  ];
  var m = document.getElementById('nav-menu');
  links.forEach(function (l) { var a = document.createElement('a'); a.href = l.href; m.appendChild(a); });
})();"""
PAGINAS = {
    "/": '<nav id="nav-menu"></nav><a href="/roto">Roto</a><script src="/static/nav.js" defer></script>'
         '<script src="https://cdn.example.com/lib.js"></script>'
         '<script type="application/json" id="datos">{"href": "/dato-no-es-pagina"}</script>'
         '<script>window.location.href = "/ajustes";</script>',
    "/productos": "<p>productos</p>", "/clientes": "<p>clientes</p>", "/ajustes": "<p>ajustes</p>",
    "/sitio": "<p>sitio</p>",
}


class Web(BaseHTTPRequestHandler):
    def responder(self, estado, cuerpo, tipo="text/html", cabeceras=()):
        self.send_response(estado)
        self.send_header("Content-Type", tipo)
        for k, v in cabeceras:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(cuerpo.encode())

    def do_GET(self):
        with open(VISITAS, "a") as f:
            f.write(self.path + "\n")
        if self.path == "/static/nav.js":
            return self.responder(200, NAV, "application/javascript")
        dentro = "sesion=ok" in (self.headers.get("Cookie") or "")
        if not dentro and self.path != "/entrar":
            return self.responder(302, "", cabeceras=[("Location", "/entrar")])
        if self.path == "/entrar":
            return self.responder(200, '<form method="post" action="/entrar"><input name="usuario">'
                                       '<input type="password" name="clave"><button>Entrar</button></form>')
        if self.path == "/solo-post":
            return self.responder(405, "no")
        if self.path == "/informe":
            raise RuntimeError("el informe falla al abrirse")
        if self.path in PAGINAS:
            return self.responder(200, PAGINAS[self.path])
        self.responder(404, "no")

    def do_POST(self):
        self.rfile.read(int(self.headers["Content-Length"]))
        self.responder(302, "", cabeceras=[("Location", "/"), ("Set-Cookie", "sesion=ok")])


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 5000), Web).serve_forever()
'''


class MenuConJavaScript(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_menu_js_"))
        (self.dir / "app.py").write_text(WEB_MENU, encoding="utf-8")
        self.r = recorrer(self.dir / "app.py", sys.executable, [("ana", "pan123")])
        self.visitas = (self.dir / "visitas.txt").read_text().split()

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_abre_las_paginas_que_enlaza_el_menu_de_javascript(self):
        self.assertEqual(self.r["usuario"], "ana")
        for ruta in ("/productos", "/clientes", "/ajustes", "/informe"):
            self.assertIn(ruta, self.visitas, ruta)

    def test_lo_que_no_es_una_pagina_no_se_abre(self):
        for ruta in ("/logout", "/productos/editar/", "/dato-no-es-pagina", "/sitio", "/sitio?x=1"):
            self.assertNotIn(ruta, self.visitas, ruta)
        self.assertFalse([v for v in self.visitas if v.startswith("/plantilla")])
        self.assertFalse([v for v in self.visitas if "cdn.example.com" in v])

    def test_una_ruta_que_solo_conoce_el_javascript_y_no_existe_no_es_un_enlace_roto(self):
        self.assertIn("/no-existe", self.visitas)
        self.assertIn("/solo-post", self.visitas)
        self.assertEqual(self.r["rotos"], ["abrir /roto"])  # el enlace del HTML sí sigue contando
        self.assertEqual(self.r["raros"], [])

    def test_si_la_pagina_del_menu_falla_al_abrirla_es_un_error(self):
        self.assertIn("abrir /informe", [e["que"] for e in self.r["errores"]])

    def test_el_script_se_pide_una_vez_por_recorrido(self):
        # Dos recorridos (el de antes, que prueba al empleado, y el de siempre): cada uno lo pide una vez, no una por página.
        self.assertLessEqual(self.visitas.count("/static/nav.js"), 2)


class Literales(unittest.TestCase):
    def rutas(self, codigo):
        return [ruta for _, ruta in JS_HREF.findall(codigo)]

    def test_formas_de_declarar_una_ruta(self):
        self.assertEqual(self.rutas("{ href: '/a' }, { \"href\": \"/b\" }, x.href = '/c'; location.href='/d'"), ["/a", "/b", "/c", "/d"])

    def test_lo_que_no_cuenta(self):
        self.assertEqual(self.rutas("{ data_href: '/a' } { href: 'https://x.com/b' } { href: `/c/${id}` } { href: '/d?x=1' } { href: '#e' }"), [])


if __name__ == "__main__":
    unittest.main()
