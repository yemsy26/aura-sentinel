"""H17: una web con servidor que carga Bootstrap, jQuery o una tipografía de internet (ferretería 22) da un aviso: sin red pierde el estilo."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from agente import verificacion
from agente.recorrido_web import recursos_externos

BOOTSTRAP = ('<link href="https://cdn.jsdelivr.net/npm/bootstrap@5.3.3/dist/css/bootstrap.min.css" rel="stylesheet">\n'
             '<script src="https://cdn.jsdelivr.net/npm/bootstrap@5.3.3/dist/js/bootstrap.bundle.min.js"></script>\n')


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_externos_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)

    def escribe(self, rel: str, texto: str) -> None:
        f = self.dir / rel
        f.parent.mkdir(parents=True, exist_ok=True)
        f.write_text(texto, encoding="utf-8")

    def externos(self) -> list[dict]:
        return recursos_externos(self.dir, self.dir)


class LoQueCuenta(Base):
    def test_bootstrap_de_un_cdn_en_la_plantilla_base(self):
        self.escribe("templates/base.html", "<head>" + BOOTSTRAP + "</head>")
        self.assertEqual(self.externos(), [
            {"archivo": "templates/base.html", "host": "cdn.jsdelivr.net", "recurso": "bootstrap.min.css"},
            {"archivo": "templates/base.html", "host": "cdn.jsdelivr.net", "recurso": "bootstrap.bundle.min.js"}])

    def test_sin_protocolo_tailwind_sin_ruta_y_jquery(self):
        self.escribe("templates/a.html", '<script src="//code.jquery.com/jquery-3.7.1.min.js"></script>'
                                         '<script src="https://cdn.tailwindcss.com"></script>')
        self.assertEqual([(e["host"], e["recurso"]) for e in self.externos()],
                         [("code.jquery.com", "jquery-3.7.1.min.js"), ("cdn.tailwindcss.com", "cdn.tailwindcss.com")])

    def test_un_import_de_css_con_una_tipografia(self):
        self.escribe("static/css/estilo.css", "@import url('https://fonts.googleapis.com/css2?family=Inter');\nbody{margin:0}")
        self.assertEqual([(e["host"], e["archivo"]) for e in self.externos()], [("fonts.googleapis.com", "static/css/estilo.css")])

    def test_el_mismo_recurso_dos_veces_en_un_archivo_cuenta_una_vez_y_en_dos_archivos_dos(self):
        self.escribe("templates/base.html", BOOTSTRAP)
        self.escribe("templates/login.html", BOOTSTRAP.splitlines()[0])
        self.escribe("templates/otra.html", BOOTSTRAP.splitlines()[0] + BOOTSTRAP.splitlines()[0])
        self.assertEqual(len(self.externos()), 4)


class LoQueNoCuenta(Base):
    def test_iconos_enlaces_imagenes_y_conexiones_previas(self):
        self.escribe("templates/base.html", (
            '<link rel="icon" href="https://example.com/favicon.ico">'
            '<link rel="canonical" href="https://example.com/">'
            '<link rel="preconnect" href="https://fonts.gstatic.com">'
            '<link rel="dns-prefetch" href="https://cdn.jsdelivr.net">'
            '<a href="https://github.com/x">código</a><img src="https://example.com/logo.png">'))
        self.assertEqual(self.externos(), [])

    def test_comentarios_locales_y_rutas_propias(self):
        self.escribe("templates/base.html", (
            "<!-- <script src=\"https://cdn.jsdelivr.net/x.js\"></script> -->{# <link href=\"https://cdn.jsdelivr.net/y.css\"> #}"
            '<script src="http://localhost:5000/x.js"></script><script src="http://127.0.0.1:8000/y.js"></script>'
            '<link href="/static/css/estilo.css" rel="stylesheet"><link href="static/a.css" rel="stylesheet">'
            "<script src=\"{{ url_for('static', filename='js/app.js') }}\"></script><script>var x = 'https://cdn.jsdelivr.net/z.js';</script>"))
        self.assertEqual(self.externos(), [])

    def test_tests_dependencias_y_entornos(self):
        for rel in ("tests/pagina.html", "test/pagina.html", "node_modules/x/index.html", ".venv/Lib/site-packages/x/index.html"):
            self.escribe(rel, BOOTSTRAP)
        self.assertEqual(self.externos(), [])

    def test_si_el_readme_dice_que_necesita_internet_se_calla(self):
        self.escribe("templates/base.html", BOOTSTRAP)
        self.escribe("README.md", "# Ferretería\nEl sistema necesita conexión a internet para cargar el diseño.\n")
        self.assertEqual(self.externos(), [])
        self.escribe("README.md", "# Ferretería\nRequiere internet.\n")
        self.assertEqual(self.externos(), [])

    def test_un_readme_que_dice_que_funciona_sin_internet_no_calla_nada(self):
        self.escribe("templates/base.html", BOOTSTRAP)
        self.escribe("README.md", "# Ferretería\nFunciona sin internet, con un doble clic.\n")
        self.assertEqual(len(self.externos()), 2)

    def test_un_readme_dentro_de_una_carpeta_no_cuenta(self):
        self.escribe("templates/base.html", BOOTSTRAP)
        self.escribe("docs/README.md", "Necesita internet.\n")
        self.assertEqual(len(self.externos()), 2)

    def test_proyecto_sin_plantillas_ni_css(self):
        self.escribe("app.py", "print('hola')\n")
        self.assertEqual(self.externos(), [])


class MensajeDeVerificar(Base):
    def informe(self) -> verificacion.Informe:
        (self.dir / "app.py").write_text("print('x')\n", encoding="utf-8")
        falso = {"errores": [], "plantillas": [], "rotos": [], "raros": [], "enviados": [], "paginas": 3, "formularios": 2,
                 "usuario": "admin", "sin_entrar": False, "entrar_js": False, "navegador": None, "lineas": None,
                 "sin_formulario": [], "registro": False, "entrar_get": False, "sin_permiso": [], "login_vacio": None,
                 "empleado": None}
        inf = verificacion.Informe()
        with mock.patch.object(verificacion, "recorrer", return_value=falso):
            verificacion.probar_web(self.dir, self.dir / "app.py", sys.executable, inf)
        return inf

    def test_es_un_aviso_y_no_un_error(self):
        self.escribe("templates/base.html", BOOTSTRAP)
        inf = self.informe()
        self.assertEqual(inf.errores, [])
        [m] = [a for a in inf.avisos if "de internet" in a]
        self.assertIn("2 archivos se cargan de internet (cdn.jsdelivr.net)", m)
        self.assertIn("bootstrap.min.css (templates/base.html), bootstrap.bundle.min.js (templates/base.html)", m)
        self.assertIn("doble clic", m)
        self.assertIn("url_for('static', filename=...)", m)
        self.assertIn("ignora este aviso", m)

    def test_uno_solo_y_muchos(self):
        self.escribe("templates/base.html", BOOTSTRAP.splitlines()[0])
        [m] = [a for a in self.informe().avisos if "de internet" in a]
        self.assertTrue(m.startswith("1 archivo se carga de internet"))
        for i in range(6):
            self.escribe(f"templates/p{i}.html", f'<script src="https://cdn.jsdelivr.net/npm/lib{i}.js"></script>')
        [m] = [a for a in self.informe().avisos if "de internet" in a]
        self.assertTrue(m.startswith("7 archivos se cargan de internet"))
        self.assertTrue(m.split(". Un sistema")[0].endswith(" y más"))

    def test_sin_recursos_externos_no_dice_nada(self):
        self.escribe("templates/base.html", '<link href="/static/css/estilo.css" rel="stylesheet">')
        self.assertEqual([a for a in self.informe().avisos if "de internet" in a], [])


if __name__ == "__main__":
    unittest.main()
