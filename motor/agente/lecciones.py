"""Lecciones del agente: sobreviven las que se usan y cada tarea recibe solo las que tienen que ver con ella.

lecciones.md sigue siendo la lista que se lee y se edita a mano (Aura la muestra tal cual). El uso va aparte, en
lecciones_uso.json: cuándo se usó cada una por última vez y cuántas veces. Una lección se usa cuando entra en una
tarea porque tiene que ver con ella, o cuando el agente la intenta guardar otra vez (el problema se repitió). Con la
lista llena (MAX), se olvida la que lleva más tiempo sin usarse, no la más antigua.
"""
import json
import re
import time
from pathlib import Path

from .config import RAIZ_INSTALACION
from .conocimiento import ALIAS, DIR_GUIAS
from .pasajes import raices
from .revision import TEMAS
from .verificacion import IGNORAR

ARCHIVO = RAIZ_INSTALACION / "lecciones.md"
USO = RAIZ_INSTALACION / "lecciones_uso.json"
MAX = 40
MAX_TAREA = 8  # lecciones en el mensaje de una tarea...
MAX_CHARS_TAREA = 1500  # ...y caracteres, como el bloque que antes iba siempre en el prompt de sistema
CABECERA = "# Lecciones aprendidas"
# Palabras de cualquier tarea o lección que no dicen de qué trata (sus raíces de 5 letras).
GENERICAS = raices("crea crear haz hacer archivo archivos proyecto codigo usa usar siempre antes despues nunca tarea "
                   "agente app aplicacion programa funciona funcionar quiero necesito falla fallo comando")


def _clave(leccion: str) -> str:
    return " ".join(leccion.lower().split())


def _temas(texto: str) -> set[str]:
    """Temas de las guías que nombra el texto, también por sus alias (juego → pygame, npm → node, .bat → bat)."""
    guias = {p.stem for p in DIR_GUIAS.glob("*.md")}
    palabras = (w.strip(".-") for w in re.findall(r"[\w.#+-]+", texto.lower()))
    return {t for w in palabras if (t := ALIAS.get(w, w)) in guias}


def conceptos(texto: str) -> set[str]:
    """De qué trata un texto: sus palabras con significado y, marcados con '#', los temas que nombra."""
    return (raices(texto) - GENERICAS) | {f"#{t}" for t in _temas(texto)}


def temas_proyecto(raiz: Path) -> set[str]:
    """Temas por los archivos del proyecto hasta un nivel de carpetas: «arregla el botón» en un proyecto Tauri no
    nombra Tauri, pero src-tauri/tauri.conf.json sí."""
    archivos = []
    try:
        for p in raiz.iterdir():
            if p.is_file():
                archivos.append(p)
            elif p.name not in IGNORAR:
                try:
                    archivos += [h for h in p.iterdir() if h.is_file()]
                except OSError:
                    pass  # carpeta sin permiso
    except OSError:
        pass
    return {t for p in archivos if (t := TEMAS.get(p.name.lower()) or TEMAS.get(p.suffix.lower()))}


def leer() -> list[str]:
    """Las lecciones de lecciones.md, sin viñetas; las líneas con '#' son títulos."""
    try:
        lineas = ARCHIVO.read_text(encoding="utf-8").splitlines()
    except OSError:
        return []
    return [re.sub(r"^[-*]\s*", "", l.strip()) for l in lineas if l.strip() and not l.lstrip().startswith("#")]


def _uso() -> dict:
    try:
        return json.loads(USO.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}


def _ultima(uso: dict, leccion: str, posicion: int) -> float:
    """Último uso. Las que no tienen registro (de antes, o escritas a mano) cuentan por su posición en el archivo:
    más viejas que cualquier uso real y, entre ellas, la primera es la más vieja (como la lista de antes)."""
    return uso.get(_clave(leccion), {}).get("usada", posicion)


def _marcar(uso: dict, lista: list[str], ahora: float | None, usos: int = 1) -> None:
    for leccion in lista:
        previo = uso.get(_clave(leccion), {})
        uso[_clave(leccion)] = {"usada": int(time.time() if ahora is None else ahora),
                                "usos": previo.get("usos", 0) + usos}


def _escribir_uso(lista: list[str], uso: dict) -> None:
    vigentes = {_clave(l) for l in lista}
    temporal = USO.with_suffix(".tmp")  # escribir y renombrar: un corte a medias no deja un JSON roto
    temporal.write_text(json.dumps({k: v for k, v in uso.items() if k in vigentes}, ensure_ascii=False, indent=1),
                        encoding="utf-8")
    temporal.replace(USO)


def guardar(leccion: str, ahora: float | None = None) -> str:
    leccion = " ".join(leccion.split())[:220]
    if len(leccion) < 15:
        return "La lección debe ser una frase concreta (qué pasó y cómo evitarlo)."
    lista, uso = leer(), _uso()
    if repetida := next((l for l in lista if leccion.lower() in l.lower()), None):
        _marcar(uso, [repetida], ahora)  # el problema se repitió: es útil
        _escribir_uso(lista, uso)
        return "Esa lección ya estaba guardada."
    ultima = {l: _ultima(uso, l, i) for i, l in enumerate(lista)}
    while len(lista) >= MAX:
        lista.remove(min(lista, key=ultima.__getitem__))
    lista.append(leccion)
    _marcar(uso, [leccion], ahora, usos=0)
    ARCHIVO.write_text(CABECERA + "\n" + "".join(f"- {l}\n" for l in lista), encoding="utf-8")
    _escribir_uso(lista, uso)
    return "Lección guardada: se tendrá en cuenta en las próximas tareas que tengan que ver con ella."


def para_tarea(texto: str, raiz: Path | None = None, ahora: float | None = None) -> str:
    """Las lecciones que tienen que ver con la tarea, para añadir a su mensaje (vacío si ninguna): las que comparten
    un tema con la tarea o el proyecto, o dos palabras con significado. Primero las que más comparten y, a igualdad,
    las usadas más recientemente. Cuentan como usadas."""
    lista = leer()
    if not lista:
        return ""
    pedidos = conceptos(texto) | {f"#{t}" for t in (temas_proyecto(raiz) if raiz else ())}
    uso = _uso()
    puntuadas = []
    for i, leccion in enumerate(lista):
        comunes = pedidos & conceptos(leccion)
        if len(comunes) >= 2 or any(c.startswith("#") for c in comunes):
            puntuadas.append((len(comunes), _ultima(uso, leccion, i), leccion))
    elegidas, total = [], 0
    for *_, leccion in sorted(puntuadas, reverse=True)[:MAX_TAREA]:
        if total + len(leccion) > MAX_CHARS_TAREA:
            break
        elegidas.append(leccion)
        total += len(leccion) + 3
    if not elegidas:
        return ""
    _marcar(uso, elegidas, ahora)
    try:
        _escribir_uso(lista, uso)
    except OSError:
        pass  # sin poder anotar el uso, la tarea sigue con sus lecciones
    return ("\n\nLECCIONES DE TAREAS ANTERIORES (tienen que ver con esta; evita repetir estos errores):\n"
            + "\n".join(f"- {l}" for l in elegidas))
