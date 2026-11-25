"""Un merge no pierde lo que la otra rama no tocó.

m4185 git_conflicto: con el mandato «sin perder lo que ya hice en main», el agente resolvió el conflicto de config.json dejando
el lado de la otra rama entero y perdió `"nombre": "demo-main"`, una línea que esa rama no había tocado (git junta líneas
contiguas en un solo bloque y no enseña la base). Aquí se compara, línea a línea, la base, cada lado y el resultado.
"""
import difflib
import re
from dataclasses import dataclass

SOLO_SIGNOS = re.compile(r"^[\W_]{1,3}$")  # «}», «)», «end»... no dicen nada de qué se perdió
MARCA = re.compile(r"^<<<<<<< .*$[\s\S]*?^>>>>>>> ", re.MULTILINE)


@dataclass(frozen=True)
class Perdida:
    lado: str          # de quién era la línea (nombre de la rama)
    linea: str         # la línea que falta en el resultado
    original: str | None  # la línea de la base a la que sustituía, si se sabe
    otro_la_toco: bool    # el otro lado también cambió esa parte de la base: es un conflicto de verdad


# `git commit` o `git merge --continue` (con opciones de git delante: -c k=v, -C dir, --no-pager), también dentro de una cadena
# con && o ;. No coinciden «git log --grep commit», «git commit-tree», «echo git commit» ni «git merge x».
CIERRA_FUSION = re.compile(r"(?:^|[&|;(\s])git(?:\.exe)?(?:\s+-{1,2}[\w-]+(?:[= ][^\s&|;-]\S*)?)*\s+(?:commit|merge\s+--continue)(?![\w-])",
                           re.IGNORECASE)


def cierra_fusion(comando: str) -> bool:
    """¿El comando hace el commit con el que termina un merge? Quita primero lo que está entre comillas de un echo."""
    return bool(CIERRA_FUSION.search(re.sub(r"(?i)\becho\b[^&|;]*", " ", comando)))


def marcas_sin_quitar(texto: str) -> bool:
    return bool(MARCA.search(texto))


def perdidas(base: str, nuestro: str, suyo: str, resultado: str, nombres: tuple[str, str] = ("HEAD", "la otra rama")) -> list[Perdida]:
    """Líneas que un lado añadió o cambió respecto a la base y que el resultado no tiene."""
    return (_de_un_lado(base, nuestro, suyo, resultado, nombres[0])
            + _de_un_lado(base, suyo, nuestro, resultado, nombres[1]))


def _de_un_lado(base: str, lado: str, otro: str, resultado: str, nombre: str) -> list[Perdida]:
    b, x = base.splitlines(), lado.splitlines()
    en_resultado = {l.strip() for l in resultado.splitlines()}
    en_otro = {l.strip() for l in otro.splitlines()}
    hallado = []
    for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(None, [l.strip() for l in b], [l.strip() for l in x],
                                                       autojunk=False).get_opcodes():
        if tag not in ("replace", "insert"):
            continue
        bloque = [l.strip() for l in b[i1:i2]]
        for k, linea in enumerate(x[j1:j2]):
            s = linea.strip()
            if not s or SOLO_SIGNOS.match(s) or s in en_resultado:
                continue
            if tag == "replace" and i2 - i1 == j2 - j1:
                original = bloque[k]
                otro_toco = original not in en_otro
            elif tag == "replace":
                original, otro_toco = None, any(l not in en_otro for l in bloque)
            else:
                original, otro_toco = None, False
            hallado.append(Perdida(nombre, s, original, otro_toco))
    return hallado


def _linea(p: Perdida, archivo: str) -> str:
    if p.original is not None and not p.otro_la_toco:
        return f"- {archivo}: `{p.linea}` (de {p.lado}); en la base era `{p.original}` y la otra rama no la cambió."
    return f"- {archivo}: `{p.linea}` (de {p.lado}), que la otra rama no cambió."


def rechazo(por_archivo: dict[str, list[Perdida]]) -> str:
    """El mensaje de RECHAZADO con lo que el resultado quita y no se debía quitar."""
    lista = [_linea(p, f) for f, ps in por_archivo.items() for p in ps if not p.otro_la_toco]
    return ("RECHAZADO: este commit del merge dejaría fuera cambios que la otra rama no tocó (no hay conflicto en ellos, así "
            "que se conservan los dos lados):\n" + "\n".join(lista[:12]) +
            "\nRecupéralas en el archivo con editar y vuelve a hacer el commit. Con la marca `|||||||` ves la base: lo que solo "
            "cambió un lado se conserva; solo si los DOS cambiaron la misma línea eliges una. (Si el usuario te pidió quedarte "
            "con un lado entero, repite el commit.)")


def nota_de_conflictos(por_archivo: dict[str, list[Perdida]]) -> str:
    """Lo que los dos lados cambiaron y de lo que el resultado no tiene una de las versiones: para decirlo en el resumen."""
    lista = [f"- {f}: no está `{p.linea}` (de {p.lado}); {('en la base era `' + p.original + '`') if p.original else 'la otra rama también cambió esa parte'}."
             for f, ps in por_archivo.items() for p in ps if p.otro_la_toco]
    if not lista:
        return ""
    return ("\n[Conflictos reales (los dos lados cambiaron la misma parte) en los que el resultado no tiene una de las versiones; "
            "di en tu resumen qué elegiste y por qué:\n" + "\n".join(lista[:10]) + "]")
