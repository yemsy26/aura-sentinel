"""verificar avisa de un `url_for('nombre')` en una plantilla que no corresponde a ninguna ruta (u4138_vet, la veterinaria del usuario con la 4.13.8:
`url_for('nueva_vacuna')` cuando la ruta se llama `vacuna_nueva`; los botones iban dentro de un `{% if can_edit %}` que nunca se cumplía, la página daba 200
y no había forma de registrar una vacuna). Es estático y solo habla si sabe cuáles son las rutas."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import verificacion
from agente.recorrido_web import rutas_de_la_web, url_for_sin_ruta

APP = '''from flask import Flask, Blueprint, render_template
app = Flask(__name__)
bp = Blueprint("main", __name__)

@app.route("/vacunas/nueva/<int:paciente_id>")
def vacuna_nueva(paciente_id):
    return "x"

@app.route("/pacientes/<int:id>/editar", endpoint="paciente_editar_ep")
def paciente_editar(id):
    return "x"

@bp.route("/ventas/<int:id>")
def ver_venta(id):
    return "x"
'''


class Hallazgos(unittest.TestCase):
    def setUp(self):
        self.raiz = Path(tempfile.mkdtemp(prefix="test_url_for_"))
        self.addCleanup(shutil.rmtree, self.raiz, ignore_errors=True)

    def web(self, plantilla: str | None = None, app: str = APP, extra: dict | None = None) -> list[dict]:
        (self.raiz / "app.py").write_text(app, encoding="utf-8")
        (self.raiz / "templates").mkdir(exist_ok=True)
        if plantilla is not None:
            (self.raiz / "templates" / "p.html").write_text(plantilla, encoding="utf-8")
        for ruta, texto in (extra or {}).items():
            destino = self.raiz / ruta
            destino.parent.mkdir(parents=True, exist_ok=True)
            destino.write_text(texto, encoding="utf-8")
        return url_for_sin_ruta(self.raiz)

    def test_el_nombre_que_no_existe_dice_donde_y_que_ruta_se_parece(self):
        [h] = self.web("<h1>Ficha</h1>\n<a href=\"{{ url_for('nueva_vacuna', paciente_id=1) }}\">Vacuna</a>")
        self.assertEqual((h["plantilla"], h["linea"], h["endpoint"], h["sugerencia"]), ("templates/p.html", 2, "nueva_vacuna", "vacuna_nueva"))

    def test_los_nombres_que_existen_no_cuentan(self):
        html = ("{{ url_for('vacuna_nueva', paciente_id=1) }} {{ url_for(\"paciente_editar_ep\", id=1) }} "
                "{{ url_for('main.ver_venta', id=1) }} {{ url_for('static', filename='a.css') }}")
        self.assertEqual(self.web(html), [])

    def test_con_endpoint_explicito_el_nombre_de_la_funcion_ya_no_vale(self):
        [h] = self.web("{{ url_for('paciente_editar', id=1) }}")  # Flask solo conoce «paciente_editar_ep»
        self.assertEqual((h["endpoint"], h["sugerencia"]), ("paciente_editar", "paciente_editar_ep"))

    def test_un_nombre_de_blueprint_solo_mira_el_ultimo_trozo(self):
        self.assertEqual(self.web("{{ url_for('main.vacuna_nueva', paciente_id=1) }}"), [])  # prudente: puede estar en otro archivo
        [h] = self.web("{{ url_for('main.venta_detalle', id=1) }}")
        self.assertEqual(h["endpoint"], "main.venta_detalle")

    def test_url_for_con_variable_o_relativo_no_cuenta(self):
        self.assertEqual(self.web("{{ url_for(destino) }} {{ url_for('.otra') }} {{ url_for('x' ~ n) }}"), [])

    def test_comentarios_de_jinja_y_raw_no_cuentan_pero_los_html_si(self):
        self.assertEqual(self.web("{# {{ url_for('no_existe') }} #}{% raw %}{{ url_for('no_existe') }}{% endraw %}"), [])
        self.assertEqual(len(self.web("<!-- {{ url_for('no_existe') }} -->")), 1)  # Jinja lo ejecuta aunque el navegador lo oculte

    def test_el_mismo_nombre_dos_veces_en_una_plantilla_cuenta_una(self):
        self.assertEqual(len(self.web("{{ url_for('x_no') }}\n{{ url_for('x_no') }}")), 1)

    def test_add_url_rule_y_as_view(self):
        app = ('from flask import Flask\nfrom flask.views import MethodView\napp = Flask(__name__)\ndef inicio(): return "x"\n'
               'class V(MethodView):\n    def get(self): return "x"\n'
               'app.add_url_rule("/", "portada", inicio)\napp.add_url_rule("/v", view_func=V.as_view("vista_v"))\n'
               'app.add_url_rule("/i", view_func=inicio)\n')
        self.assertEqual(self.web("{{ url_for('portada') }} {{ url_for('vista_v') }} {{ url_for('inicio') }}", app), [])
        self.assertEqual(len(self.web("{{ url_for('otra_cosa') }}", app)), 1)

    def test_rutas_registradas_en_un_bucle_o_por_una_extension_callan(self):
        bucle = ('from flask import Flask\napp = Flask(__name__)\n@app.route("/")\ndef a(): return "x"\n'
                 'for ruta, func in [("/b", a)]:\n    app.add_url_rule(ruta, view_func=func)\n')
        self.assertEqual(self.web("{{ url_for('no_existe') }}", bucle), [])
        ext = 'from flask import Flask\nfrom flask_restful import Api\napp = Flask(__name__)\n@app.route("/")\ndef a(): return "x"\n'
        self.assertEqual(self.web("{{ url_for('no_existe') }}", ext), [])
        dinamico = 'from flask import Flask\napp = Flask(__name__)\n@app.route("/")\ndef a(): return "x"\napp.view_functions["z"] = a\n'
        self.assertEqual(self.web("{{ url_for('no_existe') }}", dinamico), [])

    def test_sin_flask_o_sin_rutas_no_dice_nada(self):
        self.assertEqual(self.web("{{ url_for('no_existe') }}", "print('hola')\n"), [])
        self.assertIsNone(rutas_de_la_web(self.raiz))
        self.assertEqual(self.web("{{ url_for('no_existe') }}", "from flask import Flask\napp = Flask(__name__)\n"), [])

    def test_no_lee_el_entorno_virtual(self):
        solo_en_el_entorno = {".venv/Lib/site-packages/otra.py": APP, ".venv/templates/q.html": "{{ url_for('x_no') }}"}
        self.assertEqual(self.web("{{ url_for('vacuna_nueva', paciente_id=1) }}", "print('hola')\n", solo_en_el_entorno), [])
        self.assertEqual(self.web(None, APP, solo_en_el_entorno), [])

    def test_plantillas_en_una_subcarpeta_de_paquete(self):
        self.assertEqual(len(self.web(None, APP, {"app/templates/q.html": "{{ url_for('x_no') }}"})), 1)


class MensajeDeVerificar(unittest.TestCase):
    def informe(self, html: str) -> "verificacion.Informe":
        raiz = Path(tempfile.mkdtemp(prefix="test_url_for_msg_"))
        self.addCleanup(shutil.rmtree, raiz, ignore_errors=True)
        (raiz / "app.py").write_text(APP, encoding="utf-8")
        (raiz / "templates").mkdir()
        (raiz / "templates" / "paciente_detalle.html").write_text(html, encoding="utf-8")
        falso = {"errores": [], "plantillas": [], "rotos": [], "raros": [], "enviados": [], "paginas": 3, "formularios": 2,
                 "usuario": "admin", "sin_entrar": False, "entrar_js": False, "navegador": None, "lineas": None,
                 "sin_formulario": [], "registro": False, "entrar_get": False, "sin_permiso": [], "login_vacio": None,
                 "empleado": None, "negocio": None, "venta": None}
        inf = verificacion.Informe()
        with mock.patch.object(verificacion, "recorrer", return_value=falso):
            verificacion.probar_web(raiz, raiz / "app.py", sys.executable, inf)
        return inf

    def test_es_un_error_y_dice_donde_esta_y_que_ruta_quisiste(self):
        inf = self.informe("{% if can_edit %}\n<a href=\"{{ url_for('nueva_vacuna', paciente_id=1) }}\">Vacuna</a>\n{% endif %}")
        [m] = inf.errores
        self.assertIn("la plantilla templates/paciente_detalle.html (línea 2) usa url_for('nueva_vacuna')", m)
        self.assertIn("¿quisiste decir «vacuna_nueva»?", m)
        self.assertIn("BuildError", m)
        self.assertIn("«if»", m)

    def test_nada_si_las_rutas_existen(self):
        inf = self.informe("<a href=\"{{ url_for('vacuna_nueva', paciente_id=1) }}\">Vacuna</a>")
        self.assertEqual((inf.errores, inf.avisos), ([], []))


if __name__ == "__main__":
    unittest.main()
