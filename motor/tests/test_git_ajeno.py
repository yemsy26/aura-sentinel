"""Un repositorio que llega con su .git: leerlo no ejecuta nada suyo (m4209) y el modo grande no lo confirma (m4210). Sin modelo.
Las trampas son inofensivas: cada una deja una marca en una carpeta temporal."""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from agente import git_seguro, grande
from agente.herramientas import Herramientas

IDENTIDAD = {"GIT_AUTHOR_NAME": "Prueba", "GIT_AUTHOR_EMAIL": "prueba@local", "GIT_COMMITTER_NAME": "Prueba",
             "GIT_COMMITTER_EMAIL": "prueba@local"}
LECTURAS = ("status -v", "-C . status", "diff HEAD", "diff --ext-diff HEAD", "log -p", "log --show-signature -1",
            "reflog -p", "show HEAD", "blame a.txt", "range-diff HEAD~1..HEAD HEAD~1..HEAD")
FIRMA = "gpgsig -----BEGIN PGP SIGNATURE-----\n \n falsa\n -----END PGP SIGNATURE-----\n"


@unittest.skipUnless(shutil.which("git"), "git no instalado")
class TestGitAjeno(unittest.TestCase):
    def setUp(self):
        self.base = Path(tempfile.mkdtemp(prefix="agente_ga_"))
        self.dir = self.base / "repo"
        self.dir.mkdir()
        (self.base / "global").write_text("")  # la configuración git de quien prueba no cuenta
        self.entorno = mock.patch.dict(os.environ, dict(IDENTIDAD, GIT_CONFIG_NOSYSTEM="1",
                                                        GIT_CONFIG_GLOBAL=str(self.base / "global")))
        self.entorno.start()
        self.h = Herramientas(self.dir, max_chars=6000, timeout=30)
        self.git("init", "-q")
        self.escribir(".gitattributes", "*.txt filter=x diff=x\n")
        for texto, mensaje in (("1\n", "uno"), ("2\n", "dos")):
            self.escribir("a.txt", texto)
            self.git("add", "-A")
            self.git("commit", "-qm", mensaje)
        self.escribir("a.txt", "3\n")
        self.git("add", "-A")

    def tearDown(self):
        self.h.detener_todos()
        self.entorno.stop()
        for p in self.base.rglob("*"):  # los objetos de .git son de solo lectura
            os.chmod(p, 0o777)
        shutil.rmtree(self.base, ignore_errors=True)

    def git(self, *args, cwd=None, entrada=None) -> str:
        r = subprocess.run(["git", *args], cwd=cwd or self.dir, input=entrada, capture_output=True, check=True)
        return r.stdout.decode()

    def escribir(self, ruta, texto):
        (self.dir / ruta).write_text(texto, encoding="utf-8", newline="\n")

    def trampa(self, nombre) -> str:
        return f"echo m >> '{(self.base / ('marca_' + nombre)).as_posix()}'"

    def marcas(self) -> list[str]:
        """Las que haya, y las borra para la orden siguiente."""
        halladas = list(self.base.glob("marca_*"))
        for p in halladas:
            p.unlink()
        return sorted(p.name.removeprefix("marca_") for p in halladas)

    def poner_trampas(self):
        # un commit "firmado" (firma falsa) para que log.showSignature tenga a quién llamar
        cabecera, _, mensaje = self.git("cat-file", "commit", "HEAD").partition("\n\n")
        firmado = f"{cabecera}\n{FIRMA}\n{mensaje}".encode()
        self.git("update-ref", "HEAD", self.git("hash-object", "-t", "commit", "-w", "--stdin", entrada=firmado).strip())
        gpg = self.base / "gpg_falso"
        gpg.write_text(f"#!/bin/sh\n{self.trampa('gpg')}\n", newline="\n")
        hook = self.dir / ".git" / "hooks" / "post-index-change"
        hook.write_text(f"#!/bin/sh\n{self.trampa('hook')}\n", newline="\n")
        for clave, valor in (("core.fsmonitor", self.trampa("monitor") + "; false"),
                             ("filter.x.clean", self.trampa("filtro") + "; cat"),
                             ("diff.x.textconv", self.trampa("textconv") + "; cat"),
                             ("diff.external", f'sh -c "{self.trampa("externo")}"'),
                             ("log.showSignature", "true"), ("gpg.program", gpg.as_posix())):
            self.git("config", clave, valor)

    def test_las_lecturas_no_ejecutan_nada(self):
        self.poner_trampas()
        self.git("status")
        self.git("log", "-1", "-p", "--ext-diff")
        self.assertEqual({"monitor", "filtro", "hook", "gpg", "externo"} - set(self.marcas()), set())  # están vivas
        ordenes = [{"accion": a} for a in ("estado", "diff", "log")]
        ordenes += [{"accion": "otro", "comando": c} for c in LECTURAS]
        for orden in ordenes:
            with self.subTest(**orden):
                self.escribir("a.txt", "3\n")  # tocado: git tiene que releerlo con sus filtros
                self.h.git(**orden)
                self.assertEqual(self.marcas(), [])
        self.assertIn("a.txt", self.h.git(accion="estado"))
        self.assertIn("uno", self.h.git(accion="log"))

    def test_el_filtro_del_usuario_se_respeta(self):
        self.git("config", "--global", "filter.x.clean", "sed s/3/TRES/")
        self.git("config", "filter.x.clean", self.trampa("filtro") + "; cat")
        self.escribir("a.txt", "3\n")
        self.assertIn("+TRES", self.h.git(accion="diff"))
        self.assertEqual(self.marcas(), [])

    def test_commit_sigue_usando_el_filtro_del_repositorio(self):
        self.git("config", "filter.x.clean", self.trampa("filtro") + "; cat")
        self.escribir("a.txt", "4\n")
        self.h.git(accion="commit", mensaje="tres")
        self.assertIn("filtro", self.marcas())

    def test_subcarpeta_con_forma_de_repositorio_no_se_lee(self):
        # viaja con git clone: HEAD, config, objects/ y refs/ como archivos normales
        docs = self.dir / "docs"
        self.git("init", "-q", "--bare", "docs")
        for clave, valor in (("core.bare", "false"), ("core.worktree", "."),
                             ("core.fsmonitor", self.trampa("monitor") + "; false")):
            self.git("config", clave, valor, cwd=docs)
        h = Herramientas(docs, max_chars=6000, timeout=30)
        for accion in ("estado", "diff", "log"):
            with self.subTest(accion=accion):
                self.assertIn("safe.bareRepository", h.git(accion=accion))
                self.assertEqual(self.marcas(), [])

    def hook(self, nombre, carpeta=None):
        ruta = (carpeta or self.dir / ".git" / "hooks") / nombre
        ruta.parent.mkdir(parents=True, exist_ok=True)
        ruta.write_text(f"#!/bin/sh\n{self.trampa(nombre)}\n", newline="\n")
        return ruta

    def test_el_modo_grande_no_confirma_con_programas_del_repositorio(self):
        orden = self.trampa("orden") + "; cat"
        for clave in ("filter.x.clean", "filter.x.process", "lfs.extension.x.clean", "core.fsmonitor", "gpg.program",
                      "gpg.ssh.defaultKeyCommand", "gc.recentObjectsHook", "hook.mio.command", "core.hooksPath"):
            with self.subTest(clave):
                self.git("config", clave, orden)
                motivo = grande.preparar_git(self.h)
                self.assertTrue(motivo.startswith("sin git: el repositorio trae programas propios"), motivo)
                self.assertIn(clave.lower(), motivo)
                self.assertEqual(self.marcas(), [])
                self.git("config", "--unset", clave)
        for nombre in git_seguro.HOOKS_COMMIT:
            with self.subTest(nombre):
                ruta = self.hook(nombre)
                self.assertIn(f"hooks/{nombre}", grande.preparar_git(self.h))
                ruta.unlink()
        with self.subTest("en un archivo que incluye la configuración del repositorio"):
            (self.dir / ".git" / "mas").write_text('[filter "y"]\n\tclean = cat\n')
            self.git("config", "include.path", "mas")
            self.assertIn("filter.y.clean", grande.preparar_git(self.h))
            self.git("config", "--unset", "include.path")
        with self.subTest("hooks del usuario, dentro del proyecto"):
            self.git("config", "--global", "core.hooksPath", ".githooks")
            self.hook("pre-commit", self.dir / ".githooks")
            self.assertIn("hooks/pre-commit", grande.preparar_git(self.h))
        self.assertEqual(self.marcas(), [])

    def test_el_modo_grande_confirma_con_lo_del_usuario(self):
        self.assertEqual(grande.preparar_git(self.h), "repo")  # solo los .sample de git init
        self.hook("pre-push")
        self.git("config", "--global", "filter.x.clean", "cat")
        self.git("config", "filter.x.clean", "cat")  # git lfs install --local repite el del usuario
        self.git("config", "core.fsmonitor", "true")  # el vigilante de git, no un programa
        self.assertEqual(grande.preparar_git(self.h), "repo")
        self.git("config", "--unset", "core.fsmonitor")
        fuera = self.base / "mis_hooks"
        self.hook("pre-commit", fuera)
        self.git("config", "--global", "core.hooksPath", fuera.as_posix())
        estado = {"git": grande.preparar_git(self.h)}
        antes = grande._sucios(self.h)
        self.escribir("b.md", "nuevo\n")
        self.assertRegex(grande._commit(self.h, estado, "Parte 1/1: b", antes), r"^[0-9a-f]{7,}$")
        self.assertEqual(self.marcas(), ["pre-commit"])  # el hook del usuario sigue saltando

    def test_un_hook_que_aparece_a_mitad_detiene_los_commits(self):
        estado = {"git": grande.preparar_git(self.h)}
        antes, cabeza = grande._sucios(self.h), self.git("rev-parse", "HEAD")
        self.hook("pre-commit")
        self.escribir("b.md", "nuevo\n")
        self.assertEqual(grande._commit(self.h, estado, "Parte 1/1: b", antes), "")
        self.assertIn("hooks/pre-commit", estado["git"])
        self.assertEqual((self.git("rev-parse", "HEAD"), self.marcas()), (cabeza, []))

    def test_el_modo_grande_no_toca_un_git_incompleto(self):
        self.git("config", "filter.x.clean", self.trampa("filtro") + "; cat")
        self.hook("pre-commit")
        (self.dir / ".git" / "HEAD").unlink()
        self.assertTrue(grande.preparar_git(self.h).startswith("sin git: hay un .git que git no reconoce"))
        self.assertFalse((self.dir / ".git" / "HEAD").exists())
        self.assertEqual(self.marcas(), [])

    def test_sin_poder_comprobar_no_hay_commits(self):
        self.assertIsNone(git_seguro.propios(self.dir, lambda *args: (128, "fatal: bad config")))
        with mock.patch.object(self.h, "_git", lambda *args, **k: (128, "fatal: bad config")):
            self.assertTrue(grande._impedimento(self.h).startswith("sin git: no pude comprobar"))

    def test_lo_que_se_anula(self):
        salida = ("global\0filter.lfs.clean\ngit-lfs clean -- %f\0local\0filter.lfs.clean\nmalo\0"
                  "local\0filter.x.required\ntrue\0local\0diff.y.textconv\nmalo\0local\0diff.external\nmalo\0"
                  "worktree\0gpg.ssh.program\nmalo\0local\0diff.algorithm\nhistogram\0")
        self.assertEqual(git_seguro.anulados(salida), [("filter.lfs.clean", "git-lfs clean -- %f"),
                                                       ("filter.x.required", "false"), ("diff.y.textconv", "cat"),
                                                       ("diff.external", ""), ("gpg.ssh.program", "ssh-keygen")])


if __name__ == "__main__":
    unittest.main()
