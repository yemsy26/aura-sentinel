"""Usa de verdad la web de un proyecto Python (Flask, FastAPI, http.server): la arranca sobre una copia, entra con los
usuarios que da el proyecto, abre sus páginas y envía cada formulario con datos válidos, como haría quien la usa.

Una misión de negocio entregó un sistema que daba error 500 al registrar una venta (db.get_or_404 con dos argumentos):
sus 12 tests pasaban, la revisión lo aprobó y verificar solo comprobaba que el servidor arrancaba. Aquí no juzga el
modelo: cuenta lo que responde el servidor (500, una traza, un formulario que va a una ruta que no existe)."""
import ast
import difflib
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
ABRIR_FORM = re.compile(r"(?i)^\W*\+(?!\s*\d)|nuev[oa]|crear|agregar|añadir|registrar")  # «+10» ajusta el stock, no da de alta
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
MAX_PAGINAS, MAX_FORMULARIOS, MAX_SEGUNDOS, MAX_SEGUNDOS_REINTENTO, MAX_SEGUNDOS_LINEAS = 60, 25, 75, 15, 25
MAX_SEGUNDOS_ALTAS = 40  # el navegador para las altas por modal: neg3 dio sus dos 500 en 20 s
# Una venta cuyas líneas arma JavaScript («+ Agregar Producto» añade un select y una cantidad): sin JavaScript, el recorrido
# solo enviaba el cliente y recibía «Agrega al menos un producto». La ruta de venta, la más importante del negocio, no se
# probaba, y una entrega salió con «'sqlite3.Connection' object has no attribute 'last_insert_rowid'» al vender.
AGREGAR_FILA = re.compile(r"(?i)(?:agregar|a[ñn]adir|add|nuev[oa]|\+)\W*(?:otro\s+|otra\s+|un[oa]?\s+)?"
                          r"(?:producto|art[ií]culo|[ií]tem|item|l[ií]nea|fila|servicio|concepto|row|line|product)")
# El administrador entró y la web no lo deja abrir su propia sección (una entrega pasaba mal el rol al construir el usuario:
# todos eran «True» y @permiso lo rechazaba todo con «No tienes permiso»). Un vendedor sin permiso es normal: solo se mira
# a quien se llama admin.
SIN_CODIGO = re.compile(r"(?is)<(script|style)\b.*?</\1>")
ES_ADMIN = re.compile(r"(?i)^(?:admin|administrador|root|owner|due[ñn]o)\b")
SIN_PERMISO = re.compile(r"(?i)[^<>\n]{0,40}(?:no tienes permisos?|sin permisos?|acceso denegado|no autorizad[oa]|not authori[sz]ed|"
                         r"permission denied|no tiene permisos|no puedes acceder|no est[aá]s autorizad[oa])[^<>\n]{0,60}")
SIN_STOCK = re.compile(r"(?i)\b(?:stock|existencias?|disp\w*)\s*:?\s*0\b|agotad|sin\s+stock")
# Un error de programación que la propia web enseña (la venta capturaba la excepción y la mostraba con un 200). Las
# restricciones de la base (UNIQUE, NOT NULL) no: pueden venir de datos repetidos.
ERROR_DE_PROGRAMA = re.compile(
    r"(?i)[^<>\n]{0,60}(?:object has no attribute|object is not (?:subscriptable|iterable|callable)|unsupported operand type|"
    r"can only concatenate|takes \d+ positional arguments? but|missing \d+ required (?:positional )?arguments?|"
    r"got an unexpected keyword argument|name '\w+' is not defined|no such (?:column|table)|has no column named|"
    r"list index out of range|not supported between instances|error binding parameter|incorrect number of bindings|"
    r"is not json serializable|nonetype|referenced before assignment|(?:not enough|too many) values to unpack|"
    r"string indices must be integers|unhashable type|argument of type '\w+' is not iterable)[^<>\n]{0,60}")


# Un negocio con empleados («cada empleado solo debe ver lo que le corresponde»): tres entregas seguidas dejaban al empleado abrir
# /reportes con las ganancias, y otras le enseñaban el precio de compra en la lista de productos o en la API. Tras recorrer la web
# como administrador, se crea un empleado desde su propia pantalla de usuarios y se mira qué ve. Solo se actúa si se puede completar
# (pantalla de usuarios con rol, el empleado entra); si no, ni una palabra.
MAX_SEGUNDOS_EMPLEADO, MAX_SEGUNDOS_EXPLORAR = 20, 10
EMPLEADO_USUARIO, EMPLEADO_CLAVE = "empleado_aura", "Empleado1234!"
COSTO_PRUEBA, VENTA_PRUEBA = "7.77", "21.31"  # números que ningún precio normal de la web repite
CAMPO_COSTO = re.compile(r"(?i)compra|coste|costo|cost")
CAMPO_VENTA = re.compile(r"(?i)venta|pvp|sale|precio|price")
CAMPO_ROL = re.compile(r"(?i)rol|role|perfil|permiso|nivel|cargo|tipo")
ROL_EMPLEADO = re.compile(r"(?i)emplead|vendedor|cajer|operador|staff|seller|clerk|worker|\buser\b|usuario")
ROL_JEFE = re.compile(r"(?i)admin|gerente|manager|due[ñn]o|owner|root|super")
ROL_ELIGE = re.compile(r"(?i)selecc|elij|elige|choose|select|^\W*$|--")  # «-- Seleccione un rol --» no es un rol
GESTION = re.compile(r"(?i)report|ganancia|utilidad|finanz|informe|margen|costos?\b|usuarios?\b|users?\b|empleados?\b|admin")
MARCA_COSTO = re.compile(r"(?<![\d.,])7[.,]77(?!\d)")
GANANCIAS = re.compile(r"(?i)\bganancias?\b|\butilidad(?:es)?\b|\bmargen(?:es)?\b|\bbeneficios?\b|precio de compra|costo total")
SIN_MENU = re.compile(r"(?is)<(nav|header|aside|footer)\b.*?</\1>")
# Un menú que pinta JavaScript (`{ href: '/productos', label: 'Productos' }` en static/js/nav.js): en el HTML no hay ni un <a>, y el recorrido sin
# navegador solo seguía enlaces. m4153c_neg1 visitó 4 de sus 17 rutas: nunca abrió /productos ni /usuarios, no dio de alta el producto de la
# prueba y /ventas/nueva con un producto (500) no se probó. Solo literales de ruta absoluta completa: «'/productos/editar/' + id» no es una página.
JS_HREF = re.compile(r"""(?<![\w-])["']?href["']?\s*[:=]\s*(["'])(/[^"'`\s{}$<>?#]*)\1""")
SCRIPT_SRC = re.compile(r"""(?is)<script\b[^>]*?\bsrc\s*=\s*["']([^"']+)["']""")
SCRIPT_CUERPO = re.compile(r"(?is)<script\b([^>]*)>(.*?)</script\s*>")
MAX_ENLACES_JS = 25
API_HABITUAL = ("/api/productos", "/api/products", "/api/ventas", "/api/sales", "/api/reportes", "/api/usuarios", "/api/inventario")
PARECIDO = 0.85
# Ventas y facturas: la comprobación del empleado no vende nada, así que cualquier fila o detalle que vea es de otro usuario (m4141: el
# empleado veía /factura/1 del administrador). Y un 500 solo del empleado es un fallo de programa (venta['vendedor_id'] sin esa columna).
VENTAS_RUTA = re.compile(r"(?i)(?:^|/)(?:ventas?|facturas?|invoices?|sales?|pedidos?|orders?|ordenes|recibos?|tickets?)(?:/|$|\?)")
CON_ID = re.compile(r"/\d+(?:/|$|\?)")
# Lo que el dueño abre primero: el informe de ganancias y las facturas. m4141: el informe por defecto salía en 0.00 con una venta de hoy
# (el mes o «hasta hoy» dejaban fuera las ventas con hora) y una web no guardaba las facturas (sin commit): las páginas daban 200.
INFORME_RUTA = re.compile(r"(?i)report|ganancia|utilidad|informe|finanz|estad[ií]stic|resumen")
FACTURA_RUTA = re.compile(r"(?i)(?:^|/)(?:facturas?|invoices?)(?:/|$|\?)")
IMPORTE = re.compile(r"(?<![\w.,/-])(?:RD\$|US\$|[$€])?\s?(\d{1,3}(?:[.,]\d{3})+[.,]\d{2}|\d+[.,]\d{2})(?!\d)")
FORMULARIO_RUTA = re.compile(r"(?i)/(?:nuev[ao]s?|new|crear|create|agregar|add|alta|editar|edit)(?:/|$|\?)")
GENERAR_FACTURA = re.compile(r"(?i)generar|emitir|crear\s+factura|nueva\s+factura|new\s+invoice|facturar|generate")
NUM_FACTURA = re.compile(r"(?i)\b(?:fac|fact|inv|f)[-_ ]?\d{2,}\b")
# Una tabla de ventas con columna de vendedor (el panel de inicio «Últimas ventas» de la ferretería: vendedor admin para el empleado, mientras
# /ventas e /historial ya filtraban). La cabecera fuerte basta con una de importe; la débil (usuario, empleado) pide además una de venta.
CAB_VENDEDOR = re.compile(r"(?i)\b(?:vendedor\w*|vendi[óo]|vendido\s+por|atendi[óo]|atendido\s+por|cajer[oa]s?|seller|sold\s+by|cashier|salesperson)\b")
CAB_VENDEDOR_DEBIL = re.compile(r"(?i)\b(?:usuario|empleado|user|employee|registrado\s+por|creado\s+por|responsable)\b")
CAB_IMPORTE = re.compile(r"(?i)\b(?:total|importe|monto|subtotal|amount)\b")
CAB_VENTA = re.compile(r"(?i)\b(?:cliente|customer|fecha|date|folio|factura|invoice|venta|sale)\b")
CAB_CLIENTE = re.compile(r"(?i)\b(?:cliente|customer)\b")
CAB_FOLIO = re.compile(r"(?i)\b(?:folio|factura|invoice|venta|sale|ticket|recibo)\b")
CAB_MOMENTO = re.compile(r"(?i)\b(?:fecha|date|hora|time)\b")
CAB_PRODUCTO = re.compile(r"(?i)\b(?:producto|art[ií]culo|item|cant\w*|qty|quantity)\b")
PROPIO = re.compile(r"(?i)\bempleado[ _]?aura\b")  # el empleado de la prueba (EMPLEADO_USUARIO, «Empleado Aura»)
SIN_VENDEDOR = re.compile(r"(?i)^\W*(?:n/?a|ning[uú]n[oa]?|none|null|sin\s+\w+)?\W*$|^\W*(?:sub)?total(?:es)?\b")


class _Pagina(html.parser.HTMLParser):
    """Enlaces (y su texto) y formularios (con sus campos y las opciones de cada select) de una página."""

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.enlaces, self.formularios, self.textos = [], [], {}
        self.botones_sueltos: list[str] = []  # textos de los <button> que están fuera de todo formulario (un modal, fetch)
        self._form = self._select = self._opcion = self._enlace = self._boton = self._suelto = None

    def handle_starttag(self, tag, attrs):
        a = {k: v or "" for k, v in attrs}
        if tag == "button" and self._form is None:
            self.botones_sueltos.append("")
            self._suelto = len(self.botones_sueltos) - 1
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
            self._form["campos"].append({**a, "tipo": tipo, "elemento": tag})
            if tag == "button":
                self._boton = self._form["campos"][-1]
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
        if self._boton is not None:
            self._boton["texto"] = self._boton.get("texto", "") + data
        if self._suelto is not None:
            self.botones_sueltos[self._suelto] = " ".join(f"{self.botones_sueltos[self._suelto]} {data}".split())

    def handle_endtag(self, tag):
        if tag == "button":
            self._boton = self._suelto = None
        if tag == "a" and self._enlace is not None:
            self.textos.setdefault(self._enlace[0], " ".join(self._enlace[1].split())[:60])
            self._enlace = None
        elif tag == "form":
            self._form = None
        elif tag == "select":
            self._select = None
        elif tag == "option":
            self._opcion = None


class _Tablas(html.parser.HTMLParser):
    """Las tablas de una página como texto: [{"cabeceras": [...], "filas": [[celda, ...], ...]}]. La cabecera es la primera fila de <th>; las
    tablas anidadas se ignoran (cuenta la de fuera)."""

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.tablas: list[dict] = []
        self._tabla = self._fila = self._celda = None
        self._nivel = 0

    def handle_starttag(self, tag, attrs):
        if tag == "table":
            self._nivel += 1
            if self._nivel == 1:
                self._tabla = {"cabeceras": [], "filas": []}
                self.tablas.append(self._tabla)
        elif self._tabla is None or self._nivel != 1:
            return
        elif tag == "tr":
            self._fila = []
        elif tag in ("th", "td") and self._fila is not None:
            self._celda = [tag, ""]

    def handle_data(self, data):
        if self._celda is not None:
            self._celda[1] += data

    def handle_endtag(self, tag):
        if tag == "table":
            self._nivel = max(0, self._nivel - 1)
            if self._nivel == 0:
                self._tabla = self._fila = self._celda = None
        elif self._tabla is None or self._nivel != 1:
            return
        elif tag in ("th", "td") and self._celda is not None and self._fila is not None:
            self._fila.append((self._celda[0], " ".join(self._celda[1].split())))
            self._celda = None
        elif tag == "tr" and self._fila is not None:
            if self._fila and all(k == "th" for k, _ in self._fila):
                if not self._tabla["cabeceras"]:
                    self._tabla["cabeceras"] = [t for _, t in self._fila]
            elif self._fila:
                self._tabla["filas"].append([t for _, t in self._fila])
            self._fila = None


def _vendedores_ajenos(texto: str) -> list[str]:
    """Los vendedores que enseña una tabla de ventas de esta página, sin contar a ningún empleado de la prueba (que no ha vendido nada): si el
    empleado las abre, son ventas de otro. Vacío si no hay tabla con columna de vendedor y de importe."""
    tablas = _Tablas()
    try:
        tablas.feed(texto)
        tablas.close()
    except Exception:  # un HTML roto no debe estropear nada
        return []
    otros: list[str] = []
    for t in tablas.tablas:
        cab = t["cabeceras"]
        if not any(CAB_IMPORTE.search(c) for c in cab):
            continue
        fuerte = next((i for i, c in enumerate(cab) if CAB_VENDEDOR.search(c)), None)
        debil = next((i for i, c in enumerate(cab) if CAB_VENDEDOR_DEBIL.search(c)), None) if any(CAB_VENTA.search(c) for c in cab) else None
        columna = fuerte if fuerte is not None else debil
        if columna is None:
            continue
        for fila in t["filas"]:
            valor = fila[columna] if len(fila) == len(cab) else ""
            if re.search(r"[^\W\d_]", valor) and not SIN_VENDEDOR.search(valor) and not PROPIO.search(valor) and valor not in otros:
                otros.append(valor)
    return otros


def _ventas_en_tabla(texto: str) -> tuple[int, str] | None:
    """Una tabla de ventas con filas y SIN columna de vendedor (esa la mira _vendedores_ajenos): (filas con importe, un importe de ejemplo).
    Es de ventas si tiene importe y un folio/factura/venta, o un cliente junto a una fecha, hora o producto; las listas de productos o de
    clientes con un «Total comprado» no lo son. Como el empleado de la prueba no ha vendido nada, una lista filtrada por vendedor sale vacía."""
    tablas = _Tablas()
    try:
        tablas.feed(texto)
        tablas.close()
    except Exception:
        return None
    for t in tablas.tablas:
        cab = t["cabeceras"]
        i = next((n for n, c in enumerate(cab) if CAB_IMPORTE.search(c)), None)
        if i is None or any(CAB_VENDEDOR.search(c) for c in cab):
            continue
        junta = " | ".join(cab)
        if not (CAB_FOLIO.search(junta) or (CAB_CLIENTE.search(junta) and (CAB_MOMENTO.search(junta) or CAB_PRODUCTO.search(junta)))):
            continue
        if CAB_VENDEDOR_DEBIL.search(junta) and CAB_VENTA.search(junta):
            continue  # con «usuario» o «empleado» como vendedor flojo: _vendedores_ajenos la juzga
        importes = []
        for fila in t["filas"]:
            if len(fila) != len(cab) or PROPIO.search(" ".join(fila)) or SIN_VENDEDOR.search(fila[0]) and fila[0].strip():
                continue  # fila de otra forma, del propio empleado o de totales
            if any(float(n.replace(",", ".")) for n in re.findall(r"\d+(?:[.,]\d+)?", fila[i])):
                importes.append(fila[i])
        if importes:
            return len(importes), importes[0]
    return None


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


