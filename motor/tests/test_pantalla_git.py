"""Pruebas de ver_pantalla y git (sin modelo): python -m unittest discover -s tests -v"""
import asyncio
import importlib.util
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from agente import llm, pantalla
from agente.bucle import ICONOS, coaccionar
from agente.herramientas import ESQUEMAS, PELIGROSOS, Herramientas, despachar

HAY_GIT = shutil.which("git") is not None
HAY_NAVEGADOR = importlib.util.find_spec("playwright") is not None
HAY_PYGAME = subprocess.run(["python", "-c", "import pygame"], capture_output=True, check=False).returncode == 0
HAY_MCP = importlib.util.find_spec("mcp") is not None
IDENTIDAD = {"GIT_AUTHOR_NAME": "Prueba", "GIT_AUTHOR_EMAIL": "prueba@local", "GIT_COMMITTER_NAME": "Prueba",
             "GIT_COMMITTER_EMAIL": "prueba@local"}
SIN_VISION = (None, "qwen-texto", "el modelo cargado (qwen-texto) no acepta imágenes")
# Página con un canvas a pantalla completa detrás de los títulos (como un fondo 3D); {dibujo} pinta en el canvas.
ESCENA = ("<!doctype html><html><head><meta charset=utf-8></head><body style='margin:0;background:#1a1a2e;overflow:hidden'>"
          "<canvas id=c style='position:fixed;inset:0'></canvas>{titulos}<script>const c = document.getElementById('c'), "
          "x = c.getContext('2d'); c.width = innerWidth; c.height = innerHeight; {dibujo}</script></body></html>")
TITULOS = ("<h1 style='position:relative;margin:0;padding-top:300px;text-align:center;font:900 64px sans-serif;color:#fff'>"
           "Ropa Deportiva</h1><h2 style='position:absolute;top:560px;width:100%;margin:0;text-align:center;"
           "font:900 64px sans-serif;background:linear-gradient(90deg,#7c3aed,#f97316);-webkit-background-clip:text;"
           "color:transparent'>Premium</h2><nav style='position:fixed;top:0;left:0;right:0;padding:20px;"
           "background:#1a1a2e'><a href='#' style='color:#ddd'>Inicio</a></nav>")


def _palabra(t, nucleo=500, mezcla=0.0, bordes=0.0, cubierta=0.0, rango=80, movido=0.0):
    return {"t": t, "nucleo": nucleo, "mezcla": mezcla, "bordes": bordes, "cubierta": cubierta, "rango": rango,
            "movido": movido}


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_pg_"))
        self.h = Herramientas(self.dir, max_chars=6000, timeout=30)

    def tearDown(self):
        self.h.detener_todos()
        shutil.rmtree(self.dir, ignore_errors=True)

    def escribir(self, ruta, texto):
        p = self.dir / ruta
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(texto, encoding="utf-8")


class TestRegistro(unittest.TestCase):
    def test_esquemas_iconos_y_despacho(self):
        nombres = {e["function"]["name"] for e in ESQUEMAS}
        self.assertTrue({"ver_pantalla", "git"} <= nombres)
        self.assertTrue({"ver_pantalla", "git"} <= set(ICONOS))
        self.assertEqual(coaccionar("ver_pantalla", {"segundos": "3", "x": 1}), {"segundos": 3})
        d = Path(tempfile.mkdtemp(prefix="agente_pg_"))
        try:
            self.assertIn("accion debe ser", despachar(Herramientas(d), "git", {"accion": "volar"}))
            self.assertIn("No encontré qué mostrar", despachar(Herramientas(d), "ver_pantalla", {}))
        finally:
            shutil.rmtree(d, ignore_errors=True)

    @unittest.skipUnless(HAY_MCP, "mcp no instalado")
    def test_mcp_expone_las_herramientas(self):
        d = tempfile.mkdtemp(prefix="agente_pg_")
        try:
            with mock.patch.dict(os.environ, {"AGENTE_RAIZ": d}):
                spec = importlib.util.spec_from_file_location("agente_mcp_prueba", Path(__file__).resolve().parent.parent / "agente_mcp.py")
                mod = importlib.util.module_from_spec(spec)
                spec.loader.exec_module(mod)
            nombres = {t.name for t in asyncio.run(mod.mcp.list_tools())}
            self.assertTrue({"ver_pantalla", "git"} <= nombres)
            self.assertIn("nunca hace push", mod.git("otro", comando="push"))
        finally:
            shutil.rmtree(d, ignore_errors=True)


