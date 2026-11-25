"""Radar mensual de Aura: qué hay instalado en este equipo y qué ha cambiado fuera desde la última revisión (ver RADAR.md).

  python bench/radar.py                      informe en ..\\aura-radar\\AAAA-MM.md (fuera del repo: lleva el estado del equipo)
  python bench/radar.py --desde 2026-10-25   mirar desde esa fecha en vez de desde la última revisión
  python bench/radar.py --sin-red            solo el estado local
  python bench/radar.py --seco               no actualiza ultimo.json (la próxima revisión mira desde donde miraba)

Solo peticiones GET públicas a api.github.com y huggingface.co, con User-Agent genérico y sin datos del equipo en la URL.
No cambia código ni mediciones: escribe un informe y deja «qué le importa a Aura» y «qué probar» para quien revise.
"""
import argparse
import http.client
import json
import os
import re
import shutil
import subprocess
import sys
import urllib.error
import urllib.request
from datetime import UTC, datetime, timedelta
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
FUENTES = Path(__file__).resolve().parent / "radar_fuentes.json"
LMSTUDIO = Path.home() / ".lmstudio"
TIPOS = ("github", "llama_cpp", "huggingface", "web")
DIAS_PRIMERA = 30
MAX_LISTA = 8
AGENTE = "aura-radar"


# ------------------------------------------------------------------ fuentes y red
def cargar_fuentes(archivo: Path = FUENTES) -> list[dict]:
    fuentes = json.loads(archivo.read_text(encoding="utf-8"))["fuentes"]
    ids = [f["id"] for f in fuentes]
    if len(ids) != len(set(ids)):
        raise ValueError("ids repetidos en las fuentes")
    for f in fuentes:
        if f.get("tipo") not in TIPOS or not f.get("importa"):
            raise ValueError(f"fuente mal definida: {f.get('id')}")
        if f["tipo"] in ("github", "llama_cpp") and not re.fullmatch(r"[\w.-]+/[\w.-]+", f.get("repo", "")):
            raise ValueError(f"repo mal escrito en {f['id']}")
        if f["tipo"] == "huggingface" and not f.get("autor"):
            raise ValueError(f"falta el autor en {f['id']}")
        if f["tipo"] == "web" and not (f.get("url", "").startswith("https://") and f.get("que_buscar")):
            raise ValueError(f"la fuente web {f['id']} necesita https y qué buscar")
    return fuentes


def http_get(url: str, timeout: int = 20):
    """GET público sin credenciales; devuelve el JSON. Falla con OSError/ValueError (el llamador lo cuenta como «no respondió»)."""
    req = urllib.request.Request(url, headers={"User-Agent": AGENTE, "Accept": "application/json"})
    for intento in (1, 2):  # una respuesta grande puede cortarse a mitad de lectura: se repite una vez
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                return json.loads(r.read().decode("utf-8"))
        except urllib.error.HTTPError as e:
            motivo = "límite de peticiones de GitHub (60 por hora sin token)" if e.code in (403, 429) else f"HTTP {e.code}"
            raise OSError(motivo) from e
        except http.client.HTTPException as e:  # IncompleteRead y compañía no son OSError
            if intento == 2:
                raise OSError(f"respuesta cortada ({type(e).__name__})") from e
    return None


def fecha(texto: str) -> datetime:
    return datetime.fromisoformat(texto).astimezone(UTC)


def _release(r: dict) -> dict:
    return {"tag": r["tag_name"], "nombre": (r.get("name") or "").strip(), "fecha": r["published_at"][:10], "url": r["html_url"],
            "prerelease": bool(r.get("prerelease"))}


def releases_nuevas(repo: str, desde: datetime, get=http_get) -> list[dict]:
    datos = get(f"https://api.github.com/repos/{repo}/releases?per_page=100")
    return [_release(r) for r in datos if not r.get("draft") and r.get("published_at") and fecha(r["published_at"]) > desde]


def modelos_nuevos(autor: str, desde: datetime, get=http_get) -> list[dict]:
    datos = get(f"https://huggingface.co/api/models?author={autor}&sort=createdAt&direction=-1&limit=30")
    return [{"id": m["id"], "fecha": m["createdAt"][:10]} for m in datos if m.get("createdAt") and fecha(m["createdAt"]) > desde]


