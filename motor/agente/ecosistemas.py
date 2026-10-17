"""verificar() para Rust, Go, .NET, Node/TypeScript, Solidity (Foundry, Hardhat), Anchor, Android, Flutter/Dart,
Docker/Compose y YAML.

Cada proyecto se compila y pasa sus pruebas con su propia herramienta (cargo, go, dotnet, npm, docker). Si falta la
herramienta, el informe dice cómo instalarla; si Docker está apagado, se revisa el Dockerfile sin construirlo.
"""
import hashlib
import json
import os
import re
import shutil
from pathlib import Path

from . import android
from .verificacion import Informe, _correr, _rel, ignorado

COMPILAR = 600   # segundos: la primera compilación descarga dependencias
CONSTRUIR = 900  # docker build
ENTORNO = android.con_android(dict(
    os.environ, CARGO_TERM_COLOR="never", NO_COLOR="1", CI="1", DOTNET_CLI_TELEMETRY_OPTOUT="1", DOTNET_NOLOGO="1",
    DOCKER_CLI_HINTS="false", BUILDKIT_PROGRESS="plain", GOFLAGS="-buildvcs=false", FLUTTER_SUPPRESS_ANALYTICS="true"))
COMPOSE = {"docker-compose.yml", "docker-compose.yaml", "compose.yml", "compose.yaml"}
CREDENCIAL = re.compile(r"\b(\w*(?:PASSWORD|PASSWD|SECRET|TOKEN|API_?KEY|PRIVATE_?KEY|ACCESS_?KEY)\w*)\s*[= ]\s*"
                        r"['\"]?([^\s'\"$]{4,})", re.IGNORECASE)


def _buscar(base: Path, condicion) -> list[Path]:
    return sorted(p for p in base.rglob("*") if p.is_file() and condicion(p) and not ignorado(p, base))


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
        if (m.groupdict().get("tipo") or "error").lower() == "error":
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
    proyectos = [d for d in _raices(_buscar(base, lambda p: p.name == "Cargo.toml"))
                 if not (d / "Anchor.toml").exists()]  # los workspaces de Anchor los compila anchor()
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
            if any((d / n).exists() for n in HARDHAT_CONFIG):
                continue  # las pruebas de Hardhat las ejecuta solidity()
            inf.avisos.append(f"{rel}: no hay pruebas: añade scripts.test en package.json (p. ej. \"node --test\").")
            continue
        codigo, salida = _correr("npm test --silent", d, COMPILAR, ENTORNO)
        if codigo is None:
            inf.errores.append(f"{rel}: npm test no terminó en {COMPILAR} s (¿modo watch? usa vitest run / jest --ci).")
        elif codigo != 0:
            inf.errores.append(f"{rel}: npm test falla:\n    {_fallo(salida, r'fail|error|assert|expected|not ok')}")
        else:
            inf.ok.append(f"{rel}: npm test OK")


# ---------------------------------------------------------------- Solidity (Foundry, Hardhat) y Anchor
SOLC = (r"^(?:(?P<tipo>Warning)|\w*Error)(?: \(\d+\))?: (?P<texto>.+)\n\s*--> (?P<archivo>[^\n:]+?\.sol):"
        r"(?P<linea>\d+):\d+:")
RUSTC_LARGO = (r"^(?P<tipo>error|warning)(?:\[\w+\])?: (?P<texto>.+)\n\s*--> (?P<archivo>[^\n:]+?\.rs):"
               r"(?P<linea>\d+):\d+")
HARDHAT_CONFIG = ("hardhat.config.ts", "hardhat.config.js", "hardhat.config.cjs", "hardhat.config.mjs")


