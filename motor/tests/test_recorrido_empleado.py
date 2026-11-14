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
from agente.recorrido_web import recorrer

WEB = r'''import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer

COSTO = "__COSTO__"        # "muestra" u "oculta" el precio de compra al empleado
ABRE = "__ABRE__"          # reportes y usuarios para el empleado: "si", "no" (redirige), "403" o "propio" (otra página)
USUARIOS = "__USUARIOS__"  # "con" o "sin" formulario de alta de usuarios
EMPLEADO_ENTRA = "__ENTRA__"
USERS = {"admin": ("pan123", "admin")}
PRODUCTOS = []


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
                '<a href="/informes">Informes</a></nav>')
        if self.path == "/informes":  # una sección de gestión sin la palabra ganancia: solo se nota que se ve igual
            if not admin and ABRE == "403":
                return self.responder(403, "Forbidden")
            if not admin and ABRE in ("no", "propio", "parcial", "sin_aviso"):
                return self.responder(302, "", [("Location", "/")])
            return self.responder(200, "<h1>Informes</h1>" + menu + "<p>Inventario valorado por categoria, rotacion de productos, "
                                                                   "vendedores con mas movimiento y dias de mas actividad del mes.</p>")
        if self.path == "/":
            return self.responder(200, f"<p>Hola {u}</p>" + menu)
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
        self.responder(302, "", [("Location", "/")])


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 5000), Web).serve_forever()
'''


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_empleado_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)

    def recorrido(self, costo="oculta", abre="no", usuarios="con", entra="si") -> dict:
        fuente = WEB
        for k, v in {"COSTO": costo, "ABRE": abre, "USUARIOS": usuarios, "ENTRA": entra}.items():
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

    def test_nada_si_no_hay_empleado_o_todo_cerrado(self):
        self.assertEqual(self.mensajes(None), [])
        self.assertNotIn("empleado", " ".join(self.inf.ok))
        self.assertEqual(self.mensajes({"usuario": "e", "entro": True, "producto": True, "costo": [], "abre": []}), [])
        self.assertIn("como un empleado creado desde la pantalla de usuarios", " ".join(self.inf.ok))


if __name__ == "__main__":
    unittest.main()
