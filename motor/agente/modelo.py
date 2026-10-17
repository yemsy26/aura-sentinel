"""Carga cualquier modelo con el MAYOR contexto que cabe en la VRAM libre, sin desbordar a RAM compartida.

  python -m agente.modelo cargar [--modelo CLAVE] [--contexto N]
  python -m agente.modelo estado
  python -m agente.modelo descargar

  python -m agente.modelo vision off|on   (aparta/restaura el proyector de visión mmproj: libera VRAM)

Mide la memoria real por proceso (contadores GPU de Windows + nvidia-smi): dos cargas de sondeo (8K y 16K) dan
el coste fijo y el coste por token; se guarda en .cache_modelos.json y las siguientes cargas son directas.
"""
import argparse
import json
import re
import shutil
import struct
import subprocess
import sys
import time
from pathlib import Path

from . import config
from .llm import ErrorLLM, detectar, http_json, listar_modelos

CACHE = config.RAIZ_INSTALACION / ".cache_modelos.json"
MODELOS = Path.home() / ".lmstudio" / "models"
APARTADOS = Path.home() / ".lmstudio" / "mmproj-desactivados"
DIRS_CONFIG = {  # configuración por modelo que guardan las apps
    "http://127.0.0.1:1235": Path.home() / ".lmstudio/apps/bionic/.internal/user-concrete-model-default-config",
    "http://127.0.0.1:1234": Path.home() / ".lmstudio/.internal/user-concrete-model-default-config",
}


def vram() -> tuple[int, int] | None:
    """(usada, total) en MiB de la primera GPU NVIDIA, o None."""
    if not shutil.which("nvidia-smi"):
        return None
    r = subprocess.run(["nvidia-smi", "--query-gpu=memory.used,memory.total", "--format=csv,noheader,nounits"],
                       capture_output=True, text=True)
    try:
        usada, total = map(int, r.stdout.strip().splitlines()[0].split(","))
        return usada, total
    except (ValueError, IndexError):
        return None


def vram_estable(max_seg: float = 25) -> tuple[int, int] | None:
    """Tras cargar/descargar, la VRAM tarda unos segundos en asentarse: espera a dos lecturas iguales."""
    previa, t0 = vram(), time.time()
    while previa and time.time() - t0 < max_seg:
        time.sleep(1.0)
        actual = vram()
        if abs(actual[0] - previa[0]) <= 24 and time.time() - t0 >= 2:
            return actual
        previa = actual
    return previa


def cargados(base: str) -> list[str]:
    return [i["id"] for m in listar_modelos(base) if m.get("type") == "llm" for i in m.get("loaded_instances", [])]


def descargar(base: str, instancia: str) -> None:
    http_json(f"{base}/api/v1/models/unload", {"instance_id": instancia}, timeout=60)


def descargar_todos(base: str) -> None:
    for inst in cargados(base):
        descargar(base, inst)
    vram_estable()


def cargar(base: str, clave: str, contexto: int, cfg: dict, moe: bool = False) -> dict:
    # 8 puntos de restauración de la caché (32 por defecto): cada uno guarda el estado recurrente en RAM
    cuerpo = {"model": clave, "context_length": contexto, "flash_attention": cfg["carga"]["flash_attention"],
              "offload_kv_cache_to_gpu": True, "parallel": 1, "context_checkpoints": 8, "echo_load_config": True}
    if moe:  # con los expertos en CPU, lotes grandes procesan el prompt mucho más rápido
        cuerpo |= {"eval_batch_size": 2048, "physical_batch_size": 2048}
    return http_json(f"{base}/api/v1/models/load", cuerpo, timeout=600)


