"""Cobertura de idiomas (i18n) para verificar: claves que faltan entre idiomas, marcadores distintos, claves que usa el
código y no existen, y textos sin traducir. Solo actúa si el proyecto tiene archivos de idioma.

Formatos: JSON (plano, anidado o por espacio de nombres: locales/en/ventas.json), .arb, .po, YAML sencillo, Android
strings.xml, .resx, y módulos JS/TS/Python con un objeto literal (es.py, en.js o un i18n.js con {es: {...}, en: {...}}).
"""
import ast
import json
import os
import re
import xml.etree.ElementTree as ET
from collections import Counter
from pathlib import Path

from .verificacion import IGNORAR, es_codigo

CARPETAS = {"locales", "locale", "i18n", "lang", "langs", "languages", "translations", "translation", "messages",
            "l10n", "idiomas", "traducciones", "lenguajes", "textos"}
MULTI = {"i18n", "translations", "translation", "traducciones", "traduccion", "idiomas", "locales", "lang", "langs",
         "languages", "messages", "textos", "strings", "l10n", "index"}
CODIGOS = {"es", "en", "fr", "de", "it", "pt", "ca", "eu", "gl", "nl", "ru", "zh", "ja", "ko", "ar", "hi", "pl",
           "tr", "sv", "da", "no", "nb", "nn", "fi", "cs", "el", "he", "hu", "ro", "uk", "vi", "th", "id", "ms",
           "bg", "hr", "sk", "sl", "sr", "lt", "lv", "et", "fa", "ur", "bn", "ta", "te", "mr", "gu", "kn", "ml",
           "pa", "sw", "af", "sq", "hy", "az", "be", "bs", "cy", "ga", "is", "ka", "kk", "km", "lo", "mk", "mn",
           "my", "ne", "si", "tl", "uz", "zu", "fil"}
CODIGO_IDIOMA = re.compile(r"([a-z]{2,3})(?:[-_]r?([A-Za-z]{2}|[A-Za-z]{4}|\d{3}))?")
EXT_DATOS = {".json", ".arb", ".js", ".mjs", ".cjs", ".ts", ".py", ".yml", ".yaml"}
BASE = "(base)"  # Android values/ y .resx neutro: el idioma por defecto
MARCADOR = re.compile(r"\{\{\s*([\w.]+)\s*\}\}|%\{(\w+)\}|\$\{(\w+)\}|\{(\w*)(?=[,}])|%\((\w+)\)[sdifr]|%(?:\d+\$)?[sdif@]")
REFERENCIAS = [
    re.compile(r"(?<![\w.$])(?:t|i18n\.t|i18next\.t|\$t|this\.\$t|translate|_t|gettext|_)\(\s*(['\"`])([^'\"`\n]{1,100})\1"),
    re.compile(r"data-i18n(?:-[\w-]+)?\s*=\s*[\"']([^\"']+)[\"']"),
    re.compile(r"\bR\.string\.(\w+)|@string/(\w+)"),
    re.compile(r"AppLocalizations\.of\([^)]*\)!?\.(\w+)|\bS\.of\([^)]*\)\.(\w+)|\.l10n\.(\w+)"),
]
CLAVE = re.compile(r"[\w.:-]*[A-Za-z][\w.:-]*")
# Llamadas cuyo primer argumento ve el usuario (consola, tkinter, pygame, Qt) y asignaciones de texto en JS.
VISIBLES = {"print", "input", "salida", "entrada", "mostrar", "showinfo", "showerror", "showwarning", "askyesno",
            "askokcancel", "set_caption", "render", "setText", "setWindowTitle", "setToolTip", "Label", "Button",
            "QLabel", "QPushButton", "alert", "echo", "secho"}
FIJO_JS = re.compile(r"\.(?:textContent|innerText|placeholder|title)\s*=\s*(['\"`])([^'\"`$\n]{3,120})\1|"
                     r"\b(?:alert|confirm)\(\s*(['\"`])([^'\"`$\n]{3,120})\3")
