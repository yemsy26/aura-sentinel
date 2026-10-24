"""Presupuesto de contexto: el historial nunca supera la ventana cargada.

Estrategia (de barata a cara):
1. Los resultados de herramientas ya llegan recortados (max_chars).
2. Al pasar el 60 % del contexto: se acortan resultados y archivos escritos antiguos (sin llamar al modelo).
3. Si sigue por encima del 45 %: el modelo resume lo antiguo y se reinicia con TAREA + RESUMEN + PLAN + últimos mensajes.
El historial es de solo-añadir entre compactaciones para aprovechar la caché de prompt del servidor.
"""
import json

from .prompts import RESUMIR

MAX_SALIDA = 16384  # tokens por respuesta (razonamiento incluido): un archivo grande en una sola llamada cabe


def json_valido(argumentos: str | None) -> str:
    """Los argumentos tal cual si son JSON; si no (llamada cortada por longitud), "{}". El servidor del modelo
    responde HTTP 500 a todo historial que lleve una llamada con JSON roto, y la conversación quedaría inservible."""
    try:
        json.loads(argumentos or "{}", strict=False)
    except ValueError:
        return "{}"
    return argumentos or "{}"


COLA = 6  # mensajes recientes que nunca se tocan


def _largo(m: dict) -> int:
    n = len(m.get("content") or "")
    for tc in m.get("tool_calls") or []:
        n += len(tc["function"]["name"]) + len(tc["function"]["arguments"]) + 20
    return n + 12