@unittest.skipUnless(HAY_GIT, "git no instalado")
class TestGit(Base):
    def setUp(self):
        super().setUp()
        self.entorno = mock.patch.dict(os.environ, IDENTIDAD)
        self.entorno.start()

    def tearDown(self):
        self.entorno.stop()
        for p in self.dir.rglob("*"):  # los objetos de .git son de solo lectura: rmtree no podría borrarlos
            os.chmod(p, 0o777)
        super().tearDown()

    def test_sin_repo_y_crear(self):
        self.assertIn("no es un repositorio git", self.h.git("estado"))
        self.assertTrue(self.h.git("estado").startswith("exit="))  # el IDE lo marca como error
        self.assertIn("Initialized", self.h.git("otro", comando="git init -b main"))
        self.assertIn("No commits yet", self.h.git("estado"))

    def test_no_usa_el_repo_de_una_carpeta_superior(self):
        self.h.git("otro", comando="init")
        dentro = Herramientas(self.dir / "sub")
        self.assertIn("no es un repositorio git", dentro.git("estado"))

    def test_commit_rama_diff_y_log(self):
        self.h.git("otro", comando="init -b main")
        self.escribir("app.py", "print(1)\n")
        self.escribir(".agente/registro.log", "interno\n")
        self.escribir("pkg/__pycache__/app.cpython-312.pyc", "x")
        self.escribir(".venv/Scripts/python.txt", "x")
        estado = self.h.git("estado")
        self.assertIn("app.py", estado)
        for interno in (".agente", "__pycache__", ".venv"):
            self.assertNotIn(interno, estado)
        self.assertIn("app.py", self.h.git("diff"))  # repo sin commits: lista los nuevos
        self.assertIn("mensaje", self.h.git("commit"))
        self.assertIn("primer commit", self.h.git("commit", mensaje="primer commit"))
        archivos = subprocess.run(["git", "ls-files"], cwd=self.dir, capture_output=True, text=True, check=False).stdout.split()
        self.assertEqual(archivos, ["app.py"])  # ni .agente ni cachés ni .venv
        self.assertIn("mejora/menu", self.h.git("rama", nombre="mejora/menu"))
        self.escribir("app.py", "print(2)\n")
        diff = self.h.git("diff")
        self.assertIn("-print(1)", diff)
        self.assertIn("+print(2)", diff)
        self.assertIn("segundo", self.h.git("commit", mensaje="segundo", ruta="app.py"))
        self.assertIn("main", self.h.git("rama", nombre="main"))  # rama existente: cambia a ella
        self.assertEqual((self.dir / "app.py").read_text(), "print(1)\n")
        self.assertIn("primer commit", self.h.git("log"))
        self.assertIn("nombre", self.h.git("rama", nombre="mal nombre"))
        self.assertIn("fuera del proyecto", despachar(self.h, "git", {"accion": "estado", "ruta": "..\\.."}))

    def test_push_nunca_y_destructivo_con_confirmacion(self):
        self.h.git("otro", comando="init -b main")
        self.escribir("a.txt", "uno\n")
        self.h.git("commit", mensaje="base")
        self.escribir("a.txt", "cambiado\n")
        self.assertIn("BLOQUEADO", self.h.git("otro", comando="reset --hard"))  # sin confirmación: no
        self.assertEqual((self.dir / "a.txt").read_text(), "cambiado\n")
        preguntas = []
        si = Herramientas(self.dir, confirmar=lambda c: preguntas.append(c) or True)
        self.assertIn("BLOQUEADO", si.git("otro", comando="push origin main"))  # ni confirmando
        self.assertNotIn("BLOQUEADO", si.git("otro", comando="reset --hard"))
        self.assertEqual(preguntas, ["git reset --hard"])
        self.assertEqual((self.dir / "a.txt").read_text(), "uno\n")
        self.assertNotIn("BLOQUEADO", si.git("otro", comando="show --stat HEAD"))  # lo seguro no pregunta
        self.assertEqual(len(preguntas), 1)

    def test_patron_destructivo(self):
        for c in ("git push", "git reset --hard HEAD~1", "git clean -fd", "git checkout -- a.py", "git checkout .",
                  "git restore a.py", "git branch -D x", "git stash drop", "git rebase main", "git rm a.py",
                  "git commit --amend -m x", "git switch -f main"):
            self.assertTrue(PELIGROSOS.search(c), c)
        for c in ("git status", "git diff", "git restore --staged a.py", "git checkout main", "git switch -c x",
                  "git commit -m 'arreglo'", "git log --oneline", "git branch nueva"):
            self.assertFalse(PELIGROSOS.search(c), c)
        self.assertIn("BLOQUEADO", self.h.ejecutar("git reset --hard"))