UNIVERSALES = {"ok", "email", "e-mail", "total", "error", "info", "logo", "id", "url", "api", "app", "web", "online",
               "chat", "wifi", "pdf", "csv", "android", "ios", "windows", "linux", "github", "google"}


def es_idioma(texto: str) -> str | None:
    m = CODIGO_IDIOMA.fullmatch(texto)
    if not m or m.group(1) not in CODIGOS:
        return None
    return m.group(1) + (f"-{m.group(2)}" if m.group(2) else "")


# ---------------------------------------------------------------- lectura de formatos
def _aplanar(d, prefijo: str = "", out: dict | None = None) -> dict:
    out = {} if out is None else out
    if isinstance(d, dict):
        for k, v in d.items():
            _aplanar(v, f"{prefijo}.{k}" if prefijo else str(k), out)
    elif isinstance(d, list):
        for i, v in enumerate(d):
            _aplanar(v, f"{prefijo}[{i}]", out)
    elif isinstance(d, str):
        out[prefijo] = d
    elif d is not None:
        out[prefijo] = str(d)
    return out


class _Literal:
    """Objeto literal de JS/TS a dict (claves sin comillas, comillas simples, comentarios, comas finales)."""
    NUMERO = re.compile(r"-?\d+(?:\.\d+)?|true|false|null|undefined")
    NOMBRE = re.compile(r"[\w$-]+")

    def __init__(self, s: str, i: int):
        self.s, self.i = s, i

    def _ws(self) -> None:
        s = self.s
        while self.i < len(s):
            if s[self.i].isspace():
                self.i += 1
            elif s.startswith("//", self.i):
                fin = s.find("\n", self.i)
                self.i = len(s) if fin < 0 else fin
            elif s.startswith("/*", self.i):
                fin = s.find("*/", self.i + 2)
                self.i = len(s) if fin < 0 else fin + 2
            else:
                return

    def valor(self):
        self._ws()
        c = self.s[self.i]
        if c == "{":
            return self._objeto()
        if c == "[":
            return self._lista()
        if c in "'\"`":
            return self._cadena()
        m = self.NUMERO.match(self.s, self.i)
        if not m:
            raise ValueError("no es un literal")
        self.i = m.end()
        return m.group()

    def _objeto(self) -> dict:
        self.i += 1
        d = {}
        while True:
            self._ws()
            if self.s[self.i] == "}":
                self.i += 1
                return d
            if self.s[self.i] in "'\"`":
                clave = self._cadena()
            else:
                m = self.NOMBRE.match(self.s, self.i)
                if not m:
                    raise ValueError("clave calculada o propagación")
                clave, self.i = m.group(), m.end()
            self._ws()
            if self.s[self.i] != ":":
                raise ValueError("método o abreviatura")
            self.i += 1
            d[clave] = self.valor()
            self._ws()
            if self.s[self.i] == ",":
                self.i += 1
            elif self.s[self.i] != "}":
                raise ValueError("expresión")

    def _lista(self) -> list:
        self.i += 1
        out = []
        while True:
            self._ws()
            if self.s[self.i] == "]":
                self.i += 1
                return out
            out.append(self.valor())
            self._ws()
            if self.s[self.i] == ",":
                self.i += 1
            elif self.s[self.i] != "]":
                raise ValueError("expresión")

    def _cadena(self) -> str:
        comilla, s = self.s[self.i], self.s
        self.i += 1
        partes = []
        while s[self.i] != comilla:
            if s[self.i] == "\\":
                partes.append({"n": "\n", "t": "\t"}.get(s[self.i + 1], s[self.i + 1]))
                self.i += 2
            else:
                partes.append(s[self.i])
                self.i += 1
        self.i += 1
        return "".join(partes)


INICIO_JS = re.compile(r"(?:export\s+default|module\.exports\s*=|exports\.(\w+)\s*=|(?:export\s+)?(?:const|let|var)\s+"
                       r"(\w+)\s*(?::[^=\n]+)?=)\s*(?=\{)")


