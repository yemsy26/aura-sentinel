"""Usa de verdad la web de un proyecto Python (Flask, FastAPI, http.server): la arranca sobre una copia, entra con los
usuarios que da el proyecto, abre sus páginas y envía cada formulario con datos válidos, como haría quien la usa.

Una misión de negocio entregó un sistema que daba error 500 al registrar una venta (db.get_or_404 con dos argumentos):
sus 12 tests pasaban, la revisión lo aprobó y verificar solo comprobaba que el servidor arrancaba. Aquí no juzga el
modelo: cuenta lo que responde el servidor (500, una traza, un formulario que va a una ruta que no existe)."""
import html.parser
import http.cookiejar
import json
import os
import re
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from collections import deque
from datetime import date
from pathlib import Path

from . import procesos

ARRANQUE = Path(__file__).resolve().parent / "sondas" / "web_arranque.py"
# Ni se siguen ni se envían: cerrarían la sesión o borrarían los datos con los que se prueba lo demás.
NO_TOCAR = re.compile(r"(?i)log-?out|salir|cerrar[-_ ]?sesi|sign[-_ ]?out|eliminar|borrar|delete|remove|quitar|vaciar|"
                      r"reset|restablecer|desactivar|anular|cancelar")
ESTATICO = re.compile(r"(?i)\.(?:css|js|mjs|png|jpe?g|gif|svg|ico|webp|woff2?|ttf|map|pdf|xlsx?|csv|zip|txt)(?:\?|$)")
NUMERO = re.compile(r"(?i)precio|importe|cantidad|stock|monto|total|price|qty|amount|unidades|coste|costo|minimo|"
                    r"m[ií]nimo|edad|num|n[uú]mero|duraci|valor|saldo|pago|descuento|porcentaje|iva")
# value="{{ producto.nombre }}" sin producto pinta «None»: nadie lo enviaría así, lo cambiaría por un dato.
VACIO = re.compile(r"(?i)^\s*(?:none|null|undefined|nan)\s*$")
LIBRERIA = re.compile(r"[\\/](?:site-packages|Lib|lib[\\/]python[\d.]*)[\\/]|web_arranque\.py|File \"<")
# Números con sentido para el negocio: precio de venta mayor que el de compra, stock para poder vender después.
POR_NOMBRE = ((r"descuento|discount|iva|impuesto|tax|porcentaje|recargo", 0.0), (r"compra|coste|costo|cost", 10.0),
              (r"m[ií]nimo|min", 5.0), (r"stock|existencia|inventario", 20.0),
              (r"venta|pvp|sale|precio|price|importe|monto|amount|total|pago", 15.0))
CERO_VALE = re.compile(POR_NOMBRE[0][0])  # en un descuento, 0 es lo normal
MAX_PAGINAS, MAX_FORMULARIOS, MAX_SEGUNDOS = 60, 25, 75


class _Pagina(html.parser.HTMLParser):
    """Enlaces y formularios (con sus campos y las opciones de cada select) de una página."""

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.enlaces, self.formularios = [], []
        self._form = self._select = self._opcion = None

    def handle_starttag(self, tag, attrs):
        a = {k: v or "" for k, v in attrs}
        if tag == "a" and a.get("href"):
            self.enlaces.append(a["href"])
        elif tag == "form":
            self._form = {"accion": a.get("action", ""), "metodo": (a.get("method") or "get").lower(), "campos": []}
            self.formularios.append(self._form)
        elif self._form is None:
            return
        elif tag in ("input", "textarea", "button"):
            tipo = a.get("type", "text" if tag == "input" else "submit" if tag == "button" else tag).lower()
            self._form["campos"].append({**a, "tipo": tipo})
        elif tag == "select":
            self._select = {**a, "tipo": "select", "opciones": []}
            self._form["campos"].append(self._select)
        elif tag == "option" and self._select is not None:
            self._opcion = {"valor": a.get("value"), "texto": "", "desactivada": "disabled" in a}
            self._select["opciones"].append(self._opcion)

    def handle_data(self, data):
        if self._opcion is not None:
            self._opcion["texto"] += data

    def handle_endtag(self, tag):
        if tag == "form":
            self._form = None
        elif tag == "select":
            self._select = None
        elif tag == "option":
            self._opcion = None


