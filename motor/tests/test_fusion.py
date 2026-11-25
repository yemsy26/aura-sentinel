"""C16 (m4185 git_conflicto): un merge no pierde lo que la otra rama no tocó. El agente resolvió config.json dejando el lado de
ajustes entero y perdió `"nombre": "demo-main"`, que ajustes no había cambiado (git junta líneas contiguas en un bloque y no
enseñaba la base)."""
import os
import random
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.fusion import marcas_sin_quitar, perdidas
from agente.herramientas import Herramientas

HAY_GIT = shutil.which("git") is not None
IDENTIDAD = {"GIT_AUTHOR_NAME": "Prueba", "GIT_AUTHOR_EMAIL": "prueba@local", "GIT_COMMITTER_NAME": "Prueba",
             "GIT_COMMITTER_EMAIL": "prueba@local"}
BASE = '{\n  "nombre": "demo",\n  "color": "azul",\n  "version": 1\n}\n'
MAIN = '{\n  "nombre": "demo-main",\n  "color": "rojo",\n  "version": 1\n}\n'
AJUSTES = '{\n  "nombre": "demo",\n  "color": "verde",\n  "version": 2\n}\n'


def marcadas(*lineas) -> list:
    return [(p.lado, p.linea, p.otro_la_toco) for p in lineas]


class TestPerdidas(unittest.TestCase):
    def test_el_caso_de_m4185(self):
        r = perdidas(BASE, MAIN, AJUSTES, AJUSTES, ("main", "ajustes"))
        self.assertEqual(marcadas(*r), [("main", '"nombre": "demo-main",', False), ("main", '"color": "rojo",', True)])
        self.assertEqual(r[0].original, '"nombre": "demo",')

    def test_el_resultado_correcto_no_pierde_nada(self):
        bien = '{\n  "nombre": "demo-main",\n  "color": "verde",\n  "version": 2\n}\n'
        self.assertEqual([p for p in perdidas(BASE, MAIN, AJUSTES, bien) if not p.otro_la_toco], [])

    def test_simetrico_lo_que_pierde_el_otro_lado(self):
        r = perdidas(BASE, MAIN, AJUSTES, MAIN, ("main", "ajustes"))  # se queda con main entero: pierde version 2 de ajustes
        self.assertEqual([(p.lado, p.linea, p.otro_la_toco) for p in r if not p.otro_la_toco], [("ajustes", '"version": 2', False)])

    def test_una_linea_movida_no_se_pierde(self):
        base, a, b = "uno\ndos\ntres\n", "uno\ndos\nnueva\ntres\n", "uno\ndos\ntres\ncuatro\n"
        self.assertEqual(perdidas(base, a, b, "uno\ndos\ntres\ncuatro\nnueva\n"), [])

    def test_insercion_que_falta(self):
        base, a, b = "uno\ndos\n", "uno\nmia\ndos\n", "uno\ndos\nsuya\n"
        r = perdidas(base, a, b, "uno\ndos\nsuya\n")
        self.assertEqual(marcadas(*r), [("HEAD", "mia", False)])
        self.assertIsNone(r[0].original)

    def test_borrado_por_un_lado_es_conflicto_si_el_otro_lo_cambio(self):
        base, a, b = "uno\nvieja\ntres\n", "uno\ntres\n", "uno\nnueva\ntres\n"
        self.assertEqual(perdidas(base, a, b, "uno\nnueva\ntres\n"), [])  # a solo borró: no añadió nada que se pierda
        self.assertEqual(marcadas(*perdidas(base, a, b, "uno\ntres\n")), [("la otra rama", "nueva", True)])

    def test_espacios_lineas_en_blanco_y_signos_no_cuentan(self):
        base, a, b = "x = 1\n}\n", "x = 1\n\n  \n}\n}\n", "x = 1\n}\n"
        self.assertEqual(perdidas(base, a, b, "x = 1\n}\n"), [])
        self.assertEqual(perdidas("a\nb\n", "a\n  b2  \n", "a\nb\n", "a\r\nb2\r\n"), [])  # sangría y CRLF

    def test_dos_versiones_distintas_de_la_misma_linea_son_conflicto(self):
        r = perdidas("v = 1\n", "v = 2\n", "v = 3\n", "v = 4\n")
        self.assertEqual(sorted(marcadas(*r)), [("HEAD", "v = 2", True), ("la otra rama", "v = 3", True)])

    def test_marcas_sin_quitar(self):
        self.assertTrue(marcas_sin_quitar("a\n<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> ajustes\nb\n"))
        self.assertFalse(marcas_sin_quitar("# Titulo\n=======\nTexto\n"))
        self.assertFalse(marcas_sin_quitar("a < b <<<<<<< c\n"))


