"""El empleado que la prueba crea tiene que poder entrar (m4155): en cuatro entregas el formulario de usuarios solo tenía `nombre` («Nombre de
usuario»), `password` y `rol`, y el login usaba ese mismo `nombre`. El recorrido lo daba de alta como «Empleado Aura» y luego intentaba
entrar como «empleado_aura»: no entraba y la prueba de lo que ve el empleado no corría."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.recorrido_web import recorrer

WEB = r'''import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer

VARIANTE = "__VARIANTE__"  # "nombre": solo nombre; "correo": nombre + email (se entra con el correo); "usuario": usuario + nombre (nombre completo)
CAMPO_ENTRAR = {"nombre": "nombre", "correo": "email", "usuario": "usuario"}[VARIANTE]
ALTA = {
    "nombre": '<input type="text" name="nombre"><input type="password" name="password">',
    "correo": '<input type="text" name="nombre"><input type="email" name="email"><input type="password" name="password">',
    "usuario": '<input type="text" name="usuario"><input type="text" name="nombre"><input type="password" name="password">',
}[VARIANTE]
USERS = {"admin": ("pan123", "admin")}


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
        if self.path == "/entrar":
            return self.responder(200, f'<form method="post" action="/entrar"><input name="{CAMPO_ENTRAR}">'
                                       '<input type="password" name="password"><button>Entrar</button></form>')
        u = self.quien()
        if u is None:
            return self.responder(302, "", [("Location", "/entrar")])
        menu = '<nav><a href="/usuarios">Usuarios</a></nav>'
        if self.path == "/usuarios":
            if USERS[u][1] != "admin":
                return self.responder(302, "", [("Location", "/")])
            lista = "".join(f"<li>{n}</li>" for n in USERS)
            return self.responder(200, "<h1>Usuarios</h1>" + menu + f"<ul>{lista}</ul>"
                                  f'<form method="post" action="/usuarios/nuevo">{ALTA}<select name="rol"><option value="admin">Administrador</option>'
                                  '<option value="empleado">Empleado</option></select><button>Crear</button></form>')
        self.responder(200, f"<p>Hola {u}</p>" + menu)

    def do_POST(self):
        datos = dict(urllib.parse.parse_qsl(self.rfile.read(int(self.headers["Content-Length"])).decode()))
        if self.path == "/entrar":
            quien = datos.get(CAMPO_ENTRAR, "")
            if quien in USERS and USERS[quien][0] == datos.get("password"):
                return self.responder(302, "", [("Location", "/"), ("Set-Cookie", f"sesion={quien}; Path=/")])
            return self.responder(200, f'<form method="post" action="/entrar"><input name="{CAMPO_ENTRAR}">'
                                       '<input type="password" name="password"><button>Entrar</button></form><p>no</p>')
        u = self.quien()
        if u is None or USERS[u][1] != "admin":
            return self.responder(403, "Forbidden")
        if self.path == "/usuarios/nuevo":
            USERS[datos.get(CAMPO_ENTRAR, "")] = (datos.get("password", ""), datos.get("rol", "empleado"))
        self.responder(302, "", [("Location", "/usuarios")])


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 5000), Web).serve_forever()
'''


class EmpleadoQueEntra(unittest.TestCase):
    def empleado(self, variante: str) -> dict:
        carpeta = Path(tempfile.mkdtemp(prefix="test_empleado_nombre_"))
        self.addCleanup(shutil.rmtree, carpeta, ignore_errors=True)
        (carpeta / "app.py").write_text(WEB.replace("__VARIANTE__", variante), encoding="utf-8")
        r = recorrer(carpeta / "app.py", sys.executable, [("admin", "pan123")])
        self.assertIsNotNone(r["empleado"], r["empleado_motivo"])
        return r["empleado"]

    def test_con_solo_nombre_ese_campo_es_el_usuario_con_el_que_se_entra(self):
        e = self.empleado("nombre")
        self.assertEqual((e["usuario"], e["entro"]), ("empleado_aura", True))

    def test_si_se_entra_con_el_correo_nombre_sigue_siendo_el_nombre_completo(self):
        self.assertTrue(self.empleado("correo")["entro"])

    def test_con_un_campo_de_usuario_aparte_nombre_sigue_siendo_el_nombre_completo(self):
        self.assertTrue(self.empleado("usuario")["entro"])


if __name__ == "__main__":
    unittest.main()