def _valor(campo: dict, hoy: str) -> str | None:
    """Lo que escribiría alguien que usa bien la web; None si el campo no se envía."""
    tipo, nombre = campo["tipo"], (campo.get("name") or "").lower()
    if tipo in ("submit", "button", "image", "reset", "file"):
        return None
    if tipo == "select":
        opciones = [o for o in campo["opciones"] if not o["desactivada"]]
        valores = [o["texto"].strip() if o["valor"] is None else o["valor"] for o in opciones]
        return next((v for v in valores if v.strip()), valores[0] if valores else "")
    if tipo in ("hidden", "checkbox", "radio"):
        return campo.get("value") or ("on" if tipo != "hidden" else "")
    numerico = tipo in ("number", "range") or (tipo == "text" and NUMERO.search(nombre))
    cero = numerico and re.fullmatch(r"\s*0+(?:[.,]0+)?\s*", campo.get("value", "")) and not CERO_VALE.search(nombre)
    if campo.get("value") and not VACIO.match(campo["value"]) and not cero:
        return campo["value"]  # formularios de editar: lo que ya tiene
    if numerico:  # stock inicial value="0": quien da de alta un producto pone el que tiene
        valor = next((v for patron, v in POR_NOMBRE if re.search(patron, nombre)), 1.0)
        try:
            valor = max(valor, float(campo.get("min") or 0))
            valor = min(valor, float(campo["max"])) if campo.get("max") else valor
        except ValueError:
            pass
        return str(valor).removesuffix(".0")
    rellenos = {"date": campo.get("min") or hoy, "time": campo.get("min") or "10:00", "datetime-local": f"{hoy}T10:00",
                "month": hoy[:7], "week": f"{hoy[:4]}-W10", "email": "prueba@example.com", "tel": "600000000",
                "url": "http://example.com", "color": "#336699", "password": "Prueba1234!"}
    if tipo in rellenos:
        return rellenos[tipo]
    for patron, valor in ((r"fecha|date|dia\b|d[ií]a\b", hoy), (r"hora|time", "10:00"), (r"mail|correo", "prueba@example.com"),
                          (r"tel[eé]fono|phone|m[oó]vil|celular", "600000000")):
        if re.search(patron, nombre):
            return valor
    return "Prueba"


def _rellenar(form: dict, hoy: str, usuario: str | None = None, clave: str | None = None) -> list[tuple[str, str]] | None:
    """Los datos que enviaría el navegador; None si no dejaría enviarlo (un campo obligatorio sin nada que poner, como un
    select de productos cuando aún no hay ninguno)."""
    datos, radios, boton = [], set(), None
    for c in form["campos"]:
        nombre = c.get("name")
        if not nombre or "disabled" in c:
            continue
        if c["tipo"] in ("submit", "image") and boton is None and c.get("value") is not None and not NO_TOCAR.search(
                f"{nombre} {c.get('value', '')}"):
            boton = (nombre, c.get("value", ""))
        if c["tipo"] == "radio":
            if nombre in radios:
                continue
            radios.add(nombre)
        valor = clave if (clave is not None and c["tipo"] == "password") else _valor(c, hoy)
        if usuario is not None and c["tipo"] in ("text", "email") and not any(d[1] == usuario for d in datos):
            valor = usuario
        if valor == "" and "required" in c and c["tipo"] != "hidden":
            return None
        if valor is not None:
            datos.append((nombre, valor))
    return datos + ([boton] if boton else [])


def _es_entrar(form: dict) -> bool:
    visibles = [c for c in form["campos"] if c["tipo"] not in ("hidden", "submit", "button", "image", "checkbox")]
    return any(c["tipo"] == "password" for c in visibles) and len(visibles) <= 3


