"""revisar(): segunda opinión del modelo en un contexto LIMPIO (sin el historial que lo sesga).

Compara cada requisito con el código, busca lógica sin conectar y tests falsos. Devuelve el veredicto. Para no juzgar con
APIs de versiones viejas recibe, de las guías de los temas del proyecto, solo las frases que corrigen una versión y nombran
algo que está en el código o en la petición.
"""
import re
from pathlib import Path

from .conocimiento import DIR_GUIAS
from .verificacion import es_codigo, ignorado

BLOQUEOS = {"package-lock.json", "yarn.lock", "pnpm-lock.yaml", "cargo.lock", "go.sum", "packages.lock.json"}
# Archivo o extensión -> guía del tema (los mismos tipos que comprueba verificar). El orden decide quién entra antes.
TEMAS = {"tauri.conf.json": "tauri", "cargo.toml": "rust", ".rs": "rust", "package.json": "node", ".html": "web",
         ".py": "python", "go.mod": "go", ".csproj": "dotnet", "androidmanifest.xml": "android", ".java": "java",
         "pubspec.yaml": "flutter", ".sol": "evm", "anchor.toml": "solana", "dockerfile": "docker", ".bat": "bat"}
PYGAME = re.compile(r"^\s*(?:import|from)\s+pygame\b", re.MULTILINE)
# Frases que corrigen una versión o una costumbre vieja (lo que el revisor no puede saber por su cuenta).
CORRIGE = re.compile(r"no existen?\b|no valen?\b|ya no\b|\bera\b|obsolet|retirad|cambió|en vez de|no devuelvas|nada de|"
                     r"solo existe|\bnunca\b|se pueden invocar|desde la \d|\bsiempre\b|desaconsejad|sustitut", re.IGNORECASE)
MAX_PRACTICAS = 1500  # caracteres (~450 tokens); salen del cupo del código
# Un [CRITICO] que el propio revisor retira en su última frase (el modelo local lista así a veces lo que está bien):
# "Sin embargo, no rompe la funcionalidad.", "No es un error crítico, pero es una mejora.", "Esto es correcto."
NO_GRAVE = re.compile(r"\bno es (?:una? )?(?:\w+ ){0,2}?(?:cr[ií]tic[oa]|grave)\b|\bno rompe (?:la funcionalidad|el "
                      r"funcionamiento|la ejecución|nada)\b|^no es un (?:error|fallo|problema)\b(?! de (?:compilaci|sintaxis))",
                      re.IGNORECASE)
# Lo mismo en positivo, solo si no sigue un "pero" ("Es correcto llamarla al inicio, pero falta setInterval" cuenta).
BIEN = re.compile(r"^(?:esto |eso |lo cual |todo esto )?(?:es|está|son) (?:(?:la|el|una?|muy) )?(?:\w+ )?(?:correct|"
                  r"aceptable|válid|adecuad|buen)|^(?:sin embargo,? |aun así,? )?funciona\W*$|^no hay (?:ningún )?problemas?\b"
                  r"|\bpor lo que (?:es|está) (?:correct|aceptable)", re.IGNORECASE)
PERO = re.compile(r"\b(?:pero|sin embargo|aunque|salvo)\b", re.IGNORECASE)

