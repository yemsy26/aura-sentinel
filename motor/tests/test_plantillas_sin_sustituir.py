"""verificar avisa de los marcadores que Jinja no sustituye y la persona ve escritos (m4148_neg1: el panel enseñaba «${venta_hoy_total:.2f}»).
Es un aviso: la página responde 200 y no falla nada."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import verificacion
from agente.recorrido_web import plantillas_sin_sustituir


class Hallazgos(unittest.TestCase):
    def setUp(self):
        self.raiz = Path(tempfile.mkdtemp(prefix="test_plantillas_"))
        self.addCleanup(shutil.rmtree, self.raiz, ignore_errors=True)
        (self.raiz / "templates").mkdir()

    def plantilla(self, html: str, nombre: str = "dashboard.html") -> list[dict]:
        (self.raiz / "templates" / nombre).write_text(html, encoding="utf-8")
        return plantillas_sin_sustituir(self.raiz)

    def test_el_marcador_de_formato_en_el_texto_se_ve(self):
        [h] = self.plantilla('<h1>Panel</h1>\n<p class="stat-value">${venta_hoy_total:.2f}</p>\n')
        self.assertEqual((h["plantilla"], h["linea"], h["texto"]), ("templates/dashboard.html", 2, "${venta_hoy_total:.2f}"))

    def test_varios_formatos(self):
        for texto in ("${a.b:,.2f}", "${total}", "{total:.2f}", "{n:d}", "${ x:.1f }"):
            self.assertEqual(len(self.plantilla(f"<td>{texto}</td>")), 1, texto)

    def test_el_marcador_dentro_de_un_segundo_marcador_de_jinja_no_cuenta(self):
        self.assertEqual(self.plantilla("<td>${{ '%.2f'|format(total) }}</td><td>${{ total }}</td>"), [])

    def test_javascript_estilos_template_raw_y_comentarios_no_cuentan(self):
        html = ("<script>var t = `${total:.2f}`; fila.innerHTML = `<td>${nombre}</td>`;</script>\n"
                "<style>.a::after { content: '${x}'; }</style>\n"
                "<template id='fila'><td>${nombre}</td></template>\n"
                "{% raw %}<p>${total:.2f}</p>{% endraw %}\n"
                "<!-- ${total:.2f} -->\n{# ${total:.2f} #}\n")
        self.assertEqual(self.plantilla(html), [])

    def test_un_atributo_de_una_etiqueta_no_cuenta(self):
        self.assertEqual(self.plantilla('<button onclick="mostrar(`${id}`)" title="${total:.2f}">Ver</button>'), [])

    def test_el_numero_de_linea_cuenta_lo_que_se_quito(self):
        html = "<script>\nvar a = 1;\nvar b = 2;\n</script>\n<p>${ganancia:.2f}</p>\n"
        [h] = self.plantilla(html)
        self.assertEqual(h["linea"], 5)

    def test_un_precio_normal_no_es_un_marcador(self):
        self.assertEqual(self.plantilla("<p>Total: $5.00 y {{ x }} y {nombre}</p>"), [])

    def test_sin_carpeta_de_plantillas_no_hay_nada(self):
        self.assertEqual(plantillas_sin_sustituir(self.raiz / "otra"), [])

    def test_una_carpeta_repetida_no_duplica(self):
        (self.raiz / "templates" / "a.html").write_text("<p>${x:.2f}</p>", encoding="utf-8")
        self.assertEqual(len(plantillas_sin_sustituir(self.raiz, self.raiz)), 1)


class MensajeDeVerificar(unittest.TestCase):
    def avisos(self, html: str | None) -> list[str]:
        raiz = Path(tempfile.mkdtemp(prefix="test_plantillas_msg_"))
        self.addCleanup(shutil.rmtree, raiz, ignore_errors=True)
        (raiz / "app.py").write_text("print('x')\n", encoding="utf-8")
        if html is not None:
            (raiz / "templates").mkdir()
            (raiz / "templates" / "dashboard.html").write_text(html, encoding="utf-8")
        falso = {"errores": [], "plantillas": [], "rotos": [], "raros": [], "enviados": [], "paginas": 3, "formularios": 2,
                 "usuario": "admin", "sin_entrar": False, "entrar_js": False, "navegador": None, "lineas": None,
                 "sin_formulario": [], "registro": False, "entrar_get": False, "sin_permiso": [], "login_vacio": None,
                 "empleado": None, "negocio": None, "venta": None}
        inf = verificacion.Informe()
        with mock.patch.object(verificacion, "recorrer", return_value=falso):
            verificacion.probar_web(raiz, raiz / "app.py", sys.executable, inf)
        self.assertEqual(inf.errores, [])  # es un aviso, nunca un error
        return inf.avisos

    def test_el_aviso_dice_donde_esta_y_como_se_escribe(self):
        [m] = self.avisos("<p>Hoy</p>\n<p>${venta_hoy_total:.2f}</p>")
        self.assertIn("la plantilla templates/dashboard.html (línea 2) enseña «${venta_hoy_total:.2f}» tal cual", m)
        self.assertIn("{{ '%.2f'|format(x) }}", m)

    def test_nada_si_las_plantillas_estan_bien(self):
        self.assertEqual(self.avisos("<p>${{ '%.2f'|format(total) }}</p><script>var a = `${b}`;</script>"), [])
        self.assertEqual(self.avisos(None), [])


if __name__ == "__main__":
    unittest.main()
