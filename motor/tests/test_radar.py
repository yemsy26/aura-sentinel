"""El radar mensual (bench/radar.py, m4194), sin red: el HTTP se inyecta. bench/ no va en el instalador: sin él, se salta."""
import json
import re
import subprocess
import sys
import tempfile
import unittest
from datetime import UTC, datetime, timedelta
from pathlib import Path

RAIZ = Path(__file__).resolve().parent.parent.parent
BENCH = RAIZ / "bench"
HAY_BENCH = (BENCH / "radar.py").is_file() and (BENCH / "radar_fuentes.json").is_file()
if HAY_BENCH:
    sys.path.insert(0, str(BENCH))
    import radar as R

AHORA = datetime(2026, 11, 25, 12, 0, tzinfo=UTC)
LOCAL = {"aura": "4.13.20", "repo": "abc1234 algo", "rama": "main", "sat": "v0.2.3", "modelos": ["a/b"], "herramientas": {"node": "v24"},
         "llama": {"backend": "2.83.0", "version": "1.0.0-dev", "build": "1", "commit": "33aeb17"}}


def rel(tag, dias, pre=False, draft=False, nombre=""):
    return {"tag_name": tag, "name": nombre, "published_at": (AHORA - timedelta(days=dias)).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "html_url": f"https://github.com/x/y/releases/tag/{tag}", "prerelease": pre, "draft": draft}


class Red:
    """GET falso: busca la primera clave que sea parte de la URL; una excepción como valor se lanza. Graba las URLs."""

    def __init__(self, respuestas):
        self.respuestas, self.urls = respuestas, []

    def __call__(self, url, timeout=20):
        self.urls.append(url)
        for clave, valor in self.respuestas.items():
            if clave in url:
                if isinstance(valor, Exception):
                    raise valor
                return valor
        return []


def respuestas_normales():
    return {
        "ggml-org/llama.cpp/releases/latest": rel("v1.0.0", 12, nombre="v1.0.0"),
        "ggml-org/llama.cpp/compare/33aeb17...master": {"ahead_by": 26, "behind_by": 0},
        "ggml-org/llama.cpp/compare/33aeb17...v1.0.0": {"ahead_by": 0, "behind_by": 361},
        "repos/ggml-org/llama.cpp": {"default_branch": "master"},
        "openai/codex/releases": [rel("v2", 1), rel("v2-alpha", 1, pre=True), rel("v1", 20), rel("viejo", 60), rel("borrador", 2, draft=True)],
        "anthropics/claude-code/releases": [rel("c1", 3)],
        "huggingface.co/api/models": [{"id": "Qwen/Nuevo", "createdAt": "2026-11-20T10:00:00.000Z"},
                                      {"id": "Qwen/Viejo", "createdAt": "2026-08-01T10:00:00.000Z"}],
    }


@unittest.skipUnless(HAY_BENCH, "bench/ no está (el instalador no lo lleva)")
class TestFuentes(unittest.TestCase):
    def test_la_lista_real_es_valida(self):
        fuentes = R.cargar_fuentes()
        self.assertGreaterEqual(len(fuentes), 10)
        self.assertEqual({f["tipo"] for f in fuentes}, set(R.TIPOS))
        for f in fuentes:
            self.assertTrue(f["importa"])
            if f["tipo"] == "web":
                self.assertTrue(f["url"].startswith("https://"))

    def test_rechaza_fuentes_mal_escritas(self):
        malas = [
            [{"id": "a", "tipo": "github", "repo": "x/y", "importa": "i"}, {"id": "a", "tipo": "github", "repo": "x/z", "importa": "i"}],
            [{"id": "a", "tipo": "otro", "importa": "i"}],
            [{"id": "a", "tipo": "github", "repo": "sin-barra", "importa": "i"}],
            [{"id": "a", "tipo": "github", "repo": "x/y"}],
            [{"id": "a", "tipo": "huggingface", "importa": "i"}],
            [{"id": "a", "tipo": "web", "url": "http://inseguro.example", "que_buscar": "q", "importa": "i"}],
            [{"id": "a", "tipo": "web", "url": "https://ok.example", "importa": "i"}],
        ]
        for fuentes in malas:
            with self.subTest(f=fuentes), tempfile.TemporaryDirectory() as t:
                p = Path(t) / "f.json"
                p.write_text(json.dumps({"fuentes": fuentes}), encoding="utf-8")
                with self.assertRaises(ValueError):
                    R.cargar_fuentes(p)


