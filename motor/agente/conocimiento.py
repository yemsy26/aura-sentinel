"""Orientación actualizada: guías de buenas prácticas, versiones reales de paquetes e investigación web en un paso."""
import json
import re
import subprocess
import urllib.parse
from pathlib import Path

DIR_GUIAS = Path(__file__).resolve().parent / "guias"
ALIAS = {"juego": "pygame", "pygame-ce": "pygame", "canvas": "html-juego", "javascript": "html-juego", "js": "web",
         "html": "web", "css": "web", "flask": "api", "fastapi": "api", "rest": "api", "test": "tests",
         "pytest": "tests", "junit": "tests", "py": "python", "bats": "bat", "cmd": "bat",
         "rs": "rust", "cargo": "rust", "golang": "go", "typescript": "node", "ts": "node", "nodejs": "node",
         "node.js": "node", "npm": "node", "express": "node", "csharp": "dotnet", "c#": "dotnet", ".net": "dotnet",
         "net": "dotnet", "aspnet": "dotnet", "contenedor": "docker", "contenedores": "docker", "compose": "docker",
         "docker-compose": "docker", "podman": "docker", "servidor": "despliegue", "servidores": "despliegue",
         "deploy": "despliegue", "desplegar": "despliegue", "ssh": "despliegue", "kubernetes": "despliegue",
         "k8s": "despliegue"}
REEMPLAZOS = {"pygame": "pygame-ce", "pypdf2": "pypdf", "fuzzywuzzy": "rapidfuzz", "pycrypto": "pycryptodome",
              "sklearn": "scikit-learn", "bs4": "beautifulsoup4", "opencv": "opencv-python", "pil": "pillow"}


def guia(tema: str) -> str:
    disponibles = sorted(p.stem for p in DIR_GUIAS.glob("*.md"))
    clave = tema.strip().lower().replace(" ", "-")
    clave = ALIAS.get(clave, clave)
    archivo = DIR_GUIAS / f"{clave}.md"
    if not archivo.exists():
        candidatos = [d for d in disponibles if clave in d or d in clave]
        if len(candidatos) != 1:
            return f"No hay guía '{tema}'. Disponibles: {', '.join(disponibles)}. Para otros temas usa investigar."
        archivo = DIR_GUIAS / f"{candidatos[0]}.md"
    return archivo.read_text(encoding="utf-8")


def _etiqueta_python(python: str) -> str:
    try:
        r = subprocess.run([python, "-c", "import sys; print(f'cp{sys.version_info[0]}{sys.version_info[1]}')"],
                           capture_output=True, text=True, timeout=15, creationflags=subprocess.CREATE_NO_WINDOW, check=False)
        return r.stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return ""


ECOSISTEMAS = {"crates": ("crates", "crates.io", "cargo", "rust"), "go": ("go", "golang"),
               "nuget": ("nuget", "dotnet", ".net", "csharp", "c#"), "docker": ("docker", "dockerhub", "imagen", "contenedor")}
ETIQUETA_VERSION = re.compile(r"^(\d+(?:\.\d+)*)(?:-([a-z][\w.]*))?$")


def _numeros(v: str) -> tuple[int, ...]:
    return tuple(int(x) for x in re.findall(r"\d+", v))


def _crates(h, nombre: str) -> str:
    d = json.loads(h._descargar(f"https://crates.io/api/v1/crates/{urllib.parse.quote(nombre)}")[0])["crate"]
    ultima = d.get("max_stable_version") or d["max_version"]
    txt = (f"crates.io {d['name']} {ultima}: {(d.get('description') or '').strip()[:150]}\n"
           f"Instalar: cargo add {d['name']} (o en Cargo.toml: {d['name']} = \"{ultima}\")")
    return txt + (f"\nDocumentación: {d['documentation']}" if d.get("documentation") else "")


