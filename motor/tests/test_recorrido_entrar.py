"""El recorrido da por buena la entrada aunque la página de después traiga otro formulario con contraseña (el alta de empleados de un
panel de administrador): m4142 dio «nadie puede entrar» cuatro veces en una web donde admin/admin123 entraba."""
import sys
import tempfile
import threading
import unittest
import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import recorrido_web as w

LOGIN = ('<h1>Tienda</h1><form method="post"{accion}><input type="text" name="username">'
         '<input type="password" name="password"><button>Entrar</button></form>')
ALTA = ('<h2>Empleados</h2><form method="post" action="/admin/empleados"><input type="text" name="username">'
        '<input type="password" name="password"><input type="text" name="nombre"><button>Crear</button></form>')


def web(modo: str):
    """modo: 'panel_con_alta' (tras entrar, un panel con el alta de empleados), 'accion_otra' (el login envía a /auth y la página de
    fallo trae el mismo formulario) o 'raiz_vacia' (login en «/» con la acción vacía y el fallo lo repite)."""

    class H(BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def responder(self, cuerpo, estado=200, cabeceras=()):
            self.send_response(estado)
            for k, v in cabeceras:
                self.send_header(k, v)
            self.end_headers()
            self.wfile.write(cuerpo.encode())

        def do_GET(self):
            if modo == "raiz_vacia" and self.path == "/":
                return self.responder(LOGIN.format(accion=""))
            if modo == "accion_otra" and self.path == "/login":
                return self.responder(LOGIN.format(accion=' action="/auth"'))
            if self.path == "/login":
                return self.responder(LOGIN.format(accion=' action="/login"'))
            self.responder("no", 404)

        def do_POST(self):
            largo = int(self.headers.get("Content-Length") or 0)
            datos = dict(urllib.parse.parse_qsl(self.rfile.read(largo).decode()))
            bien = datos.get("username") == "admin" and datos.get("password") == "admin123"
            if modo == "panel_con_alta" and self.path == "/login":
                return self.responder("<h1>Panel Administrador</h1>" + ALTA if bien else LOGIN.format(accion=' action="/login"'))
            if modo == "accion_otra" and self.path == "/auth":
                return self.responder("<h1>Hola</h1><p>Bienvenido</p>" if bien else LOGIN.format(accion=' action="/auth"'))
            if modo == "raiz_vacia" and self.path == "/":
                return self.responder("<h1>Hola</h1><p>Bienvenido</p>" if bien else LOGIN.format(accion=""))
            self.responder("no", 404)

    return H


class EntrarConOtroFormulario(unittest.TestCase):
    def entrar(self, modo: str, clave: str, ruta_login: str):
        srv = HTTPServer(("127.0.0.1", 0), web(modo))
        hilo = threading.Thread(target=srv.serve_forever, daemon=True)
        hilo.start()
        self.addCleanup(lambda: (srv.shutdown(), srv.server_close()))
        with tempfile.TemporaryDirectory() as t:
            base = f"http://127.0.0.1:{srv.server_address[1]}/"
            registro = Path(t) / "log.txt"
            registro.write_text("", encoding="utf-8")  # el recorrido lee lo que el servidor escribió ahí
            r = w._Recorrido(base, registro, [("admin", clave)])
            _, final, texto = r.pedir(ruta_login)
            form = next(f for f in r._leer(texto).formularios if w._es_entrar(f))
            r.entrar(r.dentro(final, final) or ruta_login, form)
            return r

    def test_el_alta_de_empleados_del_panel_no_es_la_pantalla_de_entrar(self):
        r = self.entrar("panel_con_alta", "admin123", "/login")
        self.assertEqual(r.usuario, "admin")

    def test_credenciales_malas_siguen_sin_entrar(self):
        r = self.entrar("panel_con_alta", "mala", "/login")
        self.assertIsNone(r.usuario)

    def test_el_fallo_que_repite_el_formulario_con_otra_accion_sigue_sin_entrar(self):
        self.assertIsNone(self.entrar("accion_otra", "mala", "/login").usuario)
        self.assertEqual(self.entrar("accion_otra", "admin123", "/login").usuario, "admin")

    def test_login_en_la_raiz_con_la_accion_vacia(self):
        self.assertIsNone(self.entrar("raiz_vacia", "mala", "/").usuario)
        self.assertEqual(self.entrar("raiz_vacia", "admin123", "/").usuario, "admin")


if __name__ == "__main__":
    unittest.main()
