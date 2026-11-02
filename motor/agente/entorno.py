"""Entorno de los comandos del proyecto, sin lo que dejó el programa que abrió Aura para compilarse él mismo.

Aura abierto en desarrollo (cargo run, npm run tauri dev) pasa al motor CARGO_TARGET_DIR, CARGO_PKG_*, npm_*,
TAURI_CONFIG... y los heredaban los comandos del proyecto del usuario: cargo compilaba su app en la carpeta de Aura."""
import os
import re

# Los ponen cargo run y npm run para el programa que lanzan: nunca son del proyecto del usuario.
DEL_LANZADOR = re.compile(r"(CARGO_(MANIFEST_DIR|MANIFEST_PATH|PKG_\w+|CRATE_NAME|BIN_NAME|PRIMARY_PACKAGE)|"
                          r"NPM_\w+|INIT_CWD|RUST_RECURSION_COUNT)", re.IGNORECASE)
# Pueden ser una configuración del usuario: se quitan si no están guardadas en sus variables de Windows.
DEL_USUARIO = re.compile(r"(CARGO_TARGET_DIR|CARGO_BUILD_TARGET_DIR|RUSTUP_TOOLCHAIN|TAURI_(CONFIG|ENV_\w+|CLI_\w+|"
                         r"DEV_\w+))", re.IGNORECASE)
BIN_NODE = re.compile(r"[\\/]node_modules[\\/]\.bin[\\/]?$", re.IGNORECASE)  # npm run lo pone delante en el PATH
SISTEMA = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment"


def guardada(nombre: str) -> str | None:
    """Valor de la variable en las del usuario o las del sistema (registro de Windows)."""
    try:
        import winreg
    except ImportError:
        return os.environ.get(nombre)  # fuera de Windows no se sabe: se respeta
    for raiz, clave in ((winreg.HKEY_CURRENT_USER, "Environment"), (winreg.HKEY_LOCAL_MACHINE, SISTEMA)):
        try:
            with winreg.OpenKey(raiz, clave) as k:
                return os.path.expandvars(str(winreg.QueryValueEx(k, nombre)[0]))
        except OSError:
            continue
    return None


def carpetas_guardadas() -> list[str]:
    """Carpetas del PATH del sistema y luego las del usuario, como las guarda Windows."""
    try:
        import winreg
    except ImportError:
        return []
    carpetas = []
    for raiz, clave in ((winreg.HKEY_LOCAL_MACHINE, SISTEMA), (winreg.HKEY_CURRENT_USER, "Environment")):
        try:
            with winreg.OpenKey(raiz, clave) as k:
                carpetas += winreg.ExpandEnvironmentStrings(str(winreg.QueryValueEx(k, "Path")[0])).split(os.pathsep)
        except OSError:
            continue
    return carpetas


def completo(ruta: str, guardadas: list[str]) -> str:
    """El PATH heredado más las carpetas guardadas que le falten. Lo abierto desde el escritorio (Aura, Bionic)
    puede no recibir el PATH del usuario (pasa si es muy largo) y sin él no se encuentran uv, cargo, Python..."""
    def clave(carpeta: str) -> str:
        return carpeta.strip().rstrip("\\/").lower()

    vistas = {"", *(clave(c) for c in ruta.split(os.pathsep))}  # "": las vacías no se añaden
    nuevas = []
    for c in guardadas:
        if clave(c) not in vistas:
            vistas.add(clave(c))
            nuevas.append(c)
    base = ruta.rstrip(os.pathsep)
    return os.pathsep.join(([base] if base else []) + nuevas)


def limpio(env: dict, guardada=guardada) -> dict:
    env = dict(env)
    for nombre in list(env):
        if DEL_LANZADOR.fullmatch(nombre) or (DEL_USUARIO.fullmatch(nombre) and guardada(nombre) != env[nombre]):
            del env[nombre]
    for clave in [k for k in env if k.upper() == "PATH"]:
        env[clave] = os.pathsep.join(d for d in env[clave].split(os.pathsep) if not BIN_NODE.search(d))
    return env


def limpiar_proceso() -> None:
    """Deja os.environ limpio antes de que nadie lo copie (ecosistemas.ENTORNO se calcula al importarse)."""
    nuevo = limpio(dict(os.environ))
    for nombre in [n for n in os.environ if n not in nuevo]:
        del os.environ[nombre]
    os.environ.update(nuevo)
    os.environ["PATH"] = completo(os.environ.get("PATH", ""), carpetas_guardadas())
