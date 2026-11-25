"""C20 (m4189): lo que el agente hace con la herramienta git entra en los comandos que ve el revisor. Antes solo ejecutar, experimentar
y los procesos anotaban evidencia: en git_conflicto el revisor pedía «pruebas» de un merge que sí se había hecho (4, 3 y 4 terminar
rechazados por la revisión)."""
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import revision
from agente.herramientas import Herramientas

HAY_GIT = shutil.which("git") is not None
IDENTIDAD = {"GIT_AUTHOR_NAME": "Prueba", "GIT_AUTHOR_EMAIL": "prueba@local", "GIT_COMMITTER_NAME": "Prueba",
             "GIT_COMMITTER_EMAIL": "prueba@local"}


class Modelo:
    contexto = 32768

    def __init__(self):
        self.mensajes = []

    def chat(self, mensajes, herramientas, max_tokens, razonar, al_progreso=None):
        self.mensajes.append(mensajes)
        return {"content": "REQUISITOS:\n- merge -> CUMPLE\nPROBLEMAS:\n- ninguno\nVEREDICTO: APROBADO", "tool_calls": [],
                "finish_reason": "stop", "usage": {}}


@unittest.skipUnless(HAY_GIT, "sin git")
class TestEvidenciaGit(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_evgit_"))
        self.env = mock.patch.dict(os.environ, IDENTIDAD)
        self.env.start()
        self.h = Herramientas(self.dir)
        self.h.git("otro", comando="init -b main")
        (self.dir / "main.py").write_text("print(1)\n", encoding="utf-8")

    def tearDown(self):
        self.env.stop()
        shutil.rmtree(self.dir, ignore_errors=True)

    def evidencia(self) -> list[str]:
        return list(self.h.evidencia)

    def test_commit_merge_y_estado_quedan_como_evidencia(self):
        self.h.git("commit", mensaje="base")
        self.h.git("otro", comando="checkout -b ajustes")
        (self.dir / "main.py").write_text("print(2)\n", encoding="utf-8")
        self.h.git("commit", mensaje="ajustes")
        self.h.git("otro", comando="checkout main")
        self.h.git("otro", comando="git merge ajustes --no-edit")  # con el «git» delante también
        self.h.git("estado")
        texto = self.h.evidencia_texto()
        self.assertIn("git commit «base» -> exit=0: [main (root-commit)", texto)
        self.assertIn("git checkout -b ajustes -> exit=0", texto)
        self.assertIn("git merge ajustes --no-edit -> exit=0", texto)
        self.assertIn("git estado -> exit=0: ## main", texto)

    def test_los_fallos_tambien_y_con_su_exit(self):
        self.h.git("otro", comando="merge no_existe")
        self.assertTrue(any(e.startswith("git merge no_existe -> exit=1") for e in self.evidencia()), self.evidencia())

    def test_estado_y_log_solo_dejan_la_ultima_y_diff_no_se_anota(self):
        self.h.git("commit", mensaje="base")
        for _ in range(5):
            self.h.git("estado")
            self.h.git("log")
            self.h.git("diff")
        e = self.evidencia()
        self.assertEqual(sum(x.startswith("git estado ->") for x in e), 1, e)
        self.assertEqual(sum(x.startswith("git log ->") for x in e), 1, e)
        self.assertFalse(any(x.startswith("git diff") for x in e), e)

    def test_lo_que_no_llega_a_ejecutarse_no_se_anota(self):
        antes = len(self.evidencia())
        self.assertTrue(self.h.git("otro", comando="push origin main").startswith("BLOQUEADO"))
        self.assertTrue(self.h.git("commit", mensaje="  ").startswith("Indica 'mensaje'"))
        self.assertTrue(self.h.git("otro", comando="").startswith("Indica 'comando'"))
        self.assertEqual(len(self.evidencia()), antes)

    def test_el_rechazo_de_un_merge_incompleto_no_cuenta_como_commit(self):
        # un merge con conflicto resuelto perdiendo líneas del otro lado: el commit se rechaza (C16) y no se ejecuta
        (self.dir / "c.txt").write_text("a\nb\nc\n", encoding="utf-8")
        self.h.git("commit", mensaje="base")
        self.h.git("otro", comando="checkout -b otra")
        (self.dir / "c.txt").write_text("a\nB-otra\nc\nnueva-otra\n", encoding="utf-8")
        self.h.git("commit", mensaje="otra")
        self.h.git("otro", comando="checkout main")
        (self.dir / "c.txt").write_text("a\nB-main\nc\n", encoding="utf-8")
        self.h.git("commit", mensaje="main")
        self.h.git("otro", comando="merge otra --no-edit")
        (self.dir / "c.txt").write_text("a\nB-main\nc\n", encoding="utf-8")  # perdió `nueva-otra`
        self.assertTrue(self.h.git("commit", mensaje="merge").startswith("RECHAZADO"))
        self.assertFalse(any(x.startswith("git commit «merge»") for x in self.evidencia()), self.evidencia())

    def test_el_revisor_recibe_lo_hecho_con_git(self):
        self.h.git("commit", mensaje="Merge de ajustes en main")
        self.h.git("estado")
        m = Modelo()
        revision.revisar(m, self.dir, "fusiona la rama y deja todo guardado", "", "", self.h.evidencia_texto())
        usuario = m.mensajes[0][1]["content"]
        self.assertIn("COMANDOS EJECUTADOS EN ESTA TAREA", usuario)
        self.assertIn("git commit «Merge de ajustes en main» -> exit=0", usuario)
        self.assertIn("git estado -> exit=0: ## main", usuario)

    def test_nueva_tarea_las_borra(self):
        self.h.git("commit", mensaje="base")
        self.h.nueva_tarea()
        self.assertEqual(self.evidencia(), [])


if __name__ == "__main__":
    unittest.main()