class TestVision(unittest.TestCase):
    def test_detecta_si_el_modelo_ve_imagenes(self):
        modelos = [{"key": "q", "loaded_instances": [{"id": "q"}], "capabilities": {"vision": False}},
                   {"key": "g", "loaded_instances": [{"id": "g"}], "capabilities": {"vision": True}}]
        with mock.patch.object(llm, "listar_modelos", return_value=modelos):
            with mock.patch.object(llm, "detectar", return_value=("http://s", "q", 8192)):
                base, _modelo, motivo = pantalla.modelo_vision({})
                self.assertIsNone(base)
                self.assertIn("no acepta imágenes", motivo)
            with mock.patch.object(llm, "detectar", return_value=("http://s", "g", 8192)):
                self.assertEqual(pantalla.modelo_vision({}), ("http://s", "g", ""))
            with mock.patch.object(llm, "detectar", return_value=("http://s", None, 0)):
                self.assertIn("ningún modelo", pantalla.modelo_vision({})[2])
        with mock.patch.object(llm, "detectar", side_effect=llm.ErrorLLM("No responde")):
            self.assertIn("sin conexión", pantalla.modelo_vision({})[2])

    def test_envia_la_imagen_y_devuelve_el_juicio(self):
        enviado = {}

        def falso(url, cuerpo, timeout):
            enviado.update(url=url, cuerpo=cuerpo)
            return {"choices": [{"message": {"content": "<think>x</think>VISUAL: PROBLEMAS\n- botón cortado"}}]}
        with mock.patch.object(pantalla, "modelo_vision", return_value=("http://s", "gemma", "")), \
                mock.patch.object(llm, "http_json", falso):
            texto, fuente = pantalla.evaluar(b"PNG", "la página web index.html", "¿se lee el título?")
        self.assertEqual(enviado["url"], "http://s/v1/chat/completions")
        partes = enviado["cuerpo"]["messages"][0]["content"]
        self.assertEqual(partes[1]["image_url"]["url"], "data:image/png;base64,UE5H")
        self.assertIn("¿se lee el título?", partes[0]["text"])
        self.assertEqual(texto, "VISUAL: PROBLEMAS\n- botón cortado")
        self.assertEqual(fuente, "revisado por gemma")

    def test_servidor_rechaza_imagenes_sin_fallar(self):
        error = llm.ErrorLLM("HTTP 400: The provided messages contain images, but q does not support image inputs.")
        with mock.patch.object(pantalla, "modelo_vision", return_value=("http://s", "q", "")), \
                mock.patch.object(llm, "http_json", side_effect=error):
            self.assertEqual(pantalla.evaluar(b"PNG", "x"), (None, "sin visión: el modelo cargado (q) no acepta imágenes"))


class TestFondoCargado(unittest.TestCase):
    def test_juicio_por_palabra_e_instantes(self):
        limpio = {"texto": "Ropa Premium", "debajo": ["canvas"], "palabras": [_palabra("Ropa"), _palabra("Premium")]}
        malo = {"texto": "Ropa Premium", "debajo": ["canvas"],
                "palabras": [_palabra("Ropa", bordes=0.3), _palabra("Premium", mezcla=0.5, cubierta=0.6)]}
        problemas, peor = pantalla._juzgar_fondo([[limpio], [malo]])
        self.assertEqual(problemas, [("texto sobre fondo cargado: «Ropa Premium» (canvas detrás; se confunde con el "
                                      "fondo: «Premium»; líneas/bordes detrás de «Ropa»; en 1 de 2 instantes: la escena "
                                      "se mueve)")])
        self.assertEqual(peor, 1)  # se guarda la captura del instante en que se ve mal
        self.assertEqual(pantalla._juzgar_fondo([[limpio]]), ([], 0))

    def test_lo_que_no_cuenta(self):
        for p in (_palabra("Hola", nucleo=20, mezcla=0.9),  # casi sin letras visibles
                  _palabra("Hola", rango=5, mezcla=0.9),  # fondo liso: sería poco contraste, no fondo cargado
                  _palabra("Hola", movido=0.3, bordes=0.9)):  # el fondo no se dejó congelar
            self.assertEqual(pantalla._juzgar_fondo([[{"texto": "Hola", "debajo": [], "palabras": [p]}]])[0], [])
        tapada = _palabra("Hola", cubierta=0.6)  # figura detrás: solo cuenta con un canvas o un svg (no foto de fondo)
        self.assertEqual(pantalla._juzgar_fondo([[{"texto": "Hola", "debajo": ["img"], "palabras": [tapada]}]])[0], [])
        self.assertEqual(pantalla._juzgar_fondo([[{"texto": "Hola", "debajo": ["svg"], "palabras": [tapada]}]])[0],
                         ["texto sobre fondo cargado: «Hola» (svg detrás; figura detrás de «Hola»)"])