def _objetos_js(texto: str) -> dict[str, dict]:
    out = {}
    for m in INICIO_JS.finditer(texto):
        try:
            valor = _Literal(texto, m.end()).valor()
        except (ValueError, IndexError):
            continue
        if isinstance(valor, dict):
            out[m.group(1) or m.group(2) or "default"] = valor
    return out


def _objetos_py(texto: str) -> dict[str, dict]:
    out = {}
    try:
        arbol = ast.parse(texto)
    except (SyntaxError, ValueError):
        return out
    for n in arbol.body:
        destino = n.targets[0] if isinstance(n, ast.Assign) else n.target if isinstance(n, ast.AnnAssign) else None
        if isinstance(destino, ast.Name) and isinstance(getattr(n, "value", None), ast.Dict):
            try:
                out[destino.id] = ast.literal_eval(n.value)
            except (ValueError, SyntaxError, TypeError):
                continue
    return out


def _yaml(texto: str) -> dict:
    """YAML de traducciones (mapas anidados 'clave: valor'); listas y bloques cuentan como valor no vacío."""
    raiz: dict = {}
    pila = [(-1, raiz)]
    for linea in texto.splitlines():
        if not linea.strip() or linea.lstrip().startswith(("#", "---")):
            continue
        sangria = len(linea) - len(linea.lstrip(" "))
        if linea.lstrip().startswith("- "):
            continue
        m = re.match(r"\s*(['\"]?)([^'\":#]+?)\1\s*:(?:\s+(.*))?$", linea)
        if not m:
            continue
        while pila[-1][0] >= sangria:
            pila.pop()
        valor = (m.group(3) or "").strip()
        if valor and not valor.startswith("#"):
            valor = re.sub(r"\s+#.*$", "", valor) if valor[0] not in "'\"" else valor
            pila[-1][1][m.group(2).strip()] = valor[1:-1] if len(valor) > 1 and valor[0] == valor[-1] and valor[0] in "'\"" \
                else ("…" if valor in ("|", ">", "|-", ">-") else valor)
        else:
            nuevo: dict = {}
            pila[-1][1][m.group(2).strip()] = nuevo
            pila.append((sangria, nuevo))
    return raiz


def _po(texto: str) -> dict:
    """{msgid: msgstr} (con contexto, la clave es contexto + EOT + msgid); las entradas 'fuzzy' cuentan como sin traducir."""
    out, actual, campo, difusa = {}, {}, None, False

    def cerrar():
        if actual.get("msgid"):
            clave = (actual["msgctxt"] + chr(4) if actual.get("msgctxt") else "") + actual["msgid"]
            out[clave] = "" if difusa else actual.get("msgstr", actual.get("msgstr[0]", ""))
    for linea in map(str.strip, texto.splitlines()):
        m = re.match(r"(msgctxt|msgid_plural|msgid|msgstr(?:\[\d+\])?)\s+\"(.*)\"$", linea)
        nueva = not linea or linea.startswith("#") or (m and m.group(1) in ("msgctxt", "msgid"))
        if nueva and any(k.startswith("msgstr") for k in actual):
            cerrar()
            actual, campo, difusa = {}, None, False
        if linea.startswith("#,") and "fuzzy" in linea:
            difusa = True
        elif m:
            campo = m.group(1)
            actual[campo] = m.group(2)
        elif linea.startswith('"') and campo:
            actual[campo] += linea[1:-1]
    if any(k.startswith("msgstr") for k in actual):
        cerrar()
    return {k: v.replace('\\"', '"') for k, v in out.items()}


def _xml(p: Path) -> dict:
    try:
        raiz = ET.parse(p).getroot()
    except (ET.ParseError, OSError):
        return {}
    out = {}
    if p.suffix.lower() == ".resx":
        for d in raiz.iter("data"):
            if d.get("name") and not d.get("type") and not d.get("mimetype"):
                out[d.get("name")] = (d.findtext("value") or "").strip()
        return out
    for e in raiz:
        if e.get("name") and e.get("translatable") != "false" and e.tag in ("string", "plurals", "string-array"):
            out[e.get("name")] = "".join(e.itertext()).strip()
    return out