LOGIN_RUTA = re.compile(r"(?i)login|signin|sign-in|entrar|acceso|ingres|auth|iniciar")
# Un botón o enlace cuyo texto es el valor de una variable de Python (`{{ form.submit.data }}` pinta False): quien lo ve no sabe qué hace (m4153_neg4).
ETIQUETA_PYTHON = re.compile(r"^\W*(?:False|True|None)\W*$")


def _pagina_vacia(texto: str, pagina: "_Pagina") -> bool:
    """Una página que no enseña nada que usar: ni formularios, campos, botones, enlaces ni JavaScript que los pinte, y casi sin texto.
    Una página de entrar así (login.html no rellenaba el bloque de la plantilla) deja a todos fuera."""
    if pagina.formularios or pagina.botones_sueltos or pagina.enlaces:
        return False
    if re.search(r"(?i)<(?:input|button|select|textarea|script|iframe|canvas|form)\b", texto):
        return False
    visible = re.sub(r"<[^>]+>", " ", re.sub(r"(?is)<head\b.*?</head>|<!--.*?-->|<(?:script|style)\b.*?</(?:script|style)>", "", texto))
    return len(" ".join(visible.split())) < 40


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
        self.clave: str | None = None
        self.filas_js: dict[str, str] = {}  # página -> acción de su formulario, que añade líneas con un botón (JavaScript)
        self.sin_permiso: list[dict] = []  # el administrador que entró y no puede abrir una de sus páginas
        self.altas_js = False  # un botón «Nuevo» suelto, fuera de todo formulario: el alta es un modal que guarda con fetch
        self.posts = 0  # formularios POST enviados (los de buscar y filtrar son GET)
        self.login_vacio: str | None = None  # la página de entrar a la que manda la web sale sin nada que rellenar
        self.etiquetas: list[dict] = []  # botones y enlaces cuyo texto es False, True o None
        self.intento_entrar = self.entrar_js = self.entrar_fallo = self.registro = self.entrar_get = False
        self.entrar_ruta: str | None = None  # la página con el formulario de entrar que usó el recorrido
        self.sin_empleado = ""  # por qué no se pudo probar con un empleado (para diagnosticar; no se enseña al modelo)
        self.aterrizaje: str | None = None  # la página a la que llegó al entrar
        # «+ Nuevo producto» llevaba a una ruta que, sin formulario, volvía a la lista: nadie podía dar de alta nada.
        self.abre: dict[str, tuple[str, str]] = {}  # destino -> (texto del enlace, página donde está)
        self.sin_formulario: dict[str, list[bool]] = {}  # destino -> en cada vuelta, ¿volvió sin formulario?
        self.js_leidos: dict[str, str] = {}  # script del mismo origen -> su texto (cada uno se pide una vez)
        self.de_js: set[str] = set()  # rutas que solo conoce el JavaScript: si no existen (404, 405) no son un enlace roto, puede ser un trozo

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
        self.intento_entrar, self.entrar_get, self.entrar_ruta = True, form["metodo"] == "get", ruta
        accion =self.dentro(form["accion"] or ruta, urllib.parse.urljoin(self.base, ruta)) or ruta
        for usuario, clave in self.accesos[:12]:
            datos = _rellenar(form, self.hoy, usuario, clave) or []
            estado, final, texto = self.pedir(accion, datos, form["metodo"])
            if not self.mirar(f"entrar como {usuario}", estado, texto):
                self.entrar_fallo = True  # ese error ya lo dice todo
                return
            if not any(self._sigue_entrar(f, final, accion) for f in self._leer(texto).formularios):
                self.usuario, self.clave = usuario, clave
                self.aterrizaje = self.dentro(final, final)  # adonde lo lleva la web al entrar (algunas llevan «/» de vuelta a entrar)
                return

    def _sigue_entrar(self, form: dict, final: str, accion: str) -> bool:
        """¿Esta página de después de entrar sigue enseñando la pantalla de entrar? Un formulario con contraseña que envía a OTRA parte,
        a una ruta que no es de entrar (el alta de empleados de un panel: usuario, contraseña, nombre), no lo es (m4142)."""
        if not _es_entrar(form):
            return False
        if form["accion"]:
            destino = self.dentro(form["accion"], final)
            if destino and destino != accion and not LOGIN_RUTA.search(destino):
                return False
        return True

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

    def enlaces_js(self, texto: str, final: str, html: list[str]) -> list[str]:
        """Las rutas a las que apunta el JavaScript de la página (un menú, un `location.href`): en sus scripts y en los del mismo origen. Las que
        ya enlaza el HTML no se repiten (y dejan de ser «solo de JavaScript»)."""
        for h in html:
            self.de_js.discard(self.dentro(h, final) or "")
        codigo = [c for atributos, c in SCRIPT_CUERPO.findall(texto)
                  if not re.search(r"""(?i)type\s*=\s*["']?(?:application/(?:ld\+)?json|text/template)""", atributos)]
        for src in SCRIPT_SRC.findall(texto):
            url = urllib.parse.urljoin(final, src.strip()).split("#")[0]
            if not url.startswith(self.base):
                continue
            if url not in self.js_leidos:
                try:
                    with self.abridor.open(urllib.request.Request(url), timeout=10) as r:
                        self.js_leidos[url] = r.read(400_000).decode("utf-8", "replace")
                except (urllib.error.URLError, TimeoutError, ConnectionError, OSError):
                    self.js_leidos[url] = ""
            codigo.append(self.js_leidos[url])
        nuevas = []
        for c in codigo:
            for _, ruta in JS_HREF.findall(c):
                if ruta == "/" or (ruta.endswith("/") and len(ruta) > 1):
                    continue  # la raíz ya se abre; una ruta que acaba en «/» es el principio de otra («'/productos/editar/' + id»)
                dentro = self.dentro(ruta, final)
                if dentro and dentro not in nuevas and ruta not in html and dentro not in {self.dentro(h, final) for h in html}:
                    nuevas.append(dentro)
        self.de_js.update(nuevas[:MAX_ENLACES_JS])
        return nuevas[:MAX_ENLACES_JS]

    def dentro(self, href: str, desde: str) -> str | None:
        url = urllib.parse.urljoin(desde, href.strip()).split("#")[0]
        if not url.startswith(self.base) or ESTATICO.search(url) or NO_TOCAR.search(url):
            return None
        return url[len(self.base) - 1:] or "/"

    def explorar(self, limite: float, maximo: int = 40) -> None:
        """Solo lee: entra con el primer formulario de entrar y abre las páginas que enlaza la web (nada se envía, nada se
        borra). Lo que abre queda en self.orden."""
        self.orden, cola, vistas, formas = [], deque(["/"]), set(), {}
        while cola and len(self.orden) < maximo and time.time() < limite:
            ruta = cola.popleft()
            if ruta in vistas or formas.get(_forma(ruta), 0) >= 2:  # dos páginas de la misma forma (/productos/7, /productos/8) bastan
                continue
            vistas.add(ruta)
            formas[_forma(ruta)] = formas.get(_forma(ruta), 0) + 1
            estado, final, texto = self.pedir(ruta)
            if estado >= 400 or estado == 0:
                continue
            pagina = self._leer(texto)
            pagina.enlaces += self.enlaces_js(texto, final, pagina.enlaces)
            entrar = next((f for f in pagina.formularios if _es_entrar(f)), None)
            if entrar is not None and not (self.intento_entrar or self.entrar_js) and self.accesos:
                self.entrar(self.dentro(final, final) or ruta, entrar)
                if self.usuario:  # sin sesión (una entrada con JavaScript, o credenciales que no valen) no se vuelve a pedir en bucle
                    vistas.discard(ruta)
                    cola.appendleft(ruta)
                    if self.aterrizaje:
                        cola.appendleft(self.aterrizaje)
                    continue
            self.orden.append(ruta)
            for href in pagina.enlaces:
                nueva = self.dentro(href, final)
                if nueva and nueva not in vistas:
                    cola.append(nueva)

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
            self._sin_permiso(ruta, estado, final, texto)
            if ruta in self.de_js and estado in (404, 405):
                continue  # una ruta que solo conocía el JavaScript y no existe: no es un enlace roto
            if not self.mirar(f"abrir {ruta}", estado, texto):
                continue
            pagina = self._leer(texto)
            pagina.enlaces += self.enlaces_js(texto, final, pagina.enlaces)
            self._etiquetas_de_python(final, pagina)
            destino = self.dentro(final, final) or ""
            if ruta == "/" and destino.split("?")[0] != "/" and LOGIN_RUTA.search(destino) and _pagina_vacia(texto, pagina):
                self.login_vacio = destino  # el login.html no rellena el bloque de la plantilla base: ni formulario ni nada
            if (self.usuario or not self.intento_entrar) and any(
                    ABRIR_FORM.search(b) and not NO_TOCAR.search(b) for b in pagina.botones_sueltos):
                self.altas_js = True
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

    def _etiquetas_de_python(self, final: str, pagina: _Pagina) -> None:
        """Botones (<button> y <input> de enviar) y enlaces de la página cuyo texto es solo False, True o None."""
        donde = (self.dentro(final, final) or "").split("?")[0]
        vistos = [("botón", c.get("texto", "") if c.get("elemento") == "button" else c.get("value", ""))
                  for f in pagina.formularios for c in f["campos"] if c["tipo"] in ("submit", "button", "reset")]
        vistos += [("botón", b) for b in pagina.botones_sueltos] + [("enlace", t) for t in pagina.textos.values()]
        for que, etiqueta in vistos:
            etiqueta = " ".join(etiqueta.split())
            if ETIQUETA_PYTHON.match(etiqueta) and len(self.etiquetas) < 8:
                nueva = {"pagina": donde, "texto": re.sub(r"\W", "", etiqueta), "que": que}
                if nueva not in self.etiquetas:
                    self.etiquetas.append(nueva)

    def _sin_permiso(self, ruta: str, estado: int, final: str, texto: str) -> None:
        """Al administrador que acaba de entrar la web le cierra una página: un 403, o lo manda a otra con «No tienes permiso»."""
        # Solo antes de enviar ningún formulario POST: el recorrido edita usuarios (rellena el rol con la primera opción) y
        # podría quitarle el rol de administrador a quien acaba de entrar; lo que viniera después no sería un fallo de la web.
        if not (self.usuario and ES_ADMIN.match(self.usuario)) or len(self.sin_permiso) >= 3 or self.posts:
            return
        destino = (self.dentro(final, final) or "").split("?")[0]
        if any(s["ruta"] == ruta for s in self.sin_permiso):
            return
        if estado == 403:
            self.sin_permiso.append({"usuario": self.usuario, "ruta": ruta, "a": None, "frase": "403"})
        elif (estado < 400 and destino and destino != ruta.split("?")[0]
              and not re.search(r"""type\s*=\s*["']?password""", texto, re.IGNORECASE)
              and (m := SIN_PERMISO.search(SIN_CODIGO.sub("", texto)))):
            self.sin_permiso.append({"usuario": self.usuario, "ruta": ruta, "a": destino, "frase": " ".join(m[0].split())})

    def enviar_formularios(self, ruta: str, final: str, pagina: _Pagina, limite: float) -> bool:
        """Envía cada formulario nuevo de la página. True si alguno no se podía enviar todavía."""
        quedan = False
        for form in pagina.formularios:
            accion = self.dentro(form["accion"] or final, final)
            if form["metodo"] == "post" and any(c["tipo"] == "button" and AGREGAR_FILA.search(
                    f"{c.get('texto', '')} {c.get('value', '')}") for c in form["campos"]):
                self.filas_js.setdefault((self.dentro(final, final) or ruta).split("?")[0], accion or ruta)
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
            self.posts += form["metodo"] == "post"
            self.enviados.append(f"{form['metodo'].upper()} {accion} -> {estado}")
            if self.mirar(f"enviar el formulario de {ruta} ({form['metodo'].upper()} {accion})", estado, texto_f, datos):
                for href in self._leer(texto_f).enlaces:
                    if (nueva := self.dentro(href, final_f)) and nueva not in self.vistas:
                        self.cola.append(nueva)
        return quedan


def _visible(texto: str) -> str:
    sin = re.sub(r"(?is)<head\b.*?</head>|<!--.*?-->|<(?:script|style)\b.*?</(?:script|style)>", " ", texto)
    return " ".join(re.sub(r"<[^>]+>", " ", sin).split())


def _es_alta(form: dict, accion: str | None) -> bool:
    """Un formulario POST que da de alta algo: ni id en la acción ni campo oculto de id (los de editar), ni archivos."""
    if accion is None or form["metodo"] != "post" or re.search(r"/\d", accion) or LOGIN_RUTA.search(accion):
        return False  # (no _es_entrar: un alta de usuario con usuario, clave y rol también tiene tres campos y una contraseña)
    return not any(c["tipo"] == "file" or (c["tipo"] == "hidden" and c.get("value") and re.fullmatch(
        r"(?i)id|\w*_id|uid|pk", c.get("name") or "") is not None) for c in form["campos"])


def _rol_de_empleado(campo: dict) -> str | None:
    """El rol con el que se da de alta al empleado de la prueba: uno que se llame empleado, vendedor... y, si ninguno se llama así (recepcion,
    veterinario: la veterinaria del usuario, 4.13.8, se saltaba la prueba), el primero que no sea de jefe. Nunca el «elige un rol» ni uno desactivado."""
    candidatos = []
    for o in campo["opciones"]:
        valor = o["texto"].strip() if o["valor"] is None else o["valor"]
        etiqueta = f"{valor} {o['texto']}"
        if not o["desactivada"] and valor.strip() and not ROL_JEFE.search(etiqueta) and not ROL_ELIGE.search(etiqueta):
            candidatos.append((valor, etiqueta))
    return next((v for v, e in candidatos if ROL_EMPLEADO.search(e)), candidatos[0][0] if candidatos else None)