def carpeta_modelo(modelo: dict) -> Path | None:
    """Carpeta del GGUF: la que suma size_bytes (con o sin su mmproj apartado)."""
    for carpeta in MODELOS.glob("*/*"):
        ggufs = list(carpeta.glob("*.gguf")) if carpeta.is_dir() else []
        if not ggufs:
            continue
        tam = sum(g.stat().st_size for g in ggufs)
        apartado = APARTADOS / carpeta.relative_to(MODELOS)
        extra = sum(g.stat().st_size for g in apartado.glob("*.gguf")) if apartado.is_dir() else 0
        if modelo.get("size_bytes") in (tam, tam + extra):
            return carpeta
    return None


def ggufs(modelo: dict) -> list[Path]:
    """Archivos de pesos del modelo (sin el proyector de visión), mayor primero."""
    carpeta = carpeta_modelo(modelo)
    archivos = [g for g in carpeta.glob("*.gguf") if "mmproj" not in g.name.lower()] if carpeta else []
    return sorted(archivos, key=lambda g: -g.stat().st_size)


def ruta_config(base: str, modelo: dict) -> Path | None:
    """Config de carga que la app guarda por modelo y vuelve a leer en cada carga."""
    raiz_cfg = DIRS_CONFIG.get(base)
    if not raiz_cfg:
        return None
    if "/" in modelo["key"]:  # modelos del hub
        return raiz_cfg / f"{modelo['key']}.json"
    archivos = ggufs(modelo)
    return raiz_cfg / (str(archivos[0].relative_to(MODELOS)) + ".json") if archivos else None


def guardar_config(destino: Path, campos: dict) -> bool:
    """Fija campos de carga en la config guardada del modelo. True si cambió algo."""
    datos = json.loads(destino.read_text(encoding="utf-8")) if destino.exists() else {"preset": "", "operation": {"fields": []}, "load": {"fields": []}}
    lista = datos.setdefault("load", {}).setdefault("fields", [])
    actuales = {c["key"]: c.get("value") for c in lista}
    nuevos = {k: v for k, v in campos.items() if k not in actuales or actuales[k] != v}
    if not nuevos:
        return False
    lista[:] = [c for c in lista if c["key"] not in nuevos] + [{"key": k, "value": v} for k, v in nuevos.items()]
    if destino.exists() and not destino.with_suffix(".json.bak").exists():  # conserva la copia original
        shutil.copy2(destino, destino.with_suffix(".json.bak"))
    destino.parent.mkdir(parents=True, exist_ok=True)
    destino.write_text(json.dumps(datos, indent=2, ensure_ascii=False), encoding="utf-8")
    return True


TAM_GGUF = {0: 1, 1: 1, 2: 2, 3: 2, 4: 4, 5: 4, 6: 4, 7: 1, 10: 8, 11: 8, 12: 8}  # bytes por tipo de valor
EXPERTOS = re.compile(r"blk\.(\d+)\.ffn_[a-z_]+_(?:ch)?exps\.")  # los tensores que --n-cpu-moe deja en la CPU