def _cargar(p: Path) -> dict[str, dict] | None:
    """{nombre: dict} de un archivo de datos (un JSON da {"default": ...}); None si no se puede leer."""
    suf = p.suffix.lower()
    try:
        texto = p.read_text(encoding="utf-8-sig", errors="replace")
    except OSError:
        return None
    try:
        if suf in (".json", ".arb"):
            d = json.loads(texto)
            if suf == ".arb" and isinstance(d, dict):
                d = {k: v for k, v in d.items() if not k.startswith("@")}
            return {"default": d} if isinstance(d, dict) else None
    except ValueError:
        return None
    if suf in (".yml", ".yaml"):
        return {"default": _yaml(texto)}
    if suf == ".py":
        return _objetos_py(texto) or None
    return _objetos_js(texto) or None


# ---------------------------------------------------------------- detección
def _multi(objetos: dict[str, dict]) -> dict[str, dict] | None:
    """{idioma: dict} si el archivo trae varios idiomas: {es: {...}, en: {...}} o ES = {...}; EN = {...}."""
    por_nombre = {es_idioma(n.lower()): v for n, v in objetos.items() if es_idioma(n.lower()) and isinstance(v, dict)}
    if len(por_nombre) >= 2:
        return por_nombre
    for v in objetos.values():
        dentro = {es_idioma(str(k).lower()): x for k, x in v.items() if es_idioma(str(k).lower()) and isinstance(x, dict)}
        if len(dentro) >= 2 and len(dentro) >= len(v) * 0.6:
            return dentro
    return None


def _uno(objetos: dict[str, dict]) -> dict | None:
    """El diccionario de un archivo de un solo idioma (el mayor; en YAML de Rails se quita la raíz 'en:')."""
    d = max(objetos.values(), key=lambda x: len(x) if isinstance(x, dict) else 0, default=None)
    if isinstance(d, dict) and len(d) == 1 and es_idioma(str(next(iter(d))).lower()) and isinstance(next(iter(d.values())), dict):
        d = next(iter(d.values()))
    return d if isinstance(d, dict) else None


def _clasificar(rel: str) -> tuple[str, str] | None:
    """(grupo, idioma) de un archivo de un solo idioma según su ruta; None si no lo parece."""
    partes = rel.split("/")
    nombre = partes[-1]
    stem, suf = (nombre.rsplit(".", 1) + [""])[:2]
    suf = "." + suf.lower()
    if suf == ".po":
        if len(partes) >= 3 and partes[-2] == "LC_MESSAGES" and (lang := es_idioma(partes[-3])):
            return "/".join([*partes[:-3], "{L}", *partes[-2:]]), lang
        return ("/".join([*partes[:-1], "{L}.po"]), lang) if (lang := es_idioma(stem)) else None
    if suf == ".resx":
        trozos = stem.split(".")
        lang = es_idioma(trozos[-1]) if len(trozos) > 1 else None
        base = ".".join(trozos[:-1]) if lang else stem
        return "/".join([*partes[:-1], f"{base}.{{L}}.resx"]), lang or BASE
    if nombre == "strings.xml" and len(partes) >= 2 and partes[-2].startswith("values"):
        resto = partes[-2][len("values"):].lstrip("-")
        lang = BASE if not resto else es_idioma(resto.split("-")[0] + (("-" + resto.split("-")[1]) if "-" in resto else ""))
        return ("/".join([*partes[:-2], "values-{L}", nombre]), lang) if lang else None
    if suf == ".arb":
        m = re.match(r"(?:(.*)[_-])?([a-z]{2,3}(?:[_-][A-Za-z]{2,4})?)$", stem)
        if m and (lang := es_idioma(m.group(2))):
            return "/".join([*partes[:-1], f"{m.group(1) or ''}_{{L}}.arb"]), lang
        return None
    if suf not in EXT_DATOS:
        return None
    dentro = next((i for i, x in enumerate(partes[:-1]) if x.lower() in CARPETAS), None)
    if dentro is None:
        return None
    if lang := es_idioma(stem):
        return "/".join([*partes[:-1], "{L}" + suf]), lang
    for i in range(dentro + 1, len(partes) - 1):
        if lang := es_idioma(partes[i]):
            return "/".join([*partes[:i], "{L}", *partes[i + 1:]]), lang
    return None


