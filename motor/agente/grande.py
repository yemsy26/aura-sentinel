"""Mandato grande: cambios transversales en repos grandes (p. ej. añadir un idioma a todos los módulos) sin que el modelo
pierda el hilo.

1. Plan por partes en un contexto limpio (solo lectura + plan_partes): cada parte <= 15 archivos, con su criterio de
   terminado y su prueba, y unas convenciones comunes.
2. Cada parte corre el bucle normal en un contexto LIMPIO (parte + TAREA + plan + convenciones + mapa del módulo) que
   sustituye a las compactaciones; después, verificación (solo cuentan los errores nuevos), commit local y estado en
   .agente/grande.json para reanudar aunque se cierre el IDE.
3. Verificación final estricta (idiomas completos) y, si queda algo, una parte de cierre.
Los mandatos normales no pasan por aquí.
"""
import hashlib
import json
import re
import subprocess
import time
from pathlib import Path

from . import mapa
from .contexto import Contexto
from .herramientas import ESQUEMA_MAPA, ESQUEMA_PLAN_PARTES, ESQUEMAS, NO_VERSIONAR, recortar
from .verificacion import informe_proyecto, normalizar

EXPLICITO = re.compile(r"^\s*/grande\b|\bmodo grande\b", re.IGNORECASE)
TRANSVERSAL = re.compile(
    r"\btod[oa]s\s+(l[oa]s\s+)?(m[oó]dulos|archivos|pantallas|vistas|p[aá]ginas|componentes|servicios|carpetas|"
    r"secciones|men[uú]s|textos)\b|\btod[oa]\s+(la\s+app|la\s+aplicaci[oó]n|el\s+proyecto|el\s+c[oó]digo|el\s+sistema|"
    r"el\s+repo(sitorio)?|la\s+web)\b|\ben\s+todo\s+el\s+(proyecto|c[oó]digo|sistema|repo)|"
    r"\b(a[ñn]ad|agreg|pon|traduc|incorpor|soporta)\w*\b.{0,40}\b(idiomas?|ingl[eé]s|franc[eé]s|alem[aá]n|portugu[eé]s|"
    r"italiano|catal[aá]n|chino|japon[eé]s)\b|internacionaliz|\bi18n\b|multi-?idioma", re.IGNORECASE)
REANUDAR = re.compile(r"\s*(/grande\s+)?(contin|sigue|segu|retoma|reanuda|resume)", re.IGNORECASE)
LECTURA = ("mapa", "listar", "leer", "buscar")
MAX_ARCHIVOS, PASOS_PLAN, PASOS_PARTE, INTENTOS = 15, 15, 50, 2
DOCS = ("ARQUITECTURA.md", "CONVENCIONES.md", "ARCHITECTURE.md", "CONVENTIONS.md")
PLAN = """MODO GRANDE — FASE DE PLAN (en esta fase no se modifica nada).
Mandato del usuario: {tarea}

Divide el trabajo en PARTES que se puedan hacer y probar por separado, normalmente una por módulo y en orden: primero \
lo común (p. ej. la infraestructura de idiomas o las utilidades compartidas) y después cada módulo. Cada parte lleva \
'titulo', 'archivos' (rutas reales del mapa, 15 como máximo y mejor 4-10; incluye los nuevos y los compartidos que \
tocará), 'terminado' (criterio comprobable) y 'prueba' (los tests de ese módulo o verificar). En 'convenciones' escribe \
las decisiones comunes que deben seguir todas las partes: nombres, formato y ubicación (p. ej. claves de idioma \
'modulo.clave' en locales/en.json) y estilo.
Usa el mapa; si te falta detalle, mapa(ruta), buscar o leer (solo lo imprescindible). Después llama a plan_partes."""
INSTRUCCIONES = ("Trabaja SOLO en esta parte: no hagas las otras. Toca sus archivos y, si hace falta, los compartidos "
                 "(p. ej. el archivo de idioma) sin borrar lo que pusieron otras partes. Sigue las convenciones. Lee "
                 "(leer) lo que vas a cambiar antes de editarlo y corrige con editar. Haz un plan corto de ESTA parte "
                 "(3-6 pasos), verifica y termina con un resumen de una línea. No crees .bat ni README salvo que la "
                 "parte lo pida. Atiende también los AVISOS de verificar sobre tus archivos (p. ej. textos fijos sin "
                 "traducir). Los errores PREEXISTENTES de verificar no son de esta parte: no los toques.")