def _empleado(r: _Recorrido, limite: float) -> dict | None:
    """Con la sesión del administrador: da de alta un producto con costo 7.77 y un usuario con rol de empleado desde los formularios
    de la propia web, entra como ese empleado y lee (solo GET) lo que abrió el administrador. None si no se puede completar.
    {"usuario", "entro", "producto", "costo": [rutas donde el empleado ve 7.77], "abre": [páginas de gestión que ve igual que el admin]}"""
    paginas = []
    for ruta in r.orden[:MAX_PAGINAS]:
        if time.time() >= limite:
            r.sin_empleado = "sin tiempo para releer las páginas"
            return None
        estado, final, texto = r.pedir(ruta)
        if estado < 400:
            paginas.append((ruta, final, r._leer(texto)))
    usuario = producto = None
    for _, final, pagina in paginas:
        for form in pagina.formularios:
            accion = r.dentro(form["accion"] or final, final)
            if not _es_alta(form, accion):
                continue
            nombres = [c.get("name") or "" for c in form["campos"]]
            con_clave = any(c["tipo"] == "password" for c in form["campos"])
            if usuario is None and con_clave and any(CAMPO_ROL.search(n) for n in nombres):
                usuario = (accion, form)
            elif producto is None and not con_clave and any(CAMPO_COSTO.search(n) for n in nombres):
                producto = (accion, form)
    if usuario is None:
        r.sin_empleado = "ninguna página tiene un formulario de alta de usuarios con contraseña y rol"
        return None
    accion, form = usuario
    campos = {c["name"]: c for c in form["campos"] if c.get("name")}
    datos = _rellenar(form, r.hoy)
    if datos is None:
        r.sin_empleado = "el formulario de alta de usuarios tiene un campo obligatorio que no se puede rellenar"
        return None
    # Sin otro campo de usuario ni de correo, `nombre` («Nombre de usuario») es con lo que se entra: el recorrido lo creaba como «Empleado Aura»
    # y luego entraba como «empleado_aura» (m4153c_neg1, m4141_neg1, m4153_neg3, m4153b_neg1: la prueba del empleado no corría).
    nombre_es_usuario = not any(c.get("tipo") == "email" or (c.get("tipo") == "text" and re.search(
        r"user|usuario|login|cuenta|mail|correo", (c.get("name") or "").lower())) for c in form["campos"])
    nuevo, con_rol = [], False
    for nombre, valor in datos:
        c, n = campos.get(nombre, {}), nombre.lower()
        if c.get("tipo") == "password":
            valor = EMPLEADO_CLAVE
        elif c.get("tipo") == "select" and CAMPO_ROL.search(n):
            valor, con_rol = _rol_de_empleado(c), True
            if valor is None:
                r.sin_empleado = "el campo de rol no tiene una opción de empleado (solo admin o similares)"
                return None
        elif c.get("tipo") == "text" and CAMPO_ROL.search(n):
            valor, con_rol = "empleado", True
        elif c.get("tipo") == "radio" and CAMPO_ROL.search(n):
            r.sin_empleado = "el rol es un grupo de botones de radio"
            return None
        elif c.get("tipo") == "email" or re.search(r"mail|correo", n):
            valor = f"{EMPLEADO_USUARIO}@example.com"
        elif c.get("tipo") == "text" and re.search(r"user|usuario|login|cuenta", n):
            valor = EMPLEADO_USUARIO
        elif c.get("tipo") == "text" and re.search(r"nombre|name", n):
            valor = EMPLEADO_USUARIO if nombre_es_usuario else "Empleado Aura"
        elif valor == "Prueba":
            valor = "Aura"
        nuevo.append((nombre, valor))
    if not con_rol:
        r.sin_empleado = "el formulario de usuarios no tiene un campo de rol que se pueda rellenar"
        return None
    producto_ok = False
    if producto is not None:
        paccion, pform = producto
        pcampos = {c["name"]: c for c in pform["campos"] if c.get("name")}
        pdatos = _rellenar(pform, r.hoy)
        if pdatos is not None:
            ajustados = []
            for nombre, valor in pdatos:
                if pcampos.get(nombre, {}).get("tipo") in ("number", "text"):
                    valor = COSTO_PRUEBA if CAMPO_COSTO.search(nombre) else VENTA_PRUEBA if CAMPO_VENTA.search(nombre) else valor
                if valor == "Prueba":  # el recorrido de después crea el suyo con «Prueba»: si choca (código único), da un 500 falso
                    valor = "Producto Aura"
                ajustados.append((nombre, valor))
            estado, _, _ = r.pedir(paccion, ajustados, "post")
            producto_ok = estado < 400
    estado, _, _ = r.pedir(accion, nuevo, "post")
    if estado >= 400:
        r.sin_empleado = f"el alta del empleado respondió {estado} (CSRF, permiso o validación)"
        return None
    e = _Recorrido(r.base, r.log, [(EMPLEADO_USUARIO, EMPLEADO_CLAVE), (f"{EMPLEADO_USUARIO}@example.com", EMPLEADO_CLAVE)])
    estado, final, texto = e.pedir(r.entrar_ruta)
    entrar = next((f for f in e._leer(texto).formularios if _es_entrar(f)), None)
    if entrar is None:
        r.sin_empleado = "la pantalla de entrar no se vuelve a ver con una sesión nueva"
        return None
    e.entrar(e.dentro(final, final) or r.entrar_ruta, entrar)  # con una pantalla de entrar nueva: su CSRF es el de esta sesión
    resultado = {"usuario": EMPLEADO_USUARIO, "entro": bool(e.usuario), "producto": producto_ok, "costo": [], "abre": [],
                 "error": [], "ajenas": [], "cerradas": [], "fase2": False}
    if not e.usuario:
        return resultado
    del_admin = {p[0]: (r.dentro(p[1], p[1]) or "").split("?")[0] for p in paginas}  # dónde acaba cada página para el administrador
    rutas = list(dict.fromkeys(["/", *[p[0] for p in paginas], *API_HABITUAL]))
    for ruta in rutas[:MAX_PAGINAS + len(API_HABITUAL)]:
        if time.time() >= limite:
            break
        estado, final, texto = e.pedir(ruta)
        destino = (e.dentro(final, final) or "").split("?")[0]
        if estado >= 500 and ruta in del_admin:  # un 500 solo del empleado, confirmado, donde el administrador la abre bien
            if e.pedir(ruta)[0] >= 500 and 0 < r.pedir(ruta)[0] < 400:
                resultado["error"].append(ruta)
            continue
        camino = ruta.split("?")[0]
        if ruta != "/" and ruta in del_admin and del_admin[ruta] == camino and (
                estado in (401, 403) or (estado < 400 and destino and destino != camino and not LOGIN_RUTA.search(destino))):
            resultado["cerradas"].append(ruta)  # cerrada al empleado por su rol: el canario de la segunda lectura
        if estado >= 400 or estado == 0 or destino == r.entrar_ruta.split("?")[0] or LOGIN_RUTA.search(destino):
            continue  # lo mandó a entrar (no con _es_entrar: un alta de usuarios con clave y rol lo parece)
        if MARCA_COSTO.search(texto):
            resultado["costo"].append(ruta)
        camino = ruta.split("?")[0]
        if GESTION.search(camino) and destino == camino:
            suyo = _visible(texto)
            if SIN_PERMISO.search(suyo):
                continue  # la página dice que no tiene permiso: cerrada con un 200
            cuerpo = _visible(SIN_MENU.sub(" ", texto))  # sin menús: un enlace «Reportes» o «Ganancias» lo ve todo el mundo
            con_clave = any(c["tipo"] == "password" for f in e._leer(texto).formularios for c in f["campos"])
            if GANANCIAS.search(cuerpo):
                resultado["abre"].append({"ruta": ruta, "motivo": "ganancias"})
            elif con_clave and any(CAMPO_ROL.search(c.get("name") or "") for f in e._leer(texto).formularios for c in f["campos"]):
                resultado["abre"].append({"ruta": ruta, "motivo": "alta de usuarios"})
            else:
                ajeno = _visible(r.pedir(ruta)[2])
                parecido = difflib.SequenceMatcher(None, ajeno[:3000], suyo[:3000]).ratio() if len(suyo) >= 80 else 0.0
                if parecido >= PARECIDO:
                    resultado["abre"].append({"ruta": ruta, "motivo": "igual", "parecido": round(parecido, 2)})
    return resultado


def _anotar_vendedores(emp: dict, ruta: str, texto: str) -> None:
    """Si la página del empleado tiene una tabla de ventas con vendedores que no son él (no ha vendido nada), la apunta en emp["ajenas"]."""
    otros = _vendedores_ajenos(texto)
    if otros and not any(a["ruta"] == ruta and a["motivo"] == "vendedor" for a in emp["ajenas"]):
        emp["ajenas"].append({"ruta": ruta, "motivo": "vendedor", "vendedores": otros[:3]})
    tabla = _ventas_en_tabla(texto)
    if tabla and not any(a["ruta"] == ruta and a["motivo"] == "tabla" for a in emp["ajenas"]):
        emp["ajenas"].append({"ruta": ruta, "motivo": "tabla", "filas": tabla[0], "importe": tabla[1]})


def _empleado_despues(r: _Recorrido, previo: _Recorrido, emp: dict | None, limite: float) -> None:
    """Segunda lectura, DESPUÉS del recorrido de siempre (que ya vendió con los formularios de la web): el empleado vuelve a entrar y
    abre lo que el administrador abrió. Un 500 solo suyo, o las ventas y facturas de otros (él no ha vendido nada), se anotan en emp.
    Solo si sigue siendo empleado: ese recorrido edita usuarios y puede haberlo hecho administrador; el canario son las páginas que en
    la primera lectura se le cerraron, que tienen que seguir cerradas (si no hubo ninguna, no hay forma de saberlo y no se mira)."""
    if not emp or not emp.get("entro") or not emp.get("cerradas") or not previo.entrar_ruta:
        return
    accesos = [(u, c) for c in (EMPLEADO_CLAVE, "Prueba1234!") for u in (EMPLEADO_USUARIO, f"{EMPLEADO_USUARIO}@example.com")]
    e = _Recorrido(r.base, r.log, accesos)
    estado, final, texto = e.pedir(previo.entrar_ruta)
    entrar = next((f for f in e._leer(texto).formularios if _es_entrar(f)), None)
    if entrar is None:
        return
    e.entrar(e.dentro(final, final) or previo.entrar_ruta, entrar)
    if not e.usuario:
        return
    for ruta in emp["cerradas"]:
        estado, final, _ = e.pedir(ruta)
        if estado < 400 and (e.dentro(final, final) or "").split("?")[0] == ruta.split("?")[0]:
            return  # una página que le estaba cerrada se le abre: el recorrido pudo ascenderlo, no se le culpa a la web
    emp["fase2"] = True
    rutas = list(dict.fromkeys(r.orden))[:30]
    # Las ventas que armó el navegador con JavaScript no están en lo que abrió el recorrido: se buscan en los enlaces de sus listas.
    for lista in [x for x in rutas if VENTAS_RUTA.search(x.split("?")[0]) and not CON_ID.search(x.split("?")[0])][:3]:
        estado_a, final_a, texto_a = r.pedir(lista)
        if not 0 < estado_a < 400:
            continue
        for href in r._leer(texto_a).enlaces:
            nueva = r.dentro(href, final_a)
            if (nueva and len(rutas) < 36 and nueva not in rutas and VENTAS_RUTA.search(nueva.split("?")[0])
                    and CON_ID.search(nueva.split("?")[0])):
                rutas.append(nueva)
    for ruta in rutas:
        if time.time() >= limite:
            break
        camino = ruta.split("?")[0]
        estado, final, texto = e.pedir(ruta)
        if estado >= 500:
            if e.pedir(ruta)[0] >= 500 and 0 < r.pedir(ruta)[0] < 400 and ruta not in emp["error"]:
                emp["error"].append(ruta)
            continue
        destino = (e.dentro(final, final) or "").split("?")[0]
        if estado < 400 and destino == camino:
            _anotar_vendedores(emp, ruta, texto)
        if estado >= 400 or destino != camino or not VENTAS_RUTA.search(camino):
            continue
        estado_a, _, ajeno = r.pedir(ruta)
        if not 0 < estado_a < 400:
            continue  # el administrador ya no la abre: nada con qué comparar
        suyo = _visible(texto)
        if CON_ID.search(camino):
            parecido = difflib.SequenceMatcher(None, _visible(ajeno)[:3000], suyo[:3000]).ratio() if len(suyo) >= 80 else 0.0
            if parecido >= PARECIDO:
                emp["ajenas"].append({"ruta": ruta, "motivo": "detalle", "parecido": round(parecido, 2)})
        elif any(CON_ID.search(h) and VENTAS_RUTA.search(h.split("?")[0]) for h in e._leer(texto).enlaces):
            emp["ajenas"].append({"ruta": ruta, "motivo": "filas"})
    if "/" not in rutas and time.time() < limite:  # el panel de inicio: «Últimas ventas» de todos aunque /ventas ya filtre (ferretería 18)
        estado, final, texto = e.pedir("/")
        if estado < 400 and (e.dentro(final, final) or "").split("?")[0] == "/":
            _anotar_vendedores(emp, "/", texto)
    otras = {a["ruta"] for a in emp["ajenas"] if a["motivo"] != "tabla"}  # una ruta ya avisada por «filas», «detalle» o «vendedor» no se repite
    emp["ajenas"] = [a for a in emp["ajenas"] if a["motivo"] != "tabla" or a["ruta"] not in otras]


def _informes_y_facturas(r: _Recorrido, limite: float) -> dict:
    """Con la sesión del administrador y cuando ya hay ventas: ¿el informe por defecto enseña todo en 0 aunque hay ventas?, ¿la lista de
    facturas está vacía aunque hay ventas (se creó y no se guardó)? Solo con una venta con importe visible en la lista de ventas."""
    res: dict = {"ventas": None, "informe_cero": [], "facturas_vacias": [], "paginas_informe": 0, "paginas_facturas": 0}
    if not r.usuario or not ES_ADMIN.match(r.usuario):
        return res

    def leer(ruta: str) -> str | None:
        estado, final, texto = r.pedir(ruta)
        if not 0 < estado < 400 or (r.dentro(final, final) or "").split("?")[0] != ruta.split("?")[0]:
            return None
        return texto

    def importes(texto: str) -> list[str]:
        return IMPORTE.findall(_visible(SIN_MENU.sub(" ", texto)))

    sin_id = [x for x in dict.fromkeys(r.orden) if not CON_ID.search(x.split("?")[0])]
    crudo = ""
    for lista in [x for x in sin_id if VENTAS_RUTA.search(x.split("?")[0]) and not FACTURA_RUTA.search(x.split("?")[0])
                  and not FORMULARIO_RUTA.search(x.split("?")[0])][:3]:
        texto = leer(lista)
        if texto is None:
            continue
        crudo += texto
        positivos = [m for m in importes(texto) if re.search(r"[1-9]", m)]
        # Una venta de verdad: la lista tiene filas con enlace a su detalle (no el formulario de «nueva venta», que enseña precios).
        con_filas = any(CON_ID.search(h) and VENTAS_RUTA.search(h.split("?")[0]) for h in r._leer(texto).enlaces)
        if positivos and con_filas and res["ventas"] is None:
            res["ventas"] = (lista, positivos[0])
    if res["ventas"] is None or time.time() >= limite:
        return res
    for ruta in [x for x in sin_id if INFORME_RUTA.search(x.split("?")[0]) and not VENTAS_RUTA.search(x.split("?")[0])][:3]:
        texto = leer(ruta)
        if texto is None:
            continue
        cifras = importes(texto)
        res["paginas_informe"] += 1
        if len(cifras) >= 2 and not any(re.search(r"[1-9]", m) for m in cifras):
            res["informe_cero"].append(ruta)
    for ruta in [x for x in sin_id if FACTURA_RUTA.search(x.split("?")[0])][:2]:
        texto = leer(ruta)
        if texto is None:
            continue
        res["paginas_facturas"] += 1
        cuerpo = _visible(SIN_MENU.sub(" ", texto))
        con_filas = (any(CON_ID.search(h) for h in r._leer(texto).enlaces) or NUM_FACTURA.search(cuerpo)
                     or any(re.search(r"[1-9]", m) for m in importes(texto)))
        if not con_filas and not GENERAR_FACTURA.search(crudo + texto):
            res["facturas_vacias"].append(ruta)
    return res


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
                      name: e.name || e.id || e.placeholder || '', clase: String(e.className || ''),
                      value: e.value || '', min: e.min || '', max: e.max || '',
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
# Hay una contraseña en la página, ninguna se ve y no se ve nada con lo que seguir (ni enlace, ni botón, ni campo, ni texto):
# la pantalla sale en blanco. Devuelve quién la oculta [etiqueta, id, clases, cómo]; None si algo se ve o no hay contraseña.
# Una entrega tenía <div class="page active"> y su CSS no daba display a .page.active: todo el formulario iba dentro de un
# bloque con display:none.
ENTRAR_OCULTO = r"""() => {
  const vis = e => !!(e.offsetWidth || e.offsetHeight || e.getClientRects().length) && getComputedStyle(e).visibility !== 'hidden';
  const claves = [...document.querySelectorAll('input[type=password]')];
  if (!claves.length || claves.some(vis)) return null;
  if ([...document.querySelectorAll('a[href], button, input, select, textarea, [role=button]')].some(vis)) return null;
  if ((document.body.innerText || '').replace(/\s+/g, ' ').trim().length >= 20) return null;
  for (let e = claves[0]; e && e !== document.documentElement; e = e.parentElement) {
    const c = getComputedStyle(e);
    if (c.display === 'none' || c.visibility === 'hidden')
      return [e.tagName.toLowerCase(), e.id, String(e.className || ''), c.display === 'none' ? 'display:none' : 'visibility:hidden'];
  }
  return ['input', '', '', 'sin tamaño'];
}"""
BOTON_ENTRAR = re.compile(r"(?i)entrar|ingresar|acceder|iniciar|login|log in|sign in|continuar")
GUARDAR = re.compile(r"(?i)guardar|crear|registrar|finalizar|aceptar|agregar|añadir|enviar|confirmar|grabar|vender|cobrar")
BUSCADOR = re.compile(r"(?i)buscar|search|filtr")


