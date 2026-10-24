"""Qué se puede esperar de un modelo para el agente, según su ficha en Bionic/LM Studio (/api/v1/models).

El agente funciona con cualquier modelo cargado; esto solo avisa de lo que lo limita, para que el usuario sepa si un
fallo es del modelo (no usa herramientas, está muy comprimido, razona siempre, poco contexto) y no del agente."""
import re

from .llm import opciones_razonar


def bits(cuantizacion: str) -> float | None:
    """Bits por peso aproximados de la cuantización: Q1_0 -> 1, IQ2_XXS -> 2, Q3_K_XL -> 3, MXFP4 -> 4, F16 -> 16."""
    c = (cuantizacion or "").upper()
    if m := re.search(r"(?:^|[^A-Z])I?Q(\d)(?:_|$)", c) or re.match(r"Q(\d)", c):
        return float(m.group(1))
    if m := re.search(r"(?:B?F|FP)(\d{1,2})$|MXFP(\d)", c):
        return float(m.group(1) or m.group(2))
    return None


def avisos(info: dict, contexto: int, minimo: int) -> list[str]:
    caps = info.get("capabilities") or {}
    nombre = info.get("display_name") or info.get("key") or "El modelo"
    out = []
    if caps.get("trained_for_tool_use") is False:
        out.append(f"{nombre} no está entrenado para usar herramientas: el agente funcionará mal o no hará nada. "
                   "Elige uno con «tool use».")
    if opciones_razonar(info) == ["on"]:
        out.append(f"{nombre} razona siempre (no se puede apagar): cada paso tarda más que con otros modelos.")
    cuant = (info.get("quantization") or {}).get("name", "") if isinstance(info.get("quantization"), dict) else ""
    if (b := bits(cuant)) is not None and b <= 2:
        out.append(f"{nombre} está comprimido a ~{b:g} bits por peso ({cuant}): pierde mucha calidad y fallará más en "
                   "tareas largas. Con la misma memoria rinde más un modelo más pequeño menos comprimido.")
    if contexto and contexto < minimo:
        out.append(f"{nombre} trabaja con {contexto} tokens de contexto y el agente necesita al menos {minimo}: se "
                   "quedará sin sitio enseguida.")
    return out


def resumen(info: dict) -> dict:
    """Lo que la interfaz enseña del modelo activo."""
    caps = info.get("capabilities") or {}
    return {"herramientas": caps.get("trained_for_tool_use"), "vision": caps.get("vision"),
            "razonamiento": opciones_razonar(info), "contexto_max": info.get("max_context_length")}
