"""verificar avisa de una variable que la plantilla lee y que ninguna ruta le pasa (u4138_vet, la veterinaria del usuario con la 4.13.8: los botones
«Editar» y «💉 Vacuna» iban dentro de un `{% if can_edit %}` que nadie definía, el desplegable de vacunas iteraba `vacunas` y la ficha del dueño iteraba
`pacientes`, y ninguna ruta pasaba esos nombres: Jinja no da error, la lista sale vacía y el «if» es falso). La sonda (con el Python del proyecto) dice
qué nombres no define el entorno ni un procesador de contexto; el motor mira si alguna render_template se los pasa."""
import importlib.util
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import verificacion
from agente.recorrido_web import recorrer, variables_sin_pasar

# Flask no está en el entorno del motor: las pruebas con la sonda usan un Python que lo tenga (si no hay, se saltan).
PY_FLASK = os.environ.get("AURA_PRUEBA_PY_FLASK") or (sys.executable if importlib.util.find_spec("flask") else None)

APP = '''from flask import Flask, render_template
app = Flask(__name__)


@app.route("/")
def inicio():
    return render_template("inicio.html", titulo="t", productos=[])


@app.route("/otra")
def otra():
    return render_template("otra.html", dueno=1)
'''


def sonda(plantilla: str, **nombres) -> list[dict]:
    """Lo que dice web_arranque de una plantilla: nombres = {nombre: (línea, uso)}."""
    return [{"plantilla": plantilla, "nombres": {k: {"linea": v[0], "uso": v[1]} for k, v in nombres.items()}}]


class LoQueFaltaEnLasRutas(unittest.TestCase):
    def setUp(self):
        self.raiz = Path(tempfile.mkdtemp(prefix="test_variables_"))
        self.addCleanup(shutil.rmtree, self.raiz, ignore_errors=True)

    def web(self, hablo: list[dict], app: str = APP, plantillas: dict | None = None) -> list[dict]:
        (self.raiz / "app.py").write_text(app, encoding="utf-8")
        (self.raiz / "templates").mkdir(exist_ok=True)
        for nombre, texto in (plantillas or {}).items():
            (self.raiz / "templates" / nombre).write_text(texto, encoding="utf-8")
        return variables_sin_pasar(hablo, self.raiz)

    def test_el_nombre_que_ninguna_ruta_pasa_dice_que_pasan_y_que_se_parece(self):
        [h] = self.web(sonda("inicio.html", productos_bajos=(12, "for")), APP.replace("productos=[]", "productos_bajo=[]"))
        self.assertEqual((h["plantilla"], h["nombre"], h["linea"], h["uso"]), ("inicio.html", "productos_bajos", 12, "for"))
        self.assertEqual((h["pasan"], h["sugerencia"]), (["productos_bajo", "titulo"], "productos_bajo"))

    def test_lo_que_alguna_ruta_pasa_no_cuenta(self):
        self.assertEqual(self.web(sonda("inicio.html", titulo=(1, "salida"), productos=(2, "for"))), [])

    def test_varias_rutas_a_la_misma_plantilla_suman_lo_que_pasan(self):
        app = APP + '\n\n@app.route("/x")\ndef x():\n    return render_template("inicio.html", can_edit=True)\n'
        self.assertEqual(len(self.web(sonda("inicio.html", can_edit=(3, "if")), APP)), 1)
        self.assertEqual(self.web(sonda("inicio.html", can_edit=(3, "if")), app), [])

    def test_con_dos_asteriscos_o_argumentos_de_mas_se_calla_esa_plantilla(self):
        for llamada in ('render_template("inicio.html", **contexto)', 'render_template("inicio.html", titulo=1, **contexto)',
                        'render_template("inicio.html", {"a": 1})'):
            with self.subTest(llamada=llamada):
                app = APP.replace('render_template("inicio.html", titulo="t", productos=[])', llamada)
                self.assertEqual(self.web(sonda("inicio.html", can_edit=(3, "if")), app), [])
        self.assertEqual(len(self.web(sonda("otra.html", can_edit=(3, "if")))), 1)  # las otras siguen mirándose

    def test_una_web_que_pinta_plantillas_de_otra_forma_se_calla_entera(self):
        for cambio in ('render_template(nombre, titulo=1)', 'render_template_string("{{ x }}")', 'app.jinja_env.get_template("a.html").render()',
                       'render_template(*args)', 'render_template()'):
            with self.subTest(cambio=cambio):
                app = APP + f"\n\n@app.route('/z')\ndef z():\n    return {cambio}\n"
                self.assertEqual(self.web(sonda("otra.html", can_edit=(3, "if")), app), [])

    def test_una_plantilla_que_ninguna_render_template_nombra_no_se_mira(self):
        self.assertEqual(self.web(sonda("parcial.html", item=(1, "salida"))), [])  # se incluye: recibe el contexto de quien la incluye

    def test_lo_que_define_otra_plantilla_con_set_for_with_o_macro_no_cuenta(self):
        plantillas = {"base.html": "{% set rol = 1 %}{% for fila in tabla %}{% endfor %}{% macro m(valor, otro=2) %}{% endmacro %}{% with a = 1, b = 2 %}{% endwith %}"}
        for nombre in ("rol", "fila", "valor", "otro", "a", "b"):
            with self.subTest(nombre=nombre):
                self.assertEqual(self.web(sonda("inicio.html", **{nombre: (1, "salida")}), plantillas=plantillas), [])
        self.assertEqual(len(self.web(sonda("inicio.html", tabla=(1, "for")), plantillas=plantillas)), 1)  # la lista que recorre ese for, no

    def test_una_pagina_que_rellena_javascript_lo_dice(self):
        sin_js, con_js = {"inicio.html": "{% for u in usuarios %}{% endfor %}"}, {"inicio.html": "{% for u in usuarios %}{% endfor %}<script>fetch('/api')</script>"}
        self.assertFalse(self.web(sonda("inicio.html", usuarios=(1, "for")), plantillas=sin_js)[0]["con_js"])
        self.assertTrue(self.web(sonda("inicio.html", usuarios=(1, "for")), plantillas=con_js)[0]["con_js"])

    def test_sin_nombres_o_sin_sonda_no_dice_nada(self):
        self.assertEqual(self.web([]), [])
        self.assertEqual(self.web(sonda("inicio.html")), [])

    def test_no_lee_el_entorno_virtual(self):
        (self.raiz / ".venv" / "Lib").mkdir(parents=True)
        (self.raiz / ".venv" / "Lib" / "otro.py").write_text("from flask import render_template\nrender_template(nombre)\n", encoding="utf-8")
        self.assertEqual(len(self.web(sonda("inicio.html", can_edit=(3, "if")))), 1)


