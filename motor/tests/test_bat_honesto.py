"""Dos fallos de una misión real (horario de menor coste, Aura 4.13.14/4.13.15):
- el run.bat llamaba «resolver_sat problema.opb», que es una herramienta de Aura y no un comando de Windows: con doble clic,
  «"resolver_sat" no se reconoce»; ni crear_bat ni verificar lo vieron;
- resolver.bat y el resumen decían «regenera el .opb, lo resuelve y muestra el resultado», pero solo se regeneraba el .opb:
  decodificar.py mostraba el horario.modelo que Aura había calculado, y con otros datos seguía enseñando la solución vieja."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.herramientas import Herramientas
from agente.proyecto import Informe
from agente.verificacion import HERRAMIENTAS_AURA, bat_sin_calculo, bats, chequeo_rapido, verificar

CABECERA = '@echo off\r\nsetlocal EnableExtensions\r\nchcp 65001 >nul\r\ncd /d "%~dp0"\r\n'


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_bat_honesto_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)
        self.h = Herramientas(self.dir, max_chars=3000, timeout=30)
        self.addCleanup(self.h.detener_todos)

    def escribir(self, ruta: str, texto: str):
        p = self.dir / ruta
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(texto.encode("utf-8"))
        return p


class CrearBat(Base):
    def test_rechaza_cada_herramienta_de_aura_como_comando(self):
        self.assertIn("resolver_sat", HERRAMIENTAS_AURA)
        self.assertNotIn("git", HERRAMIENTAS_AURA)
        self.assertGreaterEqual(len(HERRAMIENTAS_AURA), 20)
        for nombre in sorted(HERRAMIENTAS_AURA):
            with self.subTest(nombre):
                r = self.h.crear_bat("run.bat", ["python generar.py", f"{nombre} problema.opb"])
                self.assertTrue(r.startswith("ERROR"), r)
                self.assertIn(f"'{nombre}'", r)
                self.assertFalse((self.dir / "run.bat").exists())

    def test_con_call_start_mayusculas_y_encadenado(self):
        for orden in ("call resolver_sat problema.opb", 'start "" /min cmd /k resolver_sat problema.opb', "RESOLVER_SAT problema.opb",
                      "python a.py && resolver_sat problema.opb", "python a.py & resolver_sat problema.opb",
                      '"resolver_sat" problema.opb', "cd /d %~dp0 && calcular 2+2"):
            with self.subTest(orden):
                self.assertTrue(self.h.crear_bat("run.bat", [orden]).startswith("ERROR"))
                self.assertFalse((self.dir / "run.bat").exists())

    def test_el_mensaje_dice_que_hacer(self):
        r = self.h.crear_bat("run.bat", ["resolver_sat problema.opb"])
        self.assertIn("herramienta de Aura", r)
        self.assertIn("no existe como comando de Windows", r)
        self.assertIn("script del proyecto", r)
        self.assertIn("No he creado run.bat", r)

    def test_acepta_programas_reales_y_lo_que_tiene_su_propio_bat(self):
        self.escribir("main.py", "print(1)\n")
        r = self.h.crear_bat("run.bat", ["git status", "python main.py", "echo resolver_sat no es una orden aquí"])
        self.assertTrue(r.startswith("OK"), r)
        self.escribir("calcular.bat", "@echo off\r\necho hola\r\n")  # el proyecto tiene su propio «calcular»
        self.assertTrue(self.h.crear_bat("todo.bat", ["calcular"]).startswith("OK"))


class VerificarBat(Base):
    def errores(self, cuerpo: str) -> list[str]:
        bat = self.escribir("run.bat", CABECERA + cuerpo)
        inf = Informe()
        bats(self.dir, [bat], inf)
        return [e for e in inf.errores if "herramienta de Aura" in e]

    def test_un_bat_con_resolver_sat_es_error_con_archivo_y_linea(self):
        self.escribir("codificar_opb.py", "print('x')\n")
        errores = self.errores('echo [3/4] resolver_sat problema.opb\r\nresolver_sat problema.opb\r\nif errorlevel 1 (goto :error)\r\n')
        self.assertEqual(len(errores), 1)  # la línea con echo no cuenta
        self.assertIn("run.bat:6 lanza 'resolver_sat'", errores[0])
        self.assertIn("«\"resolver_sat\" no se reconoce»", errores[0])
        self.assertIn("script del proyecto", errores[0])
        self.assertTrue(self.errores("calcular 2+2\r\n"))
        self.assertTrue(self.errores('call resolver_sat problema.opb\r\n'))
        self.assertTrue(self.errores('start "" /min cmd /k resolver_sat problema.opb\r\n'))

    def test_lo_que_no_es_una_orden_no_se_acusa(self):
        for cuerpo in ("echo resolver_sat problema.opb\r\n", "rem resolver_sat problema.opb\r\n", ":: resolver_sat\r\n",
                       "python resolver_sat.py\r\n", "call :resolver_sat\r\n", 'set "x=resolver_sat"\r\n', "git status\r\n",
                       "goto :resolver_sat\r\n"):
            with self.subTest(cuerpo):
                self.assertEqual(self.errores(cuerpo), [])

    def test_si_el_proyecto_tiene_ese_bat_no_se_acusa(self):
        self.escribir("resolver_sat.bat", "@echo off\r\necho propio\r\n")
        self.assertEqual(self.errores("call resolver_sat problema.opb\r\n"), [])

    def test_tambien_al_escribir_el_bat(self):
        bat = self.escribir("run.bat", CABECERA + "resolver_sat problema.opb\r\n")
        self.assertIn("herramienta de Aura", chequeo_rapido(self.dir, bat))


class BatQuePrometeResolver(Base):
    def proyecto(self, bat: str, nombre_bat: str = "resolver.bat", modelo: bool = True, extra: dict | None = None):
        self.escribir("generar_opb.py", 'with open("horario.opb", "w") as f:\n    f.write("min: +1 x1 ;\\n")\n')
        self.escribir("decodificar.py", 'def cargar(ruta="horario.modelo"):\n    with open(ruta) as f:\n        return f.read()\n'
                                        'if __name__ == "__main__":\n    print(cargar())\n')
        if modelo:
            self.escribir("horario.modelo", "v 1 -2 0\n")
        self.escribir(nombre_bat, CABECERA + bat)
        for ruta, texto in (extra or {}).items():
            self.escribir(ruta, texto)
        return list(self.dir.glob("*.bat"))

    def errores(self, bats_: list[Path]) -> list[str]:
        inf = Informe()
        bat_sin_calculo(self.dir, bats_, inf)
        return inf.errores

    def test_el_bat_que_promete_resolver_y_solo_muestra_es_error(self):
        lista = self.proyecto("echo Genera el problema .opb, lo resuelve con el motor SAT y muestra el horario.\r\n"
                              "python generar_opb.py\r\npython decodificar.py\r\n")
        errores = self.errores(lista)
        self.assertEqual(len(errores), 1)
        self.assertIn("resolver.bat dice que", errores[0])
        self.assertIn("horario.modelo", errores[0])
        self.assertIn("calculó Aura con resolver_sat", errores[0])
        self.assertIn("muestra la solución calculada", errores[0])
        self.assertIn("scipy.optimize.milp", errores[0])

    def test_basta_el_nombre_del_bat(self):
        lista = self.proyecto("python decodificar.py\r\n", nombre_bat="resolver.bat")
        self.assertEqual(len(self.errores(lista)), 1)

    def test_entra_en_verificar(self):
        self.proyecto("echo lo resuelve\r\npython decodificar.py\r\n")
        informe, errores = verificar(self.dir, sys.executable)
        self.assertGreaterEqual(errores, 1)
        self.assertIn("nada del proyecto calcula la solución", informe)

    def test_sin_error_si_el_proyecto_calcula(self):
        for extra in ({"solver.py": "from scipy.optimize import milp\nmilp([1])\n"},
                      {"solver.py": "from pysat.solvers import Glucose3\n"},
                      {"solver.py": "import z3\n"},
                      {"solver.py": "from ortools.sat.python import cp_model\n"},
                      {"solver.py": 'open("horario.modelo", "w").write("v 1 0")\n'}):
            with self.subTest(next(iter(extra.values()))[:30]):
                lista = self.proyecto("echo lo resuelve\r\npython solver.py\r\n", extra=extra)
                self.assertEqual(self.errores(lista), [])

    def test_sin_error_si_el_bat_dice_la_verdad(self):
        lista = self.proyecto("echo Muestra la solución calculada por Aura.\r\npython decodificar.py\r\n", nombre_bat="ver_horario.bat")
        self.assertEqual(self.errores(lista), [])

    def test_sin_error_si_no_hay_solucion_guardada_ni_bat(self):
        lista = self.proyecto("echo lo resuelve\r\npython decodificar.py\r\n", modelo=False)
        self.assertEqual(self.errores(lista), [])
        (self.dir / "horario.modelo").write_text("v 1 0\n", encoding="utf-8")
        self.assertEqual(self.errores([]), [])

    def test_una_libreria_solo_en_un_test_no_cuenta(self):
        lista = self.proyecto("echo lo resuelve\r\npython decodificar.py\r\n",
                              extra={"test_horario.py": "from scipy.optimize import milp\n"})
        self.assertEqual(len(self.errores(lista)), 1)


if __name__ == "__main__":
    unittest.main()
