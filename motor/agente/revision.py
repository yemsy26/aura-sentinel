"""revisar(): segunda opinión del modelo en un contexto LIMPIO (sin el historial que lo sesga).

Compara cada requisito con el código, busca lógica sin conectar y tests falsos. Devuelve el veredicto.
"""
import re
from pathlib import Path

from .verificacion import IGNORAR

EXT = {".py", ".js", ".mjs", ".ts", ".html", ".htm", ".css", ".java", ".bat", ".cmd", ".json", ".toml", ".txt", ".md"}

SISTEMA = """Eres un revisor de código senior, exigente y concreto. No escribes código nuevo: encuentras lo que hará que el programa NO funcione o NO cumpla lo pedido.
Comprueba, con el número de línea como prueba:
1. Cada requisito del usuario está implementado Y conectado al flujo principal (se llama de verdad, no solo se define).
2. Valores que se calculan y no se usan (colisiones sin efecto, puntos que nunca suben, flags que nunca cambian).
3. Estados inalcanzables: menús que no se muestran, fin de partida que nunca ocurre, reinicio que no reinicia, pausa que no pausa.
4. Errores en ejecución: nombres inexistentes, índices fuera de rango, None, rutas de archivos/recursos incorrectas, .bat que llama a archivos que no existen.
5. Tests que no prueban el código del proyecto o que se debilitaron para pasar.
Ignora estilo y gustos. No inventes problemas: si no ves la línea que lo demuestra, no lo pongas.
Responde EXACTAMENTE con este formato, en español:
REQUISITOS:
- <requisito> -> CUMPLE (archivo:línea) | NO CUMPLE: <motivo>
PROBLEMAS:
- [CRITICO|MEDIO] archivo:línea - <problema> - <corrección concreta>
(o "- ninguno")
VEREDICTO: APROBADO | CAMBIOS NECESARIOS"""


def _fuentes(raiz: Path) -> list[Path]:
    out = [p for p in raiz.rglob("*") if p.is_file() and p.suffix.lower() in EXT
           and not (IGNORAR & set(p.relative_to(raiz).parts[:-1])) and p.stat().st_size < 200_000
           and p.name not in ("package-lock.json",)]
    # Primero el código (lo más reciente), al final la documentación.
    return sorted(out, key=lambda p: (p.suffix.lower() in (".md", ".txt", ".json"), -p.stat().st_mtime))


def revisar(llm, raiz: Path, tarea: str, informe: str, enfoque: str = "") -> tuple[str, bool]:
    """Devuelve (texto de la revisión, aprobado)."""
    cupo = int(llm.contexto * 3.0 * 0.5)  # ~50 % del contexto para el código
    partes, usado = [], 0
    for f in _fuentes(raiz):
        lineas = f.read_text(encoding="utf-8", errors="replace").splitlines()
        bloque = f"=== {f.relative_to(raiz)} ===\n" + "\n".join(f"{i}| {l}" for i, l in enumerate(lineas, 1))
        if usado + len(bloque) > cupo:
            bloque = bloque[:max(0, cupo - usado)] + "\n[... recortado]"
        partes.append(bloque)
        usado += len(bloque)
        if usado >= cupo:
            break
    if not partes:
        return "REVISIÓN: no hay archivos de código que revisar.", True
    usuario = (f"PETICIÓN DEL USUARIO:\n{tarea[:3000]}\n\n"
               f"RESULTADO DE LA VERIFICACIÓN AUTOMÁTICA:\n{informe[:2500] or '(no ejecutada)'}\n\n"
               + (f"FÍJATE ESPECIALMENTE EN: {enfoque}\n\n" if enfoque else "")
               + "CÓDIGO:\n" + "\n\n".join(partes))
    r = llm.chat([{"role": "system", "content": SISTEMA}, {"role": "user", "content": usuario}], None, 1800, False)
    texto = r["content"].strip()
    if not texto:
        return "REVISIÓN: el revisor no respondió.", True
    aprobado = bool(re.search(r"VEREDICTO:\s*APROBADO", texto, re.IGNORECASE)) and not re.search(r"\[CRITICO\]", texto, re.IGNORECASE)
    return "REVISIÓN:\n" + texto, aprobado
