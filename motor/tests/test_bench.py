"""El banco de evaluación (bench/, m4193): reglas Q1/Q2, las 12 misiones, fixtures y jueces sobre el estado de partida, el .bat de la
subcarpeta (rust_palabras de m4192) y la CLI. bench/ no va en el instalador (no está en los recursos de Tauri): sin él, se salta."""
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

RAIZ = Path(__file__).resolve().parent.parent.parent
BENCH = RAIZ / "bench"
HAY_BENCH = (BENCH / "correr.py").is_file() and (BENCH / "misiones.py").is_file()
HAY_GIT = shutil.which("git") is not None

if HAY_BENCH:
    sys.path.insert(0, str(BENCH))
    import correr as C
    import misiones as M


def _fila(mision, termina=True, juez_ok=True, **extra):
    return {"mision": mision, "termina": termina, "juez_ok": juez_ok, "pasos": 10, "min": 1.0, "juez": "x", "llamadas": {}} | extra


@unittest.skipUnless(HAY_BENCH, "bench/ no está (el instalador no lo lleva)")
class TestReglas(unittest.TestCase):
    def test_q1_las_cuatro_combinaciones(self):
        self.assertEqual(C.veredicto(True, True), "FUNCIONA")
        self.assertEqual(C.veredicto(False, True), "PARCIAL")
        self.assertEqual(C.veredicto(True, False), "PARCIAL")
        self.assertEqual(C.veredicto(False, False), "FALLA")

    def test_q1_un_traceback_es_falla_aunque_pase(self):
        self.assertEqual(C.veredicto(True, True, traceback=True), "FALLA")

    def test_q2_umbrales(self):
        for n, esperado in ((12, "base sólida"), (10, "base sólida"), (9, "corregir"), (7, "corregir"), (6, "no pasar"), (0, "no pasar")):
            with self.subTest(n=n):
                self.assertIn(esperado, C.q2(n, 12))

    def test_q2_con_menos_misiones_es_proporcional(self):
        self.assertIn("base sólida", C.q2(2, 2))
        self.assertIn("no pasar", C.q2(0, 2))

    def test_metricas_del_registro(self):
        with tempfile.TemporaryDirectory() as t:
            c = Path(t)
            (c / ".agente").mkdir()
            (c / ".agente" / "registro.log").write_text(
                "[10:00:01] editar {} -> OK\n[10:00:02] editar {} -> Sin cambios\n[10:00:03] ejecutar {} -> exit=1\n"
                "[10:00:04] ejecutar {} -> exit=0\nVERIFICACIÓN (x): 2 ERRORES, 1 AVISOS\nFIN: listo\n", encoding="utf-8")
            (c / ".agente" / "metricas.jsonl").write_text(
                json.dumps({"tipo": "turno", "paso": 3}) + "\n" + json.dumps({"tipo": "turno", "paso": 7}) + "\nno es json\n", encoding="utf-8")
            m = C.metricas(c, "")
        self.assertEqual(m["pasos"], 7)
        self.assertTrue(m["termina"])
        self.assertEqual(m["llamadas"]["editar"], [2, 1])
        self.assertEqual(m["llamadas"]["ejecutar"], [2, 1])
        self.assertEqual(m["ultimo_verificar"], ["2", "1"])

    def test_metricas_detecta_repeticion_y_traceback(self):
        with tempfile.TemporaryDirectory() as t:
            m = C.metricas(Path(t), "Detenido: el modelo repitió 'leer' 5 veces\nTraceback (most recent call last)\nSe alcanzó el máximo de 200 pasos")
        self.assertEqual(m["repitio"], "leer")
        self.assertTrue(m["traceback"])
        self.assertEqual(m["maximo"], ["200"])
        self.assertFalse(m["termina"])


@unittest.skipUnless(HAY_BENCH, "bench/ no está (el instalador no lo lleva)")
class TestMisiones(unittest.TestCase):
    def test_son_las_doce(self):
        esperadas = {"git_conflicto", "bug_py", "csv_ventas", "mochila_sat", "investigar_web", "node_palabras", "go_palabras",
                     "rust_palabras", "escritorio_gastos", "docker_app", "evm_token", "tienda_roles"}
        self.assertEqual(set(M.MISIONES), esperadas)

    def test_cada_una_trae_tope_mandato_fixture_y_juez(self):
        for k, (tope, mandato, fx, juez) in M.MISIONES.items():
            with self.subTest(k=k):
                self.assertIsInstance(tope, int)
                self.assertGreater(tope, 0)
                self.assertGreater(len(mandato), 40)
                self.assertTrue(callable(fx) and callable(juez))

    def test_el_mandato_no_dice_que_herramienta_usar(self):
        # el banco prueba al agente con órdenes corrientes (memoria pruebas-mandato-objetivo)
        for k, (_t, mandato, _fx, _j) in M.MISIONES.items():
            with self.subTest(k=k):
                for prohibida in ("resolver_sat", "web_buscar", "investigar(", "crear_bat", "terminar("):
                    self.assertNotIn(prohibida, mandato)


