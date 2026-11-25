"""C16b (m4190): la comprobación del merge también cuando el commit va por `ejecutar git commit`, `git merge --continue` o
git(accion="otro", comando="commit …"). En m4190 2 de 3 repeticiones commitearon por ejecutar, saltaron C16 y perdieron
`"nombre": "demo-main"` otra vez."""
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.fusion import cierra_fusion
from agente.herramientas import Herramientas

HAY_GIT = shutil.which("git") is not None
IDENTIDAD = {"GIT_AUTHOR_NAME": "Prueba", "GIT_AUTHOR_EMAIL": "prueba@local", "GIT_COMMITTER_NAME": "Prueba",
             "GIT_COMMITTER_EMAIL": "prueba@local"}
BASE = '{\n  "nombre": "demo",\n  "color": "azul",\n  "version": 1\n}\n'
MAIN = '{\n  "nombre": "demo-main",\n  "color": "rojo",\n  "version": 1\n}\n'
AJUSTES = '{\n  "nombre": "demo",\n  "color": "verde",\n  "version": 2\n}\n'
BIEN = '{\n  "nombre": "demo-main",\n  "color": "verde",\n  "version": 2\n}\n'

SI = ["git commit", "git commit -m x", 'git commit -am "Merge ajustes"', 'git add -A && git commit -m "x"', "git add . ; git commit -m x",
      "git -c user.name=x commit -m y", "git.exe commit -m x", "git -C . commit -m x", "git --no-pager commit", "git merge --continue",
      "git status && git merge --continue", "(git add . & git commit -m z)", "GIT commit -m x"]
NO = ["git log --grep commit", "git status", "git diff", "git merge ajustes", "git merge --abort", "git checkout main", "echo git commit",
      "echo hola && echo git commit -m x", "git add -A", "git branch commit", "git reset --hard", "git stash", "git rebase --continue",
      "git commit-tree abc", "git log commit", "python -c \"print('git commit')\"", "npm run commit", "mygit commit"]


class TestReconocedor(unittest.TestCase):
    def test_comandos_que_cierran_un_merge(self):
        for c in SI:
            with self.subTest(c=c):
                self.assertTrue(cierra_fusion(c))

    def test_comandos_que_no(self):
        for c in NO:
            with self.subTest(c=c):
                self.assertFalse(cierra_fusion(c))


@unittest.skipUnless(HAY_GIT, "sin git")
class TestCierreConGit(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_cierre_"))
        self.env = mock.patch.dict(os.environ, IDENTIDAD)
        self.env.start()
        self.h = Herramientas(self.dir)
        self.h.git("otro", comando="init -b main")
        self.esc("README.md", "# Demo\nLinea base\n")
        self.esc("config.json", BASE)
        self.h.git("commit", mensaje="base")
        self.h.git("otro", comando="checkout -b ajustes")
        self.esc("config.json", AJUSTES)
        self.esc("README.md", "# Demo\nLinea base\nLinea de ajustes\n")
        self.h.git("commit", mensaje="ajustes")
        self.h.git("otro", comando="checkout main")
        self.esc("config.json", MAIN)
        self.esc("README.md", "# Demo\nLinea base\nLinea de main\n")
        self.h.git("commit", mensaje="main")

    def tearDown(self):
        self.env.stop()
        shutil.rmtree(self.dir, ignore_errors=True)

    def esc(self, ruta, texto):
        (self.dir / ruta).write_text(texto, encoding="utf-8")

    def cabeza(self):
        return self.h._git("rev-parse", "HEAD")[1]

    def fusion_en_curso(self):
        return self.h._git("rev-parse", "-q", "--verify", "MERGE_HEAD")[0] == 0

    def merge_resuelto_mal(self):
        r = self.h.ejecutar("git merge ajustes --no-edit")
        self.esc("README.md", "# Demo\nLinea base\nLinea de main\nLinea de ajustes\n")
        self.esc("config.json", AJUSTES)  # pierde "nombre": "demo-main"
        return r

    def test_ejecutar_git_merge_con_conflicto_trae_la_pista_y_la_base(self):
        r = self.merge_resuelto_mal()
        self.assertIn("CONFLICT", r)
        self.assertIn("[Conflicto en README.md, config.json:", r)

    def test_ejecutar_commit_se_rechaza_sin_ejecutarse(self):
        self.merge_resuelto_mal()
        antes = self.cabeza()
        r = self.h.ejecutar("git add -A && git commit -m x")
        self.assertTrue(r.startswith("RECHAZADO"), r)
        self.assertIn('config.json: `"nombre": "demo-main",`', r)
        self.assertEqual(self.cabeza(), antes)
        self.assertTrue(self.fusion_en_curso())

    def test_git_otro_commit_y_merge_continue_tambien(self):
        self.merge_resuelto_mal()
        self.assertTrue(self.h.git("otro", comando="commit -am x").startswith("RECHAZADO"))
        self.assertTrue(self.h.ejecutar("git merge --continue").startswith("RECHAZADO"))
        self.assertTrue(self.fusion_en_curso())

    def test_arreglado_el_nombre_pasa_por_cualquier_via_con_la_nota(self):
        self.merge_resuelto_mal()
        self.assertTrue(self.h.ejecutar("git add -A && git commit -m x").startswith("RECHAZADO"))
        self.esc("config.json", BIEN)
        r = self.h.ejecutar("git add -A && git commit -m x")
        self.assertTrue(r.startswith("exit=0"), r)
        self.assertIn('config.json: no está `"color": "rojo",` (de main)', r)
        self.assertFalse(self.fusion_en_curso())

    def test_el_contador_es_el_mismo_para_todas_las_vias(self):
        self.merge_resuelto_mal()
        self.assertTrue(self.h.git("commit", mensaje="m").startswith("RECHAZADO"))
        self.assertTrue(self.h.ejecutar("git commit -am m").startswith("RECHAZADO"))
        r = self.h.git("otro", comando="commit -am m")  # la 3.ª, venga por donde venga, se hace con aviso
        self.assertNotIn("RECHAZADO", r)
        self.assertIn("[AVISO: el commit se hizo aunque estas líneas", r)

    def test_lo_que_no_cierra_el_merge_no_se_toca(self):
        self.merge_resuelto_mal()
        for c in ("git log --grep commit", "git status", "git diff"):
            self.assertFalse(self.h.ejecutar(c).startswith("RECHAZADO"), c)
        self.assertEqual(self.h.fusion_rechazos, 0)

    def test_un_commit_normal_por_ejecutar_no_cambia(self):
        self.esc("nuevo.txt", "hola\n")
        r = self.h.ejecutar("git add -A && git commit -m nuevo")
        self.assertTrue(r.startswith("exit=0"), r)
        self.assertNotIn("Conflictos", r)


if __name__ == "__main__":
    unittest.main()