def _con_navegador(base: str, log: Path, accesos: list[tuple[str, str]], abrir, limite: float) -> dict | None:
    """Entra, pulsa cada sección del menú, rellena y guarda el formulario de «Nuevo» de cada una, y da otra vuelta por
    las secciones. Errores: respuestas 500 del servidor (con su traza). None si no hay navegador."""
    from playwright.sync_api import sync_playwright

    r = {"errores": [], "avisos_js": [], "usuario": None, "sin_entrar": False, "secciones": 0, "formularios": 0,
         "menu_quieto": [], "nuevo_sin_campos": [], "registro": False, "otro_puerto": None, "clave_a_la_vista": False,
         "login_invisible": None, "sin_autorizacion": []}
    hoy, malas, js, desde, fallidas = date.today().isoformat(), [], [], [log.stat().st_size], []
    puerto = urllib.parse.urlsplit(base).port
    # Respuestas 401 (o 403 al administrador) a las lecturas que hace la pantalla con fetch desde que se pulsa «entrar»:
    # una entrega mandaba sus peticiones sin la cabecera Authorization y la API las rechazaba, con listas y panel vacíos.
    # Si ese intento de entrar no vale, se descartan: comprobar la sesión da 401 con normalidad.
    intento: dict = {"usuario": None, "vistas": []}

    def respuesta(resp) -> None:
        if resp.status >= 500:
            malas.append((resp.request.method, resp.url, resp.status))
        elif (resp.status in (401, 403) and intento["usuario"] and resp.request.method == "GET"
              and resp.request.resource_type in ("fetch", "xhr")
              and (resp.status == 401 or ES_ADMIN.match(intento["usuario"]))):
            intento["vistas"].append((resp.status, urllib.parse.urlsplit(resp.url).path))
    with sync_playwright() as p:
        nav = abrir(p)
        if nav is None:
            return None
        try:
            pag = nav.new_context(accept_downloads=False, viewport={"width": 1280, "height": 900}).new_page()
            pag.on("dialog", lambda d: d.dismiss())  # confirm() -> Cancelar: nunca confirma un borrado
            pag.on("response", respuesta)
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
            if not visible_clave():
                try:
                    r["login_invisible"] = pag.evaluate(ENTRAR_OCULTO)
                except Exception:  # cargando otra página
                    pass
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
                    intento["usuario"] = usuario
                    intento["vistas"].clear()
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
                    intento["usuario"] = None  # este intento no valió: lo que respondió la API ya no cuenta
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
            if r["usuario"]:
                vistas = list(dict.fromkeys(intento["vistas"]))  # una por ruta y estado, en el orden en que salieron
                r["sin_autorizacion"] = [{"estado": estado, "ruta": ruta, "usuario": r["usuario"]} for estado, ruta in vistas]
        finally:
            nav.close()
    return r


def _esperar(pag) -> None:
    try:
        pag.wait_for_load_state("networkidle", timeout=4000)
    except Exception:
        pass
    pag.wait_for_timeout(200)


def _clic(pag, i: str) -> bool:
    try:
        pag.locator(f"[data-aura-i='{i}']").first.click(timeout=3000)
        return True
    except Exception:
        return False


def _entrar_con_navegador(pag, base: str, usuario: str | None, clave: str | None) -> None:
    """Abre la web y, si pide contraseña, entra con el usuario dado."""
    pag.goto(base, wait_until="domcontentloaded", timeout=15000)
    _esperar(pag)
    if usuario is not None and pag.locator("input[type=password]:visible").count() > 0:
        campos = pag.evaluate(LISTAR, "campos")
        quien = next((c for c in campos if c["tipo"] in ("text", "email")), None)
        secreto = next((c for c in campos if c["tipo"] == "password"), None)
        if quien and secreto:
            pag.locator(f"[data-aura-i='{quien['i']}']").fill(usuario)
            pag.locator(f"[data-aura-i='{secreto['i']}']").fill(clave or "")
            botones = pag.evaluate(LISTAR, "botones")
            boton = (next((b for b in botones if BOTON_ENTRAR.search(b["texto"])), None)
                     or next((b for b in botones if b["submit"]), None))
            if not (boton and _clic(pag, boton["i"])):
                pag.locator(f"[data-aura-i='{secreto['i']}']").press("Enter")
            _esperar(pag)


def _lineas_con_navegador(base: str, log: Path, usuario: str | None, clave: str | None, abrir, limite: float,
                          paginas: list[str]) -> dict | None:
    """Hace lo que el recorrido sin JavaScript no puede: en cada página cuyo formulario añade líneas con un botón, entra,
    lo pulsa, elige un producto y una cantidad, guarda y mira si el servidor da 500, deja una traza o la página enseña un
    error de programa. None si no hay navegador."""
    from playwright.sync_api import sync_playwright

    r = {"errores": [], "guardados": 0}
    hoy, malas, desde = date.today().isoformat(), [], log.stat().st_size
    with sync_playwright() as p:
        nav = abrir(p)
        if nav is None:
            return None
        try:
            pag = nav.new_context(accept_downloads=False, viewport={"width": 1280, "height": 900}).new_page()
            pag.on("dialog", lambda d: d.dismiss())
            pag.on("response", lambda resp: malas.append((resp.request.method, resp.url, resp.status))
                   if resp.status >= 500 else None)

            def esperar():
                _esperar(pag)

            def clic(i: str) -> bool:
                return _clic(pag, i)

            _entrar_con_navegador(pag, base, usuario, clave)
            for ruta in paginas:
                if time.time() > limite:
                    break
                pag.goto(urllib.parse.urljoin(base, ruta), wait_until="domcontentloaded", timeout=15000)
                esperar()
                anadir = next((b for b in pag.evaluate(LISTAR, "botones") if AGREGAR_FILA.search(b["texto"])), None)
                if not (anadir and clic(anadir["i"])):
                    continue
                esperar()
                datos = []
                for c in pag.evaluate(LISTAR, "campos"):
                    if c["tipo"] == "select":  # una línea con un producto sin stock no es lo que hace quien vende
                        c = {**c, "opciones": [o for o in c["opciones"] if not SIN_STOCK.search(o["texto"])]}
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
                botones = pag.evaluate(LISTAR, "botones")
                guardar = (next((b for b in botones if b["submit"] and not NO_TOCAR.search(b["texto"])), None)
                           or next((b for b in reversed(botones) if GUARDAR.search(b["texto"]) and not NO_TOCAR.search(b["texto"])
                                    and not AGREGAR_FILA.search(b["texto"])), None))
                if not (datos and guardar):
                    continue
                malas.clear()
                desde = log.stat().st_size
                if not clic(guardar["i"]):
                    continue
                esperar()
                r["guardados"] += 1
                with open(log, "rb") as f:
                    f.seek(desde)
                    traza = _traza(f.read().decode("utf-8", "replace"))
                try:
                    visible = pag.inner_text("body", timeout=3000)
                except Exception:
                    visible = ""
                fallo_visible = ERROR_DE_PROGRAMA.search(visible)
                que = f"guardar el formulario de {ruta} (tras «{anadir['texto']}»)"
                if malas or traza or fallo_visible:
                    estado = malas[0][2] if malas else 200
                    r["errores"].append({"que": que, "estado": estado, "datos": ", ".join(datos[:8]),
                                         "traza": traza or (f"«{' '.join(fallo_visible[0].split())}»" if fallo_visible else "")})
        finally:
            nav.close()
    return r


# La pantalla de «nueva venta» (m4141: dos entregas con una pantalla que no vendía y el servidor sin fallar: verificar daba la web por buena).
VENTA_ALTA = re.compile(r"(?i)(?:^|/)(?:ventas?|sales?|pedidos?|orders?)/(?:nuev[ao]|new|crear|create|agregar|add|alta|registrar)(?:/|$|\?)"
                        r"|(?:^|/)(?:nuev[ao][-_]venta|new[-_](?:sale|order)|registrar[-_]venta|pos|punto[-_]de[-_]venta)(?:/|$|\?)")
PRODUCTO_CAMPO = re.compile(r"(?i)prod|art[ií]culo|item|servicio")
# Una pantalla de venta sin desplegable: una tabla con un campo de cantidad por producto (id cant_0, valor 0), y los campos que llevan las líneas.
CANTIDAD_CAMPO = re.compile(r"(?i)cant|qty|quant|unid")
LINEAS_CAMPO = re.compile(r"(?i)detall|l[ií]nea|carrito|cart")
PRODUCTO_RUTA = re.compile(r"(?i)(?:^|/)(?:productos?|products?|inventario|art[ií]culos?)(?:/|$|\?)")
# Lo que dice un servidor cuando le llega una venta sin líneas.
VENTA_VACIA = re.compile(r"(?i)(?:al\s+menos|at\s+least)\s+(?:un|una|1|one)\s+(?:producto|art[ií]culo|[ií]tem|item|product|l[ií]nea|line)"
                         r"|(?:carrito|cesta|cart|basket)\s+(?:est[áa]\s+)?(?:vac[ií]o|empty)"
                         r"|(?:agrega|a[ñn]ade)\s+(?:primero\s+)?(?:un\s+)?productos?(?:\s+(?:al|a\s+la)\s+(?:carrito|venta))?[.!]")
ANADIR_AL_CARRITO = re.compile(r"(?i)agregar|a[ñn]adir|add|incluir|^\+")  # «Agregar», «Añadir al carrito»: lo elegido pasa a la lista de la venta
OPCION_VACIA = re.compile(r"(?i)^\W*(?:selecc\w*|elig\w*|elij\w*|escog\w*|choose|select|ning[uú]n\w*)\b|^\W*$")
# Una librería de un CDN que no cargó (la prueba no tiene internet) no dice nada de la pantalla.
JS_LIBRERIA = re.compile(r"(?i)(?:^|\W)(?:\$|jQuery|bootstrap|axios|Chart|Vue|React|Swal|moment|Alpine|Toastify|flatpickr|"
                         r"Select2|DataTable)(?!\w).*(?:is not defined|is not a function|undefined)")
MAX_SEGUNDOS_VENTA = 25
MAX_SEGUNDOS_PRODUCTO = 5
# Errores de JavaScript que no dependen de los datos: una función o variable que no existe, un script que no se puede leer. Un «Cannot read properties
# of undefined» sin productos puede ser solo la lista vacía, y no se acusa.
JS_SIEMPRE = re.compile(r"(?i)\bis not defined\b|^(?:Unexpected (?:token|identifier|end of input|string|number)|Invalid or unexpected token|"
                        r"missing \) after|Unterminated|Identifier '[^']+' has already been declared)")


# Una pantalla de venta sin desplegable de productos pero con un buscador (m4133_neg4, m4134_neg2, m4135_neg1, m4136_neg2): se escribe unas letras del
# producto en un campo de texto y o sale una lista de sugerencias en la que se pulsa o la fila se rellena sola. Los productos que hay cuando se prueba la
# venta son los que dejó la prueba del empleado («Producto Aura») o los que trae la web («Producto Prueba»): se prueba con el principio de esos nombres,
# del más seguro al más general. El buscador de clientes no cuenta.
BUSCA_PRODUCTO = ("Aur", "Pro", "Pru")
CALCULADO = re.compile(r"(?i)precio|price|total|importe|monto|subtotal")  # lo calcula la pantalla (a menudo de solo lectura): la prueba no lo toca
BUSCADOR_CLIENTE = re.compile(r"(?i)client|customer")
# Lo que ya contiene el texto buscado antes de escribirlo (no puede ser una sugerencia).
PREVIOS = r"""(q) => {
  const vis = e => !!(e.offsetWidth || e.offsetHeight || e.getClientRects().length) && getComputedStyle(e).visibility !== 'hidden';
  for (const e of document.querySelectorAll('body *')) if (vis(e) && (e.innerText || '').toLowerCase().includes(q)) e.dataset.auraPrevio = '1';
}"""
# La sugerencia: el elemento más pequeño que se ve, es nuevo y contiene lo buscado (si hace de botón, el clic sube hasta su onclick).
SUGERENCIA = r"""(q) => {
  const vis = e => !!(e.offsetWidth || e.offsetHeight || e.getClientRects().length) && getComputedStyle(e).visibility !== 'hidden';
  const no = ['INPUT', 'SELECT', 'OPTION', 'TEXTAREA', 'SCRIPT', 'STYLE'];
  const nuevos = [...document.querySelectorAll('body *')].filter(e => vis(e) && !e.dataset.auraPrevio && !no.includes(e.tagName)
                                                                   && (e.innerText || '').toLowerCase().includes(q));
  const menor = nuevos.find(e => !nuevos.some(h => h !== e && e.contains(h)));
  if (!menor) return null;
  let n = Number(document.body.dataset.auraN || 0);
  menor.dataset.auraI = menor.dataset.auraI || String(++n);
  document.body.dataset.auraN = String(n);
  return menor.dataset.auraI;
}"""
# Los campos (también los ocultos, menos el buscador) y las filas de tabla: si cambian es que algo se armó.
FOTO_CAMPOS = r"""(i) => JSON.stringify([...document.querySelectorAll('input, select, textarea')].filter(e => e.dataset.auraI !== i)
                                          .map(e => (e.name || e.id || '') + '=' + e.value))"""
FOTO_FILAS = r"""() => document.querySelectorAll('tbody tr').length"""


def _es_buscador(c: dict) -> bool:
    """Un campo de texto en el que se busca un producto (su nombre, id, placeholder o clase dicen producto y buscar), no un cliente."""
    nombre = f"{c['name']} {c.get('clase', '')}"
    return c["tipo"] == "text" and bool(PRODUCTO_CAMPO.search(nombre) and BUSCADOR.search(nombre)) and not BUSCADOR_CLIENTE.search(nombre)


def _buscar_y_elegir(pag, buscador: dict) -> str | None:
    """Escribe unas letras del producto en el buscador y elige el resultado: pulsa la sugerencia que sale o deja que la fila se rellene sola.
    El texto con el que se armó algo (un campo, un oculto o una fila cambiaron aparte del propio buscador); None si ninguno sirvió."""
    campo = pag.locator(f"[data-aura-i='{buscador['i']}']").first
    for texto in BUSCA_PRODUCTO:
        q = texto.lower()
        try:
            campo.fill("", timeout=2000)
            _esperar(pag)
            pag.evaluate("() => document.querySelectorAll('[data-aura-previo]').forEach(e => delete e.dataset.auraPrevio)")
            pag.evaluate(PREVIOS, q)
            campos_antes, filas_antes = pag.evaluate(FOTO_CAMPOS, buscador["i"]), pag.evaluate(FOTO_FILAS)
            campo.click(timeout=2000)
            campo.press_sequentially(texto, delay=60, timeout=3000)
            pag.wait_for_timeout(300)
            _esperar(pag)
            marca = pag.evaluate(SUGERENCIA, q)
            pulsada = bool(marca) and _clic(pag, marca)
            if pulsada:
                _esperar(pag)
            # una fila de más sin que cambie ningún campo solo cuenta si se pulsó una sugerencia (un filtro en vivo de una tabla no arma nada)
            if pag.evaluate(FOTO_CAMPOS, buscador["i"]) != campos_antes or (pulsada and pag.evaluate(FOTO_FILAS) != filas_antes):
                return texto
        except Exception:
            return None
    return None


def _consola_propia(m, base: str) -> bool:
    """Un console.error del script de la propia web («Error parsing productos»), no un 404/500 de red ni algo de otro origen."""
    try:
        return m.type == "error" and not m.text.startswith("Failed to load resource") and (m.location.get("url") or "").startswith(base)
    except Exception:
        return False


# Lo que dice la web cuando no pudo guardar la venta («Error al registrar venta: (sqlite3.IntegrityError) NOT NULL constraint failed…»).
# Los rechazos que una prueba sin datos del negocio puede provocar con razón (falta un cliente, no hay stock, un dato inválido) no cuentan.
VENTA_RECHAZADA = re.compile(r"(?i)\berror\b|\bfall[oó]\b|no se pudo|\bfailed\b|excepci[oó]n|exception")
VENTA_RECHAZO_LEGITIMO = re.compile(r"(?i)requerid|obligatori|selecci|required|stock|existencia|insuficien|disponible|cliente|duplicad|ya existe|"
                                    r"inv[aá]lid|invalid|contrase|sesi[oó]n|permiso|csrf|token")