@unittest.skipUnless(HAY_BENCH, "bench/ no está (el instalador no lo lleva)")
class TestJueces(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="test_bench_"))
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        M.iniciar(RAIZ / "motor", self.tmp / "tmp")

    def carpeta(self, nombre):
        c = self.tmp / nombre
        c.mkdir()
        return c

    @unittest.skipUnless(HAY_GIT, "sin git")
    def test_git_en_el_estado_de_partida_no_pasa(self):
        c = self.carpeta("git")
        ctx = M.fx_git(c)
        ok, det = M.juez_git(c, ctx)
        self.assertFalse(ok)
        self.assertIn("ajustes no está en main", det)

    def test_bug_no_pasa_al_inicio_y_tocar_las_pruebas_lo_marca(self):
        c = self.carpeta("bug")
        ctx = M.fx_bug(c)
        self.assertFalse(M.juez_bug(c, ctx)[0])
        (c / "test_inventario.py").write_text("def test_x(): pass\n", encoding="utf-8")
        ok, det = M.juez_bug(c, ctx)
        self.assertFalse(ok)
        self.assertIn("pruebas se han tocado", det)

    def test_csv_con_un_run_bat_que_imprime_los_totales_y_pruebas(self):
        c = self.carpeta("csv")
        ctx = M.fx_csv(c)
        self.assertFalse(M.juez_csv(c, ctx)[0])
        lineas = [f"echo {k} {v}" for k, v in ctx["totales"].items()] + [f"echo mejor dia {ctx['mejor_dia']}"]
        (c / "run.bat").write_text("@echo off\r\n" + "\r\n".join(lineas) + "\r\n", encoding="utf-8")
        ok, det = M.juez_csv(c, ctx)
        self.assertFalse(ok)  # sin pruebas
        self.assertIn("pruebas", det)
        (c / "test_ventas.py").write_text("import unittest\nclass T(unittest.TestCase):\n    def test_a(self):\n        self.assertTrue(True)\n", encoding="utf-8")
        ok, det = M.juez_csv(c, ctx)
        self.assertTrue(ok, det)

    def test_mochila_pasa_solo_con_el_optimo(self):
        c = self.carpeta("mochila")
        ctx = M.fx_mochila(c)
        self.assertFalse(M.juez_mochila(c, ctx)[0])
        objs, cap = ctx["objs"], ctx["cap"]
        mejor = {0: (0, [])}  # peso -> (valor, nombres): el mismo DP que el fixture, con la selección
        for n, (p, v) in objs.items():
            for peso, (val, sel) in sorted(mejor.items(), reverse=True):
                if peso + p <= cap and mejor.get(peso + p, (-1,))[0] < val + v:
                    mejor[peso + p] = (val + v, [*sel, n])
        valor, sel = max(mejor.values(), key=lambda x: x[0])
        self.assertEqual(valor, ctx["optimo"])
        (c / "resultado.txt").write_text("\n".join(sel), encoding="utf-8")
        self.assertTrue(M.juez_mochila(c, ctx)[0])
        (c / "resultado.txt").write_text(sel[0], encoding="utf-8")
        self.assertFalse(M.juez_mochila(c, ctx)[0])

    def test_palabras_fixture_deja_el_texto_y_el_juez_no_pasa(self):
        c = self.carpeta("palabras")
        ctx = M.fx_palabras(c)
        self.assertTrue((c / "texto.txt").is_file())
        self.assertEqual(len(ctx["top"]), 10)


