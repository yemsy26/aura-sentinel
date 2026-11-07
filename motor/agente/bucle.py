"""Bucle del agente: planificar -> ejecutar herramientas -> verificar -> terminar, con controles que no dependen del modelo."""
import json
import re
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

from . import grande, lecciones, metricas
from . import llm as cliente
from . import mapa as mapa_repo
from .contexto import Contexto
from .esquemas import ESQUEMA_MAPA, ESQUEMAS
from .herramientas import Herramientas, archivos_pedidos, despachar
from .llamadas import coaccionar, llamadas_en_texto, sin_llamadas
from .prompts import REGLAS_USUARIO, entorno, sistema
from .proyecto import es_codigo, ignorado
from .revision import mtime_fuentes, revisar
from .verificacion import claves_inventadas

ICONOS = {"plan": "PLAN", "paso_hecho": "HECHO", "listar": "ls", "leer": "leer", "buscar": "grep", "escribir": "+",
          "editar": "~", "ejecutar": ">", "procesos": "ps", "crear_bat": "bat", "web_buscar": "web", "web_leer": "web",
          "investigar": "web", "guia": "guia", "version": "ver", "verificar": "QA", "revisar": "REV",
          "aprender": "nota", "restaurar": "UNDO", "ver_pantalla": "VER", "git": "git", "mapa": "mapa",
          "plan_partes": "PARTES", "terminar": "FIN"}
ARG_VISIBLE = ("ruta", "comando", "consulta", "pregunta", "url", "nombre", "patron", "tema", "paquete", "accion",
               "numero", "enfoque")
ES_ERROR = (r"(exit=[1-9-]|ERROR|TIMEOUT|RECHAZADO|Argumentos|VERIFICACIÓN \([^)]*\): [1-9]|VISUAL: PROBLEMAS|"
            r"'buscar' aparece|"
            r"REVISIÓN:(?=[\s\S]*CAMBIOS NECESARIOS))")
# Tras estas herramientas el IDE refresca el editor y el árbol de archivos (modo --json).
CAMBIAN_ARCHIVOS = {"escribir", "editar", "crear_bat", "restaurar", "ejecutar", "git"}
AVISO_MAPA = "\n\n(Repositorio grande: mira mapa() antes de listar o leer archivos.)"
# Importar una herramienta desde un script: sin este aviso, el modelo acaba programando su propio resolvedor.
MODULO_ES_HERRAMIENTA = re.compile(r"No module named '(resolver_sat|calcular)'")


def aviso_herramienta(resultado: str) -> str:
    m = MODULO_ES_HERRAMIENTA.search(resultado)
    return (f"\n[AVISO: {m[1]} es una herramienta tuya, no un módulo de Python: tus scripts no pueden usarla. Quita el import, "
            f"haz que el script solo escriba el archivo o lea el resultado, y llama tú a {m[1]} entre un script y otro.]" if m else "")
APROBAR_PLAN = "Aprobar y empezar"
DEPENDEN_DEL_ESTADO = {"ejecutar", "leer", "listar", "buscar", "procesos", "verificar", "revisar", "paso_hecho",
                       "ver_pantalla", "git", "resolver_sat"}


def color(txt: str, c: str) -> str:
    codigos = {"gris": "90", "verde": "32", "rojo": "31", "amarillo": "33", "cian": "36", "negrita": "1"}
    return f"\033[{codigos[c]}m{txt}\033[0m" if sys.stdout.isatty() else txt


@dataclass
class _Marcha:
    """Lo que el bucle recuerda durante una tarea para notar que el modelo se atasca."""
    modo: str
    razonar: bool
    por_aprobar: bool
    repeticiones: dict[str, int] = field(default_factory=dict)
    lecturas: dict[tuple, int] = field(default_factory=dict)  # del mismo archivo sin escribir nada entre medias
    cambios: int = 0  # acciones NUEVAS que pueden cambiar el proyecto (un comando distinto: copiar, instalar...)
    fallos: dict[str, int] = field(default_factory=dict)
    rechazos_fin: int = 0
    empujones: int = 0
    encargo: bool = False  # mandato largo (no una pregunta ni «continúa»): se hace con herramientas
    actuo: bool = False  # el modelo llamó a alguna herramienta en esta tarea