# La lista de ventas (ruta exacta, sin «nueva» ni id) y el botón que abre ahí el panel o modal de «nueva venta» (m4148_neg4: la venta
# se hace en la propia lista, y su ruta de alta solo acepta POST, así que el recorrido nunca la vio).
VENTA_LISTA = re.compile(r"(?i)^/?(?:ventas?|sales?|pedidos?|orders?)/?$")
# El botón que vende en una pantalla de venta que es la propia lista (m4167_*: «Finalizar Venta», «Procesar Venta», «Completar Venta»). No vale uno
# que busque, filtre o exporte la lista: ese no vende.
VENDER_BOTON = re.compile(r"(?i)venta|vender|cobrar|cobro|finaliz|procesar|completar|confirmar|pagar|pedido|orden|\bsale\b|\border\b|checkout")
NO_VENDER = re.compile(r"(?i)buscar|search|filtr|export|imprimir|print|descargar|download|report")
NUEVA_VENTA = re.compile(r"(?i)nuev[oa]\s+(?:venta|pedido|orden)|new\s+(?:sale|order)|registrar\s+(?:una\s+)?venta|"
                         r"(?:agregar|a[ñn]adir|crear)\s+(?:una\s+)?venta|^\W*\+\s*venta")


def _opcion_producto(campo: dict) -> str | None:
    """El valor que elegiría quien vende: la primera opción que es un producto (ni el «Selecciona…», ni uno desactivado o agotado)."""
    for o in campo["opciones"]:
        if o["desactivada"] or not (o["valor"] or "").strip() or OPCION_VACIA.search(o["texto"] or "") or SIN_STOCK.search(o["texto"] or ""):
            continue
        return o["valor"]
    return None


def _venta_en_pantalla(base: str, log: Path, usuario: str | None, clave: str | None, abrir, limite: float,
                       paginas: list[str], hay_productos: bool, listas: list[str] = (), paneles: list[str] = ()) -> dict | None:
    """Vende desde la pantalla de «nueva venta» como lo haría una persona, con un navegador oculto. Tres fallos que el
    recorrido sin JavaScript no ve: el desplegable de productos queda vacío porque el script falla («vacio»; solo si hay
    productos creados), y se elige un producto y una cantidad, se guarda y la web contesta que falta al menos un producto
    («mensaje»: las líneas no llegan al servidor), o el servidor da 500 / deja una traza al guardar («servidor»), o no
    dice nada y no guarda («no_registrada»: la pantalla, la lista de ventas y las existencias quedan como estaban).
    «listas»: las páginas que cambian si la venta se guarda. «paneles»: listas de ventas con un botón que abre el formulario
    en la propia página (solo se prueban si ninguna ruta de alta dio una prueba). None si no hay navegador."""
    from playwright.sync_api import sync_playwright

    r = {"errores": [], "servidor": [], "probadas": 0, "guardadas": 0, "sin_desplegable": 0, "estados": [], "defecto": []}
    hoy, js, malas, envios, dialogos = date.today().isoformat(), [], [], [], []
    with sync_playwright() as p:
        nav = abrir(p)
        if nav is None:
            return None
        try:
            ctx = nav.new_context(accept_downloads=False, viewport={"width": 1280, "height": 900})
            pag = ctx.new_page()
            pag.on("dialog", lambda d: (dialogos.append(d.message), d.dismiss()))
            pag.on("pageerror", lambda e: js.append(" ".join(str(e).split())[:160]))
            pag.on("console", lambda m: js.append(" ".join(m.text.split())[:160]) if _consola_propia(m, base) else None)
            pag.on("response", lambda resp: malas.append((resp.request.method, resp.url, resp.status)) if resp.status >= 500 else None)
            pag.on("response", lambda resp: envios.append((resp.request.method, urllib.parse.urlsplit(resp.url).path, resp.status))
                   if resp.request.method != "GET" and resp.request.resource_type in ("document", "fetch", "xhr") else None)
            _entrar_con_navegador(pag, base, usuario, clave)

            def texto() -> str:
                try:
                    return pag.inner_text("body", timeout=3000)
                except Exception:
                    return ""

            def instantanea() -> dict[str, str]:
                """El texto de las listas tal como las ve quien entró (con la sesión del navegador, sin moverlo de la pantalla)."""
                vistas = {}
                for lista in listas:
                    try:
                        resp = ctx.request.get(urllib.parse.urljoin(base, lista), timeout=5000)
                        if resp.status == 200 and urllib.parse.urlsplit(resp.url).path.rstrip("/") == lista.split("?")[0].rstrip("/"):
                            vistas[lista] = _visible(SIN_MENU.sub(" ", resp.text()))
                    except Exception:
                        continue
                return vistas

            def probar(ruta: str, anadir_antes: bool, panel: bool = False, por_defecto: bool = False) -> dict:
                """«anadir_antes»: pulsar «agregar línea» antes de rellenar (filas que se añaden) o después (se elige y se añade al carrito).
                «panel»: la lista de ventas lleva el formulario oculto tras un botón de «nueva venta»: se abre primero.
                «por_defecto» (m4168): los desplegables que no son de productos se dejan como vienen («Sin cliente»), que es lo que hace quien no los toca."""
                js.clear()
                pag.goto(urllib.parse.urljoin(base, ruta), wait_until="domcontentloaded", timeout=15000)
                _esperar(pag)
                en_ruta = lambda: urllib.parse.urlsplit(pag.url).path.rstrip("/") == ruta.split("?")[0].rstrip("/")
                if not en_ruta():
                    return {"estado": "fuera"}  # la mandó a otra parte (sin permiso, a la entrada)
                abierto = None
                directa = False  # la lista de ventas es ella misma la pantalla de venta (punto de venta): sin botón que abrir
                if panel:
                    abre = next((b for b in pag.evaluate(LISTAR, "botones") if NUEVA_VENTA.search(b["texto"])
                                 and not NO_TOCAR.search(b["texto"])), None)
                    if abre:
                        if not _clic(pag, abre["i"]):
                            return {"estado": "sin_boton"}
                        _esperar(pag)
                        if not en_ruta():
                            return {"estado": "sin_boton"}  # el botón era un enlace a otra pantalla: esa se prueba por su ruta
                        abierto = abre["i"]
                    else:
                        directa = True
                botones = pag.evaluate(LISTAR, "botones")
                vender = next((b for b in botones if VENDER_BOTON.search(b["texto"]) and not NO_TOCAR.search(b["texto"])
                               and not NO_VENDER.search(b["texto"]) and not AGREGAR_FILA.search(b["texto"])), None)
                if directa and not vender:
                    return {"estado": "sin_boton"}  # una lista sin botón de vender no es una pantalla de venta
                anadir = next((b for b in botones if AGREGAR_FILA.search(b["texto"])), None)
                carrito = anadir or next((b for b in botones if (directa or not b["submit"]) and ANADIR_AL_CARRITO.search(b["texto"])
                                          and not NO_TOCAR.search(b["texto"]) and not (directa and b is vender)), None)
                es_producto = lambda c: c["tipo"] == "select" and PRODUCTO_CAMPO.search(f"{c['name']} {c.get('clase', '')}")
                inicial = any(es_producto(c) for c in pag.evaluate(LISTAR, "campos"))
                if anadir_antes and anadir and _clic(pag, anadir["i"]):
                    _esperar(pag)
                campos = pag.evaluate(LISTAR, "campos")
                productos = [c for c in campos if es_producto(c)]
                errores_js = [e for e in js if not JS_LIBRERIA.search(e)]
                res = {"estado": "sin_desplegable", "inicial": inicial, "carrito": carrito is not None, "js": errores_js[0] if errores_js else "",
                       "modulo": pag.query_selector("script[type=module]") is not None}
                cantidades = []
                if not productos and hay_productos and not any(c["tipo"] in ("checkbox", "radio") for c in campos):
                    cantidades = [c for c in campos if c["tipo"] == "number" and CANTIDAD_CAMPO.search(c["name"])
                                  and (c["value"] or "0") in ("0", "") and c["max"] != "0"]
                buscador = None
                if not productos and not cantidades:
                    buscador = next((c for c in campos if _es_buscador(c)), None) if hay_productos else None
                    escrito = _buscar_y_elegir(pag, buscador) if buscador else None
                    if not escrito:
                        return {"estado": "sin_boton"} if directa else res  # sin buscador, o no sale nada que elegir: puede ser otra forma de vender, no se acusa
                    campos = pag.evaluate(LISTAR, "campos")  # lo que elegir cambió: una fila con su cantidad, un campo relleno
                if productos and not any(_opcion_producto(c) for c in productos):
                    errores = [e for e in js if not JS_LIBRERIA.search(e)]
                    return {**res, "estado": "vacio", "js": errores[0] if errores else ""}
                datos, distintos = [], []
                for c in campos:
                    valor = _opcion_producto(c) if es_producto(c) else None
                    if c in cantidades:  # tabla de cantidades: se pide una unidad del primero y se deja el resto a cero
                        if c is not cantidades[0]:
                            continue
                        valor = "1"
                    if valor is None:
                        if c["tipo"] == "select":
                            c = {**c, "opciones": [o for o in c["opciones"] if not SIN_STOCK.search(o["texto"])]}
                        valor = None if BUSCADOR.search(c["name"]) or (buscador and CALCULADO.search(c["name"])) else _valor(c, hoy)
                        if c["tipo"] == "select" and not es_producto(c) and valor is not None and valor != c["value"]:
                            defecto = next((o for o in c["opciones"] if o["valor"] == c["value"] and not o["desactivada"]), None)
                            if defecto is not None and not OPCION_VACIA.search(defecto["texto"] or ""):  # una opción de verdad («Sin cliente»), no «Selecciona…»
                                if por_defecto:
                                    continue  # se deja como viene
                                distintos.append(f"«{c['name']}» ({' '.join((defecto['texto'] or '').split())[:40]})")
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
                            if cantidades and c is cantidades[0]:
                                campo.dispatch_event("change")
                        datos.append(f"{c['name']}={valor}")
                    except Exception:
                        continue
                res["defecto"] = distintos  # los desplegables cuyo valor por defecto es una opción de verdad y distinta de la elegida
                if buscador:
                    datos.append(f"{buscador['name']}={escrito}")
                if not anadir_antes and carrito and _clic(pag, carrito["i"]):
                    _esperar(pag)
                botones = pag.evaluate(LISTAR, "botones")
                botones = [b for b in botones if b["i"] != abierto]  # el botón que abrió el panel no es el de guardar
                guardar = (next((b for b in botones if b["submit"] and not NO_TOCAR.search(b["texto"])), None)
                           or next((b for b in reversed(botones) if GUARDAR.search(b["texto"]) and not NO_TOCAR.search(b["texto"])
                                    and not AGREGAR_FILA.search(b["texto"])), None))
                if directa:  # el que vende, no el primer «submit» (el «✕» de quitar una fila, el «+ Agregar» del carrito)
                    guardar = next((b for b in botones if b["i"] == vender["i"]), None)
                if not (datos and guardar):
                    return {**res, "estado": "sin_guardar"}
                forma = None  # el formulario del botón: ¿envía por POST?, ¿qué campos lleva?
                try:
                    forma = pag.locator(f"[data-aura-i='{guardar['i']}']").evaluate(
                        "e => e.form ? {metodo: (e.form.getAttribute('method') || 'get').toLowerCase(), "
                        "nombres: [...e.form.elements].map(x => x.name).filter(Boolean)} : null", timeout=2000)
                except Exception:
                    pass
                consulta_antes = urllib.parse.urlsplit(pag.url).query
                antes = texto()
                listas_antes = instantanea()
                malas.clear()
                envios.clear()
                dialogos.clear()
                desde = log.stat().st_size
                if not _clic(pag, guardar["i"]):
                    return {**res, "estado": "sin_guardar"}
                _esperar(pag)
                with open(log, "rb") as f:
                    f.seek(desde)
                    traza = _traza(f.read().decode("utf-8", "replace"))
                despues = texto()
                fallo_visible = ERROR_DE_PROGRAMA.search(despues)
                if malas or traza or fallo_visible:
                    return {**res, "estado": "servidor", "datos": ", ".join(datos[:8]), "traza": traza or (
                        f"«{' '.join(fallo_visible[0].split())}»" if fallo_visible else ""), "http": malas[0][2] if malas else 200}
                # Sin ninguna petición que no sea GET y con la dirección llena de los campos del formulario (m4161_neg1: <form> sin method que un script
                # envía con this.submit()): el navegador lo mandó por GET y la ruta solo guarda con POST.
                consulta = urllib.parse.urlsplit(pag.url).query
                if (forma and forma["metodo"] != "post" and not envios and consulta and consulta != consulta_antes
                        and any(n in urllib.parse.parse_qs(consulta) for n in forma["nombres"])
                        and any(PRODUCTO_CAMPO.search(n) or LINEAS_CAMPO.search(n) for n in forma["nombres"])):
                    return {**res, "estado": "por_get", "url": urllib.parse.urlsplit(pag.url).path + "?" + consulta[:100]}
                dicho = VENTA_VACIA.search(despues)
                if dicho and not VENTA_VACIA.search(antes):  # si ya estaba escrito antes de guardar, no es la respuesta del servidor
                    return {**res, "estado": "mensaje", "mensaje": " ".join(dicho.group(0).split())}
                if dicho := next(filter(None, map(VENTA_VACIA.search, dialogos)), None):  # un alert que dice lo mismo
                    return {**res, "estado": "mensaje", "mensaje": " ".join(dicho.group(0).split())}
                if por_defecto and (rechazo := next((" ".join(x.split())[:200] for x in dialogos if VENTA_RECHAZADA.search(x)
                                                     and not VENTA_RECHAZO_LEGITIMO.search(x)), None)):  # un alert con un error que no es un rechazo con razón
                    return {**res, "estado": "rechazada", "mensaje": rechazo, "envio": " ".join(f"{m} {u} -> {s}" for m, u, s in envios[-1:])}
                # No salió ninguna petición: no se envió nada (un carrito sin lo elegido, un campo que el navegador no deja pasar).
                if not envios:
                    return {**res, "estado": "sin_envio", "js": next((e for e in js if not JS_LIBRERIA.search(e)), "")}
                # Se envió y ni la lista de ventas ni las existencias cambiaron. Si la pantalla tampoco (ni mensaje, ni otra pantalla), la
                # web no hizo nada que se vea («no_registrada»); si dijo algo, no se sabe si lo que dijo es un rechazo («sin_confirmar»).
                if listas_antes:
                    ahora = instantanea()
                    comparadas = [k for k in listas_antes if k in ahora]
                    if comparadas and all(listas_antes[k] == ahora[k] for k in comparadas):
                        if despues == antes and en_ruta() and not (por_defecto and dialogos):  # un alert sin error no se acusa
                            return {**res, "estado": "no_registrada", "js": next((e for e in js if not JS_LIBRERIA.search(e)), ""),
                                    "envio": " ".join(f"{m} {u} -> {s}" for m, u, s in envios[-1:])}
                        ya = set(antes.splitlines())
                        dicho = next((" ".join(x.split())[:200] for x in despues.splitlines() if x.strip() and x not in ya
                                      and VENTA_RECHAZADA.search(x) and not VENTA_RECHAZO_LEGITIMO.search(x)), None)
                        if dicho:
                            return {**res, "estado": "rechazada", "mensaje": dicho,
                                    "envio": " ".join(f"{m} {u} -> {s}" for m, u, s in envios[-1:])}
                        return {**res, "estado": "sin_confirmar"}
                return {**res, "estado": "guardada"}

            for ruta, panel in [(x, False) for x in paginas] + [(x, True) for x in paneles]:
                if time.time() > limite:
                    break
                if panel and r["probadas"]:
                    break  # las listas con panel solo si ninguna ruta de alta dio una prueba
                antes_usado = True
                v = probar(ruta, True, panel)
                # Una pantalla de «elegir y añadir al carrito» (el desplegable ya está y el botón añade lo elegido): se prueba también así.
                if v["estado"] in ("mensaje", "no_registrada", "sin_envio") and v.get("inicial") and v.get("carrito"):
                    v = probar(ruta, False, panel)
                    antes_usado = False
                if v["estado"] == "sin_boton":
                    continue  # la lista de ventas no abre nada: ni se anota
                r["estados"].append((ruta, v["estado"]))
                if v["estado"] == "sin_desplegable":
                    r["sin_desplegable"] += 1
                    if hay_productos and v.get("js"):  # hay productos y el script de la pantalla falla al cargar: no se puede vender
                        r["errores"].append({"pagina": ruta, "tipo": "sin_desplegable", "js": v["js"], "mensaje": "", "modulo": v["modulo"]})
                    elif v.get("js") and JS_SIEMPRE.search(v["js"]):  # sin productos que mostrar, pero el script no depende de ellos para fallar
                        r["errores"].append({"pagina": ruta, "tipo": "js_roto", "js": v["js"], "mensaje": "", "modulo": v["modulo"]})
                    continue
                if v["estado"] in ("fuera", "sin_guardar", "sin_envio"):
                    continue
                r["probadas"] += 1
                r["guardadas"] += v["estado"] == "guardada"
                if v["estado"] == "servidor":
                    r["servidor"].append({"que": f"guardar la venta desde la pantalla {ruta} (con el navegador)", "estado": v["http"],
                                          "datos": v["datos"], "traza": v["traza"]})
                elif v["estado"] == "vacio" and v["js"] and hay_productos:
                    r["errores"].append({"pagina": ruta, "tipo": "vacio", "js": v["js"], "mensaje": ""})
                elif v["estado"] == "mensaje":
                    r["errores"].append({"pagina": ruta, "tipo": "mensaje", "js": "", "mensaje": v["mensaje"]})
                elif v["estado"] == "no_registrada":
                    r["errores"].append({"pagina": ruta, "tipo": "no_registrada", "js": v["js"], "mensaje": v["envio"]})
                elif v["estado"] == "rechazada":
                    r["errores"].append({"pagina": ruta, "tipo": "rechazada", "js": "", "mensaje": v["mensaje"]})
                elif v["estado"] == "por_get":
                    r["errores"].append({"pagina": ruta, "tipo": "por_get", "js": "", "mensaje": v["url"]})
                # Vendió bien con otra opción: ¿y dejando «Sin cliente» como viene, que es lo que hace quien no toca el desplegable?
                if v["estado"] == "guardada" and v.get("defecto") and time.time() < limite:
                    w = probar(ruta, antes_usado, panel, por_defecto=True)
                    r["defecto"].append((ruta, w["estado"]))
                    donde = ", ".join(v["defecto"])
                    if w["estado"] == "servidor":
                        r["servidor"].append({"que": f"guardar la venta desde la pantalla {ruta} (con el navegador) dejando {donde} como viene",
                                              "estado": w["http"], "datos": w["datos"], "traza": w["traza"]})
                    elif w["estado"] == "rechazada":
                        r["errores"].append({"pagina": ruta, "tipo": "defecto", "js": "", "mensaje": w["mensaje"], "defecto": donde})
                    elif w["estado"] == "no_registrada":
                        r["errores"].append({"pagina": ruta, "tipo": "defecto", "js": "", "mensaje": w["envio"], "defecto": donde})
        finally:
            nav.close()
    return r


