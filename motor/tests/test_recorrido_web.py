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
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.herramientas import Herramientas
from agente.recorrido_web import _forma, recorrer
from agente.verificacion import canal_navegador, claves_del_proyecto, pares_del_proyecto, usuarios_del_proyecto

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


WEB_JS = r'''import json
import traceback
from http.server import BaseHTTPRequestHandler, HTTPServer

PAGINA = """<!doctype html><html><body>
<div id="entrar"><input id="email" type="email"><input id="clave" type="password"><button onclick="entrar()">Entrar</button></div>
<div id="app" style="display:none">
  <nav><button data-section="productos" onclick="ver('productos')">Productos</button>
       <button data-section="salir" onclick="location.href='/salir'">Salir</button></nav>
  <section id="productos" style="display:none"><button onclick="document.getElementById('form').style.display='block'">+ Nuevo</button>
    <div id="form" style="display:none"><input id="nombre"><input id="precio" type="number" value="0">
      <button onclick="guardar()">Guardar</button></div></section>
</div>
<script>
async function entrar() {
  const r = await fetch('/api/login', {method: 'POST', body: JSON.stringify({email: email.value, clave: clave.value})});
  if (r.ok) { document.getElementById('entrar').style.display = 'none'; document.getElementById('app').style.display = 'block'; }
}
function ver(s) { document.getElementById(s).style.display = 'block'; }
async function guardar() {
  await fetch('/api/productos', {method: 'POST', body: JSON.stringify({nombre: nombre.value, precio: precio.value})});
}
</script></body></html>"""


class Web(BaseHTTPRequestHandler):
    def responder(self, estado, cuerpo, tipo="text/html"):
        self.send_response(estado)
        self.send_header("Content-Type", tipo)
        self.end_headers()
        self.wfile.write(cuerpo.encode())

    def do_GET(self):
        self.responder(200, PAGINA) if self.path == "/" else self.responder(404, "no")

    def do_POST(self):
        datos = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if self.path == "/api/login":
            ok = datos == {"email": "ana@tienda.com", "clave": "pan123"}
            return self.responder(200 if ok else 401, "{}", "application/json")
        try:
            float(datos["precio"]) / 0  # guardar un producto da 500
        except Exception:
            traceback.print_exc()
            self.responder(500, "{}", "application/json")


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 5000), Web).serve_forever()
'''


class ConNavegador(unittest.TestCase):
    """La entrada se envía con JavaScript (campos sin name): lo de dentro se usa con el navegador oculto."""

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_recorrido_"))
        (self.dir / "app.py").write_text(WEB_JS, encoding="utf-8")

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_entra_abre_la_seccion_y_guarda_el_formulario_que_falla(self):
        r = recorrer(self.dir / "app.py", sys.executable, [("admin", "admin"), ("ana@tienda.com", "pan123")],
                     abrir_navegador=canal_navegador)
        if r["navegador"] is None:
            self.skipTest("sin navegador")
        self.assertTrue(r["entrar_js"])
        self.assertEqual(r["usuario"], "ana@tienda.com")
        [error] = r["errores"]
        self.assertEqual((error["que"], error["estado"]), ("guardar el formulario de «Productos» (POST /api/productos)", 500))
        self.assertIn("nombre=Prueba", error["datos"])
        self.assertIn("precio=15", error["datos"])  # el value="0" de un precio se cambia por uno de verdad
        self.assertIn("ZeroDivisionError", error["traza"])

    def test_sin_navegador_solo_avisa(self):
        r = recorrer(self.dir / "app.py", sys.executable, [("ana@tienda.com", "pan123")])
        self.assertTrue(r["entrar_js"])
        self.assertIsNone(r["navegador"])
        self.assertEqual(r["errores"], [])

    def navegar(self, web: str) -> dict:
        (self.dir / "app.py").write_text(web, encoding="utf-8")
        r = recorrer(self.dir / "app.py", sys.executable, [("ana@tienda.com", "pan123")], abrir_navegador=canal_navegador)
        if r["navegador"] is None:
            self.skipTest("sin navegador")
        return r["navegador"]

    def test_menu_que_no_cambia_la_pantalla(self):
        # Como la entrega cuyo menú llamaba a secciones que no estaban en la página: solo «Productos» enseña algo.
        menu = "".join(f"<button data-section='{s}' onclick=\"ver('{s}')\">{s.title()}</button>"
                       for s in ("clientes", "ventas", "reportes"))
        nav = self.navegar(WEB_JS.replace("<nav>", "<nav>" + menu))
        self.assertEqual(nav["menu_quieto"], ["Clientes", "Ventas", "Reportes"])
        self.assertEqual(nav["nuevo_sin_campos"], [])

    def test_boton_nuevo_que_no_abre_nada(self):
        nav = self.navegar(WEB_JS.replace("document.getElementById('form').style.display='block'", "abrirFormulario()"))
        [boton] = nav["nuevo_sin_campos"]
        self.assertEqual((boton["texto"], boton["seccion"]), ("+ Nuevo", "Productos"))
        self.assertIn("abrirFormulario", boton["js"][0])
        self.assertEqual(nav["menu_quieto"], [])  # un menú de una sola entrada que funciona


