"""Revisar el plan antes de empezar (opción de Aura): sin aprobar no se cambia nada y los cambios pedidos llegan al
modelo. Sin la opción, el camino de siempre no cambia."""
import io
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_agente import LLMGuion, llamada

from agente import __main__ as cli
from agente import config
from agente.bucle import APROBAR_PLAN


class RevisarPlan(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_plan_"))

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def mision(self, respuestas: list, ordenes: list) -> tuple[list, list]:
        """Ejecuta el protocolo --json con un modelo de guion; devuelve (mensajes enviados al modelo, eventos)."""
        enviados = []

        class Grabadora(LLMGuion):
            def chat(self, mensajes, herramientas, max_tokens, razonar, al_progreso=None):
                enviados.append(json.loads(json.dumps(mensajes)))
                return super().chat(mensajes, herramientas, max_tokens, razonar, al_progreso)

        LLMGuion.respuestas = respuestas
        entrada = io.TextIOWrapper(io.BytesIO("".join(json.dumps(o) + "\n" for o in ordenes).encode()), encoding="utf-8")
        salida = io.StringIO()
        falso = lambda cfg: ("http://falso", "falso", 32768)
        with mock.patch.object(cli, "detectar", falso), mock.patch("agente.bucle.cliente.detectar", falso), \
                mock.patch("agente.bucle.cliente.LLM", Grabadora), mock.patch("agente.bucle.entorno", lambda: "prueba"), \
                mock.patch.object(sys, "stdin", entrada), mock.patch.object(sys, "stdout", salida):
            self.assertEqual(cli.modo_json(config.cargar(), str(self.dir)), 0)
        return enviados, [json.loads(x) for x in salida.getvalue().splitlines() if x.strip()]

    @staticmethod
    def resultados(enviados: list) -> list[str]:
        return [m["content"] for m in enviados[-1] if m["role"] == "tool"]

    def test_sin_aprobar_no_cambia_nada_y_los_cambios_llegan_al_modelo(self):
        enviados, eventos = self.mision([
            llamada("escribir", ruta="index.html", contenido="<p>antes</p>"),  # antes del plan: rechazado
            llamada("plan", pasos=["Crear index.html"]),
            llamada("plan", pasos=["Crear index.html con un pie de página"]),
            llamada("escribir", ruta="index.html", contenido="<p>hola</p><footer>pie</footer>"),
            llamada("paso_hecho", numero=1),
            {"content": "Listo.", "tool_calls": [], "finish_reason": "stop", "usage": {}},
        ], [{"tipo": "tarea", "texto": "crea una página", "revisar_plan": True},
            {"tipo": "respuesta", "texto": "añade un pie de página"},
            {"tipo": "respuesta", "texto": APROBAR_PLAN},
            {"tipo": "salir"}])
        r = self.resultados(enviados)
        self.assertIn("RECHAZADO: el usuario quiere revisar el plan", r[0])
        self.assertIn("pide cambios en el plan: añade un pie de página", r[1])
        self.assertIn("APROBÓ el plan", r[2])
        preguntas = [e for e in eventos if e["tipo"] == "pregunta"]
        self.assertEqual(len(preguntas), 2)
        self.assertEqual(preguntas[0]["opciones"], [APROBAR_PLAN])
        self.assertIn("1. Crear index.html", preguntas[0]["pregunta"])
        self.assertIn("pie de página", preguntas[1]["pregunta"])
        self.assertEqual((self.dir / "index.html").read_text(encoding="utf-8"), "<p>hola</p><footer>pie</footer>")

    def test_sin_la_opcion_el_camino_de_siempre(self):
        enviados, eventos = self.mision([
            llamada("escribir", ruta="index.html", contenido="<p>hola</p>"),
            {"content": "Listo.", "tool_calls": [], "finish_reason": "stop", "usage": {}},
        ], [{"tipo": "tarea", "texto": "crea una página"}, {"tipo": "salir"}])
        self.assertTrue(self.resultados(enviados)[0].startswith("OK"))
        self.assertFalse([e for e in eventos if e["tipo"] == "pregunta"])
        self.assertTrue((self.dir / "index.html").exists())

    def test_continua_no_vuelve_a_preguntar(self):
        enviados, eventos = self.mision([
            llamada("escribir", ruta="a.txt", contenido="x"),
            {"content": "Listo.", "tool_calls": [], "finish_reason": "stop", "usage": {}},
        ], [{"tipo": "tarea", "texto": "continúa", "revisar_plan": True}, {"tipo": "salir"}])
        self.assertTrue(self.resultados(enviados)[0].startswith("OK"))
        self.assertFalse([e for e in eventos if e["tipo"] == "pregunta"])


if __name__ == "__main__":
    unittest.main()
