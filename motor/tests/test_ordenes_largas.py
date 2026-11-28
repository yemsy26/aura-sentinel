"""m4197: en un proyecto grande la orden con todas las rutas pasaba de 32.767 caracteres (WinError 206) y ninguna tarea podía
empezar en esa carpeta: `nueva_tarea` analiza el código sin usar de todo el proyecto antes de que el modelo haga nada."""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import verificacion as V
from agente.herramientas import Herramientas
from agente.proyecto import Informe, archivos

RELLENO = "modulo_con_un_nombre_bien_largo_para_que_la_orden_no_quepa_en_windows_" + "x" * 40


def proyecto_grande(raiz: Path, n: int = 300) -> None:
    for i in range(n):
        (raiz / f"{RELLENO}_{i:03d}.py").write_text(f"VALOR_{i} = {i}\n", encoding="utf-8")
    (raiz / "util_ñ con espacio.py").write_text("def la_que_se_usa():\n    return 1\n\n\ndef nadie_me_llama_xyz():\n    return 2\n", encoding="utf-8")
    (raiz / "principal.py").write_text("from util import la_que_se_usa\n\nprint(la_que_se_usa(), no_esta_definido)\n", encoding="utf-8")


@unittest.skipUnless(sys.platform == "win32", "el límite es de CreateProcess")
class TestOrdenLarga(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.dir = Path(tempfile.mkdtemp(prefix="agente_largas_"))
        proyecto_grande(cls.dir)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.dir, ignore_errors=True)

    def test_la_orden_de_verdad_no_cabe(self):
        self.assertGreater(len(subprocess.list2cmdline([str(p) for p in archivos(self.dir, ".py")])), 32767)

    def test_el_analisis_de_codigo_sin_usar_ve_los_usos_entre_archivos(self):
        previos = V.muertos_ahora(self.dir)
        self.assertTrue(any("nadie_me_llama_xyz" in x for x in previos), previos)
        self.assertFalse(any("la_que_se_usa" in x for x in previos), previos)  # se usa desde otro archivo: no se parte en lotes

    def test_ruff_tambien_lee_las_rutas_de_un_archivo(self):
        inf = Informe()
        V.lint_python(self.dir, archivos(self.dir, ".py"), inf)
        self.assertTrue(any("no_esta_definido" in e for e in inf.errores), inf.errores)
        self.assertFalse(any("No pude" in a for a in inf.avisos), inf.avisos)

    def test_una_tarea_puede_empezar(self):
        h = Herramientas(self.dir)
        h.nueva_tarea(texto="en que parte vas")
        self.assertTrue(any("nadie_me_llama_xyz" in x for x in h.muertos_previos))

    def test_no_deja_el_archivo_de_rutas(self):
        antes = set(Path(tempfile.gettempdir()).glob("*.rutas"))
        V.muertos_ahora(self.dir)
        V.lint_python(self.dir, archivos(self.dir, ".py"), Informe())
        self.assertEqual(set(Path(tempfile.gettempdir()).glob("*.rutas")), antes)


class TestMismoResultado(unittest.TestCase):
    """El camino con archivo de rutas da lo mismo que la orden de siempre (se fuerza con el límite a 0)."""

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_rutas_")) / "Carpeta con espacios y ñ"
        self.dir.mkdir()
        self.addCleanup(shutil.rmtree, self.dir.parent, ignore_errors=True)
        (self.dir / "a uno.py").write_text("def usada():\n    x = 1\n\n\ndef nadie_me_llama():\n    pass\n", encoding="utf-8")
        (self.dir / "b.py").write_text("from a_uno import usada\n\nusada()\nprint(no_existe)\n", encoding="utf-8")

    def hallazgos(self):
        lint, muerto = Informe(), Informe()
        rutas = archivos(self.dir, ".py")
        V.lint_python(self.dir, rutas, lint)
        V.codigo_muerto(self.dir, rutas, muerto)
        return lint.errores, lint.avisos, muerto.errores, muerto.avisos

    def test_igual_por_los_dos_caminos(self):
        normal = self.hallazgos()
        with mock.patch.object(V, "LIMITE_ORDEN", 0):
            por_archivo = self.hallazgos()
        self.assertEqual(normal, por_archivo)
        self.assertTrue(any("no_existe" in e for e in por_archivo[0]))
        self.assertTrue(any("nadie_me_llama" in e for e in por_archivo[2]))


