"""verificar() para Rust, Go, .NET, Node/TypeScript, Docker/Compose y YAML.

Cada proyecto se compila y pasa sus pruebas con su propia herramienta (cargo, go, dotnet, npm, docker). Si falta la
herramienta, el informe dice cómo instalarla; si Docker está apagado, se revisa el Dockerfile sin construirlo.
"""
import hashlib
import json
import os
import re
import shutil
from pathlib import Path

from .verificacion import IGNORAR, Informe, _correr, _rel

COMPILAR = 600   # segundos: la primera compilación descarga dependencias
CONSTRUIR = 900  # docker build
ENTORNO = dict(os.environ, CARGO_TERM_COLOR="never", NO_COLOR="1", CI="1", DOTNET_CLI_TELEMETRY_OPTOUT="1",
               DOTNET_NOLOGO="1", DOCKER_CLI_HINTS="false", BUILDKIT_PROGRESS="plain", GOFLAGS="-buildvcs=false")
COMPOSE = {"docker-compose.yml", "docker-compose.yaml", "compose.yml", "compose.yaml"}
CREDENCIAL = re.compile(r"\b(\w*(?:PASSWORD|PASSWD|SECRET|TOKEN|API_?KEY|PRIVATE_?KEY|ACCESS_?KEY)\w*)\s*[= ]\s*"
                        r"['\"]?([^\s'\"$]{4,})", re.IGNORECASE)


def _buscar(base: Path, condicion) -> list[Path]:
    return sorted(p for p in base.rglob("*") if p.is_file() and condicion(p)
                  and not (IGNORAR & set(p.relative_to(base).parts[:-1])))


def _raices(archivos: list[Path]) -> list[Path]:
    """Carpetas de proyecto sin repetir las que están dentro de otra (workspaces de cargo, monorepos de npm)."""
    carpetas = sorted({a.parent for a in archivos}, key=lambda d: len(d.parts))
    return [d for i, d in enumerate(carpetas) if not any(o in d.parents for o in carpetas[:i])]


def _diagnosticos(salida: str, patron: str, cwd: Path, raiz: Path, inf: Informe, prefijo: str = "") -> int:
    """Líneas archivo:línea del compilador -> errores y avisos del informe (sin 'tipo' cuentan como error).

    Devuelve cuántos errores encontró."""
    vistos, errores = set(), 0
    for m in re.finditer(patron, salida, re.MULTILINE):
        archivo = cwd / m.group("archivo").lstrip(".\\/")
        detalle = f"{prefijo}{_rel(archivo, raiz)}:{m.group('linea')} {m.group('texto').strip()}"
        if detalle in vistos:
            continue
        vistos.add(detalle)
        if (m.groupdict().get("tipo") or "error") == "error":
            errores += 1
            inf.errores.append(detalle)
        elif len(inf.avisos) < 20:
            inf.avisos.append(detalle)
    return errores


def _fallo(salida: str, filtro: str = r"error|fail|panic|assert|expected|exception", n: int = 15) -> str:
    utiles = [l for l in salida.splitlines() if re.search(filtro, l, re.IGNORECASE)]
    return "\n    ".join((utiles or salida.splitlines())[-n:])


# ---------------------------------------------------------------- Rust
RUSTC = r"^(?P<archivo>[^\s:][^:\n]*?\.rs):(?P<linea>\d+):\d+: (?P<tipo>error|warning)(?:\[\w+\])?: (?P<texto>.+)$"
CLIPPY = RUSTC.replace("(?P<tipo>error|warning)", "(?P<tipo>warning)")


