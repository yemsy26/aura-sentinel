"""m4174/C5: _puede_terminar y _rescatar llamaban a self.h.verificar_cmd() directamente, sin pasar por despachar() (el único
sitio que atrapa cualquier excepción de una herramienta). Una excepción real ahí (visto con http.client.InvalidURL en una
URL sin codificar) tiraba el proceso entero sin ningún mensaje. Ahora las dos llamadas están protegidas igual que despachar."""
import shutil
import sys
import tempfile
import time
import unittest
from pathlib import Path
from typing import ClassVar
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_agente import llamada, sin_lecciones_reales

from agente import config
from agente.herramientas import Herramientas

FIN = {"content": "Listo.", "tool_calls": [], "finish_reason": "stop", "usage": {}}
setUpModule = sin_lecciones_reales


class LLMLento:
    respuestas: ClassVar[list] = []

    def __init__(self, base, modelo, contexto, cfg):
        self.contexto = contexto

    def chat(self, mensajes, herramientas, max_tokens, razonar, al_progreso=None):
        time.sleep(0.05)
        return LLMLento.respuestas.pop(0)


class VerificarCmdLanzaExcepcion(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_verificarcae_"))
        falso = lambda cfg: ("http://falso", "falso", 32768)
        self.parches = [mock.patch("agente.bucle.cliente.detectar", falso), mock.patch("agente.bucle.cliente.LLM", LLMLento),
                        mock.patch("agente.bucle.entorno", lambda: "prueba")]
        for p in self.parches:
            p.start()

    def tearDown(self):
        for p in self.parches:
            p.stop()
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_puede_terminar_no_tira_la_mision_si_verificar_cmd_lanza(self):
        from agente.bucle import Agente
        LLMLento.respuestas = [llamada("escribir", ruta="main.py", contenido="print('hola')\n"),
                                llamada("terminar", resumen="Saluda."), FIN]
        with mock.patch.object(Herramientas, "verificar_cmd", side_effect=RuntimeError("boom")):
            a = Agente(config.cargar(), self.dir, interactivo=False)
            a.tarea("haz un programa que salude")
        self.assertEqual(LLMLento.respuestas, [])  # se consumió el guion entero: no se cayó a mitad
        ids = {c["id"] for m in a.ctx.mensajes if m["role"] == "assistant" for c in m.get("tool_calls") or []
               if c["function"]["name"] == "terminar"}
        resultados = [m["content"] for m in a.ctx.mensajes if m["role"] == "tool" and m["tool_call_id"] in ids]
        self.assertEqual(len(resultados), 1)
        self.assertEqual(resultados[0], "RECHAZADO: la verificación automática falló: RuntimeError: boom")

    def test_rescatar_no_tira_la_mision_si_verificar_cmd_lanza(self):
        from agente.bucle import Agente
        a = Agente(config.cargar(), self.dir, interactivo=False)
        a.h.checkpoints = [{"paso": 1, "fecha": "2026-11-23 00:00", "archivos": []}]
        with mock.patch.object(Herramientas, "verificar_cmd", side_effect=RuntimeError("boom")):
            r = a._rescatar()
        self.assertEqual(r, "\nLa verificación final falló: RuntimeError: boom.")


if __name__ == "__main__":
    unittest.main()
