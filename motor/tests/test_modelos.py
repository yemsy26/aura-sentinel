"""El agente con cualquier modelo: sin candado en agente.json, se adapta a lo que dice la ficha del modelo (razonamiento,
herramientas, contexto), entiende las llamadas escritas por otras familias y «Probar modelo» dice si sirve y por qué.
Antes solo funcionaba bien con Qwen3.6: los demás modelos daban problemas."""
import io
import json
import sys
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from typing import ClassVar
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from agente import __main__ as cli
from agente import compat, config, llm, prueba_modelo
from agente import modelo as cargador
from agente.bucle import llamadas_en_texto, sin_llamadas

QWEN = {"key": "qwen3.6-35b-a3b", "type": "llm", "display_name": "Qwen3.6", "size_bytes": 17 * 2**30,
        "max_context_length": 262144, "architecture": "qwen35moe", "quantization": {"name": "Q3_K_XL"},
        "capabilities": {"trained_for_tool_use": True, "vision": True, "reasoning": {"allowed_options": ["off", "on"]}},
        "loaded_instances": []}
FLINT = {**QWEN, "key": "recursivesi/flint", "display_name": "Flint", "architecture": "flint",
         "quantization": {"name": "Q4_K_M"}, "capabilities": {"trained_for_tool_use": True, "reasoning": {"allowed_options": ["on"]}}}
BONSAI = {**QWEN, "key": "prism-ml/bonsai-27b", "display_name": "Bonsai 27B", "size_bytes": 4 * 2**30,
          "quantization": {"name": "Q1_0"}}


def cargado(m: dict, contexto: int = 32768) -> dict:
    return {**m, "loaded_instances": [{"id": m["key"], "config": {"context_length": contexto}}]}


class SinCandado(unittest.TestCase):
    def test_agente_json_usa_el_modelo_cargado(self):
        self.assertEqual(json.loads(config.ARCHIVO_CONFIG.read_text(encoding="utf-8"))["modelo"], "auto")
        self.assertEqual(config.recomendado({"modelo": "auto"}), config.RECOMENDADO)
        self.assertEqual(config.recomendado({"modelo": "otro"}), "otro")

    def test_en_auto_detecta_cualquier_modelo_cargado(self):
        with mock.patch.object(llm, "listar_modelos", return_value=[QWEN, cargado(BONSAI, 20480)]):
            self.assertEqual(llm.detectar({"servidor": "http://x", "modelo": "auto"}), ("http://x", BONSAI["key"], 20480))
            # con una clave fija, el que hay cargado no sirve (era el candado: agente.json traía la de Qwen)
            self.assertIsNone(llm.detectar({"servidor": "http://x", "modelo": QWEN["key"]})[1])

    def test_sin_ninguno_cargado_ni_pedido_elige_el_recomendado_no_el_primero(self):
        with mock.patch.object(cargador, "listar_modelos", return_value=[FLINT, BONSAI, QWEN]), \
                mock.patch.object(sys.stdin, "isatty", return_value=False):
            self.assertEqual(cargador.elegir("http://x", {"modelo": "auto"}, None)["key"], QWEN["key"])


class Razonamiento(unittest.TestCase):
    def cuerpo(self, info: dict, razonar: bool) -> dict:
        enviado = {}

        class Respuesta(io.BytesIO):
            def __enter__(self):
                return self

            def __exit__(self, *a):
                return False

        def falso(req, timeout=0):
            enviado.update(json.loads(req.data))
            return Respuesta(b'data: {"choices":[{"delta":{"content":"hola"},"finish_reason":"stop"}]}\ndata: [DONE]\n')

        cfg = {"muestreo": {}}
        with mock.patch.object(llm.urllib.request, "urlopen", falso):
            llm.LLM("http://x", "m", 32768, cfg, info).chat([{"role": "user", "content": "x"}], None, 10, razonar)
        return enviado

    def test_solo_se_apaga_si_el_modelo_lo_permite(self):
        self.assertEqual(self.cuerpo(QWEN, False).get("reasoning_effort"), "none")
        self.assertNotIn("reasoning_effort", self.cuerpo(FLINT, False))  # razona siempre: pedírselo daba problemas
        self.assertNotIn("reasoning_effort", self.cuerpo(QWEN, True))
        self.assertEqual(self.cuerpo({}, False).get("reasoning_effort"), "none")  # sin ficha: como antes


