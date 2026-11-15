"""verificar vende desde la pantalla de «nueva venta» con un navegador oculto (m4141: dos entregas con una pantalla que no vendía y el
servidor sin fallar). Fallos: el desplegable de productos queda vacío porque el script falla, la venta no se guarda («agrega al menos un
producto» con un producto elegido) y el 500 al guardar. Y los silencios: la venta que se guarda, la pantalla de carrito, las librerías
que no cargan y los mensajes que ya estaban escritos antes de guardar."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import verificacion
from agente.recorrido_web import recorrer
from agente.verificacion import canal_navegador

MENU = '<nav><a href="/productos">Productos</a> <a href="/ventas">Ventas</a> <a href="/ventas/nueva">Nueva venta</a></nav>'
CLIENTE = '<select name="cliente_id"><option value="1">Ana</option></select>'
PRODUCTOS_OPC = '<option value="">Selecciona un producto</option><option value="5">Tornillo</option>'

WEB = r'''import traceback
import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer

MODO = "__MODO__"
PRECIO = "__PRECIO__"   # "si" (la lista de productos enseña su precio) o "no"
VENTAS = []
MENU = '__MENU__'
CLIENTE = '__CLIENTE__'
OPC = '__OPC__'

PAGINAS = {
    "ok": '<h1>Nueva venta</h1><form method="post" action="/ventas/nueva">' + CLIENTE + '<select name="producto_id">' + OPC + '</select>'
          '<input type="number" name="cantidad" value="1" min="1"><button type="submit">Guardar venta</button></form>',
    # lo que arma JavaScript con una variable que nadie define (m4141_neg4)
    "js_roto": '<h1>Nueva venta</h1><form method="post" action="/ventas/nueva">' + CLIENTE + '<select name="producto_0_id" class="prod-select"></select>'
               '<input type="number" name="producto_0_cantidad" value="1"><button type="submit">Finalizar Venta</button></form>'
               '<script>productosData.forEach(function(p){});</script>',
    # una librería de un CDN que no cargó: no es culpa de la pantalla
    "libreria": '<h1>Nueva venta</h1><form method="post" action="/ventas/nueva">' + CLIENTE + '<select name="producto_0_id" class="prod-select"></select>'
                '<button type="submit">Finalizar Venta</button></form><script>$(function(){});</script>',
    # el campo con las líneas fuera del <form> (m4141_neg2): un formulario solo envía sus propios campos
    "fuera": '<h1>Nueva venta</h1><form id="f" method="post" action="/ventas/nueva">' + CLIENTE + '<div id="filas"></div>'
             '<button type="button" onclick="agregar()">+ Agregar producto</button><button type="submit">Guardar venta</button></form>'
             '<input type="hidden" name="items" id="items"><script>function agregar(){var d=document.createElement("div");'
             'd.innerHTML=\'<select class="item-producto">' + OPC + '</select><input type="number" class="item-cant" value="1">\';'
             'document.getElementById("filas").appendChild(d);}</script>',
    # elegir, añadir al carrito y finalizar: lo elegido viaja en un campo oculto que rellena el botón de añadir
    "carrito": '<h1>Nueva venta</h1><select id="p" name="producto">' + OPC + '</select><input type="number" id="c" name="cantidad" value="1">'
               '<button type="button" onclick="meter()">Agregar al carrito</button><ul id="lista"></ul>'
               '<form method="post" action="/ventas/nueva"><input type="hidden" name="items" id="items"><button type="submit">Finalizar venta</button></form>'
               '<script>function meter(){var p=document.getElementById("p").value;if(!p){return;}'
               'document.getElementById("items").value=p;document.getElementById("lista").innerHTML="<li>"+p+"</li>";}</script>',
    # «Agrega al menos un producto» escrito en la propia pantalla antes de guardar: no es la respuesta del servidor
    "texto_fijo": '<h1>Nueva venta</h1><p>Agrega al menos un producto para continuar.</p><form method="post" action="/ventas/nueva">' + CLIENTE
                  + '<select name="producto_id">' + OPC + '</select><button type="submit">Guardar venta</button></form>',
}


class Web(BaseHTTPRequestHandler):
    def responder(self, estado, cuerpo, cabeceras=()):
        self.send_response(estado)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        for k, v in cabeceras:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(cuerpo.encode())

    def do_GET(self):
        if self.path == "/":
            return self.responder(200, "<h1>Inicio</h1>" + MENU)
        if self.path == "/productos":
            return self.responder(200, "<h1>Productos</h1>" + MENU + ("<p>Tornillo, precio 21.31</p>" if PRECIO == "si" else "<p>No hay productos.</p>"))
        if self.path == "/ventas":
            return self.responder(200, "<h1>Ventas</h1>" + MENU + f"<p>{len(VENTAS)} ventas</p>")
        if self.path == "/ventas/nueva":
            return self.responder(200, MENU + PAGINAS[MODO.split("+")[0]])
        self.responder(404, "no")

    def do_POST(self):
        datos = urllib.parse.parse_qs(self.rfile.read(int(self.headers["Content-Length"])).decode())
        producto = (datos.get("producto_id") or datos.get("items") or datos.get("producto_0_id") or [""])[0]
        if not producto:
            return self.responder(200, MENU + "<p>Agrega al menos un producto.</p>" + PAGINAS[MODO.split("+")[0]])
        if MODO.endswith("+500"):
            try:
                {}["total"]
            except Exception:
                traceback.print_exc()
                return self.responder(500, "Internal Server Error")
        VENTAS.append(datos)
        self.responder(302, "", [("Location", "/ventas")])


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
class VentaEnPantalla(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_venta_pantalla_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)

    def recorrer(self, modo: str, precio: str = "si") -> dict:
        fuente = (WEB.replace("__MODO__", modo).replace("__PRECIO__", precio).replace("__MENU__", MENU)
                  .replace("__CLIENTE__", CLIENTE).replace("__OPC__", PRODUCTOS_OPC))
        (self.dir / "app.py").write_text(fuente, encoding="utf-8")
        return recorrer(self.dir / "app.py", sys.executable, [], abrir_navegador=canal_navegador)

    def test_la_venta_que_se_guarda_no_dice_nada(self):
        r = self.recorrer("ok")
        self.assertEqual(r["errores"], [])
        self.assertEqual((r["venta"]["errores"], r["venta"]["guardadas"]), ([], 1))

    def test_el_desplegable_vacio_por_un_error_de_javascript(self):
        [e] = self.recorrer("js_roto")["venta"]["errores"]
        self.assertEqual((e["pagina"], e["tipo"]), ("/ventas/nueva", "vacio"))
        self.assertIn("productosData is not defined", e["js"])

    def test_sin_productos_creados_no_se_dice_nada(self):
        self.assertEqual(self.recorrer("js_roto", precio="no")["venta"]["errores"], [])

    def test_una_libreria_que_no_cargo_no_es_el_fallo(self):
        self.assertEqual(self.recorrer("libreria")["venta"]["errores"], [])

    def test_las_lineas_fuera_del_formulario_no_llegan(self):
        [e] = self.recorrer("fuera")["venta"]["errores"]
        self.assertEqual((e["pagina"], e["tipo"]), ("/ventas/nueva", "mensaje"))
        self.assertIn("al menos un producto", e["mensaje"].lower())

    def test_la_pantalla_de_carrito_que_funciona_no_dice_nada(self):
        v = self.recorrer("carrito")["venta"]
        self.assertEqual((v["errores"], v["guardadas"]), ([], 1))  # la primera vez sin añadir falla; elegir y añadir, no

    def test_el_mensaje_que_ya_estaba_antes_de_guardar_no_cuenta(self):
        v = self.recorrer("texto_fijo")["venta"]
        self.assertEqual((v["errores"], v["guardadas"]), ([], 1))

    def test_el_500_al_guardar_se_ve_con_su_traza(self):
        r = self.recorrer("carrito+500")  # con «ok» el recorrido sin JavaScript ya da el mismo 500 y no se cuenta dos veces
        [e] = [x for x in r["errores"] if "pantalla" in x["que"]]
        self.assertEqual(e["estado"], 500)
        self.assertIn("guardar la venta desde la pantalla /ventas/nueva", e["que"])
        self.assertIn("KeyError", e["traza"])


class MensajesDeVerificar(unittest.TestCase):
    def errores(self, venta: dict) -> list[str]:
        raiz = Path(tempfile.mkdtemp(prefix="test_venta_msg_"))
        self.addCleanup(shutil.rmtree, raiz, ignore_errors=True)
        (raiz / "app.py").write_text("print('x')\n", encoding="utf-8")
        falso = {"errores": [], "plantillas": [], "rotos": [], "raros": [], "enviados": [], "paginas": 3, "formularios": 2,
                 "usuario": "admin", "sin_entrar": False, "entrar_js": False, "navegador": None, "lineas": None,
                 "sin_formulario": [], "registro": False, "entrar_get": False, "sin_permiso": [], "login_vacio": None,
                 "empleado": None, "negocio": {}, "venta": venta}
        inf = verificacion.Informe()
        with mock.patch.object(verificacion, "recorrer", return_value=falso):
            verificacion.probar_web(raiz, raiz / "app.py", sys.executable, inf)
        self.assertEqual(inf.avisos, [])
        return inf.errores

    def test_desplegable_vacio(self):
        [m] = self.errores({"errores": [{"pagina": "/ventas/nueva", "tipo": "vacio", "js": "productosData is not defined", "mensaje": ""}]})
        self.assertIn("en la pantalla de venta /ventas/nueva, el desplegable de productos queda vacío", m)
        self.assertIn("«productosData is not defined»", m)
        self.assertIn("nadie puede vender desde esta pantalla", m)

    def test_la_venta_no_se_guarda(self):
        [m] = self.errores({"errores": [{"pagina": "/ventas/nueva", "tipo": "mensaje", "js": "", "mensaje": "Agrega al menos un producto"}]})
        self.assertIn("tras elegir un producto y una cantidad y pulsar guardar, la web responde «Agrega al menos un producto»", m)
        self.assertIn("va fuera del <form>", m)

    def test_nada_si_no_hay_nada_que_decir(self):
        self.assertEqual(self.errores({"errores": [], "probadas": 1, "guardadas": 1}), [])
        self.assertEqual(self.errores(None), [])


if __name__ == "__main__":
    unittest.main()