def rust(base: Path, raiz: Path, inf: Informe) -> None:
    proyectos = _raices(_buscar(base, lambda p: p.name == "Cargo.toml"))
    if not proyectos:
        return
    inf.tipos.append("rust")
    if not shutil.which("cargo"):
        inf.errores.append("Hay Cargo.toml pero Rust no está instalado: https://rustup.rs/ (pide al usuario que lo instale).")
        return
    for d in proyectos:
        rel = _rel(d, raiz)
        codigo, salida = _correr(["cargo", "build", "--all-targets", "--message-format", "short"], d, COMPILAR, ENTORNO)
        if codigo is None:
            inf.errores.append(f"{rel}: cargo build no terminó en {COMPILAR} s.")
            continue
        errores = _diagnosticos(salida, RUSTC, d, raiz, inf)
        if codigo != 0:
            if not errores:
                inf.errores.append(f"{rel}: cargo build falla:\n    {_fallo(salida)}")
            continue
        codigo, salida = _correr(["cargo", "test", "--quiet"], d, COMPILAR, ENTORNO)
        pasadas = sum(int(n) for n in re.findall(r"test result: \w+\. (\d+) passed", salida))
        if codigo is None:
            inf.errores.append(f"{rel}: cargo test no terminó en {COMPILAR} s (¿una prueba espera datos o no acaba?).")
        elif codigo != 0:
            inf.errores.append(f"{rel}: cargo test falla:\n    {_fallo(salida)}")
        elif pasadas == 0:
            inf.avisos.append(f"{rel}: no hay pruebas. Añade #[cfg(test)] mod tests con #[test] que prueben los requisitos.")
        else:
            inf.ok.append(f"{rel}: compila y pasan {pasadas} pruebas (cargo test)")
        if codigo == 0 and _correr(["cargo", "clippy", "--version"], d, 30, ENTORNO)[0] == 0:
            _, salida = _correr(["cargo", "clippy", "--all-targets", "--message-format", "short"], d, COMPILAR, ENTORNO)
            _diagnosticos(salida, CLIPPY, d, raiz, inf, "clippy: ")


# ---------------------------------------------------------------- Go
FALLO_GO = r"FAIL|---|_test\.go|panic|error"
GOC = r"^(?:vet(?:\.exe)?: )?(?P<archivo>\.?[\\/]?[^\s:#][^:\n]*?\.go):(?P<linea>\d+):\d+: (?P<texto>.+)$"


def go(base: Path, raiz: Path, inf: Informe) -> None:
    proyectos = _raices(_buscar(base, lambda p: p.name == "go.mod"))
    if not proyectos:
        return
    inf.tipos.append("go")
    if not shutil.which("go"):
        inf.errores.append("Hay go.mod pero Go no está instalado: https://go.dev/dl/ (pide al usuario que lo instale).")
        return
    for d in proyectos:
        rel = _rel(d, raiz)
        # go vet compila: detecta errores de compilación y fallos típicos (formatos, copias de locks...).
        codigo, salida = _correr(["go", "vet", "./..."], d, COMPILAR, ENTORNO)
        if codigo is None:
            inf.errores.append(f"{rel}: go vet no terminó en {COMPILAR} s.")
            continue
        if codigo != 0:
            if not _diagnosticos(salida, GOC, d, raiz, inf):
                inf.errores.append(f"{rel}: go vet falla:\n    {_fallo(salida, r'.')}")
            continue
        codigo, salida = _correr(["go", "test", "./..."], d, COMPILAR, ENTORNO)
        paquetes = len(re.findall(r"^ok\s", salida, re.MULTILINE))
        if codigo is None:
            inf.errores.append(f"{rel}: go test no terminó en {COMPILAR} s (¿una prueba no acaba?).")
        elif codigo != 0:
            inf.errores.append(f"{rel}: go test falla:\n    {_fallo(salida, FALLO_GO)}")
        elif paquetes == 0:
            inf.avisos.append(f"{rel}: no hay pruebas. Crea archivos *_test.go con func TestXxx(t *testing.T).")
        else:
            inf.ok.append(f"{rel}: compila, go vet limpio y pasan las pruebas de {paquetes} paquete(s)")
        _, salida = _correr(["gofmt", "-l", "."], d, 60, ENTORNO)
        sin_formato = [l.strip() for l in salida.splitlines() if l.strip().endswith(".go")]
        if sin_formato:
            inf.avisos.append(f"{rel}: sin formato estándar: {', '.join(sin_formato[:5])} (ejecuta gofmt -w .)")


# ---------------------------------------------------------------- .NET
DOTNET = (r"^\s*(?P<archivo>[^\s(][^(\n]*?\.(?:cs|fs|vb))\((?P<linea>\d+),\d+\): (?P<tipo>error|warning) "
          r"(?P<texto>\w+: .+?)(?: \[[^\]]+\])?$")