# «+ Nuevo producto» llevaba a una ruta que volvía a la lista sin formulario. «Nueva venta» sin productos también vuelve
# (con un aviso), pero con productos ya enseña el suyo: eso está bien.
WEB_NUEVO = r'''import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer

PRODUCTOS = []
PAGINAS = {
    "/": '<a href="/productos">Productos</a> <a href="/clientes">Clientes</a> <a href="/ventas">Ventas</a>',
    "/productos": '<a href="/productos/nuevo">+ Nuevo producto</a> <form method="get"><input name="buscar"></form>',
    "/clientes": '<a href="/clientes/nuevo">+ Nuevo cliente</a>',
    "/clientes/nuevo": '<form method="post" action="/clientes"><input name="nombre"><button>Guardar</button></form>',
    "/ventas": '<a href="/ventas/nueva">Nueva venta</a> <a href="/inventario">Inventario</a>',
    "/inventario": '<form method="post" action="/inventario"><input name="producto"><button>Dar de alta</button></form>',
}


class Web(BaseHTTPRequestHandler):
    def responder(self, estado, cuerpo, cabeceras=()):
        self.send_response(estado)
        for k, v in cabeceras:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(cuerpo.encode())

    def do_GET(self):
        ruta = self.path.split("?")[0]
        if ruta == "/productos/nuevo":
            return self.responder(302, "", [("Location", "/productos")])
        if ruta == "/ventas/nueva":
            if not PRODUCTOS:
                return self.responder(302, "", [("Location", "/ventas")])
            return self.responder(200, '<form method="post" action="/ventas"><input name="cantidad"><button>Vender</button></form>')
        self.responder(*((200, PAGINAS[ruta]) if ruta in PAGINAS else (404, "no")))

    def do_POST(self):
        datos = dict(urllib.parse.parse_qsl(self.rfile.read(int(self.headers["Content-Length"])).decode()))
        if self.path == "/inventario":
            PRODUCTOS.append(datos)
        self.responder(302, "", [("Location", "/")])


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 5000), Web).serve_forever()
'''


