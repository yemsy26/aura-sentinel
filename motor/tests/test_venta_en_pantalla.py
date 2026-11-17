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
MENU_PANEL = '<nav><a href="/productos">Productos</a> <a href="/ventas">Ventas</a></nav>'  # sin enlace a la ruta de alta
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
    # elegir y añadir al carrito, y guardar con el carrito vacío no envía nada (el navegador no deja pasar el formulario): sin mensaje ni petición
    "carrito_mudo": '<h1>Nueva venta</h1><select id="p" name="producto">' + OPC + '</select><input type="number" id="c" name="cantidad" value="1">'
                    '<button type="button" onclick="meter()">Agregar al carrito</button><ul id="lista"></ul>'
                    '<form method="post" action="/ventas/nueva" onsubmit="return document.getElementById(&#39;items&#39;).value !== &#39;&#39;;">'
                    '<input type="hidden" name="items" id="items"><button type="submit">Finalizar venta</button></form>'
                    '<script>function meter(){var p=document.getElementById("p").value;if(!p){return;}'
                    'document.getElementById("items").value=p;document.getElementById("lista").innerHTML="<li>"+p+"</li>";}</script>',
    # la venta se hace en la propia lista de ventas: un botón «+ Nueva venta» (sin type, fuera de todo form) abre el panel; la ruta de alta solo acepta POST (m4148_neg4)
    "panel": '<div id="p" style="display:none"><form method="post" action="/ventas/nueva">' + CLIENTE + '<select name="producto_id">' + OPC + '</select>'
             '<input type="number" name="cantidad" value="1" min="1"><button type="submit">Registrar venta</button></form></div>'
             '<button onclick="document.getElementById(&#39;p&#39;).style.display=&#39;block&#39;">+ Nueva venta</button>',
    # «Agrega al menos un producto» escrito en la propia pantalla antes de guardar: no es la respuesta del servidor
    "texto_fijo": '<h1>Nueva venta</h1><p>Agrega al menos un producto para continuar.</p><form method="post" action="/ventas/nueva">' + CLIENTE
                  + '<select name="producto_id">' + OPC + '</select><button type="submit">Guardar venta</button></form>',
    # el desplegable queda vacío y el script lo cuenta con console.error, sin lanzar nada (m4150_neg1: JSON roto en un atributo)
    "consola_rota": '<h1>Nueva venta</h1><form method="post" action="/ventas/nueva">' + CLIENTE + '<select name="producto_0_id" class="prod-select"></select>'
                    '<button type="submit">Finalizar Venta</button></form><script>console.error("Error parsing productos: SyntaxError");</script>',
    # igual de vacío pero sin decir nada (puede cargarse por fetch o ser legítimo): sin error de JavaScript no se acusa
    "consola_muda": '<h1>Nueva venta</h1><form method="post" action="/ventas/nueva">' + CLIENTE + '<select name="producto_0_id" class="prod-select"></select>'
                    '<button type="submit">Finalizar Venta</button></form>',
    # <form> sin method que un script intercepta, rellena el campo de las líneas y envía con this.submit(): el navegador lo manda por GET (m4161_neg1)
    "por_get": '<h1>Nueva venta</h1><form id="f">' + CLIENTE + '<select name="producto_id">' + OPC + '</select><input type="number" name="cantidad" value="1" min="1">'
               '<input type="hidden" name="items" id="items"><button type="submit">Guardar venta</button></form>'
               '<script>document.getElementById("f").addEventListener("submit",function(e){e.preventDefault();'
               'document.getElementById("items").value="5";this.submit();});</script>',
    # lo mismo, pero el script pone method=POST antes de enviar: la venta llega bien (m4141_neg2)
    "por_get_post_js": '<h1>Nueva venta</h1><form id="f" action="/ventas/nueva">' + CLIENTE + '<select name="producto_id">' + OPC + '</select>'
                       '<input type="number" name="cantidad" value="1" min="1"><input type="hidden" name="items" id="items"><button type="submit">Guardar venta</button></form>'
                       '<script>document.getElementById("f").addEventListener("submit",function(e){e.preventDefault();'
                       'document.getElementById("items").value="5";this.method="POST";this.submit();});</script>',
    # no se crea ningún desplegable porque el script falla al cargar con una variable que nadie define (m4161_neg2)
    "sin_desplegable_js": '<h1>Nueva venta</h1><form method="post" action="/ventas/nueva">' + CLIENTE + '<div id="filas"></div>'
                          '<button type="submit">Guardar venta</button></form><script>productosData.forEach(function(p){});</script>',
    # sin desplegable y sin ningún error: puede ser otra forma de vender, no se acusa
    "sin_desplegable_mudo": '<h1>Nueva venta</h1><form method="post" action="/ventas/nueva">' + CLIENTE + '<div id="filas"></div>'
                            '<button type="submit">Guardar venta</button></form>',
    # una tabla con una cantidad por producto; al cambiar, el script copia la línea a un campo oculto dentro del form
    "tabla_ok": '<h1>Nueva venta</h1><form method="post" action="/ventas/nueva">' + CLIENTE + '<table><tr><td>Tornillo</td><td>'
                '<input type="number" id="cant_0" value="0" min="0" max="10" onchange="actualizar()"></td></tr></table><div id="ocultos"></div>'
                '<button type="submit">Registrar venta</button></form><script>function actualizar(){var c=document.getElementById("cant_0").value;'
                'document.getElementById("ocultos").innerHTML=c>0?\'<input type="hidden" name="items" value="5|\'+c+\'">\':"";}</script>',
    # la misma tabla con el contenedor de los campos ocultos FUERA del <form>: nunca llegan (m4161_neg4)
    "tabla_fuera": '<h1>Nueva venta</h1><form method="post" action="/ventas/nueva">' + CLIENTE + '<table><tr><td>Tornillo</td><td>'
                   '<input type="number" id="cant_0" value="0" min="0" max="10" onchange="actualizar()"></td></tr></table>'
                   '<button type="submit">Registrar venta</button></form><div id="ocultos"></div><script>function actualizar(){'
                   'var c=document.getElementById("cant_0").value;document.getElementById("ocultos").innerHTML=c>0?\'<input type="hidden" name="items" value="5|\'+c+\'">\':"";}</script>',
    # una tabla donde se marca el producto con una casilla: la prueba no sabe operarla y no acusa
    "tabla_casillas": '<h1>Nueva venta</h1><form method="post" action="/ventas/nueva">' + CLIENTE + '<table><tr><td><input type="checkbox" name="marcado" value="5"></td>'
                      '<td>Tornillo</td><td><input type="number" id="cant_0" value="0" min="0" max="10"></td></tr></table>'
                      '<button type="submit">Registrar venta</button></form>',
    # un script que no existe (404): la consola dice «Failed to load resource», que es de red y no del script de la pantalla
    "red_404": '<h1>Nueva venta</h1><form method="post" action="/ventas/nueva">' + CLIENTE + '<select name="producto_0_id" class="prod-select"></select>'
               '<button type="submit">Finalizar Venta</button></form><script src="/falta.js"></script>',
}