def _recorrer(raiz: Path):
    for carpeta, dirs, nombres in os.walk(raiz):
        dirs[:] = [d for d in dirs if d not in IGNORAR and not d.startswith(".")]
        rel = Path(carpeta).relative_to(raiz).as_posix()
        for n in nombres:
            yield (n if rel == "." else f"{rel}/{n}")


def detectar(raiz: Path) -> dict[str, dict[str, tuple[str, dict]]]:
    """{grupo: {idioma: (archivo, claves aplanadas)}} con los grupos que tienen 2 o más idiomas."""
    grupos: dict[str, dict[str, tuple[str, dict]]] = {}
    for rel in _recorrer(raiz):
        p = raiz / rel
        clase = _clasificar(rel)
        if clase:
            grupo, lang = clase
            if p.suffix.lower() == ".po":
                claves = _po(p.read_text(encoding="utf-8", errors="replace"))
            elif p.suffix.lower() in (".xml", ".resx"):
                claves = _xml(p)
            else:
                d = _uno(_cargar(p) or {})
                if d is None:
                    continue
                claves = _aplanar(d)
            grupos.setdefault(grupo, {})[lang] = (rel, claves)
        elif (p.suffix.lower() in EXT_DATOS - {".yml", ".yaml"} and p.stem.lower() in MULTI
              and (p.stem.lower() != "index" or any(x.lower() in CARPETAS for x in rel.split("/")[:-1]))):
            if p.stat().st_size > 2_000_000 or not (objetos := _cargar(p)) or not (multi := _multi(objetos)):
                continue
            grupos[rel] = {lang: (f"{rel} [{lang}]", _aplanar(d)) for lang, d in multi.items()}
    return {g: v for g, v in grupos.items() if len(v) >= 2}


def resumen_idiomas(raiz: Path) -> str:
    """Para mapa(): 'es, en (locales/{L}.json)'. Vacío si el proyecto no tiene archivos de idioma."""
    grupos = detectar(Path(raiz))
    if not grupos:
        return ""
    langs = sorted({lang for v in grupos.values() for lang in v if lang != BASE})
    return f"{', '.join(langs)} ({'; '.join(sorted(grupos)[:3])}{' …' if len(grupos) > 3 else ''})"


# ---------------------------------------------------------------- comprobaciones
def _marcadores(texto: str) -> Counter:
    return Counter(next((f"{{{g}}}" for g in m.groups() if g is not None), m.group()) for m in MARCADOR.finditer(texto))


def _traducible(texto: str) -> bool:
    letras = re.sub(r"[^A-Za-zÀ-ÿ]", "", texto)
    if len(letras) < 6 and " " not in texto.strip():
        return False
    if re.match(r"(https?://|www\.|[\w.+-]+@[\w-]+\.)", texto.strip()) or texto.strip().lower() in UNIVERSALES:
        return False
    return not (texto.isupper() and " " not in texto)


def _literal(nodo) -> str:
    """El texto fijo de un literal (en un f-string, sus partes fijas)."""
    if isinstance(nodo, ast.Constant) and isinstance(nodo.value, str):
        return nodo.value
    if isinstance(nodo, ast.JoinedStr):
        return "".join(v.value if isinstance(v, ast.Constant) and isinstance(v.value, str) else "{}" for v in nodo.values)
    return ""


