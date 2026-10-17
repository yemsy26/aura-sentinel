"""Motores complementarios: cerebros opcionales que el usuario activa y que se obtienen de su repositorio en GitHub.

Cada motor se desarrolla y versiona en su propio repositorio; aquí solo está cómo conseguirlo, cómo comprobar que
funciona y qué herramienta añade al agente. Sin activarlo, el agente funciona igual, sin esa herramienta.

Los marcados como predeterminados se activan al instalar el motor (`asegurar`), salvo que el usuario los haya
desactivado: esa decisión se respeta en las reinstalaciones.

Uso: python -m agente.complementos [listar | asegurar | activar ID | desactivar ID] [--json] [--pausa]
"""
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

from .config import RAIZ_INSTALACION

DIR = RAIZ_INSTALACION / "complementos"
UA = "AgenteLocal-complementos"
SIN_VENTANA = getattr(subprocess, "CREATE_NO_WINDOW", 0)
CATALOGO = {
    "sat-hibrido": {
        "nombre": "Sat-hibrido-rust",
        "repo": "https://github.com/yemsy26/Sat-hibrido-rust",
        "aporta": ("Resolvedor SAT (CDCL con capa de paridad GF(2), ruptura de simetrías y varios hilos). Añade la "
                   "herramienta resolver_sat: sudokus, horarios, coloreado, asignaciones y otros problemas de "
                   "restricciones, con el modelo comprobado contra cada cláusula."),
        "herramientas": ["resolver_sat"],
        "predeterminado": True,  # CPU, sin VRAM, 657 KB: vale la pena tenerlo siempre
        "ejecutable": "hunt.exe",
        "binario_release": "hunt-x86_64-pc-windows-msvc.exe",  # con su .sha256 al lado en la release
        "fuentes": [("https://github.com/yemsy26/Sat-hibrido-rust", "")],  # Chronos-VFS llega por cargo (git)
        "compilar": ("dpll_check", ["cargo", "build", "--release", "--bin", "hunt"], "target/release/hunt.exe"),
        "usa": [{"nombre": "Chronos-VFS", "repo": "https://github.com/yemsy26/Chronos-VFS",
                 "papel": "anillo en memoria y compresión zstd para las trazas de prueba del resolvedor"}],
    },
}


def _estado() -> dict:
    try:
        return json.loads((DIR / "estado.json").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}


def _guardar(estado: dict) -> None:
    DIR.mkdir(parents=True, exist_ok=True)
    (DIR / "estado.json").write_text(json.dumps(estado, indent=1, ensure_ascii=False), encoding="utf-8")


def ruta(id_: str) -> Path | None:
    """Ejecutable del complemento si está activado y en su sitio; None si no."""
    if not _estado().get(id_, {}).get("activo"):
        return None
    exe = DIR / id_ / CATALOGO[id_]["ejecutable"]
    return exe if exe.is_file() else None


def listar() -> list[dict]:
    estado = _estado()
    return [{"id": i, **{k: c[k] for k in ("nombre", "repo", "aporta", "herramientas", "usa")},
             "activo": ruta(i) is not None, "origen": estado.get(i, {}).get("origen", "")} for i, c in CATALOGO.items()]


def _descargar(url: str) -> bytes:
    pedido = urllib.request.Request(url, headers={"User-Agent": UA, "Accept": "application/vnd.github+json"})
    with urllib.request.urlopen(pedido, timeout=60) as r:  # solo URLs del catálogo y de sus releases
        return r.read()


def _desde_release(c: dict, destino: Path) -> str | None:
    """Binario de la última versión publicada en el repositorio; None si no hay versión con ese archivo."""
    api = c["repo"].replace("https://github.com/", "https://api.github.com/repos/") + "/releases/latest"
    try:
        release = json.loads(_descargar(api))
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return None
        raise
    archivos = {a["name"]: a["browser_download_url"] for a in release.get("assets", [])}
    nombre = c["binario_release"]
    if nombre not in archivos or f"{nombre}.sha256" not in archivos:
        return None
    esperado = _descargar(archivos[f"{nombre}.sha256"]).decode("utf-8", "replace").split()[0].lower()
    datos = _descargar(archivos[nombre])
    if hashlib.sha256(datos).hexdigest() != esperado:
        raise RuntimeError(f"La SHA-256 de {nombre} no coincide con la publicada: no se instala.")
    destino.write_bytes(datos)
    return f"versión {release.get('tag_name', '?')} de {c['repo']}"


def _correr(orden: list[str], cwd: Path, segundos: int, env: dict | None = None) -> str:
    r = subprocess.run(orden, cwd=cwd, capture_output=True, text=True, encoding="utf-8", errors="replace",
                       timeout=segundos, env=env, creationflags=SIN_VENTANA, check=False)
    if r.returncode != 0:
        raise RuntimeError(f"Falló {' '.join(orden[:3])}: {(r.stdout + r.stderr).strip()[-500:]}")
    return r.stdout.strip()


