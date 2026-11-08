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
# Lo que abre el formulario de dar de alta algo.
ABRIR_FORM = re.compile(r"(?i)^\W*\+|nuev[oa]|crear|agregar|añadir|registrar")
# Una web con registro se puede usar sin los usuarios del proyecto: la primera cuenta se crea registrándose.
REGISTRO = re.compile(r"(?i)reg[ií]str(?:ar)?(?:se|te)\b|crear (?:una )?cuenta|sign[ -]?up|/regist(?:er|ro)\b")
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
MAX_PAGINAS, MAX_FORMULARIOS, MAX_SEGUNDOS, MAX_SEGUNDOS_REINTENTO = 60, 25, 75, 15


class _Pagina(html.parser.HTMLParser):
    """Enlaces (y su texto) y formularios (con sus campos y las opciones de cada select) de una página."""

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.enlaces, self.formularios, self.textos = [], [], {}
        self._form = self._select = self._opcion = self._enlace = None

    def handle_starttag(self, tag, attrs):
        a = {k: v or "" for k, v in attrs}
        if tag == "a" and a.get("href"):
            self.enlaces.append(a["href"])
            self._enlace = [a["href"], a.get("title", "")]
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
        if self._enlace is not None:
            self._enlace[1] += data

    def handle_endtag(self, tag):
        if tag == "a" and self._enlace is not None:
            self.textos.setdefault(self._enlace[0], " ".join(self._enlace[1].split())[:60])
            self._enlace = None
        elif tag == "form":
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
        self.intento_entrar = self.entrar_js = self.entrar_fallo = self.registro = self.entrar_get = False
        # «+ Nuevo producto» llevaba a una ruta que, sin formulario, volvía a la lista: nadie podía dar de alta nada.
        self.abre: dict[str, tuple[str, str]] = {}  # destino -> (texto del enlace, página donde está)
        self.sin_formulario: dict[str, list[bool]] = {}  # destino -> en cada vuelta, ¿volvió sin formulario?

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
        self.intento_entrar, self.entrar_get = True, form["metodo"] == "get"
        accion = self.dentro(form["accion"] or ruta, urllib.parse.urljoin(self.base, ruta)) or ruta
        for usuario, clave in self.accesos[:12]:
            datos = _rellenar(form, self.hoy, usuario, clave) or []
            estado, _, texto = self.pedir(accion, datos, form["metodo"])
            if not self.mirar(f"entrar como {usuario}", estado, texto):
                self.entrar_fallo = True  # ese error ya lo dice todo
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
            if ruta in self.abre:
                aqui = (self.dentro(final, final) or "").split("?")[0]
                self.sin_formulario.setdefault(ruta, []).append(
                    aqui == self.abre[ruta][1] and not any(f["metodo"] == "post" for f in pagina.formularios))
            entrar = next((f for f in pagina.formularios if _es_entrar(f)), None)
            if (entrar is None and not self.intento_entrar and re.search(r"""type\s*=\s*["']?password""", texto, re.IGNORECASE)
                    and not any(c["tipo"] == "password" and c.get("name") for f in pagina.formularios for c in f["campos"])):
                self.entrar_js = True  # una clave fuera de todo formulario con name: la envía JavaScript
            if entrar is not None and not (self.intento_entrar or self.entrar_js) and self.accesos:
                self.registro = any(REGISTRO.search(f"{h} {pagina.textos.get(h, '')}") for h in pagina.enlaces)
                self.entrar(self.dentro(final, final) or ruta, entrar)
                vistas.discard(ruta)
                cola.appendleft(ruta)
                continue
            aqui = (self.dentro(final, final) or "").split("?")[0]
            for href in pagina.enlaces:
                nueva = self.dentro(href, final)
                if nueva and (self.usuario or not self.intento_entrar) and ABRIR_FORM.search(pagina.textos.get(href, "")) and nueva.split("?")[0] != aqui:
                    self.abre.setdefault(nueva, (pagina.textos[href], aqui))
                if nueva and nueva not in vistas:
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


