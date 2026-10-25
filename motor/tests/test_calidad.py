"""Pruebas de las herramientas de calidad (sin modelo): python -m unittest discover -s tests -v"""
import importlib.util
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_agente import borrar

from agente import conocimiento, lecciones
from agente.herramientas import Herramientas
from agente.verificacion import verificar

HAY_NAVEGADOR = importlib.util.find_spec("playwright") is not None
HAY_JAVA = shutil.which("javac") is not None


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_calidad_"))
        self.h = Herramientas(self.dir, max_chars=8000, timeout=30)

    def tearDown(self):
        self.h.detener_todos()
        borrar(self.dir)

    def escribir(self, ruta, texto):
        p = self.dir / ruta
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(texto, encoding="utf-8")


class TestVerificarPython(Base):
    def test_detecta_logica_sin_conectar_y_test_falso(self):
        self.escribir("main.py", (
            "class Juego:\n"
            "    def __init__(self):\n        self.puntos = 0\n\n"
            "    def colisiones(self, a, b):\n        golpes = [x for x in a if x in b]\n        return None\n\n"
            "    def mostrar_menu(self):\n        print('menu')\n\n"
            "def main():\n    j = Juego()\n    j.colisiones([1], [1])\n    print(j.puntos)\n\n"
            "if __name__ == '__main__':\n    main()\n"))
        self.escribir("test_main.py", "import os\n\ndef test_nada():\n    assert os.sep\n")
        informe, _errores = verificar(self.dir, sys.executable, segundos=6)
        self.assertIn("golpes", informe)                     # ruff F841
        self.assertIn("mostrar_menu", informe)               # vulture: método nunca usado
        self.assertIn("NO prueba tu código", informe)        # test falso
        self.assertIn("main.py se ejecuta sin errores", informe)

    def test_clase_nunca_usada_es_error_y_decorada_no(self):
        self.escribir("app.py", "def ruta(f):\n    return f\n\n@ruta\ndef listar():\n    return []\n\n"
                                "class Jugador:\n    pass\n\nif __name__ == '__main__':\n    print(ruta)\n")
        informe, _errores = verificar(self.dir, sys.executable)
        self.assertIn("clase 'Jugador' NUNCA se usa", informe)
        self.assertNotIn("'listar'", informe)

    def test_error_en_ejecucion_y_nombre_indefinido(self):
        self.escribir("main.py", "def main():\n    print(valor_inexistente)\n\nif __name__ == '__main__':\n    main()\n")
        informe, errores = verificar(self.dir, sys.executable, segundos=6)
        self.assertGreaterEqual(errores, 1)
        self.assertIn("NO definido", informe)

    def test_tests_que_fallan(self):
        self.escribir("calc.py", "def suma(a, b):\n    return a - b\n")
        self.escribir("test_calc.py", "import unittest\nfrom calc import suma\n\nclass T(unittest.TestCase):\n"
                                      "    def test_suma(self):\n        self.assertEqual(suma(2, 2), 4)\n")
        informe, _errores = verificar(self.dir, sys.executable)
        self.assertIn("Tests FALLAN", informe)


class TestVerificarBat(Base):
    def test_bat_con_ruta_rota(self):
        self.escribir("juego/main.py", "print(1)\n")
        self.escribir("run.bat", "@echo off\npython main.py\npython -m pip install -r requirements.txt\n")
        informe, _ = verificar(self.dir, sys.executable)
        self.assertIn("ejecuta 'main.py' y ese archivo NO existe", informe)
        self.assertIn('cd /d "%~dp0"', informe)
        self.assertNotIn("'-m", informe)

    def test_bat_servidor_bloquea_lineas_siguientes(self):
        self.escribir("index.html", "<!doctype html><html><body>hola</body></html>")
        self.escribir("run.bat", '@echo off\ncd /d "%~dp0"\necho [1/2] python -m http.server 8000\n'
                                 "python -m http.server 8000\nstart http://localhost:8000\n")
        informe, _ = verificar(self.dir, sys.executable)
        self.assertIn("bloquea", informe)