def _foundry(d: Path, raiz: Path, inf: Informe) -> None:
    rel = _rel(d, raiz)
    codigo, salida = _correr(["forge", "build"], d, COMPILAR, ENTORNO)
    if codigo is None:
        inf.errores.append(f"{rel}: forge build no terminó en {COMPILAR} s.")
        return
    errores = _diagnosticos(salida, SOLC, d, raiz, inf)
    if codigo != 0:
        if not errores:
            inf.errores.append(f"{rel}: forge build falla:\n    {_fallo(salida)}")
        return
    codigo, salida = _correr(["forge", "test"], d, COMPILAR, ENTORNO)
    m = re.search(r"(\d+) tests? passed, (\d+) failed", salida)
    pasadas, fallidas = (int(m.group(1)), int(m.group(2))) if m else (0, 0)
    if codigo is None:
        inf.errores.append(f"{rel}: forge test no terminó en {COMPILAR} s (¿un fork sin RPC o un bucle?).")
    elif "No tests found" in salida or codigo == 0 and pasadas == 0:
        inf.avisos.append(f"{rel}: no hay pruebas: añade test/X.t.sol con forge-std (guia('evm')).")
    elif codigo != 0 or fallidas:
        fallos = sorted(set(re.findall(r"^\[FAIL[^\n]*", salida, re.MULTILINE)))[:10]
        inf.errores.append(f"{rel}: forge test falla ({fallidas} de {pasadas + fallidas}):\n    "
                           + ("\n    ".join(fallos) or _fallo(salida)))
    else:
        inf.ok.append(f"{rel}: compila y pasan {pasadas} pruebas (forge test)")


def _hardhat(d: Path, raiz: Path, inf: Informe) -> None:
    rel = _rel(d, raiz)
    if not (d / "node_modules").is_dir():
        return  # node() ya pide npm install
    codigo, salida = _correr("npx --no-install hardhat compile", d, COMPILAR, ENTORNO)
    errores = _diagnosticos(salida, SOLC, d, raiz, inf)
    if codigo != 0:
        if not errores:
            inf.errores.append(f"{rel}: hardhat compile falla:\n    {_fallo(salida)}")
        return
    if not any((d / "test").glob("*.*")):
        inf.avisos.append(f"{rel}: no hay pruebas en test/: añade tests de los contratos (npx hardhat test).")
        return
    codigo, salida = _correr("npx --no-install hardhat test", d, COMPILAR, ENTORNO)
    pasadas = sum(int(n) for n in re.findall(r"(\d+) passing", salida))
    if codigo is None:
        inf.errores.append(f"{rel}: hardhat test no terminó en {COMPILAR} s.")
    elif codigo != 0:
        inf.errores.append(f"{rel}: hardhat test falla:\n    {_fallo(salida, r'fail|error|assert|expected')}")
    else:
        inf.ok.append(f"{rel}: compila y pasan las pruebas de Hardhat" + (f" ({pasadas})" if pasadas else ""))


def solidity(base: Path, raiz: Path, inf: Informe) -> None:
    foundry = _raices(_buscar(base, lambda p: p.name == "foundry.toml"))
    hardhat = _raices(_buscar(base, lambda p: p.name in HARDHAT_CONFIG))
    sueltos = [] if foundry or hardhat else _buscar(base, lambda p: p.suffix == ".sol")
    if not (foundry or hardhat or sueltos):
        return
    inf.tipos.append("solidity")
    if sueltos:
        inf.avisos.append("Hay contratos .sol sin proyecto: crea uno con Foundry (forge init --force .) para compilarlos "
                          "y probarlos (guia('evm')).")
    if foundry and not shutil.which("forge"):
        inf.errores.append("Hay foundry.toml pero Foundry no está instalado: https://getfoundry.sh/ (pide al usuario "
                           "que lo instale).")
        foundry = []
    for d in foundry:
        _foundry(d, raiz, inf)
    for d in hardhat:
        _hardhat(d, raiz, inf)


def _version_de(cmd: list[str]) -> tuple[int, ...]:
    codigo, salida = _correr(cmd, Path.cwd(), 30, ENTORNO)
    m = re.search(r"(\d+)\.(\d+)\.(\d+)", salida) if codigo == 0 else None
    return tuple(map(int, m.groups())) if m else ()


