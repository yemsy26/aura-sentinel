"""La prueba del empleado con roles que no se llaman «empleado» (m4158): la veterinaria del usuario (4.13.8) tenía admin / recepcion / veterinario y la prueba
se saltaba («el campo de rol no tiene una opción de empleado»). Ahora, si ninguna opción se llama empleado, vendedor..., se usa la primera que no sea de jefe."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.recorrido_web import _rol_de_empleado, recorrer


def campo(*opciones) -> dict:
    """opciones: (valor, texto) o (valor, texto, desactivada); valor None = la opción no tiene atributo value."""
    return {"tipo": "select", "name": "rol", "opciones": [{"valor": o[0], "texto": o[1], "desactivada": len(o) > 2 and o[2]} for o in opciones]}


class ElegirElRol(unittest.TestCase):
    def test_un_rol_que_se_llama_empleado_sigue_ganando(self):
        self.assertEqual(_rol_de_empleado(campo(("admin", "Administrador"), ("recepcion", "Recepción"), ("empleado", "Empleado"))), "empleado")
        self.assertEqual(_rol_de_empleado(campo(("a", "Dueño"), ("v", "Vendedor"), ("g", "Gerente"))), "v")

    def test_sin_empleado_se_usa_el_primero_que_no_es_de_jefe(self):
        self.assertEqual(_rol_de_empleado(campo(("admin", "Administrador"), ("recepcion", "Recepción"), ("veterinario", "Veterinario"))), "recepcion")
        self.assertEqual(_rol_de_empleado(campo(("veterinario", "Veterinario"), ("recepcion", "Recepción"), ("admin", "Administrador"))), "veterinario")

    def test_el_elige_un_rol_no_es_un_rol(self):
        self.assertEqual(_rol_de_empleado(campo(("", "-- Seleccione un rol --"), ("admin", "Administrador"), ("medico", "Médico"))), "medico")
        self.assertEqual(_rol_de_empleado(campo((None, "Seleccione un rol"), (None, "Elige..."), ("peluquero", "Peluquero"))), "peluquero")
        self.assertIsNone(_rol_de_empleado(campo((None, "Seleccione un rol"), ("", "--"))))

    def test_solo_roles_de_jefe_o_desactivados_no_valen(self):
        self.assertIsNone(_rol_de_empleado(campo(("admin", "Administrador"), ("owner", "Dueño"), ("gerente", "Gerente"))))
        self.assertEqual(_rol_de_empleado(campo(("cajero", "Cajero", True), ("recepcion", "Recepción"))), "recepcion")
        self.assertIsNone(_rol_de_empleado(campo(("cajero", "Cajero", True))))

    def test_sin_value_se_usa_el_texto(self):
        self.assertEqual(_rol_de_empleado(campo((None, "Administrador"), (None, "Recepción"))), "Recepción")
        self.assertIsNone(_rol_de_empleado(campo()))


WEB = r'''import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer

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
            return self.responder(200, '<form method="post" action="/entrar"><input name="usuario">'
                                       '<input type="password" name="clave"><button>Entrar</button></form>')
        u = self.quien()
        if u is None:
            return self.responder(302, "", [("Location", "/entrar")])
        menu = '<nav><a href="/usuarios">Usuarios</a></nav>'
        if self.path == "/usuarios":
            if USERS[u][1] != "admin":
                return self.responder(302, "", [("Location", "/")])
            lista = "".join(f"<li>{n} ({r})</li>" for n, (_, r) in USERS.items())
            return self.responder(200, "<h1>Usuarios</h1>" + menu + f"<ul>{lista}</ul>"
                                  '<form method="post" action="/usuarios/nuevo"><input name="usuario"><input type="password" name="clave">'
                                  '<select name="rol"><option value="admin">Administrador</option><option value="recepcion">Recepción</option>'
                                  '<option value="veterinario">Veterinario</option></select><button>Crear</button></form>')
        self.responder(200, f"<p>Hola {u}</p>" + menu)

    def do_POST(self):
        datos = dict(urllib.parse.parse_qsl(self.rfile.read(int(self.headers["Content-Length"])).decode()))
        if self.path == "/entrar":
            usuario = datos.get("usuario", "")
            if usuario in USERS and USERS[usuario][0] == datos.get("clave"):
                return self.responder(302, "", [("Location", "/"), ("Set-Cookie", f"sesion={usuario}; Path=/")])
            return self.responder(200, '<form method="post" action="/entrar"><input name="usuario">'
                                       '<input type="password" name="clave"><button>Entrar</button></form><p>no</p>')
        u = self.quien()
        if u is None or USERS[u][1] != "admin":
            return self.responder(403, "Forbidden")
        if self.path == "/usuarios/nuevo":
            USERS[datos.get("usuario", "")] = (datos.get("clave", ""), datos.get("rol", ""))
        self.responder(302, "", [("Location", "/usuarios")])


if __name__ == "__main__":
    HTTPServer(("0.0.0.0", 5000), Web).serve_forever()
'''


class ConRolesDeVeterinaria(unittest.TestCase):
    def test_la_prueba_corre_con_recepcion_y_el_empleado_entra(self):
        carpeta = Path(tempfile.mkdtemp(prefix="test_rol_empleado_"))
        self.addCleanup(shutil.rmtree, carpeta, ignore_errors=True)
        (carpeta / "app.py").write_text(WEB, encoding="utf-8")
        r = recorrer(carpeta / "app.py", sys.executable, [("admin", "pan123")])
        self.assertIsNotNone(r["empleado"], r["empleado_motivo"])
        self.assertTrue(r["empleado"]["entro"])
        self.assertIn("/usuarios", r["empleado"]["cerradas"])  # la sección de usuarios sigue cerrada a ese rol


if __name__ == "__main__":
    unittest.main()
