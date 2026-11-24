"""m4182 (Gemma): al primer turno de un encargo largo contestó con el plan escrito en el chat, sin llamar a ninguna herramienta, y
la tarea se dio por acabada con ese texto (también en m4143 r1). Ahora se le empuja una vez a empezar con las herramientas; si
la respuesta es una pregunta, el encargo es corto, el modelo no usa herramientas o ya había actuado, todo sigue como hoy."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_agente import llamada, sin_lecciones_reales
from test_terminar_revision import APROBADO, LLMLento

from agente import config

setUpModule = sin_lecciones_reales

MANDATO = ("Necesito un sistema para mi tienda: productos con su costo y su precio de venta, ventas que descuenten las "
           "existencias y un historial de ventas del día. Que funcione en mi computadora con un doble clic.")
EMPUJON = "Aún no has hecho nada en el proyecto"


def texto(contenido: str) -> dict:
    return {"content": contenido, "tool_calls": [], "finish_reason": "stop", "usage": {}}


PLAN_EN_EL_CHAT = texto("Voy a empezar. Paso 1: la base de datos. Paso 2: la pantalla de ventas. Comencemos con la base.")
OTRO_TEXTO = texto("Sigo pensando en el diseño de la tienda.")
PREGUNTA = texto("¿Prefieres que use Flask o Tkinter?")


class SinLlamadasAlPrincipio(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_solotexto_"))
        falso = lambda cfg: ("http://falso", "falso", 32768)
        self.parches = [mock.patch("agente.bucle.cliente.detectar", falso), mock.patch("agente.bucle.cliente.LLM", LLMLento),
                        mock.patch("agente.bucle.entorno", lambda: "prueba")]
        for p in self.parches:
            p.start()

    def tearDown(self):
        for p in self.parches:
            p.stop()
        shutil.rmtree(self.dir, ignore_errors=True)

    def tarea(self, respuestas: list, mandato: str = MANDATO):
        """Ejecuta la tarea con el guion; devuelve (respuesta final, nº de empujones, mensajes de herramienta)."""
        from agente.bucle import Agente
        LLMLento.respuestas = respuestas
        a = Agente(config.cargar(), self.dir, interactivo=False)
        final = a.tarea(mandato)
        self.assertEqual(LLMLento.respuestas, [])  # se consumió el guion entero
        empujones = [m for m in a.ctx.mensajes if m["role"] == "user" and m["content"].startswith(EMPUJON)]
        return final, len(empujones), [m["content"] for m in a.ctx.mensajes if m["role"] == "tool"]

    def test_el_mandato_de_la_prueba_es_largo(self):
        self.assertGreaterEqual(len(MANDATO), 150)

    def test_plan_en_el_chat_al_primer_turno_se_empuja_y_la_tarea_sigue(self):
        _, empujones, herramientas = self.tarea([PLAN_EN_EL_CHAT, llamada("escribir", ruta="main.py", contenido="print('hola')\n"),
                                                 llamada("terminar", resumen="Hecho."), APROBADO])
        self.assertEqual(empujones, 1)
        self.assertEqual(herramientas[-1], "Tarea terminada.")

    def test_solo_se_empuja_una_vez(self):
        final, empujones, _ = self.tarea([PLAN_EN_EL_CHAT, OTRO_TEXTO])
        self.assertEqual(empujones, 1)
        self.assertEqual(final, OTRO_TEXTO["content"])  # la 2.ª respuesta de texto acaba la tarea, como hoy

    def test_un_mandato_corto_no_se_empuja(self):
        final, empujones, _ = self.tarea([PLAN_EN_EL_CHAT], mandato="haz un programa que salude")
        self.assertEqual(empujones, 0)
        self.assertEqual(final, PLAN_EN_EL_CHAT["content"])

    def test_un_mandato_que_es_una_pregunta_no_se_empuja(self):
        _, empujones, _ = self.tarea([PLAN_EN_EL_CHAT], mandato=MANDATO + " ¿Cómo lo hacemos?")
        self.assertEqual(empujones, 0)

    def test_una_respuesta_que_es_una_pregunta_no_se_empuja(self):
        final, empujones, _ = self.tarea([PREGUNTA])
        self.assertEqual(empujones, 0)
        self.assertEqual(final, PREGUNTA["content"])

    def test_un_modelo_sin_tool_use_no_se_empuja(self):
        class SinHerramientas(LLMLento):
            def __init__(self, *args, **kwargs):
                super().__init__(*args, **kwargs)
                self.info = {"capabilities": {"trained_for_tool_use": False}}
                self.modelo = "modelo-sin-herramientas"

        with mock.patch("agente.bucle.cliente.LLM", SinHerramientas):
            final, empujones, _ = self.tarea([PLAN_EN_EL_CHAT])
        self.assertEqual(empujones, 0)
        self.assertIn("no está entrenado para usar herramientas", final)  # su aviso de siempre

    def test_si_ya_actuo_el_texto_acaba_la_tarea_como_hoy(self):
        final, empujones, _ = self.tarea([llamada("escribir", ruta="main.py", contenido="print('hola')\n"), OTRO_TEXTO])
        self.assertEqual(empujones, 0)
        self.assertEqual(final, OTRO_TEXTO["content"])


if __name__ == "__main__":
    unittest.main()
