"""Dónde están el SDK de Android, Android Studio y Gradle en este equipo, y el entorno para compilar con ellos."""
import os
import re
from pathlib import Path

LOCAL = Path(os.environ.get("LOCALAPPDATA", Path.home() / "AppData" / "Local"))
CASA = Path(os.environ.get("USERPROFILE", Path.home()))


def sdk() -> Path | None:
    """ANDROID_HOME, ANDROID_SDK_ROOT o la carpeta donde lo instala Android Studio."""
    for candidata in (os.environ.get("ANDROID_HOME"), os.environ.get("ANDROID_SDK_ROOT"), LOCAL / "Android" / "Sdk"):
        if candidata and (Path(candidata) / "platform-tools").is_dir():
            return Path(candidata)
    return None


def plataformas(raiz: Path | None = None) -> list[int]:
    """Niveles de API instalados (android-34 -> 34)."""
    raiz = raiz or sdk()
    if not raiz or not (raiz / "platforms").is_dir():
        return []
    return sorted(int(m.group(1)) for p in (raiz / "platforms").iterdir() if (m := re.fullmatch(r"android-(\d+)", p.name)))


def con_android(env: dict) -> dict:
    """El entorno con ANDROID_HOME y las herramientas del SDK (adb, emulator, sdkmanager) en el PATH."""
    raiz = sdk()
    if not raiz:
        return env
    env = dict(env)
    env.setdefault("ANDROID_HOME", str(raiz))
    env.setdefault("ANDROID_SDK_ROOT", str(raiz))
    extra = [raiz / "platform-tools", raiz / "emulator", raiz / "cmdline-tools" / "latest" / "bin"]
    env["PATH"] = os.pathsep.join([*(str(d) for d in extra if d.is_dir()), env.get("PATH", "")])
    return env


def _completa(exe: Path) -> bool:
    """Una instalación a medias (restos de otra versión) no trae su Java: no arrancaría."""
    raiz = exe.parent.parent
    return (raiz / "product-info.json").is_file() and (raiz / "jbr" / "lib" / "jvm.cfg").is_file()


def android_studio() -> Path | None:
    """studio64.exe de una instalación completa (puede haber varias: Android Studio, Android Studio1...)."""
    programas = Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "Android"
    toolbox = sorted((LOCAL / "JetBrains" / "Toolbox" / "apps").glob("AndroidStudio/**/bin/studio64.exe"))
    candidatas = [*sorted(programas.glob("Android Studio*/bin/studio64.exe")),
                  LOCAL / "Programs" / "Android Studio" / "bin" / "studio64.exe", *reversed(toolbox)]
    existentes = [exe for exe in candidatas if exe.is_file()]
    return next((exe for exe in existentes if _completa(exe)), existentes[0] if existentes else None)


def gradle_local() -> tuple[str, Path] | None:
    """(versión, gradle.bat) de la distribución de Gradle más reciente ya descargada por algún wrapper."""
    hallados = []
    for bat in (CASA / ".gradle" / "wrapper" / "dists").glob("gradle-*/*/gradle-*/bin/gradle.bat"):
        m = re.match(r"gradle-(\d+(?:\.\d+)+)$", bat.parent.parent.name)
        if m:
            hallados.append((tuple(map(int, m.group(1).split("."))), m.group(1), bat))
    return max(hallados)[1:] if hallados else None


def emuladores() -> list[str]:
    avd = CASA / ".android" / "avd"
    return sorted(p.stem for p in avd.glob("*.ini")) if avd.is_dir() else []
