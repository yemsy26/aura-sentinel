"""Registra el servidor MCP 'agente-local' en Bionic y LM Studio y genera el prompt de sistema para usarlo.

  python -m agente.instalar [--raiz CARPETA] [--desactivar-bionic-core] [--configurar-modelo auto|RUTA_JSON]
"""
import argparse
import json
import shutil
import sys
import uuid
from pathlib import Path

from . import config
from .prompts import METODO

INST = config.RAIZ_INSTALACION
PY = INST / ".venv" / "Scripts" / "python.exe"
LMS = Path.home() / ".lmstudio"
MCP_LMSTUDIO = LMS / "mcp.json"
MCP_BIONIC = LMS / "apps" / "bionic" / ".internal" / "ng-mcp.json"
CFG_BIONIC = LMS / "apps" / "bionic" / ".internal" / "user-concrete-model-default-config"

PROMPT_MCP = ("Eres un ingeniero de software senior que trabaja en Windows con las herramientas MCP de agente-local.\n"
              "ANTES DE NADA: llama a proyecto(ruta) con la carpeta que indique el usuario (o proyecto() para ver la actual).\n\n"
              + METODO.replace("y llama a terminar", "y resume al usuario qué hiciste y cómo usarlo"))


def respaldar(p: Path) -> None:
    """Guarda la copia ORIGINAL una sola vez: reinstalar nunca la pisa con una versión ya modificada."""
    copia = p.with_name(p.name + ".bak")
    if p.exists() and not copia.exists():
        shutil.copy2(p, copia)


def escribir_json(p: Path, datos) -> None:
    respaldar(p)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(datos, indent=2, ensure_ascii=False), encoding="utf-8")


def leer_json(p: Path, defecto):
    if not p.exists():
        return defecto
    return json.loads(p.read_text(encoding="utf-8"))  # si es inválido, falla sin tocar nada


def registrar(raiz: str, desactivar_core: bool) -> None:
    servidor = {"command": str(PY), "args": [str(INST / "agente_mcp.py")],
                "env": {"PYTHONUNBUFFERED": "1", "PYTHONUTF8": "1", "AGENTE_RAIZ": raiz}}
    datos = leer_json(MCP_LMSTUDIO, {"mcpServers": {}})
    datos.setdefault("mcpServers", {})["agente-local"] = servidor
    escribir_json(MCP_LMSTUDIO, datos)
    print(f"[OK] MCP registrado en LM Studio: {MCP_LMSTUDIO}")

    if MCP_BIONIC.parent.is_dir():
        datos = leer_json(MCP_BIONIC, {"servers": []})
        previo = next((s for s in datos["servers"] if s.get("name") == "agente-local"), None)
        datos["servers"] = [s for s in datos["servers"] if s.get("name") != "agente-local"]
        datos["servers"].append({"id": previo["id"] if previo else str(uuid.uuid4()), "name": "agente-local",
                                 "enabled": True, "connection": {"type": "stdio", **servidor}})
        if desactivar_core:
            for s in datos["servers"]:
                if s.get("name") == "bionic-core" and s.get("enabled"):
                    s["enabled"] = False
                    print("[OK] bionic-core desactivado en Bionic (herramientas duplicadas gastan tokens). "
                          "Reactívalo en Bionic si lo necesitas.")
        escribir_json(MCP_BIONIC, datos)
        print(f"[OK] MCP registrado en Bionic: {MCP_BIONIC}")


def campo(campos: list, clave: str, valor) -> None:
    campos[:] = [c for c in campos if c["key"] != clave] + [{"key": clave, "value": valor}]


def configurar_modelo(ruta_rel: str) -> None:
    """Ajustes de inferencia recomendados para usar el modelo como agente dentro de Bionic."""
    destino = CFG_BIONIC / ruta_rel
    datos = leer_json(destino, {"preset": "", "operation": {"fields": []}, "load": {"fields": []}})
    op = datos.setdefault("operation", {}).setdefault("fields", [])
    ld = datos.setdefault("load", {}).setdefault("fields", [])
    campo(op, "llm.prediction.systemPrompt", PROMPT_MCP)
    campo(op, "llm.prediction.temperature", 0.6)
    campo(op, "llm.prediction.topKSampling", 20)
    campo(op, "llm.prediction.topPSampling", {"checked": True, "value": 0.9})
    campo(op, "llm.prediction.minPSampling", {"checked": True, "value": 0})
    campo(op, "llm.prediction.repeatPenalty", {"checked": True, "value": 1.05})
    campo(op, "llm.prediction.llama.presencePenalty", {"checked": False, "value": 0})
    campo(op, "llm.prediction.reasoning.budgetTokens", {"checked": True, "value": 2048})
    campo(op, "llm.prediction.contextOverflowPolicy", "truncateMiddle")
    cache = INST / ".cache_modelos.json"
    ultimo = json.loads(cache.read_text(encoding="utf-8")).get("ultimo") if cache.exists() else None
    if ultimo:  # contexto medido por cargar_modelo.bat: el que cabe en VRAM
        campo(ld, "llm.load.contextLength", ultimo["contexto"])
    campo(ld, "llm.load.numParallelSessions", 1)
    campo(ld, "llm.load.offloadKVCacheToGpu", True)
    campo(ld, "llm.load.llama.acceleration.offloadRatio", 1)
    campo(ld, "llm.load.llama.kCacheQuantizationType", {"checked": True, "value": "q8_0"})
    campo(ld, "llm.load.llama.vCacheQuantizationType", {"checked": True, "value": "q8_0"})
    escribir_json(destino, datos)
    print(f"[OK] Modelo configurado en Bionic: {destino} (copia en .bak). Reinicia Bionic para aplicarlo.")


def main() -> int:
    for flujo in (sys.stdout, sys.stderr):
        flujo.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(prog="agente.instalar")
    ap.add_argument("--raiz", default=str(Path.home() / "Proyectos"), help="carpeta inicial del MCP")
    ap.add_argument("--desactivar-bionic-core", action="store_true")
    ap.add_argument("--configurar-modelo", metavar="RUTA_JSON",
                    help="'auto' = el recomendado (o el fijado en agente.json); o la ruta, p. ej. "
                         "unsloth/Qwen3.6-35B-A3B-GGUF/Qwen3.6-35B-A3B-UD-Q3_K_XL.gguf.json")
    a = ap.parse_args()
    if not PY.exists():
        print(f"[ERROR] Falta {PY}. Ejecuta instalar.bat.")
        return 2
    Path(a.raiz).mkdir(parents=True, exist_ok=True)
    registrar(str(Path(a.raiz).resolve()), a.desactivar_bionic_core)
    (INST / "PROMPT_SISTEMA_MCP.txt").write_text(PROMPT_MCP, encoding="utf-8")
    print(f"[OK] Prompt de sistema para Bionic/LM Studio: {INST / 'PROMPT_SISTEMA_MCP.txt'}")
    if a.configurar_modelo == "auto":  # setup.ps1: el modelo de agente.json, si es uno de los que sé descargar
        from .descargar import MODELOS
        m = MODELOS.get(config.recomendado(config.cargar()))
        if not m:
            print("[AVISO] El modelo de agente.json no es uno conocido: configúralo a mano en Bionic.")
            return 0
        a.configurar_modelo = f"{m['repo']}/{m['gguf']}.json"
    if a.configurar_modelo:
        configurar_modelo(a.configurar_modelo)
    return 0


if __name__ == "__main__":
    sys.exit(main())
