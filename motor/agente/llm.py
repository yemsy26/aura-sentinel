"""Cliente mínimo (solo stdlib) para el servidor OpenAI-compatible de LM Studio / Bionic."""
import json
import urllib.error
import urllib.request

SERVIDORES = ["http://127.0.0.1:1235", "http://127.0.0.1:1234"]  # Bionic, LM Studio


class ErrorLLM(Exception):
    pass


def http_json(url: str, cuerpo: dict | None = None, timeout: float = 10, metodo: str | None = None) -> dict:
    datos = json.dumps(cuerpo).encode() if cuerpo is not None else None
    req = urllib.request.Request(url, datos, {"Content-Type": "application/json"}, method=metodo)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.loads(r.read().decode("utf-8") or "{}")
    except urllib.error.HTTPError as e:
        raise ErrorLLM(f"HTTP {e.code}: {e.read().decode('utf-8', 'replace')[:500]}") from e
    except (urllib.error.URLError, TimeoutError, ConnectionError) as e:
        raise ErrorLLM(f"Sin conexión con {url}: {e}") from e


def listar_modelos(base: str) -> list[dict]:
    return http_json(f"{base}/api/v1/models", timeout=4).get("models", [])


def detectar(cfg: dict) -> tuple[str, str | None, int]:
    """Devuelve (url_servidor, modelo_cargado, contexto). modelo=None si no hay LLM cargado."""
    bases = SERVIDORES if cfg["servidor"] == "auto" else [cfg["servidor"].rstrip("/")]
    vivos = []
    for base in bases:
        try:
            modelos = listar_modelos(base)
        except ErrorLLM:
            continue
        vivos.append(base)
        for m in modelos:
            if m.get("type") != "llm" or not m.get("loaded_instances"):
                continue
            if cfg["modelo"] not in ("auto", m.get("key")):
                continue
            inst = m["loaded_instances"][0]
            return base, inst["id"], int(inst.get("config", {}).get("context_length") or 8192)
    if not vivos:
        raise ErrorLLM("No responde ningún servidor (Bionic :1235 / LM Studio :1234). Abre Bionic o LM Studio.")
    return vivos[0], None, 0


class LLM:
    def __init__(self, base: str, modelo: str, contexto: int, cfg: dict):
        self.base, self.modelo, self.contexto, self.cfg = base, modelo, contexto, cfg

    def chat(self, mensajes: list, herramientas: list | None, max_tokens: int, razonar: bool, al_progreso=None) -> dict:
        cuerpo = {
            "model": self.modelo,
            "messages": mensajes,
            "max_tokens": max_tokens,
            "stream": True,
            "stream_options": {"include_usage": True},
            **self.cfg["muestreo"],
        }
        if herramientas:
            cuerpo["tools"] = herramientas
        if not razonar:
            cuerpo["reasoning_effort"] = "none"
        req = urllib.request.Request(f"{self.base}/v1/chat/completions", json.dumps(cuerpo).encode(),
                                     {"Content-Type": "application/json"})
        texto, razon, llamadas, fin, uso = [], [], {}, None, {}
        try:
            with urllib.request.urlopen(req, timeout=600) as r:
                for linea in r:
                    linea = linea.decode("utf-8", "replace").strip()
                    if not linea.startswith("data:"):
                        continue
                    dato = linea[5:].strip()
                    if dato == "[DONE]":
                        break
                    ev = json.loads(dato)
                    if ev.get("error"):
                        raise ErrorLLM(str(ev["error"]))
                    uso = ev.get("usage") or uso
                    for ch in ev.get("choices", []):
                        d = ch.get("delta", {})
                        if d.get("reasoning_content"):
                            razon.append(d["reasoning_content"])
                        if d.get("content"):
                            texto.append(d["content"])
                        for tc in d.get("tool_calls") or []:
                            acc = llamadas.setdefault(tc.get("index", 0), {"id": "", "name": "", "arguments": ""})
                            acc["id"] = tc.get("id") or acc["id"]
                            f = tc.get("function") or {}
                            acc["name"] += f.get("name") or ""
                            acc["arguments"] += f.get("arguments") or ""
                        fin = ch.get("finish_reason") or fin
                    if al_progreso:
                        al_progreso(sum(map(len, razon)), sum(map(len, texto)) + sum(len(c["arguments"]) for c in llamadas.values()))
        except urllib.error.HTTPError as e:
            raise ErrorLLM(f"HTTP {e.code}: {e.read().decode('utf-8', 'replace')[:500]}") from e
        except (urllib.error.URLError, TimeoutError, ConnectionError) as e:
            raise ErrorLLM(f"Conexión perdida con el modelo: {e}") from e
        return {
            "content": "".join(texto),
            "reasoning": "".join(razon),
            "tool_calls": [llamadas[i] for i in sorted(llamadas)],
            "finish_reason": fin,
            "usage": uso,
        }