class Contexto:
    def __init__(self, ventana: int, sistema: str, tokens_herramientas: int):
        self.ventana = ventana
        self.mensajes = [{"role": "system", "content": sistema}]
        self.fijos = tokens_herramientas  # esquemas de herramientas + plantilla
        self.chars_por_token = 3.0         # se calibra con el uso real
        self.tarea = ""
        self.lecciones = ""  # las de la tarea (lecciones.para_tarea): van en su mensaje y sobreviven al resumen
        self.tras_compactar = 0

    # ---- medición ----
    def tokens(self, mensajes=None) -> int:
        return int(sum(map(_largo, mensajes or self.mensajes)) / self.chars_por_token) + self.fijos

    def calibrar(self, prompt_tokens: int, mensajes_enviados: list) -> None:
        chars = sum(map(_largo, mensajes_enviados))
        if prompt_tokens and prompt_tokens > self.fijos + 50:
            real = chars / (prompt_tokens - self.fijos)
            self.chars_por_token = max(1.8, min(5.0, 0.7 * self.chars_por_token + 0.3 * real))

    def max_salida(self) -> int:
        libre = self.ventana - self.tokens() - 256
        return max(256, min(MAX_SALIDA, libre))

    def uso(self) -> str:
        t = self.tokens()
        return f"{t}/{self.ventana} tokens ({100 * t // self.ventana}%)"

    # ---- historial ----
    def usuario(self, texto: str) -> None:
        self.tarea = texto
        self.mensajes.append({"role": "user", "content": texto})

    def asistente(self, contenido: str, llamadas: list) -> None:
        m = {"role": "assistant", "content": contenido or ""}
        if llamadas:
            m["tool_calls"] = [{"id": c["id"], "type": "function",
                                "function": {"name": c["name"], "arguments": json_valido(c["arguments"])}}
                               for c in llamadas]
        self.mensajes.append(m)

    def herramienta(self, id_llamada: str, resultado: str) -> None:
        self.mensajes.append({"role": "tool", "tool_call_id": id_llamada, "content": resultado})

    def reparar(self) -> None:
        """Tras una interrupción: toda llamada de herramienta debe tener su resultado."""
        respondidas = {m.get("tool_call_id") for m in self.mensajes if m["role"] == "tool"}
        for m in list(self.mensajes):
            for tc in m.get("tool_calls") or []:
                if tc["id"] not in respondidas:
                    self.herramienta(tc["id"], "Interrumpido por el usuario.")

    # ---- compactación ----
    def hace_falta(self, salida_minima: int = 2048) -> bool:
        t = self.tokens()
        sin_espacio = self.ventana - t < salida_minima + 512
        # Tras compactar no se repite hasta que el historial crezca otro 10 % (no rehacer en cada paso).
        return sin_espacio or (t > self.ventana * 0.6 and t > self.tras_compactar + self.ventana * 0.1)

    def compactar(self, llm, plan: str, forzar: bool = False) -> str:
        antes = self.tokens()
        self._acortar_antiguos(COLA)
        if self.tokens() > self.ventana * 0.45 or forzar:
            self._acortar_antiguos(2)  # los archivos ya están en disco: no hace falta su contenido
        if self.tokens() > self.ventana * 0.45 or forzar:
            self._resumir(llm, plan)
        self.tras_compactar = self.tokens()
        return f"contexto compactado: {antes} -> {self.tras_compactar} tokens"

    def _acortar_antiguos(self, conservar: int) -> None:
        for m in self.mensajes[1:len(self.mensajes) - conservar]:
            if m["role"] == "tool" and len(m["content"]) > 300:
                m["content"] = m["content"][:240] + " [...resultado antiguo recortado]"
            for tc in m.get("tool_calls") or []:
                f = tc["function"]
                if len(f["arguments"]) <= 400:
                    continue
                # Se QUITAN los textos largos (contenido de archivos ya guardados en disco). Nunca se deja un marcador
                # tipo "[N líneas omitidas]": el modelo imita su historial y lo escribiría como contenido real.
                try:
                    args = json.loads(f["arguments"])
                    args = {k: v for k, v in args.items() if not (isinstance(v, str) and len(v) > 300)}
                    f["arguments"] = json.dumps(args, ensure_ascii=False)
                except ValueError:
                    f["arguments"] = "{}"

    def _corte(self) -> int:
        """Índice donde empieza la cola conservada: debe ser un mensaje de asistente (nunca un 'tool' huérfano)."""
        for i in range(max(1, len(self.mensajes) - COLA), len(self.mensajes)):
            if self.mensajes[i]["role"] == "assistant":
                return i
        return len(self.mensajes)

    def _resumir(self, llm, plan: str) -> None:
        corte = self._corte()
        viejos = self.mensajes[1:corte]
        if not viejos:
            return
        lineas = []
        for m in viejos:
            if m["role"] == "user":
                lineas.append(f"USUARIO: {m['content'][:1500]}")
            elif m["role"] == "assistant":
                if m["content"]:
                    lineas.append(f"AGENTE: {m['content'][:400]}")
                for tc in m.get("tool_calls") or []:
                    lineas.append(f"AGENTE llamó {tc['function']['name']}({tc['function']['arguments'][:200]})")
            else:
                lineas.append(f"RESULTADO: {m['content'][:300]}")
        transcripcion = "\n".join(lineas)[-int(self.ventana * 0.4 * self.chars_por_token):]
        try:
            r = llm.chat([{"role": "system", "content": "Eres un asistente que resume registros de trabajo."},
                          {"role": "user", "content": f"{transcripcion}\n\n{RESUMIR}"}], None, 700, False)
            resumen = r["content"].strip() or transcripcion[-1500:]
        except Exception:  # sin modelo: conservar el final de la transcripción
            resumen = transcripcion[-1500:]
        nuevo = {"role": "user", "content": (f"TAREA: {self.tarea[:3000]}{self.lecciones}"
                                             f"\n\nPROGRESO HASTA AHORA (resumen):\n{resumen}"
                                             f"\n\nPLAN Y ESTADO:\n{plan or '(sin plan)'}\n\nContinúa desde donde quedaste.")}
        self.mensajes = [self.mensajes[0], nuevo] + self.mensajes[corte:]