class ContextoMinimo(unittest.TestCase):
    CFG: ClassVar[dict] = {"servidor": "auto", "modelo": "auto", "carga": {"contexto_min": 16384}}

    def preparar(self, detectado, info=None):
        avisos = []
        with mock.patch.object(cargador, "ajustar") as ajustar, mock.patch.object(cargador, "info_modelo", return_value=info or {}):
            cargador.preparar(dict(self.CFG), avisos.append, lambda cfg: detectado)
        return ajustar, avisos

    def test_con_contexto_suficiente_no_toca_nada(self):
        ajustar, avisos = self.preparar(("http://x", "m", 32768))
        self.assertFalse(ajustar.called)
        self.assertEqual(avisos, [])

    def test_cargado_a_mano_con_4096_se_recarga_el_mismo_con_mas(self):
        ajustar, avisos = self.preparar(("http://x", BONSAI["key"], 4096), cargado(BONSAI, 4096))
        self.assertEqual(ajustar.call_args.args[1], BONSAI["key"])
        self.assertIn("está cargado con 4096 tokens", avisos[0])

    def test_si_el_modelo_no_admite_tanto_lo_avisa(self):
        ajustar, avisos = self.preparar(("http://x", "m", 8192), {**BONSAI, "max_context_length": 8192})
        self.assertFalse(ajustar.called)
        self.assertIn("admite como mucho 8192", avisos[0])

    def test_sin_ficha_solo_avisa_y_sin_modelo_lo_carga(self):
        ajustar, avisos = self.preparar(("http://x", "m", 4096), None)
        self.assertFalse(ajustar.called)
        self.assertTrue(avisos)
        ajustar, _ = self.preparar(("http://x", None, 0))
        self.assertTrue(ajustar.called)


class Avisos(unittest.TestCase):
    def test_bits_de_la_cuantizacion(self):
        for cuant, bits in (("Q1_0", 1), ("IQ2_XXS", 2), ("Q3_K_XL", 3), ("Q4_K_M", 4), ("Q8_0", 8), ("F16", 16),
                            ("BF16", 16), ("MXFP4", 4)):
            self.assertEqual(compat.bits(cuant), bits, cuant)
        self.assertIsNone(compat.bits(""))

    def test_lo_que_limita_a_cada_modelo(self):
        self.assertEqual(compat.avisos(QWEN, 65536, 16384), [])
        self.assertIn("razona siempre", " ".join(compat.avisos(FLINT, 65536, 16384)))
        self.assertIn("comprimido a ~1 bits", " ".join(compat.avisos(BONSAI, 65536, 16384)))
        sin = {**QWEN, "capabilities": {"trained_for_tool_use": False}}
        self.assertIn("no está entrenado para usar herramientas", " ".join(compat.avisos(sin, 65536, 16384)))
        self.assertIn("8192 tokens", " ".join(compat.avisos(QWEN, 8192, 16384)))