SISTEMA = """Eres un revisor de código senior, exigente y concreto. No escribes código nuevo: encuentras lo que hará que el programa NO funcione o NO cumpla lo pedido.
Comprueba, con el número de línea como prueba:
1. Cada requisito del usuario está implementado Y conectado al flujo principal (se llama de verdad, no solo se define).
2. Valores que se calculan y no se usan (colisiones sin efecto, puntos que nunca suben, flags que nunca cambian).
3. Estados inalcanzables: menús que no se muestran, fin de partida que nunca ocurre, reinicio que no reinicia, pausa que no pausa.
4. Errores en ejecución: nombres inexistentes, índices fuera de rango, None, rutas de archivos/recursos incorrectas, .bat que llama a archivos que no existen.
5. Tests que no prueban el código del proyecto o que se debilitaron para pasar.
6. Los requisitos que son ACCIONES (desplegar, copiar a un servidor, arrancar, publicar, comprobar que responde) se
   juzgan con los COMANDOS EJECUTADOS y su resultado: si un comando lo hizo con exit=0 y la salida lo confirma, CUMPLE
   (cita el comando). No exijas un script para eso salvo que el usuario lo pida.
7. Si un requisito nombra una API, import, opción o comando que NO existe en la versión que usa el proyecto (lo dicen las
   PRÁCTICAS ACTUALES o el compilador), usar la forma actual es una desviación justificada: ponlo como
   CUMPLE (desviación justificada: <motivo>), nunca CRITICO, y no pidas volver a la forma del usuario.
Ignora estilo y gustos. No inventes problemas: si no ves la línea que lo demuestra, no lo pongas. Las PRÁCTICAS ACTUALES mandan sobre lo que recuerdes de versiones antiguas.
Gravedad por la consecuencia: CRITICO si, tal como está el código, al ejecutarlo algo falla o un requisito no se cumple;
MEDIO si solo fallaría en otras condiciones o es mejorable. Lo que compruebas y está bien no es un problema: nunca lo
marques CRITICO.
Responde EXACTAMENTE con este formato, en español:
REQUISITOS:
- <requisito> -> CUMPLE (archivo:línea) | NO CUMPLE: <motivo>
PROBLEMAS:
- [CRITICO|MEDIO] archivo:línea - <problema> - <corrección concreta>
(o "- ninguno")
VEREDICTO: APROBADO | CAMBIOS NECESARIOS"""


def _fuentes(raiz: Path) -> list[Path]:
    out = [p for p in raiz.rglob("*") if p.is_file() and (es_codigo(p) or p.suffix.lower() in (".txt", ".md"))
           and not ignorado(p, raiz) and p.stat().st_size < 200_000 and p.name.lower() not in BLOQUEOS]
    # Primero el código (lo más reciente), al final la documentación.
    return sorted(out, key=lambda p: (p.suffix.lower() in (".md", ".txt", ".json"), -p.stat().st_mtime))


def temas(fuentes: list[Path], textos: list[str]) -> list[str]:
    """Guías que tocan al proyecto, por sus archivos (pygame, por los import de los .py que se leyeron)."""
    vistos = {TEMAS.get(p.name.lower()) or TEMAS.get(p.suffix.lower()) for p in fuentes}
    if any(PYGAME.search(t) for t in textos):
        vistos.add("pygame")
    orden = dict.fromkeys(("tauri", "pygame", *TEMAS.values()))
    return [t for t in orden if t in vistos and (DIR_GUIAS / f"{t}.md").is_file()]


def _frases(linea: str) -> list[str]:
    """Parte una viñeta en frases sin cortar el código entre comillas invertidas ni 'p. ej.'."""
    protegida = re.sub(r"`[^`]*`", lambda m: m.group(0).replace(" ", "\0"), linea)
    return [f.replace("\0", " ") for f in re.split(r"(?<!\bej\.)(?<!\bp\.)(?<=\.)\s+(?=[A-ZÁÉÍÓÚ¿(`])", protegida)]


def _nombra(frase: str, texto: str) -> bool:
    """La frase nombra algo concreto (ruta, módulo, macro, camelCase: @tauri-apps/api/invoke, generate_handler!,
    pygame-ce...) que aparece tal cual en el texto."""
    for c in {t.strip(".:;,-") for t in re.findall(r"[\w@$.:/<>!*-]+", frase)}:
        if (len(c) >= 4 and len(re.findall(r"[A-Za-z]", c)) >= 3 and re.search(r"[/:._@!<-]|[a-z][A-Z]", c)
                and re.search(r"(?<![\w@/.-])" + re.escape(c) + r"(?![\w/-])", texto)):
            return True
    return False