@unittest.skipUnless(HAY_BENCH, "bench/ no está (el instalador no lo lleva)")
class TestRevision(unittest.TestCase):
    def test_releases_filtradas_por_fecha_y_borradores(self):
        red = Red(respuestas_normales())
        r = R.releases_nuevas("openai/codex", AHORA - timedelta(days=30), red)
        self.assertEqual([x["tag"] for x in r], ["v2", "v2-alpha", "v1"])

    def test_primera_vez_mira_30_dias_y_las_prereleases_solo_se_cuentan(self):
        fuentes = [f for f in R.cargar_fuentes() if f["id"] == "codex"]
        res = R.revisar(fuentes, {}, AHORA, None, LOCAL, Red(respuestas_normales()))["codex"]
        self.assertEqual([x["tag"] for x in res["releases"]], ["v2", "v1"])
        self.assertEqual(res["prereleases"], 1)
        self.assertEqual(res["desde"], "2026-10-26")

    def test_desde_manda_sobre_lo_visto(self):
        fuentes = [f for f in R.cargar_fuentes() if f["id"] == "codex"]
        previo = {"fuentes": {"codex": AHORA.isoformat()}}
        r1 = R.revisar(fuentes, previo, AHORA, None, LOCAL, Red(respuestas_normales()))["codex"]
        self.assertEqual(r1["releases"], [])
        r2 = R.revisar(fuentes, previo, AHORA, AHORA - timedelta(days=5), LOCAL, Red(respuestas_normales()))["codex"]
        self.assertEqual([x["tag"] for x in r2["releases"]], ["v2"])

    def test_llama_cuenta_la_distancia_y_no_lista_compilaciones(self):
        fuentes = [f for f in R.cargar_fuentes() if f["id"] == "llama-cpp"]
        red = Red(respuestas_normales())
        res = R.revisar(fuentes, {}, AHORA, None, LOCAL, red)["llama-cpp"]
        self.assertEqual([x["tag"] for x in res["releases"]], ["v1.0.0"])
        self.assertEqual(res["comparacion"]["nuevos_en_rama"], 26)
        self.assertEqual(res["comparacion"]["locales_sobre_release"], 361)
        self.assertFalse(any("releases?per_page" in u for u in red.urls))

    def test_una_fuente_caida_no_tumba_las_demas(self):
        respuestas = {"openai/codex": OSError("HTTP 500"), **respuestas_normales()}
        res = R.revisar(R.cargar_fuentes(), {}, AHORA, None, LOCAL, Red({"openai/codex": OSError("HTTP 500"), **respuestas}))
        self.assertFalse(res["codex"]["ok"])
        self.assertIn("HTTP 500", res["codex"]["motivo"])
        self.assertTrue(res["claude-code"]["ok"])
        self.assertTrue(res["qwen-modelos"]["ok"])
        self.assertEqual([m["id"] for m in res["qwen-modelos"]["modelos"]], ["Qwen/Nuevo"])

    def test_respuesta_rara_tampoco_tumba(self):
        res = R.revisar(R.cargar_fuentes(), {}, AHORA, None, LOCAL, Red({"openai/codex": {"no": "es una lista"}, **respuestas_normales()}))
        self.assertFalse(res["codex"]["ok"])
        self.assertTrue(res["claude-code"]["ok"])

    def test_http_get_convierte_la_respuesta_cortada_en_oserror(self):
        import http.client
        from unittest import mock

        class Corta:
            def __enter__(self):
                return self

            def __exit__(self, *a):
                return False

            def read(self):
                raise http.client.IncompleteRead(b"")
        with mock.patch("urllib.request.urlopen", return_value=Corta()) as m, self.assertRaises(OSError) as ctx:
            R.http_get("https://api.github.com/x")
        self.assertEqual(m.call_count, 2)  # lo repite una vez
        self.assertIn("cortada", str(ctx.exception))