class FormatosDeLlamada(unittest.TestCase):
    def una(self, texto: str) -> tuple[str, dict]:
        c = llamadas_en_texto(texto)
        self.assertEqual(len(c), 1, texto)
        return c[0]["name"], json.loads(c[0]["arguments"])

    def test_los_de_cada_familia(self):
        esperado = ("escribir", {"ruta": "a.py", "contenido": "print(1)"})
        for texto in (
            '[TOOL_CALLS][{"name": "escribir", "arguments": {"ruta": "a.py", "contenido": "print(1)"}}]',  # Mistral
            '[TOOL_CALLS]escribir[ARGS]{"ruta": "a.py", "contenido": "print(1)"}',  # Mistral (nuevo)
            '<|python_tag|>{"name": "escribir", "parameters": {"ruta": "a.py", "contenido": "print(1)"}}',  # Llama
            'Voy a crearlo.\n```json\n{"name": "escribir", "arguments": {"ruta": "a.py", "contenido": "print(1)"}}\n```',
            '{"name": "escribir", "arguments": "{\\"ruta\\": \\"a.py\\", \\"contenido\\": \\"print(1)\\"}"}',  # JSON suelto
            '```tool_code\nprint(default_api.escribir(ruta="a.py", contenido="print(1)"))\n```',  # Gemma
            '<function=escribir>{"ruta": "a.py", "contenido": "print(1)"}</function>',  # Llama 3.1
            '{"function": {"name": "escribir", "arguments": {"ruta": "a.py", "contenido": "print(1)"}}}',
        ):
            self.assertEqual(self.una(texto), esperado, texto)

    def test_varias_seguidas(self):
        c = llamadas_en_texto('[TOOL_CALLS]listar[ARGS]{"ruta": "."}leer[ARGS]{"ruta": "a.py"}')
        self.assertEqual([x["name"] for x in c], ["listar", "leer"])

    def test_json_o_codigo_que_no_son_llamadas_se_ignoran(self):
        for texto in ('```json\n{"name": "mi-app", "version": "1.0.0"}\n```', '{"name": "borrar_todo", "arguments": {}}',
                      '```python\nprint(sumar(2, 3))\n```', 'El JSON es {"a": 1}', "Listo, todo hecho."):
            self.assertEqual(llamadas_en_texto(texto), [], texto)

    def test_el_texto_queda_sin_la_llamada(self):
        self.assertEqual(sin_llamadas('Creo el archivo. [TOOL_CALLS][{"name": "listar", "arguments": {}}]'), "Creo el archivo.")
        self.assertEqual(sin_llamadas('{"name": "listar", "arguments": {"ruta": "."}}'), "")