def _producto_de_prueba(r: _Recorrido, limite: float) -> bool:
    """Da de alta un producto desde el formulario HTML de la propia web, con la sesión del recorrido (m4165: la prueba de venta solo corre con productos y
    los únicos que había eran los de la prueba del empleado, que se salta si no hay formulario de usuarios con rol o si no entró como administrador).
    «Producto Aura», costo 7.77 y precio 21.31 como en esa prueba. True si la web contestó sin error."""
    for ruta in dict.fromkeys(x for x in r.orden if PRODUCTO_RUTA.search(x.split("?")[0]) and not CON_ID.search(x.split("?")[0])):
        if time.time() >= limite:
            return False
        estado, final, texto = r.pedir(ruta)
        if not 0 < estado < 400:
            continue
        for form in r._leer(texto).formularios:
            accion = r.dentro(form["accion"] or final, final)
            nombres = [c.get("name") or "" for c in form["campos"]]
            if (not _es_alta(form, accion) or any(c["tipo"] == "password" for c in form["campos"])
                    or not any(CAMPO_COSTO.search(n) for n in nombres)):
                continue
            datos = _rellenar(form, r.hoy)
            if datos is None:
                continue
            campos = {c["name"]: c for c in form["campos"] if c.get("name")}
            ajustados = []
            for nombre, valor in datos:
                if campos.get(nombre, {}).get("tipo") in ("number", "text"):
                    valor = COSTO_PRUEBA if CAMPO_COSTO.search(nombre) else VENTA_PRUEBA if CAMPO_VENTA.search(nombre) else valor
                if valor == "Prueba":  # el recorrido de después crea el suyo con «Prueba»: si choca (código único), da un 500 falso
                    valor = "Producto Aura"
                ajustados.append((nombre, valor))
            estado, _, _ = r.pedir(accion, ajustados, "post")
            return estado < 400
    return False


def _hay_productos(r: _Recorrido) -> bool:
    """¿Hay productos con precio a la vista en la lista de productos (el recorrido da de alta los suyos)?"""
    for ruta in [x for x in dict.fromkeys(r.orden) if PRODUCTO_RUTA.search(x.split("?")[0]) and not CON_ID.search(x.split("?")[0])
                 and not FORMULARIO_RUTA.search(x.split("?")[0])][:3]:
        estado, final, texto = r.pedir(ruta)
        if (0 < estado < 400 and (r.dentro(final, final) or "").split("?")[0] == ruta.split("?")[0]
                and any(re.search(r"[1-9]", m) for m in IMPORTE.findall(_visible(SIN_MENU.sub(" ", texto))))):
            return True
    return False


# Lo que NO es texto que lea la persona en una plantilla: lo que Jinja no emite, el código y las etiquetas.
PLANTILLA_NO_TEXTO = re.compile(r"(?is)\{%-?\s*raw\s*-?%\}.*?\{%-?\s*endraw\s*-?%\}|\{#.*?#\}|<script\b.*?</script\s*>|<style\b.*?</style\s*>"
                                r"|<template\b.*?</template\s*>|<!--.*?-->")
PLANTILLA_JINJA = re.compile(r"(?s)\{\{.*?\}\}|\{%.*?%\}")
PLANTILLA_ETIQUETA = re.compile(r"(?s)</?[A-Za-z][^<>]*>")
# `${total:.2f}` (un literal de JavaScript o un f-string de Python metido en HTML con Jinja) y `{total:.2f}` (un format de Python): Jinja los deja tal cual.
PLANTILLA_MARCADOR = re.compile(r"\$\{\s*[A-Za-z_][\w.]*\s*(?::[^}\s]{1,8})?\s*\}|(?<![{$\w])\{[A-Za-z_][\w.]*:[.,0-9]*[dfsxXeEg%]\}")


def plantillas_sin_sustituir(*carpetas: Path) -> list[dict]:
    """Marcadores de formato que quedan en el texto de las plantillas (`${venta_hoy_total:.2f}`) y que la persona verá escritos tal cual.
    Solo mira el texto: no el JavaScript, los estilos, `<template>`, `{% raw %}`, los comentarios ni los atributos de las etiquetas."""
    vistos, hallazgos = set(), []
    for base in carpetas:
        for sub in ("templates", "plantillas"):
            for f in sorted((base / sub).rglob("*")) if (base / sub).is_dir() else []:
                if f.suffix.lower() not in (".html", ".htm", ".jinja", ".j2") or f in vistos:
                    continue
                vistos.add(f)
                try:
                    texto = f.read_text(encoding="utf-8", errors="replace")
                except OSError:
                    continue
                for patron, hueco in ((PLANTILLA_NO_TEXTO, ""), (PLANTILLA_JINJA, " X"), (PLANTILLA_ETIQUETA, "")):
                    texto = patron.sub(lambda m, h=hueco: h + "\n" * m.group(0).count("\n"), texto)
                for n, linea in enumerate(texto.splitlines(), 1):
                    m = PLANTILLA_MARCADOR.search(linea)
                    if m:
                        hallazgos.append({"plantilla": f.relative_to(base).as_posix(), "linea": n, "texto": m.group(0)})
    return hallazgos


# Bootstrap, Tailwind, jQuery o una tipografía cargados de internet (<link> o <script src> con https://): la ferretería (22) del usuario (4.13.17) cargaba
# Bootstrap de cdn.jsdelivr.net y su mandato era «que funcione en mi computadora con un doble clic»: sin conexión la web pierde el estilo y los menús, y
# verificar la abre con red y no lo nota. Solo se mira en webs con servidor (probar_web); un juego o una página 3D estática sí puede llevar su CDN.
EXTERNO_ETIQUETA = re.compile(r"(?is)<(link|script)\b([^<>]*)>")
EXTERNO_URL = re.compile(r"""(?is)\b(?:href|src)\s*=\s*["']?((?:https?:)?//[^"'\s>/][^"'\s>]*)""")
EXTERNO_NO_CODIGO = re.compile(r"""(?is)\brel\s*=\s*["']?(?:icon|shortcut|canonical|alternate|preconnect|dns-prefetch|author|license)""")
EXTERNO_CSS = re.compile(r"""(?i)@import\s+(?:url\(\s*)?["']?((?:https?:)?//[^"')\s;]+)""")
EXTERNO_LOCAL = re.compile(r"(?i)^(?:https?:)?//(?:localhost|127\.0\.0\.1|\[::1\]|0\.0\.0\.0)(?:[:/]|$)")
EXTERNO_SIN_CODIGO = re.compile(r"(?s)<!--.*?-->|\{#.*?#\}")
NECESITA_INTERNET = re.compile(r"(?i)\b(?:necesita|requiere)n?\s+(?:de\s+)?(?:una\s+)?(?:conexi[oó]n(?:\s+a\s+internet)?|internet)\b")


def recursos_externos(*carpetas: Path) -> list[dict]:
    """Lo que las plantillas y los CSS cargan de internet con <link> o <script src> (o @import en un CSS): [{"archivo", "host", "recurso"}].
    Vacío si el README de la raíz dice que la web necesita internet. No cuenta iconos, enlaces <a>, imágenes, comentarios, localhost, tests ni dependencias."""
    for base in carpetas:
        for _, f in _archivos([base], (".md", ".txt")):
            if f.parent == base and f.name.lower().startswith("readme"):
                try:
                    if NECESITA_INTERNET.search(f.read_text(encoding="utf-8", errors="replace")):
                        return []
                except OSError:
                    pass
    hallados, vistos = [], set()
    for base, f in _archivos(carpetas, (".html", ".htm", ".jinja", ".j2", ".css")):
        if {"tests", "test"} & set(f.relative_to(base).parts):
            continue
        try:
            texto = f.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        if f.suffix.lower() == ".css":
            urls = EXTERNO_CSS.findall(texto)
        else:
            urls = []
            for m in EXTERNO_ETIQUETA.finditer(EXTERNO_SIN_CODIGO.sub(" ", texto)):
                u = EXTERNO_URL.search(m.group(2))
                if u and not EXTERNO_NO_CODIGO.search(m.group(2)):
                    urls.append(u.group(1))
        for url in urls:
            if EXTERNO_LOCAL.search(url) or (f, url) in vistos:
                continue
            vistos.add((f, url))
            host = re.sub(r"^(?:https?:)?//([^/:?#]+).*$", r"\1", url)
            hallados.append({"archivo": f.relative_to(base).as_posix(), "host": host, "recurso": url.split("?")[0].rstrip("/").rsplit("/", 1)[-1] or host})
    return hallados


# `url_for('nueva_vacuna')` en una plantilla cuando la ruta se llama `vacuna_nueva`: Flask da BuildError (500) en cuanto esa parte se pinta. En la veterinaria
# del usuario (4.13.8) los botones «Editar» y «💉 Vacuna» iban dentro de un `{% if can_edit %}` que nunca se cumplía: la página daba 200, verificar daba 0/0
# y no había forma de registrar una vacuna desde la web. Es estático: lee las rutas con `ast` y los `url_for` de las plantillas.
URL_FOR = re.compile(r"""\burl_for\(\s*(["'])([A-Za-z_][\w.]*)\1\s*[,)]""")  # no «'x' ~ n»: eso es un nombre armado en la plantilla
RUTA_METODOS = {"route", "get", "post", "put", "delete", "patch", "api_route"}
EXTENSIONES_CON_RUTAS = ("flask_restful", "flask_restx", "flask_restplus", "flask_admin", "flask_security", "flask_appbuilder", "flask_user",
                         "flask_dance", "flask_smorest", "apiflask", "connexion", "flask_openapi3", "flask_classful")
WEB_FRAMEWORK = re.compile(r"(?m)^\s*(?:from|import)\s+(?:flask|fastapi|starlette)\b")
FUERA_DE_AQUI = {".venv", "venv", "env", "node_modules", "__pycache__", "site-packages", "dist", "build", ".git", ".agente"}
JINJA_SIN_CODIGO = re.compile(r"(?s)\{#.*?#\}|\{%-?\s*raw\s*-?%\}.*?\{%-?\s*endraw\s*-?%\}")


def _literal(nodo) -> str | None:
    return nodo.value if isinstance(nodo, ast.Constant) and isinstance(nodo.value, str) else None


def _argumento(llamada: ast.Call, nombre: str):
    return next((k.value for k in llamada.keywords if k.arg == nombre), None)


def _archivos(bases, sufijos, solo_carpetas=()):
    """Archivos con esos sufijos bajo las carpetas dadas (sin entrar en entornos virtuales ni dependencias), cada uno una vez."""
    vistos = set()
    for base in bases:
        if not base.is_dir():
            continue
        for raiz, carpetas_, ficheros in os.walk(base):
            carpetas_[:] = sorted(c for c in carpetas_ if c not in FUERA_DE_AQUI)
            dentro = Path(raiz).relative_to(base).parts
            if solo_carpetas and not any(p in solo_carpetas for p in dentro):
                continue
            for nombre in sorted(ficheros):
                f = Path(raiz) / nombre
                if f.suffix.lower() in sufijos and f not in vistos:
                    vistos.add(f)
                    yield base, f


def rutas_de_la_web(*carpetas: Path) -> set[str] | None:
    """Los nombres con los que `url_for` puede llamar a una ruta (`vacuna_nueva`, `auth.login`, `static`). None si no es una web
    de la que se pueda estar seguro (sin Flask/FastAPI, sin rutas, con rutas registradas de forma dinámica o por una extensión)."""
    nombres, web, dinamico, rutas = {"static"}, False, False, 0
    for _, f in _archivos(carpetas, {".py"}):
        try:
            if f.stat().st_size > 400_000:
                continue
            texto = f.read_text(encoding="utf-8", errors="replace")
            arbol = ast.parse(texto)
        except (OSError, SyntaxError, ValueError, RecursionError):
            continue
        web = web or bool(WEB_FRAMEWORK.search(texto))
        if any(re.search(rf"(?m)^\s*(?:from|import)\s+{e}\b", texto) for e in EXTENSIONES_CON_RUTAS):
            dinamico = True
        planos = {}
        definidas = {n.name for n in ast.walk(arbol) if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef))}
        for n in ast.walk(arbol):
            if isinstance(n, ast.Assign) and isinstance(n.value, ast.Call) and n.value.args:
                llamada = n.value.func
                if (llamada.attr if isinstance(llamada, ast.Attribute) else getattr(llamada, "id", "")) == "Blueprint" and _literal(n.value.args[0]):
                    planos.update({t.id: _literal(n.value.args[0]) for t in n.targets if isinstance(t, ast.Name)})
        for n in ast.walk(arbol):
            if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)):
                for d in n.decorator_list:
                    if isinstance(d, ast.Call) and isinstance(d.func, ast.Attribute) and d.func.attr in RUTA_METODOS:
                        nombre = _literal(_argumento(d, "endpoint")) or _literal(_argumento(d, "name")) or n.name
                        dueno = d.func.value.id if isinstance(d.func.value, ast.Name) else ""
                        nombres.add(nombre)
                        rutas += 1
                        if dueno in planos:
                            nombres.add(f"{planos[dueno]}.{nombre}")
            elif isinstance(n, ast.Call):
                llamada = n.func
                metodo = llamada.attr if isinstance(llamada, ast.Attribute) else getattr(llamada, "id", "")
                if metodo == "add_url_rule":
                    rutas += 1
                    vista = _argumento(n, "view_func") or (n.args[2] if len(n.args) > 2 else None)
                    nombre = _literal(_argumento(n, "endpoint")) or (_literal(n.args[1]) if len(n.args) > 1 else None)
                    if nombre is None and isinstance(vista, ast.Call) and getattr(vista.func, "attr", "") == "as_view" and vista.args:
                        nombre = _literal(vista.args[0])
                    if nombre is None and isinstance(vista, ast.Name) and vista.id in definidas:
                        nombre = vista.id  # una variable de un bucle no dice cómo se llama la ruta
                    if nombre:
                        nombres.add(nombre)
                    else:
                        dinamico = True
                elif metodo == "as_view" and n.args and _literal(n.args[0]):
                    nombres.add(_literal(n.args[0]))
                elif metodo == "Rule" and _argumento(n, "endpoint") is not None and not _literal(_argumento(n, "endpoint")):
                    dinamico = True
                elif metodo in ("mount", "Mount", "Route") and _literal(_argumento(n, "name")):
                    nombres.add(_literal(_argumento(n, "name")))
        if "view_functions" in texto or "url_map" in texto:
            dinamico = True
    return nombres if (web and rutas and not dinamico) else None