# el panel que envía con fetch y no mira la respuesta: la pantalla no cambia, pase lo que pase
PANEL_FETCH = PAGINAS["panel"].replace('<form method="post" action="/ventas/nueva">', '<form onsubmit="fetch(&#39;/ventas/nueva&#39;,{method:&#39;POST&#39;,'
                                       'body:new URLSearchParams(new FormData(this))});return false;">')


class Web(BaseHTTPRequestHandler):
    def responder(self, estado, cuerpo, cabeceras=()):
        self.send_response(estado)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        for k, v in cabeceras:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(cuerpo.encode())

    def do_GET(self):
        self.path = self.path.split("?")[0]  # un formulario que envía por GET llega con la consulta: la ruta es la misma
        if self.path == "/":
            return self.responder(200, "<h1>Inicio</h1>" + MENU)
        if self.path == "/productos":
            return self.responder(200, "<h1>Productos</h1>" + MENU + ("<p>Tornillo, precio 21.31</p>" if PRECIO == "si" else "<p>No hay productos.</p>"))
        if self.path == "/ventas":
            return self.responder(200, "<h1>Ventas</h1>" + MENU + f"<p>{len(VENTAS)} ventas</p>" + ((PANEL_FETCH if MODO.endswith("+silencio") else PAGINAS["panel"]) if MODO.startswith("panel") else ""))
        if self.path == "/ventas/nueva":
            if MODO.startswith("panel"):
                return self.responder(405, "Method Not Allowed")
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
        if MODO.endswith("+error"):  # el servidor captura la excepción, la enseña y no guarda
            return self.responder(200, MENU + "<p>Error al registrar venta: IntegrityError NOT NULL constraint failed</p>" + PAGINAS[MODO.split("+")[0]])
        if MODO.endswith("+sinstock"):  # un rechazo con razón: la prueba no sabe cuánto hay
            return self.responder(200, MENU + "<p>Error: stock insuficiente</p>" + PAGINAS[MODO.split("+")[0]])
        if MODO.endswith("+rechazo"):  # dice algo, pero no guarda: ¿es un rechazo con razón? No se sabe
            return self.responder(200, MENU + "<p>Hubo un problema con la venta.</p>" + PAGINAS[MODO.split("+")[0]])
        if MODO.endswith("+silencio"):  # lo que hizo m4148_neg3: 200 con la misma pantalla, sin guardar y sin decir nada
            return self.responder(200, MENU + PAGINAS[MODO.split("+")[0]])
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

    def recorrer(self, modo: str, precio: str = "si", menu: str = MENU) -> dict:
        fuente = (WEB.replace("__MODO__", modo).replace("__PRECIO__", precio).replace("__MENU__", menu)
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

    def test_el_desplegable_vacio_que_el_script_cuenta_con_console_error(self):
        [e] = self.recorrer("consola_rota")["venta"]["errores"]
        self.assertEqual((e["pagina"], e["tipo"]), ("/ventas/nueva", "vacio"))
        self.assertIn("Error parsing productos", e["js"])

    def test_el_desplegable_vacio_sin_ningun_error_no_se_acusa(self):
        self.assertEqual(self.recorrer("consola_muda")["venta"]["errores"], [])

    def test_un_404_de_un_script_no_es_el_fallo_de_la_pantalla(self):
        self.assertEqual(self.recorrer("red_404")["venta"]["errores"], [])

    def test_sin_productos_creados_no_se_dice_nada(self):
        self.assertEqual(self.recorrer("js_roto", precio="no")["venta"]["errores"], [])

    def test_una_libreria_que_no_cargo_no_es_el_fallo(self):
        self.assertEqual(self.recorrer("libreria")["venta"]["errores"], [])

    def test_las_lineas_fuera_del_formulario_no_llegan(self):
        [e] = self.recorrer("fuera")["venta"]["errores"]
        self.assertEqual((e["pagina"], e["tipo"]), ("/ventas/nueva", "mensaje"))
        self.assertIn("al menos un producto", e["mensaje"].lower())

    def test_el_formulario_que_el_script_envia_por_get_se_ve(self):
        v = self.recorrer("por_get")["venta"]
        [e] = v["errores"]
        self.assertEqual((e["pagina"], e["tipo"]), ("/ventas/nueva", "por_get"))
        self.assertIn("/ventas/nueva?", e["mensaje"])
        self.assertIn("items=5", e["mensaje"])
        self.assertEqual(v["guardadas"], 0)

    def test_el_formulario_cuyo_script_lo_pasa_a_post_no_dice_nada(self):
        v = self.recorrer("por_get_post_js")["venta"]
        self.assertEqual((v["errores"], v["guardadas"]), ([], 1))

    def test_sin_desplegable_por_un_error_de_javascript_al_cargar(self):
        v = self.recorrer("sin_desplegable_js")["venta"]
        [e] = v["errores"]
        self.assertEqual((e["pagina"], e["tipo"]), ("/ventas/nueva", "sin_desplegable"))
        self.assertIn("productosData", e["js"])

    def test_sin_desplegable_ni_error_ni_productos_no_se_acusa(self):
        self.assertEqual(self.recorrer("sin_desplegable_mudo")["venta"]["errores"], [])
        self.assertEqual(self.recorrer("sin_desplegable_js", precio="no")["venta"]["errores"], [])  # sin productos creados no hay de qué

    def test_la_tabla_de_cantidades_que_funciona_se_vende(self):
        v = self.recorrer("tabla_ok")["venta"]
        self.assertEqual((v["errores"], v["guardadas"]), ([], 1))

    def test_la_tabla_de_cantidades_con_los_campos_ocultos_fuera_del_form(self):
        [e] = self.recorrer("tabla_fuera")["venta"]["errores"]
        self.assertEqual((e["pagina"], e["tipo"]), ("/ventas/nueva", "mensaje"))
        self.assertIn("al menos un producto", e["mensaje"].lower())

    def test_la_tabla_sin_productos_creados_o_con_casillas_no_se_toca(self):
        self.assertEqual(self.recorrer("tabla_fuera", precio="no")["venta"]["errores"], [])
        self.assertEqual(self.recorrer("tabla_casillas")["venta"]["errores"], [])

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

    def test_la_venta_que_no_se_registra_y_no_dice_nada(self):
        [e] = self.recorrer("ok+silencio")["venta"]["errores"]
        self.assertEqual((e["pagina"], e["tipo"], e["mensaje"]), ("/ventas/nueva", "no_registrada", "POST /ventas/nueva -> 200"))

    def test_guardar_sin_que_salga_nada_se_intenta_con_el_carrito(self):
        v = self.recorrer("carrito_mudo")["venta"]
        self.assertEqual((v["errores"], v["guardadas"], v["estados"]), ([], 1, [("/ventas/nueva", "guardada")]))

    def test_si_la_web_dice_algo_pero_no_guarda_no_se_acusa(self):
        v = self.recorrer("ok+rechazo")["venta"]
        self.assertEqual((v["errores"], v["guardadas"], v["estados"]), ([], 0, [("/ventas/nueva", "sin_confirmar")]))

    def test_el_error_que_la_web_enseña_y_no_guarda_se_dice(self):
        [e] = self.recorrer("ok+error")["venta"]["errores"]
        self.assertEqual((e["pagina"], e["tipo"]), ("/ventas/nueva", "rechazada"))
        self.assertIn("Error al registrar venta", e["mensaje"])

    def test_un_rechazo_con_razon_no_se_acusa(self):
        v = self.recorrer("ok+sinstock")["venta"]
        self.assertEqual((v["errores"], v["estados"]), ([], [("/ventas/nueva", "sin_confirmar")]))

    def test_sin_lista_que_comparar_no_se_dice_nada(self):
        menu = '<nav><a href="/ventas/nueva">Nueva venta</a></nav>'  # sin lista de ventas ni de productos: no hay con qué saber si se guardó
        v = self.recorrer("ok+silencio", menu=menu)["venta"]
        self.assertEqual((v["errores"], v["guardadas"]), ([], 1))

    def test_el_panel_de_nueva_venta_en_la_lista_se_abre_y_vende(self):
        v = self.recorrer("panel", menu=MENU_PANEL)["venta"]
        self.assertEqual((v["errores"], v["guardadas"], v["estados"]), ([], 1, [("/ventas", "guardada")]))

    def test_el_panel_que_no_registra_la_venta_tambien_se_ve(self):
        [e] = self.recorrer("panel+silencio", menu=MENU_PANEL)["venta"]["errores"]
        self.assertEqual((e["pagina"], e["tipo"]), ("/ventas", "no_registrada"))

    def test_una_lista_de_ventas_sin_boton_de_nueva_venta_no_lanza_nada(self):
        self.assertIsNone(self.recorrer("js_roto", menu=MENU_PANEL)["venta"])  # sin ruta de alta ni botón en /ventas


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

    def test_la_venta_se_envia_por_get(self):
        [m] = self.errores({"errores": [{"pagina": "/ventas/nueva", "tipo": "por_get", "js": "",
                                         "mensaje": "/ventas/nueva?cliente_id=&items=%5B1%5D"}]})
        self.assertIn("al pulsar guardar el navegador envía los datos por GET (la dirección acaba en «/ventas/nueva?cliente_id=&items=%5B1%5D»)", m)
        self.assertIn('method="post"', m)
        self.assertIn("la venta no se guarda nunca", m)

    def test_sin_desplegable_por_un_error_de_javascript(self):
        [m] = self.errores({"errores": [{"pagina": "/ventas/nueva", "tipo": "sin_desplegable",
                                         "js": "Cannot read properties of undefined (reading 'toFixed')", "mensaje": ""}]})
        self.assertIn("en la pantalla de venta /ventas/nueva no aparece ningún desplegable de productos aunque hay productos creados", m)
        self.assertIn("«Cannot read properties of undefined (reading 'toFixed')»", m)
        self.assertIn("nadie puede vender desde esta pantalla", m)

    def test_la_venta_no_se_guarda(self):
        [m] = self.errores({"errores": [{"pagina": "/ventas/nueva", "tipo": "mensaje", "js": "", "mensaje": "Agrega al menos un producto"}]})
        self.assertIn("tras elegir un producto y una cantidad y pulsar guardar, la web responde «Agrega al menos un producto»", m)
        self.assertIn("va fuera del <form>", m)

    def test_la_venta_no_se_registra(self):
        [m] = self.errores({"errores": [{"pagina": "/ventas/nueva", "tipo": "no_registrada", "js": "updateTotal is not defined",
                                         "mensaje": "POST /ventas/nueva -> 200"}]})
        self.assertIn("no se registra la venta (POST /ventas/nueva -> 200)", m)
        self.assertIn("ni la lista de ventas ni las existencias cambian (error de JavaScript: «updateTotal is not defined»)", m)
        self.assertIn("campo obligatorio que la plantilla no pinta", m)

    def test_la_venta_rechazada_con_un_error(self):
        [m] = self.errores({"errores": [{"pagina": "/ventas", "tipo": "rechazada", "js": "", "mensaje": "Error de conexión"}]})
        self.assertIn("la web responde «Error de conexión» y la venta no se registra", m)
        self.assertIn("ni la lista de ventas ni las existencias cambian", m)

    def test_nada_si_no_hay_nada_que_decir(self):
        self.assertEqual(self.errores({"errores": [], "probadas": 1, "guardadas": 1}), [])
        self.assertEqual(self.errores(None), [])


if __name__ == "__main__":
    unittest.main()
