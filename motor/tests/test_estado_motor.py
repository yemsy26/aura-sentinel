"""C13 (m4185 tienda_roles): el modelo no puede escribir, borrar ni falsificar el estado del motor en .agente/ (revisión, registro,
plan, puntos de restauración, métricas). Antes editó revision.json, lo borró y vació registro.log con `echo [] >` hasta dejar
{"aprobado": true}; terminar pasó con una revisión que pedía cambios."""
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_agente import llamada, sin_lecciones_reales
from test_terminar_revision import APROBADO, CAMBIOS, FIN, LLMLento

from agente import config
from agente.estado_motor import ESTADO, es_estado, toca_estado
from agente.herramientas import Herramientas

setUpModule = sin_lecciones_reales

# Las formas exactas de tienda_roles (salida.txt, pasos 174-184) y variantes.
TIENDA = [r"echo [] > .agente\registro.log", r"echo {} > .agente\revision.json", r"del .agente\revision.json",
          r"del /q .agente\registro.log",
          r"del /q .agente\revision.json && del /q .agente\plan.md && echo [] > .agente\metricas.jsonl"]
VARIANTES = [r"echo x >> .agente\registro.log", r"type nul > .agente\registro.log", r"copy NUL .agente\checkpoints.json",
             r"move .agente\plan.md plan_vieja.md", r"ren .agente\revision.json x.json",
             r"powershell Remove-Item .agente\revision.json", """powershell -c "Set-Content .agente/revision.json '{}'" """,
             """powershell "'{}' | Out-File .agente\\revision.json" """, r"powershell Clear-Content .agente\registro.log",
             """python -c "open('.agente/revision.json','w').write('{}')" """,
             """python -c "import pathlib; pathlib.Path('.agente/revision.json').unlink()" """,
             r"rd /s /q .agente", r"rmdir /s /q .agente", r"del .agente\*", r'del /q "C:\x\y\.agente\revision.json"',
             r"del C:\Users\yo\Proyectos\x\.agente\registro.log",
             """node -e "require('fs').writeFileSync('.agente/revision.json','{}')" """,
             r'echo [] > ".agente\registro.log"', r"echo {} > .AGENTE\REVISION.JSON",
             r"Remove-Item -Recurse -Force .agente", r"del .agente\*.json"]
LEGITIMOS = [r"type .agente\registro.log", r"findstr /i error .agente\registro.log", "cat .agente/plan.md",
             r"python .agente/experimento/prueba.py", r"dir /s /b .agente\experimento\*.py 2>nul",
             r"solana-keygen new -o .agente\localnet.json --no-bip39-passphrase --silent",
             r"echo hola > .agente\experimento\a.txt", r"del .agente\experimento\a.txt", r"del .agente\experimento\*.py",
             r"rd /s /q .agente\experimento", "git status", """python -c "print(open('.agente/registro.log').read()[:100])" """,
             """python -c "print(open('.agente/registro.log', 'r').read())" """, r"copy .agente\capturas\a.png fuera.png",
             r"mkdir .agente\tmp", "echo agente > notas.txt", r"del business.db 2>nul && del .agente\test_init*.py 2>nul && echo Limpio", r"type .agente\registro.log > copia.txt", "pytest tests"]


class TestFiltro(unittest.TestCase):
    def test_las_formas_de_tienda_roles_se_detectan(self):
        for c in TIENDA:
            with self.subTest(c=c):
                self.assertIsNotNone(toca_estado(c))

    def test_las_variantes_se_detectan(self):
        for c in VARIANTES:
            with self.subTest(c=c):
                self.assertIsNotNone(toca_estado(c))

    def test_leer_y_usar_las_subcarpetas_no_se_toca(self):
        for c in LEGITIMOS:
            with self.subTest(c=c):
                self.assertIsNone(toca_estado(c))


