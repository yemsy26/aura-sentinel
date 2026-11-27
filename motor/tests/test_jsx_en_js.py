"""m4198: React y Docusaurus escriben JSX dentro de .js; node no lo entiende y verificar daba «ERROR DE SINTAXIS JS» en código
correcto de un repositorio ajeno (su documentación, hecha con Docusaurus). Solo se deja de acusar si algo transforma ese JSX."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import verificacion as V
from agente.proyecto import Informe, archivos

PAGINA = "import React from 'react';\n\nexport default function P() {\n  return (\n    <div className=\"x\">hola</div>\n  );\n}\n"
SIN_IMPORTAR = "export default function P() {\n  return <Layout title=\"x\" />;\n}\n"  # React 17+: no hace falta importar React
SUELTO = "function pintar() {\n  document.body.append(<p>hola</p>);\n}\n"
PAQUETE = '{"dependencies": {"react": "^18.0.0"}}'


@unittest.skipUnless(shutil.which("node"), "necesita Node.js")
class TestJsxEnJs(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_jsx_")).resolve()
        self.addCleanup(shutil.rmtree, self.dir, True)

    def revisar(self, **ficheros: str) -> Informe:
        for nombre, texto in ficheros.items():
            p = self.dir / nombre.replace("__", "/")
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(texto, encoding="utf-8", newline="")
        inf = Informe()
        V.html_estatico(self.dir, archivos(self.dir, ".html"), archivos(self.dir, ".js", ".mjs"), inf)
        return inf

    def acusados(self, inf: Informe) -> list[str]:
        return [e for e in inf.errores if "ERROR DE SINTAXIS JS" in e]

    def assert_exento(self, inf: Informe, cuantos: int = 1):
        self.assertEqual(self.acusados(inf), [])
        aviso = [a for a in inf.avisos if "con JSX" in a]
        self.assertEqual(len(aviso), 1, inf.avisos)  # no se calla: no se comprobó
        self.assertTrue(aviso[0].startswith(f"{cuantos} archivo(s) .js con JSX"), aviso[0])

    # --- JSX legítimo: no se acusa
    def test_package_json_de_una_carpeta_superior(self):
        self.assert_exento(self.revisar(**{"docs__package.json": PAQUETE, "docs__src__pages__a.js": SIN_IMPORTAR,
                                           "docs__src__pages__b.js": PAGINA}), 2)

    def test_importa_react_sin_package_json(self):
        self.assert_exento(self.revisar(**{"a.js": PAGINA}))

    def test_importa_docusaurus(self):
        self.assert_exento(self.revisar(**{"a.js": "import Layout from '@theme/Layout';\nexport default () => <Layout />;\n"}))

    def test_fragmento_con_crlf_tabuladores_y_emoji_en_la_linea(self):
        texto = "import x from 'preact';\r\nexport default () => {\r\n\tconst t = '🎉🎉'; return <>{t}</>;\r\n};\r\n"
        self.assert_exento(self.revisar(**{"a.mjs": texto}))

    def test_cargado_con_text_babel(self):
        html = '<html><body><script type="text/babel" src="app.js"></script></body></html>'
        self.assert_exento(self.revisar(**{"index.html": html, "app.js": SUELTO}))

    def test_parcel_empaqueta_desde_el_html(self):
        html = '<html><body><script type="module" src="app.js"></script></body></html>'
        self.assert_exento(self.revisar(**{"index.html": html, "app.js": PAGINA,
                                           "package.json": '{"dependencies": {"react": "1"}, "devDependencies": {"parcel": "2"}}'}))

    def test_el_chequeo_tras_escribir_tampoco_lo_acusa(self):
        self.revisar(**{"package.json": PAQUETE, "a.js": SIN_IMPORTAR})
        self.assertEqual(V.chequeo_rapido(self.dir, self.dir / "a.js"), "")

    # --- lo demás se sigue acusando
    def assert_acusado(self, inf: Informe, archivo: str = "a.js"):
        self.assertTrue(any(e.startswith(archivo) for e in self.acusados(inf)), inf.errores)
        self.assertFalse(any("con JSX" in a for a in inf.avisos), inf.avisos)

    def test_error_real_sin_jsx_aunque_sea_un_proyecto_react(self):
        self.assert_acusado(self.revisar(**{"package.json": PAQUETE, "a.js": "import React from 'react';\nconst x = 1 + ;\n"}))

    def test_jsx_en_una_web_sin_libreria(self):
        html = '<html><body><script src="a.js"></script></body></html>'
        self.assert_acusado(self.revisar(**{"index.html": html, "a.js": SUELTO}))

    def test_jsx_en_un_modulo_sin_html_ni_package_json(self):
        self.assert_acusado(self.revisar(**{"a.js": SIN_IMPORTAR}))

    def test_script_corriente_en_un_html_aunque_haya_react(self):
        html = '<html><body><script src="/web/a.js?v=2"></script></body></html>'
        self.assert_acusado(self.revisar(**{"index.html": html, "web__a.js": PAGINA, "package.json": PAQUETE}),
                            str(Path("web") / "a.js"))

    def test_pagina_html_guardada_como_js(self):
        for texto in ("<!DOCTYPE html>\n<html></html>\n", "<html>\n<body></body>\n</html>\n", "const a = 1;\n<!DOCTYPE html>\n"):
            with self.subTest(texto=texto):
                self.assert_acusado(self.revisar(**{"package.json": PAQUETE, "a.js": texto}))

    def test_conflicto_de_git_y_menor_que_suelto(self):
        for texto in ("const a = 1;\n<<<<<<< HEAD\nconst b = 2;\n=======\n>>>>>>> otra\n", "const x = < 1;\n"):
            with self.subTest(texto=texto):
                self.assert_acusado(self.revisar(**{"package.json": PAQUETE, "a.js": texto}))

    def test_el_package_json_de_fuera_del_proyecto_no_cuenta(self):
        (self.dir / "package.json").write_text(PAQUETE, encoding="utf-8")
        proyecto = self.dir / "dentro"
        proyecto.mkdir()
        (proyecto / "a.js").write_text(SIN_IMPORTAR, encoding="utf-8")
        inf = Informe()
        V.html_estatico(proyecto, [], [proyecto / "a.js"], inf)
        self.assertEqual(len(self.acusados(inf)), 1, inf.errores)


if __name__ == "__main__":
    unittest.main()