def _go(h, modulo: str) -> str:
    modulo = modulo.strip().rstrip("/")
    if "." not in modulo.split("/")[0]:  # sin dominio: biblioteca estándar o nombre suelto
        return (f"'{modulo}' no es una ruta de módulo de Go. Si es de la biblioteca estándar (net/http, "
                "encoding/json...) no se instala; si no, indica la ruta completa, p. ej. github.com/gin-gonic/gin.")
    ruta = re.sub(r"[A-Z]", lambda m: "!" + m.group(0).lower(), modulo)  # escape del proxy para mayúsculas
    d = json.loads(h._descargar(f"https://proxy.golang.org/{ruta}/@latest")[0])
    txt = f"Go {modulo} {d['Version']} ({d.get('Time', '')[:10]})\nInstalar: go get {modulo}@{d['Version']}"
    mod = h._descargar(f"https://proxy.golang.org/{ruta}/@v/{d['Version']}.mod")[0]
    obsoleto = re.search(r"^//\s*Deprecated:\s*(.+)$", mod, re.MULTILINE)
    return txt + (f"\nOBSOLETO: {obsoleto.group(1).strip()}" if obsoleto else "")


def _nuget(h, paquete: str) -> str:
    versiones = json.loads(h._descargar(f"https://api.nuget.org/v3-flatcontainer/{paquete.lower()}/index.json")[0])["versions"]
    estables = [v for v in versiones if "-" not in v]
    ultima = max(estables or versiones, key=_numeros)
    return f"NuGet {paquete} {ultima}\nInstalar: dotnet add package {paquete} --version {ultima}"


def _docker(h, imagen: str) -> str:
    nombre, _, variante = imagen.strip().partition(":")
    repo = nombre if "/" in nombre else f"library/{nombre}"
    url = f"https://hub.docker.com/v2/repositories/{repo}/tags?page_size=100&ordering=last_updated"
    etiquetas = [r["name"] for r in json.loads(h._descargar(url)[0])["results"]]
    versiones = [(m.group(1), m.group(2) or "", t) for t in etiquetas if (m := ETIQUETA_VERSION.match(t))]
    if variante:
        versiones = [v for v in versiones if variante.lower() in v[1]]
    if not versiones:
        return f"No encontré etiquetas con versión de '{imagen}' en Docker Hub (¿nombre correcto? p. ej. python, node, bitnami/redis)."
    maxima = max((v[0] for v in versiones), key=_numeros)
    misma = sorted({t for num, _, t in versiones if num == maxima}, key=lambda t: (len(t), t))
    corta = ".".join(maxima.split(".")[:2])
    lineas = [f"Docker Hub {nombre}: versión más reciente {maxima}.",
              "Etiquetas para FROM: " + ", ".join(f"{nombre}:{t}" for t in misma[:6]),
              f"Con {nombre}:{corta}[-variante] recibes los parches de {corta}. Evita latest.",
              "Para producción, si el lenguaje tiene versión LTS (Node: las pares), usa la LTS aunque no sea la más reciente."]
    return "\n".join(lineas)