class TestHerramientas(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_estado_"))
        self.h = Herramientas(self.dir)
        self.h.dir_agente.mkdir(exist_ok=True)

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_escribir_y_editar_rechazan_cada_archivo_de_estado_sin_tocarlo(self):
        for nombre in ESTADO:
            with self.subTest(nombre=nombre):
                f = self.h.dir_agente / nombre
                f.write_text("original", encoding="utf-8")
                for r in (self.h.escribir(f".agente/{nombre}", "x"), self.h.escribir(f".agente/{nombre}", "x", anadir=True),
                          self.h.editar(f".agente/{nombre}", "original", "falsificado")):
                    self.assertTrue(r.startswith("BLOQUEADO"), r)
                self.assertEqual(f.read_text(encoding="utf-8"), "original")

    def test_mayusculas_no_se_cuelan(self):
        (self.h.dir_agente / "revision.json").write_text("original", encoding="utf-8")
        self.assertTrue(self.h.escribir(".AGENTE/Revision.JSON", "x").startswith("BLOQUEADO"))
        self.assertTrue(es_estado(self.h.ruta(".AGENTE/REVISION.JSON"), self.h.dir_agente))

    def test_lo_que_no_es_del_motor_se_escribe_como_siempre(self):
        self.assertTrue(self.h.escribir(".agente/localnet.json", '{"clave": "de prueba", "red": "local"}').startswith("OK creado"))
        self.assertTrue(self.h.escribir(".agente/experimento/x.py", "print(1)\n").startswith("OK creado"))
        self.assertTrue(self.h.editar(".agente/experimento/x.py", "print(1)", "print(2)").startswith("OK editado"))
        self.assertTrue(self.h.escribir("revision.json", "{}").startswith("OK creado"))  # el del proyecto, no el del motor

    def test_ejecutar_no_vacia_el_registro(self):
        f = self.h.dir_agente / "registro.log"
        f.write_text("original", encoding="utf-8")
        for c in (r"echo [] > .agente\registro.log", r"del .agente\registro.log"):
            self.assertTrue(self.h.ejecutar(c).startswith("BLOQUEADO"), c)
        self.assertEqual(f.read_text(encoding="utf-8"), "original")

    def test_ejecutar_deja_usar_las_subcarpetas(self):
        (self.h.dir_agente / "experimento").mkdir()
        r = self.h.ejecutar(r"echo hola > .agente\experimento\a.txt")
        self.assertTrue(r.startswith("exit=0"), r)

    def test_experimentar_y_crear_bat_tampoco(self):
        f = self.h.dir_agente / "revision.json"
        f.write_text("original", encoding="utf-8")
        self.assertTrue(self.h.experimentar("open('.agente/revision.json', 'w').write('{}')").startswith("BLOQUEADO"))
        r = self.h.crear_bat("limpiar", [r"echo {} > .agente\revision.json"])
        self.assertTrue(r.startswith("BLOQUEADO") and "No he creado limpiar.bat" in r, r)
        self.assertFalse((self.dir / "limpiar.bat").exists())
        self.assertEqual(f.read_text(encoding="utf-8"), "original")


class TestBucle(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_estado_"))
        falso = lambda cfg: ("http://falso", "falso", 32768)
        self.parches = [mock.patch("agente.bucle.cliente.detectar", falso), mock.patch("agente.bucle.cliente.LLM", LLMLento),
                        mock.patch("agente.bucle.entorno", lambda: "prueba")]
        for p in self.parches:
            p.start()

    def tearDown(self):
        for p in self.parches:
            p.stop()
        shutil.rmtree(self.dir, ignore_errors=True)

    def agente(self, respuestas):
        from agente.bucle import Agente
        LLMLento.respuestas = respuestas
        return Agente(config.cargar(), self.dir, interactivo=False)

    def terminar_de(self, a):
        ids = {c["id"] for m in a.ctx.mensajes if m["role"] == "assistant" for c in m.get("tool_calls") or []
               if c["function"]["name"] == "terminar"}
        return [m["content"] for m in a.ctx.mensajes if m["role"] == "tool" and m["tool_call_id"] in ids]

    def registro(self):
        return (self.dir / ".agente" / "registro.log").read_text(encoding="utf-8")

    def mision(self, tocar):
        """Revisión que pide cambios → el modelo toca revision.json con `tocar` (un comando que el filtro NO ve) → terminar."""
        a = self.agente([llamada("escribir", ruta="main.py", contenido="print('adiós')\n"), llamada("revisar"), CAMBIOS,
                         llamada("ejecutar", comando=tocar), llamada("terminar", resumen="Listo. Revisión: APROBADO."), FIN])
        a.tarea("haz un programa que salude")
        self.assertEqual(LLMLento.respuestas, [])
        return a

    def test_borrar_revision_no_deja_terminar(self):
        a = self.mision("""python -c "import os; d='.agen'+'te'; os.remove(os.path.join(d, 'revision.json'))" """)
        r = self.terminar_de(a)
        self.assertEqual(len(r), 1)
        self.assertTrue(r[0].startswith("RECHAZADO: la última revisión pidió cambios"), r[0])
        self.assertIn("[CRITICO] main.py:1 - imprime adiós", r[0])
        self.assertFalse(json.loads((self.dir / ".agente" / "revision.json").read_text(encoding="utf-8"))["aprobado"])
        self.assertIn("MANIPULACIÓN: .agente/revision.json no es el que escribió el motor (borrado)", self.registro())

    def test_falsificar_revision_aprobada_no_deja_terminar(self):
        falsa = ("""python -c "import json,os; d='.agen'+'te'; json.dump({'aprobado': True, 'fecha': 0, 'texto': ''}, """
                 """open(os.path.join(d, 'revision.json'), 'w'))" """)
        a = self.mision(falsa)
        r = self.terminar_de(a)
        self.assertTrue(r[0].startswith("RECHAZADO: la última revisión pidió cambios"), r[0])
        self.assertFalse(json.loads((self.dir / ".agente" / "revision.json").read_text(encoding="utf-8"))["aprobado"])
        self.assertIn("(cambiado)", self.registro())

    def test_el_comando_que_toca_revision_ni_se_ejecuta(self):
        a = self.mision(r"echo {} > .agente\revision.json")
        resultados = [m["content"] for m in a.ctx.mensajes if m["role"] == "tool" and m["content"].startswith("BLOQUEADO")]
        self.assertEqual(len(resultados), 1)
        self.assertIn("estado interno del motor", resultados[0])
        self.assertTrue(self.terminar_de(a)[0].startswith("RECHAZADO: la última revisión pidió cambios"))
        self.assertNotIn("MANIPULACIÓN", self.registro())  # no llegó a tocar nada

    def test_una_revision_normal_no_da_manipulacion(self):
        a = self.agente([llamada("escribir", ruta="main.py", contenido="print('hola')\n"), llamada("revisar"), APROBADO,
                         llamada("ejecutar", comando="python main.py"), llamada("terminar", resumen="Saluda."), FIN])
        a.tarea("haz un programa que salude")
        self.assertEqual(self.terminar_de(a), ["Tarea terminada."])
        self.assertNotIn("MANIPULACIÓN", self.registro())

    def test_registro_vaciado_deja_rastro(self):
        a = self.agente([])
        a._log("uno")
        a._log("dos con texto")
        a.registro.write_text("", encoding="utf-8")
        a._log("tres")
        texto = self.registro()
        self.assertIn("MANIPULACIÓN: este registro tenía", texto)
        self.assertTrue(texto.rstrip().endswith("] tres"))
        a._log("cuatro")
        self.assertEqual(self.registro().count("MANIPULACIÓN"), 1)


if __name__ == "__main__":
    unittest.main()
