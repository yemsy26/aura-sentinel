"""El recorrido crea un empleado desde la pantalla de usuarios y mira qué ve (m4139 y m4140): /reportes con las ganancias abierto al
empleado, el precio de compra en la lista de productos o en la API. Solo habla si puede completar todos los pasos."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import verificacion
from agente.recorrido_web import _vendedores_ajenos, _ventas_en_tabla, recorrer

WEB = r'''import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer

COSTO = "__COSTO__"        # "muestra" u "oculta" el precio de compra al empleado
ABRE = "__ABRE__"          # reportes y usuarios para el empleado: "si", "no" (redirige), "403" o "propio" (otra página)
USUARIOS = "__USUARIOS__"  # "con" o "sin" formulario de alta de usuarios
EMPLEADO_ENTRA = "__ENTRA__"
PANEL = "__PANEL__"        # "no", "ajeno" (el panel de inicio lista las ventas de todos con su vendedor), "filtrado" (solo las del que mira),
                           # "sin_col" (la tabla de ventas de todos, sin columna de vendedor, en «/» y en /ventas) o "sin_col_filtrado" (la misma, solo las suyas)
VENTAS = "__VENTAS__"      # "no" (sin ventas), "ajenas" (el empleado ve todas), "filtra" (solo las suyas) o "500" (el detalle falla al empleado)
USERS = {"admin": ("pan123", "admin")}
PRODUCTOS = []
VENTAS_HECHAS = []


class Web(BaseHTTPRequestHandler):
    def responder(self, estado, cuerpo, cabeceras=()):
        self.send_response(estado)
        for k, v in cabeceras:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(cuerpo.encode())

    def quien(self):
        for trozo in (self.headers.get("Cookie") or "").split(";"):
            k, _, v = trozo.strip().partition("=")
            if k == "sesion" and v in USERS:
                return v
        return None

    def do_GET(self):
        u = self.quien()
        if self.path == "/entrar":
            return self.responder(200, '<form method="post" action="/entrar"><input name="usuario">'
                                       '<input type="password" name="clave"><button>Entrar</button></form>')
        if u is None:
            return self.responder(302, "", [("Location", "/entrar")])
        admin = USERS[u][1] == "admin"
        menu = ('<nav><a href="/productos">Productos</a> <a href="/reportes">Ganancias y reportes</a> <a href="/usuarios">Usuarios</a> '
                '<a href="/informes">Informes</a>' + ('<a href="/ventas">Ventas</a>' if VENTAS != "no" else "") + '</nav>')
        if VENTAS != "no" and self.path.startswith("/ventas"):
            if not admin and VENTAS == "filtra" and self.path != "/ventas":
                return self.responder(302, "", [("Location", "/")])  # el detalle de una venta que no es suya: cerrado
            if not admin and VENTAS == "500" and self.path != "/ventas":
                return self.responder(500, "Internal Server Error")  # venta['vendedor_id'] sin esa columna para el empleado
            if self.path == "/ventas":
                propias = [] if (not admin and VENTAS == "filtra") else VENTAS_HECHAS
                filas = "".join(f'<li><a href="/ventas/{i}">Venta {i}</a></li>' for i, _ in enumerate(propias, 1))
                alta = ('<form method="post" action="/ventas/nueva"><input name="producto"><input type="number" name="cantidad">'
                        '<button>Vender</button></form>')
                extra = ""
                if PANEL == "sin_col" and propias:
                    extra = ("<table><tr><th>Hora</th><th>Total</th><th>Cliente</th></tr>"
                             + "".join("<tr><td>10:00</td><td>$450.00</td><td>Pedro</td></tr>" for _ in propias) + "</table>")
                return self.responder(200, "<h1>Ventas</h1>" + menu + (f"<ul>{filas}</ul>" if filas else "<p>No hay ventas.</p>") + extra + alta)
            n = self.path.rsplit("/", 1)[-1]
            return self.responder(200, f"<h1>Venta {n}</h1>" + menu + f"<p>Venta {n} de la tienda: dos tornillos, vendedor administrador, "
                                                                f"total 40.00, cliente de mostrador, factura F-{n} emitida y pagada.</p>")
        if self.path == "/informes":  # una sección de gestión sin la palabra ganancia: solo se nota que se ve igual
            if not admin and ABRE == "403":
                return self.responder(403, "Forbidden")
            if not admin and ABRE in ("no", "propio", "parcial", "sin_aviso"):
                return self.responder(302, "", [("Location", "/")])
            return self.responder(200, "<h1>Informes</h1>" + menu + "<p>Inventario valorado por categoria, rotacion de productos, "
                                                                   "vendedores con mas movimiento y dias de mas actividad del mes.</p>")
        if self.path == "/":
            tabla = ""
            if PANEL != "no" and VENTAS_HECHAS:
                propias = VENTAS_HECHAS if (admin or PANEL in ("ajeno", "sin_col")) else []
                if PANEL.startswith("sin_col"):  # como la pantalla de venta de la ferretería (22): ventas recientes de todos, sin vendedor
                    filas = "".join(f"<tr><td>{i}</td><td>10:00</td><td>Martillo</td><td>3</td><td>$450.00</td><td>Pedro</td></tr>"
                                    for i, _ in enumerate(propias, 1))
                    tabla = ("<h2>Ventas recientes</h2><table><tr><th>#</th><th>Hora</th><th>Producto</th><th>Cant.</th><th>Total</th>"
                             "<th>Cliente</th></tr>" + filas + "</table>")
                else:
                    filas = "".join(f"<tr><td>#{i}</td><td>2026-11-21</td><td>40.00</td><td>admin</td><td>Mostrador</td></tr>"
                                    for i, _ in enumerate(propias, 1))
                    tabla = ("<h2>Ultimas ventas</h2><table><tr><th>ID</th><th>Fecha</th><th>Total</th><th>Vendedor</th><th>Cliente</th></tr>"
                             + filas + "</table>")
            return self.responder(200, f"<p>Hola {u}</p>" + menu + tabla)
        if self.path == "/productos":
            filas = "".join(f"<li>{n}" + (f" costo {c}" if admin or COSTO == "muestra" else "") + f" venta {v}</li>"
                            for n, c, v in PRODUCTOS)
            alta = ('<form method="post" action="/productos/nuevo"><input name="nombre"><input name="precio_compra">'
                    '<input name="precio_venta"><input name="stock"><button>Guardar</button></form>')
            return self.responder(200, "<h1>Productos</h1>" + menu + f"<ul>{filas}</ul>" + (alta if admin else ""))
        if self.path in ("/reportes", "/usuarios"):
            if not admin and ABRE == "sin_aviso":  # un 200 con el menú y nada más: ni ganancias ni formulario
                return self.responder(200, "<h1>Seccion</h1>" + menu + "<p>Esta seccion no esta disponible para tu cuenta.</p>")
            if not admin and ABRE == "parcial":  # el informe de ganancias, sin el bloque que solo ve el administrador
                if self.path == "/usuarios":
                    return self.responder(302, "", [("Location", "/")])
                return self.responder(200, "<h1>Reportes</h1>" + menu + "<table><tr><th>Dia</th><th>Ingresos</th><th>Ganancia</th></tr></table>")
            if not admin and ABRE == "no":
                return self.responder(302, "", [("Location", "/")])
            if not admin and ABRE == "403":
                return self.responder(403, "Forbidden")
            if not admin and ABRE == "propio":
                return self.responder(200, "<h1>Mis ventas</h1><p>Aqui salen solo tus propias ventas del dia, una por una, "
                                           "con su hora y su importe; nada del resto del equipo.</p>")
            if self.path == "/reportes":
                return self.responder(200, "<h1>Reportes</h1>" + menu + "<p>Ganancia total del mes: 24.00 sobre ventas por 40.00; "
                                                                   "productos mas vendidos y movimiento de caja.</p>")
            alta = ""
            if USUARIOS == "con":
                alta = ('<form method="post" action="/usuarios/nuevo"><input name="usuario"><input type="password" name="clave">'
                        '<select name="rol"><option value="admin">Administrador</option><option value="empleado">Empleado</option>'
                        '</select><button>Crear</button></form>')
            lista = "".join(f"<li>{n} ({r})</li>" for n, (_, r) in USERS.items())
            return self.responder(200, "<h1>Usuarios</h1>" + menu + f"<ul>{lista}</ul>" + alta)
        self.responder(200, "<p>pagina</p>")

    def do_POST(self):
        datos = dict(urllib.parse.parse_qsl(self.rfile.read(int(self.headers["Content-Length"])).decode()))
        if self.path == "/entrar":
            usuario = datos.get("usuario", "")
            if usuario in USERS and USERS[usuario][0] == datos.get("clave") and (usuario == "admin" or EMPLEADO_ENTRA == "si"):
                return self.responder(302, "", [("Location", "/"), ("Set-Cookie", f"sesion={usuario}; Path=/")])
            return self.responder(200, '<form method="post" action="/entrar"><input name="usuario">'
                                       '<input type="password" name="clave"><button>Entrar</button></form><p>no</p>')
        u = self.quien()
        if u is None or USERS[u][1] != "admin":
            return self.responder(403, "Forbidden")
        if self.path == "/productos/nuevo":
            PRODUCTOS.append((datos.get("nombre", ""), datos.get("precio_compra", ""), datos.get("precio_venta", "")))
        if self.path == "/usuarios/nuevo":
            USERS[datos.get("usuario", "")] = (datos.get("clave", ""), datos.get("rol", "empleado"))
        if self.path == "/ventas/nueva":
            VENTAS_HECHAS.append((datos.get("producto", ""), datos.get("cantidad", "")))
            return self.responder(302, "", [("Location", f"/ventas/{len(VENTAS_HECHAS)}")])
        self.responder(302, "", [("Location", "/")])


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 5000), Web).serve_forever()
'''


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_empleado_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)

    def recorrido(self, costo="oculta", abre="no", usuarios="con", entra="si", ventas="no", panel="no") -> dict:
        fuente = WEB
        for k, v in {"COSTO": costo, "ABRE": abre, "USUARIOS": usuarios, "ENTRA": entra, "VENTAS": ventas, "PANEL": panel}.items():
            fuente = fuente.replace(f"__{k}__", v)
        (self.dir / "app.py").write_text(fuente, encoding="utf-8")
        return recorrer(self.dir / "app.py", sys.executable, [("admin", "pan123")])


class EmpleadoVeLoDelAdmin(Base):
    def test_el_precio_de_compra_llega_al_empleado(self):
        e = self.recorrido(costo="muestra")["empleado"]
        self.assertTrue(e["entro"] and e["producto"])
        self.assertEqual(e["costo"], ["/productos"])
        self.assertEqual(e["abre"], [])

    def test_reportes_y_usuarios_abiertos_al_empleado(self):
        e = self.recorrido(abre="si")["empleado"]
        motivos = {s["ruta"]: s["motivo"] for s in e["abre"]}
        self.assertEqual(motivos, {"/reportes": "ganancias", "/usuarios": "alta de usuarios", "/informes": "igual"})
        self.assertGreaterEqual(next(s for s in e["abre"] if s["ruta"] == "/informes")["parecido"], 0.85)
        self.assertEqual(e["costo"], [])

    def test_el_informe_de_ganancias_con_menos_cosas_que_el_del_admin_tambien_cuenta(self):
        """m4138_neg3: el empleado veía «Ganancias por día» sin el ranking de vendedores: no era igual, pero era el informe."""
        e = self.recorrido(abre="parcial")["empleado"]
        self.assertEqual(e["abre"], [{"ruta": "/reportes", "motivo": "ganancias"}])

    def test_un_enlace_de_ganancias_en_el_menu_no_es_ver_ganancias(self):
        self.assertEqual(self.recorrido(abre="sin_aviso")["empleado"]["abre"], [])

    def test_todo_cerrado_no_dice_nada(self):
        e = self.recorrido()["empleado"]
        self.assertTrue(e["entro"])
        self.assertEqual((e["costo"], e["abre"]), ([], []))

    def test_un_403_o_una_redireccion_es_cerrar(self):
        self.assertEqual(self.recorrido(abre="403")["empleado"]["abre"], [])

    def test_una_pagina_propia_con_el_mismo_nombre_no_es_la_del_admin(self):
        self.assertEqual(self.recorrido(abre="propio")["empleado"]["abre"], [])

    def test_sin_pantalla_de_usuarios_no_se_puede_y_no_se_dice_nada(self):
        self.assertIsNone(self.recorrido(usuarios="sin")["empleado"])

    def test_si_el_empleado_no_entra_tampoco_se_dice_nada(self):
        e = self.recorrido(costo="muestra", abre="si", entra="no")["empleado"]
        self.assertFalse(e["entro"])
        self.assertEqual((e["costo"], e["abre"]), ([], []))


class EmpleadoYLasVentas(Base):
    """m4141: el empleado veía la factura del administrador (neg4) y recibía un 500 en el detalle de una venta (neg2). La segunda lectura
    corre DESPUÉS del recorrido de siempre, que ya vendió con los formularios de la web."""

    def test_ventas_de_otros_abiertas_al_empleado(self):
        e = self.recorrido(ventas="ajenas")["empleado"]
        self.assertTrue(e["fase2"])
        motivos = {a["ruta"]: a["motivo"] for a in e["ajenas"]}
        self.assertEqual(motivos.get("/ventas/1"), "detalle")
        self.assertEqual(motivos.get("/ventas"), "filas")
        self.assertEqual(e["error"], [])

    def test_solo_las_suyas_no_dice_nada(self):
        e = self.recorrido(ventas="filtra")["empleado"]
        self.assertTrue(e["fase2"])
        self.assertEqual((e["ajenas"], e["error"]), ([], []))

    def test_un_500_solo_del_empleado_en_el_detalle(self):
        e = self.recorrido(ventas="500")["empleado"]
        self.assertTrue(e["fase2"])
        self.assertEqual(e["error"], ["/ventas/1"])

    def test_sin_ninguna_pagina_cerrada_en_la_primera_lectura_no_se_mira(self):
        """Sin el canario (páginas que el empleado no puede abrir) no se sabe si el recorrido lo hizo administrador: silencio."""
        e = self.recorrido(abre="si", ventas="ajenas")["empleado"]
        self.assertFalse(e["fase2"])
        self.assertEqual((e["ajenas"], e["error"]), ([], []))

    def test_sin_ventas_no_hay_nada_que_decir(self):
        e = self.recorrido()["empleado"]
        self.assertEqual((e["ajenas"], e["error"]), ([], []))

    def test_el_panel_de_inicio_con_las_ventas_de_otros_vendedores(self):
        """Ferretería (18): /ventas e /historial filtraban por vendedor, pero «Últimas ventas» de «/» le enseñaba al empleado las del admin."""
        e = self.recorrido(ventas="filtra", panel="ajeno")["empleado"]
        self.assertTrue(e["fase2"])
        self.assertEqual(e["ajenas"], [{"ruta": "/", "motivo": "vendedor", "vendedores": ["admin"]}])

    def test_el_panel_que_solo_enseña_lo_suyo_no_dice_nada(self):
        e = self.recorrido(ventas="filtra", panel="filtrado")["empleado"]
        self.assertTrue(e["fase2"])
        self.assertEqual(e["ajenas"], [])

    def test_sin_ventas_el_panel_no_dice_nada(self):
        self.assertEqual(self.recorrido(panel="ajeno")["empleado"]["ajenas"], [])

    def test_la_tabla_de_ventas_de_todos_sin_columna_de_vendedor(self):
        """Ferretería (22): /ventas/nueva enseñaba «ventas recientes» de todos, sin vendedor y sin enlaces: ni «vendedor» ni «filas» la cogían."""
        e = self.recorrido(ventas="filtra", panel="sin_col")["empleado"]
        self.assertTrue(e["fase2"])
        self.assertEqual(e["ajenas"], [{"ruta": "/", "motivo": "tabla", "filas": 1, "importe": "$450.00"}])

    def test_la_misma_tabla_filtrada_por_vendedor_no_dice_nada(self):
        e = self.recorrido(ventas="filtra", panel="sin_col_filtrado")["empleado"]
        self.assertTrue(e["fase2"])
        self.assertEqual(e["ajenas"], [])

    def test_una_ruta_ya_avisada_por_sus_filas_no_se_repite_por_la_tabla(self):
        e = self.recorrido(ventas="ajenas", panel="sin_col")["empleado"]
        motivos = sorted((a["ruta"], a["motivo"]) for a in e["ajenas"])
        self.assertIn(("/ventas", "filas"), motivos)
        self.assertNotIn(("/ventas", "tabla"), motivos)
        self.assertIn(("/", "tabla"), motivos)

    def test_sin_ventas_la_tabla_no_dice_nada(self):
        self.assertEqual(self.recorrido(panel="sin_col")["empleado"]["ajenas"], [])


def tabla(cabeceras: list[str], *filas: list[str], th: bool = True) -> str:
    c = "".join(f"<{'th' if th else 'td'}>{x}</{'th' if th else 'td'}>" for x in cabeceras)
    f = "".join("<tr>" + "".join(f"<td>{x}</td>" for x in fila) + "</tr>" for fila in filas)
    return f"<table><tr>{c}</tr>{f}</table>"


class VendedoresDeUnaTabla(unittest.TestCase):
    def test_columna_de_vendedor_con_importe(self):
        self.assertEqual(_vendedores_ajenos(tabla(["ID", "Total", "Vendedor"], ["#1", "5.00", "admin"], ["#2", "7.00", "admin"])), ["admin"])
        self.assertEqual(_vendedores_ajenos(tabla(["Fecha", "Importe", "Atendido por"], ["hoy", "5.00", "ana"], ["hoy", "7.00", "luis"])),
                         ["ana", "luis"])
        self.assertEqual(_vendedores_ajenos(tabla(["Sold by", "Amount"], ["maria", "10.00"])), ["maria"])

    def test_la_cabecera_debil_pide_contexto_de_venta(self):
        self.assertEqual(_vendedores_ajenos(tabla(["Fecha", "Total", "Usuario"], ["hoy", "5.00", "ana"])), ["ana"])
        self.assertEqual(_vendedores_ajenos(tabla(["Cliente", "Total", "Empleado"], ["Pedro", "5.00", "ana"])), ["ana"])
        self.assertEqual(_vendedores_ajenos(tabla(["Usuario", "Total"], ["ana", "5.00"])), [])  # una lista de usuarios con un total: no es de ventas

    def test_sin_importe_no_es_una_tabla_de_ventas(self):
        self.assertEqual(_vendedores_ajenos(tabla(["Vendedor", "Comision"], ["ana", "5 %"])), [])
        self.assertEqual(_vendedores_ajenos(tabla(["Vendedor", "Zona"], ["ana", "Norte"])), [])

    def test_sin_vendedor_ni_el_propio_empleado(self):
        self.assertEqual(_vendedores_ajenos(tabla(["Total", "Vendedor"], ["5.00", ""], ["5.00", "N/A"], ["5.00", "-"], ["5.00", "—"],
                                                  ["5.00", "Sin vendedor"], ["5.00", "Total"], ["5.00", "12"])), [])
        self.assertEqual(_vendedores_ajenos(tabla(["Total", "Vendedor"], ["5.00", "empleado_aura"], ["5.00", "Empleado Aura"])), [])
        self.assertEqual(_vendedores_ajenos(tabla(["Total", "Vendedor"], ["5.00", "empleado_aura"], ["5.00", "admin"])), ["admin"])

    def test_tablas_raras_no_dicen_nada_ni_fallan(self):
        self.assertEqual(_vendedores_ajenos(tabla(["Total", "Vendedor"], ["5.00", "admin"], th=False)), [])  # sin <th> no hay cabecera
        self.assertEqual(_vendedores_ajenos(tabla(["Total", "Vendedor"], ["admin"])), [])  # fila con otro número de celdas
        self.assertEqual(_vendedores_ajenos("<table><tr><th>Total</th><th>Vendedor"), [])
        self.assertEqual(_vendedores_ajenos("<p>sin tablas</p>"), [])
        self.assertEqual(_vendedores_ajenos(""), [])
        self.assertIsInstance(_vendedores_ajenos("<table><tr><th>Total</th><th>Vendedor</th></tr><tr><td>5</td><td><table><tr><td>x</td></tr>"
                                                 "</table>ana</td></tr></table>"), list)  # una tabla dentro de otra no rompe nada


CABECERA_VENTA = ["#", "Hora", "Producto", "Cant.", "Total", "Cliente"]  # la tabla de «ventas recientes» de la ferretería (22)


class VentasEnUnaTabla(unittest.TestCase):

    def test_la_tabla_de_la_pantalla_de_venta(self):
        self.assertEqual(_ventas_en_tabla(tabla(CABECERA_VENTA, ["1", "10:00", "Martillo", "3", "$450.00", "Pedro"],
                                                ["2", "10:05", "Clavos", "1", "$2.00", "—"])), (2, "$450.00"))

    def test_otras_cabeceras_de_ventas(self):
        self.assertEqual(_ventas_en_tabla(tabla(["Factura", "Total"], ["F-1", "5.00"])), (1, "5.00"))
        self.assertEqual(_ventas_en_tabla(tabla(["Cliente", "Fecha", "Importe"], ["Pedro", "hoy", "7,50"])), (1, "7,50"))
        self.assertEqual(_ventas_en_tabla(tabla(["Customer", "Item", "Amount"], ["Ann", "bolt", "3.00"])), (1, "3.00"))

    def test_las_listas_de_productos_o_clientes_no_son_de_ventas(self):
        self.assertIsNone(_ventas_en_tabla(tabla(["Producto", "Precio", "Existencia"], ["Martillo", "150", "10"])))
        self.assertIsNone(_ventas_en_tabla(tabla(["Producto", "Total en inventario"], ["Martillo", "1500.00"])))
        self.assertIsNone(_ventas_en_tabla(tabla(["Producto", "Fecha de ingreso", "Total"], ["Martillo", "hoy", "1500.00"])))
        self.assertIsNone(_ventas_en_tabla(tabla(["Cliente", "Teléfono", "Total comprado"], ["Pedro", "809", "450.00"])))

    def test_filas_que_no_son_una_venta_con_importe(self):
        self.assertIsNone(_ventas_en_tabla(tabla(CABECERA_VENTA)))  # sin filas: una lista filtrada que sale vacía
        self.assertIsNone(_ventas_en_tabla("<table><tr><th>Hora</th><th>Total</th><th>Cliente</th></tr><tr><td>No hay ventas.</td></tr></table>"))
        self.assertIsNone(_ventas_en_tabla(tabla(["Hora", "Total", "Cliente"], ["10:00", "0.00", "Pedro"], ["10:01", "—", "Ana"])))
        self.assertIsNone(_ventas_en_tabla(tabla(CABECERA_VENTA, ["Total", "", "", "", "450.00", ""])))  # fila de totales
        self.assertEqual(_ventas_en_tabla(tabla(CABECERA_VENTA, ["Total", "", "", "", "450.00", ""],
                                                ["1", "10:00", "Martillo", "3", "$150.00", "Pedro"])), (1, "$150.00"))

    def test_el_propio_empleado_no_cuenta(self):
        self.assertIsNone(_ventas_en_tabla(tabla(["Fecha", "Total", "Cliente"], ["hoy", "5.00", "empleado_aura"])))

    def test_con_columna_de_vendedor_la_juzga_la_otra_regla(self):
        self.assertIsNone(_ventas_en_tabla(tabla(["ID", "Total", "Vendedor", "Cliente"], ["1", "5.00", "admin", "Pedro"])))
        self.assertIsNone(_ventas_en_tabla(tabla(["Cliente", "Total", "Usuario"], ["Pedro", "5.00", "ana"])))
        self.assertEqual(_vendedores_ajenos(tabla(["Cliente", "Total", "Usuario"], ["Pedro", "5.00", "ana"])), ["ana"])

    def test_tablas_raras_no_dicen_nada_ni_fallan(self):
        self.assertIsNone(_ventas_en_tabla(tabla(CABECERA_VENTA, ["1", "10:00", "Martillo", "3", "$450.00", "Pedro"], th=False)))
        self.assertIsNone(_ventas_en_tabla(tabla(CABECERA_VENTA, ["1", "$450.00"])))  # fila con otro número de celdas
        self.assertIsNone(_ventas_en_tabla("<table><tr><th>Total</th><th>Cliente"))
        self.assertIsNone(_ventas_en_tabla("<p>sin tablas</p>"))
        self.assertIsNone(_ventas_en_tabla(""))
        _ventas_en_tabla("<table><tr><th>Total</th><th>Cliente</th></tr><tr><td>5</td><td><table><tr><td>x</td></tr></table>Pedro</td></tr></table>")


class MensajesDeVerificar(unittest.TestCase):
    def mensajes(self, empleado: dict | None) -> list[str]:
        raiz = Path(tempfile.mkdtemp(prefix="test_empleado_msg_"))
        self.addCleanup(shutil.rmtree, raiz, ignore_errors=True)
        (raiz / "app.py").write_text("print('x')\n", encoding="utf-8")
        falso = {"errores": [], "plantillas": [], "rotos": [], "raros": [], "enviados": [], "paginas": 3, "formularios": 2,
                 "usuario": "admin", "sin_entrar": False, "entrar_js": False, "navegador": None, "lineas": None,
                 "sin_formulario": [], "registro": False, "entrar_get": False, "sin_permiso": [], "login_vacio": None,
                 "empleado": empleado}
        inf = verificacion.Informe()
        with mock.patch.object(verificacion, "recorrer", return_value=falso):
            verificacion.probar_web(raiz, raiz / "app.py", sys.executable, inf)
        self.inf = inf
        return inf.errores

    def test_costo_visible(self):
        [m] = self.mensajes({"usuario": "empleado_aura", "entro": True, "producto": True, "costo": ["/productos", "/api/productos"],
                             "abre": []})
        self.assertIn("ve el costo", m)
        self.assertIn("/productos, /api/productos", m)

    def test_gestion_abierta(self):
        [m] = self.mensajes({"usuario": "empleado_aura", "entro": True, "producto": True, "costo": [],
                             "abre": [{"ruta": "/reportes", "motivo": "ganancias"}]})
        self.assertIn("abre /reportes y enseña ganancias, utilidades o costos", m)
        [m] = self.mensajes({"usuario": "empleado_aura", "entro": True, "producto": True, "costo": [],
                             "abre": [{"ruta": "/informes", "motivo": "igual", "parecido": 0.97}]})
        self.assertIn("abre /informes y la ve igual que el administrador (parecido 0.97)", m)
        [m] = self.mensajes({"usuario": "empleado_aura", "entro": True, "producto": True, "costo": [],
                             "abre": [{"ruta": "/usuarios", "motivo": "alta de usuarios"}]})
        self.assertIn("abre /usuarios y puede dar de alta usuarios", m)

    def test_error_de_programa_del_empleado(self):
        [m] = self.mensajes({"usuario": "empleado_aura", "entro": True, "producto": True, "costo": [], "abre": [],
                             "error": ["/ventas/1", "/factura/1"], "ajenas": []})
        self.assertIn("recibe un error de programa (500) al abrir /ventas/1, /factura/1", m)
        self.assertIn("donde el administrador la abre bien", m)

    def test_ventas_de_otros_es_un_aviso_y_no_un_error(self):
        self.assertEqual(self.mensajes({"usuario": "empleado_aura", "entro": True, "producto": True, "costo": [], "abre": [],
                                        "error": [], "ajenas": [{"ruta": "/factura/1", "motivo": "detalle", "parecido": 1.0},
                                                                {"ruta": "/facturas", "motivo": "filas"}]}), [])
        [m] = self.inf.avisos
        self.assertIn("abre /factura/1, /facturas: ventas o facturas de otros usuarios (él no ha vendido nada)", m)
        self.assertIn("ignora este aviso", m)

    def test_el_panel_con_ventas_de_otros_vendedores_es_un_aviso(self):
        self.assertEqual(self.mensajes({"usuario": "empleado_aura", "entro": True, "producto": True, "costo": [], "abre": [], "error": [],
                                        "ajenas": [{"ruta": "/", "motivo": "vendedor", "vendedores": ["admin", "ana"]}]}), [])
        [m] = self.inf.avisos
        self.assertIn("ve en / (vendedor «admin», «ana») una lista de ventas de otros vendedores (él no ha vendido nada)", m)
        self.assertIn("«Últimas ventas» de todos aunque /ventas ya filtre", m)
        self.assertIn("ignora este aviso", m)

    def test_una_ruta_ya_avisada_por_sus_filas_no_se_repite_por_el_vendedor(self):
        self.mensajes({"usuario": "empleado_aura", "entro": True, "producto": True, "costo": [], "abre": [], "error": [],
                       "ajenas": [{"ruta": "/ventas", "motivo": "filas"}, {"ruta": "/ventas", "motivo": "vendedor", "vendedores": ["admin"]}]})
        [m] = self.inf.avisos
        self.assertIn("abre /ventas: ventas o facturas de otros usuarios", m)

    def test_la_tabla_de_ventas_de_otros_sin_vendedor_es_un_aviso(self):
        self.assertEqual(self.mensajes({"usuario": "empleado_aura", "entro": True, "producto": True, "costo": [], "abre": [], "error": [],
                                        "ajenas": [{"ruta": "/ventas/nueva", "motivo": "tabla", "filas": 2, "importe": "$450.00"}]}), [])
        [m] = self.inf.avisos
        self.assertIn("ve en /ventas/nueva (2 filas, por ejemplo con importe $450.00) una tabla con ventas de otros usuarios", m)
        self.assertIn("(él no ha vendido nada)", m)
        self.assertIn("filtra esa lista por el vendedor en la consulta", m)
        self.assertIn("ignora este aviso", m)
        self.mensajes({"usuario": "empleado_aura", "entro": True, "producto": True, "costo": [], "abre": [], "error": [],
                       "ajenas": [{"ruta": "/", "motivo": "tabla", "filas": 1, "importe": "5.00"}]})
        self.assertIn("ve en / (1 fila, por ejemplo con importe 5.00)", self.inf.avisos[0])

    def test_una_ruta_ya_avisada_no_se_repite_por_la_tabla(self):
        self.mensajes({"usuario": "empleado_aura", "entro": True, "producto": True, "costo": [], "abre": [], "error": [],
                       "ajenas": [{"ruta": "/ventas", "motivo": "filas"}, {"ruta": "/ventas", "motivo": "tabla", "filas": 1, "importe": "5.00"},
                                  {"ruta": "/", "motivo": "vendedor", "vendedores": ["admin"]},
                                  {"ruta": "/", "motivo": "tabla", "filas": 1, "importe": "5.00"}]})
        self.assertEqual(len(self.inf.avisos), 2)
        self.assertNotIn("una tabla con ventas de otros usuarios", " ".join(self.inf.avisos))

    def test_nada_si_no_hay_empleado_o_todo_cerrado(self):
        self.assertEqual(self.mensajes(None), [])
        self.assertNotIn("empleado", " ".join(self.inf.ok))
        self.assertEqual(self.mensajes({"usuario": "e", "entro": True, "producto": True, "costo": [], "abre": []}), [])
        self.assertIn("como un empleado creado desde la pantalla de usuarios", " ".join(self.inf.ok))


if __name__ == "__main__":
    unittest.main()