@unittest.skipUnless(PY_FLASK, "sin un Python con Flask")
class ConLaSondaDeVerdad(unittest.TestCase):
    def setUp(self):
        self.raiz = Path(tempfile.mkdtemp(prefix="test_variables_sonda_"))
        self.addCleanup(shutil.rmtree, self.raiz, ignore_errors=True)

    def test_la_sonda_dice_los_nombres_y_como_se_usan(self):
        app = ('from flask import Flask, render_template\napp = Flask(__name__)\n\n@app.context_processor\ndef contexto():\n    return {"empresa": "X"}\n\n'
               '@app.route("/")\ndef inicio():\n    return render_template("inicio.html", titulo="t")\n\nif __name__ == "__main__":\n    app.run(port=5000)\n')
        html = ("{{ titulo }} {{ empresa }} {{ request.path }} {{ url_for('inicio') }}\n"          # 1: pasan, procesador y globales de Flask
                "{% if can_edit %}<a>Editar</a>{% endif %}\n"                                         # 2
                "{% for v in vacunas %}<p>{{ v }}</p>{% endfor %}\n"                                  # 3
                "{{ ventas_hoy|length }}\n"                                                           # 4
                "{{ cita.fecha if cita else '' }}\n"                                                  # 5
                "{% if rol is not defined %}x{% endif %}{{ opcional|default('') }}\n"                 # 6: a propósito
                "{{ abs(3) }}\n"                                                                      # 7
                "{% for p in dueno.pacientes %}{{ p }}{% endfor %}\n")                                # 8
        (self.raiz / "app.py").write_text(app, encoding="utf-8")
        (self.raiz / "templates").mkdir()
        (self.raiz / "templates" / "inicio.html").write_text(html, encoding="utf-8")
        r = recorrer(self.raiz / "app.py", PY_FLASK, [])
        [entrada] = r["variables"]
        self.assertEqual(entrada["plantilla"], "inicio.html")
        usos = {k: (v["linea"], v["uso"]) for k, v in entrada["nombres"].items()}
        self.assertEqual(usos, {"titulo": (1, "salida"), "can_edit": (2, "if"), "vacunas": (3, "for"), "ventas_hoy": (4, "salida"),
                                "cita": (5, "if"), "abs": (7, "atributo"), "dueno": (8, "atributo")})
        # y con la ruta que solo pasa «titulo», el motor deja solo lo que de verdad falta
        self.assertEqual(sorted(h["nombre"] for h in variables_sin_pasar(r["variables"], self.raiz)),
                         ["abs", "can_edit", "cita", "dueno", "vacunas", "ventas_hoy"])

    def test_un_procesador_que_falla_calla_la_sonda(self):
        app = ('from flask import Flask, render_template\napp = Flask(__name__)\n\n@app.context_processor\ndef contexto():\n    return 1 / 0\n\n'
               '@app.route("/")\ndef inicio():\n    return render_template("inicio.html")\n\nif __name__ == "__main__":\n    app.run(port=5000)\n')
        (self.raiz / "app.py").write_text(app, encoding="utf-8")
        (self.raiz / "templates").mkdir()
        (self.raiz / "templates" / "inicio.html").write_text("{% if can_edit %}x{% endif %}", encoding="utf-8")
        self.assertEqual(recorrer(self.raiz / "app.py", PY_FLASK, [])["variables"], [])