# Webs que entran con JavaScript (fetch, campos sin name): se usan con un navegador oculto. Cada elemento visible que
# interesa se marca con data-aura-i para poder pulsarlo o rellenarlo después.
LISTAR = r"""(tipo) => {
  const vis = e => !!(e.offsetWidth || e.offsetHeight || e.getClientRects().length) && getComputedStyle(e).visibility !== 'hidden';
  let n = Number(document.body.dataset.auraN || 0);
  const marca = e => (e.dataset.auraI = e.dataset.auraI || String(++n));
  const texto = e => (e.innerText || e.value || e.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 40);
  let r;
  if (tipo === 'menu') {
    r = [...document.querySelectorAll('nav a, nav button, aside a, aside button, .sidebar a, .sidebar button, .nav-link, ' +
          '[data-section], [data-page], [data-view], [data-tab], [role=tab]')].filter(vis).map(e => ({i: marca(e), texto: texto(e)}));
  } else if (tipo === 'botones') {
    r = [...document.querySelectorAll('button, a.btn, input[type=submit], input[type=button]')].filter(vis)
          .map(e => ({i: marca(e), texto: texto(e), submit: e.type === 'submit'}));
  } else {
    r = [...document.querySelectorAll('input, select, textarea')].filter(vis)
          .filter(e => !['hidden', 'submit', 'button', 'image', 'reset', 'file', 'search'].includes(e.type))
          .map(e => ({i: marca(e), tipo: e.tagName === 'SELECT' ? 'select' : e.tagName === 'TEXTAREA' ? 'textarea' : (e.type || 'text'),
                      name: e.name || e.id || e.placeholder || '', value: e.value || '', min: e.min || '', max: e.max || '',
                      opciones: e.tagName === 'SELECT' ? [...e.options].map(o => ({valor: o.value, texto: o.text, desactivada: o.disabled})) : []}));
  }
  document.body.dataset.auraN = String(n);
  return r;
}"""
# [¿cargó otra página?, texto que se ve fuera del menú]. Lo oculto no cuenta: en una web de una sola página, cada sección
# es un bloque que se enseña o se oculta. Una entrega llamaba desde el menú a secciones que no existían en la página: al
# pulsar «Productos» no cambiaba nada.
PANTALLA = r"""() => {
  const zona = document.querySelector('main, [role=main]') || document.body;
  let t = zona.innerText || '';
  if (zona === document.body)
    for (const e of document.querySelectorAll('nav, aside, header, .sidebar')) if (e.innerText) t = t.split(e.innerText).join('');
  const otra = window.__auraMarca !== 1;
  window.__auraMarca = 1;
  return [otra, t.replace(/\s+/g, ' ').trim().slice(0, 4000)];
}"""
BOTON_ENTRAR = re.compile(r"(?i)entrar|ingresar|acceder|iniciar|login|log in|sign in|continuar")
GUARDAR = re.compile(r"(?i)guardar|crear|registrar|finalizar|aceptar|agregar|añadir|enviar|confirmar|grabar|vender|cobrar")
BUSCADOR = re.compile(r"(?i)buscar|search|filtr")


