"""verificar mira lo primero que abre el dueño cuando ya hay ventas: el informe de ganancias por defecto en 0 y la lista de facturas vacía
(m4141: el informe salía 0.00 con una venta de hoy; otra entrega creaba la factura y no la guardaba). Son avisos."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import verificacion
from agente.recorrido_web import recorrer

WEB = r'''import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer

INFORME = "__INFORME__"      # "bien" (cifras de las ventas) o "cero" (todo en 0.00)
FACTURAS = "__FACTURAS__"    # "con" (filas), "vacias", "generar" (vacía con un botón para generarlas) o "no" (no hay facturas)
GUARDA = "__GUARDA__"        # "si" o "no" (la venta no se guarda: no hay evidencia de ventas)
VENTAS = []
SESION = {"u": None}


class Web(BaseHTTPRequestHandler):
    def responder(self, estado, cuerpo, cabeceras=()):
        self.send_response(estado)
        for k, v in cabeceras:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(cuerpo.encode())

    def do_GET(self):
        if self.path == "/entrar":
            return self.responder(200, '<form method="post" action="/entrar"><input name="usuario">'
                                       '<input type="password" name="clave"><button>Entrar</button></form>')
        if "sesion=admin" not in (self.headers.get("Cookie") or ""):
            return self.responder(302, "", [("Location", "/entrar")])
        menu = ('<nav><a href="/ventas">Ventas</a> <a href="/reportes">Reportes</a>'
                + (' <a href="/facturas">Facturas</a>' if FACTURAS != "no" else "") + '</nav>')
        if self.path == "/":
            return self.responder(200, "<h1>Inicio</h1>" + menu)
        if self.path == "/ventas":
            filas = "".join(f'<li><a href="/ventas/{i}">Venta {i}</a> total 40.00</li>' for i, _ in enumerate(VENTAS, 1))
            alta = '<a href="/ventas/nueva">Nueva venta</a>'
            return self.responder(200, "<h1>Ventas</h1>" + menu + (f"<ul>{filas}</ul>" if filas else "<p>No hay ventas.</p>") + alta)
        if self.path == "/ventas/nueva":  # el formulario enseña precios de productos: no es una venta hecha
            return self.responder(200, "<h1>Nueva venta</h1>" + menu + "<p>Tornillo, precio 21.31</p>"
                                       '<form method="post" action="/ventas/nueva"><input name="producto"><input type="number" name="cantidad">'
                                       "<button>Vender</button></form>")
        if self.path.startswith("/ventas/"):
            return self.responder(200, "<h1>Venta</h1>" + menu + "<p>Venta de dos tornillos, total 40.00, pagada.</p>")
        if self.path == "/reportes":
            if INFORME == "cero":
                return self.responder(200, "<h1>Reportes</h1>" + menu + "<p>Periodo: este mes. Ingresos $0.00 Costos $0.00 Ganancia neta $0.00</p>")
            total = 40.0 * len(VENTAS)
            return self.responder(200, "<h1>Reportes</h1>" + menu + f"<p>Ingresos ${total:.2f} Costos ${total * 0.4:.2f} Ganancia neta ${total * 0.6:.2f}</p>")
        if self.path == "/facturas" and FACTURAS != "no":
            if FACTURAS == "con" and VENTAS:
                filas = "".join(f'<li><a href="/facturas/{i}">FAC-000{i}</a> 40.00</li>' for i, _ in enumerate(VENTAS, 1))
                return self.responder(200, "<h1>Facturas</h1>" + menu + f"<ul>{filas}</ul>")
            extra = '<a href="/facturas/generar">Generar factura</a>' if FACTURAS == "generar" else ""
            return self.responder(200, "<h1>Facturas</h1>" + menu + "<p>No hay facturas generadas.</p>" + extra)
        self.responder(404, "no")

    def do_POST(self):
        datos = dict(urllib.parse.parse_qsl(self.rfile.read(int(self.headers["Content-Length"])).decode()))
        if self.path == "/entrar":
            if datos.get("usuario") == "admin" and datos.get("clave") == "pan123":
                return self.responder(302, "", [("Location", "/"), ("Set-Cookie", "sesion=admin; Path=/")])
            return self.responder(200, '<form method="post" action="/entrar"><input name="usuario"><input type="password" name="clave"></form>')
        if self.path == "/ventas/nueva" and GUARDA == "si":
            VENTAS.append(datos)
        self.responder(302, "", [("Location", "/ventas")])


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 5000), Web).serve_forever()
'''


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_informe_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)

    def negocio(self, informe="bien", facturas="con", guarda="si") -> dict:
        fuente = WEB
        for k, v in {"INFORME": informe, "FACTURAS": facturas, "GUARDA": guarda}.items():
            fuente = fuente.replace(f"__{k}__", v)
        (self.dir / "app.py").write_text(fuente, encoding="utf-8")
        return recorrer(self.dir / "app.py", sys.executable, [("admin", "pan123")])["negocio"]


class InformeYFacturas(Base):
    def test_informe_en_cero_con_ventas(self):
        n = self.negocio(informe="cero")
        self.assertEqual(n["informe_cero"], ["/reportes"])
        self.assertEqual(n["ventas"], ("/ventas", "40.00"))

    def test_informe_con_cifras_no_dice_nada(self):
        n = self.negocio(informe="bien")
        self.assertEqual((n["informe_cero"], n["paginas_informe"]), ([], 1))

    def test_facturas_vacias_con_ventas(self):
        self.assertEqual(self.negocio(facturas="vacias")["facturas_vacias"], ["/facturas"])

    def test_facturas_con_filas_no_dice_nada(self):
        n = self.negocio(facturas="con")
        self.assertEqual((n["facturas_vacias"], n["paginas_facturas"]), ([], 1))

    def test_vacia_pero_con_boton_de_generar_no_dice_nada(self):
        self.assertEqual(self.negocio(facturas="generar")["facturas_vacias"], [])

    def test_sin_pagina_de_facturas_no_dice_nada(self):
        n = self.negocio(facturas="no")
        self.assertEqual((n["facturas_vacias"], n["paginas_facturas"]), ([], 0))

    def test_sin_ninguna_venta_guardada_no_se_dice_nada(self):
        n = self.negocio(informe="cero", facturas="vacias", guarda="no")
        self.assertIsNone(n["ventas"])
        self.assertEqual((n["informe_cero"], n["facturas_vacias"]), ([], []))


class MensajesDeVerificar(unittest.TestCase):
    def avisos(self, negocio: dict) -> list[str]:
        raiz = Path(tempfile.mkdtemp(prefix="test_informe_msg_"))
        self.addCleanup(shutil.rmtree, raiz, ignore_errors=True)
        (raiz / "app.py").write_text("print('x')\n", encoding="utf-8")
        falso = {"errores": [], "plantillas": [], "rotos": [], "raros": [], "enviados": [], "paginas": 3, "formularios": 2,
                 "usuario": "admin", "sin_entrar": False, "entrar_js": False, "navegador": None, "lineas": None,
                 "sin_formulario": [], "registro": False, "entrar_get": False, "sin_permiso": [], "login_vacio": None,
                 "empleado": None, "negocio": negocio}
        inf = verificacion.Informe()
        with mock.patch.object(verificacion, "recorrer", return_value=falso):
            verificacion.probar_web(raiz, raiz / "app.py", sys.executable, inf)
        self.assertEqual(inf.errores, [])  # son avisos, nunca errores
        return inf.avisos

    def test_informe_en_cero(self):
        [m] = self.avisos({"ventas": ("/ventas", "60.00"), "informe_cero": ["/reportes"], "facturas_vacias": []})
        self.assertIn("el informe /reportes enseña todos sus importes en 0 aunque hay ventas (60.00 en /ventas)", m)
        self.assertIn("UTC frente a la hora local", m)

    def test_facturas_vacias(self):
        [m] = self.avisos({"ventas": ("/ventas", "60.00"), "informe_cero": [], "facturas_vacias": ["/facturas"]})
        self.assertIn("la lista de facturas /facturas está vacía", m)
        self.assertIn("falta el commit", m)

    def test_nada_si_no_hay_nada_que_decir(self):
        self.assertEqual(self.avisos({"ventas": None, "informe_cero": [], "facturas_vacias": []}), [])
        self.assertEqual(self.avisos({}), [])


if __name__ == "__main__":
    unittest.main()
