"""Dos fallos de entregas reales que el recorrido con navegador de verificar no veía (juicio de m4137):
- la pantalla de entrar existe en la página, pero su contenedor tiene display:none (el CSS no define la clase que lo
  enseña): sale en blanco y nadie puede entrar;
- se entra bien, pero la pantalla manda sus fetch sin la cabecera Authorization y la API los rechaza con 401: las
  listas y el panel salen vacíos."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.recorrido_web import recorrer
from agente.verificacion import canal_navegador

WEB_API = r'''import json
from http.server import BaseHTTPRequestHandler, HTTPServer

CABECERA = "__CABECERA__"  # "con": el fetch lleva Authorization; "sin": no la lleva
CIERRA = "__CIERRA__"  # "401", "403" u "ok": lo que responde /api/productos con la cabecera bien puesta
ANTES = "__ANTES__"  # "si": al cargar la página, antes de entrar, pide /api/yo (que da 401)
POST401 = "__POST401__"  # "si": después de entrar, un POST a /api/cambiar da 401
PAGINA = """<html><body>
<div id="entrada"><input id="u" type="text"><input id="p" type="password"><button onclick="entrar()">Entrar</button></div>
<div id="app" style="display:none"><nav><a href="#" onclick="return false">Productos</a></nav><ul id="lista"></ul></div>
<script>
const CABECERA = __CAB__;
async function entrar() {
  const r = await fetch('/api/login', {method: 'POST', headers: {'Content-Type': 'application/json'},
                                       body: JSON.stringify({u: u.value, p: p.value})});
  if (!r.ok) { if ('__ANTES__' === 'si') fetch('/api/yo'); return; }  // comprueba la sesión tras fallar
  entrada.style.display = 'none'; app.style.display = 'block';
  if ('__POST401__' === 'si') fetch('/api/cambiar', {method: 'POST', headers: CABECERA});
  const l = await fetch('/api/productos', {headers: CABECERA});
  if (l.ok) lista.innerHTML = (await l.json()).map(x => '<li>' + x + '</li>').join('');
}
if ('__ANTES__' === 'si') fetch('/api/yo');
</script></body></html>"""


class Web(BaseHTTPRequestHandler):
    def responder(self, estado, cuerpo="{}", tipo="application/json"):
        self.send_response(estado)
        self.send_header("Content-Type", tipo)
        self.end_headers()
        self.wfile.write(cuerpo.encode())

    def do_GET(self):
        if self.path == "/":
            cab = '{"Authorization": "Bearer tok"}' if CABECERA == "con" else "{}"
            return self.responder(200, PAGINA.replace("__CAB__", cab), "text/html; charset=utf-8")
        if self.path == "/api/productos":
            if self.headers.get("Authorization") != "Bearer tok":
                return self.responder(401, '{"error": "No autenticado"}')
            if CIERRA != "ok":
                return self.responder(int(CIERRA), '{"error": "no"}')
            return self.responder(200, json.dumps(["tornillos"]))
        if self.path == "/api/yo":
            return self.responder(401, '{"error": "No autenticado"}')
        self.responder(404, "{}")

    def do_POST(self):
        largo = int(self.headers.get("Content-Length") or 0)
        datos = json.loads(self.rfile.read(largo) or "{}")
        if self.path == "/api/login":
            return self.responder(200 if datos.get("p") == "pan123" else 401)
        self.responder(401)


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 5000), Web).serve_forever()
'''

WEB_ENTRAR = r'''from http.server import BaseHTTPRequestHandler, HTTPServer

PAGINA = """<html><head><style>.pantalla{display:none}</style></head><body>__CUERPO__</body></html>"""


class Web(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.end_headers()
        self.wfile.write(PAGINA.encode())


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 5000), Web).serve_forever()
'''

FORM = '<input id="u" type="text"><input id="p" type="password"><button>Entrar</button>'


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
class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_sinaut_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)

    def navegador(self, fuente: str, accesos=(("admin", "pan123"),), **cambios: str) -> dict:
        for k, v in cambios.items():
            fuente = fuente.replace(f"__{k}__", v)
        (self.dir / "app.py").write_text(fuente, encoding="utf-8")
        r = recorrer(self.dir / "app.py", sys.executable, list(accesos), abrir_navegador=canal_navegador)
        self.assertIsNotNone(r["navegador"], "el navegador no se usó")
        return r["navegador"]


class SinAutorizacion(Base):
    def api(self, cabecera: str = "sin", cierra: str = "ok", antes: str = "no", post401: str = "no", **kw) -> list:
        return self.navegador(WEB_API, CABECERA=cabecera, CIERRA=cierra, ANTES=antes, POST401=post401,
                              **kw)["sin_autorizacion"]

    def test_la_pantalla_que_manda_su_fetch_sin_credencial(self):
        [s] = self.api(cabecera="sin")
        self.assertEqual((s["estado"], s["ruta"], s["usuario"]), (401, "/api/productos", "admin"))

    def test_con_la_credencial_puesta_no_hay_nada(self):
        self.assertEqual(self.api(cabecera="con"), [])

    def test_el_401_de_antes_de_entrar_no_cuenta(self):
        self.assertEqual(self.api(cabecera="con", antes="si"), [])

    def test_un_403_al_administrador_cuenta(self):
        [s] = self.api(cabecera="con", cierra="403")
        self.assertEqual((s["estado"], s["ruta"]), (403, "/api/productos"))

    def test_un_403_a_un_vendedor_es_normal(self):
        lista = self.navegador(WEB_API, accesos=[("vendedor1", "pan123")], CABECERA="con", CIERRA="403", ANTES="no",
                               POST401="no")["sin_autorizacion"]
        self.assertEqual(lista, [])

    def test_un_401_en_un_post_no_cuenta(self):
        self.assertEqual(self.api(cabecera="con", post401="si"), [])

    def test_un_intento_que_no_vale_no_cuenta(self):
        # La contraseña buena es la segunda: lo que respondió la API al primer intento no se anota.
        lista = self.navegador(WEB_API, accesos=[("admin", "mala"), ("admin", "pan123")], CABECERA="con", CIERRA="ok",
                               ANTES="si", POST401="no")["sin_autorizacion"]
        self.assertEqual(lista, [])


class LoginInvisible(Base):
    def entrar(self, cuerpo: str) -> list | None:
        return self.navegador(WEB_ENTRAR, CUERPO=cuerpo)["login_invisible"]

    def test_el_formulario_dentro_de_un_bloque_con_display_none(self):
        oculto = self.entrar(f'<div class="pantalla activa"><form>{FORM}</form></div>')
        self.assertEqual((oculto[0], oculto[3]), ("div", "display:none"))
        self.assertIn("pantalla", oculto[2])

    def test_un_boton_visible_que_abre_el_formulario_no_es_pantalla_en_blanco(self):
        abre = "<button onclick=\"document.getElementById('f').style.display='block'\">Acceder</button>"
        self.assertIsNone(self.entrar(f'{abre}<div id="f" class="pantalla">{FORM}</div>'))

    def test_un_formulario_a_la_vista_no_es_pantalla_en_blanco(self):
        self.assertIsNone(self.entrar(FORM))

    def test_con_texto_a_la_vista_no_es_pantalla_en_blanco(self):
        self.assertIsNone(self.entrar(f'<h1>Mi tienda</h1><p>Cargando la aplicación, un momento</p><div class="pantalla">{FORM}</div>'))


if __name__ == "__main__":
    unittest.main()
