"""Juega un programa pygame sin ventana con entradas aleatorias. Uso: python pygame_sonda.py main.py SEGUNDOS
- Semilla fija: el resultado es reproducible (si un checkpoint pasa, vuelve a pasar).
- Reloj acelerado: Clock.tick no espera, así en pocos segundos se simulan minutos de partida (choques, game over...).
Imprime una línea 'SONDA {json}' con frames, coste por frame, pantallas distintas, el error si lo hubo, las teclas y
clics enviados, los últimos textos dibujados y si la última pantalla era de un solo color.
Con SONDA_CAPTURA=ruta.png guarda la última pantalla antes de cerrar (y sus colores/brillo en 'captura') y anota el
texto de Font.render que se dibuja medio fuera de la pantalla ('textos_cortados')."""
import hashlib
import json
import os
import random
import re
import runpy
import sys
import time
import traceback
import weakref

os.environ["SDL_VIDEODRIVER"] = "dummy"
os.environ["SDL_AUDIODRIVER"] = "dummy"
destino, duracion = os.path.abspath(sys.argv[1]), float(sys.argv[2])
os.chdir(os.path.dirname(destino))
sys.path.insert(0, os.path.dirname(destino))
sys.argv = [destino]
random.seed(int(os.environ.get("SONDA_SEMILLA", "7")))

import pygame
import pygame.sysfont

TECLAS = {pygame.K_RETURN: "ENTER", pygame.K_SPACE: "ESPACIO", pygame.K_LEFT: "IZQUIERDA", pygame.K_RIGHT: "DERECHA",
          pygame.K_UP: "ARRIBA", pygame.K_DOWN: "ABAJO", pygame.K_a: "A", pygame.K_d: "D", pygame.K_w: "W",
          pygame.K_s: "S", pygame.K_p: "P", pygame.K_r: "R", pygame.K_1: "1"}
estado = {"frames": 0, "pantallas": set(), "inicio": time.time(), "quit_enviado": False, "ms_simulados": 0,
          "captura": None, "teclas": {}, "clics": 0, "ultima": b""}
ULTIMOS_TEXTOS: dict[str, None] = {}  # los últimos textos de Font.render, para decir qué se ve si no cambia nada
_get, _poll, _quit = pygame.event.get, pygame.event.poll, pygame.quit
CAPTURA = os.environ.get("SONDA_CAPTURA")


def _capturar():
    """Una sola vez: al acabar el tiempo, si el juego llama a pygame.quit() o al terminar."""
    s = pygame.display.get_surface() if pygame.display.get_init() else None
    if not CAPTURA or estado["captura"] is not None or s is None:
        return
    try:
        pygame.image.save(s, CAPTURA)
        copia = pygame.Surface(s.get_size(), 0, 32)
        copia.blit(s, (0, 0))
        peq = pygame.transform.smoothscale(copia, (32, 24))  # promedia: los sprites pequeños también cuentan
        px = [tuple(peq.get_at((x, y)))[:3] for x in range(32) for y in range(24)]
        estado["captura"] = {"ancho": s.get_width(), "alto": s.get_height(), "colores": len(set(px)),
                             "brillo": round(sum(map(sum, px)) / (3 * len(px)))}
    except Exception as e:
        estado["captura"] = {"error": str(e)}


def _quit_capturando():
    _capturar()
    _quit()


# ---- texto cortado: el juego dibuja en una copia de la pantalla que anota dónde cae cada texto renderizado ----
TEXTOS = weakref.WeakKeyDictionary()  # superficie de Font.render -> (texto, rect de los píxeles visibles)
CORTES: dict[str, dict] = {}          # texto con los números como N -> veces dibujado, veces cortado, ejemplo
pantallas = {"real": None, "juego": None}
_set_mode, _get_surface, _flip, _update = (pygame.display.set_mode, pygame.display.get_surface, pygame.display.flip,
                                           pygame.display.update)


class FuenteSonda(pygame.font.Font):
    def render(self, texto, *a, **k):
        s = super().render(texto, *a, **k)
        t = str(texto).strip()
        if t:
            ULTIMOS_TEXTOS.pop(t, None)
            ULTIMOS_TEXTOS[t] = None
            if len(ULTIMOS_TEXTOS) > 8:
                del ULTIMOS_TEXTOS[next(iter(ULTIMOS_TEXTOS))]
            if CAPTURA:
                TEXTOS[s] = (str(texto), s.get_bounding_rect())  # sin el margen transparente de la fuente
        return s


def _anotar(pantalla, fuente, destino, area=None):
    if area is not None or fuente not in TEXTOS:
        return
    texto, visible = TEXTOS[fuente]
    try:
        r = visible.move(int(destino[0]), int(destino[1]))
    except (TypeError, IndexError, ValueError):
        return
    ancho, alto = pantalla.get_size()
    c = CORTES.setdefault(re.sub(r"\d+", "N", texto.strip())[:60], {"veces": 0, "cortado": 0})
    c["veces"] += 1
    lados = [n for n, fuera in (("izquierdo", r.left < 0), ("derecho", r.right > ancho), ("superior", r.top < 0),
                                ("inferior", r.bottom > alto)) if fuera]
    if lados and r.colliderect(pantalla.get_rect()):  # del todo fuera no cuenta: entra o sale de escena
        c["cortado"] += 1
        c.setdefault("ejemplo", {"texto": texto[:60], "x": r.x, "y": r.y, "ancho": r.w, "alto": r.h,
                                 "borde": " y ".join(lados)})