def textos_fijos(rel: str, texto: str) -> list[tuple[int, str]]:
    """(línea, texto) que ve el usuario escrito a mano en vez de pasar por t(): argumentos de print/input/salida...,
    mensajes de excepciones, textContent/alert en JS. Solo se mira en archivos que ya usan traducciones."""
    out = []
    if rel.endswith(".py"):
        try:
            arbol = ast.parse(texto)
        except (SyntaxError, ValueError):
            return []
        for n in ast.walk(arbol):
            llamada = n.exc if isinstance(n, ast.Raise) and isinstance(n.exc, ast.Call) else n if isinstance(n, ast.Call) \
                else None
            if llamada is None or not llamada.args:
                continue
            f = llamada.func
            nombre = f.id if isinstance(f, ast.Name) else f.attr if isinstance(f, ast.Attribute) else ""
            if (nombre in VISIBLES or isinstance(n, ast.Raise)) and _humano(s := _literal(llamada.args[0])):
                out.append((llamada.lineno, s.strip()))
    elif rel.endswith((".js", ".mjs", ".jsx", ".ts", ".tsx")):
        for m in FIJO_JS.finditer(texto):
            if _humano(s := m.group(2) or m.group(4)):
                out.append((texto.count("\n", 0, m.start()) + 1, s.strip()))
    return list(dict.fromkeys(out))


def _humano(texto: str) -> bool:
    return len(re.findall(r"[A-Za-zÀ-ÿ]{2,}", texto)) >= 2 or len(re.sub(r"[^A-Za-zÀ-ÿ]", "", texto)) >= 6


def _referencias(raiz: Path, excluir: set[str], fijos: dict | None = None) -> dict[str, list[tuple[str, int]]]:
    """{clave: [(archivo, línea)]} de las claves de traducción que usa el código (no los tests ni los idiomas).
    fijos (si se pasa) recibe {archivo: textos_fijos} de los archivos que ya usan traducciones."""
    out: dict[str, list[tuple[str, int]]] = {}
    for rel in _recorrer(raiz):
        p = raiz / rel
        if rel in excluir or not es_codigo(p) or p.suffix.lower() in (".json", ".yml", ".yaml", ".xml", ".toml") \
                or re.search(r"(^|/)(tests?|__tests__|spec)/|(^|/)test_|[._-](test|spec)\.", rel):
            continue
        try:
            if p.stat().st_size > 1_000_000:
                continue
            texto = p.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        usa = False
        for rx in REFERENCIAS:
            for m in rx.finditer(texto):
                usa = True
                clave = next((g for g in reversed(m.groups()) if g), "")
                if clave and "${" not in clave and not clave.endswith((".", ":")) and CLAVE.fullmatch(clave):
                    out.setdefault(clave, []).append((rel, texto.count("\n", 0, m.start()) + 1))
        if usa and fijos is not None and (encontrados := textos_fijos(rel, texto)):
            fijos[rel] = encontrados
    return out


def _lista(claves, n: int = 8) -> str:
    claves = sorted(claves)
    return ", ".join(claves[:n]) + (f" (+{len(claves) - n})" if len(claves) > n else "")


