"""python -m agente.descargar [--modelo CLAVE] [--sin-vision] [--carpeta DIR]

Descarga el modelo del agente (GGUF y, si se quiere, su proyector de visión mmproj) desde Hugging Face a la carpeta de
modelos de LM Studio / Bionic. Reanuda si se corta y comprueba tamaño y SHA-256 con los que publica Hugging Face. Lo
que ya está y cuadra no se vuelve a bajar."""
import argparse
import hashlib
import json
import shutil
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

from . import config

MODELOS = {  # clave de LM Studio -> repositorio, GGUF y proyector de visión
    "qwen3.6-35b-a3b": {"repo": "unsloth/Qwen3.6-35B-A3B-GGUF", "gguf": "Qwen3.6-35B-A3B-UD-Q3_K_XL.gguf",
                        "vision": "mmproj-F16.gguf"},
}
APARTADOS = Path.home() / ".lmstudio" / "mmproj-desactivados"  # donde deja el mmproj `vision.bat off`
BLOQUE = 4 * 1024 * 1024


def carpeta_modelos() -> Path:
    """La de LM Studio (settings.json → downloadsFolder) o la de por defecto."""
    try:
        propia = json.loads((Path.home() / ".lmstudio" / "settings.json").read_text(encoding="utf-8")).get("downloadsFolder")
    except (OSError, ValueError):
        propia = None
    return Path(propia) if propia else Path.home() / ".lmstudio" / "models"


def publicados(repo: str) -> dict[str, tuple[int, str]]:
    """{archivo: (bytes, sha256)} según Hugging Face."""
    with urllib.request.urlopen(f"https://huggingface.co/api/models/{repo}/tree/main", timeout=30) as r:
        return {x["path"]: (x["lfs"]["size"], x["lfs"]["oid"]) for x in json.load(r) if x.get("lfs")}


def _gb(n: float) -> str:
    return f"{n / 1024 ** 3:.1f}".replace(".", ",")


def sha256(ruta: Path) -> str:
    h, total, hecho, aviso = hashlib.sha256(), ruta.stat().st_size, 0, time.time()
    with open(ruta, "rb") as f:
        while bloque := f.read(BLOQUE):
            h.update(bloque)
            hecho += len(bloque)
            if time.time() - aviso > 5:
                print(f"    comprobando {ruta.name}: {100 * hecho // total}%", flush=True)
                aviso = time.time()
    return h.hexdigest()


def bajar(url: str, parte: Path, tam: int) -> None:
    """Descarga a `parte` continuando lo que ya hubiera (cabecera Range)."""
    hecho = parte.stat().st_size if parte.exists() else 0
    if hecho > tam:
        parte.unlink()
        hecho = 0
    if hecho == tam:
        return
    pedido = urllib.request.Request(url, headers={"Range": f"bytes={hecho}-"} if hecho else {})
    with urllib.request.urlopen(pedido, timeout=60) as r:
        if hecho and r.status != 206:  # el servidor no reanuda: se empieza de cero
            hecho = 0
        with open(parte, "ab" if hecho else "wb") as f:
            inicio, base, aviso = time.time(), hecho, 0.0
            while bloque := r.read(BLOQUE):
                f.write(bloque)
                hecho += len(bloque)
                if time.time() - aviso > 3:
                    vel = (hecho - base) / max(time.time() - inicio, 0.1)
                    falta = (tam - hecho) / vel if vel else 0
                    print(f"    {parte.name[:-5]}: {100 * hecho // tam}%  {_gb(hecho)}/{_gb(tam)} GB  "
                          f"{vel / 1024 ** 2:.0f} MB/s  quedan {falta / 60:.0f} min", flush=True)
                    aviso = time.time()