def comparar_llama(repo: str, commit: str, get=http_get, ultima: dict | None = None) -> dict:
    """Cuánto se separa el commit local de llama.cpp de su rama principal y de su última release (`ultima`, si ya se pidió)."""
    info = get(f"https://api.github.com/repos/{repo}")
    principal = info.get("default_branch") or "master"
    a = get(f"https://api.github.com/repos/{repo}/compare/{commit}...{principal}")
    out = {"rama": principal, "nuevos_en_rama": a.get("ahead_by"), "locales_fuera_de_rama": a.get("behind_by")}
    try:
        ultima = ultima or get(f"https://api.github.com/repos/{repo}/releases/latest")
        b = get(f"https://api.github.com/repos/{repo}/compare/{commit}...{ultima['tag_name']}")
        out |= {"release": ultima["tag_name"], "nuevos_en_release": b.get("ahead_by"), "locales_sobre_release": b.get("behind_by")}
    except OSError:
        pass  # sin release o sin respuesta: queda la comparación con la rama
    return out


# ------------------------------------------------------------------ estado local
def _texto(args: list[str], timeout: int = 20, env: dict | None = None) -> str:
    """Salida (estándar y de error) de una orden corta; vacía si no existe o tarda más de lo debido."""
    if not shutil.which(args[0]) and not Path(args[0]).is_file():
        return ""
    try:
        r = subprocess.run(args, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout, env=env,
                           stdin=subprocess.DEVNULL, check=False)
    except (OSError, subprocess.TimeoutExpired):
        return ""
    return ((r.stdout or "") + "\n" + (r.stderr or "")).strip()


def _linea(args: list[str], timeout: int = 20, env: dict | None = None) -> str | None:
    texto = _texto(args, timeout, env)
    return texto.splitlines()[0] if texto else None


def leer_version_llama(texto: str) -> dict | None:
    """«version: 1.0.0-dev (build 1, commit 1094646)» de llama-server --version."""
    m = re.search(r"version:\s*(\S+)\s*\(build\s*(\d+),\s*commit\s*([0-9a-f]{7,40})\)", texto or "")
    return {"version": m.group(1), "build": m.group(2), "commit": m.group(3)} if m else None


def llama_local(lmstudio: Path = LMSTUDIO) -> dict | None:
    """El backend de LM Studio más nuevo con CUDA 12. `--version` con CUDA oculto: no carga modelo ni usa VRAM."""
    def clave(p: Path) -> tuple:
        return tuple(int(x) for x in re.findall(r"\d+", p.name.rsplit("-", 1)[-1]))
    backends = sorted((lmstudio / "extensions" / "backends").glob("llama.cpp-win-x86_64-nvidia-cuda12-avx2-*"), key=clave)
    if not backends:
        return None
    b = backends[-1]
    out = {"backend": b.name.rsplit("-", 1)[-1]}
    exe = b / "llama-server.exe"
    if exe.is_file():
        entorno = {**os.environ, "CUDA_VISIBLE_DEVICES": "-1"}
        out |= leer_version_llama(_texto([str(exe), "--version"], 30, entorno)) or {}
        if "commit" not in out:  # `--version` imprime la versión en una línea; si no se pudo leer, se dice
            out["aviso"] = "no se pudo leer la versión de llama-server"
    return out


def modelos_instalados(lmstudio: Path = LMSTUDIO) -> list[str]:
    raiz = lmstudio / "models"
    if not raiz.is_dir():
        return []
    return sorted(f"{d.parent.name}/{d.name}" for d in raiz.glob("*/*") if d.is_dir())


def estado_local(repo: Path = REPO, lmstudio: Path = LMSTUDIO) -> dict:
    paquete = json.loads((repo / "package.json").read_text(encoding="utf-8")) if (repo / "package.json").is_file() else {}
    comp = repo / "motor" / "agente" / "complementos.py"
    sat = re.search(r'"version":\s*"(v[\d.]+)"', comp.read_text(encoding="utf-8")) if comp.is_file() else None
    venv = repo / "motor" / ".venv" / "Scripts" / "python.exe"
    return {
        "aura": paquete.get("version"),
        "repo": _linea(["git", "-C", str(repo), "log", "-1", "--format=%h %s"]),
        "rama": _linea(["git", "-C", str(repo), "branch", "--show-current"]),
        "sat": sat.group(1) if sat else None,
        "llama": llama_local(lmstudio),
        "modelos": modelos_instalados(lmstudio),
        "herramientas": {n: _linea(c) for n, c in (("python del motor", [str(venv), "--version"]), ("node", ["node", "--version"]),
                                                    ("go", ["go", "version"]), ("cargo", ["cargo", "--version"]),
                                                    ("docker", ["docker", "--version"]), ("forge", ["forge", "--version"]))},
    }


