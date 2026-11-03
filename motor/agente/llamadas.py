"""Lo que el modelo pide: sacar las llamadas a herramientas de su texto y corregir los tipos de sus argumentos."""
import ast
import json
import re
import uuid

from .esquemas import ESQUEMA_MAPA, ESQUEMA_PLAN_PARTES, ESQUEMAS

TIPOS = {e["function"]["name"]: {k: v.get("type") for k, v in e["function"]["parameters"]["properties"].items()}
         for e in [*ESQUEMAS, ESQUEMA_MAPA, ESQUEMA_PLAN_PARTES]}


def coaccionar(nombre: str, args: dict) -> dict:
    """Los modelos pequeños mandan '3' en vez de 3 o 'true' en vez de true."""
    for k, tipo in TIPOS.get(nombre, {}).items():
        v = args.get(k)
        if isinstance(v, str):
            if tipo == "integer" and re.fullmatch(r"-?\d+", v.strip()):
                args[k] = int(v)
            elif tipo == "boolean":
                args[k] = v.strip().lower() in ("true", "1", "si", "sí", "yes")
            elif tipo == "array":
                try:
                    args[k] = json.loads(v)
                except ValueError:
                    args[k] = [l for l in v.splitlines() if l.strip()]
    return {k: v for k, v in args.items() if k in TIPOS.get(nombre, args)}


def _llamada(nombre, args) -> dict | None:
    """Llamada válida solo si la herramienta existe (en texto libre, un JSON cualquiera no debe ejecutarse)."""
    if isinstance(args, str):
        try:
            args = json.loads(args) if args.strip() else {}
        except ValueError:
            return None
    if nombre not in TIPOS or not isinstance(args, dict):
        return None
    return {"name": nombre, "arguments": json.dumps(args, ensure_ascii=False)}


def _de_json(obj) -> list:
    """{"name", "arguments"|"parameters"|"args"|"input"}, {"function": {...}} o una lista de ellos."""
    if isinstance(obj, list):
        return [c for o in obj for c in _de_json(o)]
    if not isinstance(obj, dict):
        return []
    f = obj["function"] if isinstance(obj.get("function"), dict) else obj
    args = next((f[k] for k in ("arguments", "parameters", "args", "input") if k in f), {})
    c = _llamada(f.get("name") or obj.get("tool"), args)
    return [c] if c else []


def _jsons(texto: str) -> list:
    """Los objetos o listas JSON seguidos que empiezan el texto (separados por espacios, comas o ';')."""
    dec, out, i = json.JSONDecoder(), [], 0
    while (m := re.compile(r"[\s,;]*([\[{])").match(texto, i)):
        try:
            obj, i = dec.raw_decode(texto, m.start(1))
        except ValueError:
            break
        out += _de_json(obj)
    return out


def _de_python(codigo: str) -> list:
    """```tool_code de Gemma: escribir(ruta="a.py", contenido="...") o print(default_api.escribir(...)), solo eso."""
    try:
        sentencias = ast.parse(codigo).body
    except SyntaxError:
        return []
    out = []
    for s in sentencias:
        llamada = s.value if isinstance(s, ast.Expr) else None
        if isinstance(llamada, ast.Call) and getattr(llamada.func, "id", "") == "print" and llamada.args:
            llamada = llamada.args[0]
        if not isinstance(llamada, ast.Call):
            return []
        nombre = getattr(llamada.func, "attr", None) or getattr(llamada.func, "id", "")
        try:
            args = {k.arg: ast.literal_eval(k.value) for k in llamada.keywords}
        except ValueError:
            return []
        if not (c := _llamada(nombre, args)):
            return []
        out.append(c)
    return out


def llamadas_en_texto(texto: str) -> list:
    """Respaldo para modelos/servidores que escriben la llamada como texto en vez de usar tool_calls: Qwen
    (<tool_call>, <function=...>), Mistral ([TOOL_CALLS]), Llama (<|python_tag|>), Gemma (```tool_code) y JSON suelto
    ({"name": ..., "arguments": ...}). Antes solo se entendía el de Qwen: con otro modelo el agente no hacía nada."""
    out = []
    for bloque in re.findall(r"<tool_call>(.*?)(?:</tool_call>|$)", texto, re.DOTALL):
        if "<function=" not in bloque:
            out += _jsons(bloque.strip())
    for nombre, cuerpo in re.findall(r"<function=([\w.-]+)>(.*?)(?:</function>|$)", texto, re.DOTALL):
        args = {p: v.strip("\n") for p, v in re.findall(r"<parameter=([\w-]+)>\n?(.*?)\n?</parameter>", cuerpo, re.DOTALL)}
        out += [c for c in [_llamada(nombre, args if args or "{" not in cuerpo else cuerpo.strip())] if c]
    if not out and "[TOOL_CALLS]" in texto:
        resto = texto.split("[TOOL_CALLS]", 1)[1]
        pares = re.findall(r"([\w.-]+)\[ARGS\](\{.*?\})(?=\s*(?:[\w.-]+\[ARGS\]|$))", resto, re.DOTALL)
        out = [c for n, a in pares if (c := _llamada(n, a))] or _jsons(resto.strip())
    if not out and "<|python_tag|>" in texto:
        out = _jsons(texto.split("<|python_tag|>", 1)[1].strip())
    for lenguaje, bloque in ([] if out else re.findall(r"```(\w*)[ \t]*\n(.*?)```", texto, re.DOTALL)):
        out += _de_python(bloque) if lenguaje.lower() in ("tool_code", "python", "py") else _jsons(bloque.strip())
    if not out and texto.strip()[:1] in ("{", "["):
        out = _jsons(texto.strip())
    for c in out:
        c["id"] = uuid.uuid4().hex[:24]
    return out


def sin_llamadas(texto: str) -> str:
    """El texto del modelo sin las llamadas que se sacaron de él."""
    texto = re.sub(r"<tool_call>.*?(</tool_call>|$)|<function=[\w.-]+>.*?(</function>|$)", "", texto, flags=re.DOTALL)
    texto = re.sub(r"(\[TOOL_CALLS\]|<\|python_tag\|>).*", "", texto, flags=re.DOTALL)
    texto = re.sub(r"```(?:json|tool_call|tool_code)?[ \t]*\n.*?```", "", texto, flags=re.DOTALL)
    return "" if texto.strip()[:1] in ("{", "[") and llamadas_en_texto(texto) else texto.strip()