def version(h, paquete: str, ecosistema: str = "pypi") -> str:
    """h: Herramientas (para descargar y conocer el Python del proyecto)."""
    paquete, eco = paquete.strip(), ecosistema.strip().lower()
    consulta = {"crates": _crates, "go": _go, "nuget": _nuget, "docker": _docker}
    try:
        for clave, alias in ECOSISTEMAS.items():
            if eco in alias:
                return consulta[clave](h, paquete)
        if eco in ("npm", "node", "js"):
            d = json.loads(h._descargar(f"https://registry.npmjs.org/{urllib.parse.quote(paquete, safe='@/')}/latest")[0])
            txt = f"npm {d['name']} {d['version']}: {d.get('description', '')[:150]}\nInstalar: npm install {d['name']}"
            if d.get("deprecated"):
                txt += f"\nOBSOLETO: {d['deprecated']}"
            return txt
        if eco in ("maven", "java", "gradle"):
            if ":" in paquete:
                coordenadas = [tuple(paquete.split(":")[:2])]
            else:
                url = "https://search.maven.org/solrsearch/select?" + urllib.parse.urlencode({"q": f'a:"{paquete}"', "rows": 5, "wt": "json"})
                coordenadas = [(x["g"], x["a"]) for x in json.loads(h._descargar(url)[0])["response"]["docs"]]
            if not coordenadas:
                return f"No encontré '{paquete}' en Maven Central."
            lineas = ["Maven Central (versión estable más reciente):"]
            for g, a in coordenadas[:4]:
                meta = h._descargar(f"https://repo1.maven.org/maven2/{g.replace('.', '/')}/{a}/maven-metadata.xml")[0]
                versiones = re.findall(r"<version>([^<]+)</version>", meta)
                estables = [v for v in versiones if not re.search(r"(alpha|beta|rc|m\d|snapshot|cr\d|ea)", v, re.IGNORECASE)]
                ultima = (estables or versiones or ["?"])[-1]
                lineas.append(f"- {g}:{a}:{ultima}   (pom.xml: <groupId>{g}</groupId><artifactId>{a}</artifactId><version>{ultima}</version>)")
            return "\n".join(lineas)
        nombre = REEMPLAZOS.get(paquete.lower(), paquete)
        d = json.loads(h._descargar(f"https://pypi.org/pypi/{urllib.parse.quote(nombre)}/json")[0])
    except Exception as e:
        return f"No pude consultar {paquete} ({eco}): {e}"
    info = d["info"]
    lineas = [(f"PyPI {info['name']} {info['version']} (requiere Python {info.get('requires_python') or '?'}): "
               f"{(info.get('summary') or '')[:150]}")]
    if nombre != paquete:
        lineas.insert(0, f"'{paquete}' está desaconsejado hoy: usa '{nombre}'.")
    etiqueta = _etiqueta_python(h.python_proyecto())
    archivos = [u["filename"] for u in d.get("urls", [])]
    if etiqueta and archivos:
        puro = any(f.endswith("none-any.whl") or "abi3" in f for f in archivos)
        compatible = puro or any(f"-{etiqueta}-" in f and "win_amd64" in f for f in archivos)
        lineas.append(f"Wheel para tu Python ({etiqueta}, Windows): {'SÍ' if compatible else 'NO -> la instalación probablemente fallará; usa otra librería o py -3.12 -m venv .venv'}")
    urls = info.get("project_urls") or {}
    docs = next((v for k, v in urls.items() if re.search(r"doc", k, re.IGNORECASE)), info.get("home_page") or "")
    if docs:
        lineas.append(f"Documentación: {docs}")
    lineas.append(f"requirements.txt: {info['name']}>={info['version'].split('.')[0]}.{info['version'].split('.')[1] if '.' in info['version'] else '0'}")
    return "\n".join(lineas)


def investigar(h, pregunta: str, max_paginas: int = 3) -> str:
    """Busca, lee las mejores páginas y devuelve solo los párrafos relevantes, con fuentes."""
    resultados = h.web_buscar(pregunta)
    urls = re.findall(r"^\s+(https?://\S+)", resultados, re.MULTILINE)
    urls = [u for u in urls if not re.search(r"youtube\.com|tiktok\.com|facebook\.com|pinterest\.|instagram\.", u)]
    if not urls:
        return resultados
    cupo = max(800, h.max_chars // max_paginas - 100)
    partes, vistos = [], set()
    for u in urls:
        if len(partes) >= max_paginas:
            break
        texto = h.web_leer(u, buscar=pregunta)
        if texto.startswith(("No se pudo", "URL inválida")):
            continue  # página bloqueada: probar la siguiente
        cuerpo = texto.split("\n", 1)[1] if "\n" in texto else texto
        utiles = []
        for par in cuerpo.split("\n\n"):
            par = par.strip()
            # fuera menús e índices: párrafos muy cortos sin código, y repetidos entre páginas
            lineas = [l for l in par.splitlines() if l.strip()]
            es_menu = len(lineas) >= 4 and sum(len(l.strip()) for l in lineas) / len(lineas) < 28 and "(" not in par
            if (len(par) < 50 and "(" not in par) or es_menu or par in vistos:
                continue
            vistos.add(par)
            utiles.append(par)
        if utiles:
            partes.append(f"### {u}\n" + "\n\n".join(utiles)[:cupo])
    if not partes:
        return f"No pude leer ninguna página útil. Resultados de búsqueda:\n{resultados}"
    return f"Investigación: {pregunta}\n\n" + "\n\n".join(partes)