# ------------------------------------------------------------------ revisión
def revisar(fuentes: list[dict], previo: dict, ahora: datetime, desde: datetime | None, local: dict, get=http_get) -> dict:
    """Una entrada por fuente: {"ok": bool, ...}. Una fuente que falla no tumba las demás."""
    resultados = {}
    for f in fuentes:
        if f["tipo"] == "web":
            continue
        ultima = previo.get("fuentes", {}).get(f["id"])
        ini = desde or (fecha(ultima) if ultima else ahora - timedelta(days=DIAS_PRIMERA))
        try:
            ultima = None
            if f["tipo"] == "huggingface":
                r = {"ok": True, "modelos": modelos_nuevos(f["autor"], ini, get)}
            elif f["tipo"] == "llama_cpp":  # publica una compilación por commit: solo la última release estable y la distancia al commit local
                ultima = get(f"https://api.github.com/repos/{f['repo']}/releases/latest")
                r = {"ok": True, "releases": [_release(ultima)] if fecha(ultima["published_at"]) > ini else [], "prereleases": 0, "por_commit": True}
            else:  # las prereleases (alphas, nightlies) solo se cuentan
                todas = releases_nuevas(f["repo"], ini, get)
                r = {"ok": True, "releases": [x for x in todas if not x["prerelease"]], "prereleases": sum(x["prerelease"] for x in todas)}
            if f["tipo"] == "llama_cpp" and (local.get("llama") or {}).get("commit"):
                try:
                    r["comparacion"] = comparar_llama(f["repo"], local["llama"]["commit"], get, ultima)
                except (OSError, ValueError, KeyError) as e:
                    r["comparacion"] = {"error": str(e)[:120]}
            r["desde"] = ini.date().isoformat()
        except (OSError, ValueError, KeyError, TypeError, AttributeError) as e:  # una respuesta inesperada también cuenta como «no respondió»
            r = {"ok": False, "motivo": f"{type(e).__name__}: {e}"[:160]}
        resultados[f["id"]] = r
    return resultados


# ------------------------------------------------------------------ informe
def _lista(items: list[str]) -> list[str]:
    visibles = [f"- {x}" for x in items[:MAX_LISTA]]
    return visibles + ([f"- … y {len(items) - MAX_LISTA} más"] if len(items) > MAX_LISTA else [])


