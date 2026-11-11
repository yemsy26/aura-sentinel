"""Ingeniería inversa de APK, para apps propias o con permiso: dónde están apktool, jadx y las build-tools del SDK,
cómo quedan en el PATH de los comandos del agente y cómo se instalan (versión y SHA-256 fijados, solo con permiso).
Uso: python -m agente.inversa instalar"""
import hashlib
import os
import shutil
import sys
import urllib.request
import zipfile
from pathlib import Path

from . import android

BASE = Path(os.environ.get("LOCALAPPDATA", Path.home() / "AppData" / "Local")) / "AuraSentinel" / "herramientas"
HERRAMIENTAS = {
    "apktool": {"version": "3.0.3", "bytes": 15478013,
                "url": "https://github.com/iBotPeaches/Apktool/releases/download/v3.0.3/apktool_3.0.3.jar",
                "sha256": "dbf930b076c6b9be08d57c449cacefc3bdd6b71ebd59b3066fc0e1f5b14f9423"},
    "jadx": {"version": "1.5.6", "bytes": 72646741,
             "url": "https://github.com/skylot/jadx/releases/download/v1.5.6/jadx-1.5.6.zip",
             "sha256": "545ea2be9c242511bc145755cf4bda2485ade42966e096f8b4d3da2a230e8974"},
}
# apktool es un .jar: este .bat permite escribir «apktool d app.apk» como en su documentación.
ENVOLTORIO = '@echo off\r\njava -jar "%~dp0apktool.jar" %*\r\n'


def carpeta(nombre: str) -> Path:
    return BASE / f"{nombre}-{HERRAMIENTAS[nombre]['version']}"


def apktool() -> Path | None:
    jar = carpeta("apktool") / "apktool.jar"
    return jar if jar.is_file() else None


def jadx() -> Path | None:
    bat = carpeta("jadx") / "bin" / "jadx.bat"
    return bat if bat.is_file() else None


def build_tools() -> Path | None:
    """La versión más nueva de las build-tools que trae apksigner, zipalign y aapt2."""
    raiz = android.sdk()
    if not raiz or not (raiz / "build-tools").is_dir():
        return None
    def clave(d: Path) -> tuple:
        return tuple(int(x) if x.isdigit() else 0 for x in d.name.split("."))
    validas = [d for d in (raiz / "build-tools").iterdir() if (d / "apksigner.bat").is_file() and (d / "zipalign.exe").is_file()]
    return max(validas, key=clave) if validas else None


def con_herramientas(env: dict) -> dict:
    """El entorno con apktool, jadx, apksigner, zipalign y aapt2 en el PATH (las que estén instaladas)."""
    extra = []
    if (jar := apktool()) and (jar.parent / "apktool.bat").is_file():
        extra.append(jar.parent)
    if (bat := jadx()):
        extra.append(bat.parent)
    if (bt := build_tools()):
        extra.append(bt)
    if not extra:
        return env
    env = dict(env)
    env["PATH"] = os.pathsep.join([*map(str, extra), env.get("PATH", "")])
    return env


def resumen() -> str:
    """Lo que el agente lee en su entorno."""
    tiene = [f"{n} {HERRAMIENTAS[n]['version']}" for n, ruta in (("apktool", apktool()), ("jadx", jadx())) if ruta]
    if (bt := build_tools()):
        tiene.append(f"apksigner/zipalign/aapt2 (build-tools {bt.name})")
    faltan = [n for n, ruta in (("apktool", apktool()), ("jadx", jadx())) if not ruta]
    texto = f"ingeniería inversa de APK: {', '.join(tiene)}" if tiene else ""
    if faltan and bt:
        texto += (f"{'; ' if texto else 'ingeniería inversa de APK: '}faltan {' y '.join(faltan)} (las instala el "
                  "usuario con instalar_herramientas.bat del motor; pídeselo con preguntar si las necesitas)")
    return texto


def _bajar(url: str, destino: Path) -> None:
    peticion = urllib.request.Request(url, headers={"User-Agent": "AuraSentinel"})
    with urllib.request.urlopen(peticion, timeout=60) as r, destino.open("wb") as f:
        shutil.copyfileobj(r, f, 4 * 1024 * 1024)


def instalar(nombres: list[str] | None = None) -> list[str]:
    """Baja a una carpeta vacía, comprueba el SHA-256 y coloca cada herramienta que falte. Devuelve lo hecho."""
    hecho = []
    for nombre in nombres or list(HERRAMIENTAS):
        datos = HERRAMIENTAS[nombre]
        if (apktool() if nombre == "apktool" else jadx()):
            hecho.append(f"{nombre} {datos['version']}: ya estaba")
            continue
        temporal = BASE / f"_descarga_{nombre}"
        shutil.rmtree(temporal, ignore_errors=True)
        temporal.mkdir(parents=True)
        archivo = temporal / datos["url"].rsplit("/", 1)[1]
        print(f"Bajando {archivo.name} ({datos['bytes'] / 1e6:.1f} MB) de {datos['url']} ...", flush=True)
        _bajar(datos["url"], archivo)
        real = hashlib.sha256(archivo.read_bytes()).hexdigest()
        if real != datos["sha256"]:
            shutil.rmtree(temporal, ignore_errors=True)
            raise RuntimeError(f"{archivo.name}: la huella SHA-256 no coincide ({real}); no se instala")
        destino = carpeta(nombre)
        shutil.rmtree(destino, ignore_errors=True)
        destino.mkdir(parents=True)
        if nombre == "apktool":
            shutil.move(str(archivo), destino / "apktool.jar")
        else:
            with zipfile.ZipFile(archivo) as z:
                for m in z.infolist():
                    if not str((destino / m.filename).resolve()).startswith(str(destino.resolve())):
                        raise RuntimeError(f"{archivo.name}: ruta fuera de su carpeta ({m.filename}); no se instala")
                z.extractall(destino)
        shutil.rmtree(temporal, ignore_errors=True)
        hecho.append(f"{nombre} {datos['version']}: instalado en {destino}")
    if (jar := apktool()) and not (jar.parent / "apktool.bat").is_file():
        (jar.parent / "apktool.bat").write_bytes(ENVOLTORIO.encode("ascii"))
    return hecho


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    if sys.argv[1:2] != ["instalar"]:
        sys.exit(__doc__)
    if not shutil.which("java"):
        sys.exit("[ERROR] apktool y jadx necesitan Java 11 o más nuevo (no encuentro java).")
    try:
        for linea in instalar():
            print("[OK]", linea)
    except (OSError, RuntimeError) as e:
        sys.exit(f"[ERROR] {e}")
