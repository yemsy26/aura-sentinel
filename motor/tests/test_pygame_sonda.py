"""verificar() con juegos pygame mínimos (sin modelo): python -m unittest tests.test_pygame_sonda -v"""
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from agente.verificacion import verificar

HAY_PYGAME = subprocess.run(["python", "-c", "import pygame"], capture_output=True, check=False).returncode == 0
# Menú que espera ENTER; {empezar} es lo que hace al pulsarlo. Jugando, un cuadrado se mueve.
JUEGO = '''import pygame


class Juego:
    def __init__(self):
        pygame.init()
        self.pantalla = pygame.display.set_mode((320, 240))
        self.reloj = pygame.time.Clock()
        self.fuente = pygame.font.Font(None, 30)
        self.reiniciar()

    def reiniciar(self):
        self.x = 0
        self.estado = "menu"

    def ejecutar(self):
        while True:
            for evento in pygame.event.get():
                if evento.type == pygame.QUIT:
                    pygame.quit()
                    return
                if self.estado == "menu" and evento.type == pygame.KEYDOWN and evento.key == pygame.K_RETURN:
{empezar}
            self.pantalla.fill((20, 20, 40))
            if self.estado == "menu":
                self.pantalla.blit(self.fuente.render("Pulsa ENTER para jugar", True, (255, 255, 255)), (20, 100))
            else:
                self.x = (self.x + 3) % 300
                pygame.draw.rect(self.pantalla, (0, 200, 255), (self.x, 100, 20, 20))
            pygame.display.flip()
            self.reloj.tick(60)


if __name__ == "__main__":
    Juego().ejecutar()
'''
SANGRIA = " " * 20


@unittest.skipUnless(HAY_PYGAME, "pygame no instalado")
class TestPantallaFija(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_sonda_"))

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def verificar(self, codigo):
        (self.dir / "main.py").write_text(codigo, encoding="utf-8")
        return verificar(self.dir, "python", segundos=3)

    def test_reiniciar_tras_empezar_deja_el_menu_atascado(self):
        # El fallo real de una misión: ENTER pone 'jugando' y reiniciar() lo devuelve a 'menu' en el mismo frame.
        informe, errores = self.verificar(JUEGO.format(empezar=f"{SANGRIA}self.estado = 'jugando'\n"
                                                                f"{SANGRIA}self.reiniciar()"))
        self.assertGreaterEqual(errores, 1)
        self.assertIn("ATASCADO", informe)
        self.assertRegex(informe, r"ENTER×\d+")
        self.assertIn("'Pulsa ENTER para jugar'", informe)
        self.assertIn("reiniciar()", informe)

    def test_menu_que_arranca_bien_pasa(self):
        informe, _errores = self.verificar(JUEGO.format(empezar=f"{SANGRIA}self.reiniciar()\n"
                                                                 f"{SANGRIA}self.estado = 'jugando'"))
        self.assertNotIn("NUNCA cambia", informe)
        self.assertIn("3 partidas simuladas", informe)

    def test_pantalla_vacia_y_driver_cambiado(self):
        informe, errores = self.verificar(
            "import os\nimport pygame\npygame.init()\nv = pygame.display.set_mode((320, 240))\n"
            "os.environ['SDL_VIDEODRIVER'] = ''\nr = pygame.time.Clock()\nwhile True:\n"
            "    for e in pygame.event.get():\n        if e.type == pygame.QUIT:\n            pygame.quit()\n"
            "            raise SystemExit\n    v.fill((0, 0, 0))\n    pygame.display.flip()\n    r.tick(60)\n")
        self.assertGreaterEqual(errores, 1)
        self.assertIn("VACÍA", informe)
        self.assertIn("cambia SDL_VIDEODRIVER", informe)


if __name__ == "__main__":
    unittest.main()