class ProbarModelo(unittest.TestCase):
    CFG: ClassVar[dict] = {"servidor": "auto", "modelo": "auto", "muestreo": {}, "carga": {"contexto_min": 16384}}

    def probar(self, ficha: dict, respuestas: list, contexto: int = 32768, tokens: int = 40, segundos=2.0, ram: int = 10000):
        """segundos: lo que tarda cada respuesta (una lista, una por respuesta)."""
        reloj = [0.0]
        duracion = segundos if isinstance(segundos, list) else [segundos]

        class Falso:
            def __init__(self, *a):
                self.n = 0

            def chat(self, mensajes, herramientas, max_tokens, razonar, al_progreso=None):
                r = respuestas[min(self.n, len(respuestas) - 1)]
                reloj[0] += duracion[min(self.n, len(duracion) - 1)]
                self.n += 1
                return {"content": r.get("content", ""), "tool_calls": r.get("tool_calls", []), "usage": {"completion_tokens": tokens}}

        with mock.patch.object(prueba_modelo, "detectar", return_value=("http://x", ficha["key"], contexto)), \
                mock.patch.object(prueba_modelo, "listar_modelos", return_value=[ficha]), \
                mock.patch.object(prueba_modelo.cargador, "preparar"), mock.patch.object(prueba_modelo, "LLM", Falso), \
                mock.patch.object(prueba_modelo, "info_modelo", return_value=ficha), \
                mock.patch.object(prueba_modelo.cargador, "vram", return_value=(0, 8188)), \
                mock.patch.object(prueba_modelo, "ram_libre_mib", return_value=ram), \
                mock.patch.object(prueba_modelo.time, "time", lambda: reloj[0]):
            return prueba_modelo.probar(dict(self.CFG), ficha["key"], avisar=lambda t: None)

    ESCRIBIR: ClassVar[dict] = {"tool_calls": [{"id": "1", "name": "escribir", "arguments": '{"ruta": "hola.txt", "contenido": "hola mundo"}'}]}
    TERMINAR: ClassVar[dict] = {"tool_calls": [{"id": "2", "name": "terminar", "arguments": '{"resumen": "hecho"}'}]}

    def test_un_modelo_que_va_bien_es_apto(self):
        r = self.probar(QWEN, [self.ESCRIBIR, self.TERMINAR], contexto=65536)
        self.assertEqual(r["veredicto"], "apto", r)
        self.assertEqual(r["detalles"]["formato"], "nativo")
        self.assertIn("no cabe entero en la GPU", r["detalles"]["memoria"])

    def test_el_que_escribe_las_llamadas_como_texto_tiene_limites(self):
        texto = {"content": '<|python_tag|>{"name": "escribir", "parameters": {"ruta": "hola.txt", "contenido": "hola mundo"}}'}
        r = self.probar(QWEN, [texto, self.TERMINAR], contexto=65536)
        self.assertEqual(r["veredicto"], "apto con límites")
        self.assertTrue(any("como texto" in x for x in r["limites"]))

    def test_el_que_no_usa_herramientas_no_es_apto(self):
        r = self.probar(BONSAI, [{"content": "Claro, aquí tienes el archivo hola.txt: hola mundo"}])
        self.assertEqual(r["veredicto"], "no apto")
        self.assertIn("respondió sin llamarla", r["motivos"][0])

    def test_lento_y_comprimido_se_dicen(self):
        r = self.probar(BONSAI, [self.ESCRIBIR, self.TERMINAR], tokens=20, segundos=10)
        self.assertEqual(r["veredicto"], "apto con límites")
        texto = " ".join(r["limites"])
        self.assertIn("lento en este equipo", texto)
        self.assertIn("comprimido a ~1 bits", texto)

    def test_la_memoria_se_mide_con_el_modelo_ya_cargado(self):
        # antes de cargar Qwen, Flint ocupaba la RAM (0,3 GB libres) y se dijo que Qwen no cabía; cargó y fue bien
        r = self.probar(QWEN, [self.ESCRIBIR, self.TERMINAR], contexto=65536, ram=300)
        self.assertEqual(r["veredicto"], "apto", r)
        self.assertIn("es MoE", r["detalles"]["memoria"])

    def test_la_velocidad_es_la_de_la_segunda_respuesta(self):
        # la primera tras cargar aún lee el modelo del disco (Flint: 34 s); la segunda va al ritmo de una misión
        r = self.probar(QWEN, [self.ESCRIBIR, self.TERMINAR], contexto=65536, segundos=[34, 2])
        self.assertEqual(r["veredicto"], "apto", r)
        self.assertEqual((r["detalles"]["respuesta_s"], r["detalles"]["tokens_s"]), (2, 20))

    def test_no_descargado(self):
        with mock.patch.object(prueba_modelo, "detectar", return_value=("http://x", None, 0)), \
                mock.patch.object(prueba_modelo, "listar_modelos", return_value=[QWEN]):
            self.assertEqual(prueba_modelo.probar(dict(self.CFG), "no-existe")["veredicto"], "no apto")

    def test_la_ultima_linea_es_el_json_que_lee_aura(self):
        salida = io.StringIO()
        with mock.patch("agente.prueba_modelo.probar", return_value={"modelo": "m", "veredicto": "apto", "motivos": [],
                                                                       "limites": [], "detalles": {}}), redirect_stdout(salida):
            self.assertEqual(cli.probar_modelo(dict(self.CFG), "m"), 0)
        self.assertEqual(json.loads(salida.getvalue().strip().splitlines()[-1])["veredicto"], "apto")


if __name__ == "__main__":
    unittest.main()
