"""Pruebas sin modelo: python -m unittest discover -s tests -v"""
import json
import math
import shutil
import struct
import sys
import tempfile
import unittest
from pathlib import Path
from typing import ClassVar
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from agente.bucle import archivos_pedidos, coaccionar, llamadas_en_texto
from agente.contexto import Contexto
from agente.herramientas import INSTALACIONES, MIN_INSTALACION, Herramientas, despachar, recortar
from agente.modelo import fraccion_expertos, guardar_config, leer_gguf


class TestHerramientas(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_test_"))
        self.h = Herramientas(self.dir, max_chars=3000, timeout=30)

    def tearDown(self):
        self.h.detener_todos()
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_ruta_fuera_bloqueada(self):
        self.assertIn("fuera del proyecto", despachar(self.h, "leer", {"ruta": "..\\..\\Windows\\win.ini"}))

    def test_escribir_valida_python(self):
        self.assertIn("Sintaxis OK", self.h.escribir("app/main.py", "print('hola')\n"))
        self.assertIn("ERROR DE SINTAXIS", self.h.escribir("malo.py", "def f(:\n"))
        self.assertIn("JSON INVÁLIDO", self.h.escribir("d.json", "{'a': 1}"))
        self.assertIn("AVISO", self.h.escribir("x.py", "def f():\n    # TODO implementar\n    pass\n"))

    def test_leer_por_bloques(self):
        self.h.escribir("n.txt", "\n".join(f"linea {i}" for i in range(1, 501)))
        r = self.h.leer("n.txt", desde=100, lineas=10)
        self.assertIn("100| linea 100", r)
        self.assertIn("desde=110", r)

    def test_editar_exacto_tolerante_y_ambiguo(self):
        self.h.escribir("a.py", "def f():\n    return 1\n\ndef g():\n    return 1\n")
        self.assertIn("aparece 2 veces", self.h.editar("a.py", "return 1", "return 2"))
        self.assertIn("OK", self.h.editar("a.py", "def g():\n    return 1", "def g():\n    return 3"))
        self.assertIn("OK", self.h.editar("a.py", "def f():\n  return 1", "def f():\n    return 5"))  # sangría distinta
        self.assertEqual((self.dir / "a.py").read_text(), "def f():\n    return 5\n\ndef g():\n    return 3\n")
        self.assertIn("No encontré", self.h.editar("a.py", "no existe", "x"))

    def test_ejecutar_codigo_y_timeout(self):
        self.assertTrue(self.h.ejecutar("echo hola").startswith("exit=0"))
        self.assertTrue(self.h.ejecutar("exit /b 3").startswith("exit=3"))
        r = self.h.ejecutar('python -c "import time; time.sleep(30)"', segundos=5)
        self.assertIn("TIMEOUT", r)

    def test_instalar_tiene_tiempo_minimo(self):
        for c in ("python -m venv .venv", ".venv\\Scripts\\pip.exe install pygame", "pip install -r requirements.txt",
                  "npm ci", "uv sync", "python -m playwright install chromium"):
            self.assertTrue(INSTALACIONES.search(c), c)
        for c in ("python main.py", "npm run build", "npm init -y", "echo pipeline"):
            self.assertFalse(INSTALACIONES.search(c), c)
        with mock.patch("agente.herramientas.subprocess.Popen") as popen:
            popen.return_value.wait.return_value = 0
            self.h.ejecutar("python -m pip install pygame", segundos=10)
            self.assertEqual(popen.return_value.wait.call_args.kwargs["timeout"], MIN_INSTALACION)
            self.h.ejecutar("python main.py", segundos=10)
            self.assertEqual(popen.return_value.wait.call_args.kwargs["timeout"], 10)

    def test_ejecutar_peligroso_bloqueado(self):
        self.assertIn("BLOQUEADO", self.h.ejecutar("rmdir /s /q C:\\algo"))

    def test_fondo_y_detener(self):
        r = self.h.ejecutar('python -u -c "import time; print(\'arranco\'); time.sleep(60)"', fondo=True)
        self.assertIn("en ejecución", r)
        self.assertIn("arranco", r)
        self.assertIn("detenido", self.h.procesos_cmd("detener", 1))

    def test_crear_bat_y_ejecutarlo(self):
        self.h.escribir("main.py", "import sys\nprint('ok 100%')\nsys.exit(int(sys.argv[1]) if len(sys.argv) > 1 else 0)\n")
        self.h.crear_bat("run", ["python main.py"], "Prueba")
        self.assertIn(b"\r\n", (self.dir / "run.bat").read_bytes())
        r = self.h.ejecutar("run.bat")
        self.assertTrue(r.startswith("exit=0"), r)
        self.assertIn("[OK] Terminado", r)
        self.h.crear_bat("falla", ["python main.py 4", "echo no debe llegar"])
        r = self.h.ejecutar("falla.bat")
        self.assertTrue(r.startswith("exit=4"), r)
        self.assertNotIn("no debe llegar\n", r.replace("echo no debe llegar", ""))

    def test_plan(self):
        self.h.plan(["crear", "probar"])
        self.assertIn("Siguiente: 2. probar", self.h.paso_hecho(1, "listo"))
        self.assertIn("RECHAZADO", self.h.paso_hecho("2"))  # "probar" exige una ejecución real
        self.h.ejecutar("echo ok")
        self.assertIn("Todos los pasos hechos", self.h.paso_hecho("2"))

    def test_recortar_conserva_final(self):
        r = recortar("A" * 5000 + "ERROR FINAL", 1000)
        self.assertTrue(r.endswith("ERROR FINAL"))
        self.assertLess(len(r), 1100)


class TestParseo(unittest.TestCase):
    def test_tool_call_xml_qwen(self):
        t = "<tool_call>\n<function=escribir>\n<parameter=ruta>\na.py\n</parameter>\n<parameter=contenido>\nprint(1)\n</parameter>\n</function>\n</tool_call>"
        c = llamadas_en_texto(t)[0]
        self.assertEqual(c["name"], "escribir")
        self.assertEqual(json.loads(c["arguments"]), {"ruta": "a.py", "contenido": "print(1)"})

    def test_tool_call_json_hermes(self):
        c = llamadas_en_texto('<tool_call>{"name": "listar", "arguments": {"ruta": "."}}</tool_call>')[0]
        self.assertEqual((c["name"], json.loads(c["arguments"])), ("listar", {"ruta": "."}))

    def test_archivos_pedidos(self):
        self.assertEqual(archivos_pedidos("Crea app.py y test.bat con node.js y README.md, ver python.org"),
                         {"app.py", "test.bat", "README.md"})

    def test_coaccionar(self):
        self.assertEqual(coaccionar("ejecutar", {"comando": "x", "fondo": "true", "segundos": "30", "raro": 1}),
                         {"comando": "x", "fondo": True, "segundos": 30})
        self.assertEqual(coaccionar("plan", {"pasos": "uno\ndos"})["pasos"], ["uno", "dos"])


class TestContexto(unittest.TestCase):
    def test_compactar_respeta_orden_y_presupuesto(self):
        class LLMFalso:
            def chat(self, *a, **k):
                return {"content": "- resumen de prueba"}
        c = Contexto(4000, "sistema", 100)
        c.usuario("crea una app")
        for i in range(12):
            c.asistente("", [{"id": f"t{i}", "name": "escribir",
                              "arguments": json.dumps({"ruta": f"f{i}.py", "contenido": "x = 1\n" * 300})}])
            c.herramienta(f"t{i}", "OK " + "y" * 2000)
        self.assertTrue(c.hace_falta())
        c.compactar(LLMFalso(), "- [ ] 1. probar")
        self.assertLess(c.tokens(), 4000 * 0.6)
        self.assertEqual(c.mensajes[1]["role"], "user")
        self.assertIn("resumen de prueba", c.mensajes[1]["content"])
        self.assertEqual(c.mensajes[2]["role"], "assistant")  # nunca un 'tool' huérfano
        ids = {tc["id"] for m in c.mensajes for tc in m.get("tool_calls") or []}
        self.assertTrue(all(m["tool_call_id"] in ids for m in c.mensajes if m["role"] == "tool"))


def gguf_falso(ruta: Path, tensores: list[tuple[str, int]], capas: int) -> None:
    def s(t):
        return struct.pack("<Q", len(t.encode())) + t.encode()
    kv = [s("general.architecture") + struct.pack("<I", 8) + s("prueba"),
          s("prueba.block_count") + struct.pack("<II", 4, capas),
          s("tokenizer.ggml.tokens") + struct.pack("<IIQ", 9, 8, 2) + s("a") + s("b"),
          s("tokenizer.ggml.scores") + struct.pack("<IIQff", 9, 6, 2, 0, 0)]
    cuerpo, offset = b"GGUF" + struct.pack("<IQQ", 3, len(tensores), len(kv)) + b"".join(kv), 0
    for nombre, tam in tensores:
        cuerpo += s(nombre) + struct.pack("<IQIQ", 1, tam, 0, offset)
        offset += tam
    ruta.write_bytes(cuerpo + b"\0" * (-len(cuerpo) % 32 + offset))


class TestModelo(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_test_"))

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_fraccion_expertos_da_las_capas_exactas(self):
        for capas in (24, 40, 48, 94):
            for n in range(capas + 1):
                self.assertEqual(math.ceil(fraccion_expertos(n, capas) * capas), n)

    def test_leer_gguf_expertos_por_capa(self):
        moe = self.dir / "moe.gguf"
        gguf_falso(moe, [("blk.0.attn_q.weight", 64), ("blk.0.ffn_up_exps.weight", 1000),
                         ("blk.0.ffn_down_exps.weight", 1000), ("blk.1.ffn_gate_exps.weight", 500),
                         ("blk.1.ffn_gate_shexp.weight", 96), ("output.weight", 32)], capas=2)
        self.assertEqual(leer_gguf(moe), (2, {0: 2000, 1: 500}))  # el experto compartido no cuenta
        denso = self.dir / "denso.gguf"
        gguf_falso(denso, [("blk.0.ffn_up.weight", 100), ("blk.1.ffn_up.weight", 100)], capas=2)
        self.assertEqual(leer_gguf(denso), (2, {}))
        (self.dir / "otro.gguf").write_bytes(b"no es gguf")
        self.assertEqual(leer_gguf(self.dir / "otro.gguf"), (0, {}))

    def test_guardar_config_conserva_campos_y_original(self):
        destino = self.dir / "m.gguf.json"
        destino.write_text(json.dumps({"preset": "", "load": {"fields": [{"key": "otro", "value": 1}]}}))
        self.assertTrue(guardar_config(destino, {"llm.load.llama.keepModelInMemory": False}))
        self.assertFalse(guardar_config(destino, {"llm.load.llama.keepModelInMemory": False}))
        self.assertTrue(guardar_config(destino, {"llm.load.numCpuExpertLayersRatio": 0.6875}))
        campos = {c["key"]: c["value"] for c in json.loads(destino.read_text())["load"]["fields"]}
        self.assertEqual(campos, {"otro": 1, "llm.load.llama.keepModelInMemory": False,
                                  "llm.load.numCpuExpertLayersRatio": 0.6875})
        self.assertNotIn("keepModelInMemory", destino.with_suffix(".json.bak").read_text())


class LLMGuion:
    """Modelo falso: devuelve en orden las respuestas preparadas."""
    respuestas: ClassVar[list] = []

    def __init__(self, base, modelo, contexto, cfg):
        self.base, self.modelo, self.contexto, self.cfg = base, modelo, contexto, cfg

    def chat(self, mensajes, herramientas, max_tokens, razonar, al_progreso=None):
        return LLMGuion.respuestas.pop(0)


def llamada(nombre: str, **args) -> dict:
    return {"content": "", "finish_reason": "tool_calls", "usage": {},
            "tool_calls": [{"id": nombre, "name": nombre, "arguments": json.dumps(args)}]}


class TestModoJson(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_test_"))

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_protocolo_eventos_y_confirmacion(self):
        import io

        from agente import __main__ as cli
        from agente import config
        (self.dir / "basura").mkdir()
        LLMGuion.respuestas = [
            llamada("escribir", ruta="notas.txt", contenido="hola\nhola\n"),
            llamada("editar", ruta="notas.txt", buscar="hola", reemplazar="adiós"),  # ambiguo: falla
            llamada("ejecutar", comando="rmdir /s /q basura"),
            {"content": "Listo, creé notas.txt.", "tool_calls": [], "finish_reason": "stop", "usage": {}},
        ]
        ordenes = [{"tipo": "tarea", "texto": "crea notas.txt"}, {"tipo": "respuesta", "si": False},
                   {"tipo": "plan"}, {"tipo": "salir"}]
        entrada = io.TextIOWrapper(io.BytesIO("".join(json.dumps(o) + "\n" for o in ordenes).encode()), encoding="utf-8")
        salida, real = io.StringIO(), sys.stdout
        falso = lambda cfg: ("http://falso", "falso", 32768)
        with mock.patch.object(cli, "detectar", falso), mock.patch("agente.bucle.cliente.detectar", falso), \
                mock.patch("agente.bucle.cliente.LLM", LLMGuion), mock.patch("agente.bucle.entorno", lambda: "prueba"), \
                mock.patch.object(sys, "stdin", entrada), mock.patch.object(sys, "stdout", salida):
            self.assertEqual(cli.modo_json(config.cargar(), str(self.dir)), 0)
            self.assertIs(sys.stdout, salida)  # restaura la salida al terminar
        self.assertIs(sys.stdout, real)
        eventos = [json.loads(linea) for linea in salida.getvalue().splitlines()]
        tipos = [e["tipo"] for e in eventos]
        self.assertEqual(tipos[0], "listo")
        self.assertEqual(eventos[0]["modelo"], "falso")
        pasos = [e for e in eventos if e["tipo"] == "paso"]
        self.assertEqual([p["herramienta"] for p in pasos], ["escribir", "editar", "ejecutar"])
        self.assertTrue(pasos[1]["error"] and "aparece 2 veces" in pasos[1]["resultado"])  # no es un éxito
        self.assertTrue(pasos[2]["error"] and "BLOQUEADO" in pasos[2]["resultado"])
        self.assertIn({"tipo": "archivos", "rutas": ["notas.txt"]}, eventos)
        self.assertIn({"tipo": "confirmar", "comando": "rmdir /s /q basura",
                       "motivo": "ejecutar un comando potencialmente destructivo"}, eventos)
        self.assertTrue((self.dir / "basura").is_dir())  # el usuario dijo que no
        fines = [e for e in eventos if e["tipo"] == "fin"]
        self.assertEqual([(f["orden"], f["estado"]) for f in fines], [("tarea", "ok"), ("plan", "ok")])
        self.assertEqual(fines[0]["respuesta"], "Listo, creé notas.txt.")
        self.assertEqual((self.dir / "notas.txt").read_text(), "hola\nhola\n")

    def test_ordenes_de_procesos_y_restaurar(self):
        import io

        from agente import __main__ as cli
        from agente import config
        ordenes = [{"tipo": "procesos"}, {"tipo": "detener", "id": 7}, {"tipo": "restaurar"}, {"tipo": "salir"}]
        entrada = io.TextIOWrapper(io.BytesIO("".join(json.dumps(o) + "\n" for o in ordenes).encode()), encoding="utf-8")
        salida = io.StringIO()
        falso = lambda cfg: ("http://falso", "falso", 32768)
        with mock.patch.object(cli, "detectar", falso), mock.patch("agente.bucle.cliente.detectar", falso), \
                mock.patch("agente.bucle.cliente.LLM", LLMGuion), mock.patch("agente.bucle.entorno", lambda: "prueba"), \
                mock.patch.object(sys, "stdin", entrada), mock.patch.object(sys, "stdout", salida):
            cli.modo_json(config.cargar(), str(self.dir))
        eventos = [json.loads(linea) for linea in salida.getvalue().splitlines()]
        fines = {e["orden"]: e for e in eventos if e["tipo"] == "fin"}
        self.assertEqual(fines["procesos"]["datos"], [])
        self.assertIn("No existe el proceso #7", fines["detener"]["respuesta"])
        self.assertIn("No hay checkpoints", fines["restaurar"]["respuesta"])
        self.assertIn({"tipo": "archivos", "rutas": []}, eventos)  # el IDE refresca el árbol


class TestBat(unittest.TestCase):
    def test_bats_del_agente_existen(self):
        raiz = Path(__file__).resolve().parent.parent
        for b in ("instalar.bat", "agente.bat", "cargar_modelo.bat", "probar.bat"):
            self.assertTrue((raiz / b).exists(), b)


if __name__ == "__main__":
    unittest.main()