def anchor(base: Path, raiz: Path, inf: Informe) -> None:
    from .herramientas import transaccion  # aquí: herramientas importa este módulo al verificar
    proyectos = _raices(_buscar(base, lambda p: p.name == "Anchor.toml"))
    if not proyectos:
        return
    inf.tipos.append("anchor")
    if not shutil.which("anchor"):
        inf.errores.append("Hay Anchor.toml pero Anchor no está instalado: https://www.anchor-lang.com/docs/installation")
        return
    for d in proyectos:
        rel = _rel(d, raiz)
        codigo, salida = _correr(["anchor", "build"], d, CONSTRUIR, ENTORNO)
        if codigo is None:
            inf.errores.append(f"{rel}: anchor build no terminó en {CONSTRUIR} s (la primera vez descarga las "
                               "herramientas de Solana).")
            continue
        errores = _diagnosticos(salida, RUSTC_LARGO, d, raiz, inf)
        if codigo != 0:
            if not errores:
                inf.errores.append(f"{rel}: anchor build falla:\n    {_fallo(salida)}")
            ancla, sol = _version_de(["anchor", "--version"]), _version_de(["solana", "--version"])
            if ancla[:2] >= (0, 31) and sol and sol[0] < 2:
                inf.avisos.append(f"Anchor {'.'.join(map(str, ancla))} espera Solana/Agave 2.x y la CLI de Solana es "
                                  f"{'.'.join(map(str, sol))}: pide al usuario que la actualice o usa Anchor 0.30 (avm).")
            continue
        inf.ok.append(f"{rel}: anchor build OK (IDL en target/idl)")
        if transaccion("anchor test", d):
            inf.avisos.append(f"{rel}: Anchor.toml apunta a una red que no es local: anchor test desplegaría allí. Pon "
                              "cluster = \"Localnet\" para probar.")
        elif (d / "node_modules").is_dir() and any((d / "tests").glob("*.*")):
            codigo, salida = _correr(["anchor", "test"], d, CONSTRUIR, ENTORNO)
            if codigo == 0:
                inf.ok.append(f"{rel}: anchor test OK (validador local)")
            else:
                inf.errores.append(f"{rel}: anchor test falla:\n    {_fallo(salida, r'fail|error|assert|expected')}")
        else:
            inf.avisos.append(f"{rel}: faltan tests/ o node_modules (npm install) para anchor test.")


# ---------------------------------------------------------------- Android (Gradle) y Flutter/Dart
KOTLIN = r"^(?:(?P<tipo>w)|e): (?:file:///)?(?P<archivo>[^\n]+?\.kts?):(?P<linea>\d+):\d+ (?P<texto>.+)$"
KOTLIN_ANTIGUO = r"^(?:(?P<tipo>w)|e): (?P<archivo>[^\n]+?\.kts?): \((?P<linea>\d+), \d+\): (?P<texto>.+)$"
JAVAC = r"^(?P<archivo>[^\n]+?\.java):(?P<linea>\d+): (?P<tipo>error|warning): (?P<texto>.+)$"
AAPT = r"^ERROR: ?(?P<archivo>[^\n]+?\.xml):(?P<linea>\d+): AAPT: (?P<texto>.+)$"
DART = r"^\s*(?P<tipo>error|warning|info) • (?P<texto>.+?) • (?P<archivo>[^\n•]+?\.dart):(?P<linea>\d+):\d+ • \S+\s*$"


def pistas_gradle(salida: str) -> list[str]:
    """Qué falta en el equipo o en el proyecto según los fallos típicos de Gradle y del plugin de Android."""
    instaladas = ", ".join(f"android-{n}" for n in android.plataformas()) or "ninguna"
    pistas = [
        (r"SDK location not found", ("No encuentra el SDK de Android: abre el proyecto con abrir_android_studio o "
                                     "crea local.properties con sdk.dir (barras dobles \\\\).")),
        (r"licen[cs]es? .*not (been )?accepted|accept the SDK license",
         "Faltan licencias del SDK: el usuario debe aceptarlas en Android Studio (SDK Manager); es su decisión."),
        (r"Failed to find (?:target with hash string|Platform SDK with path)[^\n]*?android-(\d+)",
         (f"Falta esa plataforma del SDK: usa en compileSdk una instalada ({instaladas}) o pide al usuario que la "
          "instale en Android Studio.")),
        (r"requires Java 17|Unsupported class file major version|Java 17|JAVA_HOME is not set",
         "La versión de Java no vale: el plugin de Android 8 necesita JDK 17 o superior en JAVA_HOME."),
        (r"Minimum supported Gradle version is ([\d.]+)",
         "Sube distributionUrl en gradle/wrapper/gradle-wrapper.properties a esa versión de Gradle."),
        (r"Plugin \[id: 'com\.android\.[a-z]+'[^\n]*not found",
         "No encuentra el plugin de Android: añade google() en pluginManagement { repositories } de settings.gradle(.kts)."),
        (r"Could not (?:resolve|GET|download)",
         ("No pudo descargar una dependencia: revisa google() y mavenCentral() en settings.gradle(.kts) y la versión "
          "(version('grupo:artefacto', 'google')).")),
    ]
    return [texto for patron, texto in pistas if re.search(patron, salida, re.IGNORECASE)]


