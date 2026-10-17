"""Bucle del agente: planificar -> ejecutar herramientas -> verificar -> terminar, con controles que no dependen del modelo."""
import json
import re
import sys
import time
import uuid
from pathlib import Path

from . import llm as cliente
from .contexto import Contexto
from .herramientas import ESQUEMAS, Herramientas, archivos_pedidos, despachar
from .prompts import REGLAS_USUARIO, entorno, sistema
from .revision import revisar

TIPOS = {e["function"]["name"]: {k: v.get("type") for k, v in e["function"]["parameters"]["properties"].items()}
         for e in ESQUEMAS}
ICONOS = {"plan": "PLAN", "paso_hecho": "HECHO", "listar": "ls", "leer": "leer", "buscar": "grep", "escribir": "+",
          "editar": "~", "ejecutar": ">", "procesos": "ps", "crear_bat": "bat", "web_buscar": "web", "web_leer": "web",
          "investigar": "web", "guia": "guia", "version": "ver", "verificar": "QA", "revisar": "REV",
          "aprender": "nota", "restaurar": "UNDO", "ver_pantalla": "VER", "git": "git", "terminar": "FIN"}
ARG_VISIBLE = ("ruta", "comando", "consulta", "pregunta", "url", "nombre", "patron", "tema", "paquete", "accion",
               "numero", "enfoque")
ES_ERROR = (r"(exit=[1-9-]|ERROR|TIMEOUT|RECHAZADO|Argumentos|VERIFICACIÓN \([^)]*\): [1-9]|VISUAL: PROBLEMAS|"
            r"'buscar' aparece|"
            r"REVISIÓN:(?=[\s\S]*CAMBIOS NECESARIOS))")
# Tras estas herramientas el IDE refresca el editor y el árbol de archivos (modo --json).
CAMBIAN_ARCHIVOS = {"escribir", "editar", "crear_bat", "restaurar", "ejecutar", "git"}
DEPENDEN_DEL_ESTADO = {"ejecutar", "leer", "listar", "buscar", "procesos", "verificar", "revisar", "paso_hecho",
                       "ver_pantalla", "git"}


def color(txt: str, c: str) -> str:
    codigos = {"gris": "90", "verde": "32", "rojo": "31", "amarillo": "33", "cian": "36", "negrita": "1"}
    return f"\033[{codigos[c]}m{txt}\033[0m" if sys.stdout.isatty() else txt


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


