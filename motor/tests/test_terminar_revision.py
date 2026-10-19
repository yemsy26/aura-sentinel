"""terminar no ignora una revisión que pidió cambios si nada de lo que lee el revisor cambió desde entonces: tampoco cuando
la pidió el agente a mano (revisar) ni cuando ya se había hecho al terminar. Corregir un .json de configuración sí cuenta."""
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
from test_agente import llamada

from agente import config

CAMBIOS = {"content": "REQUISITOS:\n- saludar -> NO CUMPLE: dice adiós\nPROBLEMAS:\n- [CRITICO] main.py:1 - imprime adiós - "
                      "imprime hola\nVEREDICTO: CAMBIOS NECESARIOS", "tool_calls": [], "finish_reason": "stop", "usage": {}}
APROBADO = {"content": "REQUISITOS:\n- saludar -> CUMPLE (main.py:1)\nPROBLEMAS:\n- ninguno\nVEREDICTO: APROBADO",
            "tool_calls": [], "finish_reason": "stop", "usage": {}}
FIN = {"content": "Listo.", "tool_calls": [], "finish_reason": "stop", "usage": {}}


class LLMLento:
    """Modelo de guion que tarda un poco, como el de verdad: si no, la revisión y la edición siguiente caen en el mismo
    instante y la fecha del archivo (de grano más grueso en Windows) parecería anterior a la revisión."""
    respuestas: ClassVar[list] = []

    def __init__(self, base, modelo, contexto, cfg):
        self.contexto = contexto

    def chat(self, mensajes, herramientas, max_tokens, razonar, al_progreso=None):
        time.sleep(0.05)
        return LLMLento.respuestas.pop(0)


class TerminarConRevisionPendiente(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_terminar_"))
        falso = lambda cfg: ("http://falso", "falso", 32768)
        self.parches = [mock.patch("agente.bucle.cliente.detectar", falso), mock.patch("agente.bucle.cliente.LLM", LLMLento),
                        mock.patch("agente.bucle.entorno", lambda: "prueba")]
        for p in self.parches:
            p.start()

    def tearDown(self):
        for p in self.parches:
            p.stop()
        shutil.rmtree(self.dir, ignore_errors=True)

    def mision(self, respuestas: list) -> list[str]:
        """Ejecuta una tarea con el guion; devuelve los resultados de terminar en orden."""
        from agente.bucle import Agente
        LLMLento.respuestas = respuestas
        a = Agente(config.cargar(), self.dir, interactivo=False)
        a.tarea("haz un programa que salude")
        self.assertEqual(LLMLento.respuestas, [])  # se consumió el guion entero (también las revisiones)
        ids = {c["id"] for m in a.ctx.mensajes if m["role"] == "assistant" for c in m.get("tool_calls") or []
               if c["function"]["name"] == "terminar"}
        return [m["content"] for m in a.ctx.mensajes if m["role"] == "tool" and m["tool_call_id"] in ids]

    def test_revisar_a_mano_que_pide_cambios_no_se_ignora_al_terminar(self):
        r = self.mision([llamada("escribir", ruta="main.py", contenido="print('adiós')\n"),
                         llamada("revisar"), CAMBIOS,
                         llamada("terminar", resumen="Listo."), FIN])
        self.assertEqual(len(r), 1)
        self.assertTrue(r[0].startswith("RECHAZADO: la última revisión pidió cambios"), r[0])
        self.assertIn("[CRITICO] main.py:1 - imprime adiós", r[0])

    def test_terminar_otra_vez_sin_corregir_tampoco_pasa(self):
        r = self.mision([llamada("escribir", ruta="main.py", contenido="print('adiós')\n"),
                         llamada("terminar", resumen="Listo."), CAMBIOS,  # la 1ª revisión, la de terminar
                         llamada("terminar", resumen="Listo."), FIN])
        self.assertTrue(r[0].startswith("RECHAZADO: la revisión independiente pide cambios"), r[0])
        self.assertTrue(r[1].startswith("RECHAZADO: la última revisión pidió cambios"), r[1])

    def test_tras_corregir_se_vuelve_a_revisar_y_termina(self):
        r = self.mision([llamada("escribir", ruta="main.py", contenido="print('adiós')\n"),
                         llamada("revisar"), CAMBIOS,
                         llamada("editar", ruta="main.py", buscar="adiós", reemplazar="hola"),
                         llamada("terminar", resumen="Saluda."), APROBADO])
        self.assertEqual(r, ["Tarea terminada."])

    def test_corregir_solo_un_json_de_configuracion_cuenta_como_cambio(self):
        """capabilities/default.json o tauri.conf.json: antes no contaban y terminar pasaba sin la 2ª revisión."""
        json_mal = {**CAMBIOS, "content": CAMBIOS["content"].replace("main.py:1 - imprime adiós - imprime hola",
                                                                     "config.json:1 - falta saludo - añade \"saludo\"")}
        r = self.mision([llamada("escribir", ruta="main.py", contenido="import json\n\nprint(json.load(open('config.json')))\n"),
                         llamada("escribir", ruta="config.json", contenido="{}\n"),
                         llamada("revisar"), json_mal,
                         llamada("escribir", ruta="config.json", contenido='{"saludo": "hola"}\n'),
                         llamada("terminar", resumen="Saluda."), APROBADO])
        self.assertEqual(r, ["Tarea terminada."])


if __name__ == "__main__":
    unittest.main()