# ---------------------------------------------------------------- estado
def archivo_estado(dir_agente: Path) -> Path:
    return dir_agente / "grande.json"


def cargar(dir_agente: Path) -> dict | None:
    try:
        return json.loads(archivo_estado(dir_agente).read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None


def guardar(dir_agente: Path, estado: dict) -> None:
    dir_agente.mkdir(exist_ok=True)
    tmp = archivo_estado(dir_agente).with_suffix(".tmp")
    tmp.write_text(json.dumps(estado, ensure_ascii=False, indent=1), encoding="utf-8")
    tmp.replace(archivo_estado(dir_agente))


def modo(texto: str, repo_grande: bool, dir_agente: Path) -> str:
    """'reanudar', 'nuevo' o '' (camino normal). Un mandato normal aparca el modo grande en curso."""
    estado = cargar(dir_agente)
    if estado and REANUDAR.match(texto):
        if estado.get("estado") in ("en_curso", "detenido") or (estado.get("estado") == "aparcado"
                                                                and EXPLICITO.search(texto)):
            return "reanudar"
        return ""  # 'continúa' de un mandato normal
    if EXPLICITO.search(texto) or (repo_grande and TRANSVERSAL.search(texto)):
        return "nuevo"
    if estado and estado.get("estado") in ("en_curso", "detenido"):
        estado["estado"] = "aparcado"
        guardar(dir_agente, estado)
        plan = dir_agente / "plan.md"  # la lista de partes no debe frenar al mandato nuevo
        if plan.exists() and ". Parte: " in plan.read_text(encoding="utf-8"):
            plan.replace(dir_agente / "plan_anterior.md")
    return ""


# ---------------------------------------------------------------- git
def preparar_git(h) -> str:
    """'repo', 'init' o 'sin git: motivo'. Nunca crea un repo dentro de otro (subcarpeta de un monorepo)."""
    try:
        if h._git("rev-parse", "--show-toplevel")[0] == 0:
            return "repo"
        r = subprocess.run(["git", "rev-parse", "--show-toplevel"], cwd=h.raiz, capture_output=True, text=True,
                           timeout=30, creationflags=subprocess.CREATE_NO_WINDOW, check=False)
        if r.returncode == 0:
            return f"sin git: la carpeta está dentro de otro repositorio ({r.stdout.strip()}); no hago commits"
        codigo, salida = h._git("init")
        if codigo != 0:
            return f"sin git: {salida[:200]}"
        h.git("commit", mensaje="Estado inicial (antes del modo grande)")  # repo nuevo: cada parte será su diferencia
        return "init"
    except (OSError, subprocess.SubprocessError) as e:
        return f"sin git: {e}"


def _sucios(h) -> dict[str, str]:
    """{ruta: huella} de lo que difiere del último commit (incluye archivos nuevos)."""
    try:
        codigo, salida = h._git("status", "--porcelain", "--untracked-files=all", *NO_VERSIONAR)
    except (OSError, subprocess.SubprocessError):
        return {}
    if codigo != 0:
        return {}
    out = {}
    for linea in salida.splitlines():
        m = re.match(r"[ MADRCU?!]{1,2} (.+)$", linea)
        if not m:
            continue
        ruta = m.group(1).split(" -> ")[-1].strip('"')
        p = h.raiz / ruta
        out[ruta] = hashlib.sha1(p.read_bytes()).hexdigest() if p.is_file() else "borrado"
    return out


def _commit(h, estado: dict, mensaje: str, antes: dict[str, str]) -> str:
    """Commit local SOLO de lo que cambió durante la parte (el trabajo a medias del usuario no se mezcla)."""
    if estado.get("git") not in ("repo", "init"):
        return ""
    cambiados = sorted(p for p, huella in _sucios(h).items() if antes.get(p) != huella)
    if not cambiados:
        return "sin cambios"
    codigo, salida = h._git("add", "-A", "--", *cambiados)
    if codigo == 0:
        codigo, salida = h._git("commit", "-m", mensaje, "--", *cambiados)
    if codigo != 0:
        return f"sin commit: {(salida.strip().splitlines() or ['?'])[-1][:150]}"
    return h._git("rev-parse", "--short", "HEAD")[1].strip()


# ---------------------------------------------------------------- plan
def documentos(raiz: Path) -> str:
    partes = []
    for nombre in DOCS:
        for p in (raiz / nombre, raiz / "docs" / nombre):
            if p.is_file():
                texto = p.read_text(encoding="utf-8", errors="replace").strip()
                partes.append(f"{p.relative_to(raiz).as_posix()}:\n{texto[:1500]}{' [...]' if len(texto) > 1500 else ''}")
                break
    return "\n\n".join(partes)


def _mapa_plan(h, maximo: int = 8000) -> str:
    """Resumen por módulo y, mientras quepa, el detalle de cada módulo (así el plan suele salir en una llamada)."""
    texto = h.mapa_cmd()
    for nombre, _ in mapa.modulos(list(mapa.actualizar(h.raiz))):
        detalle = mapa.texto(h.raiz, nombre if nombre != "./" else "", 3000) if nombre != "./" else ""
        if detalle and len(texto) + len(detalle) < maximo:
            texto += "\n\n" + detalle
    return texto


def _norma(ruta) -> str:
    return str(ruta).strip().strip("\"'").replace("\\", "/").removeprefix("./").strip("/")


def validar(h, args: dict, rechazos: int) -> dict | str:
    """El estado inicial del modo grande, o el motivo del rechazo para que el modelo corrija el plan."""
    partes = args.get("partes")
    if isinstance(partes, str):
        try:
            partes = json.loads(partes)
        except ValueError:
            partes = None
    if not isinstance(partes, list) or not partes:
        return "RECHAZADO: 'partes' debe ser una lista de partes con titulo, archivos, terminado y prueba."
    existentes = list(mapa.actualizar(h.raiz))
    errores, limpias = [], []
    for i, p in enumerate(partes, 1):
        if not isinstance(p, dict):
            errores.append(f"parte {i}: debe ser un objeto con titulo, archivos, terminado y prueba")
            continue
        titulo = str(p.get("titulo") or f"Parte {i}").strip()[:120]
        crudos = p.get("archivos") or []
        crudos = [a for a in re.split(r"[,\n]", crudos)] if isinstance(crudos, str) else crudos
        archivos = []
        for a in dict.fromkeys(_norma(a) for a in crudos if _norma(a)):
            try:
                destino = h.ruta(a)
            except ValueError:
                errores.append(f"parte {i}: '{a}' está fuera del proyecto")
                continue
            if destino.is_dir():  # una carpeta = sus archivos de código
                archivos += [e for e in existentes if e.startswith(a + "/")]
                continue
            if not destino.exists() and (parecidos := [e for e in existentes if e.rsplit("/", 1)[-1] == a.rsplit("/", 1)[-1]]):
                errores.append(f"parte {i}: '{a}' no existe; ¿quisiste decir '{parecidos[0]}'?")
            archivos.append(a)
        archivos = list(dict.fromkeys(archivos))
        faltan = [c for c in ("terminado", "prueba") if not str(p.get(c) or "").strip()]
        if faltan:
            errores.append(f"parte {i} ({titulo}): falta {' y '.join(faltan)}")
        if not archivos:
            errores.append(f"parte {i} ({titulo}): sin archivos")
        elif len(archivos) > MAX_ARCHIVOS:
            errores.append(f"parte {i} ({titulo}): {len(archivos)} archivos; divídela en partes de {MAX_ARCHIVOS} como máximo")
        limpias.append({"titulo": titulo, "archivos": archivos,
                        "terminado": str(p.get("terminado") or "verificar sin errores nuevos").strip()[:400],
                        "prueba": str(p.get("prueba") or "verificar").strip()[:300]})
    if errores and rechazos < 3:
        return "RECHAZADO: corrige el plan y vuelve a llamar a plan_partes:\n- " + "\n- ".join(errores[:12])
    finales = []  # tras 3 rechazos se arregla solo: las partes enormes se trocean y las vacías se descartan
    for p in limpias:
        for j in range(0, len(p["archivos"]), MAX_ARCHIVOS):
            trozo = p["archivos"][j:j + MAX_ARCHIVOS]
            sufijo = f" ({j // MAX_ARCHIVOS + 1})" if len(p["archivos"]) > MAX_ARCHIVOS else ""
            finales.append({**p, "titulo": p["titulo"] + sufijo, "archivos": trozo})
    if not finales:
        return "RECHAZADO: ninguna parte tiene archivos."
    for p in finales:
        p.update(estado="pendiente", intentos=0, pasos=0, segundos=0, resumen="", commit="", motivo="")
    return {"version": 1, "estado": "en_curso", "creado": time.strftime("%Y-%m-%d %H:%M"),
            "convenciones": str(args.get("convenciones") or "").strip()[:2000], "partes": finales}


def planificar(agente, tarea: str) -> dict | str:
    from .bucle import coaccionar, llamadas_en_texto, sin_llamadas
    from .llm import ErrorLLM
    h = agente.h
    docs = documentos(h.raiz)
    usuario = PLAN.format(tarea=tarea) + (f"\n\nDOCUMENTACIÓN DEL PROYECTO:\n{docs}" if docs else "") + \
        f"\n\nMAPA:\n{_mapa_plan(h)}"
    ctx = Contexto(agente.llm.contexto, agente.ctx.mensajes[0]["content"], agente.tokens_herr)
    ctx.usuario(usuario)
    lectura = [ESQUEMA_MAPA] + [e for e in ESQUEMAS if e["function"]["name"] in LECTURA[1:]]
    rechazos, razonar, solo_plan = 0, agente.cfg["razonamiento"] in ("on", "auto"), False
    for i in range(PASOS_PLAN):
        agente.pasos_totales += 1
        h.paso = agente.pasos_totales
        if not solo_plan and (ctx.hace_falta() or i >= PASOS_PLAN - 2):  # sin más lecturas: a planificar
            solo_plan = True
            ctx.mensajes.append({"role": "user", "content": "Ya tienes suficiente información: llama AHORA a plan_partes."})
        try:
            r = agente.llm.chat(list(ctx.mensajes), [ESQUEMA_PLAN_PARTES] if solo_plan else lectura + [ESQUEMA_PLAN_PARTES],
                                ctx.max_salida(), razonar, agente._progreso)
        except ErrorLLM as e:
            agente._limpiar_linea()
            return f"Error del modelo: {e}"
        agente._limpiar_linea()
        ctx.calibrar((r["usage"] or {}).get("prompt_tokens", 0), ctx.mensajes)
        razonar = agente.cfg["razonamiento"] == "on"
        llamadas = r["tool_calls"] or llamadas_en_texto(r["content"])
        contenido = (sin_llamadas(r["content"]) if llamadas and not r["tool_calls"]
                     else re.sub(r"<tool_call>.*?(</tool_call>|$)", "", r["content"], flags=re.DOTALL).strip())
        ctx.asistente(contenido, llamadas)
        if not llamadas:
            ctx.mensajes.append({"role": "user", "content": "Responde llamando a plan_partes (no escribas el plan en el chat)."})
            continue
        for c in llamadas:
            try:
                args = coaccionar(c["name"], json.loads(c["arguments"] or "{}", strict=False))
            except ValueError as e:
                args, resultado = {}, f"Argumentos JSON inválidos ({e}). Repite la llamada con JSON válido."
            else:
                if c["name"] == "plan_partes":
                    estado = validar(h, args, rechazos)
                    if isinstance(estado, dict):
                        estado["tarea"] = tarea
                        resultado = "Plan aceptado:\n" + listado(estado)
                        agente._mostrar(h.paso, c["name"], args, resultado)
                        agente._log(f"plan_partes -> {resultado[:600]}")
                        return estado
                    rechazos += 1
                    resultado = estado
                elif c["name"] in LECTURA:
                    from .herramientas import despachar
                    resultado = despachar(h, c["name"], args)
                else:
                    resultado = ("En la fase de plan solo se lee: usa mapa, listar, leer o buscar y termina con "
                                 "plan_partes.")
                    razonar = True
            agente._mostrar(h.paso, c["name"], args, resultado)
            ctx.herramienta(c["id"], resultado)
    return f"Detenido: el modelo no entregó un plan por partes válido en {PASOS_PLAN} pasos."


def listado(estado: dict, actual: int = 0) -> str:
    return "\n".join(f"[{'x' if p['estado'] == 'hecha' else '>' if k == actual else ' '}] {k}. {p['titulo']} "
                     f"({len(p['archivos'])} archivos)" for k, p in enumerate(estado["partes"], 1))


# ---------------------------------------------------------------- partes
def prompt_parte(h, estado: dict, k: int) -> str:
    parte, n = estado["partes"][k - 1], len(estado["partes"])
    archivos = ", ".join(a + ("" if (h.raiz / a).exists() else " (nuevo)") for a in parte["archivos"])
    lineas = [f"MODO GRANDE — PARTE {k}/{n}: {parte['titulo']}",
              f"Archivos de esta parte: {archivos or '(los que indique la verificación)'}",
              f"Terminada cuando: {parte['terminado']}", f"Prueba: {parte['prueba']}", ""]
    if parte["intentos"] > 1:
        lineas.append(f"INTENTO {parte['intentos']}: el anterior no terminó ({parte['motivo'][:500] or 'se interrumpió'}). "
                      "Puede haber cambios a medias sin commit: míralos con git(accion='diff') antes de seguir.\n")
    lineas += [INSTRUCCIONES, "", f"TAREA GLOBAL (contexto; las otras partes hacen el resto): {estado['tarea'][:2000]}"]
    if estado.get("convenciones"):
        lineas.append(f"CONVENCIONES (comunes a todas las partes): {estado['convenciones']}")
    if docs := documentos(h.raiz):
        lineas.append(docs)
    lineas.append(f"PARTES:\n{listado(estado, k)}")
    hechas = [f"- Parte {i}: {p['resumen']}" for i, p in enumerate(estado["partes"], 1) if p["estado"] == "hecha"]
    if hechas:
        lineas.append("HECHO EN PARTES ANTERIORES:\n" + "\n".join(hechas[-8:]))
    rutas = sorted({a.rsplit("/", 1)[0] if "/" in a else a for a in parte["archivos"]})
    mapa_modulo = "\n".join(mapa.texto(h.raiz, r, 1500) for r in rutas[:5])
    lineas.append(f"MAPA DEL MÓDULO:\n{recortar(mapa_modulo, 2500)}")
    return "\n".join(lineas)


def _requisitos_revision(estado: dict, k: int) -> str:
    """Lo que la revisión independiente debe exigir: ESTA parte, no el mandato entero."""
    parte = estado["partes"][k - 1]
    return (f"PARTE {k}/{len(estado['partes'])} de un cambio grande; las demás partes hacen el resto: NO las exijas.\n"
            f"Objetivo de esta parte: {parte['titulo']}\nArchivos: {', '.join(parte['archivos'])}\n"
            f"Terminada cuando: {parte['terminado']}\nPrueba: {parte['prueba']}\n"
            f"Convenciones: {estado.get('convenciones') or '-'}\nMandato completo (solo contexto): {estado['tarea'][:1200]}")


def _una_parte(agente, estado: dict, k: int) -> tuple[bool, str]:
    from .bucle import INCOMPLETA
    h, parte, n = agente.h, estado["partes"][k - 1], len(estado["partes"])
    quedan_despues = any(p["estado"] != "hecha" for p in estado["partes"][k:])
    h.errores_previos = set(estado.get("errores_previos", []))
    h.idiomas_parcial = parte["archivos"] if quedan_despues else None
    if h.archivo_plan.exists():  # el plan de la parte empieza limpio (el de otra parte no debe frenarla)
        h.archivo_plan.replace(h.dir_agente / "plan_anterior.md")
    if "sucios_antes" not in parte:
        parte["sucios_antes"] = _sucios(h) if estado.get("git") in ("repo", "init") else {}
        guardar(h.dir_agente, estado)
    nuevos = {a for a in parte["archivos"] if not (h.raiz / a).exists()}
    agente.nuevo_contexto()
    agente.requisitos_parte = _requisitos_revision(estado, k)
    agente._salida("aviso", f"Modo grande — parte {k}/{n}: {parte['titulo']} ({len(parte['archivos'])} archivos, "
                            "contexto limpio)")
    t0, pasos0 = time.time(), agente.pasos_totales
    try:
        respuesta = agente.tarea(prompt_parte(h, estado, k), max_pasos=min(PASOS_PARTE, agente.cfg["max_pasos"]),
                                 requeridos=nuevos, etiqueta=f"PARTE {k}/{n}: {parte['titulo']}")
    finally:
        agente.requisitos_parte = ""
        parte["pasos"] += agente.pasos_totales - pasos0
        parte["segundos"] += round(time.time() - t0)
    if INCOMPLETA.match(respuesta):
        return False, respuesta
    if faltan := sorted(a for a in nuevos if not (h.raiz / a).exists()):
        return False, f"la parte debía crear {', '.join(faltan)} y no existe"
    v = h.verificacion  # vale la última si es del proyecto entero, posterior al último cambio y sin errores
    if not (v["paso"] >= max(h.paso_escritura, h.inicio_tarea) and v["errores"] == 0 and v.get("ruta", ".") in ("", ".")):
        informe = h.verificar_cmd()
        agente._mostrar(h.paso, "verificar", {}, informe)
    if h.verificacion["errores"] > 0:
        return False, "la verificación tiene errores nuevos:\n" + h.verificacion["informe"][:1500]
    parte["resumen"] = " ".join(respuesta.split())[:280]
    parte["commit"] = _commit(h, estado, f"Parte {k}/{n}: {parte['titulo']}", parte["sucios_antes"])
    return True, ""


def _rutas_en(informe: str, h) -> list[str]:
    rutas = re.findall(r"(?:^|[\s(])([\w./\\-]+\.\w{1,6})(?=[:\s)\[]|$)", informe, re.MULTILINE)
    return list(dict.fromkeys(_norma(r) for r in rutas if (h.raiz / _norma(r)).is_file()))[:MAX_ARCHIVOS]


def ejecutar(agente, texto: str, como: str) -> str:
    h = agente.h
    agente.esperar_mapa()
    agente._log(f"USUARIO: {texto}")
    if como == "nuevo":
        tarea = re.sub(r"^\s*/grande\b", "", texto, flags=re.IGNORECASE).strip() or texto
        previo = archivo_estado(h.dir_agente)
        if previo.exists():
            previo.replace(h.dir_agente / "grande_anterior.json")
        agente._salida("aviso", "Modo grande: primero un plan por partes (contexto limpio, solo lectura).")
        t0, pasos0 = time.time(), agente.pasos_totales
        estado = planificar(agente, tarea)
        if isinstance(estado, str):
            agente._salida("aviso", estado, "rojo")
            return estado
        estado["pasos_plan"], estado["segundos_plan"] = agente.pasos_totales - pasos0, round(time.time() - t0)
        guardar(h.dir_agente, estado)
        agente._salida("aviso", f"Plan por partes:\n{listado(estado)}")
    else:
        estado = cargar(h.dir_agente)
        if estado.get("estado") == "detenido":  # el usuario pidió seguir: nuevos intentos para lo que falta
            for p in estado["partes"]:
                if p["estado"] != "hecha":
                    p["intentos"] = 0
        estado["estado"] = "en_curso"
        guardar(h.dir_agente, estado)
        agente._salida("aviso", f"Modo grande: reanudo.\n{listado(estado)}")
    try:
        return _partes(agente, estado)
    finally:
        h.errores_previos = h.idiomas_parcial = None


def _partes(agente, estado: dict) -> str:
    h = agente.h
    if "git" not in estado:
        estado["git"] = preparar_git(h)
    if "errores_previos" not in estado:
        agente._salida("nota", "(modo grande: verificación inicial para separar los errores que ya había...)", "gris")
        inf = informe_proyecto(h.raiz, h.python_proyecto())
        estado["errores_previos"] = sorted({normalizar(e) for e in inf.errores}) if not isinstance(inf, str) else []
        guardar(h.dir_agente, estado)
    while True:
        k = next((i for i, p in enumerate(estado["partes"], 1) if p["estado"] != "hecha"), None)
        if k is None:
            h.errores_previos, h.idiomas_parcial = set(estado["errores_previos"]), None
            informe = h.verificar_cmd()  # final y estricta: idiomas completos
            agente._mostrar(h.paso, "verificar", {}, informe)
            if h.verificacion["errores"] == 0 or any(p.get("cierre") for p in estado["partes"]):
                return _final(agente, estado)
            errores = informe.split("ERRORES (corrígelos todos):")[-1].split("\nAVISOS")[0].strip()
            estado["partes"].append({"titulo": "Cierre: corregir la verificación final", "archivos": _rutas_en(informe, h),
                                     "terminado": f"verificar sin errores nuevos (idiomas completos). Errores de la "
                                                  f"verificación final:\n{errores[:1500]}", "prueba": "verificar",
                                     "estado": "pendiente", "intentos": 0, "pasos": 0, "segundos": 0, "resumen": "",
                                     "commit": "", "motivo": "", "cierre": True})
            guardar(h.dir_agente, estado)
            continue
        parte = estado["partes"][k - 1]
        if parte["intentos"] >= INTENTOS:
            estado["estado"] = "detenido"
            guardar(h.dir_agente, estado)
            _plan_global(h, estado)
            texto = (f"Detenido: la parte {k}/{len(estado['partes'])} ({parte['titulo']}) no quedó terminada tras "
                     f"{INTENTOS} intentos: {parte['motivo'][:600]}\nLas partes hechas tienen su commit; el estado queda "
                     "en .agente/grande.json. Escribe 'continúa' para reintentarla.")
            _seguir_conversacion(agente, estado, texto)
            agente._salida("aviso", texto, "rojo")
            return texto
        parte["intentos"] += 1
        guardar(h.dir_agente, estado)
        ok, motivo = _una_parte(agente, estado, k)
        if ok:
            parte["estado"], parte["motivo"] = "hecha", ""
            agente._salida("aviso", f"Parte {k}/{len(estado['partes'])} hecha" +
                           (f" (commit {parte['commit']})" if parte["commit"] else ""))
        else:
            parte["motivo"] = " ".join(motivo.split())[:800]
        guardar(h.dir_agente, estado)


def _plan_global(h, estado: dict) -> None:
    """plan.md con las partes: el IDE lo muestra como misión pendiente y ofrece continuar."""
    h.archivo_plan.write_text("# Plan\n" + "".join(f"- [{'x' if p['estado'] == 'hecha' else ' '}] {k}. Parte: {p['titulo']}\n"
                                                   for k, p in enumerate(estado["partes"], 1)), encoding="utf-8")


def _seguir_conversacion(agente, estado: dict, resultado: str) -> None:
    """La conversación sigue con el mandato y su resultado, no con el contexto de la última parte."""
    agente.nuevo_contexto()
    agente.ctx.usuario(estado["tarea"])
    agente.ctx.asistente(resultado, [])


def _final(agente, estado: dict) -> str:
    h, n = agente.h, len(estado["partes"])
    errores = h.verificacion["errores"]
    pasos = sum(p["pasos"] for p in estado["partes"]) + estado.get("pasos_plan", 0)
    minutos = (sum(p["segundos"] for p in estado["partes"]) + estado.get("segundos_plan", 0)) / 60
    lineas = [(f"Modo grande terminado: {n} partes, {pasos} pasos, {minutos:.0f} min; verificación final "
               f"{'sin errores nuevos' if errores == 0 else f'con {errores} errores nuevos'}.")]
    if errores:
        lineas.insert(0, "[AVISO: la tarea termina SIN resolver todos los errores de verificación.]")
    for k, p in enumerate(estado["partes"], 1):
        lineas.append(f"- Parte {k}/{n} {p['titulo']}: {p['resumen'][:200]}"
                      + (f" (commit {p['commit']})" if p["commit"] else ""))
    if estado["git"] not in ("repo", "init"):
        lineas.append(f"Sin commits ({estado['git'].removeprefix('sin git: ')}).")
    lineas.append("Detalle en .agente/grande.json.")
    resumen = "\n".join(lineas)
    estado["estado"] = "terminado"
    guardar(h.dir_agente, estado)
    if h.archivo_plan.exists():
        h.archivo_plan.replace(h.dir_agente / "plan_anterior.md")
    (h.dir_agente / "resumen.md").write_text(f"# Resumen ({time.strftime('%Y-%m-%d %H:%M')})\n\n## Tarea\n{estado['tarea']}"
                                             f"\n\n## Resultado\n{resumen}\n", encoding="utf-8")
    _seguir_conversacion(agente, estado, resumen)
    agente._decir(resumen)
    agente._log(f"FIN: {resumen}")
    return resumen