def leer_gguf(ruta: Path) -> tuple[int, dict[int, int]]:
    """(número de capas, {capa: bytes de sus tensores de expertos}) leyendo solo la cabecera del GGUF."""
    capas, alineado, tensores = 0, 32, []
    with open(ruta, "rb") as f:
        def lee(fmt: str):
            return struct.unpack("<" + fmt, f.read(struct.calcsize(fmt)))[0]

        def saltar(tipo: int) -> None:
            if tipo == 9:  # array
                sub, n = lee("I"), lee("Q")
                if sub in TAM_GGUF:
                    f.seek(TAM_GGUF[sub] * n, 1)
                else:
                    for _ in range(n):
                        saltar(sub)
            else:
                f.seek(lee("Q") if tipo == 8 else TAM_GGUF[tipo], 1)

        if f.read(4) != b"GGUF" or lee("I") < 2:
            return 0, {}
        n_tensores, n_kv = lee("Q"), lee("Q")
        for _ in range(n_kv):
            clave, tipo = f.read(lee("Q")).decode("utf-8", "replace"), lee("I")
            if clave.endswith(".block_count") and tipo == 4:
                capas = lee("I")
            elif clave == "general.alignment" and tipo == 4:
                alineado = lee("I")
            else:
                saltar(tipo)
        for _ in range(n_tensores):
            nombre = f.read(lee("Q")).decode("utf-8", "replace")
            f.seek(8 * lee("I") + 4, 1)  # dimensiones y tipo
            tensores.append((lee("Q"), nombre))
        datos = -(-f.tell() // alineado) * alineado
    tensores.sort()
    fines = [o for o, _ in tensores[1:]] + [ruta.stat().st_size - datos]
    por_capa: dict[int, int] = {}
    for (inicio, nombre), fin in zip(tensores, fines, strict=True):
        if (m := EXPERTOS.match(nombre)):
            por_capa[int(m[1])] = por_capa.get(int(m[1]), 0) + fin - inicio
    return capas, por_capa


def expertos(modelo: dict) -> list[float]:
    """MiB de expertos de cada capa (índice = capa); [] si no es un modelo MoE o no se localiza el GGUF."""
    capas, por_capa = 0, {}
    for g in ggufs(modelo):
        n, d = leer_gguf(g)
        capas = max(capas, n)
        for i, b in d.items():
            por_capa[i] = por_capa.get(i, 0) + b
    if not por_capa:
        return []
    return [por_capa.get(i, 0) / 2**20 for i in range(max(capas, max(por_capa) + 1))]


def fraccion_expertos(n_cpu: int, capas: int) -> float:
    """Fracción para llm.load.numCpuExpertLayersRatio: la app pasa ceil(fracción × capas) a --n-cpu-moe."""
    return round(max(n_cpu - 0.5, 0) / capas, 6)


def argumento_llama(nombre: str) -> str | None:
    """Valor de un argumento en la línea de comandos de llama-server: lo que la app aplicó de verdad."""
    ps = "(Get-CimInstance Win32_Process -Filter \"Name='llama-server.exe'\").CommandLine"
    try:
        r = subprocess.run(["powershell", "-NoProfile", "-Command", ps], capture_output=True, text=True, timeout=40)
    except (OSError, subprocess.SubprocessError):
        return None
    m = re.search(rf"{re.escape(nombre)}\s+(\S+)", r.stdout)
    return m[1] if m else None


def vision(cfg: dict, pedido: str | None, activar: bool) -> int:
    """El mmproj (visión) se carga en VRAM aunque no envíes imágenes. Apartarlo libera ~0,5-1 GB."""
    base, _, _ = detectar(dict(cfg, modelo="auto"))
    modelo = elegir(base, cfg, pedido)
    carpeta = carpeta_modelo(modelo)
    if carpeta is None:
        print("[ERROR] No localicé la carpeta del modelo.")
        return 2
    apartado = APARTADOS / carpeta.relative_to(MODELOS)
    origen, destino = (apartado, carpeta) if activar else (carpeta, apartado)
    archivos = [g for g in origen.glob("*.gguf") if "mmproj" in g.name.lower()] if origen.is_dir() else []
    if not archivos:
        print(f"Visión ya {'activada' if activar else 'desactivada'} para {modelo['key']}.")
        return 0
    if cargados(base):
        descargar_todos(base)  # el archivo no se puede mover mientras está cargado
    destino.mkdir(parents=True, exist_ok=True)
    for g in archivos:
        shutil.move(str(g), str(destino / g.name))
    CACHE.unlink(missing_ok=True)  # el consumo cambia: volver a medir
    print(f"[OK] Visión {'ACTIVADA' if activar else 'DESACTIVADA'} para {modelo['key']} ({', '.join(g.name for g in archivos)}).\n"
          f"     Si Bionic/LM Studio no lo refleja, reinícialo. Después ejecuta cargar_modelo.bat.")
    return 0


def elegir(base: str, cfg: dict, pedido: str | None) -> dict:
    llms = [m for m in listar_modelos(base) if m.get("type") == "llm"]
    if not llms:
        raise ErrorLLM("No hay modelos LLM descargados.")
    clave = pedido or (cfg["modelo"] if cfg["modelo"] != "auto" else None)
    if clave:
        for m in llms:
            if clave in (m["key"], m.get("display_name")):
                return m
        raise ErrorLLM(f"Modelo '{clave}' no encontrado. Disponibles: {', '.join(m['key'] for m in llms)}")
    activos = [m for m in llms if m.get("loaded_instances")]
    if activos:
        return activos[0]
    if len(llms) == 1 or not sys.stdin.isatty():
        return llms[0]
    for i, m in enumerate(llms, 1):
        print(f"  {i}. {m.get('display_name', m['key'])}  ({m['size_bytes'] / 2**30:.1f} GB, {m.get('quantization', {}).get('name', '')})")
    n = input("Elige modelo (número): ").strip()
    return llms[int(n) - 1] if n.isdigit() and 0 < int(n) <= len(llms) else llms[0]


RESERVA_WDDM = 300  # Windows no deja usar los últimos ~300 MiB: a partir de ahí desborda a RAM compartida (lento)


def memoria_procesos() -> dict[int, list] | None:
    """{pid: [MiB dedicados, MiB compartidos]} según los contadores de GPU de Windows (None si no hay)."""
    ps = ("foreach($c in 'Dedicated','Shared'){(Get-Counter \"\\GPU Process Memory(*)\\$c Usage\" -EA Stop)."
          "CounterSamples | % { \"$c|$($_.InstanceName)|$([int64]$_.CookedValue)\" }}")
    try:
        r = subprocess.run(["powershell", "-NoProfile", "-Command", ps], capture_output=True, text=True, timeout=40)
    except (OSError, subprocess.SubprocessError):
        return None
    datos: dict[int, list] = {}
    for linea in r.stdout.splitlines():
        partes = linea.strip().split("|")
        if len(partes) == 3 and partes[1].startswith("pid_"):
            pid = int(partes[1].split("_")[1])
            datos.setdefault(pid, [0, 0])[0 if partes[0] == "Dedicated" else 1] += int(partes[2]) // 2**20
    return datos or None


def nombre_proceso(pid: int) -> str:
    r = subprocess.run(["tasklist", "/FI", f"PID eq {pid}", "/FO", "CSV", "/NH"], capture_output=True, text=True)
    return r.stdout.split(",")[0].strip('"') if r.stdout.startswith('"') else str(pid)


COMPARTIDA_NORMAL = 160  # llama.cpp siempre usa ~120 MiB de memoria compartida (búferes del host): no es desborde


def huella(antes: dict | None, usada0: int) -> int:
    """MiB de VRAM que necesita lo recién cargado (incluye lo que se haya desbordado a memoria compartida)."""
    usada = vram_estable()[0]
    despues = memoria_procesos() if antes is not None else None
    if despues is None:
        return usada - usada0
    dedicada = compartida = 0
    for pid, (d, s) in despues.items():
        d0, s0 = antes.get(pid, (0, 0))
        if (d - d0) + (s - s0) > 50:
            dedicada += d - d0
            compartida += s - s0
    return dedicada + max(0, compartida - COMPARTIDA_NORMAL)


def en_el_techo(total: int) -> bool:
    """WDDM desborda a RAM cuando la VRAM dedicada llega a ~total-300 MiB."""
    return total - vram_estable()[0] < RESERVA_WDDM + 40


def ajustar(cfg: dict, pedido: str | None, contexto: int | None) -> int:
    base, _, _ = detectar(dict(cfg, modelo="auto"))
    modelo = elegir(base, cfg, pedido)
    clave, maximo = modelo["key"], int(modelo.get("max_context_length") or 32768)
    c = cfg["carga"]
    print(f"Servidor: {base}\nModelo:   {modelo.get('display_name', clave)}")
    destino = ruta_config(base, modelo)
    exp = expertos(modelo) if destino else []  # MoE: los expertos que no caben en VRAM van a la CPU (--n-cpu-moe)
    campos = {"llm.load.llama.keepModelInMemory": False}  # sin mlock: no bloquea en RAM una copia entera del modelo
    if c["kv_cache"] not in ("", "f16"):  # q8_0 = mitad de memoria que f16
        campos |= {f"llm.load.llama.{x}CacheQuantizationType": {"checked": True, "value": c["kv_cache"]} for x in "kv"}
    if exp:  # todas las capas en GPU salvo los expertos que se indiquen
        campos |= {"llm.load.llama.autoFit": False, "llm.load.llama.acceleration.offloadRatio": 1}
        print(f"  MoE: {len(exp)} capas, {sum(exp) / 1024:.1f} GiB de expertos (los que no quepan en VRAM irán a la CPU)")
    if destino is None:
        print("  (no localicé la config del modelo: caché KV, mlock y expertos quedan como los tenga la app)")
    elif guardar_config(destino, campos):
        print(f"  config de carga actualizada en {destino.name} (caché KV {c['kv_cache']}, sin mlock)")

    def poner(ctx: int, n_cpu: int) -> None:
        if exp:
            guardar_config(destino, {"llm.load.numCpuExpertLayersRatio": fraccion_expertos(n_cpu, len(exp))})
        cargar(base, clave, ctx, cfg, moe=bool(exp))

    if cargados(base):  # libera la VRAM de otros LLM
        print(f"  descargando {', '.join(cargados(base))} ...")
        descargar_todos(base)
    medida = vram_estable()
    if medida is None or (contexto and not exp):
        ctx = contexto or 16384
        poner(ctx, len(exp))
        print(f"[OK] Cargado con contexto {ctx}" + ("" if medida else " (sin GPU NVIDIA detectable: no se midió VRAM)"))
        return 0
    usada0, total = medida
    antes = memoria_procesos()
    disponible = total - RESERVA_WDDM - usada0 - c["margen_vram_mib"]
    print(f"VRAM: {total} MiB; otras apps usan {usada0} MiB -> disponibles para el modelo {disponible} MiB")
    if antes:
        for pid, (d, _) in sorted(antes.items(), key=lambda x: -x[1][0])[:3]:
            if d > 300:
                print(f"   - {nombre_proceso(pid)} (pid {pid}): {d} MiB")

    cache = json.loads(CACHE.read_text(encoding="utf-8")) if CACHE.exists() else {}
    id_cache = f"{clave}|{c['kv_cache']}|{c['flash_attention']}" + ("|moe" if exp else "")
    ultimo = None
    if id_cache not in cache:
        print("Midiendo consumo real del modelo (2 cargas de prueba" + (" con todos los expertos en CPU" if exp else "")
              + ", solo la primera vez) ...")
        poner(8192, len(exp))
        f8 = huella(antes, usada0)
        descargar_todos(base)
        poner(16384, len(exp))
        f16 = huella(antes, usada0)
        ultimo = (16384, len(exp))
        por_k = max((f16 - f8) / 8, 1.0)
        cache[id_cache] = {"fijo_mib": round(f8 - 8 * por_k), "mib_por_1k": round(por_k, 2)}
        CACHE.write_text(json.dumps(cache, indent=2), encoding="utf-8")
    fijo, por_k = cache[id_cache]["fijo_mib"], cache[id_cache]["mib_por_1k"]
    ctx = contexto or min(int((disponible - fijo) / por_k * 1024) // 4096 * 4096, c["contexto_max"], maximo)
    print(f"Modelo: {fijo} MiB fijos + {por_k} MiB por cada 1K tokens de contexto -> contexto que cabe: {max(ctx, 0)}")
    if ctx < c["contexto_min"]:
        falta = fijo + c["contexto_min"] / 1024 * por_k - disponible
        print(f"[AVISO] Faltan ~{int(falta)} MiB de VRAM para {c['contexto_min']} tokens. Se desbordará a RAM (más lento).\n"
              f"        Libera VRAM: desactiva la voz (Voxtral) en Bionic, cierra apps con GPU, quita la visión\n"
              f"        (vision.bat off) o usa una cuantización menor. Cargo {c['contexto_min']} igualmente.")
        ctx = c["contexto_min"]
    # MoE: con el contexto ya fijado, los expertos de las últimas capas ocupan la VRAM que sobra
    libre = disponible - fijo - ctx / 1024 * por_k
    n_cpu = next((n for n in range(len(exp) + 1) if sum(exp[n:]) <= libre), len(exp))
    techo = False
    for _ in range(8):
        if ultimo != (ctx, n_cpu):
            if ultimo:
                descargar_todos(base)
            poner(ctx, n_cpu)
            ultimo = (ctx, n_cpu)
        techo = en_el_techo(total)
        if not techo:
            break
        if n_cpu < len(exp):
            print("  VRAM en el límite (desbordaría a RAM); paso a la CPU los expertos de una capa más ...")
            n_cpu += 1
        elif ctx > c["contexto_min"]:
            print("  VRAM en el límite (desbordaría a RAM); reduzco contexto ...")
            ctx -= 4096
        else:
            break
    cache["ultimo"] = {"modelo": clave, "contexto": ctx}  # lo usa instalar --configurar-modelo
    CACHE.write_text(json.dumps(cache, indent=2), encoding="utf-8")
    usada, _ = vram()
    estado_txt = "todo en VRAM" if not techo else "VRAM llena: parte desbordada a RAM (más lento)"
    if exp:
        estado_txt = f"expertos de {n_cpu}/{len(exp)} capas en CPU, resto en VRAM" if not techo else estado_txt
        real = argumento_llama("--n-cpu-moe")
        if real not in (None, str(n_cpu)):
            print(f"[AVISO] La app aplicó --n-cpu-moe {real} en vez de {n_cpu}: la VRAM medida puede no cuadrar.")
    print(f"[OK] {clave} cargado: contexto {ctx} tokens, VRAM {usada}/{total} MiB, {estado_txt}")
    return 0


def estado(cfg: dict) -> int:
    base, modelo, ctx = detectar(cfg)
    v = vram()
    print(f"Servidor: {base}\nModelo cargado: {modelo or '(ninguno)'}" + (f"  contexto={ctx}" if modelo else ""))
    if v:
        print(f"VRAM: {v[0]}/{v[1]} MiB ({v[1] - v[0]} libres)")
    return 0 if modelo else 1


def main() -> int:
    for flujo in (sys.stdout, sys.stderr):
        flujo.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(prog="agente.modelo")
    ap.add_argument("accion", choices=["cargar", "estado", "descargar", "asegurar", "vision"], nargs="?", default="estado")
    ap.add_argument("valor", nargs="?", choices=["on", "off"], help="para 'vision'")
    ap.add_argument("--modelo")
    ap.add_argument("--contexto", type=int)
    ap.add_argument("--remedir", action="store_true", help="borra la medición guardada y vuelve a medir")
    a = ap.parse_args()
    cfg = config.cargar()
    try:
        if a.remedir and CACHE.exists():
            CACHE.unlink()
        if a.accion == "estado":
            return estado(cfg)
        if a.accion == "vision":
            return vision(cfg, a.modelo, a.valor != "off")
        if a.accion == "descargar":
            base, _, _ = detectar(cfg)
            for inst in cargados(base):
                descargar(base, inst)
                print(f"Descargado {inst}")
            return 0
        if a.accion == "asegurar":  # carga solo si falta el modelo de agente.json (con "auto", si no hay ninguno)
            _, modelo, _ = detectar(cfg)
            if modelo:
                return 0
        return ajustar(cfg, a.modelo, a.contexto)
    except ErrorLLM as e:
        print(f"[ERROR] {e}")
        return 2


if __name__ == "__main__":
    sys.exit(main())