class Agente:
    def __init__(self, cfg: dict, raiz: Path, interactivo: bool = True):
        self.cfg, self.interactivo = cfg, interactivo
        base, modelo, ventana = cliente.detectar(cfg)
        if not modelo:
            raise cliente.ErrorLLM("No hay ningún LLM cargado. Ejecuta cargar_modelo.bat (o carga uno en Bionic/LM Studio).")
        self.llm = cliente.LLM(base, modelo, ventana, cfg)
        max_chars = cfg["max_chars_resultado"] or max(2500, min(12000, int(ventana * 0.2)))
        confirmar = (lambda c: True) if not cfg["confirmar_peligrosos"] else self._confirmar if interactivo else None
        self.h = Herramientas(raiz, max_chars, cfg["timeout_comando"], confirmar)
        self.h.cfg_llm = cfg  # ver_pantalla consulta al mismo servidor/modelo
        self.tokens_herr = len(json.dumps(ESQUEMAS, ensure_ascii=False)) // 3 + 400
        self.esquemas = ESQUEMAS  # + mapa en repos grandes (se decide al empezar cada mandato y ya no se quita)
        self.con_mapa = False
        self.hilo_mapa = None  # caché del mapa poniéndose al día en segundo plano
        self.requisitos_parte = ""  # modo grande: lo que la revisión debe exigir en la parte en curso
        self.entorno = entorno()
        self.ctx = Contexto(ventana, sistema(self.h.raiz, ventana, self.entorno), self.tokens_herr)
        self.registro = self.h.dir_agente / "registro.log"
        self.h.dir_agente.mkdir(exist_ok=True)
        self._ultimo_progreso = 0.0
        self.pasos_totales = 0
        self.requeridos: set[str] = set()
        self.inicio_tarea = 0
        self.revisiones = 0
        self.h.revisar_cb = self._revisar
        self.h.preguntar_cb = self._preguntar if interactivo else None
        self.emitir = None  # modo --json: emitir(tipo, **datos) publica un evento para el IDE
        self.responder = None  # modo --json: responder(tipo, **datos) espera la respuesta del usuario
        self.revisar_plan = False  # el usuario aprueba el plan antes de que se cambie nada (opción de Aura)

    # ---------- utilidades de consola ----------
    def _confirmar(self, comando: str, motivo: str = "ejecutar un comando potencialmente destructivo") -> bool:
        if self.responder:
            return bool(self.responder("confirmar", comando=comando, motivo=motivo).get("si"))
        print(color(f"\n  ¿Permitir {motivo}?\n    {comando}", "amarillo"))
        return input("  (s/n) > ").strip().lower() in ("s", "si", "sí", "y")

    def _preguntar(self, pregunta: str, opciones: list[str]) -> str:
        if self.responder:
            return str(self.responder("pregunta", pregunta=pregunta, opciones=opciones).get("texto", ""))
        print(color(f"\n  El agente pregunta: {pregunta}", "amarillo"))
        for i, o in enumerate(opciones, 1):
            print(f"    {i}. {o}")
        respuesta = input("  (número u otra respuesta) > ").strip()
        return opciones[int(respuesta) - 1] if respuesta.isdigit() and 0 < int(respuesta) <= len(opciones) else respuesta

    def _aprobar_plan(self, resultado: str) -> tuple[bool, str]:
        """El usuario revisa el plan antes de empezar: lo aprueba o pide cambios. Devuelve (sigue por aprobar, resultado)."""
        respuesta = self._preguntar("Revisa el plan antes de que empiece:\n" + self.h.plan_texto(), [APROBAR_PLAN]).strip()
        if not respuesta or respuesta == APROBAR_PLAN:
            self._log("PLAN aprobado por el usuario")
            return False, resultado + "\n[El usuario APROBÓ el plan: empieza por el paso 1.]"
        self._log(f"PLAN: el usuario pide cambios: {respuesta}")
        return True, resultado + (f"\n[El usuario pide cambios en el plan: {respuesta}\nRehaz el plan con plan(...) "
                                  "con esos cambios y espera a que lo apruebe; no cambies archivos todavía.]")

    def _progreso(self, razon: int, texto: int) -> None:
        ahora = time.time()
        if self.emitir:
            if ahora - self._ultimo_progreso > 1.0:
                self._ultimo_progreso = ahora
                self.emitir("progreso", fase="escribiendo" if texto else "pensando", caracteres=texto or razon)
        elif sys.stdout.isatty() and ahora - self._ultimo_progreso > 0.25:
            self._ultimo_progreso = ahora
            fase = f"escribiendo {texto} car" if texto else f"pensando {razon} car"
            print(f"\r  {color(fase, 'gris')}   ", end="", flush=True)

    @staticmethod
    def _limpiar_linea() -> None:
        if sys.stdout.isatty():
            print("\r" + " " * 70 + "\r", end="")

    def _salida(self, tipo: str, texto: str, tono: str = "amarillo") -> None:
        """Consola: línea con color. Modo --json: evento (aviso o nota)."""
        if self.emitir:
            self.emitir(tipo, texto=texto)
        else:
            print(color(f"  {texto}", tono))

    def _decir(self, texto: str) -> None:
        if not self.emitir:  # en modo --json la respuesta final va en el evento "fin"
            print(color("\nAgente: ", "negrita") + texto)

    def _log(self, texto: str) -> None:
        with open(self.registro, "a", encoding="utf-8") as f:
            f.write(f"[{time.strftime('%H:%M:%S')}] {texto}\n")

    def _medir(self, c: dict, args: dict, inicio: float, resultado: str) -> None:
        metricas.anotar(self.h.dir_agente, tipo="llamada", paso=self.pasos_totales, nombre=c["name"],
                        s=round(time.time() - inicio, 2), args_car=len(c.get("arguments") or ""),
                        resultado_car=len(resultado), **metricas.detalle(c["name"], args))

    def _compactar(self, forzar: bool = False) -> None:
        inicio = time.time()
        texto = self.ctx.compactar(self.llm, self._estado(), forzar=forzar)
        metricas.anotar(self.h.dir_agente, tipo="compactar", paso=self.pasos_totales, s=round(time.time() - inicio, 2),
                        texto=texto)
        self._salida("aviso", f"({texto})")

    def _mostrar(self, paso: int, nombre: str, args: dict, resultado: str) -> None:
        visible = next((str(args[k]) for k in ARG_VISIBLE if k in args), "")
        primera = resultado.strip().splitlines()[0] if resultado.strip() else ""
        malo = bool(re.match(r"(exit=[1-9-]|ERROR|TIMEOUT|BLOQUEADO|No existe|No encontré|RECHAZADO|'buscar' aparece|"
                             r"VERIFICACIÓN \([^)]*\): [1-9]|VISUAL: PROBLEMAS)", primera))
        if self.emitir:
            evento = {"paso": paso, "herramienta": nombre, "detalle": visible[:300], "resultado": primera[:300],
                      "error": malo, "salida": resultado[:2000]}
            if nombre in ("plan", "paso_hecho"):
                evento["plan"] = self.h.plan_texto()
            self.emitir("paso", **evento)
            if nombre in CAMBIAN_ARCHIVOS:  # rutas vacías = refrescar todo el árbol
                ruta = args.get("ruta") or args.get("nombre")
                self.emitir("archivos", rutas=[str(ruta)] if ruta and nombre not in ("ejecutar", "git") else [])
            return
        print(f"  [{paso}] {color(ICONOS.get(nombre, nombre), 'cian')} {visible[:90]}  "
              f"{color('-> ' + primera[:110], 'rojo' if malo else 'gris')}")
        if nombre == "plan":
            print(color("      " + self.h.plan_texto().replace("\n", "\n      "), "gris"))

    # ---------- bucle ----------
    def tarea(self, texto: str, max_pasos: int | None = None, requeridos: set[str] | None = None,
              etiqueta: str = "") -> str:
        """Un mandato del usuario. Con max_pasos es una parte del modo grande (no se vuelve a enrutar)."""
        if max_pasos is None and (como := grande.modo(texto, self._repo_grande(), self.h.dir_agente)):
            return grande.ejecutar(self, texto, como)
        respuesta = self._tarea(texto, max_pasos, requeridos, etiqueta)
        if max_pasos is None and grande.INCOMPLETA.match(respuesta):
            self.h.tarea_interrumpida()
        return respuesta

    def _tarea(self, texto: str, max_pasos: int | None, requeridos: set[str] | None, etiqueta: str) -> str:
        m = self._preparar(texto, max_pasos, requeridos, etiqueta)
        limite = max_pasos or self.cfg["max_pasos"]
        for paso in range(1, limite + 1):
            self.pasos_totales += 1
            self.h.paso = self.pasos_totales  # contador global: válido entre tareas sucesivas
            if self.ctx.hace_falta():
                self._compactar()
            try:
                r, llamadas, contenido = self._pedir_al_modelo(m)
            except cliente.ErrorLLM as e:
                self._limpiar_linea()
                if "context" in str(e).lower() and len(self.ctx.mensajes) > 3:
                    self._compactar(forzar=True)
                    continue
                self._salida("aviso", f"Error del modelo: {e}", "rojo")
                return f"Error del modelo: {e}"

            if not llamadas:
                respuesta = self._sin_llamadas(contenido, r["finish_reason"], m)
                if respuesta is None:
                    continue
                return respuesta

            m.empujones, m.actuo = 0, True
            self.ctx.asistente(contenido, llamadas)
            if contenido:
                self._salida("nota", f"· {contenido[:300]}", "gris")
            hubo_error = False
            for c in llamadas:
                fin, error = self._atender(c, r["finish_reason"], paso, m)
                if fin is not None:
                    return fin
                hubo_error |= error
            if hubo_error and m.modo == "auto":
                m.razonar = True  # pensar solo cuando hace falta (ahorra tokens)
        msg = f"Se alcanzó el máximo de {limite} pasos. Escribe 'continúa' para seguir." + self._rescatar()
        self._salida("aviso", msg)
        return msg

    def _preparar(self, texto: str, max_pasos: int | None, requeridos: set[str] | None, etiqueta: str) -> _Marcha:
        """Deja el contexto y las herramientas listos para una tarea nueva o para seguir la anterior."""
        self.ctx.usuario(texto)
        if self.con_mapa and max_pasos is None:
            self.ctx.mensajes[-1]["content"] += AVISO_MAPA
        self._log(etiqueta or f"USUARIO: {texto}")
        nuevos = archivos_pedidos(texto)
        sigue = bool(re.match(r"\s*(contin|sigue|segu|finaliza|termina)", texto, re.IGNORECASE))
        # Las lecciones que tocan van con la tarea, no en el prompt de sistema: así este no cambia entre tareas
        # (su prefijo sigue en la caché del servidor) y no ocupan sitio las que no tienen que ver.
        if not sigue:
            self.ctx.lecciones = lecciones.para_tarea(texto, self.h.raiz)
            self.ctx.mensajes[-1]["content"] += self.ctx.lecciones
        if requeridos is not None:
            self.requeridos = set(requeridos)
        elif nuevos or not sigue:
            self.requeridos = nuevos  # 'continúa' conserva los de la tarea en curso
        por_aprobar = self.revisar_plan and max_pasos is None and self.responder is not None and not sigue
        self.h.nueva_tarea(self.pasos_totales + 1, texto if max_pasos is None and not sigue else "")
        self.inicio_tarea, self.revisiones = self.pasos_totales + 1, 0
        metricas.anotar(self.h.dir_agente, tipo="tarea", paso=self.pasos_totales + 1,  # sin esto no se sabía el modelo
                        modelo=getattr(self.llm, "modelo", None), ventana=getattr(self.llm, "contexto", None))
        modo = self.cfg["razonamiento"]
        encargo = not sigue and len(texto.strip()) >= 150 and not texto.rstrip().endswith("?")
        return _Marcha(modo=modo, razonar=modo in ("on", "auto"), por_aprobar=por_aprobar,  # planificar con razonamiento
                       encargo=encargo)

    def _pedir_al_modelo(self, m: _Marcha) -> tuple[dict, list, str]:
        """Un turno del modelo: su respuesta, las llamadas que pide (también si las escribió como texto) y lo que dijo."""
        enviados = list(self.ctx.mensajes)
        t_modelo = time.time()
        r = self.llm.chat(enviados, self.esquemas, self.ctx.max_salida(), m.razonar, self._progreso)
        self._limpiar_linea()
        self.ctx.calibrar((r["usage"] or {}).get("prompt_tokens", 0), enviados)
        llamadas = r["tool_calls"] or llamadas_en_texto(r["content"])
        contenido = (sin_llamadas(r["content"]) if llamadas and not r["tool_calls"]
                     else re.sub(r"<tool_call>.*?(</tool_call>|$)", "", r["content"], flags=re.DOTALL).strip())
        uso = r["usage"] or {}
        metricas.anotar(self.h.dir_agente, tipo="turno", paso=self.pasos_totales,
                        modelo_s=round(time.time() - t_modelo, 2), espera_s=r.get("espera") and round(r["espera"], 2),
                        tokens_entrada=uso.get("prompt_tokens"), tokens_salida=uso.get("completion_tokens"),
                        razonamiento_car=len(r.get("reasoning") or ""), nota_car=len(contenido),
                        args_car=sum(len(c.get("arguments") or "") for c in llamadas), llamadas=len(llamadas),
                        fin=r["finish_reason"], timings=r.get("timings") or None)
        m.razonar = m.modo == "on"
        return r, llamadas, contenido

    def _sin_llamadas(self, contenido: str, fin: str, m: _Marcha) -> str | None:
        """El modelo respondió sin pedir nada: la respuesta final, o None si hay que empujarlo a seguir."""
        self.ctx.asistente(contenido, [])
        if fin == "length":
            self.ctx.mensajes.append({"role": "user", "content": (
                "Tu respuesta se cortó por longitud. Divide el trabajo: archivos más pequeños (módulos) "
                "y una llamada a escribir por archivo.")})
            return None
        pendiente = "- [ ]" in self.h.plan_texto()
        pregunta = contenido.rstrip().endswith("?")
        if contenido.count("```") >= 2 and contenido.count("\n") > 25 and m.empujones < 2:
            m.empujones += 1  # conducta: pegar código en el chat en vez de escribir archivos
            self.ctx.mensajes.append({"role": "user", "content": (
                "No pegues código en el chat: guárdalo con escribir (un archivo por llamada) y sigue el método.")})
            return None
        if pendiente and not pregunta and m.empujones < 2:
            m.empujones += 1
            self.ctx.mensajes.append({"role": "user", "content": (
                "Continúa con el plan usando las herramientas (no describas, actúa). "
                f"Pasos:\n{self.h.plan_texto()}")})
            return None
        # Acabar con texto se saltaba la puerta de terminar: con otro modelo, una misión dijo «He completado todas las
        # fases» con un paso del plan sin hacer, sin verificar ni revisar. Con el plan a medias y archivos cambiados en
        # la tarea, pasa la misma puerta (un arreglo pequeño que acaba con texto y sin plan pendiente, no).
        if pendiente and not pregunta and self.h.paso_escritura >= self.inicio_tarea and \
                (motivo := self._puede_terminar(m.rechazos_fin, self.pasos_totales, contenido)):
            m.rechazos_fin += 1
            self.ctx.mensajes.append({"role": "user", "content": f"{motivo}\nCuando esté, llama a terminar."})
            return None
        caps = (getattr(self.llm, "info", None) or {}).get("capabilities") or {}
        if m.encargo and not m.actuo and caps.get("trained_for_tool_use") is False:
            contenido = (f"[AVISO: el modelo {self.llm.modelo} no está entrenado para usar herramientas y no ha hecho "
                         "nada en el proyecto. Elige uno con «tool use», como qwen3.6-35b-a3b.]\n" + contenido)
        self._decir(contenido)
        self._log(f"AGENTE: {contenido}")
        return contenido

    def _atender(self, c: dict, fin: str, paso: int, m: _Marcha) -> tuple[str | None, bool]:
        """Una llamada del modelo: (respuesta final si con ella acaba la tarea, si el resultado es un error)."""
        t_llamada = time.time()
        try:
            args = coaccionar(c["name"], json.loads(c["arguments"] or "{}", strict=False))
        except ValueError as e:
            if fin == "length":  # se acabó el máximo de una respuesta en mitad de la llamada
                resultado = (f"Tu llamada a {c['name']} se CORTÓ: superó el máximo de una respuesta. No la repitas "
                             "igual. Escribe el archivo en partes: la primera con escribir y el resto con "
                             "escribir(ruta, contenido, anadir=true), cada parte de unas 150 líneas como mucho; "
                             "o divídelo en archivos más pequeños (p. ej. css/base.css, css/productos.css).")
            else:
                resultado = f"Argumentos JSON inválidos ({e}). Repite la llamada con JSON válido."
            self.ctx.herramienta(c["id"], resultado)
            self._mostrar(paso, c["name"], {}, resultado)
            self._medir(c, {}, t_llamada, resultado)
            return None, True

        clave = c["name"] + json.dumps(args, sort_keys=True, ensure_ascii=False)
        if c["name"] in DEPENDEN_DEL_ESTADO:  # tras modificar archivos, repetir la prueba es progreso
            clave += f"|{self.h.paso_escritura}"
            if c["name"] not in CAMBIAN_ARCHIVOS:  # y verificar tras un comando nuevo (no el mismo repetido)
                clave += f"|{m.cambios}"
        m.repeticiones[clave] = m.repeticiones.get(clave, 0) + 1

        if c["name"] == "terminar":
            motivo = self._puede_terminar(m.rechazos_fin, paso, str(args.get("resumen", "")))
            if not motivo:
                return self._cerrar_tarea(c, args, paso, t_llamada), False
            m.rechazos_fin += 1
            resultado = motivo
        elif m.por_aprobar and c["name"] in CAMBIAN_ARCHIVOS:
            resultado = ("RECHAZADO: el usuario quiere revisar el plan antes de que cambies nada. Lee lo que "
                         "necesites, crea el plan con plan(...) y espera su respuesta.")
        else:
            self.esperar_mapa()
            resultado = despachar(self.h, c["name"], args)
            resultado += aviso_herramienta(resultado)
            resultado += self._avisos_de_conducta(c["name"], args, clave, m)
            if m.repeticiones[clave] >= 5:
                self.ctx.herramienta(c["id"], resultado)
                msg = f"Detenido: el modelo repitió '{c['name']}' 5 veces sin avanzar." + self._rescatar()
                self._salida("aviso", msg, "rojo")
                return msg, False
            resultado += self._avisos_de_ejecucion(c["name"], args, resultado, m)
            if c["name"] == "plan" and m.por_aprobar and resultado.startswith("Plan guardado"):
                m.por_aprobar, resultado = self._aprobar_plan(resultado)
            siguiente = next((l[6:] for l in self.h.plan_texto().splitlines() if l.startswith("- [ ]")), "")
            if siguiente and c["name"] not in ("plan", "paso_hecho"):
                resultado += f"\n[Plan, paso pendiente: {siguiente}]"
        self._mostrar(paso, c["name"], args, resultado)
        self._log(f"{c['name']} {json.dumps(args, ensure_ascii=False)[:300]} -> {resultado[:300]}")
        self._medir(c, args, t_llamada, resultado)
        self.ctx.herramienta(c["id"], resultado)
        return None, bool(re.match(ES_ERROR, resultado))

    def _cerrar_tarea(self, c: dict, args: dict, paso: int, t_llamada: float) -> str:
        """terminar aceptado: guarda y dice el resumen."""
        resumen = str(args.get("resumen", "")).strip()
        if self.h.verificacion["errores"] > 0:  # salió por la válvula anti-bucle con errores
            resumen = ("[AVISO: la tarea termina SIN resolver todos los errores de verificación."
                       f"{self._rescatar()}]\n" + resumen)
        self.ctx.herramienta(c["id"], "Tarea terminada.")
        self._mostrar(paso, "terminar", {}, "Tarea terminada")
        self._medir(c, args, t_llamada, "Tarea terminada.")
        self._guardar_resumen(resumen)
        self._decir(resumen)
        self._log(f"FIN: {resumen}")
        return resumen

    def _avisos_de_conducta(self, nombre: str, args: dict, clave: str, m: _Marcha) -> str:
        """Avisos al modelo cuando repite lo mismo sin avanzar."""
        aviso = ""
        if nombre == "leer":
            vez = (str(args.get("ruta")), self.h.paso_escritura)
            m.lecturas[vez] = m.lecturas.get(vez, 0) + 1
            if m.lecturas[vez] >= 5:
                aviso += (f"\n[AVISO: es la {m.lecturas[vez]}ª vez que lees {vez[0]} sin cambiar nada: el texto no va a "
                          "cambiar. Decide ya: corrige con editar o ejecuta algo que aporte datos nuevos.]")
        if nombre in CAMBIAN_ARCHIVOS and m.repeticiones[clave] == 1:
            m.cambios += 1
        if m.repeticiones[clave] >= 3:
            aviso += (f"\n[AVISO: es la {m.repeticiones[clave]}ª vez que haces exactamente esta llamada. "
                      "Cambia de estrategia: lee el error, investiga con web_buscar o pregunta al usuario.]")
        return aviso

    def _avisos_de_ejecucion(self, nombre: str, args: dict, resultado: str, m: _Marcha) -> str:
        """Avisos al modelo cuando un comando falla seguido o el plan olvida lo que pidió el usuario."""
        aviso = ""
        if nombre == "ejecutar":
            cmd = str(args.get("comando", ""))
            m.fallos[cmd] = m.fallos.get(cmd, 0) + 1 if re.match(r"(exit=[1-9-]|TIMEOUT)", resultado) else 0
            if m.fallos[cmd] in (3, 6):
                aviso += ("\n[ATASCO: este comando falla por " + str(m.fallos[cmd]) + "ª vez seguida. No reescribas "
                          "a ciegas: lee el error y el código implicado (leer), escribe en una frase la causa raíz "
                          "y corrige solo eso con editar. Si no la encuentras, usa web_buscar con el error.]")
        if nombre == "plan" and (olvidados := [f for f in self.requeridos if f.lower() not in resultado.lower()]):
            aviso += f"\n[AVISO: el usuario pidió {', '.join(sorted(olvidados))} y el plan no lo incluye. Corrige el plan.]"
        return aviso

    def _revisar(self, requisitos: str = "", enfoque: str = "") -> tuple[str, bool]:
        self.revisiones += 1
        self._salida("nota", "(revisión independiente del código en curso...)", "gris")
        texto, ok = revisar(self.llm, self.h.raiz, requisitos or self.requisitos_parte or self.ctx.tarea,
                            self.h.verificacion["informe"], enfoque, self.h.evidencia_texto())
        self.h.dir_agente.mkdir(exist_ok=True)
        (self.h.dir_agente / "revision.json").write_text(json.dumps(
            {"aprobado": ok, "fecha": time.time(), "texto": texto[:5000]}, ensure_ascii=False), encoding="utf-8")
        self._log(f"REVISIÓN ({'aprobada' if ok else 'cambios'}): {texto[:1500]}")
        return texto, ok

    def _puede_terminar(self, rechazos: int, paso: int, resumen: str = "") -> str:
        """Puerta de calidad: lo que un agente avanzado comprueba antes de decir 'hecho'."""
        if rechazos >= 4:
            return ""
        faltan = [f for f in sorted(self.requeridos) if not self._existe(f)]
        if faltan:
            return (f"RECHAZADO: el usuario pidió {', '.join(faltan)} y no existe en el proyecto. "
                    "Créalo con ese nombre exacto (crear_bat para .bat), pruébalo y vuelve a llamar a terminar.")
        escribio = self.h.paso_escritura >= self.inicio_tarea
        hay_codigo = self._mtime_codigo() > 0
        ultima = self.h.verificacion["paso"]
        if hay_codigo and (ultima < self.h.paso_escritura or ultima < self.inicio_tarea):
            informe = self.h.verificar_cmd()  # verificación automática del estado final
            self._mostrar(paso, "verificar", {}, informe)
            if self.h.verificacion["errores"] > 0:
                return "RECHAZADO: la verificación automática encontró errores:\n" + informe
        elif escribio and self.h.verificacion["errores"] > 0:
            return "RECHAZADO: la última verificación tiene errores sin corregir:\n" + self.h.verificacion["informe"]
        previa = self._revision_previa()
        # Lo que lee el revisor (también capabilities/*.json o tauri.conf.json) no cambió desde que pidió cambios. Si la
        # revisión es de esta tarea (también un revisar a mano), terminar sin tocar nada la ignoraba.
        pendiente = previa and not previa["aprobado"] and previa["fecha"] >= mtime_fuentes(self.h.raiz)
        if pendiente and (not escribio or self.revisiones):
            return ("RECHAZADO: la última revisión pidió cambios y el código no se ha modificado desde entonces. "
                    "Corrige estos puntos con editar, verifica y luego termina:\n" + previa["texto"])
        # 1ª revisión siempre; 2ª si se cambió código después de una revisión que pidió cambios.
        if escribio and (self.revisiones == 0 or (self.revisiones < 2 and previa and not previa["aprobado"]
                                                    and not pendiente)):
            texto, ok = self._revisar()
            self._mostrar(paso, "revisar", {}, texto)
            if not ok:
                return ("RECHAZADO: la revisión independiente pide cambios. Corrige cada punto CRITICO con editar, "
                        "llama a verificar y después a terminar.\n" + texto)
        if self.h.paso_escritura > self.h.paso_prueba:
            return ("RECHAZADO: modificaste archivos y no has ejecutado nada con éxito desde entonces. "
                    "Ejecuta el programa, los tests o el .bat, revisa la salida y después llama a terminar.")
        pendientes = [l for l in self.h.plan_texto().splitlines() if l.startswith("- [ ]")]
        if pendientes:
            return ("RECHAZADO: quedan pasos del plan sin marcar:\n" + "\n".join(pendientes) +
                    "\nComplétalos o márcalos con paso_hecho si ya están hechos.")
        if (malas := claves_inventadas(self.h.raiz, resumen)):
            # registro.log corta cada llamada a 300 caracteres: sin el resumen entero no se podía saber si fue justo.
            self._log("RESUMEN RECHAZADO (contraseña):\n" + resumen)
            return "RECHAZADO: " + malas
        return ""

    def _repo_grande(self) -> bool:
        """En repos grandes se ofrece mapa (y su caché se pone al día en segundo plano). Una vez ofrecido no se quita:
        cambiar la lista de herramientas invalidaría la caché de prompt del servidor."""
        if not mapa_repo.es_grande(self.h.raiz):
            return False
        if not self.con_mapa:
            self.con_mapa, self.esquemas = True, [*ESQUEMAS, ESQUEMA_MAPA]
            extra = len(json.dumps(ESQUEMA_MAPA, ensure_ascii=False)) // 3
            self.tokens_herr += extra
            self.ctx.fijos += extra
        self.hilo_mapa = mapa_repo.precalentar(self.h.raiz)
        return True

    def esperar_mapa(self) -> None:
        """Antes de cualquier herramienta: en Windows, leer un archivo a la vez que otro lo reescribe o lo borra falla.
        La caché suele estar lista mucho antes (el modelo tarda más en responder que el mapa en hacerse)."""
        if self.hilo_mapa is not None:
            self.hilo_mapa.join()
            self.hilo_mapa = None

    def nuevo_contexto(self) -> None:
        """Contexto limpio con el mismo prompt de sistema (el prefijo sigue en la caché del servidor)."""
        self.ctx = Contexto(self.llm.contexto, self.ctx.mensajes[0]["content"], self.tokens_herr)

    def _estado(self) -> str:
        return (f"{self.h.plan_texto() or '(sin plan)'}\n\nARCHIVOS DEL PROYECTO (nombres reales: úsalos tal cual):\n"
                f"{self.h.mapa()}")

    def _rescatar(self) -> str:
        """Si la tarea acaba con el proyecto roto pero hubo una versión que funcionaba, se entrega esa."""
        self.esperar_mapa()
        self.h._cargar_checkpoints()
        if not self.h.checkpoints:
            return ""
        self.h.verificar_cmd()
        if self.h.verificacion["errores"] == 0:
            return ""
        r = self.h.restaurar()
        self._log(f"RESCATE: {r}")
        return f"\nEl estado final tenía errores: {r}"

    def _mtime_codigo(self) -> float:
        fuentes = [p for p in self.h.raiz.rglob("*") if es_codigo(p) and p.suffix.lower() not in (".json", ".xml")
                   and not ignorado(p, self.h.raiz)]
        return max((p.stat().st_mtime for p in fuentes), default=0)

    def _revision_previa(self) -> dict | None:
        f = self.h.dir_agente / "revision.json"
        try:
            return json.loads(f.read_text(encoding="utf-8")) if f.exists() else None
        except ValueError:
            return None

    def nueva_regla(self, texto: str) -> str:
        previo = REGLAS_USUARIO.read_text(encoding="utf-8") if REGLAS_USUARIO.exists() else "# Reglas del usuario\n"
        REGLAS_USUARIO.write_text(previo.rstrip("\n") + f"\n- {texto.strip()}\n", encoding="utf-8")
        self.ctx.mensajes[0]["content"] = sistema(self.h.raiz, self.llm.contexto, self.entorno)
        return f"Regla guardada en {REGLAS_USUARIO.name} (se aplica ya y en todas las tareas futuras)."

    def _existe(self, nombre: str) -> bool:
        return self.h.existe(nombre)

    def _guardar_resumen(self, resumen: str) -> None:
        f = self.h.dir_agente / "resumen.md"
        f.write_text(f"# Resumen ({time.strftime('%Y-%m-%d %H:%M')})\n\n## Tarea\n{self.ctx.tarea}\n\n"
                     f"## Resultado\n{resumen}\n\n## Plan\n{self.h.plan_texto()}\n", encoding="utf-8")
        if self.h.archivo_plan.exists():  # la próxima tarea empieza con plan limpio
            self.h.archivo_plan.replace(self.h.dir_agente / "plan_anterior.md")

    def cerrar(self) -> None:
        self.h.detener_todos()