class TestNoEjecutaLoAnalizado(unittest.TestCase):
    """m4206: el camino con archivo de rutas lanza python con cwd = proyecto; nada de la carpeta analizada puede ejecutarse."""

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_ajeno_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)
        (self.dir / "módulo_ñ.py").write_text("def función_sin_uso_ñ():\n    pass\n", encoding="utf-8")

    def analizar(self, limite: int = 0) -> list[str]:
        inf = Informe()
        with mock.patch.object(V, "LIMITE_ORDEN", limite):
            V.codigo_muerto(self.dir, archivos(self.dir, ".py"), inf)
        return inf.errores + inf.avisos

    def no_se_ejecuta(self, nombre: str):
        (self.dir / nombre).write_text("open(__file__ + '.ejecutado', 'w').close()\n", encoding="utf-8")
        hallados = self.analizar()
        self.assertFalse((self.dir / f"{nombre}.ejecutado").exists(), f"se ejecutó {nombre} de la carpeta analizada")
        self.assertTrue(any("función_sin_uso_ñ" in x for x in hallados), hallados)

    def test_un_vulture_py_en_la_raiz(self):
        self.no_se_ejecuta("vulture.py")

    def test_un_modulo_con_el_nombre_de_uno_que_vulture_importa(self):
        self.no_se_ejecuta("tomllib.py")

    def test_acentos_igual_que_la_orden_corta(self):
        with mock.patch.dict("os.environ", {"PYTHONUTF8": "1"}):  # como arranca el motor
            corta = self.analizar(V.LIMITE_ORDEN)
            self.assertEqual(self.analizar(), corta)
        self.assertTrue(any("módulo_ñ.py" in x and "función_sin_uso_ñ" in x for x in corta), corta)

    def test_acentos_sin_la_variable_de_entorno(self):
        with mock.patch.dict("os.environ"):
            os.environ.pop("PYTHONUTF8", None)
            hallados = self.analizar()
        self.assertTrue(any("módulo_ñ.py" in x and "función_sin_uso_ñ" in x for x in hallados), hallados)


class TestElAnalizadorNuncaTiraLaTarea(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_analizador_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)
        (self.dir / "a.py").write_text("def nadie():\n    pass\n", encoding="utf-8")

    def test_si_no_arranca_no_se_acusa_codigo_sin_usar_y_la_tarea_empieza(self):
        with mock.patch.object(V, "correr_texto", side_effect=OSError(206, "El nombre del archivo o la extensión es demasiado largo")):
            h = Herramientas(self.dir)
            h.nueva_tarea(texto="hola")
            self.assertEqual(h.muertos_previos, set())
            inf = Informe()
            V.codigo_muerto(self.dir, archivos(self.dir, ".py"), inf)
            self.assertEqual(inf.errores, [])
            self.assertEqual(inf.avisos, [V.SIN_ANALISIS])
            lint = Informe()
            V.lint_python(self.dir, archivos(self.dir, ".py"), lint)
            self.assertEqual(lint.errores, [])
            self.assertTrue(any("No pude pasar ruff" in a for a in lint.avisos))

    def test_si_no_acaba_a_tiempo_tampoco_se_acusa_nada(self):
        with mock.patch.object(V, "correr_texto", return_value=(None, "a.py:1: unused function 'nadie' (60% confidence)")):
            inf = Informe()
            V.codigo_muerto(self.dir, archivos(self.dir, ".py"), inf)
        self.assertEqual(inf.errores, [])
        self.assertEqual(inf.avisos, [V.SIN_ANALISIS])

    def test_el_aviso_no_entra_en_la_foto_de_partida(self):
        with mock.patch.object(V, "correr_texto", side_effect=OSError("no")):
            self.assertEqual(V.muertos_ahora(self.dir), set())


if __name__ == "__main__":
    unittest.main()