def asegurar(repo: str, nombre: str, carpeta: Path, tam: int, sha: str, intentos: int = 5) -> Path:
    """Deja `carpeta/nombre` descargado y verificado. Devuelve su ruta."""
    destino = carpeta / nombre
    if destino.exists() and destino.stat().st_size == tam:
        print(f"  {nombre}: ya está; compruebo que no esté dañado...", flush=True)
        if sha256(destino) == sha:
            print(f"  [OK] {nombre} verificado.")
            return destino
        print(f"  [AVISO] {nombre} no coincide con el publicado: lo descargo de nuevo.")
        destino.unlink()
    carpeta.mkdir(parents=True, exist_ok=True)
    parte = destino.with_name(nombre + ".part")
    url = f"https://huggingface.co/{repo}/resolve/main/{nombre}"
    print(f"  Descargando {nombre} ({_gb(tam)} GB) de huggingface.co/{repo} ...", flush=True)
    for intento in range(1, intentos + 1):
        try:
            bajar(url, parte, tam)
            break
        except (urllib.error.URLError, ConnectionError, TimeoutError) as e:
            if intento == intentos:
                raise RuntimeError(f"la descarga de {nombre} falló {intentos} veces: {e}. Vuelve a ejecutarlo: "
                                   "continúa donde se quedó.") from e
            print(f"    corte de red ({e}); reintento {intento + 1}/{intentos} en 10 s, sin perder lo bajado...")
            time.sleep(10)
    if parte.stat().st_size != tam or sha256(parte) != sha:
        parte.unlink()
        raise RuntimeError(f"{nombre} llegó dañado (el SHA-256 no coincide con el de Hugging Face). Vuelve a ejecutarlo.")
    parte.replace(destino)
    print(f"  [OK] {nombre} descargado y verificado.")
    return destino


def main() -> int:
    for flujo in (sys.stdout, sys.stderr):
        flujo.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(prog="agente.descargar", description=__doc__.split("\n\n")[1])
    ap.add_argument("--modelo", help=f"clave del modelo ({', '.join(MODELOS)}); por defecto la de agente.json")
    ap.add_argument("--sin-vision", action="store_true", help="no bajar el proyector de visión (mmproj, ~0,9 GB)")
    ap.add_argument("--carpeta", type=Path, help="carpeta de modelos (por defecto la de LM Studio)")
    a = ap.parse_args()
    clave = a.modelo or config.cargar().get("modelo", "")
    if clave not in MODELOS:
        print(f"[ERROR] No sé descargar '{clave}'. Conocidos: {', '.join(MODELOS)}. Otros modelos: desde LM Studio.")
        return 2
    m = MODELOS[clave]
    carpeta = (a.carpeta or carpeta_modelos()) / m["repo"]
    try:
        info = publicados(m["repo"])
        archivos = [m["gguf"]] + ([] if a.sin_vision else [m["vision"]])
        if not a.sin_vision and (APARTADOS / m["repo"] / m["vision"]).exists():
            print("  La visión está desactivada (vision.bat off): no bajo el mmproj. `vision.bat on` la restaura.")
            archivos.remove(m["vision"])
        falta = sum(info[x][0] for x in archivos if not (carpeta / x).exists())
        existente = next(d for d in (carpeta, *carpeta.parents) if d.exists())
        libre = shutil.disk_usage(existente).free
        if falta and libre < falta + 2 * 1024 ** 3:
            print(f"[ERROR] Hace falta {_gb(falta)} GB y quedan {_gb(libre)} GB libres en el disco de {carpeta}.")
            return 1
        print(f"Modelo {clave} → {carpeta}")
        for nombre in archivos:
            asegurar(m["repo"], nombre, carpeta, *info[nombre])
    except (OSError, RuntimeError, KeyError, ValueError) as e:
        print(f"[ERROR] {e}")
        return 1
    print("\n[OK] Modelo listo. Abre (o reinicia) Bionic para que lo vea y después ejecuta cargar_modelo.bat:\n"
          "     ajusta a tu VRAM cuántas capas de expertos van a la RAM/CPU y el contexto que cabe.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