def _compilar(c: dict, destino: Path, aviso) -> str:
    falta = [h for h in ("git", "cargo") if not shutil.which(h)]
    if falta:
        raise RuntimeError(f"No hay versión publicada con binario y para compilarlo falta {' y '.join(falta)} "
                           "(Git: https://git-scm.com/, Rust: https://rustup.rs/).")
    with tempfile.TemporaryDirectory(prefix="complemento_") as tmp:
        base, origen = Path(tmp) / "fuente", []
        for repo, sub in c["fuentes"]:
            carpeta = base / sub if sub else base
            if carpeta.exists():  # el repositorio ya la trae
                continue
            aviso(f"Descargando {repo} ...")
            _correr(["git", "clone", "--depth", "1", "-q", repo, str(carpeta)], Path(tmp), 300)
            origen.append(f"{repo.rsplit('/', 1)[1]}@{_correr(['git', 'rev-parse', '--short', 'HEAD'], carpeta, 30)}")
        sub, orden, salida = c["compilar"]
        aviso("Compilando (la primera vez tarda uno o dos minutos) ...")
        _correr(orden, base / sub, 1200, dict(os.environ, CARGO_TERM_COLOR="never"))
        shutil.copyfile(base / sub / salida, destino)
    return "compilado de " + ", ".join(origen)


def _probar(exe: Path) -> None:
    """El resolvedor debe decir satisfacible (10) e insatisfacible (20) en dos fórmulas conocidas."""
    with tempfile.TemporaryDirectory(prefix="complemento_prueba_") as tmp:
        for texto, esperado in (("p cnf 2 2\n1 2 0\n-1 0\n", 10), ("p cnf 1 2\n1 0\n-1 0\n", 20)):
            f = Path(tmp) / "f.cnf"
            f.write_text(texto, encoding="utf-8")
            r = subprocess.run([str(exe), str(f), "--timeout", "20"], capture_output=True, timeout=60,
                               creationflags=SIN_VENTANA, check=False)
            if r.returncode != esperado:
                raise RuntimeError(f"{exe.name} no funciona: devolvió {r.returncode} y se esperaba {esperado}.")


def activar(id_: str, aviso=print) -> str:
    if id_ not in CATALOGO:
        raise ValueError(f"No existe el complemento '{id_}'. Disponibles: {', '.join(CATALOGO)}")
    c = CATALOGO[id_]
    carpeta = DIR / id_
    carpeta.mkdir(parents=True, exist_ok=True)
    nuevo = carpeta / (c["ejecutable"] + ".nuevo")
    aviso(f"Buscando una versión publicada de {c['nombre']} en GitHub ...")
    origen = _desde_release(c, nuevo) or _compilar(c, nuevo, aviso)
    _probar(nuevo)
    os.replace(nuevo, carpeta / c["ejecutable"])
    estado = _estado()
    estado[id_] = {"activo": True, "origen": origen, "fecha": time.strftime("%Y-%m-%d %H:%M"),
                   "sha256": hashlib.sha256((carpeta / c["ejecutable"]).read_bytes()).hexdigest()}
    _guardar(estado)
    return f"{c['nombre']} activado ({origen}). Herramientas nuevas del agente: {', '.join(c['herramientas'])}."


def asegurar(aviso=print) -> list[str]:
    """Activa los predeterminados que nunca se han configurado (o cuyo ejecutable ya no está). Nunca falla: sin red,
    el motor sigue funcionando sin ellos y se pueden activar después desde Aura."""
    estado, mensajes = _estado(), []
    for id_, c in CATALOGO.items():
        if not c.get("predeterminado"):
            continue
        if id_ in estado and (not estado[id_].get("activo") or ruta(id_)):
            mensajes.append(f"{c['nombre']}: {'activo' if ruta(id_) else 'desactivado por el usuario'}.")
            continue
        try:
            mensajes.append(activar(id_, aviso))
        except (RuntimeError, ValueError, OSError, subprocess.SubprocessError) as e:
            mensajes.append(f"{c['nombre']} no se pudo activar ahora ({e}). Actívalo después en Aura: Configuración "
                            "→ Motor → Motores complementarios.")
    return mensajes


def desactivar(id_: str) -> str:
    if id_ not in CATALOGO:
        raise ValueError(f"No existe el complemento '{id_}'.")
    estado = _estado()
    if id_ in estado:
        estado[id_]["activo"] = False
        _guardar(estado)
    return f"{CATALOGO[id_]['nombre']} desactivado: el agente deja de usar {', '.join(CATALOGO[id_]['herramientas'])}."


def main() -> int:
    ap = argparse.ArgumentParser(description="Motores complementarios del agente")
    ap.add_argument("accion", choices=["listar", "asegurar", "activar", "desactivar"])
    ap.add_argument("id", nargs="?")
    ap.add_argument("--json", action="store_true", help="salida JSON (para Aura)")
    ap.add_argument("--pausa", action="store_true", help="espera Intro al terminar (consola de Aura)")
    a = ap.parse_args()
    codigo = 0
    try:
        if a.accion == "listar":
            datos = listar()
            print(json.dumps(datos, ensure_ascii=False) if a.json else "\n".join(
                f"{'[activo] ' if d['activo'] else '[ -- ]   '}{d['id']}: {d['nombre']} — {d['repo']}" for d in datos))
        elif a.accion == "asegurar":
            mensajes = asegurar(aviso=(lambda _: None) if a.json else print)
            print(json.dumps(listar(), ensure_ascii=False) if a.json else "\n".join(f"[info] {m}" for m in mensajes))
        elif not a.id:
            ap.error("falta el ID del complemento")
        else:
            texto = activar(a.id) if a.accion == "activar" else desactivar(a.id)
            print(json.dumps(listar(), ensure_ascii=False) if a.json else f"[OK] {texto}")
    except (RuntimeError, ValueError, OSError, subprocess.SubprocessError) as e:
        print(f"[ERROR] {e}", file=sys.stderr if a.json else sys.stdout)
        codigo = 1
    if a.pausa:
        input("\nPulsa Intro para cerrar esta ventana...")
    return codigo


if __name__ == "__main__":
    sys.exit(main())
