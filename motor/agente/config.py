"""Configuración: valores por defecto + agente.json (junto a la instalación)."""
import json
from pathlib import Path

RAIZ_INSTALACION = Path(__file__).resolve().parent.parent
ARCHIVO_CONFIG = RAIZ_INSTALACION / "agente.json"
# El que se descarga e instala por defecto (descargar_modelo.bat) y el que se elige si no hay ninguno cargado ni pedido.
# No es un candado: con "modelo": "auto" el agente usa el que esté cargado o el que se elija en Aura.
RECOMENDADO = "qwen3.6-35b-a3b"

DEFECTO = {
    # "auto" prueba Bionic (1235) y LM Studio (1234); o una URL fija: "http://127.0.0.1:1234"
    "servidor": "auto",
    # "auto" usa el LLM ya cargado (o el elegido en Aura); una clave fija obliga a usar ese modelo
    "modelo": "auto",
    # auto: razona al planificar y tras errores; off: nunca; on: siempre
    "razonamiento": "auto",
    "max_pasos": 100,
    # 0 = automático (proporcional al contexto cargado)
    "max_chars_resultado": 0,
    "timeout_comando": 120,
    "confirmar_peligrosos": True,
    "muestreo": {
        "temperature": 0.6,
        "top_p": 0.9,
        "top_k": 20,
        "min_p": 0.0,
        "presence_penalty": 0.0,
        "repeat_penalty": 1.05,
    },
    # Carga del modelo (cargar_modelo.bat)
    "carga": {
        "kv_cache": "q8_0",
        "flash_attention": True,
        # el agente no cabe en menos: sus instrucciones y herramientas ocupan ~5.000 tokens antes de empezar
        "contexto_min": 16384,
        "contexto_max": 65536,
        # VRAM que se deja libre para Windows y otras apps (MiB)
        "margen_vram_mib": 450,
    },
}


def _fusionar(base: dict, extra: dict) -> dict:
    out = dict(base)
    for k, v in extra.items():
        out[k] = _fusionar(out[k], v) if isinstance(v, dict) and isinstance(out.get(k), dict) else v
    return out


def recomendado(cfg: dict) -> str:
    """El modelo a descargar o instalar: el fijado en agente.json o, en auto, el recomendado."""
    return cfg.get("modelo") if cfg.get("modelo") not in (None, "", "auto") else RECOMENDADO


def cargar() -> dict:
    if ARCHIVO_CONFIG.exists():
        try:
            return _fusionar(DEFECTO, json.loads(ARCHIVO_CONFIG.read_text(encoding="utf-8")))
        except (OSError, ValueError) as e:
            print(f"[aviso] agente.json inválido ({e}); uso valores por defecto.")
    return json.loads(json.dumps(DEFECTO))