class PantallaSonda(pygame.Surface):
    def blit(self, fuente, destino, area=None, special_flags=0):
        _anotar(self, fuente, destino, area)
        return super().blit(fuente, destino, area, special_flags)

    def blits(self, secuencia, doreturn=True):
        secuencia = list(secuencia)
        for item in secuencia:
            _anotar(self, item[0], item[1], item[2] if len(item) > 2 else None)
        return super().blits(secuencia, doreturn)

    if hasattr(pygame.Surface, "fblits"):
        def fblits(self, secuencia, special_flags=0):
            secuencia = list(secuencia)
            for item in secuencia:
                _anotar(self, item[0], item[1])
            return super().fblits(secuencia, special_flags)


def set_mode(*a, **k):
    real = _set_mode(*a, **k)
    if real.get_flags() & pygame.OPENGL:
        return real
    pantallas["real"], pantallas["juego"] = real, PantallaSonda(real.get_size(), 0, real)
    return pantallas["juego"]


def get_surface():
    real = _get_surface()
    return pantallas["juego"] if real is not None and real is pantallas["real"] else real


def _volcar():
    if pantallas["juego"] is not None and _get_surface() is pantallas["real"]:
        pantallas["real"].blit(pantallas["juego"], (0, 0))


def flip():
    _volcar()
    return _flip()


def update(*a, **k):
    _volcar()
    return _update(*a, **k)


def textos_cortados() -> list[dict]:
    """Los que salen cortados en al menos la mitad de las veces (un HUD mal colocado, no un rótulo que pasa)."""
    malos = [dict(c["ejemplo"], veces=c["cortado"], de=c["veces"]) for c in CORTES.values()
             if c["cortado"] >= 2 and c["cortado"] * 2 >= c["veces"]]
    return sorted(malos, key=lambda m: -m["veces"])[:6]


class RelojRapido:
    """Sustituye a pygame.time.Clock: devuelve 16 ms por frame sin dormir (60 FPS simulados, a toda velocidad)."""

    def __init__(self):
        self._ultimo = 16

    def tick(self, fps=0):
        estado["ms_simulados"] += 16
        return 16

    tick_busy_loop = tick

    def get_fps(self):
        return 60.0

    def get_time(self):
        return self._ultimo

    get_rawtime = get_time


def _muestrear():
    s = pygame.display.get_surface()
    if s is not None and estado["frames"] % 25 == 0:
        try:
            estado["ultima"] = pygame.image.tobytes(s, "RGB")
            estado["pantallas"].add(hashlib.md5(estado["ultima"][::97]).hexdigest())
        except Exception:
            pass


def _falsos():
    t = time.time() - estado["inicio"]
    if t > duracion:
        _capturar()
        estado["quit_enviado"] = True
        return [pygame.event.Event(pygame.QUIT)]
    evs = []
    if estado["frames"] > 20 and random.random() < 0.12:  # tras los primeros frames (menú visible)
        k = random.choice(list(TECLAS))
        estado["teclas"][TECLAS[k]] = estado["teclas"].get(TECLAS[k], 0) + 1
        evs += [pygame.event.Event(pygame.KEYDOWN, key=k, mod=0, unicode="", scancode=0),
                pygame.event.Event(pygame.KEYUP, key=k, mod=0, unicode="", scancode=0)]
    if estado["frames"] > 20 and random.random() < 0.02:
        pos = (random.randint(0, 799), random.randint(0, 599))
        estado["clics"] += 1
        evs += [pygame.event.Event(pygame.MOUSEBUTTONDOWN, pos=pos, button=1),
                pygame.event.Event(pygame.MOUSEBUTTONUP, pos=pos, button=1)]
    return evs


def get(*a, **k):
    estado["frames"] += 1
    _muestrear()
    return list(_get(*a, **k)) + _falsos()


def wait(*a, **k):
    ev = _falsos()
    return ev[0] if ev else pygame.event.Event(pygame.KEYDOWN, key=pygame.K_RETURN, mod=0, unicode="", scancode=0)


def poll():
    ev = _falsos()
    return ev[0] if ev else _poll()


class Pulsadas:
    def __getitem__(self, i):
        return random.random() < 0.3

    def __len__(self):
        return 512


pygame.event.get, pygame.event.wait, pygame.event.poll = get, wait, poll
pygame.key.get_pressed = lambda: Pulsadas()
pygame.time.Clock = RelojRapido
pygame.time.delay = pygame.time.wait = lambda ms: 0
pygame.quit = _quit_capturando
pygame.font.Font = pygame.sysfont.Font = FuenteSonda  # SysFont crea la fuente con pygame.sysfont.Font
if CAPTURA:
    pygame.display.set_mode, pygame.display.get_surface = set_mode, get_surface
    pygame.display.flip, pygame.display.update = flip, update

resultado = {"error": None}
try:
    runpy.run_path(destino, run_name="__main__")
except SystemExit:
    pass
except Exception:
    resultado["error"] = "\n".join(traceback.format_exc().strip().splitlines()[-14:])
_capturar()
t = max(0.1, time.time() - estado["inicio"])
ultima = estado["ultima"]
resultado.update(frames=estado["frames"], segundos=round(t, 1), ms_por_frame=round(1000 * t / max(1, estado["frames"]), 1),
                 segundos_de_juego=round(estado["ms_simulados"] / 1000), pantallas_distintas=len(estado["pantallas"]),
                 termino_por_quit=estado["quit_enviado"], captura=estado["captura"], textos_cortados=textos_cortados(),
                 teclas={n: estado["teclas"][n] for n in TECLAS.values() if n in estado["teclas"]}, clics=estado["clics"], textos=list(ULTIMOS_TEXTOS)[-4:],
                 un_color=bool(ultima) and ultima == ultima[:3] * (len(ultima) // 3),
                 driver=os.environ.get("SDL_VIDEODRIVER"))
print("SONDA " + json.dumps(resultado, ensure_ascii=False), flush=True)