class TestVerPantalla(Base):
    def test_rechaza_lo_que_no_es_local_ni_interfaz(self):
        self.assertIn("Solo se capturan páginas locales", self.h.ver_pantalla("https://example.com"))
        self.escribir("datos.txt", "x")
        self.assertIn("acepta un .html", self.h.ver_pantalla("datos.txt"))
        self.assertIn("No existe", self.h.ver_pantalla("falta.html"))

    def test_juicio_del_modelo_con_vision(self):
        self.escribir("index.html", "<p>hola</p>")

        def captura(raiz, objetivo, png, espera):
            png.write_bytes(b"PNG")
            return []
        respuesta = {"choices": [{"message": {"content": "VISUAL: PROBLEMAS\n- el título se sale por la derecha"}}]}
        with mock.patch.object(pantalla, "capturar_web", captura), \
                mock.patch.object(pantalla, "modelo_vision", return_value=("http://s", "gemma", "")), \
                mock.patch.object(llm, "http_json", return_value=respuesta):
            r = self.h.ver_pantalla()
        self.assertTrue(r.startswith("VISUAL: PROBLEMAS en index.html (revisado por gemma)"), r)
        self.assertIn("el título se sale", r)
        self.assertTrue((self.dir / ".agente" / "capturas" / "pantalla_index.png").exists())

    def test_veredicto_en_la_ultima_linea(self):
        self.escribir("index.html", "<h1>Ropa</h1>")

        def captura(raiz, objetivo, png, espera):
            png.write_bytes(b"PNG")
            return []
        respuesta = {"choices": [{"message": {"content": "1) Título: sí, una figura 3D lo tapa\n2) Bordes: no\nVISUAL: PROBLEMAS"}}]}
        with mock.patch.object(pantalla, "capturar_web", captura), \
                mock.patch.object(pantalla, "modelo_vision", return_value=("http://s", "qwen", "")), \
                mock.patch.object(llm, "http_json", return_value=respuesta):
            r = self.h.ver_pantalla()
        self.assertTrue(r.startswith("VISUAL: PROBLEMAS en index.html (revisado por qwen)"), r)
        self.assertIn("una figura 3D lo tapa", r)
        self.assertFalse(r.rstrip().endswith("VISUAL: PROBLEMAS"), r)

    @unittest.skipUnless(HAY_NAVEGADOR, "playwright no instalado")
    def test_web_sin_vision_detecta_cortes_y_negro(self):
        self.escribir("index.html", (
            "<!doctype html><html><body style='margin:0'><h1>Marcador</h1>"
            "<div style='position:absolute;left:1500px;top:10px'>Fuera</div>"
            "<p style='width:40px;overflow:hidden;white-space:nowrap'>Texto muy largo que se corta</p></body></html>"))
        self.escribir("negra.html", "<!doctype html><html><body style='background:#000'></body></html>")
        with mock.patch.object(pantalla, "modelo_vision", return_value=SIN_VISION):
            r = self.h.ver_pantalla()
            self.assertTrue(r.startswith("VISUAL: PROBLEMAS en index.html (sin visión:"), r)
            self.assertIn("scroll horizontal", r)
            self.assertIn('"Fuera" se sale de la pantalla', r)
            self.assertIn("texto cortado", r)
            self.assertIn("Solo comprobaciones automáticas", r)
            self.assertTrue((self.dir / ".agente" / "capturas" / "pantalla_index.png").stat().st_size > 1000)
            r = self.h.ver_pantalla("negra.html", segundos=1)
            self.assertIn("NEGRO", r)
            self.assertIn("en blanco", r)

    @unittest.skipUnless(HAY_NAVEGADOR, "playwright no instalado")
    def test_panel_cerrado_fuera_de_pantalla_no_es_problema(self):
        panel = ("<aside style='position:fixed;top:0;right:0;width:300px;height:100%;transform:translateX(100%)'>"
                 "<h3>Tu carrito</h3><button>Finalizar compra</button></aside>")
        self.escribir("index.html", "<!doctype html><html><body style='overflow-x:hidden'><h1>Catálogo</h1>"
                                    f"{panel}</body></html>")
        self.escribir("cortado.html", "<!doctype html><html><body style='overflow-x:hidden'><h1>Catálogo</h1>"
                                      "<div style='position:fixed;left:-30px;top:50px;width:120px'>Puntos: 10</div>"
                                      "</body></html>")
        with mock.patch.object(pantalla, "modelo_vision", return_value=SIN_VISION):
            r = self.h.ver_pantalla("index.html", segundos=1)
            self.assertTrue(r.startswith("VISUAL: OK"), r)  # carrito cerrado, sin scroll horizontal
            r = self.h.ver_pantalla("cortado.html", segundos=1)
            self.assertIn('"Puntos: 10" se sale de la pantalla', r)  # medio fuera: sí es un problema

    @unittest.skipUnless(HAY_NAVEGADOR, "playwright no instalado")
    def test_texto_sobre_fondo_cargado(self):
        self.escribir("malo.html", ESCENA.format(titulos=TITULOS, dibujo=(
            "x.lineWidth = 1; for (let i = 0; i < 60; i++) { x.strokeStyle = i % 2 ? '#e070b0' : '#20d0a0'; x.beginPath();"
            " x.moveTo(380 + i * 4, 240); x.lineTo(300 + (i * 37) % 420, 420); x.lineTo(640 - (i * 23) % 260, 250 + (i * 13) % 160);"
            " x.stroke(); }  x.fillStyle = '#b05aa0'; x.beginPath(); x.arc(512, 600, 70, 0, 7); x.fill();")))
        with mock.patch.object(pantalla, "modelo_vision", return_value=SIN_VISION):
            r = self.h.ver_pantalla("malo.html", segundos=0.5)
        self.assertTrue(r.startswith("VISUAL: PROBLEMAS"), r)
        self.assertIn("texto sobre fondo cargado: «Ropa Deportiva» (canvas detrás; líneas/bordes detrás de «Deportiva»", r)
        self.assertIn("texto sobre fondo cargado: «Premium» (canvas detrás; se confunde con el fondo: «Premium»", r)
        self.assertNotIn("instantes", r)  # dibujo quieto: un solo instante (y el degradado en línea queda como estaba)

    @unittest.skipUnless(HAY_NAVEGADOR, "playwright no instalado")
    def test_fondo_liso_estrellas_y_figura_por_el_borde_no_son_problema(self):
        self.escribir("index.html", ESCENA.format(titulos=TITULOS, dibujo=(
            "const g = x.createRadialGradient(512, 384, 50, 512, 384, 700); g.addColorStop(0, '#3b1d6e');"
            " g.addColorStop(1, '#0b0b1a'); x.fillStyle = g; x.fillRect(0, 0, c.width, c.height);"
            " x.fillStyle = '#fff'; for (let i = 0; i < 400; i++) x.fillRect((i * 7919) % 1024, (i * 104729) % 768, 1, 1);"
            " x.fillStyle = '#c0508a'; x.beginPath(); x.arc(c.width + 150, 300, 220, 0, 7); x.fill();"
            " x.strokeStyle = '#20d0a0'; for (let i = 0; i < 40; i++) { x.beginPath(); x.moveTo(-50 + i * 5, 640);"
            " x.lineTo(150 - i * 3, 800); x.stroke(); }")))
        with mock.patch.object(pantalla, "modelo_vision", return_value=SIN_VISION):
            r = self.h.ver_pantalla(segundos=0.5)
        self.assertTrue(r.startswith("VISUAL: OK"), r)

    @unittest.skipUnless(HAY_NAVEGADOR, "playwright no instalado")
    def test_escena_animada_se_mira_en_varios_instantes(self):
        self.escribir("index.html", ESCENA.format(titulos=TITULOS, dibujo=(
            "const t0 = performance.now(); (function cuadro() { x.clearRect(0, 0, c.width, c.height);"
            " if (performance.now() - t0 > 1500) for (let i = 0; i < 60; i++) { x.strokeStyle = i % 2 ? '#e070b0' : '#20d0a0';"
            " x.beginPath(); x.moveTo(380 + i * 4, 240); x.lineTo(300 + (i * 37) % 420, 420); x.stroke(); }"
            " requestAnimationFrame(cuadro); })();")))
        with mock.patch.object(pantalla, "modelo_vision", return_value=SIN_VISION):
            r = self.h.ver_pantalla(segundos=0.5)
        # el primer instante (~0,6 s) sale limpio; las líneas solo llegan si la animación sigue tras congelarla, y luego
        # ya no cambia nada: el tercer instante sería igual al segundo y no se mira más
        self.assertIn("«Ropa Deportiva» (canvas detrás; líneas/bordes detrás de «Deportiva»; en 1 de 2 instantes: la "
                      "escena se mueve)", r)

    @unittest.skipUnless(HAY_PYGAME, "pygame no instalado")
    def test_pygame_negro_y_con_dibujo(self):
        bucle = ("import pygame\npygame.init()\nv = pygame.display.set_mode((320, 240))\nr = pygame.time.Clock()\n"
                 "while True:\n    for e in pygame.event.get():\n        if e.type == pygame.QUIT:\n"
                 "            pygame.quit()\n            raise SystemExit\n    v.fill((0, 0, 0))\n{dibujo}"
                 "    pygame.display.flip()\n    r.tick(60)\n")
        self.escribir("negro.py", bucle.format(dibujo=""))
        self.escribir("main.py", bucle.format(dibujo="    pygame.draw.rect(v, (200, 60, 30), (40, 40, 120, 80))\n"
                                                    "    pygame.draw.circle(v, (30, 160, 220), (240, 160), 40)\n"))
        with mock.patch.object(pantalla, "modelo_vision", return_value=SIN_VISION):
            r = self.h.ver_pantalla("negro.py", segundos=1)
            self.assertIn("NEGRO", r)
            r = self.h.ver_pantalla(segundos=1)  # sin ruta: elige main.py por ser un juego pygame
            self.assertTrue(r.startswith("VISUAL: OK en main.py"), r)
            self.assertTrue((self.dir / ".agente" / "capturas" / "pantalla_main.png").exists())
            self.escribir("roto.py", "import pygame\nprint('sin ventana')\n")
            self.assertIn("no creó ninguna ventana", self.h.ver_pantalla("roto.py", segundos=1))

    @unittest.skipUnless(HAY_PYGAME, "pygame no instalado")
    def test_pygame_texto_cortado_en_el_borde(self):
        self.escribir("main.py", (
            "import pygame\npygame.init()\nv = pygame.display.set_mode((320, 240))\n"
            "f = pygame.font.Font(None, 36)\ns = pygame.font.SysFont('arial', 30)\nr = pygame.time.Clock()\n"
            "jugando = False\n"
            "while True:\n"
            "    for e in pygame.event.get():\n"
            "        if e.type == pygame.QUIT:\n            pygame.quit()\n            raise SystemExit\n"
            "        if e.type == pygame.KEYDOWN and e.key in (pygame.K_RETURN, pygame.K_SPACE):\n"
            "            jugando = True\n"
            "    v.fill((10, 10, 40))\n"
            "    if jugando:  # el HUD solo aparece en partida: la sonda tiene que empezarla\n"
            "        t = s.render('Puntos: 0', True, (255, 255, 0))\n"
            "        v.blit(t, t.get_rect(center=(20, 20)))\n"
            "    else:\n"
            "        t = f.render('Pulsa ENTER', True, (255, 255, 255))\n"
            "        v.blit(t, t.get_rect(center=(160, 120)))\n"
            "    pygame.display.flip()\n    r.tick(60)\n"))
        with mock.patch.object(pantalla, "modelo_vision", return_value=SIN_VISION):
            r = self.h.ver_pantalla(segundos=1)
        self.assertTrue(r.startswith("VISUAL: PROBLEMAS en main.py"), r)
        self.assertIn("texto cortado por el borde izquierdo: 'Puntos: 0'", r)
        self.assertNotIn("Pulsa ENTER", r)  # centrado: bien
        self.assertNotIn("NEGRO", r)


if __name__ == "__main__":
    unittest.main()