def dotnet(base: Path, raiz: Path, inf: Informe) -> None:
    soluciones = _buscar(base, lambda p: p.suffix.lower() in (".sln", ".slnx"))
    proyectos = _buscar(base, lambda p: p.suffix.lower() in (".csproj", ".fsproj", ".vbproj"))
    if not soluciones and not proyectos:
        return
    inf.tipos.append(".net")
    if not shutil.which("dotnet"):
        inf.errores.append("Hay proyecto .NET pero el SDK no está instalado: https://dotnet.microsoft.com/download")
        return
    compila = True
    for objetivo in soluciones[:1] or proyectos:
        codigo, salida = _correr(["dotnet", "build", str(objetivo), "-nologo", "-v", "q", "-clp:NoSummary"],
                                 objetivo.parent, COMPILAR, ENTORNO)
        if codigo is None:
            inf.errores.append(f"{_rel(objetivo, raiz)}: dotnet build no terminó en {COMPILAR} s.")
            compila = False
            continue
        errores = _diagnosticos(salida, DOTNET, objetivo.parent, raiz, inf)
        if codigo != 0:
            compila = False
            if not errores:
                inf.errores.append(f"{_rel(objetivo, raiz)}: dotnet build falla:\n    {_fallo(salida)}")
    if not compila:
        return
    pruebas = [p for p in proyectos if re.search(r"Microsoft\.NET\.Test\.Sdk|xunit|nunit|mstest",
                                                p.read_text(encoding="utf-8", errors="replace"), re.IGNORECASE)]
    if not pruebas:
        inf.ok.append("dotnet build sin errores")
        inf.avisos.append("No hay proyecto de pruebas (.NET): crea uno con 'dotnet new xunit' y referencia el código.")
        return
    for t in pruebas:
        codigo, salida = _correr(["dotnet", "test", str(t), "--no-build", "-nologo", "-v", "q"], t.parent, COMPILAR, ENTORNO)
        if codigo == 0:
            inf.ok.append(f"{_rel(t, raiz)}: dotnet test OK")
        else:
            inf.errores.append(f"{_rel(t, raiz)}: dotnet test falla:\n    {_fallo(salida)}")


# ---------------------------------------------------------------- Node / TypeScript
TSC = r"^(?P<archivo>[^\s(][^(\n]*?\.(?:ts|tsx|mts|cts))\((?P<linea>\d+),\d+\): (?P<tipo>error) (?P<texto>TS\d+: .+)$"
TEST_POR_DEFECTO = "no test specified"


def node(base: Path, raiz: Path, inf: Informe) -> None:
    paquetes = _raices(_buscar(base, lambda p: p.name == "package.json"))
    if not paquetes:
        return
    inf.tipos.append("node")
    if not shutil.which("npm"):
        inf.errores.append("Hay package.json pero Node.js no está instalado: https://nodejs.org/")
        return
    for d in paquetes:
        rel = _rel(d, raiz)
        try:
            pkg = json.loads((d / "package.json").read_text(encoding="utf-8"))
        except ValueError as e:
            inf.errores.append(f"{rel}/package.json no es JSON válido: {e}")
            continue
        if ({**pkg.get("dependencies", {}), **pkg.get("devDependencies", {})}
                and not (d / "node_modules").is_dir()):
            inf.errores.append(f"{rel}: faltan las dependencias (no hay node_modules): ejecuta npm install en esa carpeta.")
            continue
        scripts = pkg.get("scripts", {})
        if (d / "tsconfig.json").exists():
            tsc = d / "node_modules" / ".bin" / "tsc.cmd"
            if tsc.exists():
                codigo, salida = _correr([str(tsc), "--noEmit", "-p", "."], d, 300, ENTORNO)
                if codigo != 0 and not _diagnosticos(salida, TSC, d, raiz, inf):
                    inf.errores.append(f"{rel}: tsc falla:\n    {_fallo(salida)}")
            else:
                inf.avisos.append(f"{rel}: hay tsconfig.json pero TypeScript no está instalado: npm install -D typescript")
        if "build" in scripts:
            codigo, salida = _correr("npm run build --silent", d, COMPILAR, ENTORNO)
            if codigo != 0:
                inf.errores.append(f"{rel}: npm run build falla:\n    {_fallo(salida)}")
                continue
        prueba = scripts.get("test", "")
        if not prueba or TEST_POR_DEFECTO in prueba:
            inf.avisos.append(f"{rel}: no hay pruebas: añade scripts.test en package.json (p. ej. \"node --test\").")
            continue
        codigo, salida = _correr("npm test --silent", d, COMPILAR, ENTORNO)
        if codigo is None:
            inf.errores.append(f"{rel}: npm test no terminó en {COMPILAR} s (¿modo watch? usa vitest run / jest --ci).")
        elif codigo != 0:
            inf.errores.append(f"{rel}: npm test falla:\n    {_fallo(salida, r'fail|error|assert|expected|not ok')}")
        else:
            inf.ok.append(f"{rel}: npm test OK")