def url_for_sin_ruta(*carpetas: Path) -> list[dict]:
    """`url_for('nombre')` de las plantillas que no corresponde a ninguna ruta de la web: [{"plantilla", "linea", "endpoint", "sugerencia"}].
    Con nombre de Blueprint (`auth.login`) solo se mira el último trozo. Solo habla si está seguro de cuáles son las rutas (rutas_de_la_web)."""
    nombres = rutas_de_la_web(*carpetas)
    if nombres is None:
        return []
    sueltos = {n.rsplit(".", 1)[-1] for n in nombres}
    hallazgos, vistos = [], set()
    for base, f in _archivos(carpetas, {".html", ".htm", ".jinja", ".j2"}, ("templates", "plantillas")):
        try:
            texto = f.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        texto = JINJA_SIN_CODIGO.sub(lambda m: "\n" * m.group(0).count("\n"), texto)
        for m in URL_FOR.finditer(texto):
            endpoint = m.group(2)
            ultimo = endpoint.rsplit(".", 1)[-1]
            if endpoint in nombres or ultimo in sueltos or ultimo == "static" or (f.relative_to(base).as_posix(), endpoint) in vistos:
                continue
            vistos.add((f.relative_to(base).as_posix(), endpoint))
            palabras = frozenset(endpoint.split("_"))
            parecidas = [s for s in sorted(sueltos) if frozenset(s.split("_")) == palabras and s != endpoint]
            parecidas = parecidas or difflib.get_close_matches(endpoint, sorted(sueltos - {"static"}), n=1, cutoff=0.6)
            hallazgos.append({"plantilla": f.relative_to(base).as_posix(), "linea": texto.count("\n", 0, m.start()) + 1,
                              "endpoint": endpoint, "sugerencia": parecidas[0] if parecidas else ""})
    return hallazgos


# Una ruta que lee `request.form["nombre_completo"]` y un formulario que no trae ese campo: Flask da 400 Bad Request (KeyError) en cada envío. En la veterinaria
# del usuario (4.13.8) el administrador no podía crear usuarios: el formulario de /usuarios/nuevo solo tenía username, password, rol y activo. Es estático y solo
# habla si está seguro: la ruta exige el campo siempre (no dentro de un if, un bucle ni un try), el formulario va seguro a esa ruta, y no hay JavaScript ni
# nombres dinámicos que puedan añadir campos.
CAMPOS_FORM_ABRE = re.compile(r"(?i)<form\b([^>]*)>")
CAMPOS_FORM_METODO = re.compile(r"""(?i)\bmethod\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+))""")
CAMPOS_FORM_ACCION = re.compile(r"""(?i)\baction\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+))""")
CAMPOS_URL_FOR = re.compile(r"""\burl_for\(\s*(["'])([A-Za-z_][\w.]*)\1""")
CAMPOS_ETIQUETA_REF = re.compile(r"(?s)\{%-?\s*(extends|include|import|from)\b(.*?)-?%\}")
CAMPOS_NOMBRE_DINAMICO = re.compile(r"""(?i)\bname\s*=\s*["']?[^"'>\s]*\{[{%]""")
CAMPOS_JS_ARMA = re.compile(r"""FormData|\.append\(|preventDefault|createElement\(\s*["'](?:input|select|textarea)|insertAdjacentHTML|"""
                            r"""\.setAttribute\(\s*["']name|\.name\s*=""")
CAMPOS_COMENTARIO_JINJA = re.compile(r"(?s)\{#.*?#\}")
CAMPOS_WTF = re.compile(r"(?m)^\s*(?:from|import)\s+(?:flask_wtf|wtforms)\b")
CAMPOS_SCRIPT_LOCAL = re.compile(r"""(?is)<script\b[^>]*?\bsrc\s*=\s*(?:"([^"]*)"|'([^']*)')""")
# Lo que en una plantilla es código y puede nombrar un campo: scripts, expresiones y etiquetas de Jinja y atributos on...= (el resto es HTML plano)
CAMPOS_CODIGO = re.compile(r"""(?is)<script\b[^>]*>.*?</script\s*>|\{\{.*?\}\}|\{%.*?%\}|\bon\w+\s*=\s*(?:"[^"]*"|'[^']*')""")
CAMPOS_NOMBRE_ATRIBUTO = re.compile(r"""(?i)\bname\s*=\s*["']?([\w.-]+)""")
CAMPOS_SI_POST = re.compile(r"""^(?:request\.method\s*==\s*["']POST["']|["']POST["']\s*==\s*request\.method)$""")


def _es_request_form(nodo) -> bool:
    return isinstance(nodo, ast.Attribute) and nodo.attr == "form" and isinstance(nodo.value, ast.Name) and nodo.value.id == "request"


def _form_exigidos(funcion) -> set[str]:
    """Campos `request.form["k"]` que la función lee siempre que se hace el POST (sin if, bucle, try ni expresión condicional de por medio)
    y que no se comprueban en ningún sitio de la función (`"k" in request.form`, `request.form.get("k")`)."""
    protegidos = set()
    for n in ast.walk(funcion):
        if isinstance(n, ast.Compare) and any(_es_request_form(c) for c in n.comparators) and _literal(n.left):
            protegidos.add(_literal(n.left))
        elif isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute) and _es_request_form(n.func.value) and n.args and _literal(n.args[0]):
            protegidos.add(_literal(n.args[0]))
    exigidos: set[str] = set()
    condicionales = (ast.IfExp, ast.BoolOp, ast.Lambda, ast.ListComp, ast.SetComp, ast.DictComp, ast.GeneratorExp, ast.NamedExpr)
    saltar = (ast.For, ast.AsyncFor, ast.While, ast.Try, getattr(ast, "TryStar", ast.Try), ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef, ast.Match)

    def expresion(nodo) -> None:
        if isinstance(nodo, condicionales):
            return
        if isinstance(nodo, ast.Subscript) and _es_request_form(nodo.value) and _literal(nodo.slice):
            exigidos.add(_literal(nodo.slice))
        for hijo in ast.iter_child_nodes(nodo):
            expresion(hijo)

    def cuerpo(sentencias) -> None:
        for s in sentencias:
            if isinstance(s, ast.If):
                if CAMPOS_SI_POST.match(ast.unparse(s.test).strip()):
                    cuerpo(s.body)
            elif isinstance(s, ast.With):
                cuerpo(s.body)
            elif not isinstance(s, saltar):
                expresion(s)

    cuerpo(funcion.body)
    return exigidos - protegidos


def _clave_del_get(nodo) -> str | None:
    """`request.form.get("k")` (o con un valor por defecto vacío, y con .strip()/.lower()/.upper()/.title() encadenados): "k". Con un valor
    por defecto que no sea vacío el campo es opcional (`get("rol", "empleado")`): None."""
    while (isinstance(nodo, ast.Call) and isinstance(nodo.func, ast.Attribute) and nodo.func.attr in ("strip", "lower", "upper", "title")
           and not nodo.args):
        nodo = nodo.func.value
    if not (isinstance(nodo, ast.Call) and isinstance(nodo.func, ast.Attribute) and nodo.func.attr == "get"
            and _es_request_form(nodo.func.value) and nodo.args and not nodo.keywords and len(nodo.args) <= 2):
        return None
    if len(nodo.args) == 2 and not (isinstance(nodo.args[1], ast.Constant) and not nodo.args[1].value):
        return None
    return _literal(nodo.args[0])


def _form_validados(funcion) -> set[str]:
    """Campos que la función lee con `request.form.get("k")` en una variable y rechaza si vienen vacíos, siempre que se hace el POST:
    `if not v: ... return|raise|abort` o `if not v or not w: ...` (con `or` cada uno es obligatorio; con `and`, ninguno). La ferretería (18)
    leía `password` y el formulario enviaba `contrasena`: cada envío acababa en «usuario y contraseña son obligatorios»."""
    variables: dict[str, str] = {}
    validados: set[str] = set()

    def rechaza(sentencias) -> bool:
        return any(isinstance(s, (ast.Return, ast.Raise)) or (isinstance(s, ast.Expr) and isinstance(s.value, ast.Call)
                                                              and getattr(s.value.func, "id", getattr(s.value.func, "attr", "")) == "abort")
                   for s in sentencias)

    def vacias(test) -> list[str]:
        if isinstance(test, ast.UnaryOp) and isinstance(test.op, ast.Not) and isinstance(test.operand, ast.Name):
            return [test.operand.id]
        if isinstance(test, ast.BoolOp) and isinstance(test.op, ast.Or):
            return [v for x in test.values for v in vacias(x)]
        return []

    def cuerpo(sentencias) -> None:
        for s in sentencias:
            if isinstance(s, ast.If):
                if CAMPOS_SI_POST.match(ast.unparse(s.test).strip()):
                    cuerpo(s.body)
                elif rechaza(s.body):
                    validados.update(variables[v] for v in vacias(s.test) if v in variables)
            elif isinstance(s, ast.With):
                cuerpo(s.body)
            elif isinstance(s, ast.Assign) and len(s.targets) == 1 and isinstance(s.targets[0], ast.Name):
                clave = _clave_del_get(s.value)
                if clave:
                    variables[s.targets[0].id] = clave
                else:
                    variables.pop(s.targets[0].id, None)  # ya no es el campo del formulario

    cuerpo(funcion.body)
    return validados


def _rutas_con_exigidos(carpetas) -> tuple[list[dict], dict[str, int]] | None:
    """Las rutas Flask que aceptan POST y leen campos con request.form["k"], y cuántas rutas hay por nombre de endpoint (para saber si url_for es ambiguo).
    None si la web usa flask_wtf/wtforms (sus campos no se ven en la plantilla)."""
    rutas, por_endpoint = [], {}
    for _, f in _archivos(carpetas, {".py"}):
        try:
            if f.stat().st_size > 400_000:
                continue
            texto = f.read_text(encoding="utf-8", errors="replace")
            arbol = ast.parse(texto)
        except (OSError, SyntaxError, ValueError, RecursionError):
            continue
        if CAMPOS_WTF.search(texto):
            return None
        apps = set()
        for n in ast.walk(arbol):
            if isinstance(n, ast.Assign) and isinstance(n.value, ast.Call):
                llamada = n.value.func
                if (llamada.attr if isinstance(llamada, ast.Attribute) else getattr(llamada, "id", "")) == "Flask":
                    apps.update(t.id for t in n.targets if isinstance(t, ast.Name))
        for n in ast.walk(arbol):
            if not isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)):
                continue
            reglas_post, endpoint, es_ruta, desconocida = [], n.name, False, False
            for d in n.decorator_list:
                if not (isinstance(d, ast.Call) and isinstance(d.func, ast.Attribute) and d.func.attr in RUTA_METODOS):
                    continue
                es_ruta = True
                endpoint = _literal(_argumento(d, "endpoint")) or _literal(_argumento(d, "name")) or endpoint
                regla = _literal(d.args[0]) if d.args else None
                if d.func.attr == "route":
                    metodos = _argumento(d, "methods")
                    if metodos is None:
                        continue
                    if not isinstance(metodos, (ast.List, ast.Tuple, ast.Set)) or not all(_literal(e) for e in metodos.elts):
                        desconocida = True
                        continue
                    lleva_post = any(_literal(e).upper() == "POST" for e in metodos.elts)
                else:
                    lleva_post = d.func.attr == "post"
                if lleva_post and regla is not None:
                    reglas_post.append((regla, isinstance(d.func.value, ast.Name) and d.func.value.id in apps))
            if not es_ruta:
                continue
            por_endpoint[endpoint] = por_endpoint.get(endpoint, 0) + 1
            exigidos = _form_exigidos(n) if reglas_post and not desconocida else set()
            validados = _form_validados(n) - exigidos if reglas_post and not desconocida else set()
            if exigidos or validados:
                dibuja = {_literal(c.args[0]) for c in ast.walk(n) if isinstance(c, ast.Call) and c.args and _literal(c.args[0])
                          and getattr(c.func, "id", getattr(c.func, "attr", "")) == "render_template"}
                rutas.append({"funcion": n.name, "endpoint": endpoint, "reglas": reglas_post, "exigidos": exigidos, "validados": validados,
                              "dibuja": dibuja})
    return rutas, por_endpoint


def _regla_a_regex(regla: str) -> re.Pattern:
    return re.compile(re.sub(r"<[^>]+>", "[^/]+", re.escape(regla)).rstrip("/") + "/?")


