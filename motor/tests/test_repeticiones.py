"""La válvula anti-bucle: verificar tras un comando nuevo (copiar, instalar) es progreso; repetir lo mismo sin que
cambie nada sigue deteniendo la tarea. Y los archivos pedidos con varios puntos (tauri.conf.json)."""
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
from agente.herramientas import Herramientas, archivos_pedidos

LISTO = {"content": "Listo.", "tool_calls": [], "finish_reason": "stop", "usage": {}}


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_rep_"))

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def mision(self, respuestas: list, texto: str = "haz la tarea") -> list[dict]:
        LLMGuion.respuestas = respuestas
        ordenes = [{"tipo": "tarea", "texto": texto}, {"tipo": "salir"}]
        entrada = io.TextIOWrapper(io.BytesIO("".join(json.dumps(o) + "\n" for o in ordenes).encode()), encoding="utf-8")
        salida = io.StringIO()
        falso = lambda cfg: ("http://falso", "falso", 32768)
        with mock.patch.object(cli, "detectar", falso), mock.patch("agente.bucle.cliente.detectar", falso), \
                mock.patch("agente.bucle.cliente.LLM", LLMGuion), mock.patch("agente.bucle.entorno", lambda: "prueba"), \
                mock.patch.object(sys, "stdin", entrada), mock.patch.object(sys, "stdout", salida):
            cli.modo_json(config.cargar(), str(self.dir))
        return [json.loads(x) for x in salida.getvalue().splitlines() if x.strip()]

    @staticmethod
    def fin(eventos: list[dict]) -> str:
        return next(e["respuesta"] for e in eventos if e["tipo"] == "fin")


class Repeticiones(Base):
    def test_verificar_tras_comandos_nuevos_no_detiene(self):
        """El caso real (Aura-Monitor): verificar, copiar un archivo, instalar, verificar... no es un bucle."""
        guion = [llamada("escribir", ruta="a.txt", contenido="x\n")]
        for cmd in ("echo uno", "echo dos", "echo tres"):
            guion += [llamada("verificar"), llamada("ejecutar", comando=cmd)]
        guion += [llamada("verificar"), llamada("leer", ruta="a.txt"), llamada("verificar"), LISTO]
        eventos = self.mision(guion)
        self.assertEqual(self.fin(eventos), "Listo.")
        self.assertFalse([e for e in eventos if e["tipo"] == "aviso" and "Detenido" in e["texto"]])

    def test_verificar_cinco_veces_sin_cambiar_nada_detiene(self):
        eventos = self.mision([llamada("escribir", ruta="a.txt", contenido="x\n"),
                               *[llamada("verificar") for _ in range(5)], LISTO])
        self.assertIn("repitió 'verificar' 5 veces", self.fin(eventos))

    def test_el_mismo_comando_cinco_veces_detiene(self):
        eventos = self.mision([llamada("ejecutar", comando="echo igual") for _ in range(5)] + [LISTO])
        self.assertIn("repitió 'ejecutar' 5 veces", self.fin(eventos))


class NombresConVariosPuntos(Base):
    def test_extrae_el_nombre_entero(self):
        self.assertEqual(archivos_pedidos("Escribir src-tauri/tauri.conf.json configuración básica"), {"tauri.conf.json"})
        self.assertEqual(archivos_pedidos("crea vite.config.ts y main.ts"), {"vite.config.ts", "main.ts"})
        self.assertEqual(archivos_pedidos("usa three.js o three.module.js desde la CDN"), set())

    def test_el_paso_de_tauri_conf_se_marca_cuando_existe(self):
        h = Herramientas(self.dir, max_chars=8000, timeout=30)
        h.plan(["Escribir src-tauri/tauri.conf.json configuración básica"])
        self.assertIn("pide tauri.conf.json", h.paso_hecho(1))
        h.escribir("src-tauri/tauri.conf.json", '{"productName": "x"}')
        self.assertIn("marcado como hecho", h.paso_hecho(1))  # antes pedía «conf.json» para siempre


if __name__ == "__main__":
    unittest.main()