# ---------------------------------------------------------------- Docker / Compose / YAML
def es_dockerfile(p: Path) -> bool:
    n = p.name.lower()
    return n == "dockerfile" or n.startswith("dockerfile.") or n.endswith(".dockerfile")


def docker_activo() -> bool:
    if not shutil.which("docker"):
        return False
    codigo, salida = _correr(["docker", "info", "--format", "{{.ServerVersion}}"], None, 20, ENTORNO)
    return codigo == 0 and bool(re.fullmatch(r"[\w.\-+]+", salida.strip()))


def _instrucciones(texto: str) -> list[tuple[int, str, str]]:
    """(línea, INSTRUCCIÓN, argumentos), uniendo las líneas que acaban en barra invertida."""
    out, acumulado, inicio = [], "", 0
    for i, linea in enumerate(texto.splitlines(), 1):
        limpia = linea.strip()
        if not acumulado and (not limpia or limpia.startswith("#")):
            continue
        if not acumulado:
            inicio = i
        acumulado += " " + limpia.rstrip("\\")
        if not limpia.endswith("\\"):
            partes = acumulado.strip().split(None, 1)
            out.append((inicio, partes[0].upper(), partes[1] if len(partes) > 1 else ""))
            acumulado = ""
    return out


def _origenes(args: str) -> list[str]:
    args = re.sub(r"--[\w-]+(=\S+)?\s+", "", args).strip()
    try:
        partes = json.loads(args) if args.startswith("[") else args.split()
    except ValueError:
        return []
    return partes[:-1] if len(partes) > 1 else []


def _existe(contexto: Path, origen: str) -> bool:
    relativo = origen.lstrip("/") or "."
    if (contexto / relativo).exists():
        return True
    try:
        return any(contexto.glob(relativo))
    except ValueError:  # patrón que glob no acepta: no se puede afirmar que falte
        return True


def _credencial(args: str) -> bool:
    return any(not m.group(1).upper().endswith(("_FILE", "_PATH", "_DIR")) for m in CREDENCIAL.finditer(args))


def revisar_dockerfile(f: Path, raiz: Path, inf: Informe, contexto_listo: bool = True) -> None:
    rel, contexto = _rel(f, raiz), f.parent
    pasos = _instrucciones(f.read_text(encoding="utf-8", errors="replace"))
    if not any(ins == "FROM" for _, ins, _ in pasos):
        inf.errores.append(f"{rel}: no tiene FROM (toda imagen parte de una base).")
        return
    etapas = set()
    for n, ins, args in pasos:
        if ins == "FROM":
            palabras = [x for x in args.split() if not x.startswith("--")]
            imagen = palabras[0] if palabras else ""
            if len(palabras) >= 3 and palabras[1].upper() == "AS":
                etapas.add(palabras[2].lower())
            sin_version = ":" not in imagen.split("/")[-1] or imagen.endswith(":latest")
            if imagen.lower() not in etapas and imagen not in ("scratch", "") and "$" not in imagen \
                    and "@" not in imagen and sin_version:
                inf.avisos.append(f"{rel}:{n} FROM {imagen} sin versión fija: usa una etiqueta concreta (p. ej. :1.23-alpine).")
        elif ins in ("COPY", "ADD") and "--from" not in args and contexto_listo:
            for o in _origenes(args):
                if not re.match(r"https?://", o) and "$" not in o and not _existe(contexto, o):
                    inf.errores.append(f"{rel}:{n} {ins} '{o}': no existe en el contexto de construcción "
                                       f"({_rel(contexto, raiz)}).")
        elif ins in ("ENV", "ARG") and _credencial(args):
            inf.errores.append(f"{rel}:{n} {ins} con una credencial escrita en la imagen: pásala al ejecutar (-e o secrets).")
    if not any(ins == "USER" for _, ins, _ in pasos):
        inf.avisos.append(f"{rel}: la imagen se ejecuta como root: crea un usuario y añade USER antes del CMD.")
    pesados = [d for d in ("node_modules", ".venv", "target", ".git", "bin", "obj") if (contexto / d).is_dir()]
    if pesados and not (contexto / ".dockerignore").exists():
        inf.avisos.append(f"{rel}: falta .dockerignore (el contexto incluye {', '.join(pesados)}): la imagen será lenta y pesada.")


