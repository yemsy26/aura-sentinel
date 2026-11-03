"""Lo común a quienes miran o comprueban un proyecto: qué archivos cuentan como código, el informe de calidad y cómo
correr un comando sin colgarse. No depende de ningún otro módulo del agente salvo procesos."""
import re
import subprocess
import tempfile
from pathlib import Path

from . import procesos

IGNORAR = {".git", ".venv", "venv", "env", "node_modules", "__pycache__", "dist", "build", ".next", ".agente",
           "target", "out", "obj", ".idea", ".vscode", ".pytest_cache", ".mypy_cache", ".ruff_cache", "site-packages"}
# Lo que el agente trata como código (revisión, mapa, puntos de restauración, protección contra reescrituras).
CODIGO = {".py", ".js", ".mjs", ".cjs", ".ts", ".tsx", ".jsx", ".vue", ".svelte", ".html", ".htm", ".css", ".scss",
          ".java", ".kt", ".kts", ".gradle", ".rs", ".go", ".cs", ".fs", ".csproj", ".fsproj", ".sln", ".c", ".h",
          ".cpp", ".hpp", ".bat", ".cmd", ".ps1", ".sh", ".sql", ".proto", ".json", ".yml", ".yaml", ".toml", ".xml",
          ".sol", ".dart"}
NOMBRES_CODIGO = {"dockerfile", "makefile", "go.mod", ".dockerignore", ".gitignore", ".env.example", "procfile"}


def es_codigo(p: Path) -> bool:
    nombre = p.name.lower()
    return (p.suffix.lower() in CODIGO or nombre in NOMBRES_CODIGO or nombre.startswith("dockerfile.")
            or nombre.endswith(".dockerfile"))

SIN_VENTANA = 0x08000000  # CREATE_NO_WINDOW


class Informe:
    def __init__(self):
        self.errores, self.avisos, self.ok, self.tipos = [], [], [], []
        self.previos: list[str] = []  # modo grande: errores que ya estaban antes de empezar (no cuentan)

    def texto(self, maximo: int) -> str:
        cab = (f"VERIFICACIÓN ({', '.join(self.tipos) or 'sin código'}): {len(self.errores)} ERRORES, "
               f"{len(self.avisos)} AVISOS")
        partes = [cab]
        for titulo, lista in (("ERRORES (corrígelos todos)", self.errores), ("AVISOS (revisa cada uno)", self.avisos),
                              ("OK", self.ok)):
            if lista:
                partes.append(f"{titulo}:\n" + "\n".join(f"- {x}" for x in lista[:25]))
        if self.previos:
            partes.append(f"PREEXISTENTES ({len(self.previos)}; ya estaban antes de esta tarea: no los toques salvo que "
                          "el usuario lo pida):\n" + "\n".join(f"- {x[:160]}" for x in self.previos[:5]))
        txt = "\n".join(partes)
        return txt if len(txt) <= maximo else txt[:maximo - 40] + "\n[... informe recortado]"


def ignorado(p: Path, raiz: Path) -> bool:
    """Carpetas que no son código del proyecto: entornos, compilados y dependencias (también lib/ de Foundry, con
    forge-std y OpenZeppelin, que traen sus propios scripts y package.json, y sus cache/ y broadcast/; y gen/ e icons/
    junto a tauri.conf.json, que Tauri regenera al compilar: sus esquemas, ~300 KB, tapaban el código al revisor)."""
    partes = p.relative_to(raiz).parts[:-1]
    if IGNORAR & set(partes):
        return True
    return any((parte in ("lib", "cache", "broadcast") and (raiz.joinpath(*partes[:i]) / "foundry.toml").exists())
               or (parte in ("gen", "icons") and (raiz.joinpath(*partes[:i]) / "tauri.conf.json").exists())
               for i, parte in enumerate(partes))


def archivos(raiz: Path, *exts: str) -> list[Path]:
    return sorted(p for p in raiz.rglob("*") if p.suffix.lower() in exts and p.is_file() and not ignorado(p, raiz))


def relativa(p: Path, raiz: Path) -> str:
    try:
        return str(p.resolve().relative_to(raiz))
    except ValueError:
        return str(p)


def correr(cmd, cwd, timeout, entorno=None, quedan: list | None = None) -> tuple[int | None, bytes]:
    """(código o None si se pasó del tiempo, salida en bruto). Si se pasa del tiempo, cierra su trabajo entero (solo
    sus procesos: ver procesos.py).

    La salida va a un archivo temporal y no a una tubería: si el comando deja algo en segundo plano (`start /b`, un
    servidor), ese proceso hereda la salida y con una tubería la espera no acabaría nunca. Lo que deje abierto (una
    ventana con `start`, el demonio de Gradle o el compilador de .NET, que aceleran la siguiente compilación) sigue
    abierto como antes, también si el motor acaba; con `quedan`, su trabajo se añade ahí para poder cerrarlo."""
    with tempfile.TemporaryFile() as archivo:
        proc, trabajo = procesos.lanzar(cmd, cwd=cwd, env=entorno, stdin=subprocess.DEVNULL, stdout=archivo,
                                        stderr=subprocess.STDOUT, creationflags=SIN_VENTANA, shell=isinstance(cmd, str))
        try:
            codigo = proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            procesos.detener(proc, trabajo)
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                pass
            codigo = None
        if trabajo:
            trabajo.matar_al_cerrar(False)  # lo que quede abierto sobrevive al motor, como antes
            if quedan is not None and trabajo.pids():
                quedan.append((proc, trabajo))
            else:
                trabajo.cerrar()
        archivo.seek(0)
        return codigo, archivo.read()


def correr_texto(cmd, cwd, timeout=60, entorno=None) -> tuple[int | None, str]:
    """(código o None si timeout, salida como texto)."""
    codigo, salida = correr(cmd, cwd, timeout, entorno)
    try:
        texto = salida.decode("utf-8")
    except UnicodeDecodeError:
        texto = salida.decode("cp850", "replace")
    return codigo, texto.strip()


def cola(texto: str, n: int = 12) -> str:
    return "\n    ".join(texto.strip().splitlines()[-n:])


def normalizar(error: str) -> str:
    """Un error sin números (líneas, recuentos): reconoce los que ya estaban aunque el código se mueva."""
    return re.sub(r"\d+", "N", error)