@unittest.skipUnless(HAY_BENCH, "bench/ no está (el instalador no lo lleva)")
class TestInforme(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="test_radar_"))
        self.addCleanup(lambda: __import__("shutil").rmtree(self.tmp, ignore_errors=True))

    def correr(self, red, ahora=AHORA, **kw):
        return R.ejecutar(self.tmp, get=red, ahora=ahora, local=LOCAL, **kw)

    def test_informe_completo_con_sus_secciones(self):
        ruta = self.correr(Red(respuestas_normales()))
        texto = ruta.read_text(encoding="utf-8")
        self.assertEqual(ruta.name, "2026-11.md")
        for parte in ("## 1. Estado local", "## 2. Cambios desde la última revisión", "## 3. Para revisar a mano", "## 4. Qué le importa a Aura",
                      "## 5. Qué probar", "PENDIENTE", "v1.0.0", "Qwen/Nuevo", "26 commits por detrás de `master`", "361 commits por encima",
                      "No listadas: 1 prereleases", "agentskills.io", "4.13.20", "v0.2.3"):
            self.assertIn(parte, texto)
        self.assertNotIn("Qwen/Viejo", texto)

    def test_segunda_corrida_no_repite_y_la_fuente_caida_se_vuelve_a_mirar(self):
        red1 = Red({"openai/codex": OSError("HTTP 500"), **respuestas_normales()})
        self.correr(red1)
        ultimo = json.loads((self.tmp / "ultimo.json").read_text(encoding="utf-8"))
        self.assertNotIn("codex", ultimo["fuentes"])
        self.assertIn("claude-code", ultimo["fuentes"])
        texto2 = self.correr(Red(respuestas_normales()), ahora=AHORA + timedelta(hours=1)).read_text(encoding="utf-8")
        seccion_claude = texto2.split("### claude-code")[1].split("###")[0]
        self.assertIn("Nada nuevo", seccion_claude)
        seccion_codex = texto2.split("### codex")[1].split("###")[0]
        self.assertIn("v2", seccion_codex)  # no se había visto: se mira desde la ventana de la primera vez

    def test_no_pisa_informes_anteriores(self):
        a = self.correr(Red(respuestas_normales()))
        a.write_text("rellenado a mano", encoding="utf-8")
        b = self.correr(Red(respuestas_normales()))
        self.assertNotEqual(a, b)
        self.assertEqual(a.read_text(encoding="utf-8"), "rellenado a mano")
        self.assertEqual(b.name, "2026-11_2.md")

    def test_seco_no_avanza_la_ventana(self):
        self.correr(Red(respuestas_normales()), seco=True)
        self.assertFalse((self.tmp / "ultimo.json").exists())

    def test_sin_red_no_pide_nada(self):
        def prohibido(url, timeout=20):
            raise AssertionError(f"pidió {url}")
        ruta = self.correr(prohibido, sin_red=True)
        self.assertIn("Sin red", ruta.read_text(encoding="utf-8"))
        self.assertFalse((self.tmp / "ultimo.json").exists())

    def test_las_urls_pedidas_solo_llevan_lo_de_la_fuente(self):
        red = Red(respuestas_normales())
        self.correr(red)
        repos = "|".join(re.escape(f["repo"]) for f in R.cargar_fuentes() if "repo" in f)
        permitida = re.compile(rf"^https://api\.github\.com/repos/({repos})(/releases(/latest|\?per_page=100)|/compare/[0-9a-f]{{7,40}}\.\.\.[\w.-]+)?$"
                               r"|^https://huggingface\.co/api/models\?author=\w+&sort=createdAt&direction=-1&limit=30$")
        self.assertGreater(len(red.urls), 8)
        for u in red.urls:
            with self.subTest(u=u):
                self.assertRegex(u, permitida)
                self.assertNotIn("\\", u)
                self.assertNotIn("Users", u)

    def test_cli_sin_red_con_el_estado_real(self):
        r = subprocess.run([sys.executable, str(BENCH / "radar.py"), "--sin-red", "--salida", str(self.tmp)], capture_output=True, text=True,
                           encoding="utf-8", errors="replace", timeout=300, check=False, stdin=subprocess.DEVNULL)
        self.assertEqual(r.returncode, 0, r.stderr[-400:])
        self.assertIn("Informe:", r.stdout)
        texto = next(self.tmp.glob("*.md")).read_text(encoding="utf-8")
        version = json.loads((RAIZ / "package.json").read_text(encoding="utf-8"))["version"]
        self.assertIn(f"| Aura | {version} |", texto)

    def test_desde_mal_escrito(self):
        r = subprocess.run([sys.executable, str(BENCH / "radar.py"), "--sin-red", "--desde", "ayer", "--salida", str(self.tmp)],
                           capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=60, check=False, stdin=subprocess.DEVNULL)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("AAAA-MM-DD", r.stderr + r.stdout)


@unittest.skipUnless(HAY_BENCH, "bench/ no está (el instalador no lo lleva)")
class TestLocal(unittest.TestCase):
    def test_version_de_llama_server(self):
        self.assertEqual(R.leer_version_llama("version: 1.0.0-dev (build 1, commit 1094646)\nbuilt with Clang"),
                         {"version": "1.0.0-dev", "build": "1", "commit": "1094646"})
        self.assertEqual(R.leer_version_llama("ggml_cuda_init: x\nversion: 6123 (build 6123, commit abcdef0123)")["commit"], "abcdef0123")
        self.assertIsNone(R.leer_version_llama("otra cosa"))
        self.assertIsNone(R.leer_version_llama(""))

    def test_backend_mas_nuevo_por_numero_no_por_texto(self):
        with tempfile.TemporaryDirectory() as t:
            for v in ("2.9.0", "2.10.0", "2.82.0"):
                (Path(t) / "extensions" / "backends" / f"llama.cpp-win-x86_64-nvidia-cuda12-avx2-{v}").mkdir(parents=True)
            (Path(t) / "extensions" / "backends" / "llama.cpp-win-x86_64-avx2-9.9.9").mkdir()
            self.assertEqual(R.llama_local(Path(t))["backend"], "2.82.0")
            self.assertIsNone(R.llama_local(Path(t) / "no_existe"))

    def test_modelos_instalados(self):
        with tempfile.TemporaryDirectory() as t:
            (Path(t) / "models" / "autor" / "modelo-GGUF").mkdir(parents=True)
            (Path(t) / "models" / "autor" / "suelto.txt").write_text("x", encoding="utf-8")
            self.assertEqual(R.modelos_instalados(Path(t)), ["autor/modelo-GGUF"])
            self.assertEqual(R.modelos_instalados(Path(t) / "no_existe"), [])

    def test_estado_del_repo(self):
        e = R.estado_local()
        self.assertEqual(e["aura"], json.loads((RAIZ / "package.json").read_text(encoding="utf-8"))["version"])
        self.assertRegex(e["sat"], r"^v\d+\.\d+\.\d+$")


if __name__ == "__main__":
    unittest.main()