@unittest.skipUnless(HAY_NAVEGADOR, "playwright no instalado")
class TestVerificarWeb(Base):
    def test_web_rota(self):
        self.escribir("index.html", '<!doctype html><html><head><link rel="stylesheet" href="no.css"></head><body>'
                                    '<canvas id="lienzo" width="200" height="100"></canvas><script src="app.js"></script>'
                                    "</body></html>")
        self.escribir("app.js", 'const m = document.getElementById("marcador");\nfunction bucle(){ dibujar(); '
                                "requestAnimationFrame(bucle); }\nrequestAnimationFrame(bucle);\n")
        informe, _errores = verificar(self.dir, sys.executable)
        for esperado in ("no.css", "id='marcador'", "dibujar is not defined", "VACÍO"):
            self.assertIn(esperado, informe)

    def test_web_que_funciona(self):
        self.escribir("index.html", '<!doctype html><html><body><canvas id="c" width="200" height="100"></canvas>'
                                    '<script src="app.js"></script></body></html>')
        self.escribir("app.js", 'const ctx = document.getElementById("c").getContext("2d"); let x = 0;\n'
                                "function bucle(){ ctx.fillStyle='#000'; ctx.fillRect(0,0,200,100); ctx.fillStyle='#0f0';"
                                " ctx.fillRect(x % 200, 40, 20, 20); x += 2; requestAnimationFrame(bucle); }\n"
                                "requestAnimationFrame(bucle);\n")
        informe, errores = verificar(self.dir, sys.executable)
        self.assertEqual(errores, 0, informe)
        self.assertIn("anima: sí", informe)

    def test_verificar_un_archivo_prueba_el_proyecto(self):
        self.escribir("index.html", '<!doctype html><html><body><script src="app.js"></script></body></html>')
        self.escribir("app.js", "noExiste();\n")
        informe, errores = verificar(self.dir, sys.executable, "index.html")
        self.assertIn("VERIFICACIÓN (web)", informe)
        self.assertGreater(errores, 0, informe)
        (self.dir / "vacia").mkdir()
        informe, errores = verificar(self.dir, sys.executable, "vacia")
        self.assertIn("nada se ha comprobado", informe)

    def test_libreria_cdn_desactualizada(self):
        from agente import verificacion
        actuales = {"three.js": "0.77.1", "three": "0.170.0", "gsap": "3.12.5"}  # "three.js" de npm: abandonado
        self.escribir("index.html", '<script src="https://cdnjs.cloudflare.com/ajax/libs/three.js/r128/three.min.js">'
                                    '</script><script src="https://cdn.jsdelivr.net/npm/gsap@3.12.2/dist/gsap.min.js">'
                                    '</script>')
        self.escribir("nueva.html", '<script type="importmap">{"imports":{"three":'
                                    '"https://cdn.jsdelivr.net/npm/three@0.170.0/build/three.module.js"}}</script>')
        inf = verificacion.Informe()
        with mock.patch.object(verificacion, "ultima_npm", side_effect=actuales.get):
            verificacion.librerias_cdn(self.dir, [self.dir / "index.html", self.dir / "nueva.html"], inf)
        self.assertEqual(len(inf.errores), 1, inf.errores)  # gsap 3.12.2 y three 0.170 están al día
        self.assertIn("three.js 0.128.0 está DESACTUALIZADA (la actual es 0.170.0)", inf.errores[0])
        self.assertIn("importmap", inf.errores[0])

    def test_web_con_canvas_decorativo_no_es_juego(self):
        texto = "".join(f"<p>Producto {i}: camiseta deportiva transpirable con tejido técnico.</p>" for i in range(20))
        self.escribir("index.html", '<!doctype html><html><body><canvas id="c" width="200" height="100"></canvas>'
                                    f'{texto}<script src="app.js"></script></body></html>')
        self.escribir("app.js", 'const ctx = document.getElementById("c").getContext("2d");\n'
                                "ctx.fillStyle = '#123'; ctx.fillRect(0, 0, 200, 100); ctx.fillStyle = '#f80';"
                                " ctx.fillRect(10, 10, 50, 50);\n")
        informe, errores = verificar(self.dir, sys.executable)
        self.assertEqual(errores, 0, informe)
        for juego in ("nada se mueve", "no se anima", "nada cambia al pulsar"):
            self.assertNotIn(juego, informe)


