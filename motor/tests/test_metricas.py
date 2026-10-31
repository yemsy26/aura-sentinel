"""Métricas por turno (.agente/metricas.jsonl): se anotan en cada misión sin cambiar lo que hace el agente."""
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_agente import LLMGuion, borrar, llamada, sin_lecciones_reales

from agente import __main__ as cli
from agente import config, metricas

setUpModule = sin_lecciones_reales  # una tarea lee y anota motor/lecciones.md
LISTO = {"content": "Listo.", "tool_calls": [], "finish_reason": "stop", "usage": {}}


class Metricas(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_met_"))

    def tearDown(self):
        borrar(self.dir)

    def test_cada_turno_y_llamada_queda_anotado(self):
        nota = dict(llamada("escribir", ruta="a.py", contenido="x = 1\nprint(x)\n"), content="Escribo a.py.",
                    usage={"prompt_tokens": 900, "completion_tokens": 40})
        LLMGuion.respuestas = [nota, llamada("editar", ruta="a.py", buscar="x = 1", reemplazar="x = 2"), LISTO]
        ordenes = [{"tipo": "tarea", "texto": "crea a.py"}, {"tipo": "salir"}]
        entrada = io.TextIOWrapper(io.BytesIO("".join(json.dumps(o) + "\n" for o in ordenes).encode()), encoding="utf-8")
        falso = lambda cfg: ("http://falso", "falso", 32768)
        with mock.patch.object(cli, "detectar", falso), mock.patch("agente.bucle.cliente.detectar", falso), \
                mock.patch("agente.bucle.cliente.LLM", LLMGuion), mock.patch("agente.bucle.entorno", lambda: "prueba"), \
                mock.patch.object(sys, "stdin", entrada), mock.patch.object(sys, "stdout", io.StringIO()):
            cli.modo_json(config.cargar(), str(self.dir))
        filas = metricas.leer([str(self.dir)])
        turnos = [f for f in filas if f["tipo"] == "turno"]
        llamadas = {f["nombre"]: f for f in filas if f["tipo"] == "llamada"}
        self.assertEqual(len(turnos), 3)
        self.assertEqual((turnos[0]["tokens_entrada"], turnos[0]["tokens_salida"], turnos[0]["nota_car"]), (900, 40, 13))
        self.assertEqual(llamadas["escribir"]["contenido_car"], len("x = 1\nprint(x)\n"))
        self.assertEqual((llamadas["editar"]["buscar_car"], llamadas["editar"]["reemplazar_car"]), (5, 5))
        self.assertGreater(llamadas["escribir"]["resultado_car"], 0)
        self.assertEqual((self.dir / "a.py").read_text(encoding="utf-8"), "x = 2\nprint(x)\n")  # la misión, igual
        texto = metricas.resumen(filas)
        self.assertIn("3 turnos, 2 llamadas", texto)
        self.assertIn("escribir (contenido)", texto)

    def test_sin_carpeta_o_sin_permiso_no_falla(self):
        metricas.anotar(self.dir / "no_existe", tipo="turno")  # medir nunca para una misión
        self.assertIn("Sin métricas", metricas.resumen([]))


if __name__ == "__main__":
    unittest.main()