@unittest.skipUnless(HAY_BENCH, "bench/ no está (el instalador no lo lleva)")
class TestBatDeLaSubcarpeta(unittest.TestCase):
    """m4192: tras el rechazo de crear_bat (C15) el agente dejó run.bat y test.bat en la carpeta del proyecto, no en la raíz."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="test_bench_bat_"))
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        M.iniciar(RAIZ / "motor", self.tmp / "tmp")

    def test_sin_bat(self):
        self.assertIsNone(M.bat_de(self.tmp))

    def test_el_de_la_raiz_gana(self):
        (self.tmp / "run.bat").write_text("@echo raiz\r\n", encoding="utf-8")
        (self.tmp / "proyecto").mkdir()
        (self.tmp / "proyecto" / "run.bat").write_text("@echo sub\r\n", encoding="utf-8")
        self.assertEqual(M.bat_de(self.tmp), self.tmp / "run.bat")

    def test_el_de_la_subcarpeta_si_no_hay_en_la_raiz_y_corre_desde_alli(self):
        p = self.tmp / "palabras_repeticas"
        p.mkdir()
        (p / "run.bat").write_text("@echo off\r\necho estoy en %CD%\r\n", encoding="utf-8")
        (p / "test.bat").write_text("@echo off\r\necho prueba\r\n", encoding="utf-8")
        self.assertEqual(M.bat_de(self.tmp), p / "run.bat")
        rc, out = M.correr_bat(self.tmp)
        self.assertEqual(rc, 0)
        self.assertIn("palabras_repeticas", out)

    def test_ignora_test_bat_y_las_carpetas_de_dependencias(self):
        (self.tmp / "test.bat").write_text("@echo t\r\n", encoding="utf-8")
        nm = self.tmp / "node_modules"
        nm.mkdir()
        (nm / "run.bat").write_text("@echo dep\r\n", encoding="utf-8")
        self.assertIsNone(M.bat_de(self.tmp))
        (self.tmp / "iniciar.bat").write_text("@echo i\r\n", encoding="utf-8")
        self.assertEqual(M.bat_de(self.tmp).name, "iniciar.bat")


@unittest.skipUnless(HAY_BENCH, "bench/ no está (el instalador no lo lleva)")
class TestCLI(unittest.TestCase):
    def correr(self, *args):
        return subprocess.run([sys.executable, str(BENCH / "correr.py"), *args], capture_output=True, text=True, encoding="utf-8",
                              errors="replace", timeout=300, check=False, stdin=subprocess.DEVNULL)

    def test_seco_de_una_mision_sin_modelo(self):
        r = self.correr("--seco", "bug_py")
        self.assertEqual(r.returncode, 0, r.stderr[-400:])
        self.assertIn("bug_py: juez -> False", r.stdout)

    def test_seco_de_una_que_pide_red_o_docker_solo_crea_el_fixture(self):
        r = self.correr("--seco", "docker_app")
        self.assertEqual(r.returncode, 0, r.stderr[-400:])
        self.assertIn("el juez no se ejecuta en seco", r.stdout)

    def test_mision_desconocida(self):
        r = self.correr("--seco", "no_existe")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("Misiones desconocidas", r.stderr + r.stdout)

    def test_resumen_y_contra(self):
        with tempfile.TemporaryDirectory() as t:
            a, b = Path(t) / "a.jsonl", Path(t) / "b.jsonl"
            claves = list(M.MISIONES)
            a.write_text("\n".join(json.dumps(_fila(k, juez_ok=i < 10)) for i, k in enumerate(claves)) + "\n", encoding="utf-8")
            b.write_text("\n".join(json.dumps(_fila(k, termina=i < 6)) for i, k in enumerate(claves)) + "\n", encoding="utf-8")
            r = self.correr("--resumen", str(a))
            self.assertIn("10 de 12 FUNCIONA", r.stdout)
            self.assertIn("base sólida", r.stdout)
            r = self.correr("--resumen", str(b), "--contra", str(a))
            self.assertIn("6 de 12 FUNCIONA", r.stdout)
            self.assertIn("no pasar a mejoras nuevas", r.stdout)
            self.assertIn("(contra)", r.stdout)
            self.assertIn("(FUNCIONA)", r.stdout)

    def test_q3_marca_herramientas_con_muchos_errores(self):
        with tempfile.TemporaryDirectory() as t:
            a = Path(t) / "a.jsonl"
            a.write_text(json.dumps(_fila("bug_py", llamadas={"editar": [10, 5], "leer": [20, 0]})) + "\n", encoding="utf-8")
            r = self.correr("--resumen", str(a))
        self.assertIn("editar (5/10 errores)", r.stdout)
        self.assertNotIn("leer (", r.stdout)


class TestInstalador(unittest.TestCase):
    def test_bench_no_va_en_los_recursos_de_tauri(self):
        conf = RAIZ / "src-tauri" / "tauri.conf.json"
        if not conf.is_file():
            self.skipTest("sin tauri.conf.json (instalado)")
        texto = conf.read_text(encoding="utf-8")
        self.assertNotIn("bench", texto)
        self.assertIn("../motor/tests/*.py", texto)


if __name__ == "__main__":
    unittest.main()