def campos_sin_formulario(*carpetas: Path, motivos: dict | None = None) -> list[dict]:
    """Campos que una ruta exige con `request.form["k"]` y que el formulario que envía a esa ruta no tiene:
    [{"plantilla", "linea", "ruta", "funcion", "campo"}]. Solo habla si está seguro (ver el comentario de arriba).
    `motivos`, si se da, cuenta por qué se calló cada formulario (para medir el alcance)."""
    def nota(clave: str, n: int = 1) -> None:
        if motivos is not None:
            motivos[clave] = motivos.get(clave, 0) + n

    if rutas_de_la_web(*carpetas) is None:
        nota("web_no_segura")
        return []
    leidas = _rutas_con_exigidos(carpetas)
    if leidas is None:
        nota("wtforms")
        return []
    if not leidas[0]:
        nota("sin_rutas_con_exigidos")
        return []
    rutas, por_endpoint = leidas
    nota("rutas_con_exigidos", len(rutas))
    plantillas, bases = _plantillas_por_clave(carpetas)
    textos: dict[Path, str] = {}

    def texto(f: Path) -> str:
        if f not in textos:
            try:
                textos[f] = CAMPOS_COMENTARIO_JINJA.sub(lambda m: "\n" * m.group(0).count("\n"), f.read_text(encoding="utf-8", errors="replace"))
            except OSError:
                textos[f] = ""
        return textos[f]

    def cierre(f: Path, padres: bool, visto: set | None = None) -> list[Path] | None:
        """La plantilla y lo que lleva dentro (include, import, from y, si padres, extends). None si alguna referencia no se puede seguir."""
        visto = set() if visto is None else visto
        if f in visto:
            return []
        visto.add(f)
        salida = [f]
        for m in CAMPOS_ETIQUETA_REF.finditer(texto(f)):
            if m.group(1) == "extends" and not padres:
                continue
            ref = re.match(r"""\s*(["'])([^"']+)\1(.*)$""", m.group(2), re.DOTALL)
            if not ref or re.search(r"\b(?:if|else)\b|[~+\[]", ref.group(3)):
                return None
            destino = plantillas.get(ref.group(2))
            resto = cierre(destino, padres, visto) if destino is not None else None
            if resto is None:
                return None
            salida += resto
        return salida

    js_por_nombre: dict[str, list[Path]] = {}

    def js_de(archivos_: list[Path]) -> list[str]:
        """Los textos de los .js del proyecto que esas plantillas cargan con <script src=...> (m4148_neg4: static/js/ventas.js crea el campo
        oculto `items` al enviar y la plantilla no lo tiene)."""
        if not js_por_nombre:
            js_por_nombre["."] = []
            for _, j in _archivos(carpetas, {".js"}):
                js_por_nombre.setdefault(j.name, []).append(j)
        salida: list[str] = []
        for a in archivos_:
            for m in CAMPOS_SCRIPT_LOCAL.finditer(texto(a)):
                nombres = re.findall(r"([\w.-]+\.js)\b", m.group(1) if m.group(1) is not None else m.group(2))
                for j in js_por_nombre.get(nombres[-1], []) if nombres else []:
                    try:
                        if j.stat().st_size <= 400_000:
                            salida.append(j.read_text(encoding="utf-8", errors="replace"))
                    except OSError:
                        continue
        return salida

    def tiene(archivos_: list[Path], campo: str) -> bool:
        c = re.escape(campo)  # el literal entre comillas solo cuenta en código: id="password", for=, placeholder= o type="password" no son el campo
        nombre, literal = rf"""(?i)\bname\s*=\s*["']?{c}(?![\w.-])""", rf"""["']{c}["']"""
        return (any(re.search(nombre, texto(a)) or re.search(literal, " ".join(m.group(0) for m in CAMPOS_CODIGO.finditer(texto(a))))
                    for a in archivos_)
                or any(re.search(nombre, t) or re.search(literal, t) for t in js_de(archivos_)))

    dibujadas: dict[int, set[Path]] = {}  # por ruta: los archivos que pinta su plantilla (ella y lo que incluye)
    for r in rutas:
        pintados = set()
        for nombre in r["dibuja"]:
            destino = plantillas.get(nombre)
            propios = cierre(destino, False) if destino is not None else None
            pintados |= set(propios or [])
        dibujadas[id(r)] = pintados

    hallazgos, vistos = [], set()
    for f in sorted(bases):
        if "<form" not in texto(f).lower():
            continue
        propios, todos = cierre(f, False), cierre(f, True)
        if propios is None or todos is None:
            nota("plantilla_no_seguible")
            continue
        if (any(CAMPOS_JS_ARMA.search(texto(a)) for a in propios) or any(CAMPOS_JS_ARMA.search(t) for t in js_de(propios))
                or any(CAMPOS_NOMBRE_DINAMICO.search(texto(a)) for a in todos)):
            nota("js_o_nombre_dinamico")
            continue
        rel = f.relative_to(bases[f]).as_posix()
        t = texto(f)
        for m in CAMPOS_FORM_ABRE.finditer(t):
            metodo = CAMPOS_FORM_METODO.search(m.group(1))
            if not metodo or next((g for g in metodo.groups() if g), "").strip().lower() != "post":
                nota("form_no_post")
                continue
            accion = CAMPOS_FORM_ACCION.search(m.group(1))
            accion = next((g for g in accion.groups() if g is not None), "").strip() if accion else ""
            uf = CAMPOS_URL_FOR.search(accion)
            if accion in ("", "#"):  # envía a la ruta que pinta esta plantilla
                destinos = [r for r in rutas if f in dibujadas[id(r)]]
            elif uf:
                ultimo = uf.group(2).rsplit(".", 1)[-1]
                hay = sum(v for e, v in por_endpoint.items() if e.rsplit(".", 1)[-1] == ultimo)
                destinos = [r for r in rutas if r["endpoint"].rsplit(".", 1)[-1] == ultimo] if hay == 1 else []
            elif accion.startswith("/"):
                literal = re.sub(r"\{\{.*?\}\}", "0", accion.split("?")[0].split("#")[0])
                destinos = [r for r in rutas if any(es_app and _regla_a_regex(regla).fullmatch(literal) for regla, es_app in r["reglas"])]
                destinos = destinos if len(destinos) == 1 else []
            else:
                destinos = []
            nota("form_post_con_ruta_exigente" if destinos else "form_post_sin_ruta_exigente")
            for r in destinos:
                aqui = todos + [a for a in dibujadas[id(r)] if a not in todos] if accion in ("", "#") else todos
                fin = t.lower().find("</form", m.end())
                suyos = list(dict.fromkeys(n for n in CAMPOS_NOMBRE_ATRIBUTO.findall(t[m.end(): fin if fin >= 0 else len(t)])
                                           if not re.search(r"(?i)csrf", n)))[:8]
                for campo, tipo in sorted([(c, "indexa") for c in r["exigidos"]] + [(c, "valida") for c in r["validados"]]):
                    if tiene(aqui, campo) or (rel, r["funcion"], campo) in vistos:
                        continue
                    vistos.add((rel, r["funcion"], campo))
                    hallazgos.append({"plantilla": rel, "linea": t.count("\n", 0, m.start()) + 1, "ruta": r["reglas"][0][0],
                                      "funcion": r["funcion"], "campo": campo, "tipo": tipo, "formulario": suyos})
    return hallazgos


# Una variable que la plantilla lee y que ninguna ruta le pasa: Jinja no da error, la lista sale vacía, el «if» es falso y el texto sale en blanco. En la
# veterinaria del usuario (4.13.8) los botones «Editar» y «💉 Vacuna» iban dentro de un `{% if can_edit %}` que nadie definía y el desplegable de vacunas
# iteraba `vacunas`, que la ruta no pasaba. La sonda (sondas/web_arranque.py, con el Python del proyecto) dice qué nombres de cada plantilla no define el
# entorno ni un procesador de contexto; aquí se mira, leyendo las render_template, si alguna ruta se los pasa.
RENDER_DINAMICO = re.compile(r"render_template_string|get_template\(|stream_template|select_template|\bTemplate\(|render_template\s+as\s")
IDENTIFICADOR = re.compile(r"[A-Za-z_]\w*")
JS_CARGA = re.compile(r"\bfetch\s*\(|XMLHttpRequest|\$\.(?:get|post|ajax|getJSON)\b|\baxios\b")  # una tabla que rellena JavaScript


def _plantillas_por_clave(carpetas) -> tuple[dict[str, Path | None], dict[Path, Path]]:
    """Las plantillas del proyecto por su nombre para render_template (la ruta después de la última carpeta templates; None si el nombre
    está repetido) y la carpeta base de cada archivo."""
    plantillas: dict[str, Path | None] = {}
    bases: dict[Path, Path] = {}
    for base, f in _archivos(carpetas, {".html", ".htm", ".jinja", ".j2"}, ("templates", "plantillas")):
        partes = f.relative_to(base).parts
        i = max(k for k, p in enumerate(partes[:-1]) if p in ("templates", "plantillas"))
        clave = "/".join(partes[i + 1:])
        plantillas[clave] = None if clave in plantillas else f
        bases[f] = base
    return plantillas, bases


def _lo_que_pasan_las_rutas(carpetas) -> tuple[dict[str, set[str]], set[str]] | None:
    """Por plantilla (literal de render_template): los nombres que alguna ruta le pasa, y las plantillas con `**` o argumentos de más
    (no se sabe qué reciben). None si alguna plantilla se pinta de forma que no se puede leer (nombre no literal, render_template_string...)."""
    pasan: dict[str, set[str]] = {}
    con_extra: set[str] = set()
    for _, f in _archivos(carpetas, {".py"}):
        try:
            if f.stat().st_size > 400_000:
                continue
            texto = f.read_text(encoding="utf-8", errors="replace")
            arbol = ast.parse(texto)
        except (OSError, SyntaxError, ValueError, RecursionError):
            continue
        if RENDER_DINAMICO.search(texto):
            return None
        for n in ast.walk(arbol):
            if isinstance(n, ast.Call) and getattr(n.func, "id", getattr(n.func, "attr", "")) == "render_template":
                nombre = _literal(n.args[0]) if n.args else None
                if nombre is None:
                    return None
                pasan.setdefault(nombre, set())
                con_extra.update([nombre] if len(n.args) > 1 else [])
                for k in n.keywords:
                    if k.arg is None:
                        con_extra.add(nombre)
                    else:
                        pasan[nombre].add(k.arg)
    return pasan, con_extra


def _definidos_en_plantillas(carpetas) -> set[str]:
    """Nombres que alguna plantilla define con set, for, with, macro, import o from (lo que un padre o un include puede dar a la hija)."""
    nombres: set[str] = set()
    for _, f in _archivos(carpetas, {".html", ".htm", ".jinja", ".j2"}, ("templates", "plantillas")):
        try:
            texto = f.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        for m in re.finditer(r"(?s)\{%-?\s*(set|for|with|macro|import|from|call)\b(.*?)-?%\}", texto):
            tipo, resto = m.groups()
            if tipo == "set":
                nombres.update(IDENTIFICADOR.findall(resto.split("=", 1)[0]))
            elif tipo == "for":
                nombres.update(IDENTIFICADOR.findall(re.split(r"\bin\b", resto, maxsplit=1)[0]))
            elif tipo == "with":
                nombres.update(re.findall(r"(\w+)\s*=(?!=)", resto))
            else:
                nombres.update(IDENTIFICADOR.findall(resto))
    return nombres


def variables_sin_pasar(sonda: list[dict], *carpetas: Path) -> list[dict]:
    """Nombres de plantilla que ninguna ruta pasa (`sonda`: lo que dijo web_arranque, [{"plantilla", "nombres": {nombre: {"linea", "uso"}}}]):
    [{"plantilla", "nombre", "linea", "uso", "pasan", "sugerencia", "con_js"}]. Solo habla de plantillas que alguna render_template pinta con su nombre
    literal y sin `**`, y solo si la web no pinta plantillas de otra forma."""
    if not sonda:
        return []
    leidas = _lo_que_pasan_las_rutas(carpetas)
    if leidas is None:
        return []
    pasan, con_extra = leidas
    definidos = _definidos_en_plantillas(carpetas)
    archivos, _ = _plantillas_por_clave(carpetas)
    hallazgos = []
    for entrada in sonda:
        plantilla = entrada["plantilla"]
        if plantilla not in pasan or plantilla in con_extra:
            continue
        try:
            con_js = bool(archivos.get(plantilla) and JS_CARGA.search(archivos[plantilla].read_text(encoding="utf-8", errors="replace")))
        except OSError:
            con_js = False
        for nombre, info in entrada["nombres"].items():
            if nombre in pasan[plantilla] or nombre in definidos:
                continue
            parecidas = difflib.get_close_matches(nombre, sorted(pasan[plantilla]), n=1, cutoff=0.6)
            hallazgos.append({"plantilla": plantilla, "nombre": nombre, "linea": info["linea"], "uso": info["uso"],
                              "pasan": sorted(pasan[plantilla]), "sugerencia": parecidas[0] if parecidas else "", "con_js": con_js})
    return hallazgos


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
        base = f"http://127.0.0.1:{puerto}/"
        # El empleado se prueba ANTES del recorrido de siempre y con una sesión propia: ese recorrido envía todos los formularios,
        # también el de editar usuarios (rol y «activo» con la primera opción), y puede dejar al administrador sin permisos.
        previo = _Recorrido(base, log, accesos)
        empleado = None
        try:
            previo.explorar(time.time() + MAX_SEGUNDOS_EXPLORAR)
            if previo.usuario and ES_ADMIN.match(previo.usuario) and not previo.entrar_js and previo.entrar_ruta:
                empleado = _empleado(previo, time.time() + MAX_SEGUNDOS_EMPLEADO)
            else:
                previo.sin_empleado = ("el recorrido sin JavaScript no entró como administrador"
                                       if not (previo.usuario and ES_ADMIN.match(previo.usuario)) else "la entrada usa JavaScript")
        except Exception as exc:  # una web rara no debe estropear el recorrido
            empleado, previo.sin_empleado = None, f"excepción {type(exc).__name__}: {exc}"
        # La pantalla de venta también va antes del recorrido de siempre, por lo mismo: ese recorrido puede cambiar la contraseña
        # del administrador (el formulario de editar usuarios) y el navegador ya no entraría. Los productos que tiene son los del empleado.
        venta = None
        if abrir_navegador is not None and not previo.entrar_js and (previo.usuario or not previo.intento_entrar):
            vistas = [x for x in dict.fromkeys(previo.orden) if not CON_ID.search(x.split("?")[0])]
            alta = [x for x in vistas if VENTA_ALTA.search(x.split("?")[0])][:3]
            ventas = [x for x in vistas if VENTA_LISTA.search(x.split("?")[0])][:1]
            productos = [x for x in vistas if PRODUCTO_RUTA.search(x.split("?")[0]) and not FORMULARIO_RUTA.search(x.split("?")[0])][:1]
            try:
                # La lista de ventas solo si dice «nueva venta» en algún sitio (sin lanzar el navegador para nada).
                paneles = [x for x in ventas if NUEVA_VENTA.search(_visible(previo.pedir(x)[2]))]
                if alta or paneles:
                    if not _hay_productos(previo):  # sin productos no se prueba la venta: se da de alta uno desde el formulario de la web
                        _producto_de_prueba(previo, time.time() + MAX_SEGUNDOS_PRODUCTO)
                    venta = _venta_en_pantalla(base, log, previo.usuario, previo.clave, abrir_navegador,
                                               time.time() + MAX_SEGUNDOS_VENTA, alta, _hay_productos(previo), ventas + productos, paneles)
            except Exception:  # una web rara no debe estropear el recorrido
                venta = None
        r = _Recorrido(base, log, accesos)
        r.recorrer(time.time() + MAX_SEGUNDOS)
        lineas = None
        if abrir_navegador is not None and r.filas_js and not r.entrar_js and (r.usuario or not r.intento_entrar):
            lineas = _lineas_con_navegador(r.base, log, r.usuario, r.clave, abrir_navegador,
                                           time.time() + MAX_SEGUNDOS_LINEAS, list(r.filas_js)[:3])
            if lineas:
                r.errores += [e for e in lineas["errores"] if e not in r.errores]
        if venta:  # el mismo fallo que ya dio otro paso (con la misma traza) no se cuenta dos veces
            r.errores += [e for e in venta.pop("servidor") if e not in r.errores
                          and not any(e["traza"] and e["traza"] == x["traza"] and e["estado"] == x["estado"] for x in r.errores)]
        try:  # cuando ya hay ventas (las de los formularios y las que el navegador armó con JavaScript)
            _empleado_despues(r, previo, empleado, time.time() + MAX_SEGUNDOS_EMPLEADO)
        except Exception:  # una web rara no debe estropear el recorrido
            pass
        negocio = {}
        try:
            negocio = _informes_y_facturas(r, time.time() + MAX_SEGUNDOS_EMPLEADO)
        except Exception:
            pass
        navegador = None
        # Lo de dentro solo se ve con un navegador si la entrada es JavaScript. También se prueba con él si no se pudo
        # entrar: un formulario con name que JavaScript enviaba como JSON no dejaba entrar al recorrido, y a una persona sí.
        # También cuando las altas son un modal que guarda con fetch (un botón «Nuevo» fuera de todo formulario) y el recorrido
        # sin JavaScript no pudo enviar ningún formulario: no habría creado nada ni visto las páginas que fallan con datos.
        solo_altas = r.altas_js and r.posts == 0 and not r.entrar_js and bool(r.usuario or not r.intento_entrar)
        if abrir_navegador is not None and (r.entrar_js or solo_altas or (r.intento_entrar and not r.usuario and not r.entrar_fallo)):
            # Si solo es para ver si se puede entrar, con poco tiempo: una web así tardaba 30 s en recorrerse entera.
            plazo = MAX_SEGUNDOS if r.entrar_js else MAX_SEGUNDOS_ALTAS if solo_altas else MAX_SEGUNDOS_REINTENTO
            navegador = _con_navegador(r.base, log, accesos, abrir_navegador, time.time() + plazo)
            if navegador:
                r.errores += [e for e in navegador.pop("errores") if e not in r.errores]
                if solo_altas:  # ya se entró sin JavaScript: que el navegador no lo consiga no es «nadie puede entrar»
                    navegador["sin_entrar"] = False
                r.usuario = navegador["usuario"] or r.usuario
        texto = log.read_text(encoding="utf-8", errors="replace")
        plantillas = [json.loads(m) for m in re.findall(r"^WEB_PLANTILLA (\{.*\})\s*$", texto, re.MULTILINE)]
        variables = [json.loads(m) for m in re.findall(r"^WEB_VARIABLE (\{.*\})\s*$", texto, re.MULTILINE)]
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
                "navegador": navegador, "lineas": lineas, "sin_formulario": sin_formulario, "registro": r.registro,
                "entrar_get": r.entrar_get, "sin_permiso": r.sin_permiso, "login_vacio": r.login_vacio, "empleado": empleado,
                "empleado_motivo": previo.sin_empleado, "negocio": negocio, "venta": venta, "variables": variables,
                "etiquetas": r.etiquetas}
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