def llamadas_en_texto(texto: str) -> list:
    """Respaldo para modelos/servidores que escriben la llamada como texto en vez de usar tool_calls."""
    out = []
    for bloque in re.findall(r"<tool_call>(.*?)(?:</tool_call>|$)", texto, re.DOTALL):
        m = re.search(r"<function=([\w.-]+)>(.*?)(?:</function>|$)", bloque, re.DOTALL)
        if m:
            args = {p: v.strip("\n") for p, v in re.findall(r"<parameter=([\w-]+)>\n?(.*?)\n?</parameter>", m.group(2), re.DOTALL)}
            out.append({"name": m.group(1), "arguments": json.dumps(args, ensure_ascii=False)})
            continue
        try:
            d = json.loads(bloque.strip())
            out.append({"name": d["name"], "arguments": json.dumps(d.get("arguments", {}), ensure_ascii=False)})
        except (ValueError, KeyError, TypeError):
            pass
    for c in out:
        c["id"] = uuid.uuid4().hex[:24]
    return out


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
        tokens_herr = len(json.dumps(ESQUEMAS, ensure_ascii=False)) // 3 + 400
        self.entorno = entorno()
        self.ctx = Contexto(ventana, sistema(self.h.raiz, ventana, self.entorno), tokens_herr)
        self.registro = self.h.dir_agente / "registro.log"
        self.h.dir_agente.mkdir(exist_ok=True)
        self._ultimo_progreso = 0.0
        self.pasos_totales = 0
        self.requeridos: set[str] = set()
        self.inicio_tarea = 0
        self.revisiones = 0
        self.h.revisar_cb = self._revisar
        self.emitir = None  # modo --json: emitir(tipo, **datos) publica un evento para el IDE
        self.responder = None  # modo --json: responder(tipo, **datos) espera la respuesta del usuario

    # ---------- utilidades de consola ----------
    def _confirmar(self, comando: str) -> bool:
        if self.responder:
            return bool(self.responder("confirmar", comando=comando).get("si"))
        print(color(f"\n  ¿Permitir comando potencialmente destructivo?\n    {comando}", "amarillo"))
        return input("  (s/n) > ").strip().lower() in ("s", "si", "sí", "y")

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
    def tarea(self, texto: str) -> str:
        self.ctx.usuario(texto)
        self._log(f"USUARIO: {texto}")
        nuevos = archivos_pedidos(texto)
        if nuevos or not re.match(r"\s*(contin|sigue|segu|finaliza|termina)", texto, re.IGNORECASE):
            self.requeridos = nuevos  # 'continúa' conserva los de la tarea en curso
        self.h.nueva_tarea(self.pasos_totales + 1)
        self.inicio_tarea, self.revisiones = self.pasos_totales + 1, 0
        modo = self.cfg["razonamiento"]
        razonar = modo in ("on", "auto")  # planificar con razonamiento
        repeticiones: dict[str, int] = {}
        fallos: dict[str, int] = {}
        rechazos_fin = 0
        empujones = 0
        for paso in range(1, self.cfg["max_pasos"] + 1):
            self.pasos_totales += 1
            self.h.paso = self.pasos_totales  # contador global: válido entre tareas sucesivas
            if self.ctx.hace_falta():
                self._salida("aviso", f"({self.ctx.compactar(self.llm, self._estado())})")
            enviados = list(self.ctx.mensajes)
            try:
                r = self.llm.chat(enviados, ESQUEMAS, self.ctx.max_salida(), razonar, self._progreso)
            except cliente.ErrorLLM as e:
                self._limpiar_linea()
                if "context" in str(e).lower() and len(self.ctx.mensajes) > 3:
                    self._salida("aviso", f"({self.ctx.compactar(self.llm, self._estado(), forzar=True)})")
                    continue
                self._salida("aviso", f"Error del modelo: {e}", "rojo")
                return f"Error del modelo: {e}"
            self._limpiar_linea()
            self.ctx.calibrar((r["usage"] or {}).get("prompt_tokens", 0), enviados)
            llamadas = r["tool_calls"] or llamadas_en_texto(r["content"])
            contenido = re.sub(r"<tool_call>.*?(</tool_call>|$)", "", r["content"], flags=re.DOTALL).strip()
            razonar = modo == "on"

            if not llamadas:
                self.ctx.asistente(contenido, [])
                if r["finish_reason"] == "length":
                    self.ctx.mensajes.append({"role": "user", "content": (
                        "Tu respuesta se cortó por longitud. Divide el trabajo: archivos más pequeños (módulos) "
                        "y una llamada a escribir por archivo.")})
                    continue
                pendiente = "- [ ]" in self.h.plan_texto()
                pregunta = contenido.rstrip().endswith("?")
                if contenido.count("```") >= 2 and contenido.count("\n") > 25 and empujones < 2:
                    empujones += 1  # conducta: pegar código en el chat en vez de escribir archivos
                    self.ctx.mensajes.append({"role": "user", "content": (
                        "No pegues código en el chat: guárdalo con escribir (un archivo por llamada) y sigue el método.")})
                    continue
                if pendiente and not pregunta and empujones < 2:
                    empujones += 1
                    self.ctx.mensajes.append({"role": "user", "content": (
                        "Continúa con el plan usando las herramientas (no describas, actúa). "
                        f"Pasos:\n{self.h.plan_texto()}")})
                    continue
                self._decir(contenido)
                self._log(f"AGENTE: {contenido}")
                return contenido

            empujones = 0
            self.ctx.asistente(contenido, llamadas)
            if contenido:
                self._salida("nota", f"· {contenido[:300]}", "gris")
            hubo_error = False
            for c in llamadas:
                try:
                    args = coaccionar(c["name"], json.loads(c["arguments"] or "{}", strict=False))
                except ValueError as e:
                    resultado = f"Argumentos JSON inválidos ({e}). Repite la llamada con JSON válido."
                    self.ctx.herramienta(c["id"], resultado)
                    self._mostrar(paso, c["name"], {}, resultado)
                    hubo_error = True
                    continue

                clave = c["name"] + json.dumps(args, sort_keys=True, ensure_ascii=False)
                if c["name"] in DEPENDEN_DEL_ESTADO:  # tras modificar archivos, repetir la prueba es progreso
                    clave += f"|{self.h.paso_escritura}"
                repeticiones[clave] = repeticiones.get(clave, 0) + 1

                if c["name"] == "terminar":
                    motivo = self._puede_terminar(rechazos_fin, paso)
                    if motivo:
                        rechazos_fin += 1
                        resultado = motivo
                    else:
                        resumen = str(args.get("resumen", "")).strip()
                        if self.h.verificacion["errores"] > 0:  # salió por la válvula anti-bucle con errores
                            resumen = ("[AVISO: la tarea termina SIN resolver todos los errores de verificación."
                                       f"{self._rescatar()}]\n" + resumen)
                        self.ctx.herramienta(c["id"], "Tarea terminada.")
                        self._mostrar(paso, "terminar", {}, "Tarea terminada")
                        self._guardar_resumen(resumen)
                        self._decir(resumen)
                        self._log(f"FIN: {resumen}")
                        return resumen
                else:
                    resultado = despachar(self.h, c["name"], args)
                    if repeticiones[clave] >= 3:
                        resultado += (f"\n[AVISO: es la {repeticiones[clave]}ª vez que haces exactamente esta llamada. "
                                      "Cambia de estrategia: lee el error, investiga con web_buscar o pregunta al usuario.]")
                    if repeticiones[clave] >= 5:
                        self.ctx.herramienta(c["id"], resultado)
                        msg = f"Detenido: el modelo repitió '{c['name']}' 5 veces sin avanzar." + self._rescatar()
                        self._salida("aviso", msg, "rojo")
                        return msg
                    if c["name"] == "ejecutar":
                        cmd = str(args.get("comando", ""))
                        fallos[cmd] = fallos.get(cmd, 0) + 1 if re.match(r"(exit=[1-9-]|TIMEOUT)", resultado) else 0
                        if fallos[cmd] in (3, 6):
                            resultado += ("\n[ATASCO: este comando falla por " + str(fallos[cmd]) + "ª vez seguida. No reescribas "
                                          "a ciegas: lee el error y el código implicado (leer), escribe en una frase la causa raíz "
                                          "y corrige solo eso con editar. Si no la encuentras, usa web_buscar con el error.]")
                    if c["name"] == "plan" and (olvidados := [f for f in self.requeridos if f.lower() not in resultado.lower()]):
                        resultado += f"\n[AVISO: el usuario pidió {', '.join(sorted(olvidados))} y el plan no lo incluye. Corrige el plan.]"
                    siguiente = next((l[6:] for l in self.h.plan_texto().splitlines() if l.startswith("- [ ]")), "")
                    if siguiente and c["name"] not in ("plan", "paso_hecho"):
                        resultado += f"\n[Plan, paso pendiente: {siguiente}]"
                self._mostrar(paso, c["name"], args, resultado)
                self._log(f"{c['name']} {json.dumps(args, ensure_ascii=False)[:300]} -> {resultado[:300]}")
                hubo_error |= bool(re.match(ES_ERROR, resultado))
                self.ctx.herramienta(c["id"], resultado)
            if hubo_error and modo == "auto":
                razonar = True  # pensar solo cuando hace falta (ahorra tokens)
        msg = f"Se alcanzó el máximo de {self.cfg['max_pasos']} pasos. Escribe 'continúa' para seguir." + self._rescatar()
        self._salida("aviso", msg)
        return msg

    def _revisar(self, requisitos: str = "", enfoque: str = "") -> tuple[str, bool]:
        self.revisiones += 1
        self._salida("nota", "(revisión independiente del código en curso...)", "gris")
        texto, ok = revisar(self.llm, self.h.raiz, requisitos or self.ctx.tarea, self.h.verificacion["informe"], enfoque)
        self.h.dir_agente.mkdir(exist_ok=True)
        (self.h.dir_agente / "revision.json").write_text(json.dumps(
            {"aprobado": ok, "fecha": time.time(), "texto": texto[:5000]}, ensure_ascii=False), encoding="utf-8")
        self._log(f"REVISIÓN ({'aprobada' if ok else 'cambios'}): {texto[:1500]}")
        return texto, ok

    def _puede_terminar(self, rechazos: int, paso: int) -> str:
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
        if not escribio and previa and not previa["aprobado"] and previa["fecha"] >= self._mtime_codigo():
            return ("RECHAZADO: la última revisión pidió cambios y el código no se ha modificado desde entonces. "
                    "Corrige estos puntos con editar, verifica y luego termina:\n" + previa["texto"])
        # 1ª revisión siempre; 2ª si se cambió código después de una revisión que pidió cambios.
        if escribio and (self.revisiones == 0 or (self.revisiones < 2 and previa and not previa["aprobado"]
                                                    and self._mtime_codigo() > previa["fecha"])):
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
        return ""

    def _estado(self) -> str:
        return (f"{self.h.plan_texto() or '(sin plan)'}\n\nARCHIVOS DEL PROYECTO (nombres reales: úsalos tal cual):\n"
                f"{self.h.mapa()}")

    def _rescatar(self) -> str:
        """Si la tarea acaba con el proyecto roto pero hubo una versión que funcionaba, se entrega esa."""
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
        fuentes = [p for p in self.h.raiz.rglob("*") if p.suffix.lower() in (".py", ".js", ".mjs", ".html", ".css", ".java")
                   and not ({".venv", "node_modules", ".agente", ".git", "__pycache__"} & set(p.parts))]
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