@unittest.skipUnless(HAY_GIT, "sin git")
class TestFusionesAleatorias(unittest.TestCase):
    """P3: fusiones limpias (git merge-file) no marcan nada; borrando una línea del resultado, se marca."""

    def fusion(self, base, a, b):
        d = Path(tempfile.mkdtemp(prefix="agente_fusion_"))
        try:
            for n, t in (("a", a), ("base", base), ("b", b)):
                (d / n).write_text(t, encoding="utf-8")
            r = subprocess.run(["git", "merge-file", "-p", "a", "base", "b"], cwd=d, capture_output=True, text=True, check=False)
            return r.returncode, r.stdout
        finally:
            shutil.rmtree(d, ignore_errors=True)

    def edita(self, rng, lineas, zona, etiqueta):
        out, lo, hi = list(lineas), *zona
        for _ in range(rng.randint(1, 3)):
            i = rng.randint(lo, hi - 1)
            op = rng.choice(("cambiar", "insertar", "borrar"))
            if op == "cambiar" and out[i]:
                out[i] = f"{etiqueta} cambio {rng.randint(0, 999)}"
            elif op == "insertar":
                out.insert(i, f"{etiqueta} nueva {rng.randint(0, 999)}")
                hi += 1
            elif op == "borrar" and len(out) > 20:
                out.pop(i)
                hi -= 1
        return out

    def test_cien_fusiones_limpias_sin_falsos_positivos_y_con_aciertos(self):
        rng = random.Random(4189)
        limpias = marcadas_al_quitar = quitables = 0
        for _ in range(100):
            base = [f"linea {i} " + "x" * rng.randint(0, 8) for i in range(40)]
            a = self.edita(rng, base, (2, 15), "A")
            b = self.edita(rng, base, (24, 37), "B")
            codigo, resultado = self.fusion("\n".join(base) + "\n", "\n".join(a) + "\n", "\n".join(b) + "\n")
            if codigo != 0:
                continue
            limpias += 1
            self.assertEqual(perdidas("\n".join(base) + "\n", "\n".join(a) + "\n", "\n".join(b) + "\n", resultado), [])
            nuevas = [l for l in resultado.splitlines() if l.startswith(("A ", "B "))]
            if nuevas:
                quitables += 1
                quitada = rng.choice(nuevas)
                sin = "\n".join(l for l in resultado.splitlines() if l != quitada) + "\n"
                marcadas_al_quitar += bool(perdidas("\n".join(base) + "\n", "\n".join(a) + "\n", "\n".join(b) + "\n", sin))
        self.assertGreaterEqual(limpias, 95)
        self.assertGreaterEqual(marcadas_al_quitar, quitables * 0.95 - 1, (marcadas_al_quitar, quitables))