def verificar_idiomas(raiz: Path, inf, parcial: list[str] | None = None) -> bool:
    """Añade al informe los problemas de idiomas. parcial = archivos de la parte en curso del modo grande: lo que falta
    y no usan esos archivos es trabajo de otras partes (aviso, no error). Devuelve si había archivos de idioma."""
    grupos = detectar(raiz)
    if not grupos:
        return False
    inf.tipos.append("idiomas")
    errores_antes = len(inf.errores)
    archivos_idioma = {archivo.split(" [")[0] for v in grupos.values() for archivo, _ in v.values()}
    fijos: dict[str, list[tuple[int, str]]] = {}
    refs = _referencias(raiz, archivos_idioma, fijos)
    parte = {p.replace("\\", "/").removeprefix("./") for p in parcial or []}
    de_la_parte = {c for c, usos in refs.items() if any(r in parte for r, _ in usos)}
    definidas: set[str] = set()
    for grupo, por_idioma in sorted(grupos.items()):
        espacio = Path(grupo).stem if "/{L}/" in f"/{grupo}" else ""
        mtimes = {lang: _mtime(raiz / archivo.split(" [")[0]) for lang, (archivo, _) in por_idioma.items()}
        orden = list(por_idioma)
        base = BASE if BASE in por_idioma else max(orden, key=lambda lang: (len(por_idioma[lang][1]), -mtimes[lang],
                                                                         -orden.index(lang)))
        base_archivo, base_claves = por_idioma[base]
        todas = set().union(*(set(c) for _, c in por_idioma.values()))
        definidas |= todas | ({f"{espacio}.{k}" for k in todas} | {f"{espacio}:{k}" for k in todas} if espacio else set())
        for lang, (archivo, claves) in sorted(por_idioma.items()):
            faltan = todas - set(claves)
            if faltan:
                propias = {k for k in faltan if k in de_la_parte or f"{espacio}.{k}" in de_la_parte}
                pendientes = faltan - propias if parcial is not None else set()
                if faltan - pendientes:
                    inf.errores.append(f"{archivo}: faltan {len(faltan - pendientes)} claves de idioma que sí están en "
                                       f"otros idiomas del grupo: {_lista(faltan - pendientes)}")
                if pendientes:
                    inf.avisos.append(f"{archivo}: faltan {len(pendientes)} claves que harán otras partes del modo "
                                      f"grande: {_lista(pendientes, 5)}")
            vacias = [k for k, v in claves.items() if not v.strip()
                      and any(c.get(k, "").strip() for otro, (_, c) in por_idioma.items() if otro != lang)]
            if vacias:
                inf.errores.append(f"{archivo}: {len(vacias)} textos vacíos (sin traducir): {_lista(vacias)}")
            if lang == base:
                continue
            distintos = [k for k, v in claves.items() if k in base_claves and v.strip()
                         and _marcadores(v) != _marcadores(base_claves[k])]
            if distintos:
                inf.errores.append(f"{archivo}: marcadores distintos a {base_archivo} ({{nombre}}, %s...) en: "
                                   f"{_lista(distintos)}")
            traducibles = [k for k, v in base_claves.items() if k in claves and _traducible(v)]
            iguales = [k for k in traducibles if claves[k].strip() == base_claves[k].strip()]
            if iguales and len(iguales) >= 5 and len(iguales) >= 0.3 * len(traducibles):
                inf.errores.append(f"{archivo}: {len(iguales)} de {len(traducibles)} textos son idénticos a {base_archivo}: "
                                   f"el idioma está SIN TRADUCIR ({_lista(iguales, 5)})")
            elif iguales:
                inf.avisos.append(f"{archivo}: {len(iguales)} textos iguales que en {base_archivo} (¿sin traducir?): "
                                  f"{_lista(iguales, 5)}")
    sin_definir = {c: usos for c, usos in refs.items() if c not in definidas}
    for lista, claves in ((inf.errores, {c for c in sin_definir if parcial is None or c in de_la_parte}),
                          (inf.avisos, {c for c in sin_definir if parcial is not None and c not in de_la_parte})):
        if claves:
            ejemplos = [f"{c} ({sin_definir[c][0][0]}:{sin_definir[c][0][1]})" for c in sorted(claves)[:8]]
            lista.append(f"el código usa {len(claves)} claves de idioma que no existen en ningún archivo de idioma"
                         f"{'' if lista is inf.errores else ' (otras partes del modo grande)'}: {', '.join(ejemplos)}"
                         f"{' …' if len(claves) > 8 else ''}")
    for rel, lista in sorted(fijos.items()):
        if parcial is None or rel in parte:
            ejemplos = ", ".join(f"'{s[:40]}' (l. {n})" for n, s in lista[:4])
            inf.avisos.append(f"{rel}: {len(lista)} textos fijos que ve el usuario sin pasar por las traducciones: "
                              f"{ejemplos}{' …' if len(lista) > 4 else ''}")
    if len(inf.errores) == errores_antes:
        langs = sorted({lang for v in grupos.values() for lang in v})
        inf.ok.append(f"idiomas completos ({', '.join(langs)}) en {len(grupos)} grupo(s)")
    return True


def _mtime(p: Path) -> float:
    try:
        return p.stat().st_mtime
    except OSError:
        return 0.0