def practicas(lista: list[str], tarea: str, codigo: str) -> str:
    """De las guías de esos temas, las frases que corrigen una versión y nombran algo del código o de la petición (con
    la frase inicial de su viñeta, que suele dar la forma correcta). Primero lo que nombra la petición; hasta MAX_PRACTICAS."""
    elegidas = []
    for tema in lista:
        for linea in (DIR_GUIAS / f"{tema}.md").read_text(encoding="utf-8").splitlines():
            if not linea.lstrip().startswith("- "):
                continue
            frases = _frases(linea.lstrip()[2:])
            pide = [i for i, f in enumerate(frases) if CORRIGE.search(f) and _nombra(f, tarea)]
            idx = [i for i, f in enumerate(frases) if i in pide or (CORRIGE.search(f) and _nombra(f, codigo))]
            if not idx:
                continue
            if idx[0] != 0 and len(frases[0]) <= 220 and (_nombra(frases[0], codigo) or _nombra(frases[0], tarea)):
                idx.insert(0, 0)
            elegidas.append((not pide, f"- [{tema}] " + " ".join(frases[i] for i in idx)))
    lineas, total = [], 0
    for _, linea in sorted(elegidas, key=lambda e: e[0]):
        if total + len(linea) < MAX_PRACTICAS:
            lineas.append(linea)
            total += len(linea) + 1
    return "\n".join(lineas)


def revisar(llm, raiz: Path, tarea: str, informe: str, enfoque: str = "", evidencia: str = "") -> tuple[str, bool]:
    """Devuelve (texto de la revisión, aprobado)."""
    cupo = int(llm.contexto * 3.0 * 0.5)  # ~50 % del contexto para el código y las prácticas
    fuentes, leidos, leido = _fuentes(raiz), [], 0
    for f in fuentes:  # solo lo que puede caber
        if leido >= cupo:
            break
        leidos.append((f, f.read_text(encoding="utf-8", errors="replace")))
        leido += len(leidos[-1][1])
    if not leidos:
        return "REVISIÓN: no hay archivos de código que revisar.", True
    guia = practicas(temas(fuentes, [t for _, t in leidos]), tarea, "\n".join(t for _, t in leidos))
    cupo -= len(guia)
    partes, usado = [], 0
    for f, texto in leidos:
        bloque = f"=== {f.relative_to(raiz)} ===\n" + "\n".join(f"{i}| {l}" for i, l in enumerate(texto.splitlines(), 1))
        if usado + len(bloque) > cupo:
            bloque = bloque[:max(0, cupo - usado)] + "\n[... recortado]"
        partes.append(bloque)
        usado += len(bloque)
        if usado >= cupo:
            break
    usuario = (f"PETICIÓN DEL USUARIO:\n{tarea[:3000]}\n\n"
               + (f"PRÁCTICAS ACTUALES (de las guías, para la versión que usa el proyecto):\n{guia}\n\n" if guia else "")
               + f"RESULTADO DE LA VERIFICACIÓN AUTOMÁTICA:\n{informe[:2500] or '(no ejecutada)'}\n\n"
               + (f"COMANDOS EJECUTADOS EN ESTA TAREA (con su resultado):\n{evidencia}\n\n" if evidencia else "")
               + (f"FÍJATE ESPECIALMENTE EN: {enfoque}\n\n" if enfoque else "")
               + "CÓDIGO:\n" + "\n\n".join(partes))
    r = llm.chat([{"role": "system", "content": SISTEMA}, {"role": "user", "content": usuario}], None, 1800, False)
    texto = r["content"].strip()
    if not texto:
        return "REVISIÓN: el revisor no respondió.", True
    aprobado = bool(re.search(r"VEREDICTO:\s*APROBADO", texto, re.IGNORECASE)) and not criticos(texto)
    return "REVISIÓN:\n" + texto, aprobado


def _retirado(linea: str) -> bool:
    final = _frases(linea.strip())[-1].strip(" *_")
    bien = BIEN.search(final)
    return bool(NO_GRAVE.search(final) or (bien and not PERO.search(final, bien.end())))


def criticos(texto: str) -> list[str]:
    """Las líneas [CRITICO] que piden un cambio de verdad (sin las que el revisor retira en su última frase)."""
    return [l for l in texto.splitlines() if re.search(r"\[CR[IÍ]TICO\]", l, re.IGNORECASE) and not _retirado(l)]


def mtime_fuentes(raiz: Path) -> float:
    """Última modificación de lo que lee el revisor (también los .json de configuración, como capabilities/)."""
    return max((p.stat().st_mtime for p in _fuentes(raiz)), default=0)
