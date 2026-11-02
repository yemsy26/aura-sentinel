"""Al arrancar un servidor, el motor mira la dirección que abrirá el usuario, y terminar exige que haya respondido: en una
misión, las pruebas, el revisor y verificar pasaron con la portada dando 404 porque solo miraban /api; en otra, el servidor
nunca se arrancó y todo daba 501. Con servidores de verdad."""
import socket
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_agente import borrar, llamada

from agente import config
from agente.bucle import Agente
from agente.herramientas import Herramientas

SERVIDOR = """import http.server, sys
codigos = dict(p.split("=") for p in sys.argv[2:])
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(int(codigos.get(self.path, 404)))
        self.end_headers()
        self.wfile.write(b"x")
    def log_message(self, *a):
        pass
http.server.HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
"""


def puerto_libre() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class AvisoPortada(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_portada_"))
        self.h = Herramientas(self.dir)
        (self.dir / "servidor.py").write_text(SERVIDOR, encoding="utf-8")
        self.puerto = puerto_libre()

    def tearDown(self):
        self.h.detener_todos()
        borrar(self.dir)

    def arrancar(self, *codigos: str) -> str:
        return self.h.ejecutar(f"python servidor.py {self.puerto} {' '.join(codigos)}", fondo=True)

    def test_la_portada_del_mandato_da_404(self):
        self.h.nueva_tarea(1, f"Crea un panel en http://localhost:{self.puerto}, por favor.")
        r = self.arrancar("/api/meses=200")
        self.assertIn(f"[AVISO: http://localhost:{self.puerto}/ responde 404", r)
        self.assertIn("ver_pantalla", r)

    def test_la_portada_que_abre_no_avisa(self):
        self.h.nueva_tarea(1, f"Crea un panel en http://localhost:{self.puerto}")
        self.assertNotIn("AVISO", self.arrancar("/=200"))

    def test_la_direccion_del_bat_cuenta_aunque_el_mandato_no_la_nombre(self):
        self.h.nueva_tarea(1, "Crea un panel web")
        (self.dir / "run.bat").write_text(f"start http://localhost:{self.puerto}/\npython servidor.py\n", encoding="utf-8")
        self.assertIn("responde 404", self.arrancar())

    def test_un_error_del_servidor_avisa_y_un_acceso_con_clave_no(self):
        self.h.nueva_tarea(1, f"Panel en http://localhost:{self.puerto}/")
        self.assertIn("responde 500", self.arrancar("/=500"))
        self.h.detener_todos()
        self.puerto = puerto_libre()
        self.h.nueva_tarea(1, f"Panel en http://localhost:{self.puerto}/")
        self.assertNotIn("AVISO", self.arrancar("/=401"))

    def test_si_otro_proceso_no_sirve_esa_direccion_dice_que_no_responde(self):
        self.h.nueva_tarea(1, f"Panel en http://localhost:{puerto_libre()}/")
        self.assertIn("no responde", self.arrancar("/=200"))

    def test_sin_direccion_no_mira_nada(self):
        self.h.nueva_tarea(1, "Crea un programa")
        self.assertEqual(self.h._direcciones("python servidor.py"), [])

    def test_las_direcciones_sin_repetir_y_solo_locales(self):
        self.h.nueva_tarea(1, "Abre http://localhost:8765, http://localhost:8765/ y http://127.0.0.1:9000/panel. Ver https://ejemplo.com:8080/")
        self.assertEqual(self.h._direcciones(""), ["http://localhost:8765/", "http://127.0.0.1:9000/panel"])

    def test_un_proceso_que_ya_termino_no_comprueba_nada(self):
        self.h.nueva_tarea(1, f"Panel en http://localhost:{self.puerto}/")
        r = self.h.ejecutar('python -c "print(1)"', fondo=True)
        self.assertIn("TERMINÓ", r)
        self.assertNotIn("AVISO", r)


class Terminar(unittest.TestCase):
    """Con el modelo guionizado: terminar no pasa hasta que la dirección haya respondido."""

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_web_"))
        self.puerto = puerto_libre()
        self.agente = None

    def tearDown(self):
        if self.agente:
            self.agente.h.detener_todos()
        borrar(self.dir)

    def correr(self, mandato, *respuestas):
        guion = list(respuestas)

        class Guion:
            def __init__(self, *_):
                pass

            def chat(self, mensajes, *_):
                return guion.pop(0)

        falso = lambda cfg: ("http://falso", "falso", 32768)
        with mock.patch("agente.bucle.cliente.detectar", falso), mock.patch("agente.bucle.cliente.LLM", Guion), \
                mock.patch("agente.bucle.entorno", lambda: "prueba"), \
                mock.patch("agente.bucle.Agente._revisar", lambda *a, **k: ("REVISIÓN: aprobado", True)):
            self.agente = Agente(config.cargar(), self.dir, interactivo=False)
            return self.agente.tarea(mandato)

    def rechazos(self):
        return [m["content"] for m in self.agente.ctx.mensajes if m["role"] == "tool" and m["content"].startswith("RECHAZADO")]

    def escribir(self):
        return llamada("escribir", ruta="servidor.py", contenido=SERVIDOR)

    def arrancar(self, codigo="/=200"):
        return llamada("ejecutar", comando=f"python servidor.py {self.puerto} {codigo}", fondo=True)

    def test_no_termina_sin_arrancar_la_web(self):
        fin = self.correr(f"Panel en http://localhost:{self.puerto}", self.escribir(), llamada("terminar", resumen="hecho"),
                          self.arrancar(), llamada("terminar", resumen="ahora sí"))
        self.assertEqual(fin, "ahora sí")
        self.assertIn("y no ha respondido", self.rechazos()[0])

    def test_una_web_que_da_error_no_cuenta_como_abierta(self):
        fin = self.correr(f"Panel en http://localhost:{self.puerto}", self.escribir(), self.arrancar("/x=200"),
                          *[llamada("terminar", resumen="hecho") for _ in range(5)])
        self.assertIn("SIN haber comprobado que abre", fin)

    def test_cambiar_el_programa_obliga_a_abrirla_otra_vez_y_cambiar_la_documentacion_no(self):
        h = Herramientas(self.dir)
        h.nueva_tarea(1, f"Panel en http://localhost:{self.puerto}")
        h.paso = 5
        h._guardar(self.dir / "app.py", "print(1)\n")
        self.assertEqual(len(h.web_sin_abrir()), 1)
        h.web_ok[f"http://localhost:{self.puerto}/"] = h.paso_programa
        self.assertEqual(h.web_sin_abrir(), [])
        h.paso = 6
        h._guardar(self.dir / "README.md", "x\n")
        h._guardar(self.dir / "test_app.py", "x = 1\n")
        self.assertEqual(h.web_sin_abrir(), [])
        h._guardar(self.dir / "app.py", "print(2)\n")
        self.assertEqual(len(h.web_sin_abrir()), 1)

    def test_sin_direccion_termina_como_siempre(self):
        fin = self.correr("Crea un programa que imprima 1", llamada("escribir", ruta="a.py", contenido="print(1)\n"),
                          llamada("ejecutar", comando="python a.py"), llamada("terminar", resumen="hecho"))
        self.assertEqual(fin, "hecho")


if __name__ == "__main__":
    unittest.main()