@unittest.skipUnless(HAY_GIT, "sin git")
class TestConGitReal(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_fusion_"))
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

    def leer(self, ruta):
        return (self.dir / ruta).read_text(encoding="utf-8")

    def fusionar(self):
        return self.h.git("otro", comando="merge ajustes --no-edit")

    def test_el_conflicto_trae_la_base_y_la_pista(self):
        r = self.fusionar()
        self.assertIn("CONFLICT", r)
        self.assertIn("[Conflicto en README.md, config.json:", r)
        self.assertIn("|||||||", self.leer("config.json"))
        self.assertIn('"nombre": "demo",', self.leer("config.json").split("|||||||")[1])  # la base entre ||||||| y =======

    def test_ejecutar_git_tambien_trae_la_base(self):
        self.h.ejecutar("git merge ajustes --no-edit")
        self.assertIn("|||||||", self.leer("config.json"))

    def test_resolver_con_el_lado_de_ajustes_entero_se_rechaza_y_dice_que_linea(self):
        self.fusionar()
        self.esc("config.json", AJUSTES)
        self.esc("README.md", "# Demo\nLinea base\nLinea de main\nLinea de ajustes\n")
        r = self.h.git("commit", mensaje="merge")
        self.assertTrue(r.startswith("RECHAZADO"), r)
        self.assertIn('config.json: `"nombre": "demo-main",`', r)
        self.assertIn('en la base era `"nombre": "demo",`', r)
        self.assertNotIn("version", r)
        self.assertNotIn("README", r)
        self.assertEqual(self.h._git("rev-parse", "-q", "--verify", "MERGE_HEAD")[0], 0)  # el merge sigue en curso

    def test_arreglado_el_nombre_el_commit_pasa_con_la_nota_del_color(self):
        self.fusionar()
        self.esc("README.md", "# Demo\nLinea base\nLinea de main\nLinea de ajustes\n")
        self.esc("config.json", AJUSTES)
        self.assertTrue(self.h.git("commit", mensaje="merge").startswith("RECHAZADO"))
        self.esc("config.json", '{\n  "nombre": "demo-main",\n  "color": "verde",\n  "version": 2\n}\n')
        r = self.h.git("commit", mensaje="merge")
        self.assertNotIn("RECHAZADO", r)
        self.assertEqual(len(self.h._git("log", "-1", "--format=%P")[1].split()), 2)  # el commit del merge tiene dos padres
        self.assertIn('config.json: no está `"color": "rojo",` (de main)', r)
        self.assertEqual(self.h._git("rev-parse", "-q", "--verify", "MERGE_HEAD")[0], 1)  # el merge terminó

    def test_la_tercera_vez_se_hace_con_aviso(self):
        self.fusionar()
        self.esc("README.md", "# Demo\nLinea base\nLinea de main\nLinea de ajustes\n")
        self.esc("config.json", AJUSTES)
        self.assertTrue(self.h.git("commit", mensaje="m").startswith("RECHAZADO"))
        self.assertTrue(self.h.git("commit", mensaje="m").startswith("RECHAZADO"))
        r = self.h.git("commit", mensaje="m")
        self.assertNotIn("RECHAZADO", r)
        self.assertIn('[AVISO: el commit se hizo aunque estas líneas de un lado no están en el resultado: config.json: `"nombre": "demo-main",`', r)

    def test_las_marcas_sin_quitar_se_rechazan(self):
        self.fusionar()
        r = self.h.git("commit", mensaje="merge")
        self.assertRegex(r, r"^RECHAZADO: (README\.md|config\.json) sigue teniendo marcas de conflicto")

    def test_nueva_tarea_cuenta_de_nuevo(self):
        self.fusionar()
        self.esc("README.md", "# Demo\nLinea base\nLinea de main\nLinea de ajustes\n")
        self.esc("config.json", AJUSTES)
        self.h.git("commit", mensaje="m")
        self.h.git("commit", mensaje="m")
        self.h.nueva_tarea()
        self.assertTrue(self.h.git("commit", mensaje="m").startswith("RECHAZADO"))

    def test_un_merge_sin_conflicto_y_un_commit_normal_no_cambian(self):
        self.h.git("otro", comando="checkout -b otra HEAD~1")
        self.esc("nuevo.txt", "hola\n")
        self.assertNotIn("RECHAZADO", self.h.git("commit", mensaje="nuevo"))
        self.h.git("otro", comando="checkout main")
        r = self.h.git("otro", comando="merge otra --no-edit")
        self.assertNotIn("Conflicto", r)
        self.assertNotIn("RECHAZADO", r)
        self.esc("x.txt", "x\n")
        self.assertNotIn("RECHAZADO", self.h.git("commit", mensaje="x"))


if __name__ == "__main__":
    unittest.main()
