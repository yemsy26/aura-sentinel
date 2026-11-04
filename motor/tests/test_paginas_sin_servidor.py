"""Páginas probadas sin su servidor: verificar abre los .html como archivos sueltos, así que lo que solo da el servidor
(una ruta /api/..., un {{ url_for }} de una plantilla) fallaba y salía como error. Las misiones perseguían esos errores
falsos hasta quedarse sin pasos (en 23 sistemas guardados, 31 errores así y ninguno de verdad). También los atributos
que lee una librería (conn.row_factory, app.secret_key...) salían como «se define pero NUNCA se usa»."""
import importlib.util
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.proyecto import Informe
from agente.verificacion import codigo_muerto, del_servidor, es_plantilla, rutas_servidor, verificar

HAY_NAVEGADOR = importlib.util.find_spec("playwright") is not None
APP = ('class App:\n    def route(self, ruta, methods=None):\n        return lambda f: f\n\n\napp = App()\n\n\n'
       '@app.route("/api/me")\ndef yo():\n    return "{}"\n\n\n@app.route("/api/productos/<int:pid>")\ndef producto(pid):\n'
       '    return str(pid)\n')


class PaginasSinServidor(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_sin_servidor_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)

    def escribir(self, ruta: str, texto: str) -> Path:
        p = self.dir / ruta
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(texto, encoding="utf-8")
        return p

    def test_rutas_que_define_el_servidor(self):
        self.escribir("app.py", APP)
        self.escribir("api.py", 'router = APIRouter(prefix="/v1")\n\n\n@router.get("/ventas/{vid}")\ndef venta(vid):\n    pass\n')
        self.escribir("viejo.py", 'class H:\n    def do_GET(self):\n        if self.path == "/api/resumen":\n            pass\n'
                                  '        elif self.path.startswith("/api/mov/"):\n            pass\n')
        rutas = rutas_servidor(self.dir)
        for ruta in ("api/me", "/api/me", "api/productos/3", "v1/ventas/7", "api/resumen", "api/mov/12", "api/me?x=1",
                     "templates/%7B%7B%20url_for('static',%20filename='app.js')%20%7D%7D"):
            with self.subTest(ruta=ruta):
                self.assertTrue(del_servidor(ruta, rutas))
        for ruta in ("api/noexiste", "static/css/style.css", "api/productos/3/extra"):
            with self.subTest(ruta=ruta):
                self.assertFalse(del_servidor(ruta, rutas))

    def test_plantillas_de_servidor(self):
        self.assertTrue(es_plantilla(self.escribir("templates/base.html", "<html></html>"), self.dir))
        self.assertTrue(es_plantilla(self.escribir("paginas/lista.html", "{% for x in xs %}{{ x }}{% endfor %}"), self.dir))
        self.assertFalse(es_plantilla(self.escribir("static/index.html", "<html><body>hola</body></html>"), self.dir))

    def test_atributos_que_lee_la_libreria_no_son_codigo_sin_usar(self):
        self.escribir("app.py", "import sqlite3\n\n\nclass Servidor:\n    pass\n\n\nconn = sqlite3.connect(':memory:')\n"
                                "conn.row_factory = sqlite3.Row\nservidor = Servidor()\nservidor.secret_key = 'x'\n"
                                "servidor.login_view = 'entrar'\nservidor.color_favorito = 'azul'\n")
        inf = Informe()
        codigo_muerto(self.dir, [self.dir / "app.py"], inf)
        texto = "\n".join(inf.avisos + inf.errores)
        for nombre in ("row_factory", "secret_key", "login_view"):
            self.assertNotIn(f"'{nombre}'", texto)
        self.assertIn("'color_favorito'", texto)  # un atributo propio sin usar se sigue avisando

    @unittest.skipUnless(HAY_NAVEGADOR, "sin playwright")
    def test_la_pagina_pide_rutas_del_servidor_y_una_que_no_existe(self):
        self.escribir("app.py", APP)
        self.escribir("static/index.html", '<!doctype html><html><body><h1>Tienda</h1><script>'
                                           'fetch("/api/me"); fetch("/api/noexiste");</script></body></html>')
        self.escribir("templates/base.html", '<!doctype html><html><head><link rel="stylesheet" '
                                             "href=\"{{ url_for('static', filename='s.css') }}\"></head><body></body></html>")
        informe, _ = verificar(self.dir, sys.executable)
        errores = informe.split("AVISOS (revisa")[0]  # la cabecera también dice «N AVISOS»
        self.assertNotIn("api/me", errores)  # la sirve el servidor
        self.assertNotIn("url_for", errores)  # plantilla: no se abre como archivo
        self.assertIn("HTTP 404: api/noexiste", errores)  # no la define nadie: error de verdad
        self.assertIn("sin tu servidor, no pudo pedir api/me", informe)


if __name__ == "__main__":
    unittest.main()