def _con_navegador(base: str, log: Path, accesos: list[tuple[str, str]], abrir, limite: float) -> dict | None:
    """Entra, pulsa cada sección del menú, rellena y guarda el formulario de «Nuevo» de cada una, y da otra vuelta por
    las secciones. Errores: respuestas 500 del servidor (con su traza). None si no hay navegador."""
    from playwright.sync_api import sync_playwright

    r = {"errores": [], "avisos_js": [], "usuario": None, "sin_entrar": False, "secciones": 0, "formularios": 0,
         "menu_quieto": [], "nuevo_sin_campos": [], "registro": False, "otro_puerto": None, "clave_a_la_vista": False}
    hoy, malas, js, desde, fallidas = date.today().isoformat(), [], [], [log.stat().st_size], []
    puerto = urllib.parse.urlsplit(base).port
    with sync_playwright() as p:
        nav = abrir(p)
        if nav is None:
            return None
        try:
            pag = nav.new_context(accept_downloads=False, viewport={"width": 1280, "height": 900}).new_page()
            pag.on("dialog", lambda d: d.dismiss())  # confirm() -> Cancelar: nunca confirma un borrado
            pag.on("response", lambda resp: malas.append((resp.request.method, resp.url, resp.status))
                   if resp.status >= 500 else None)
            pag.on("pageerror", lambda e: js.append(str(e)[:160]))
            pag.on("requestfailed", lambda req: fallidas.append(req.url))

            def esperar():
                try:
                    pag.wait_for_load_state("networkidle", timeout=4000)
                except Exception:
                    pass
                pag.wait_for_timeout(200)

            def mirar(que: str, datos: str = "") -> None:
                with open(log, "rb") as f:
                    f.seek(desde[0])
                    traza = _traza(f.read().decode("utf-8", "replace"))
                desde[0] = log.stat().st_size
                for metodo, url, estado in malas:
                    error = {"que": f"{que} ({metodo} {url[len(base) - 1:] if url.startswith(base) else url})",
                             "estado": estado, "traza": traza, "datos": datos}
                    if not any(e["que"] == error["que"] and e["traza"] == traza for e in r["errores"]):
                        r["errores"].append(error)
                malas.clear()
                r["avisos_js"] += [e for e in js if e not in r["avisos_js"]]
                js.clear()

            def clic(i: str) -> bool:
                try:
                    pag.locator(f"[data-aura-i='{i}']").first.click(timeout=3000)
                    return True
                except Exception:
                    return False

            def visible_clave() -> bool:
                return pag.locator("input[type=password]:visible").count() > 0

            def pantalla() -> list:
                try:
                    return pag.evaluate(PANTALLA)
                except Exception:  # cargando otra página
                    return [True, ""]

            pag.goto(base, wait_until="domcontentloaded", timeout=15000)
            esperar()
            mirar("abrir /")
            if visible_clave():
                r["registro"] = bool(REGISTRO.search(pag.content()))
                menu_antes = len(pag.evaluate(LISTAR, "menu"))
                for usuario, clave in accesos[:12]:
                    campos = pag.evaluate(LISTAR, "campos")
                    quien = next((c for c in campos if c["tipo"] in ("text", "email")), None)
                    secreto = next((c for c in campos if c["tipo"] == "password"), None)
                    if not (quien and secreto) or time.time() > limite:
                        break
                    pag.locator(f"[data-aura-i='{quien['i']}']").fill(usuario)
                    pag.locator(f"[data-aura-i='{secreto['i']}']").fill(clave)
                    botones = pag.evaluate(LISTAR, "botones")
                    boton = (next((b for b in botones if BOTON_ENTRAR.search(b["texto"])), None)
                             or next((b for b in botones if b["submit"]), None))
                    if not (boton and clic(boton["i"])):
                        pag.locator(f"[data-aura-i='{secreto['i']}']").press("Enter")
                    esperar()
                    errores = len(r["errores"])
                    mirar(f"entrar como {usuario}")
                    if len(r["errores"]) > errores:
                        break  # entrar da un 500: ese error ya lo dice todo
                    # Una entrega dejaba a la vista la pantalla de entrar (su CSS ganaba a .hidden) y la app salía debajo.
                    debajo = visible_clave() and len(pag.evaluate(LISTAR, "menu")) >= menu_antes + 3
                    if not visible_clave() or debajo:
                        r["usuario"], r["clave_a_la_vista"] = usuario, debajo
                        break
                # La página llamaba a la API en http://127.0.0.1:5000 fijo y aquí la web escucha en otro puerto.
                r["otro_puerto"] = next((u for u in fallidas if re.match(r"https?://(?:localhost|127\.0\.0\.1|0\.0\.0\.0)", u)
                                         and urllib.parse.urlsplit(u).port != puerto), None)
                r["sin_entrar"] = r["usuario"] is None and not r["errores"]
            hechas, pulsadas, quietas = set(), 0, []
            for vuelta in range(2):  # la segunda vuelta ve las secciones ya con los datos creados
                textos = list(dict.fromkeys(m["texto"] for m in pag.evaluate(LISTAR, "menu")
                                            if m["texto"] and not NO_TOCAR.search(m["texto"])))[:15]
                for texto in textos:
                    menu = next((m for m in pag.evaluate(LISTAR, "menu") if m["texto"] == texto), None)
                    antes = pantalla()
                    if time.time() > limite or not menu or not clic(menu["i"]):
                        continue
                    esperar()
                    despues = pantalla()
                    if vuelta == 0:
                        pulsadas += 1
                        if not despues[0] and despues[1] == antes[1]:
                            quietas.append(texto)
                    r["secciones"] += 1
                    mirar(f"abrir «{texto}»")
                    if texto in hechas:
                        continue
                    hechas.add(texto)
                    nuevo = next((b for b in pag.evaluate(LISTAR, "botones")
                                  if ABRIR_FORM.search(b["texto"]) and not NO_TOCAR.search(b["texto"])), None)
                    if nuevo and clic(nuevo["i"]):
                        esperar()
                        if not any(not BUSCADOR.search(c["name"]) for c in pag.evaluate(LISTAR, "campos")):
                            r["nuevo_sin_campos"].append({"texto": nuevo["texto"], "seccion": texto, "js": js[:1]})
                    datos = []
                    for c in pag.evaluate(LISTAR, "campos"):
                        valor = None if BUSCADOR.search(c["name"]) else _valor(c, hoy)
                        if valor is None:
                            continue
                        campo = pag.locator(f"[data-aura-i='{c['i']}']")
                        try:
                            if c["tipo"] == "select":
                                campo.select_option(value=valor, timeout=2000)
                            elif c["tipo"] in ("checkbox", "radio"):
                                campo.check(timeout=2000)
                            else:
                                campo.fill(valor, timeout=2000)
                            datos.append(f"{c['name']}={valor}")
                        except Exception:
                            continue
                    guardar = [b for b in pag.evaluate(LISTAR, "botones")
                               if GUARDAR.search(b["texto"]) and not NO_TOCAR.search(b["texto"])]
                    if datos and guardar and clic(guardar[-1]["i"]):  # el del formulario, que va detrás del de abrirlo
                        esperar()
                        r["formularios"] += 1
                        mirar(f"guardar el formulario de «{texto}»", ", ".join(datos[:8]))
                    pag.keyboard.press("Escape")
                if vuelta == 0 and len(quietas) >= 3 and 2 * len(quietas) >= pulsadas:
                    r["menu_quieto"] = quietas  # la sección en la que ya se estaba tampoco cambia: va en la lista
        finally:
            nav.close()
    return r