class MensajeDeVerificar(unittest.TestCase):
    def informe(self, hablo: list[dict], app: str = APP) -> "verificacion.Informe":
        raiz = Path(tempfile.mkdtemp(prefix="test_variables_msg_"))
        self.addCleanup(shutil.rmtree, raiz, ignore_errors=True)
        (raiz / "app.py").write_text(app, encoding="utf-8")
        falso = {"errores": [], "plantillas": [], "rotos": [], "raros": [], "enviados": [], "paginas": 3, "formularios": 2,
                 "usuario": "admin", "sin_entrar": False, "entrar_js": False, "navegador": None, "lineas": None,
                 "sin_formulario": [], "registro": False, "entrar_get": False, "sin_permiso": [], "login_vacio": None,
                 "empleado": None, "negocio": None, "venta": None, "variables": hablo}
        inf = verificacion.Informe()
        with mock.patch.object(verificacion, "recorrer", return_value=falso):
            verificacion.probar_web(raiz, raiz / "app.py", sys.executable, inf)
        return inf

    def test_una_lista_vacia_para_siempre_es_un_error_con_lo_que_pasa_la_ruta(self):
        inf = self.informe(sonda("inicio.html", productos_bajos=(12, "for"), can_edit=(3, "if")), APP.replace("productos=[]", "productos_bajo=[]"))
        [m] = inf.errores
        self.assertIn("la plantilla inicio.html lee «productos_bajos» (línea 12: el bucle sale siempre vacío), «can_edit» (línea 3: esa parte nunca se muestra)", m)
        self.assertIn("render_template solo le pasa productos_bajo, titulo", m)
        self.assertIn("en vez de «productos_bajos» ¿«productos_bajo»?", m)

    def test_un_if_o_un_texto_en_blanco_solo_es_un_aviso(self):
        inf = self.informe(sonda("inicio.html", can_edit=(3, "if"), ventas_hoy=(4, "salida")))
        self.assertEqual(inf.errores, [])
        [m] = inf.avisos
        self.assertIn("«can_edit» (línea 3: esa parte nunca se muestra), «ventas_hoy» (línea 4: sale en blanco)", m)

    def test_un_atributo_de_algo_sin_definir_da_500(self):
        [m] = self.informe(sonda("inicio.html", dueno=(8, "atributo")), APP.replace("dueno=1", "x=1")).errores
        self.assertIn("da error 500 al pintar esa parte", m)

    def test_un_bucle_en_una_pagina_con_javascript_es_solo_un_aviso(self):
        raiz = Path(tempfile.mkdtemp(prefix="test_variables_js_"))
        self.addCleanup(shutil.rmtree, raiz, ignore_errors=True)
        (raiz / "templates").mkdir()
        (raiz / "templates" / "inicio.html").write_text("{% for u in usuarios %}{% endfor %}<script>fetch('/api/usuarios')</script>", encoding="utf-8")
        (raiz / "app.py").write_text(APP, encoding="utf-8")
        falso = {"errores": [], "plantillas": [], "rotos": [], "raros": [], "enviados": [], "paginas": 3, "formularios": 2,
                 "usuario": "admin", "sin_entrar": False, "entrar_js": False, "navegador": None, "lineas": None,
                 "sin_formulario": [], "registro": False, "entrar_get": False, "sin_permiso": [], "login_vacio": None,
                 "empleado": None, "negocio": None, "venta": None, "variables": sonda("inicio.html", usuarios=(1, "for"))}
        inf = verificacion.Informe()
        with mock.patch.object(verificacion, "recorrer", return_value=falso):
            verificacion.probar_web(raiz, raiz / "app.py", sys.executable, inf)
        self.assertEqual((len(inf.errores), len(inf.avisos)), (0, 1))

    def test_sin_nada_que_decir_no_hay_mensajes(self):
        inf = self.informe(sonda("inicio.html", titulo=(1, "salida")))
        self.assertEqual((inf.errores, inf.avisos), ([], []))
        inf = self.informe([])
        self.assertEqual((inf.errores, inf.avisos), ([], []))


if __name__ == "__main__":
    unittest.main()