def _fallo_gradle(rel: str, tarea: str, salida: str, d: Path, raiz: Path, inf: Informe) -> None:
    errores = sum(_diagnosticos(salida, p, d, raiz, inf) for p in (KOTLIN, KOTLIN_ANTIGUO, JAVAC, AAPT))
    pistas = pistas_gradle(salida)
    if pistas or not errores:
        detalle = "\n    ".join(pistas) if pistas else _fallo(salida, r"error|fail|what went wrong|exception|>")
        inf.errores.append(f"{rel}: {tarea} falla:\n    {detalle}")


def _pruebas_junit(d: Path, rel: str, inf: Informe) -> None:
    total = fallos = 0
    nombres = []
    for xml in d.glob("**/build/test-results/testDebugUnitTest/*.xml"):
        texto = xml.read_text(encoding="utf-8", errors="replace")
        if (m := re.search(r'<testsuite [^>]*tests="(\d+)"[^>]*failures="(\d+)"[^>]*errors="(\d+)"', texto)):
            total += int(m.group(1))
            fallos += int(m.group(2)) + int(m.group(3))
        nombres += re.findall(r'<testcase name="([^"]+)"[^>]*>\s*<(?:failure|error)', texto)
    if fallos:
        inf.errores.append(f"{rel}: fallan {fallos} de {total} pruebas unitarias: {', '.join(nombres[:8])}")
    elif total:
        inf.ok.append(f"{rel}: pasan {total} pruebas unitarias (testDebugUnitTest)")
    else:
        inf.avisos.append(f"{rel}: no hay pruebas unitarias: añade app/src/test/java/.../XTest.kt con JUnit.")


def android_gradle(base: Path, raiz: Path, inf: Informe) -> None:
    def es_android(p: Path) -> bool:
        return p.name in ("build.gradle", "build.gradle.kts") and "com.android" in p.read_text("utf-8", "replace")

    proyectos = set()
    for modulo in (p.parent for p in _buscar(base, es_android)):
        dentro = [modulo, *(a for a in modulo.parents if a == base or base in a.parents)]
        if any((a / "pubspec.yaml").exists() for a in dentro[:3]):
            continue  # el android/ de Flutter lo compila flutter()
        proyectos.add(next((a for a in dentro if (a / "settings.gradle").exists() or (a / "settings.gradle.kts").exists()),
                           modulo))
    if not proyectos:
        return
    inf.tipos.append("android")
    if not android.sdk():
        inf.errores.append("No encuentro el SDK de Android: instala Android Studio (https://developer.android.com/studio) "
                           "y ábrelo una vez para que lo descargue.")
        return
    java = _version_de(["java", "-version"])
    mayor = java[1] if java[:1] == (1,) else java[0] if java else 0  # Java 8 se anuncia como 1.8
    if java and mayor < 17:
        inf.errores.append(f"Java {mayor} no vale para el plugin de Android 8: hace falta JDK 17 o superior en JAVA_HOME.")
        return
    for d in sorted(set(proyectos)):
        rel = _rel(d, raiz)
        if not (d / "gradlew.bat").exists():
            local = android.gradle_local()
            orden = f'"{local[1]}" wrapper --gradle-version {local[0]}' if local else "abrir_android_studio"
            inf.errores.append(f"{rel}: falta el Gradle wrapper (gradlew.bat). Créalo con: {orden}")
            continue
        codigo, salida = _correr("gradlew.bat assembleDebug --console=plain", d, CONSTRUIR, ENTORNO)
        if codigo is None:
            inf.errores.append(f"{rel}: assembleDebug no terminó en {CONSTRUIR} s (la primera vez descarga el plugin y "
                               "las librerías; vuelve a verificar).")
            continue
        if codigo != 0:
            _fallo_gradle(rel, "assembleDebug", salida, d, raiz, inf)
            continue
        apk = next(iter(sorted(d.glob("**/build/outputs/apk/debug/*.apk"))), None)
        inf.ok.append(f"{rel}: compila (assembleDebug)" + (f"; APK en {_rel(apk, raiz)}" if apk else ""))
        codigo, salida = _correr("gradlew.bat testDebugUnitTest --console=plain", d, CONSTRUIR, ENTORNO)
        if codigo is None:
            inf.errores.append(f"{rel}: testDebugUnitTest no terminó en {CONSTRUIR} s.")
        else:
            _pruebas_junit(d, rel, inf)