@unittest.skipUnless(HAY_JAVA, "sin JDK")
class TestVerificarJava(Base):
    def test_java_error_de_compilacion(self):
        self.escribir("src/app/Main.java", "package app;\npublic class Main {\n  public static void main(String[] a) {\n"
                                           "    int x = \"texto\";\n  }\n}\n")
        informe, errores = verificar(self.dir, sys.executable)
        self.assertGreaterEqual(errores, 1)
        self.assertIn("Main.java:4", informe)

    def test_java_excepcion(self):
        self.escribir("src/app/Main.java", "package app;\npublic class Main {\n  public static void main(String[] a) {\n"
                                           "    String s = null;\n    System.out.println(s.length());\n  }\n}\n")
        informe, _ = verificar(self.dir, sys.executable)
        self.assertIn("NullPointerException", informe)


class TestConducta(Base):
    def test_lint_inmediato_al_escribir(self):
        r = self.h.escribir("a.py", "def f():\n    x = 1\n    return y\n")
        self.assertIn("NO definido", r)

    def test_reescritura_repetida(self):
        for _ in range(3):
            r = self.h.escribir("a.py", "print(1)\n")
        self.assertIn("CONDUCTA", r)

    def test_bloquea_reescrituras_y_repara_saltos_escapados(self):
        for _ in range(5):  # crear + 3 reescrituras permitidas; la 4ª se bloquea
            r = self.h.escribir("b.py", "print(1)\n")
        self.assertIn("BLOQUEADO", r)
        self.assertIn("editar para cambiar solo", r)
        r = self.h.escribir("c.py", "def f():\\n    return 1\\n\\nprint(f())\\n")
        self.assertIn("escapados corregidos", r)
        self.assertEqual((self.dir / "c.py").read_text(encoding="utf-8"), "def f():\n    return 1\n\nprint(f())\n")

    def test_bloquea_marcadores_y_truncados(self):
        self.assertIn("BLOQUEADO", self.h.escribir("m.py", "[71 líneas omitidas]"))
        self.h.escribir("grande.py", "".join(f"x{i} = {i}\n" for i in range(200)))
        self.assertIn("parece truncado", self.h.escribir("grande.py", "x = 1\n"))
        self.assertIn("x199", (self.dir / "grande.py").read_text(encoding="utf-8"))

    def test_checkpoint_y_restaurar(self):
        self.h.escribir("main.py", "def f():\n    return 1\n\nif __name__ == '__main__':\n    print(f())\n")
        self.assertIn("checkpoint #1", self.h.verificar_cmd())
        self.h.escribir("main.py", "def f():\n    return valor_roto\n\nif __name__ == '__main__':\n    print(f())\n")
        self.assertIn("ERRORES", self.h.verificar_cmd())
        self.assertIn("restaurado el checkpoint #1", self.h.restaurar())
        self.assertIn("return 1", (self.dir / "main.py").read_text(encoding="utf-8"))

    def test_detecta_tests_debilitados(self):
        self.h.escribir("test_a.py", "".join(f"def test_{i}():\n    assert {i} == {i}\n" for i in range(6)))
        r = self.h.escribir("test_a.py", "def test_0():\n    assert True\n")
        self.assertIn("los tests bajaron de 6 a 1", r)

    def test_marcar_prueba_sin_ejecutar_la_verifica_el_motor(self):
        self.h.plan(["crear main.py", "probar el programa"])
        self.h.paso = 1
        self.h.escribir("main.py", "print(1)\n")
        self.h.paso = 2
        r = self.h.paso_hecho(2)  # sin ejecución tras escribir: verifica ahora en vez de rechazar
        self.assertIn("marcado como hecho (verificado ahora: 0 errores)", r)
        self.assertEqual(self.h.paso_prueba, 2)
        self.assertEqual(self.h.verificacion["errores"], 0)

    def test_marcar_prueba_con_errores_la_rechaza_con_el_informe(self):
        self.h.plan(["crear calc.py y su test", "ejecutar los tests"])
        self.h.paso = 1
        self.h.escribir("calc.py", "def doble(x):\n    return x * 3\n")
        self.h.escribir("test_calc.py", "import unittest\nfrom calc import doble\n\n\nclass T(unittest.TestCase):\n"
                                        "    def test_doble(self):\n        self.assertEqual(doble(2), 4)\n")
        self.h.paso = 2
        r = self.h.paso_hecho(2)
        self.assertIn("RECHAZADO", r)
        self.assertIn("he verificado ahora", r)
        self.assertGreater(self.h.verificacion["errores"], 0)
        self.assertIn("[ ] 2.", self.h.plan_texto())
        self.h.paso = 3
        self.h.ejecutar("python -c \"print('ok')\"")  # una ejecución con éxito sigue valiendo como antes
        self.assertIn("marcado como hecho.", self.h.paso_hecho(2))

    def test_paso_que_crea_tests_sin_el_archivo_no_verifica(self):
        self.h.plan(["Crear calc.py", "Crear test_calc.py con tests de la lógica"])
        self.h.paso = 1
        self.h.escribir("calc.py", "def doble(x):\n    return x * 2\n")
        self.h.paso = 2
        antes = dict(self.h.verificacion)
        r = self.h.paso_hecho(2)  # falta el archivo: rechazo barato, sin lanzar la verificación completa
        self.assertIn("pide test_calc.py y todavía no existe", r)
        self.assertEqual(self.h.verificacion, antes)

    def test_verificar_con_una_url_revisa_el_proyecto(self):
        self.h.escribir("main.py", "print(1)\n")
        r = self.h.verificar_cmd("http://localhost:8000")
        self.assertTrue(r.startswith("VERIFICACIÓN"), r[:80])
        self.assertIn("para mirar una URL usa ver_pantalla", r)
        self.assertEqual(self.h.verificacion["ruta"], ".")

    def test_commit_sin_repositorio_lo_crea(self):
        self.h.escribir("main.py", "print(1)\n")
        r = self.h.git("commit", mensaje="Primera versión")
        self.assertIn("Repositorio git creado", r)
        self.assertTrue((self.dir / ".git").is_dir())
        self.assertIn("Primera versión", self.h.git("log"))
        self.h.escribir("main.py", "print(2)\n")
        self.assertNotIn("Repositorio git creado", self.h.git("commit", mensaje="Segunda"))

    def test_commit_dentro_de_otro_repositorio_no_crea_uno(self):
        subprocess.run(["git", "init", "-q"], cwd=self.dir, check=True)
        sub = self.dir / "app"
        sub.mkdir()
        h = Herramientas(sub, max_chars=8000, timeout=30)
        h.escribir("main.py", "print(1)\n")
        self.assertIn("no es un repositorio git", h.git("commit", mensaje="x"))
        self.assertFalse((sub / ".git").exists())

    def test_no_marcar_creacion_sin_el_archivo(self):
        self.h.plan(["Crear css/styles.css con los estilos", "Crear el hero 3D con Three.js"])
        r = self.h.paso_hecho(1)
        self.assertIn("RECHAZADO", r)
        self.assertIn("styles.css", r)
        self.h.escribir("css/styles.css", "body { margin: 0; }\n")
        self.assertIn("marcado como hecho", self.h.paso_hecho(1))
        self.assertIn("marcado como hecho", self.h.paso_hecho(2))  # three.js es una librería, no un archivo

    def test_aprender(self):
        destino = self.dir / "lecciones.md"
        with mock.patch.object(lecciones, "ARCHIVO", destino), \
                mock.patch.object(lecciones, "USO", self.dir / "lecciones_uso.json"):
            self.assertIn("guardada", self.h.aprender("En pygame no uses emojis en textos: la fuente no los tiene."))
            self.assertIn("ya estaba", self.h.aprender("En pygame no uses emojis en textos: la fuente no los tiene."))
        self.assertIn("emojis", destino.read_text(encoding="utf-8"))


class TestConocimiento(unittest.TestCase):
    def test_guias_y_alias(self):
        self.assertIn("pygame-ce", conocimiento.guia("juego"))
        self.assertIn("requestAnimationFrame", conocimiento.guia("canvas"))
        self.assertIn("Disponibles", conocimiento.guia("cobol"))


if __name__ == "__main__":
    unittest.main()
