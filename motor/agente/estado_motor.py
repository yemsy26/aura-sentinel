"""El estado del propio motor en .agente/ (revisión, registro, plan, puntos de restauración, métricas) lo escribe solo el motor.

m4185 tienda_roles: tras una revisión que pedía cambios, el modelo editó .agente/revision.json, lo borró y vació registro.log
con `echo [] > .agente\\registro.log` hasta dejar {"aprobado": true}; terminar pasó y el resumen decía «Revisión: APROBADO».
Aquí está lo que detecta qué archivos y qué comandos tocan ese estado. Es un filtro por texto: quita los caminos que un modelo
toma por inercia, no es un sandbox (el de revision.json además se restaura desde la memoria del bucle).
"""
import fnmatch
import re

# Archivos que están directamente en .agente/ y son del motor. Las subcarpetas (experimento, capturas, tmp...) y otros
# archivos (la cartera localnet.json de las guías de Solana) siguen siendo del modelo.
ESTADO = ("revision.json", "registro.log", "checkpoints.json", "plan.md", "plan_anterior.md", "metricas.jsonl", "resumen.md",
          "sin_usar.json", "grande.json", "grande_anterior.json", "mapa.json", "sesion_navegador.json")
_NOMBRES = "|".join(re.escape(n) for n in ESTADO)
# Una ruta que cae en ese estado: un archivo de la lista, un comodín bajo .agente\ o la propia carpeta.
_RUTA = (rf"""(?<![\w.-])\.agente(?:[\\/]+(?:{_NOMBRES})(?![\w.-])|[\\/]+(?P<glob>[^\s"'&|;<>\\/]*[*?])|[\\/]*(?=$|[\s"'&|;)<>*]))""")
_VERBOS = (r"del|erase|rm|rd|rmdir|remove-item|ri|move|mv|move-item|ren|rename|rename-item|copy|cp|copy-item|xcopy|robocopy|"
           r"set-content|add-content|out-file|clear-content|clc|tee|tee-object|truncate|attrib|icacls|takeown")
# Redirección o verbo que cambia lo que sigue, y a continuación esa ruta.
_ESCRIBE = re.compile(rf"""(?:>>?\s*|(?<![\w.-])(?:{_VERBOS})(?![\w-])[^&|;>\n]*?)["']?{_RUTA}""", re.IGNORECASE)
_RUTA_RE = re.compile(_RUTA, re.IGNORECASE)
# Código (python -c, node -e, un experimento) que abre en escritura, borra o escribe sobre esa ruta en la misma línea.
_CODIGO = re.compile(r"""\bopen\([^)\n]*,\s*["'][^"'\n]*[wa+]|\bwrite_text\b|\bwrite_bytes\b|\bunlink\b|\brmtree\b|"""
                     r"""\bos\.(?:remove|replace|rename)\b|\.truncate\b|\.write\(|\bwriteFileSync\b|\bappendFileSync\b|"""
                     r"""\bunlinkSync\b|\brmSync\b""", re.IGNORECASE)

MENSAJE = ("BLOQUEADO: {que!r} toca el estado interno del motor (.agente: revisión, registro, plan, puntos de restauración, "
           "métricas). Ese estado lo escribe solo el motor; cambiarlo no arregla nada y no cuenta como revisión. Si una "
           "revisión pide cambios, corrige el código con editar y vuelve a verificar y a terminar; si crees que la revisión "
           "se equivoca, explícalo con claridad en el resumen de terminar.")


def es_estado(p, dir_agente) -> bool:
    """¿Es p un archivo de estado del motor (directamente en .agente/)? Windows no distingue mayúsculas."""
    return p.parent == dir_agente and p.name.lower() in ESTADO


def toca_estado(texto: str) -> str | None:
    """El fragmento del comando o código que escribe, borra, mueve o renombra el estado del motor; None si no lo toca.
    Leer (type, findstr, cat, python que solo lee) no cuenta."""
    for linea in texto.splitlines() or [texto]:
        for m in _ESCRIBE.finditer(linea):
            if _alcanza(m):
                return m.group(0).strip()[:100]
        if (m := next((m for m in _RUTA_RE.finditer(linea) if _alcanza(m)), None)) and _CODIGO.search(linea):
            return linea.strip()[:100]
    return None


def _alcanza(m: re.Match) -> bool:
    """Un comodín bajo .agente/ solo cuenta si puede casar con un archivo de estado (`test_init*.py` no)."""
    glob = m.groupdict().get("glob")
    return glob is None or any(fnmatch.fnmatch(n, glob.lower()) for n in ESTADO)


def bloqueo(texto: str) -> str | None:
    """El mensaje de BLOQUEADO si el comando o código toca el estado del motor, o None."""
    return MENSAJE.format(que=que) if (que := toca_estado(texto)) else None
