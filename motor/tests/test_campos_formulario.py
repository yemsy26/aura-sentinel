"""verificar avisa de un campo que la ruta exige con `request.form["x"]` y que el formulario que envía a esa ruta no tiene (u4138_vet, la veterinaria del
usuario con la 4.13.8: /usuarios/nuevo lee `nombre_completo` y el formulario no lo trae, y /vacunas/nueva lee `nombre` y `fecha_aplicacion` cuando el
formulario tiene `vacuna_id` y `fecha`: Flask da 400 en cada envío y esas acciones no se podían hacer desde la pantalla). Es estático y solo habla si
está seguro."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import verificacion
from agente.recorrido_web import campos_sin_formulario

APP = '''from flask import Flask, render_template, request, redirect
app = Flask(__name__)

@app.route("/")
def inicio():
    return "x"

@app.route("/usuarios/nuevo", methods=["GET", "POST"])
def usuario_nuevo():
    if request.method == "POST":
        usuario = request.form["username"]
        nombre = request.form["nombre_completo"]
        return redirect("/")
    return render_template("form_usuario.html")
'''
FORM = '<h1>Nuevo</h1>\n<form method="post">\n  <input name="username">\n</form>'
FORM_COMPLETO = '<form method="post"><input name="username"><input type="text" name=\'nombre_completo\'></form>'


class Hallazgos(unittest.TestCase):
    def setUp(self):
        self.raiz = Path(tempfile.mkdtemp(prefix="test_campos_form_"))
        self.addCleanup(shutil.rmtree, self.raiz, ignore_errors=True)

    def web(self, app: str = APP, plantillas: dict | None = None, extra: dict | None = None) -> list[dict]:
        (self.raiz / "app.py").write_text(app, encoding="utf-8")
        for nombre, texto in {"form_usuario.html": FORM, **(plantillas or {})}.items():
            destino = self.raiz / "templates" / nombre
            destino.parent.mkdir(parents=True, exist_ok=True)
            destino.write_text(texto, encoding="utf-8")
        for ruta, texto in (extra or {}).items():
            destino = self.raiz / ruta
            destino.parent.mkdir(parents=True, exist_ok=True)
            destino.write_text(texto, encoding="utf-8")
        return campos_sin_formulario(self.raiz)

    def test_el_campo_que_la_ruta_exige_y_el_formulario_no_trae(self):
        [h] = self.web()
        self.assertEqual((h["plantilla"], h["linea"], h["ruta"], h["funcion"], h["campo"]),
                         ("templates/form_usuario.html", 2, "/usuarios/nuevo", "usuario_nuevo", "nombre_completo"))

    def test_con_el_campo_no_avisa(self):
        self.assertEqual(self.web(plantillas={"form_usuario.html": FORM_COMPLETO}), [])
        self.assertEqual(self.web(plantillas={"form_usuario.html": '<form method=post><input name=username><textarea name=nombre_completo></textarea></form>'}), [])

    def test_name_parecido_no_cuenta(self):
        [h] = self.web(plantillas={"form_usuario.html": '<form method="post"><input name="username"><input name="nombre_completo2"></form>'})
        self.assertEqual(h["campo"], "nombre_completo")

    def test_un_get_no_exige_ni_un_formulario_get_envia(self):
        self.assertEqual(self.web(plantillas={"form_usuario.html": '<form method="get"><input name="username"></form>'}), [])
        self.assertEqual(self.web(plantillas={"form_usuario.html": '<form action="/buscar"><input name="username"></form>'}), [])

    def test_solo_cuenta_lo_que_se_lee_siempre(self):
        base = 'from flask import Flask, render_template, request\napp = Flask(__name__)\n@app.route("/x", methods=["GET", "POST"])\ndef usuario_nuevo():\n%s\n    return render_template("form_usuario.html")\n'
        for cuerpo in ('    if request.method == "POST":\n        a = request.form.get("nombre_completo")',
                       '    if request.method == "POST":\n        if hay:\n            a = request.form["nombre_completo"]',
                       '    if request.method == "POST":\n        for i in x:\n            a = request.form["nombre_completo"]',
                       '    if request.method == "POST":\n        try:\n            a = request.form["nombre_completo"]\n        except KeyError:\n            a = 1',
                       '    if request.method == "POST":\n        a = request.form["nombre_completo"] if hay else ""',
                       '    if request.method == "POST":\n        a = hay and request.form["nombre_completo"]',
                       '    if request.method == "POST":\n        if "nombre_completo" in request.form:\n            pass\n        a = request.form["nombre_completo"]',
                       '    if request.method == "POST" and hay:\n        a = request.form["nombre_completo"]',
                       '    if request.method == "GET":\n        a = request.form["nombre_completo"]'):
            with self.subTest(cuerpo=cuerpo):
                self.assertEqual(self.web(base % cuerpo), [])

    def test_lo_que_se_lee_siempre_cuenta(self):
        base = 'from flask import Flask, render_template, request\napp = Flask(__name__)\n@app.route("/x", methods=["GET", "POST"])\ndef usuario_nuevo():\n%s\n    return render_template("form_usuario.html")\n'
        for cuerpo in ('    a = request.form["nombre_completo"]',
                       '    if request.method == "POST":\n        a = int(request.form["nombre_completo"].strip())',
                       "    if request.method == 'POST':\n        with ctx():\n            a = request.form['nombre_completo']"):
            with self.subTest(cuerpo=cuerpo):
                self.assertEqual([h["campo"] for h in self.web(base % cuerpo)], ["nombre_completo"])

    def test_una_ruta_que_no_acepta_post_no_cuenta(self):
        self.assertEqual(self.web(APP.replace('methods=["GET", "POST"]', "")), [])
        self.assertEqual(self.web(APP.replace('methods=["GET", "POST"]', 'methods=["GET"]')), [])
        self.assertEqual(len(self.web(APP.replace('@app.route("/usuarios/nuevo", methods=["GET", "POST"])', '@app.post("/usuarios/nuevo")'))), 1)

    def test_action_con_url_for_al_endpoint(self):
        app = APP.replace('return render_template("form_usuario.html")', 'return "x"')
        [h] = self.web(app, {"otra.html": '<form method="post" action="{{ url_for(\'usuario_nuevo\') }}"><input name="username"></form>'})
        self.assertEqual((h["plantilla"], h["campo"]), ("templates/otra.html", "nombre_completo"))
        self.assertEqual(self.web(app, {"otra.html": '<form method="post" action="{{ url_for(\'inicio\') }}"><input name="username"></form>'}), [])
        self.assertEqual(self.web(app, {"otra.html": '<form method="post" action="{{ url_for(\'no_se\') }}"><input name="username"></form>'}), [])

    def test_un_nombre_de_endpoint_repetido_en_otro_archivo_calla(self):
        app = APP.replace('return render_template("form_usuario.html")', 'return "x"')
        otro = 'from flask import Blueprint\nbp = Blueprint("b", __name__)\n@bp.route("/otro")\ndef usuario_nuevo():\n    return "y"\n'
        self.assertEqual(self.web(app, {"otra.html": '<form method="post" action="{{ url_for(\'usuario_nuevo\') }}"></form>'}, {"b.py": otro}), [])

    def test_action_literal_con_convertidor(self):
        app = ('from flask import Flask, request\napp = Flask(__name__)\n@app.route("/vacunas/nueva/<int:paciente_id>", methods=["GET", "POST"])\n'
               'def vacuna_nueva(paciente_id):\n    nombre = request.form["nombre"]\n    return "x"\n')
        [h] = self.web(app, {"v.html": '<form method="post" action="/vacunas/nueva/{{ p.id }}"><input name="fecha"></form>'})
        self.assertEqual((h["ruta"], h["campo"]), ("/vacunas/nueva/<int:paciente_id>", "nombre"))
        self.assertEqual(self.web(app, {"v.html": '<form method="post" action="/vacunas/nueva/{{ p.id }}"><input name="nombre"></form>'}), [])
        self.assertEqual(self.web(app, {"v.html": '<form method="post" action="/otra/{{ p.id }}"><input name="fecha"></form>'}), [])
        self.assertEqual(self.web(app, {"v.html": '<form method="post" action="{{ destino }}"><input name="fecha"></form>'}), [])

    def test_los_campos_pueden_venir_de_un_include_un_import_o_el_padre(self):
        self.assertEqual(self.web(plantillas={"form_usuario.html": '<form method="post"><input name="username">{% include "_campos.html" %}</form>',
                                              "_campos.html": '<input name="nombre_completo">'}), [])
        self.assertEqual(self.web(plantillas={"form_usuario.html": '{% extends "base.html" %}<form method="post"><input name="username"></form>',
                                              "base.html": '<input name="nombre_completo">{% block c %}{% endblock %}'}), [])
        self.assertEqual(self.web(plantillas={"form_usuario.html": '{% from "m.html" import campo %}<form method="post"><input name="username">'
                                                                   '{{ campo("nombre_completo") }}</form>', "m.html": "{% macro campo(n) %}{% endmacro %}"}), [])

    def test_un_include_que_no_se_puede_seguir_calla(self):
        for texto in ('<form method="post"><input name="username">{% include "no_existe.html" %}</form>',
                      '<form method="post"><input name="username">{% include campos %}</form>',
                      '<form method="post"><input name="username">{% include "a.html" if x else "b.html" %}</form>'):
            with self.subTest(texto=texto):
                self.assertEqual(self.web(plantillas={"form_usuario.html": texto, "a.html": "", "b.html": ""}), [])

    def test_javascript_o_nombres_dinamicos_callan(self):
        for texto in ('<form method="post"><input name="username"></form><script>new FormData(f)</script>',
                      '<form method="post"><input name="username"></form><script>f.addEventListener("submit", e => e.preventDefault())</script>',
                      '<form method="post"><input name="username"></form><script>const i = document.createElement("input"); i.name = x</script>',
                      '<form method="post"><input name="username">{% for c in campos %}<input name="{{ c }}">{% endfor %}</form>',
                      '<form method="post"><input name="username"><input name="pre_{{ c }}"></form>'):
            with self.subTest(texto=texto):
                self.assertEqual(self.web(plantillas={"form_usuario.html": texto}), [])

    def test_el_nombre_entre_comillas_en_la_plantilla_cuenta_como_campo(self):
        self.assertEqual(self.web(plantillas={"form_usuario.html": '<form method="post"><input name="username">{{ campo(\'nombre_completo\') }}</form>'}), [])
        [h] = self.web(plantillas={"form_usuario.html": '{{ current_user.nombre_completo }}<form method="post"><input name="username"></form>'})
        self.assertEqual(h["campo"], "nombre_completo")  # aparecer como atributo de un objeto no es un campo
        self.assertEqual(len(self.web(plantillas={"form_usuario.html": '{# <input name="nombre_completo"> #}<form method="post"><input name="username"></form>'})), 1)

    def test_flask_wtf_y_las_webs_que_no_son_claras_callan(self):
        self.assertEqual(self.web("from flask_wtf import FlaskForm\n" + APP), [])
        self.assertEqual(self.web("from flask_restful import Api\n" + APP), [])
        self.assertEqual(self.web("print('hola')\n"), [])

    def test_plantillas_del_entorno_virtual_no_cuentan(self):
        self.assertEqual(self.web(plantillas={"form_usuario.html": FORM_COMPLETO}, extra={".venv/templates/x.html": FORM}), [])

    def test_el_mismo_hallazgo_se_da_una_vez(self):
        [h1] = self.web(plantillas={"form_usuario.html": FORM + "\n" + FORM})
        self.assertEqual(h1["campo"], "nombre_completo")


class MensajeDeVerificar(unittest.TestCase):
    def informe(self, app: str = APP) -> "verificacion.Informe":
        raiz = Path(tempfile.mkdtemp(prefix="test_campos_msg_"))
        self.addCleanup(shutil.rmtree, raiz, ignore_errors=True)
        (raiz / "app.py").write_text(app, encoding="utf-8")
        (raiz / "templates").mkdir()
        (raiz / "templates" / "form_usuario.html").write_text(FORM, encoding="utf-8")
        falso = {"errores": [], "plantillas": [], "rotos": [], "raros": [], "enviados": [], "paginas": 3, "formularios": 2,
                 "usuario": "admin", "sin_entrar": False, "entrar_js": False, "navegador": None, "lineas": None,
                 "sin_formulario": [], "registro": False, "entrar_get": False, "sin_permiso": [], "login_vacio": None,
                 "empleado": None, "negocio": None, "venta": None}
        inf = verificacion.Informe()
        with mock.patch.object(verificacion, "recorrer", return_value=falso):
            verificacion.probar_web(raiz, raiz / "app.py", sys.executable, inf)
        return inf

    def test_es_un_error_y_dice_donde_esta_y_que_hacer(self):
        [m] = self.informe().errores
        self.assertIn("el formulario de templates/form_usuario.html (línea 2) envía a /usuarios/nuevo", m)
        self.assertIn('lee request.form["nombre_completo"]', m)
        self.assertIn("400 Bad Request", m)
        self.assertIn('request.form.get("nombre_completo")', m)
        self.assertIn("no tiene ese campo", m)

    def test_los_campos_de_un_mismo_formulario_van_en_un_solo_mensaje(self):
        dos = APP.replace('nombre = request.form["nombre_completo"]', 'nombre = request.form["nombre_completo"]\n        edad = request.form["edad"]')
        [m] = self.informe(dos).errores
        self.assertIn('lee request.form["edad"] y request.form["nombre_completo"] y el formulario no tiene esos campos', m)


if __name__ == "__main__":
    unittest.main()