def flutter(base: Path, raiz: Path, inf: Informe) -> None:
    proyectos = _raices(_buscar(base, lambda p: p.name == "pubspec.yaml"))
    if not proyectos:
        return
    inf.tipos.append("flutter")
    for d in proyectos:
        rel = _rel(d, raiz)
        es_flutter = re.search(r"^\s*sdk:\s*flutter\b", (d / "pubspec.yaml").read_text("utf-8", "replace"), re.MULTILINE)
        cli = "flutter" if es_flutter else "dart"
        if not shutil.which(cli):
            inf.errores.append(f"Hay pubspec.yaml pero falta {cli.capitalize()}: https://docs.flutter.dev/get-started/install")
            continue
        if not (d / ".dart_tool" / "package_config.json").exists():
            codigo, salida = _correr(f"{cli} pub get", d, COMPILAR, ENTORNO)
            if codigo != 0:
                inf.errores.append(f"{rel}: {cli} pub get falla:\n    {_fallo(salida)}")
                continue
        analizar = "flutter analyze --no-pub --no-fatal-infos --no-fatal-warnings" if es_flutter else "dart analyze"
        _, salida = _correr(analizar, d, COMPILAR, ENTORNO)
        if _diagnosticos(salida, DART, d, raiz, inf):
            continue
        if any((d / "test").glob("**/*_test.dart")):
            codigo, salida = _correr(f"{cli} test" + (" --no-pub" if es_flutter else ""), d, COMPILAR, ENTORNO)
            m = re.search(r"\+(\d+)(?: ~\d+)?(?: -(\d+))?: (?:All tests passed|Some tests failed)", salida)
            if codigo is None:
                inf.errores.append(f"{rel}: {cli} test no terminó en {COMPILAR} s.")
                continue
            if codigo != 0:
                inf.errores.append(f"{rel}: {cli} test falla:\n    {_fallo(salida, r'fail|error|expected|actual')}")
                continue
            inf.ok.append(f"{rel}: pasan {m.group(1) if m else 'las'} pruebas ({cli} test)")
        else:
            inf.avisos.append(f"{rel}: no hay pruebas: añade test/x_test.dart (flutter_test).")
        if es_flutter and (d / "android").is_dir() and android.sdk():
            codigo, salida = _correr("flutter build apk --debug --no-pub", d, CONSTRUIR, ENTORNO)
            if codigo is None:
                inf.errores.append(f"{rel}: flutter build apk no terminó en {CONSTRUIR} s (la primera vez descarga Gradle "
                                   "y el plugin de Android; vuelve a verificar).")
            elif codigo != 0:
                _fallo_gradle(rel, "flutter build apk", salida, d, raiz, inf)
            else:
                inf.ok.append(f"{rel}: compila para Android (build/app/outputs/flutter-apk/app-debug.apk)")


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
    for comprobar in (rust, go, dotnet, node, solidity, anchor, android_gradle, flutter, docker, yaml):
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