def recorrer(entrada: Path, python: str, accesos: list[tuple[str, str]], env: dict | None = None,
             abrir_navegador=None) -> dict | None:
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
        navegador = None
        # Lo de dentro solo se ve con un navegador si la entrada es JavaScript. También se prueba con él si no se pudo
        # entrar: un formulario con name que JavaScript enviaba como JSON no dejaba entrar al recorrido, y a una persona sí.
        if abrir_navegador is not None and (r.entrar_js or (r.intento_entrar and not r.usuario and not r.entrar_fallo)):
            # Si solo es para ver si se puede entrar, con poco tiempo: una web así tardaba 30 s en recorrerse entera.
            plazo = MAX_SEGUNDOS if r.entrar_js else MAX_SEGUNDOS_REINTENTO
            navegador = _con_navegador(r.base, log, accesos, abrir_navegador, time.time() + plazo)
            if navegador:
                r.errores += [e for e in navegador.pop("errores") if e not in r.errores]
                r.usuario = navegador["usuario"]
        texto = log.read_text(encoding="utf-8", errors="replace")
        plantillas = [json.loads(m) for m in re.findall(r"^WEB_PLANTILLA (\{.*\})\s*$", texto, re.MULTILINE)]
        if plantillas:  # el 500 de una página por esa plantilla ya va en su error de plantilla
            r.errores = [e for e in r.errores if "TemplateSyntaxError" not in e["traza"]
                         and "TemplateAssertionError" not in e["traza"]]
        # Solo si volvió sin formulario cada vez, también en la segunda vuelta: sin productos, «nueva venta» puede
        # devolver a la lista con un aviso, y eso está bien.
        sin_formulario = [{"texto": r.abre[a][0], "desde": r.abre[a][1], "a": a}
                          for a, veces in r.sin_formulario.items() if len(veces) >= 2 and all(veces)]
        return {"errores": r.errores, "plantillas": plantillas, "rotos": r.rotos, "raros": r.raros,
                "enviados": r.enviados, "paginas": r.paginas, "formularios": r.formularios, "usuario": r.usuario,
                "sin_entrar": r.intento_entrar and not r.usuario and not r.entrar_fallo, "entrar_js": r.entrar_js,
                "navegador": navegador, "sin_formulario": sin_formulario, "registro": r.registro, "entrar_get": r.entrar_get}
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