def docker(base: Path, raiz: Path, inf: Informe) -> None:
    dockerfiles = _buscar(base, es_dockerfile)
    composes = _buscar(base, lambda p: p.name.lower() in COMPOSE)
    if not dockerfiles and not composes:
        return
    inf.tipos.append("docker")
    errores_antes = len(inf.errores)
    for f in dockerfiles:
        revisar_dockerfile(f, raiz, inf)
    if not shutil.which("docker"):
        inf.avisos.append("Docker no está instalado: solo se revisó el Dockerfile. Dile al usuario que instale "
                          "Docker Desktop (https://docs.docker.com/desktop/) para construir y probar la imagen.")
        return
    for c in composes:
        codigo, salida = _correr(["docker", "compose", "-f", str(c), "config", "-q"], c.parent, 60, ENTORNO)
        if codigo != 0:
            inf.errores.append(f"{_rel(c, raiz)}: docker compose config falla:\n    {_fallo(salida, r'.')}")
    if not dockerfiles or len(inf.errores) > errores_antes:  # con COPY rotos el build falla igual: primero eso
        return
    if not docker_activo():
        inf.avisos.append("Docker está instalado pero NO en marcha: solo se revisó el Dockerfile. Pide al usuario que "
                          "abra Docker Desktop y vuelve a verificar para construir la imagen.")
        return
    for f in dockerfiles:
        etiqueta = "aura-verificar-" + hashlib.sha1(str(f).encode()).hexdigest()[:10]
        codigo, salida = _correr(["docker", "build", "-f", str(f), "-t", etiqueta, str(f.parent)], f.parent,
                                 CONSTRUIR, ENTORNO)
        if codigo is None:
            inf.errores.append(f"{_rel(f, raiz)}: docker build no terminó en {CONSTRUIR} s.")
        elif codigo != 0:
            inf.errores.append(f"{_rel(f, raiz)}: docker build falla:\n    {_fallo(salida, r'error|failed|not found|denied')}")
        else:
            inf.ok.append(f"{_rel(f, raiz)}: la imagen se construye (docker build)")
            _correr(["docker", "image", "rm", "-f", etiqueta], None, 60, ENTORNO)


def tabuladores_yaml(f: Path, raiz: Path) -> str:
    for n, linea in enumerate(f.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
        if re.match(r" *\t", linea):
            return f"{_rel(f, raiz)}:{n} YAML no admite tabuladores para sangrar: usa espacios."
    return ""


def yaml(base: Path, raiz: Path, inf: Informe) -> None:
    archivos = _buscar(base, lambda p: p.suffix.lower() in (".yml", ".yaml"))
    if archivos and not inf.tipos:
        inf.tipos.append("yaml")
    inf.errores.extend(e for e in (tabuladores_yaml(f, raiz) for f in archivos) if e)


# ---------------------------------------------------------------- entrada
def verificar_todos(base: Path, raiz: Path, inf: Informe) -> None:
    for comprobar in (rust, go, dotnet, node, docker, yaml):
        comprobar(base, raiz, inf)


def chequeo_rapido(raiz: Path, p: Path, inf: Informe) -> None:
    """Tras escribir un archivo: solo lo inmediato (sintaxis de Go y Rust, Dockerfile, tabuladores en YAML)."""
    suf = p.suffix.lower()
    if suf == ".go" and shutil.which("gofmt"):
        codigo, salida = _correr(["gofmt", "-e", "-l", str(p)], p.parent, 30, ENTORNO)
        if codigo != 0:
            inf.errores.extend(f"{_rel(p, raiz)}:{m.group(1)} {m.group(2)}"
                               for m in re.finditer(r":(\d+):\d+: (.+)$", salida, re.MULTILINE))
    elif suf == ".rs" and shutil.which("rustfmt"):
        _, salida = _correr(["rustfmt", "--check", "--edition", "2021", str(p)], p.parent, 30, ENTORNO)
        for m in re.finditer(r"^error: (.+)\n\s*--> [^\n]*?:(\d+):\d+", salida, re.MULTILINE):
            if "file not found for module" not in m.group(1):  # el módulo puede estar aún por crear
                inf.errores.append(f"{_rel(p, raiz)}:{m.group(2)} {m.group(1)}")
    elif es_dockerfile(p):
        revisar_dockerfile(p, raiz, inf, contexto_listo=False)  # lo que copia puede estar aún por crear
    elif suf in (".yml", ".yaml") and (error := tabuladores_yaml(p, raiz)):
        inf.errores.append(error)