def informe(local: dict, resultados: dict, fuentes: list[dict], ahora: datetime, con_red: bool = True) -> str:
    ll = local.get("llama") or {}
    filas = [("Aura", local.get("aura") or "?"), ("Repo", f"{local.get('rama') or '?'} · {local.get('repo') or '?'}"),
             ("Motor SAT fijado", local.get("sat") or "?"),
             ("llama.cpp (LM Studio)", (f"backend {ll.get('backend')} · {ll.get('version', '?')} build {ll.get('build', '?')} commit "
                                        f"{ll.get('commit', '?')}") if ll else "no encontrado"),
             ("Modelos instalados", ", ".join(local.get("modelos") or []) or "ninguno")]
    filas += [(n, v or "no instalado") for n, v in (local.get("herramientas") or {}).items()]
    out = [f"# Radar de Aura — {ahora:%Y-%m}", "",
           f"Generado {ahora:%Y-%m-%d %H:%M} UTC. Solo propone: no cambia código ni mediciones. Lleva el estado de este equipo: no subir al repo.",
           "", "## 1. Estado local", "", "| Qué | Valor |", "|---|---|"] + [f"| {k} | {v} |" for k, v in filas]
    out += ["", "## 2. Cambios desde la última revisión", ""]
    for f in fuentes:
        if f["tipo"] == "web":
            continue
        r = resultados.get(f["id"])
        titulo = f"{f['id']} — {f.get('repo') or 'Hugging Face: ' + f['autor']}"
        out += [f"### {titulo}" + (f" (desde {r['desde']})" if r and r.get("ok") else ""), f"Le importa a Aura: {f['importa']}", ""]
        if not con_red:
            out += ["- Sin red (`--sin-red`).", ""]
            continue
        if not r or not r["ok"]:
            out += [f"- **No respondió:** {(r or {}).get('motivo', 'sin motivo')}", ""]
            continue
        c = r.get("comparacion")
        if c and "error" in c:
            out.append(f"- Comparación con el commit local no disponible: {c['error']}")
        elif c:
            out.append(f"- Tu build está {c['nuevos_en_rama']} commits por detrás de `{c['rama']}`"
                       + (f" y {c['locales_sobre_release']} commits por encima de la release {c['release']}" if c.get("release") else "") + ".")
        nuevos = ([f"{m['id']} ({m['fecha']})" for m in r["modelos"]] if "modelos" in r else
                  [f"[{x['tag']}]({x['url']}) ({x['fecha']})" + (f" — {x['nombre']}" if x["nombre"] and x["nombre"] != x["tag"] else "")
                   for x in r["releases"]])
        out += _lista(nuevos) if nuevos else ["- Nada nuevo."]
        if nuevos:
            out.append(f"- Total: {len(nuevos)}.")
        if r.get("prereleases"):
            out.append(f"- No listadas: {r['prereleases']} prereleases o compilaciones intermedias.")
        if r.get("por_commit"):
            out.append("- (llama.cpp publica una compilación por commit: aquí solo cuenta la última release estable y la distancia al commit local.)")
        out.append("")
    out += ["## 3. Para revisar a mano (sin API estable; las lee quien revisa)", ""]
    out += [f"- **{f['id']}** — {f['url']}\n  - Buscar: {f['que_buscar']}\n  - Le importa a Aura: {f['importa']}" for f in fuentes if f["tipo"] == "web"]
    out += ["", "## 4. Qué le importa a Aura", "", "PENDIENTE: lo rellena quien revise (máximo 5 puntos, cada uno con su fuente; sin cifras de blogs).", "",
            "## 5. Qué probar", "",
            ("PENDIENTE: una prueba por punto, con la regla de decisión escrita antes. Un modelo o una versión de llama.cpp se mide con el banco "
            "(`python bench\\correr.py --motor … --modelo …`) y pasa por `--puerta` antes de adoptarse. Una cosa a la vez; la medición con GPU pide OK."), ""]
    return "\n".join(out)


def nombre_libre(salida: Path, ahora: datetime) -> Path:
    base = salida / f"{ahora:%Y-%m}.md"
    n = 2
    while base.exists():
        base = salida / f"{ahora:%Y-%m}_{n}.md"
        n += 1
    return base


def ejecutar(salida: Path, desde: str | None = None, sin_red: bool = False, seco: bool = False, get=http_get,
             ahora: datetime | None = None, fuentes: list[dict] | None = None, local: dict | None = None) -> Path:
    ahora = ahora or datetime.now(UTC)
    fuentes = fuentes or cargar_fuentes()
    local = local or estado_local()
    salida.mkdir(parents=True, exist_ok=True)
    ultimo = salida / "ultimo.json"
    previo = json.loads(ultimo.read_text(encoding="utf-8")) if ultimo.is_file() else {}
    limite = fecha(f"{desde}T00:00:00Z") if desde else None
    resultados = {} if sin_red else revisar(fuentes, previo, ahora, limite, local, get)
    informe_md = nombre_libre(salida, ahora)
    informe_md.write_text(informe(local, resultados, fuentes, ahora, not sin_red), encoding="utf-8")
    if not (sin_red or seco):
        vistas = {**previo.get("fuentes", {}), **{k: ahora.isoformat() for k, r in resultados.items() if r["ok"]}}
        ultimo.write_text(json.dumps({"fecha": ahora.isoformat(), "fuentes": vistas}, indent=1), encoding="utf-8")
    return informe_md


def main(argv: list[str] | None = None) -> None:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--salida", type=Path, default=REPO.parent / "aura-radar")
    ap.add_argument("--desde", help="AAAA-MM-DD")
    ap.add_argument("--sin-red", action="store_true")
    ap.add_argument("--seco", action="store_true")
    a = ap.parse_args(argv)
    if a.desde and not re.fullmatch(r"\d{4}-\d{2}-\d{2}", a.desde):
        sys.exit("--desde pide AAAA-MM-DD")
    ruta = ejecutar(a.salida.resolve(), a.desde, a.sin_red, a.seco)
    print(f"Informe: {ruta}")


if __name__ == "__main__":
    main()
