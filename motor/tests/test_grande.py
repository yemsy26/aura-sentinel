"""Modo grande, mapa del repositorio y cobertura de idiomas (sin modelo): python -m unittest tests.test_grande -v"""
import json
import os
import sys
import tempfile
import time
import unittest
from pathlib import Path
from typing import ClassVar
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_agente import borrar, sin_lecciones_reales

from agente import config, grande, idiomas, mapa
from agente.herramientas import ESQUEMA_MAPA, ESQUEMAS, Herramientas, despachar
from agente.verificacion import Informe, normalizar

setUpModule = sin_lecciones_reales

GIT = {"GIT_AUTHOR_NAME": "prueba", "GIT_AUTHOR_EMAIL": "prueba@local", "GIT_COMMITTER_NAME": "prueba",
       "GIT_COMMITTER_EMAIL": "prueba@local"}


def escribir(raiz: Path, rel: str, texto: str) -> Path:
    p = raiz / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(texto, encoding="utf-8")
    return p


class Temporal(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_grande_"))

    def tearDown(self):
        borrar(self.dir)


# ---------------------------------------------------------------- mapa
class TestMapa(Temporal):
    def test_simbolos_modulos_y_detalle(self):
        escribir(self.dir, "main.py", "from ventas import vender\n\ndef main():\n    vender()\n")
        escribir(self.dir, "ventas/servicio.py", "TASA = 21\n\nclass Caja:\n    def cobrar(self):\n        pass\n\n"
                                                 "def vender():\n    return Caja()\n")
        escribir(self.dir, "web/app.js", "export function iniciar() {}\nclass Vista {}\n")
        escribir(self.dir, "node_modules/x/index.js", "function no_cuenta() {}\n")
        escribir(self.dir, "package-lock.json", "{}")
        resumen = mapa.texto(self.dir)
        self.assertIn("3 archivos de código", resumen)
        self.assertIn("- ventas/ (1 arch., 8 lín.): servicio.py", resumen)
        self.assertNotIn("node_modules", resumen)
        detalle = mapa.texto(self.dir, "ventas")
        self.assertIn("ventas/servicio.py (8 lín.): TASA, class Caja(cobrar), vender()", detalle)
        self.assertIn("iniciar, Vista", mapa.texto(self.dir, "web/app.js"))
        self.assertIn("No hay archivos", mapa.texto(self.dir, "nada"))

    def test_cache_incremental(self):
        for i in range(5):
            escribir(self.dir, f"m{i}.py", f"def f{i}():\n    return {i}\n")
        mapa.actualizar(self.dir)
        self.assertTrue((self.dir / ".agente" / "mapa.json").exists())
        p = escribir(self.dir, "m0.py", "def nueva():\n    return 0\n")
        os.utime(p, ns=(time.time_ns(), time.time_ns() + 10_000_000))
        (self.dir / "m4.py").unlink()
        llamadas = []
        real = mapa._simbolos
        with mock.patch.object(mapa, "_simbolos", lambda n, t: llamadas.append(n) or real(n, t)):
            archivos = mapa.actualizar(self.dir)
        self.assertEqual(llamadas, ["m0.py"])  # solo lo que cambió
        self.assertEqual(archivos["m0.py"][3], ["nueva()"])
        self.assertNotIn("m4.py", archivos)

    def test_grande_y_modulos_profundos(self):
        self.assertFalse(mapa.es_grande(self.dir))
        for i in range(24):
            escribir(self.dir, f"src/app/mod{i % 4}/f{i}.py", "x = 1\n")
        self.assertFalse(mapa.es_grande(self.dir))
        escribir(self.dir, "src/app/mod0/ultimo.py", "x = 1\n")
        self.assertTrue(mapa.es_grande(self.dir))
        for i in range(30):
            escribir(self.dir, f"src/app/mod{i % 4}/g{i}.py", "y = 2\n")
        nombres = [n for n, _ in mapa.modulos(list(mapa.actualizar(self.dir)))]
        self.assertEqual(nombres, ["src/app/mod0/", "src/app/mod1/", "src/app/mod2/", "src/app/mod3/"])

    def test_rendimiento_50000_lineas(self):
        cuerpo = "".join(f"def funcion_{j}(a, b):\n    return a + b * {j}\n\n\n" for j in range(50))  # 200 líneas
        clase = "class Servicio:\n" + "".join(f"    def metodo_{j}(self):\n        return {j}\n\n" for j in range(30))
        for i in range(250):
            escribir(self.dir, f"mod{i % 10}/archivo_{i}.py", cuerpo if i % 5 else clase + cuerpo[:3000])
        t0 = time.perf_counter()
        archivos = mapa.actualizar(self.dir)
        frio = time.perf_counter() - t0
        t0 = time.perf_counter()
        mapa.actualizar(self.dir)
        caliente = time.perf_counter() - t0
        lineas = sum(a[2] for a in archivos.values())
        self.assertGreater(lineas, 45_000)
        self.assertLess(frio, 4.0, f"en frío {frio:.2f}s")
        self.assertLess(caliente, 0.5, f"en caliente {caliente:.2f}s")
        print(f"\n  mapa de {lineas} líneas: {frio:.2f}s en frío, {caliente * 1000:.0f} ms en caliente", file=sys.stderr)

    def test_herramienta_y_esquema_barato(self):
        h = Herramientas(self.dir)
        escribir(self.dir, "a/b.py", "def f():\n    pass\n")
        self.assertIn("a/ (1 arch.", despachar(h, "mapa", {}))
        self.assertIn("f()", despachar(h, "mapa", {"ruta": "a"}))
        self.assertLess(len(json.dumps(ESQUEMA_MAPA, ensure_ascii=False)) / 3, 150)  # ~tokens, límite pedido 250
        self.assertNotIn("mapa", [e["function"]["name"] for e in ESQUEMAS])


# ---------------------------------------------------------------- idiomas
class TestIdiomas(Temporal):
    def informe(self, parcial=None) -> Informe:
        inf = Informe()
        idiomas.verificar_idiomas(self.dir, inf, parcial)
        return inf

    def test_sin_archivos_de_idioma_no_hace_nada(self):
        escribir(self.dir, "main.py", "print('hola')\n")
        escribir(self.dir, "datos/productos.json", '{"es": "no es un idioma"}')
        escribir(self.dir, "locales/es.json", '{"a": "Hola"}')  # un solo idioma: nada que comparar
        inf = self.informe()
        self.assertEqual((inf.tipos, inf.errores, inf.avisos, inf.ok), ([], [], [], []))

    def test_json_claves_marcadores_vacios_y_sin_traducir(self):
        escribir(self.dir, "locales/es.json", json.dumps({
            "menu": {"inicio": "Pantalla de inicio", "salir": "Salir del programa", "hola": "Hola, {nombre}"},
            "ventas": {"total": "Total de la venta", "vacio": "Carrito vacío"}}))
        escribir(self.dir, "locales/en.json", json.dumps({
            "menu": {"inicio": "Home screen", "salir": "", "hola": "Hello"}, "ventas": {"total": "Sale total"}}))
        inf = self.informe()
        texto = "\n".join(inf.errores)
        self.assertIn("locales/en.json: faltan 1 claves", texto)
        self.assertIn("ventas.vacio", texto)
        self.assertIn("1 textos vacíos (sin traducir): menu.salir", texto)
        self.assertIn("marcadores distintos", texto)
        self.assertIn("menu.hola", texto)
        self.assertIn("idiomas", inf.tipos)

    def test_idioma_copiado_sin_traducir_es_error(self):
        es = {f"k{i}": f"Texto número {i} en español" for i in range(8)}
        escribir(self.dir, "i18n/es.json", json.dumps(es))
        escribir(self.dir, "i18n/en.json", json.dumps({**es, "k0": "Number 0 text in English"}))
        inf = self.informe()
        self.assertTrue(any("SIN TRADUCIR" in e for e in inf.errores), inf.errores)
        escribir(self.dir, "i18n/en.json", json.dumps({k: f"English {k} text" for k in es} | {"k1": es["k1"]}))
        inf = self.informe()
        self.assertEqual(inf.errores, [])
        self.assertTrue(any("¿sin traducir?" in a for a in inf.avisos), inf.avisos)
        self.assertTrue(inf.ok)

    def test_claves_usadas_en_el_codigo_y_modo_parcial(self):
        escribir(self.dir, "locales/es.json", json.dumps({"ventas": {"titulo": "Ventas del día"},
                                                          "clientes": {"titulo": "Lista de clientes"}}))
        escribir(self.dir, "locales/en.json", json.dumps({"ventas": {"titulo": "Today's sales"}}))
        escribir(self.dir, "ventas/vista.py", "from i18n import t\nprint(t('ventas.titulo'))\nprint(t('ventas.nueva'))\n")
        escribir(self.dir, "clientes/vista.py", "from i18n import t\nprint(t('clientes.titulo'))\n")
        escribir(self.dir, "tests/test_x.py", "t('clave.de.prueba')\n")
        estricto = self.informe()
        self.assertTrue(any("faltan 1 claves" in e and "clientes.titulo" in e for e in estricto.errores))
        self.assertTrue(any("ventas.nueva (ventas/vista.py:3)" in e for e in estricto.errores), estricto.errores)
        self.assertFalse(any("clave.de.prueba" in e for e in estricto.errores))
        # parte en curso = ventas: lo que falta de clientes lo hará otra parte (aviso)
        parcial = self.informe(parcial=["ventas/vista.py"])
        self.assertFalse(any("clientes.titulo" in e for e in parcial.errores), parcial.errores)
        self.assertTrue(any("clientes.titulo" in a and "otras partes" in a for a in parcial.avisos))
        self.assertTrue(any("ventas.nueva" in e for e in parcial.errores))
        # parte en curso = clientes: su clave es SU trabajo (error)
        parcial = self.informe(parcial=["clientes/vista.py"])
        self.assertTrue(any("clientes.titulo" in e for e in parcial.errores))

    def test_textos_fijos_sin_traducir(self):
        escribir(self.dir, "locales/es.json", '{"menu": {"titulo": "Menú principal"}}')
        escribir(self.dir, "locales/en.json", '{"menu": {"titulo": "Main menu"}}')
        escribir(self.dir, "menu.py", "from i18n import t\n\n\ndef ver(salida=print, n=1):\n    salida(t('menu.titulo'))\n"
                                      "    salida(f'Tienes {n} productos en el carrito')\n    print('-' * 20)\n"
                                      "    if n < 0:\n        raise ValueError('La cantidad no es válida')\n"
                                      "    return input('Elige una opción: ')\n")
        escribir(self.dir, "viejo.py", "print('Este módulo aún no usa traducciones')\n")
        escribir(self.dir, "web/app.js", "el.textContent = t('menu.titulo');\nboton.textContent = 'Comprar ahora';\n")
        inf = self.informe()
        avisos = "\n".join(inf.avisos)
        self.assertIn("menu.py: 3 textos fijos", avisos)
        self.assertIn("'Tienes {} productos en el carrito' (l. 6)", avisos)
        self.assertIn("'La cantidad no es válida' (l. 9)", avisos)
        self.assertIn("web/app.js: 1 textos fijos que ve el usuario sin pasar por las traducciones: 'Comprar ahora' (l. 2)",
                      avisos)
        self.assertNotIn("viejo.py", avisos)  # aún no usa traducciones: lo hará su parte
        self.assertEqual(inf.errores, [])
        self.assertNotIn("menu.py", "\n".join(self.informe(parcial=["web/app.js"]).avisos))

    def test_espacios_de_nombres(self):
        escribir(self.dir, "public/locales/es/ventas.json", '{"titulo": "Ventas", "boton": "Comprar ahora"}')
        escribir(self.dir, "public/locales/en/ventas.json", '{"titulo": "Sales"}')
        escribir(self.dir, "src/app.js", "t('ventas:titulo'); t('ventas.boton');\n")
        inf = self.informe()
        self.assertEqual(len(inf.errores), 1, inf.errores)
        self.assertIn("public/locales/en/ventas.json: faltan 1 claves", inf.errores[0])

    def test_objeto_js_y_dict_python_con_varios_idiomas(self):
        escribir(self.dir, "js/i18n.js", """// traducciones
const traducciones = {
  es: { titulo: 'Catálogo de productos', comprar: "Añadir al carrito", /* nota */ },
  en: { titulo: `Product catalog`, },
};
export default traducciones;
""")
        escribir(self.dir, "app/textos.py", "TEXTOS = {\n    'es': {'saludo': 'Bienvenido a la tienda'},\n"
                                            "    'en': {'saludo': 'Welcome to the shop', 'extra': 'Extra'},\n}\n")
        inf = self.informe()
        texto = "\n".join(inf.errores)
        self.assertIn("js/i18n.js [en]: faltan 1 claves", texto)
        self.assertIn("comprar", texto)
        self.assertIn("app/textos.py [es]: faltan 1 claves", texto)

    def test_po_android_resx_arb_y_yaml(self):
        escribir(self.dir, "locale/en/LC_MESSAGES/app.po", 'msgid ""\nmsgstr ""\n\nmsgid "Guardar"\nmsgstr "Save"\n\n'
                                                           '#, fuzzy\nmsgid "Abrir"\nmsgstr "Open"\n\nmsgid "Cerrar"\nmsgstr ""\n')
        escribir(self.dir, "locale/fr/LC_MESSAGES/app.po", 'msgid "Guardar"\nmsgstr "Enregistrer"\n\nmsgid "Abrir"\n'
                                                           'msgstr "Ouvrir"\n\nmsgid "Cerrar"\nmsgstr "Fermer"\n')
        escribir(self.dir, "app/src/main/res/values/strings.xml",
                 '<resources><string name="app">Tienda</string><string name="hola">Hola</string>'
                 '<string name="url" translatable="false">x</string></resources>')
        escribir(self.dir, "app/src/main/res/values-en/strings.xml", '<resources><string name="app">Shop</string></resources>')
        escribir(self.dir, "Props/Recursos.resx", '<root><data name="Titulo"><value>Título</value></data>'
                                                  '<data name="Pie"><value>Pie de página</value></data></root>')
        escribir(self.dir, "Props/Recursos.en.resx", '<root><data name="Titulo"><value>Title</value></data></root>')
        escribir(self.dir, "lib/l10n/app_es.arb", '{"@@locale": "es", "hola": "Hola {nombre}", "@hola": {}}')
        escribir(self.dir, "lib/l10n/app_en.arb", '{"@@locale": "en", "hola": "Hello"}')
        escribir(self.dir, "config/locales/es.yml", "es:\n  tienda:\n    titulo: \"Mi tienda\"\n    pie: Pie\n")
        escribir(self.dir, "config/locales/en.yml", "en:\n  tienda:\n    titulo: My shop\n")
        texto = "\n".join(self.informe().errores)
        self.assertIn("locale/en/LC_MESSAGES/app.po: 2 textos vacíos (sin traducir): Abrir, Cerrar", texto)
        self.assertIn("values-en/strings.xml: faltan 1 claves de idioma que sí están en otros idiomas del grupo: hola", texto)
        self.assertNotIn("url", texto)
        self.assertIn("Props/Recursos.en.resx: faltan 1 claves de idioma que sí están en otros idiomas del grupo: Pie", texto)
        self.assertRegex(texto, r"lib/l10n/app_e[ns]\.arb: marcadores distintos")
        self.assertIn("config/locales/en.yml: faltan 1 claves de idioma que sí están en otros idiomas del grupo: tienda.pie",
                      texto)

    def test_resumen_para_mapa(self):
        self.assertEqual(idiomas.resumen_idiomas(self.dir), "")
        escribir(self.dir, "locales/es.json", '{"a": "x"}')
        escribir(self.dir, "locales/en.json", '{"a": "y"}')
        self.assertEqual(idiomas.resumen_idiomas(self.dir), "en, es (locales/{L}.json)")


# ---------------------------------------------------------------- verificar con errores previos
class TestPreexistentes(Temporal):
    def test_solo_cuentan_los_errores_nuevos(self):
        escribir(self.dir, "viejo.py", "def f(:\n")
        h = Herramientas(self.dir)
        informe = h.verificar_cmd()
        self.assertGreater(h.verificacion["errores"], 0)
        from agente.verificacion import informe_proyecto
        h.errores_previos = {normalizar(e) for e in informe_proyecto(self.dir, "python").errores}
        escribir(self.dir, "viejo.py", "\n\ndef f(:\n")  # se movió de línea: sigue siendo el mismo error
        informe = h.verificar_cmd()
        self.assertEqual(h.verificacion["errores"], 0, informe)
        self.assertIn("PREEXISTENTES (", informe)
        escribir(self.dir, "nuevo.py", "x = (\n")
        h.verificar_cmd()
        self.assertGreater(h.verificacion["errores"], 0)


class TestVerificarSubcarpeta(Temporal):
    def test_codigo_sin_usar_mira_todo_el_proyecto(self):
        escribir(self.dir, "main.py", "from inventario.servicio import Inventario\n\n\ndef fuera_de_carpeta():\n    pass\n\n\n"
                                      "if __name__ == '__main__':\n    print(Inventario().total())\n")
        escribir(self.dir, "inventario/__init__.py", "")
        escribir(self.dir, "inventario/servicio.py", "class Inventario:\n    def total(self):\n        return 0\n\n\n"
                                                     "def huerfana():\n    return 1\n")
        h = Herramientas(self.dir)
        informe = h.verificar_cmd(ruta="inventario")
        self.assertNotIn("'Inventario' NUNCA", informe)  # se usa desde main.py, fuera de la carpeta
        self.assertIn("'huerfana' NUNCA se usa", informe)
        self.assertNotIn("fuera_de_carpeta", informe)  # fuera de la carpeta pedida: no se informa
        self.assertEqual(h.verificacion["ruta"], "inventario")
        h.verificar_cmd(ruta="main.py")
        self.assertEqual(h.verificacion["ruta"], ".")  # un archivo verifica el proyecto entero


# ---------------------------------------------------------------- enrutado y plan
class TestEnrutado(Temporal):
    def test_modo(self):
        d = self.dir / ".agente"
        self.assertEqual(grande.modo("crea un juego de naves con pygame", True, d), "")
        self.assertEqual(grande.modo("añade inglés a toda la app", False, d), "")  # repo pequeño: camino normal
        self.assertEqual(grande.modo("añade inglés a toda la app", True, d), "nuevo")
        self.assertEqual(grande.modo("traduce todos los módulos al francés", True, d), "nuevo")
        self.assertEqual(grande.modo("/grande renombra Cliente por Usuario", False, d), "nuevo")
        self.assertEqual(grande.modo("hazlo en modo grande: migra a pathlib", False, d), "nuevo")
        self.assertEqual(grande.modo("continúa", True, d), "")  # sin modo grande pendiente
        grande.guardar(d, {"estado": "en_curso", "partes": []})
        self.assertEqual(grande.modo("continúa", False, d), "reanudar")
        self.assertEqual(grande.modo("arregla el bug del login", False, d), "")
        self.assertEqual(grande.cargar(d)["estado"], "aparcado")  # un mandato normal lo aparca
        self.assertEqual(grande.modo("continúa", False, d), "")
        self.assertEqual(grande.modo("continúa el modo grande", False, d), "reanudar")

    def test_validar_plan(self):
        h = Herramientas(self.dir)
        for i in range(20):
            escribir(self.dir, f"ventas/v{i}.py", "x = 1\n")
        escribir(self.dir, "clientes/vista.py", "x = 1\n")
        r = grande.validar(h, {"partes": [{"titulo": "ventas", "archivos": ["ventas"], "terminado": "t", "prueba": "p"},
                                          {"titulo": "c", "archivos": ["vista.py", "../fuera.py"], "terminado": "t"}]}, 0)
        self.assertIn("20 archivos; divídela", r)
        self.assertIn("¿quisiste decir 'clientes/vista.py'?", r)
        self.assertIn("fuera del proyecto", r)
        self.assertIn("falta prueba", r)
        estado = grande.validar(h, {"partes": [{"titulo": "ventas", "archivos": ["ventas/"], "terminado": "t",
                                                "prueba": "p"}], "convenciones": "claves modulo.clave"}, 3)
        self.assertEqual([len(p["archivos"]) for p in estado["partes"]], [15, 5])  # tras 3 rechazos se trocea solo
        self.assertEqual(estado["partes"][1]["titulo"], "ventas (2)")
        self.assertEqual(estado["convenciones"], "claves modulo.clave")
        ok = grande.validar(h, {"partes": [{"titulo": "c", "archivos": ["clientes\\vista.py", "locales/en.json"],
                                            "terminado": "t", "prueba": "p"}]}, 0)
        self.assertEqual(ok["partes"][0]["archivos"], ["clientes/vista.py", "locales/en.json"])  # los nuevos valen


# ---------------------------------------------------------------- flujo completo con un modelo simulado
class LLMGrabador:
    """Modelo falso: responde en orden y guarda qué recibió (mensajes y herramientas)."""
    respuestas: ClassVar[list] = []
    recibido: ClassVar[list] = []

    def __init__(self, base, modelo, contexto, cfg):
        self.base, self.modelo, self.contexto, self.cfg = base, modelo, contexto, cfg

    def chat(self, mensajes, herramientas, max_tokens, razonar, al_progreso=None):
        LLMGrabador.recibido.append({"mensajes": json.loads(json.dumps(mensajes)), "herramientas": herramientas})
        return LLMGrabador.respuestas.pop(0)


def llamada(nombre: str, **args) -> dict:
    return {"content": "", "finish_reason": "tool_calls", "usage": {},
            "tool_calls": [{"id": f"{nombre}{time.perf_counter_ns()}", "name": nombre, "arguments": json.dumps(args)}]}


def varias(*respuestas: dict) -> dict:
    """Varias llamadas en una misma respuesta del modelo."""
    return {**respuestas[0], "tool_calls": [r["tool_calls"][0] for r in respuestas]}


APROBADO = {"content": "REQUISITOS:\n- parte -> CUMPLE\nPROBLEMAS:\n- ninguno\nVEREDICTO: APROBADO", "tool_calls": [],
            "finish_reason": "stop", "usage": {}}


class TestFlujo(Temporal):
    def agente(self):
        from agente.bucle import Agente
        falso = lambda cfg: ("http://falso", "falso", 32768)
        self.parches = [mock.patch("agente.bucle.cliente.detectar", falso), mock.patch("agente.bucle.cliente.LLM", LLMGrabador),
                        mock.patch("agente.bucle.entorno", lambda: "prueba"), mock.patch.dict(os.environ, GIT)]
        for p in self.parches:
            p.start()
        LLMGrabador.respuestas, LLMGrabador.recibido = [], []
        return Agente(config.cargar(), self.dir, interactivo=False)

    def tearDown(self):
        for p in getattr(self, "parches", []):
            p.stop()
        super().tearDown()

    def test_mandato_normal_en_repo_pequeno_no_cambia_la_peticion(self):
        a = self.agente()
        LLMGrabador.respuestas = [{"content": "Hola.", "tool_calls": [], "finish_reason": "stop", "usage": {}}]
        a.tarea("di hola")
        self.assertIs(LLMGrabador.recibido[0]["herramientas"], ESQUEMAS)  # misma lista, byte a byte
        self.assertEqual(LLMGrabador.recibido[0]["mensajes"][-1]["content"], "di hola")
        self.assertFalse((self.dir / ".agente" / "mapa.json").exists())

    def test_repo_grande_ofrece_mapa_y_no_lo_quita(self):
        for i in range(25):
            escribir(self.dir, f"m{i // 5}/f{i}.py", "x = 1\n")
        a = self.agente()
        fijos = a.ctx.fijos
        LLMGrabador.respuestas = [{"content": "Hola.", "tool_calls": [], "finish_reason": "stop", "usage": {}}] * 2
        a.tarea("di hola")
        nombres = [e["function"]["name"] for e in LLMGrabador.recibido[0]["herramientas"]]
        self.assertEqual(nombres[-1], "mapa")
        self.assertGreater(a.ctx.fijos, fijos)
        self.assertIn("mira mapa()", LLMGrabador.recibido[0]["mensajes"][-1]["content"])
        a.esperar_mapa()  # la caché se hacía en segundo plano
        self.assertTrue((self.dir / ".agente" / "mapa.json").exists())
        for i in range(25):
            (self.dir / f"m{i // 5}" / f"f{i}.py").unlink()
        a.tarea("di adiós")  # el repo encogió, pero la lista de herramientas no cambia (caché de prompt)
        self.assertEqual(LLMGrabador.recibido[1]["herramientas"], LLMGrabador.recibido[0]["herramientas"])

    def _proyecto(self):
        escribir(self.dir, "main.py", "from clientes.datos import nombre\nfrom ventas.precios import precio\n\n"
                                      "if __name__ == '__main__':\n    print(precio(2), nombre('ana'))\n")
        escribir(self.dir, "ventas/__init__.py", "")
        escribir(self.dir, "clientes/__init__.py", "")
        escribir(self.dir, "ventas/precios.py", "def precio(x):\n    return x * 2\n")
        escribir(self.dir, "clientes/datos.py", "def nombre(c):\n    return c.title()\n")

    def test_dos_partes_con_contexto_limpio_commit_y_final(self):
        self._proyecto()
        a = self.agente()
        plan = {"partes": [
            {"titulo": "Ventas", "archivos": ["ventas/precios.py", "ventas/iva.py"], "terminado": "iva.py existe",
             "prueba": "verificar"},
            {"titulo": "Clientes", "archivos": ["clientes/datos.py"], "terminado": "sin espacios", "prueba": "verificar"}],
            "convenciones": "funciones en español"}
        LLMGrabador.respuestas = [
            llamada("mapa", ruta="ventas"),
            llamada("plan_partes", **plan),
            varias(llamada("escribir", ruta="ventas/iva.py", contenido="def iva(x):\n    return x * 0.21\n"),
                   llamada("escribir", ruta="ventas/precios.py",
                           contenido="from ventas.iva import iva\n\n\ndef precio(x):\n    return x * 2 + iva(x)\n")),
            llamada("terminar", resumen="Añadido iva() en ventas."), APROBADO,
            llamada("editar", ruta="clientes/datos.py", buscar="return c.title()", reemplazar="return c.strip().title()"),
            llamada("terminar", resumen="nombre() quita espacios."), APROBADO,
        ]
        r = a.tarea("/grande añade utilidades a todos los módulos")
        self.assertTrue(r.startswith("Modo grande terminado: 2 partes"), r)
        self.assertEqual(LLMGrabador.respuestas, [])
        # fase de plan: solo lectura + plan_partes
        nombres_plan = [e["function"]["name"] for e in LLMGrabador.recibido[0]["herramientas"]]
        self.assertEqual(sorted(nombres_plan), ["buscar", "leer", "listar", "mapa", "plan_partes"])
        self.assertIn("ventas/precios.py", LLMGrabador.recibido[0]["mensajes"][1]["content"])  # el mapa va en el prompt
        # cada parte empieza con un contexto limpio: sistema + su mensaje
        inicio2 = LLMGrabador.recibido[2]["mensajes"]
        self.assertEqual([m["role"] for m in inicio2], ["system", "user"])
        self.assertTrue(inicio2[1]["content"].startswith("MODO GRANDE — PARTE 1/2: Ventas"))
        self.assertIn("ventas/iva.py (nuevo)", inicio2[1]["content"])
        self.assertIn("CONVENCIONES (comunes a todas las partes): funciones en español", inicio2[1]["content"])
        inicio3 = LLMGrabador.recibido[5]["mensajes"]
        self.assertEqual([m["role"] for m in inicio3], ["system", "user"])
        self.assertIn("HECHO EN PARTES ANTERIORES:\n- Parte 1: Añadido iva() en ventas.", inicio3[1]["content"])
        # la revisión exige la parte, no el mandato entero
        self.assertIn("PARTE 1/2 de un cambio grande", LLMGrabador.recibido[4]["mensajes"][1]["content"])
        estado = grande.cargar(self.dir / ".agente")
        self.assertEqual(estado["estado"], "terminado")
        self.assertEqual([p["estado"] for p in estado["partes"]], ["hecha", "hecha"])
        self.assertEqual(estado["git"], "init")
        log = a.h._git("log", "--format=%s")[1].splitlines()
        self.assertEqual(log, ["Parte 2/2: Clientes", "Parte 1/2: Ventas", "Estado inicial (antes del modo grande)"])
        self.assertIn("ventas/iva.py", a.h._git("show", "--name-only", "--format=", "HEAD~1")[1])
        self.assertFalse(a.h.archivo_plan.exists())  # nada pendiente para el IDE
        self.assertEqual([m["role"] for m in a.ctx.mensajes], ["system", "user", "assistant"])  # sigue la conversación
        self.assertIsNone(a.h.errores_previos)

    def test_reanuda_la_parte_pendiente_tras_cerrar(self):
        self._proyecto()
        a = self.agente()
        grande.guardar(a.h.dir_agente, {
            "version": 1, "estado": "en_curso", "tarea": "añade utilidades a todos los módulos", "convenciones": "",
            "git": "sin git: prueba", "errores_previos": [], "partes": [
                {"titulo": "Ventas", "archivos": ["ventas/precios.py"], "terminado": "t", "prueba": "verificar",
                 "estado": "hecha", "intentos": 1, "pasos": 5, "segundos": 60, "resumen": "Hecho ventas.", "commit": "",
                 "motivo": ""},
                {"titulo": "Clientes", "archivos": ["clientes/datos.py"], "terminado": "t", "prueba": "verificar",
                 "estado": "pendiente", "intentos": 1, "pasos": 9, "segundos": 90, "resumen": "", "commit": "",
                 "motivo": "", "sucios_antes": {}}]})
        LLMGrabador.respuestas = [
            llamada("editar", ruta="clientes/datos.py", buscar="c.title()", reemplazar="c.strip().title()"),
            llamada("terminar", resumen="Clientes listo."), APROBADO]
        r = a.tarea("continúa")
        self.assertTrue(r.startswith("Modo grande terminado: 2 partes"), r)
        primero = LLMGrabador.recibido[0]["mensajes"][1]["content"]
        self.assertTrue(primero.startswith("MODO GRANDE — PARTE 2/2: Clientes"))
        self.assertIn("INTENTO 2", primero)  # avisa de cambios a medias
        self.assertIn("Sin commits (prueba)", r)

    def test_parte_que_falla_dos_veces_se_detiene_y_se_puede_reanudar(self):
        self._proyecto()
        a = self.agente()
        a.cfg["max_pasos"] = 2
        sin_fin = [llamada("leer", ruta="ventas/precios.py"), llamada("leer", ruta="ventas/precios.py", desde=2)]
        LLMGrabador.respuestas = [
            llamada("plan_partes", partes=[{"titulo": "Ventas", "archivos": ["ventas/precios.py"], "terminado": "t",
                                            "prueba": "verificar"}]), *sin_fin, *sin_fin]
        r = a.tarea("/grande cambia algo en todos los módulos")
        self.assertTrue(r.startswith("Detenido: la parte 1/1 (Ventas)"), r)
        from agente.bucle import INCOMPLETA
        self.assertTrue(INCOMPLETA.match(r))
        self.assertEqual(grande.cargar(a.h.dir_agente)["estado"], "detenido")
        self.assertIn("- [ ] 1. Parte: Ventas", a.h.archivo_plan.read_text(encoding="utf-8"))  # el IDE ofrece continuar
        self.assertEqual(grande.modo("continúa", False, a.h.dir_agente), "reanudar")
        self.assertEqual(grande.modo("arregla otra cosa", False, a.h.dir_agente), "")
        self.assertFalse(a.h.archivo_plan.exists())  # aparcado: la lista de partes no frena al mandato nuevo


class TestCommitSoloLoDeLaParte(Temporal):
    def test_no_mezcla_el_trabajo_a_medias_del_usuario(self):
        with mock.patch.dict(os.environ, GIT):
            h = Herramientas(self.dir)
            escribir(self.dir, "a.py", "x = 1\n")
            escribir(self.dir, "usuario.txt", "v1\n")
            self.assertEqual(grande.preparar_git(h), "init")
            h.git("commit", mensaje="base")
            escribir(self.dir, "usuario.txt", "a medias\n")
            estado = {"git": "repo"}
            antes = grande._sucios(h)
            escribir(self.dir, "a.py", "x = 2\n")
            escribir(self.dir, "b.py", "y = 1\n")
            commit = grande._commit(h, estado, "Parte 1/1: prueba", antes)
            self.assertRegex(commit, r"^[0-9a-f]{7,}$")
            self.assertEqual(sorted(h._git("show", "--name-only", "--format=", "HEAD")[1].split()), ["a.py", "b.py"])
            self.assertIn("usuario.txt", h._git("status", "--short")[1])
            self.assertEqual(grande._commit(h, estado, "otra", grande._sucios(h)), "sin cambios")

    def test_no_crea_un_repo_dentro_de_otro(self):
        with mock.patch.dict(os.environ, GIT):
            Herramientas(self.dir)._git("init")
            hija = Herramientas(self.dir / "sub")
            self.assertTrue(grande.preparar_git(hija).startswith("sin git: la carpeta está dentro de otro repositorio"))
            self.assertFalse((self.dir / "sub" / ".git").exists())


if __name__ == "__main__":
    unittest.main()
