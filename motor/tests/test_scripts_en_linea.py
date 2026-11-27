"""Los <script> en línea que no son JavaScript no pasan por node --check (sin modelo)."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_agente import borrar

from agente.verificacion import Informe, html_estatico

HAY_NODE = shutil.which("node") is not None
MAL = "const x = ;"


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_scripts_"))

    def tearDown(self):
        borrar(self.dir)

    def revisar(self, cuerpo: str) -> Informe:
        h = self.dir / "index.html"
        h.write_text(f"<!doctype html><html><body>{cuerpo}</body></html>", encoding="utf-8")
        inf = Informe()
        html_estatico(self.dir, [h], [], inf)
        return inf


class TestBloquesDeDatos(Base):
    def test_no_se_acusan(self):
        bloques = {
            "importmap": '{"imports": {"three": "./three.js"}}',
            "speculationrules": '{"prerender": []}',
            "application/ld+json": '{"@type": "Store"}',
            "application/json": "[1, 2]",
            "text/babel": "const A = () => <div>hola</div>;",
            "text/template": "<li>{{ n }}</li>",
            "text/x-template": "<li>{{ n }}</li>",
            "text/html": "<li>a</li>",
            "text/typescript": "const x: number = 1;",
            "x-shader/x-fragment": "void main() { gl_FragColor = vec4(1.0); }",
            "text/javascript; charset=utf-8": MAL,  # con parámetros el navegador no lo ejecuta
        }
        for tipo, codigo in bloques.items():
            with self.subTest(tipo=tipo):
                inf = self.revisar(f'<script type="{tipo}">{codigo}</script>')
                self.assertEqual((inf.errores, inf.avisos), ([], []))

    def test_importmap_mal_escrito_es_un_error_de_json(self):
        inf = self.revisar('<script type="importmap">{"imports": {"three": "./three.js",}}</script>')
        self.assertEqual(len(inf.errores), 1, inf.errores)
        self.assertIn('index.html <script> #1 (type="importmap"): JSON mal escrito', inf.errores[0])
        self.assertNotIn("SINTAXIS JS", inf.errores[0])

    def test_otro_json_mal_escrito_solo_avisa(self):
        inf = self.revisar('<script type="application/ld+json">{"a": }</script>')
        self.assertEqual(inf.errores, [])
        self.assertIn("JSON mal escrito", inf.avisos[0])


@unittest.skipUnless(HAY_NODE, "sin node")
class TestJavaScriptEnLinea(Base):
    def test_un_error_real_sigue_contando(self):
        for atributos in ("", ' type=""', ' type="module"', ' type="text/javascript"',
                          ' type=" Application/JavaScript "'):
            with self.subTest(atributos=atributos):
                inf = self.revisar(f"<script{atributos}>{MAL}</script>")
                self.assertEqual(len(inf.errores), 1, inf.errores)
                self.assertIn("ERROR DE SINTAXIS JS", inf.errores[0])

    def test_el_numero_cuenta_tambien_los_bloques_de_datos(self):
        inf = self.revisar(f'<script type="importmap">{{}}</script><script>{MAL}</script>')
        self.assertEqual(len(inf.errores), 1, inf.errores)
        self.assertIn("<script> #2", inf.errores[0])

    def test_un_script_normal_no_hereda_el_modo_de_un_modulo(self):
        # «with» es válido en un script normal y está prohibido en un módulo
        inf = self.revisar('<script>with (Math) { PI; }</script><script type="module">export {};</script>')
        self.assertEqual(inf.errores, [])


if __name__ == "__main__":
    unittest.main()