class SinFormulario(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_recorrido_"))
        (self.dir / "app.py").write_text(WEB_NUEVO, encoding="utf-8")

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_nuevo_que_vuelve_a_la_lista_sin_formulario(self):
        r = recorrer(self.dir / "app.py", sys.executable, [])
        self.assertEqual(r["sin_formulario"], [{"texto": "+ Nuevo producto", "desde": "/productos", "a": "/productos/nuevo"}])
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

    def test_nadie_puede_entrar_es_un_error(self):
        # Como la entrega cuyos tests dejaron en la base real un admin de prueba: el del README ya no entraba.
        (self.dir / "README.md").write_text("Entra con usuario `admin` y contraseña `admin123`.\n", encoding="utf-8")
        (self.dir / "app.py").write_text(WEB.replace('"pan123"', '"pan" + "123"'), encoding="utf-8")  # ni en el código
        r = Herramientas(self.dir).verificar_cmd()
        self.assertIn("app.py: nadie puede entrar en la web con los usuarios y contraseñas que da el proyecto", r)
        self.assertNotIn("usé la web de verdad", r)

    def test_con_registro_solo_avisa(self):
        # Una web cuyo README decía «regístrate para crear la cuenta de administrador»: una persona sí puede entrar.
        (self.dir / "README.md").write_text("Entra con usuario `admin` y contraseña `admin123`.\n", encoding="utf-8")
        web = WEB.replace('"pan123"', '"pan" + "123"').replace("<button>Entrar</button></form>",
                                                                  '<button>Entrar</button></form><a href="/registro">Regístrate</a>')
        (self.dir / "app.py").write_text(web, encoding="utf-8")
        r = Herramientas(self.dir).verificar_cmd()
        self.assertNotIn("nadie puede entrar", r)
        self.assertIn("la web tiene registro", r)

    def test_si_entrar_falla_no_lo_repite(self):
        (self.dir / "app.py").write_text(WEB.replace('if datos == {"usuario": "ana", "clave": "pan123"}:',
                                                     'if datos["no_existe"]:'), encoding="utf-8")
        r = Herramientas(self.dir).verificar_cmd()
        self.assertIn("app.py: al entrar como ", r)
        self.assertIn("KeyError", r)
        self.assertNotIn("nadie puede entrar", r)


class Credenciales(unittest.TestCase):
    """Una entrega daba admin/admin123 en un comentario sin comillas: la pareja quedaba la 15.ª de 96 y solo se prueban
    12, así que «nadie puede entrar» saltó en falso y el agente gastó la misión intentando arreglar una web que iba bien."""

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_credenciales_"))

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def accesos(self) -> list[tuple[str, str]]:
        claves = claves_del_proyecto(self.dir)
        usuarios = usuarios_del_proyecto(self.dir, claves)
        return list(dict.fromkeys(pares_del_proyecto(self.dir, claves, usuarios) + [(u, c) for c in claves for u in usuarios]))

    def test_pareja_en_un_comentario_sin_comillas_va_primero(self):
        (self.dir / "app.py").write_text(
            'def entrar(usuario: str, password: str):\n'
            '    password = request.form.get("password")\n'
            '    user = buscar_usuario(usuario)\n'
            '    return user\n\n'
            'def crear_admin():\n'
            '    # Crear admin por defecto (usuario: admin / contraseña: admin123)\n'
            '    clave = hashlib.sha256("admin123".encode()).hexdigest()\n'
            '    datos = {"nombre": "Administrador", "rol": "admin", "empleado": "Demo", "activo": 1}\n', encoding="utf-8")
        acc = self.accesos()
        self.assertEqual(acc[0], ("admin", "admin123"))
        self.assertFalse([p for p in acc if "request.form.get" in p or "str" in p])  # el código no son credenciales

    def test_readme_con_etiquetas_y_login_por_email(self):
        (self.dir / "README.md").write_text("- **Usuario:** `dueña@tienda.com`\n- **Contraseña:** `tienda2026`\n",
                                            encoding="utf-8")
        self.assertEqual(self.accesos()[0], ("dueña@tienda.com", "tienda2026"))

    def test_usuario_solo_en_un_insert_con_columnas(self):
        # m4134_neg1: sin README; probaba admin/Administrador y «nadie puede entrar» saltó 3 veces en falso.
        (self.dir / "app.py").write_text(
            'def init_db(db):\n    db.executescript("""\n'
            "        INSERT OR IGNORE INTO usuarios (nombre, username, password, rol) "
            "VALUES ('Administrador', 'admin', 'admin123', 'admin');\n"
            '    """)\n\n'
            'def crear(db, f):\n    db.execute("INSERT INTO usuarios (nombre, username, password, rol) VALUES (?,?,?,?)",\n'
            '               (f["nombre"], f["username"], f["password"], f["rol"]))\n', encoding="utf-8")
        self.assertEqual(self.accesos()[0], ("admin", "admin123"))
        self.assertNotIn("Administrador", claves_del_proyecto(self.dir))

    def test_usuario_por_defecto_con_barra(self):
        (self.dir / "app.py").write_text('print("   Usuario por defecto: admin / admin123")\n', encoding="utf-8")
        self.assertEqual(self.accesos()[0], ("admin", "admin123"))

    def test_un_hash_en_el_insert_no_es_una_contrasena(self):
        (self.dir / "seed.sql").write_text("INSERT INTO users (username, password) VALUES ('admin', "
                                           "'$2b$12$abcdefghijklmnopqrstuvwxyz0123');\n", encoding="utf-8")
        self.assertFalse([c for c in claves_del_proyecto(self.dir) if c.startswith("$2b$")])


class SinEscritorio(unittest.TestCase):
    """Durante las mediciones el agente abría el navegador del usuario con «start http://…» (m4134_neg1, m4132)."""

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_sin_escritorio_"))
        self.corridos = []
        self.parche = mock.patch("agente.herramientas.correr",
                                 lambda cmd, *a, **k: (self.corridos.append(cmd), (0, b"hola"))[1])
        self.parche.start()  # nada se ejecuta de verdad: si el arreglo fallara, no se abriría ningún navegador

    def tearDown(self):
        self.parche.stop()
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_con_la_variable_no_abre_el_navegador_y_corre_lo_demas(self):
        with mock.patch.dict(os.environ, {"AGENTE_SIN_ESCRITORIO": "1"}):
            r = Herramientas(self.dir).ejecutar('python seed.py && start "" http://127.0.0.1:5055 && python app.py')
        self.assertEqual(self.corridos, ["python seed.py && cd . && python app.py"])
        self.assertIn("No se abrió http://127.0.0.1:5055 en el navegador", r)
        self.assertIn("ver_pantalla(url='http://127.0.0.1:5055')", r)

    def test_sin_url_o_sin_la_variable_nada_cambia(self):
        with mock.patch.dict(os.environ, {"AGENTE_SIN_ESCRITORIO": "1"}):
            Herramientas(self.dir).ejecutar("start run.bat")
        with mock.patch.dict(os.environ, {"AGENTE_SIN_ESCRITORIO": ""}):
            Herramientas(self.dir).ejecutar("start http://127.0.0.1:5055/login")
        self.assertEqual(self.corridos, ["start run.bat", "start http://127.0.0.1:5055/login"])


class ServidorAusente(unittest.TestCase):
    """Una entrega acababa su run.bat en «python app.py», pero app.py solo definía la aplicación: nadie la ponía a servir,
    con doble clic no se veía nada y verificar decía «se ejecuta sin errores»."""
    SIN_SERVIR = "class FastAPI:\n    pass\n\n\napp = FastAPI()\nprint('listo', app)\n"

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_sin_servidor_"))
        (self.dir / "app.py").write_text(self.SIN_SERVIR, encoding="utf-8")

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def verificar(self):
        from agente.verificacion import verificar
        return verificar(self.dir, sys.executable, segundos=6)

    def test_con_bat_de_doble_clic_es_un_error(self):
        (self.dir / "run.bat").write_text('@echo off\ncd /d "%~dp0"\npython app.py\n', encoding="utf-8")
        informe, errores = self.verificar()
        self.assertIn("app.py: crea la aplicación web pero nunca la pone a servir", informe)
        self.assertIn("lo que abre run.bat", informe)
        self.assertNotIn("se ejecuta sin errores", informe)
        self.assertGreaterEqual(errores, 1, informe)

    def test_sin_bat_solo_avisa(self):
        informe, errores = self.verificar()
        self.assertIn("termina sin abrir ningún servidor", informe)
        self.assertNotIn("nunca la pone a servir", informe)
        self.assertNotIn("app.py se ejecuta sin errores", informe)
        self.assertEqual(errores, 0, informe)

    def test_si_el_proyecto_nombra_otra_forma_de_arrancar_no_dice_nada(self):
        (self.dir / "README.md").write_text("Arranca con `uvicorn app:app --reload`.\n", encoding="utf-8")
        informe, errores = self.verificar()
        self.assertNotIn("sin abrir ningún servidor", informe)
        self.assertIn("app.py se ejecuta sin errores", informe)
        self.assertEqual(errores, 0, informe)

    def test_un_programa_con_un_modo_web_opcional_no_se_marca(self):
        # Gestores de gastos de consola que solo sirven con el modo web: `python app.py` abre un menú y termina.
        (self.dir / "app.py").write_text(self.SIN_SERVIR + "\n\ndef modo_web(servidor):\n    servidor.serve_forever()\n",
                                         encoding="utf-8")
        (self.dir / "run.bat").write_text('@echo off\ncd /d "%~dp0"\npython app.py\n', encoding="utf-8")
        informe, _ = self.verificar()
        self.assertNotIn("nunca la pone a servir", informe)
        self.assertNotIn("sin abrir ningún servidor", informe)

    def test_una_web_que_si_sirve_no_cambia(self):
        (self.dir / "app.py").write_text(WEB, encoding="utf-8")
        (self.dir / "README.md").write_text("Entra con usuario `ana` y contraseña `pan123`.\n", encoding="utf-8")
        (self.dir / "run.bat").write_text('@echo off\ncd /d "%~dp0"\npython app.py\n', encoding="utf-8")
        informe, _ = self.verificar()
        self.assertNotIn("nunca la pone a servir", informe)
        self.assertNotIn("sin abrir ningún servidor", informe)


if __name__ == "__main__":
    unittest.main()
