# Juegos con pygame (2026)
- Instala `pygame-ce` (`pip install pygame-ce`; se importa igual: `import pygame`). El `pygame` original (2.6.1) solo tiene wheels hasta Python 3.13: con Python 3.14 su instalación falla en Windows.
- Estructura: `config.py` (constantes), `entidades.py` (clases `pygame.sprite.Sprite`), `juego.py` (clase Juego), `main.py` (arranque). Archivos < 250 líneas.
- Estados en UNA variable: `self.estado = "menu" | "jugando" | "pausa" | "fin"`. El bucle hace `if self.estado == ...` en eventos, actualizar y dibujar. Cada estado debe ser alcanzable: menú -> jugando (Enter/Espacio), jugando <-> pausa (P/Esc), vidas <= 0 -> fin, fin -> reiniciar() con R.
- Bucle: `dt = self.reloj.tick(60) / 1000` -> eventos -> `actualizar(dt)` -> `dibujar()` -> `pygame.display.flip()`. Velocidades en píxeles/segundo multiplicadas por dt.
- TODO lo que calculas debe tener efecto. Colisiones:
  `golpes = pygame.sprite.groupcollide(self.enemigos, self.balas, True, True)` y luego `self.puntos += 10 * len(golpes)`.
  `if pygame.sprite.spritecollide(self.nave, self.enemigos, True): self.vidas -= 1` y `if self.vidas <= 0: self.estado = "fin"`.
- `reiniciar()` vacía los grupos y restablece puntos, vidas, nivel y estado; llámala al empezar y al pulsar R.
- Rendimiento: crea fondos, degradados y textos fijos UNA vez en `__init__`, no en cada frame. Elimina sprites que salen de pantalla (`self.kill()`).
- Fuentes: `pygame.font.Font(None, 36)`. No uses emojis (❤, 🚀) en textos: la fuente por defecto no los tiene y salen cuadros. Dibuja corazones con formas o usa texto "Vidas: 3".
- Dificultad: súbela por tiempo o al pasar umbrales de puntos (guarda el último umbral), no `if puntos % 10 == 0` en cada frame.
- Tests (pytest): antes de importar pygame en el test pon `os.environ["SDL_VIDEODRIVER"] = "dummy"` y `os.environ["SDL_AUDIODRIVER"] = "dummy"`. Prueba la lógica: bala+enemigo solapados -> suben puntos y desaparecen; nave golpeada -> baja vida; vidas 0 -> estado "fin"; reiniciar() -> valores iniciales.
- `verificar` ejecuta el juego sin ventana con teclas aleatorias: informa excepciones, FPS y si la pantalla cambia.
- run.bat: `crear_bat("run", ["python main.py"], usar_venv=true)` en la MISMA carpeta que main.py.
