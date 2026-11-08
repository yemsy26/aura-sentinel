"""Botones que llaman a una función que la página no ve: al pulsarlos no pasa nada. Cuatro webs de negocio cargaban
app.js con type="module" y sus onclick="abrirModalProducto()" daban «is not defined» (las funciones de un módulo no se
ven desde onclick); otra llamaba a funciones que no existían. Se mira el código, sin abrir nada."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.proyecto import Informe
from agente.verificacion import funciones_de_botones


class Botones(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_botones_"))
        (self.dir / "static" / "js").mkdir(parents=True)

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def errores(self, pagina: str, script: str = "", ruta: str = "static/index.html") -> list[str]:
        (self.dir / ruta).parent.mkdir(parents=True, exist_ok=True)
        (self.dir / ruta).write_text(pagina, encoding="utf-8")
        (self.dir / "static" / "js" / "app.js").write_text(script, encoding="utf-8")
        inf = Informe()
        funciones_de_botones(self.dir, self.dir, inf)
        return inf.errores

    def test_funcion_de_un_modulo(self):
        [error] = self.errores('<button onclick="abrir()">+ Nuevo</button><script type="module" src="js/app.js"></script>',
                               "function abrir() {}\n")
        self.assertIn("static\\index.html:1 un botón llama a abrir()" if sys.platform == "win32"
                      else "static/index.html:1 un botón llama a abrir()", error)
        self.assertIn('type="module"', error)
        self.assertIn("window.abrir = abrir", error)

    def test_modulo_que_la_deja_en_window(self):
        pagina = '<button onclick="abrir()">+</button><button onclick="cerrar(1)">x</button>' \
                 '<script type="module" src="js/app.js"></script>'
        self.assertEqual(self.errores(pagina, "function abrir() {}\nwindow.abrir = abrir;\nfunction cerrarModal() {}\n"
                                              "Object.assign(window, {cerrar: cerrarModal});\n"), [])

    def test_script_normal_y_codigo_que_no_es_una_llamada(self):
        pagina = ('<button onclick="abrir(); return confirm(\'¿Seguro (de verdad)?\')">+</button>'
                  '<form onsubmit="event.preventDefault(); guardar({{ p.id }})"></form>'
                  '<button onclick="alert(&quot;Hola (mundo)&quot;)">?</button><script src="js/app.js"></script>')
        self.assertEqual(self.errores(pagina, "function abrir() {}\nconst guardar = async (id) => {};\n"), [])

    def test_funcion_que_no_existe_en_el_html_que_construye_un_script(self):
        pagina = ("<table id='t'></table><script>function pintar(u) { t.innerHTML = '<button onclick=\"editarUsuario(' "
                  "+ u.id + ')\">Editar</button>'; }</script>")
        [error] = self.errores(pagina, ruta="templates/usuarios.html")
        self.assertIn("editarUsuario()", error)
        self.assertIn("no está en ningún script del proyecto", error)

    def test_modulo_cargado_con_url_for_de_flask(self):
        pagina = ('<button onclick="agregarItem()">+</button>'
                  '<script type="module" src="{{ url_for(\'static\', filename=\'js/app.js\') }}"></script>')
        [error] = self.errores(pagina, "export function agregarItem() {}\n", ruta="templates/venta.html")
        self.assertIn("agregarItem()", error)
        self.assertIn('type="module"', error)


if __name__ == "__main__":
    unittest.main()