def _forma(ruta: str) -> str:
    """/productos/7/editar?pagina=2 -> /productos/{n}/editar?pagina: las páginas de cada producto son la misma."""
    camino, _, consulta = ruta.partition("?")
    camino = re.sub(r"/\d+(?=/|$)", "/{n}", camino)
    return camino + ("?" + "&".join(sorted(k for k, _ in urllib.parse.parse_qsl(consulta))) if consulta else "")


def _libre() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _traza(texto: str) -> str:
    """La última traza de Python del servidor: las líneas del proyecto (no las de las librerías) y el error."""
    i = texto.rfind("Traceback (most recent call last)")
    if i < 0:
        return ""
    lineas, marcos, final = texto[i:].splitlines()[1:], [], ""
    for j, linea in enumerate(lineas):
        if linea.startswith('  File "'):
            if not LIBRERIA.search(linea):
                siguiente = lineas[j + 1] if j + 1 < len(lineas) else ""
                marcos.append(linea.strip() + (f"\n      {siguiente.strip()}" if siguiente.startswith("    ") else ""))
        elif linea and not linea.startswith(" "):
            final = linea
            break
    return "\n".join([*marcos[-4:], final]).strip()


class _Recorrido:
    def __init__(self, base: str, log: Path, accesos: list[tuple[str, str]]):
        self.base, self.log, self.accesos, self.hoy = base, log, accesos, date.today().isoformat()
        self.abridor = urllib.request.build_opener(urllib.request.ProxyHandler({}),
                                                   urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
        self.errores, self.rotos, self.raros, self.enviados = [], [], [], []
        self.paginas = self.formularios = 0
        self.usuario: str | None = None
        self.intento_entrar = self.entrar_js = False

    def pedir(self, ruta: str, datos: list | None = None, metodo: str = "get") -> tuple[int, str, str]:
        """(estado, url final, html). Cada petición deja su salida del servidor en self.salida."""
        antes = self.log.stat().st_size if self.log.exists() else 0
        url = urllib.parse.urljoin(self.base, ruta)
        cuerpo = urllib.parse.urlencode(datos or []).encode() if metodo == "post" else None
        if metodo != "post" and datos:
            url = url.split("?")[0] + "?" + urllib.parse.urlencode(datos)
        try:
            with self.abridor.open(urllib.request.Request(url, data=cuerpo), timeout=15) as r:
                estado, final, texto = r.status, r.geturl(), r.read(400_000).decode("utf-8", "replace")
        except urllib.error.HTTPError as e:
            estado, final, texto = e.code, url, e.read(400_000).decode("utf-8", "replace")
        except (urllib.error.URLError, TimeoutError, ConnectionError, OSError) as e:
            estado, final, texto = 0, url, f"sin respuesta: {e}"
        time.sleep(0.05)  # el servidor escribe la traza después de responder
        with open(self.log, "rb") as f:
            f.seek(antes)
            self.salida = f.read().decode("utf-8", "replace")
        return estado, final, texto

    def mirar(self, que: str, estado: int, texto: str, datos: list | None = None) -> bool:
        """Apunta el fallo si lo hay; True si la respuesta es buena."""
        traza = _traza(self.salida) or _traza(texto)
        if estado >= 500 or estado == 0 or traza or (estado in (404, 405) and que.startswith("enviar")):
            enviado = ", ".join(f"{k}={v}" for k, v in (datos or [])[:8] if not re.search(r"(?i)csrf|token", k))
            if not any(e["que"] == que and e["traza"] == traza for e in self.errores):  # la 2.ª vuelta no lo repite
                self.errores.append({"que": que, "estado": estado, "traza": traza, "datos": enviado})
            return False
        if estado == 404 and que not in self.rotos:
            self.rotos.append(que)
        elif estado >= 400 and estado != 404 and f"{que} -> {estado}" not in self.raros:
            self.raros.append(f"{que} -> {estado}")
        return estado < 400

    def entrar(self, ruta: str, form: dict) -> None:
        if not any(c["tipo"] == "password" and c.get("name") for c in form["campos"]):
            self.entrar_js = True  # sin name, la envía JavaScript (fetch): este recorrido no la puede usar
            return
        self.intento_entrar = True
        accion = self.dentro(form["accion"] or ruta, urllib.parse.urljoin(self.base, ruta)) or ruta
        for usuario, clave in self.accesos[:12]:
            datos = _rellenar(form, self.hoy, usuario, clave) or []
            estado, _, texto = self.pedir(accion, datos, form["metodo"])
            if not self.mirar(f"entrar como {usuario}", estado, texto):
                return
            if not any(_es_entrar(f) for f in self._leer(texto).formularios):
                self.usuario = usuario
                return

    def vivo(self) -> bool:
        partes = urllib.parse.urlsplit(self.base)
        try:
            socket.create_connection((partes.hostname, partes.port), timeout=2).close()
            return True
        except OSError:
            return False

    def _leer(self, texto: str) -> _Pagina:
        p = _Pagina()
        try:
            p.feed(texto)
        except Exception:  # HTML roto: lo leído hasta ahí vale
            pass
        return p

    def dentro(self, href: str, desde: str) -> str | None:
        url = urllib.parse.urljoin(desde, href.strip()).split("#")[0]
        if not url.startswith(self.base) or ESTATICO.search(url) or NO_TOCAR.search(url):
            return None
        return url[len(self.base) - 1:] or "/"

    def recorrer(self, limite: float) -> None:
        self.enviados_claves, self.orden = set(), []
        pendientes = self.vuelta(deque(["/"]), limite)
        for ruta in pendientes:  # al final ya hay productos, clientes... que elegir
            if time.time() >= limite or self.formularios >= MAX_FORMULARIOS:
                break
            estado, final, texto = self.pedir(ruta)
            if self.mirar(f"abrir {ruta}", estado, texto):
                self.enviar_formularios(ruta, final, self._leer(texto), limite)
        # Segunda vuelta: lo que crearon los formularios cambia las páginas (la de vender, con productos que elegir) y
        # trae enlaces nuevos (la venta recién hecha). Una web abría bien «nueva venta» vacía y daba 500 con productos.
        self.vuelta(deque(self.orden), limite)

    def vuelta(self, cola: deque, limite: float) -> list[str]:
        """Recorre desde cola: abre cada página, sigue sus enlaces y envía sus formularios nuevos. Devuelve las páginas
        con formularios que aún no se podían enviar."""
        self.cola, self.vistas = cola, set()
        vistas, formas, pendientes, paginas = self.vistas, {}, [], 0
        while cola and paginas < MAX_PAGINAS and time.time() < limite:
            if self.errores and self.errores[-1]["estado"] == 0 and not self.vivo():
                break  # el servidor se cayó: lo demás fallaría igual
            ruta = cola.popleft()
            if ruta in vistas or formas.get(_forma(ruta), 0) >= 2:
                continue
            vistas.add(ruta)
            formas[_forma(ruta)] = formas.get(_forma(ruta), 0) + 1
            if ruta not in self.orden:
                self.orden.append(ruta)
            estado, final, texto = self.pedir(ruta)
            self.paginas += 1
            paginas += 1
            if not self.mirar(f"abrir {ruta}", estado, texto):
                continue
            pagina = self._leer(texto)
            entrar = next((f for f in pagina.formularios if _es_entrar(f)), None)
            if entrar is not None and not (self.intento_entrar or self.entrar_js) and self.accesos:
                self.entrar(self.dentro(final, final) or ruta, entrar)
                vistas.discard(ruta)
                cola.appendleft(ruta)
                continue
            for href in pagina.enlaces:
                if (nueva := self.dentro(href, final)) and nueva not in vistas:
                    cola.append(nueva)
            if self.enviar_formularios(ruta, final, pagina, limite):
                pendientes.append(ruta)
        return pendientes

    def enviar_formularios(self, ruta: str, final: str, pagina: _Pagina, limite: float) -> bool:
        """Envía cada formulario nuevo de la página. True si alguno no se podía enviar todavía."""
        quedan = False
        for form in pagina.formularios:
            accion = self.dentro(form["accion"] or final, final)
            botones = " ".join(c.get("value", "") + " " + c.get("name", "") for c in form["campos"]
                               if c["tipo"] in ("submit", "button", "image"))
            clave = (accion and _forma(accion), form["metodo"], tuple(sorted(c.get("name", "") for c in form["campos"])))
            if (accion is None or clave in self.enviados_claves or _es_entrar(form) or NO_TOCAR.search(botones)
                    or any(c["tipo"] == "file" for c in form["campos"])
                    or self.formularios >= MAX_FORMULARIOS or time.time() >= limite):
                continue
            datos = _rellenar(form, self.hoy)
            if datos is None:
                quedan = True  # un select obligatorio sin opciones: quizá más tarde haya productos que elegir
                continue
            self.enviados_claves.add(clave)
            estado, final_f, texto_f = self.pedir(accion, datos, form["metodo"])
            self.formularios += 1
            self.enviados.append(f"{form['metodo'].upper()} {accion} -> {estado}")
            if self.mirar(f"enviar el formulario de {ruta} ({form['metodo'].upper()} {accion})", estado, texto_f, datos):
                for href in self._leer(texto_f).enlaces:
                    if (nueva := self.dentro(href, final_f)) and nueva not in self.vistas:
                        self.cola.append(nueva)
        return quedan


def recorrer(entrada: Path, python: str, accesos: list[tuple[str, str]], env: dict | None = None) -> dict | None:
    """Arranca entrada (en una copia del proyecto que el llamador borra después) y la recorre. None si no llegó a
    escuchar (no es un servidor, falta algo o tarda): entonces se prueba como un programa cualquiera."""
    puerto = _libre()
    log = Path(tempfile.mkstemp(prefix="agente_web_", suffix=".log")[1])
    entorno = dict(env or os.environ, PYTHONUTF8="1", PYTHONIOENCODING="utf-8", PYTHONDONTWRITEBYTECODE="1")
    with open(log, "wb") as salida:
        proc, trabajo = procesos.lanzar([python, str(ARRANQUE), str(entrada), str(puerto)], cwd=entrada.parent,
                                        env=entorno, stdin=subprocess.DEVNULL, stdout=salida, stderr=subprocess.STDOUT,
                                        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    try:
        listo = time.time() + 25
        while time.time() < listo and proc.poll() is None:
            try:
                socket.create_connection(("127.0.0.1", puerto), timeout=1).close()
                break
            except OSError:
                time.sleep(0.3)
        else:
            return None
        r = _Recorrido(f"http://127.0.0.1:{puerto}/", log, accesos)
        r.recorrer(time.time() + MAX_SEGUNDOS)
        texto = log.read_text(encoding="utf-8", errors="replace")
        plantillas = [json.loads(m) for m in re.findall(r"^WEB_PLANTILLA (\{.*\})\s*$", texto, re.MULTILINE)]
        if plantillas:  # el 500 de una página por esa plantilla ya va en su error de plantilla
            r.errores = [e for e in r.errores if "TemplateSyntaxError" not in e["traza"]
                         and "TemplateAssertionError" not in e["traza"]]
        return {"errores": r.errores, "plantillas": plantillas, "rotos": r.rotos, "raros": r.raros,
                "enviados": r.enviados, "paginas": r.paginas, "formularios": r.formularios, "usuario": r.usuario,
                "sin_entrar": r.intento_entrar and not r.usuario, "entrar_js": r.entrar_js}
    finally:
        procesos.detener(proc, trabajo)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass
        try:
            log.unlink()
        except OSError:
            pass
