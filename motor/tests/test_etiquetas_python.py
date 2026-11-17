"""Un botón o enlace cuyo texto es False, True o None: la plantilla pinta el valor de una variable de Python (m4153_neg4)."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from agente import verificacion
from agente.recorrido_web import _Recorrido

BASE = "http://127.0.0.1:5000/"


def etiquetas(html: str, final: str = BASE + "ventas/nueva?x=1") -> list[dict]:
    r = _Recorrido(BASE, Path("sin_registro.log"), [])
    r._etiquetas_de_python(final, r._leer(html))
    return r.etiquetas


class EtiquetasDePython(unittest.TestCase):
    def test_el_boton_de_un_formulario_que_dice_false(self):
        self.assertEqual(etiquetas('<form method="post"><button type="submit">False</button></form>'),
                         [{"pagina": "/ventas/nueva", "texto": "False", "que": "botón"}])

    def test_el_input_de_enviar_con_value_none(self):
        self.assertEqual(etiquetas('<form><input type="submit" value="None"></form>'),
                         [{"pagina": "/ventas/nueva", "texto": "None", "que": "botón"}])

    def test_con_emoji_y_espacios_alrededor(self):
        [e] = etiquetas('<form><button>\n  \u2705 True </button></form>')
        self.assertEqual(e["texto"], "True")

    def test_el_enlace_que_dice_none(self):
        self.assertEqual(etiquetas('<a href="/categorias/3" class="btn">None</a>'),
                         [{"pagina": "/ventas/nueva", "texto": "None", "que": "enlace"}])

    def test_el_boton_fuera_de_todo_formulario(self):
        self.assertEqual([e["texto"] for e in etiquetas('<button onclick="x()">False</button>')], ["False"])

    def test_los_textos_legitimos_no_cuentan(self):
        html = ('<form><button>Guardar</button><button>Ninguno</button><button>None of the above</button><button>Falsear</button>'
                '<button>Verdadero</button><input type="submit" value="Enviar"></form><a href="/x">Falsa alarma</a><p>None</p><td>False</td>')
        self.assertEqual(etiquetas(html), [])

    def test_solo_cuenta_el_texto_que_se_ve(self):
        html = ('<form><button type="submit" value="False">Guardar</button><input type="text" value="False"></form>'
                '<script>document.write("<button>False</button>")</script><!-- <button>None</button> -->')
        self.assertEqual(etiquetas(html), [])

    def test_el_mismo_texto_en_la_misma_pagina_se_cuenta_una_vez(self):
        self.assertEqual(len(etiquetas('<form><button>False</button></form><form><button>False</button></form>')), 1)


class MensajeDeEtiquetas(unittest.TestCase):
    def avisos(self, lista: list[dict]) -> tuple[list[str], list[str]]:
        raiz = Path(tempfile.mkdtemp(prefix="test_etiquetas_msg_"))
        self.addCleanup(shutil.rmtree, raiz, ignore_errors=True)
        (raiz / "app.py").write_text("print('x')\n", encoding="utf-8")
        falso = {"errores": [], "plantillas": [], "rotos": [], "raros": [], "enviados": [], "paginas": 3, "formularios": 2,
                 "usuario": "admin", "sin_entrar": False, "entrar_js": False, "navegador": None, "lineas": None,
                 "sin_formulario": [], "registro": False, "entrar_get": False, "sin_permiso": [], "login_vacio": None,
                 "empleado": None, "negocio": {}, "venta": None, "etiquetas": lista}
        inf = verificacion.Informe()
        with mock.patch.object(verificacion, "recorrer", return_value=falso):
            verificacion.probar_web(raiz, raiz / "app.py", sys.executable, inf)
        return inf.avisos, inf.errores

    def test_un_boton_que_dice_false_es_un_aviso(self):
        avisos, errores = self.avisos([{"pagina": "/ventas/nueva", "texto": "False", "que": "botón"}])
        self.assertEqual(errores, [])
        [m] = avisos
        self.assertIn("un botón de /ventas/nueva dice «False»", m)
        self.assertIn("form.submit.label.text", m)

    def test_varios_con_el_mismo_texto_se_juntan(self):
        avisos, _ = self.avisos([{"pagina": "/ventas/nueva", "texto": "False", "que": "botón"},
                                 {"pagina": "/clientes", "texto": "False", "que": "enlace"}])
        [m] = avisos
        self.assertIn("(y 1 más: enlace de /clientes)", m)

    def test_sin_etiquetas_no_dice_nada(self):
        self.assertEqual(self.avisos([]), ([], []))


if __name__ == "__main__":
    unittest.main()
