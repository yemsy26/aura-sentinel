"""Lo que hacía fallar a modelos distintos de Qwen3.6 en la misión de la peluquería:
- contestar el encargo como un chat («Entendido. ¿Cómo puedo ayudarte?») y acabar sin hacer nada;
- acabar con texto («He completado todas las fases») con el plan a medias, sin pasar la puerta de terminar;
- con 16K de contexto, compactar cada pocos pasos: las instrucciones y herramientas ocupan ~5.800 tokens fijos;
- las métricas no decían con qué modelo se había hecho la misión."""
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import config
from agente.bucle import Agente
from agente.contexto import Contexto

ENCARGO = ("Tengo una peluquería pequeña y necesito un sistema para llevar las citas. Quiero registrar clientes y "
           "servicios, dar citas sin que se pisen y ver cuánto he ganado en un mes. Quiero abrirlo con doble clic.")


def texto(t: str) -> dict:
    return {"content": t, "finish_reason": "stop", "usage": {}, "tool_calls": []}


def llamada(nombre: str, **args) -> dict:
    return {"content": "", "finish_reason": "tool_calls", "usage": {},
            "tool_calls": [{"id": nombre, "name": nombre, "arguments": json.dumps(args)}]}


class OtrosModelos(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_modelos_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)
        self.guion, self.enviados = [], []
        prueba = self

        class Falso:
            def __init__(self, base, modelo, contexto, cfg):
                self.base, self.modelo, self.contexto, self.cfg = base, modelo, contexto, cfg
                self.info = {}

            def chat(self, mensajes, herramientas, max_tokens, razonar, al_progreso=None):
                prueba.enviados.append(mensajes[-1]["content"] if mensajes[-1]["role"] == "user" else "")
                return {"reasoning": "", **prueba.guion.pop(0)} if prueba.guion else texto("fin")

        for p in (mock.patch("agente.bucle.cliente.detectar", lambda cfg: ("http://falso", "modelo-x", 32768)),
                  mock.patch("agente.bucle.cliente.LLM", Falso), mock.patch("agente.bucle.entorno", lambda: "prueba")):
            p.start()
            self.addCleanup(p.stop)
        cfg = config.cargar()
        cfg["razonamiento"] = "auto"
        self.a = Agente(cfg, self.dir, interactivo=False)
        self.addCleanup(self.a.cerrar)
        self.a._revisar = lambda *a, **k: ("REVISIÓN: aprobado", True)

    def test_arreglo_pequeno_que_acaba_con_texto_sigue_igual(self):
        self.guion = [llamada("escribir", ruta="run.bat", contenido="@echo off\r\npy main.py\r\n"),
                      texto("Corregido: run.bat ya usa py.")]
        self.assertEqual(self.a.tarea("inicio el run bat y no funciona"), "Corregido: run.bat ya usa py.")
        self.assertFalse(any(e.startswith("RECHAZADO:") for e in self.enviados))  # sin plan pendiente: sin puerta

    def test_modelo_sin_herramientas_avisa_en_la_respuesta(self):
        self.a.llm.info = {"capabilities": {"trained_for_tool_use": False}}
        self.guion = [texto("Claro, aquí tienes una idea: usa Excel.")]
        r = self.a.tarea(ENCARGO)
        self.assertIn("no está entrenado para usar herramientas", r)
        self.assertIn("usa Excel.", r)
        self.a.llm.info = {"capabilities": {"trained_for_tool_use": True}}
        self.guion = [texto("Claro, aquí tienes una idea: usa Excel.")]
        self.assertNotIn("AVISO", self.a.tarea(ENCARGO))

    def test_acabar_con_texto_pasa_la_puerta_de_terminar(self):
        self.guion = [llamada("plan", pasos=["crear app.py", "probarlo"]),
                      llamada("escribir", ruta="app.py", contenido="print('hola')\n"),
                      texto("He completado todas las fases."), texto("He completado todas las fases."),
                      texto("He completado todas las fases."),
                      llamada("ejecutar", comando="python app.py"), llamada("paso_hecho", numero=1),
                      llamada("paso_hecho", numero=2), texto("Hecho: app.py imprime hola.")]
        r = self.a.tarea(ENCARGO)
        self.assertEqual(r, "Hecho: app.py imprime hola.")
        self.assertTrue(any(e.startswith("RECHAZADO:") and "Cuando esté, llama a terminar." in e for e in self.enviados))

    def test_metricas_dicen_el_modelo_y_la_ventana(self):
        self.guion = [texto("¿Qué necesitas?")]
        self.a.tarea("hola")
        filas = [json.loads(x) for x in (self.dir / ".agente" / "metricas.jsonl").read_text(encoding="utf-8").splitlines()]
        tarea = next(f for f in filas if f["tipo"] == "tarea")
        self.assertEqual((tarea["modelo"], tarea["ventana"]), ("modelo-x", 32768))


class VentanaPequena(unittest.TestCase):
    def llenar(self, ventana: int, tokens: int) -> Contexto:
        c = Contexto(ventana, "sistema", 100)
        c.chars_por_token = 1.0
        c.mensajes.append({"role": "user", "content": "x" * (tokens - 100 - len("sistema"))})
        return c

    def test_con_16k_compacta_al_75_por_ciento(self):
        self.assertFalse(self.llenar(16384, 11000).hace_falta())  # al 60 % (9.830) ya compactaba
        self.assertTrue(self.llenar(16384, 12500).hace_falta())

    def test_con_ventana_grande_sigue_al_60_por_ciento(self):
        self.assertTrue(self.llenar(65536, 40000).hace_falta())
        self.assertFalse(self.llenar(65536, 39000).hace_falta())


if __name__ == "__main__":
    unittest.main()
