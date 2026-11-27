"""m4198/m4199: en una auditoría de solo lectura, `verificar` lanzaba las pruebas y programas de un repositorio ajeno. Solo
ejecuta lo que Aura escribió o cuando la tarea cambia código. El repositorio de juguete solo deja una marca al ejecutarse."""
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from typing import ClassVar
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_agente import llamada, sin_lecciones_reales

from agente import config, ecosistemas, grande, herramientas
from agente import verificacion as V
from agente.autoria import Autoria, es_ejecutable
from agente.estado_motor import toca_estado
from agente.herramientas import Herramientas

MARCA = "from pathlib import Path\nPath(r'{}').write_text('ejecutado')\n"
FIN = {"content": "Listo.", "tool_calls": [], "finish_reason": "stop", "usage": {}}
setUpModule = sin_lecciones_reales


class LLMGuion:
    respuestas: ClassVar[list] = []

    def __init__(self, base, modelo, contexto, cfg):
        self.contexto = contexto

    def chat(self, mensajes, herramientas, max_tokens, razonar, al_progreso=None):
        return LLMGuion.respuestas.pop(0)


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="agente_ajeno_")).resolve()
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.dir, self.marcas = self.tmp / "repo", self.tmp / "marcas"
        self.dir.mkdir()
        self.marcas.mkdir()

    def poner(self, nombre: str, texto: str) -> None:
        p = self.dir / nombre
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(texto, encoding="utf-8")

    def juguete(self) -> None:
        """Un repositorio «clonado»: sus pruebas, su conftest y su programa dejan una marca si alguien los ejecuta."""
        self.poner("app.py", MARCA.format(self.marcas / "programa") + "\nif __name__ == '__main__':\n    print('hola')\n")
        self.poner("conftest.py", MARCA.format(self.marcas / "conftest"))
        self.poner("tests/test_app.py", MARCA.format(self.marcas / "pruebas") + "\n\ndef test_nada():\n    assert True\n")

    def ejecutado(self) -> list[str]:
        return sorted(p.name for p in self.marcas.iterdir())

    def abrir(self, texto: str = "audita sin modificar nada") -> Herramientas:
        h = Herramientas(self.dir)
        h.nueva_tarea(1, texto)
        return h


class TestRegla(Base):
    def assert_solo_lectura(self, informe: str):
        self.assertEqual(self.ejecutado(), [], informe)
        self.assertIn("NO EJECUTADO", informe)
        self.assertIn("pruebas y programa de Python", informe)

    def test_una_tarea_que_no_modifica_no_ejecuta_nada(self):
        self.juguete()
        self.assert_solo_lectura(self.abrir().verificar_cmd())

    def test_escribir_un_informe_no_es_cambiar_codigo(self):
        self.juguete()
        h = self.abrir()
        h.escribir("AUDITORIA.md", "# Informe\n")
        h.escribir("hallazgos.json", '{"n": 1}\n')
        self.assert_solo_lectura(h.verificar_cmd())

    def test_tras_cambiar_codigo_si_ejecuta_y_continua_lo_conserva(self):
        self.juguete()
        h = self.abrir("arregla el saludo")
        self.assertTrue(h.editar("app.py", "print('hola')", "print('adiós')").startswith("OK"))
        informe = h.verificar_cmd()
        self.assertNotIn("NO EJECUTADO", informe)
        self.assertEqual(self.ejecutado(), ["conftest", "programa", "pruebas"], informe)
        h.nueva_tarea(5, "")  # «continúa»
        self.assertTrue(h.autoria.puede_ejecutar())
        reabierta = Herramientas(self.dir)  # se cerró Aura y se pide «continúa»
        reabierta.nueva_tarea(9, "")
        self.assertTrue(reabierta.autoria.puede_ejecutar())
        reabierta.nueva_tarea(10, "ahora solo dime qué hace")  # una petición nueva empieza de cero
        self.assertFalse(reabierta.autoria.puede_ejecutar())

    def test_un_manifiesto_cambiado_cuenta_y_restaurar_no(self):
        self.juguete()
        h = self.abrir("añade una dependencia")
        h.escribir("requirements.txt", "flask\n")
        self.assertTrue(h.autoria.puede_ejecutar())
        h = self.abrir("restaura")
        h.checkpoints = [{"paso": 1, "archivos": {"app.py": (self.dir / "app.py").read_text(encoding="utf-8")}}]
        self.assertTrue(h.restaurar().startswith("OK"))
        self.assertFalse(h.autoria.puede_ejecutar())

    def test_lo_que_escribio_aura_se_ejecuta_hasta_que_llega_mas_codigo_ajeno(self):
        h = self.abrir("crea un programa que salude")
        h.escribir("app.py", MARCA.format(self.marcas / "propio") + "\nif __name__ == '__main__':\n    print('hola')\n")
        h = self.abrir("revisa que funcione")  # otra petición, sin cambios
        informe = h.verificar_cmd()
        self.assertEqual(self.ejecutado(), ["propio"], informe)
        shutil.rmtree(self.marcas)
        self.marcas.mkdir()
        self.juguete_ajeno_dentro()  # se clona un repositorio dentro
        informe = self.abrir("audita lo clonado").verificar_cmd()
        self.assertEqual(self.ejecutado(), [], informe)
        self.assertIn("NO EJECUTADO", informe)

    def juguete_ajeno_dentro(self) -> None:
        for i in range(5):
            self.poner(f"clonado/m{i}.py", f"VALOR_{i} = {i}\n")

    def test_la_carpeta_vacia_con_codigo_clonado_despues_no_se_ejecuta(self):
        h = self.abrir("clona el repositorio")  # la carpeta estaba vacía: el caso de la auditoría real
        self.assertFalse(h.autoria.puede_ejecutar())
        self.juguete()
        self.assert_solo_lectura(h.verificar_cmd())

    def test_en_un_proyecto_de_aura_basta_con_la_mayoria(self):
        h = self.abrir("crea cinco módulos")
        for i in range(4):
            h.escribir(f"m{i}.py", f"VALOR = {i}\n")
        self.poner("suelto.py", "X = 1\n")  # 4 de 5: uno lo generó una orden
        self.assertTrue(Autoria(self.dir, h.dir_agente).es_de_aura())
        self.poner("otro.py", "Y = 1\n")  # 4 de 6
        self.assertFalse(Autoria(self.dir, h.dir_agente).es_de_aura())

    def test_el_modelo_no_puede_escribir_la_lista(self):
        h = self.abrir()
        lista = '{"escritos": ["app.py"], "tarea_cambia": true}'
        self.assertTrue(h.escribir(".agente/escritos.json", lista).startswith("BLOQUEADO"))
        self.assertTrue(h.editar(".agente/escritos.json", "a", "b").startswith("BLOQUEADO"))
        self.assertTrue(toca_estado("echo {} > .agente\\escritos.json"))

    def test_una_lista_rota_cuenta_como_ajeno(self):
        self.juguete()
        (self.dir / ".agente").mkdir()
        (self.dir / ".agente" / "escritos.json").write_text("no es json", encoding="utf-8")
        self.assert_solo_lectura(self.abrir().verificar_cmd())

    def test_verificar_directo_sigue_ejecutando(self):
        self.juguete()
        informe, _ = V.verificar(self.dir, sys.executable)
        self.assertIn("pruebas", self.ejecutado(), informe)


class TestEcosistemas(Base):
    def test_en_codigo_ajeno_no_se_lanza_ninguna_herramienta(self):
        self.poner("package.json", json.dumps({"scripts": {"build": "x", "test": "y"}, "dependencies": {"react": "1"}}))
        self.poner("tsconfig.json", "{}")
        self.poner("Cargo.toml", '[package]\nname = "x"\nversion = "0.1.0"\n')
        self.poner("src/main.rs", "fn main() {}\n")
        self.poner("go.mod", "module x\n")
        self.poner("x.csproj", "<Project/>\n")
        self.poner("foundry.toml", "[profile.default]\n")
        self.poner("movil/pubspec.yaml", "name: x\n")  # en la raíz, el .java contaría como de Flutter
        self.poner("Dockerfile", "FROM python:3.12\nCOPY falta.txt .\nUSER app\n")
        self.poner("compose.yaml", "services: {}\n")
        self.poner("pom.xml", "<project/>\n")
        self.poner("Main.java", "class Main { public static void main(String[] a) {} }\n")
        self.poner("index.html", "<!doctype html><html><body><script>1;</script></body></html>\n")
        lanzado = []

        def anotar(cmd, *a, **k):
            if not (isinstance(cmd, list) and cmd[:2] == ["node", "--check"]):
                lanzado.append(cmd)
            return 0, ""

        with mock.patch.object(ecosistemas, "correr_texto", anotar), mock.patch.object(V, "correr_texto", anotar), \
                mock.patch.object(V, "navegador", lambda *a, **k: lanzado.append("navegador")):
            informe, _ = V.verificar(self.dir, "python", ejecutar=False)
        self.assertEqual(lanzado, [], informe)
        for texto in ("NO EJECUTADO", "rust", "go", ".net", "solidity", "flutter", "npm", "docker", "Java", "navegador"):
            self.assertIn(texto, informe)
        self.assertNotIn("npm install", informe)  # instalar ejecuta los scripts del repositorio
        self.assertIn("COPY 'falta.txt'", informe)  # lo que se ve leyendo se sigue diciendo


class TestModoGrande(Base):
    """La verificación inicial ejecuta solo si el plan va a tocar código."""

    def inicial(self, archivos: list[str]) -> bool:
        self.juguete()
        h = self.abrir("/grande revisa todo el proyecto")
        estado = {"git": "sin git: prueba", "partes": [{"titulo": "p", "archivos": archivos, "estado": "hecha"}], "tarea": "t"}

        class Visto(Exception):
            pass

        def informe(raiz, python, ejecutar=True, **k):
            raise Visto(ejecutar)

        agente = mock.Mock(h=h)
        with mock.patch.object(grande, "informe_proyecto", informe):
            try:
                grande._partes(agente, estado)
            except Visto as e:
                return e.args[0]
        self.fail("no llegó a la verificación inicial")

    def test_un_plan_de_informes_no_ejecuta(self):
        self.assertFalse(self.inicial(["AUDITORIA.md"]))

    def test_un_plan_que_toca_codigo_ejecuta(self):
        self.assertTrue(self.inicial(["app.py", "README.md"]))


class TestErroresQueYaEstaban(Base):
    """m4200: en una auditoría, los errores que el repositorio ya traía se enseñan, pero no frenan el final."""

    VIEJO = "roto.py:1 ERROR DE SINTAXIS"

    def repo_con_error(self):
        self.juguete()
        self.poner("roto.py", "def (:\n")

    def assert_no_cuenta(self, h: Herramientas, informe: str):
        self.assertEqual(h.verificacion["errores"], 0, informe)
        self.assertRegex(informe.splitlines()[0], r": 0 ERRORES, \d+ AVISOS \(y \d+ errores que ya estaban")
        self.assertIn(self.VIEJO, informe.split("PREEXISTENTES DEL CÓDIGO (")[1])
        self.assertNotIn("SIN ERRORES", informe)
        self.assertFalse(h.archivo_checkpoints.exists())  # no es un estado «sin errores» al que volver

    def assert_cuenta(self, h: Herramientas, informe: str, texto: str):
        self.assertGreater(h.verificacion["errores"], 0, informe)
        self.assertIn(texto, informe.split("ERRORES (corrígelos todos):")[1].split("PREEXISTENTES DEL CÓDIGO")[0])

    def test_sin_escribir_nada_se_listan_pero_no_cuentan(self):
        self.repo_con_error()
        h = self.abrir()
        self.assert_no_cuenta(h, h.verificar_cmd())
        self.assertEqual(self.ejecutado(), [])

    def test_escribir_el_informe_no_los_hace_contar_ni_tras_continuar(self):
        self.repo_con_error()
        h = self.abrir()
        h.escribir("AUDITORIA.md", "# Informe\n")
        self.assert_no_cuenta(h, h.verificar_cmd())
        reabierta = Herramientas(self.dir)  # se cerró Aura y se pide «continúa»
        reabierta.nueva_tarea(9, "")
        reabierta.escribir("AUDITORIA.md", "más\n", anadir=True)
        self.assert_no_cuenta(reabierta, reabierta.verificar_cmd())

    def test_lo_que_la_auditoria_rompe_si_cuenta(self):
        self.repo_con_error()
        h = self.abrir()
        h.escribir("README.md", "Para instalar, abre `instalar.bat`.\n")
        informe = h.verificar_cmd()
        self.assert_cuenta(h, informe, "README.md cita instalar.bat")
        self.assertIn(self.VIEJO, informe.split("PREEXISTENTES DEL CÓDIGO (")[1])

    def test_cuenta_aunque_se_anote_en_otro_archivo(self):
        self.repo_con_error()
        self.poner("locales/es.json", '{"hola": "Hola"}')
        self.poner("locales/en.json", '{"hola": "Hello"}')
        h = self.abrir()
        h.escribir("locales/es.json", '{"hola": "Hola", "adios": "Adiós"}')
        self.assert_cuenta(h, h.verificar_cmd(), "en.json: faltan 1 claves")

    def test_una_peticion_nueva_hace_otra_foto(self):
        self.repo_con_error()
        h = self.abrir()
        h.escribir("README.md", "Para instalar, abre `instalar.bat`.\n")
        h.nueva_tarea(5, "ahora resume lo que hay")
        h.escribir("RESUMEN.md", "# Resumen\n")  # lo que rompió la anterior ya estaba para esta
        self.assert_no_cuenta(h, h.verificar_cmd())

    def test_en_cuanto_la_tarea_cambia_codigo_cuentan_todos(self):
        self.repo_con_error()
        h = self.abrir("arregla el saludo")
        h.editar("app.py", "print('hola')", "print('adiós')")
        self.assertIsNone(h.autoria.antes)  # no se gasta una foto que no va a usarse
        self.assert_cuenta(h, h.verificar_cmd(), self.VIEJO)

    def test_en_codigo_de_aura_cuentan_todos(self):
        h = self.abrir("crea un programa")
        h.escribir("roto.py", "def (:\n")
        h = self.abrir("revisa que funcione")
        self.assert_cuenta(h, h.verificar_cmd(), self.VIEJO)

    def test_en_modo_grande_manda_su_propia_foto(self):
        self.repo_con_error()
        h = self.abrir("/grande revisa todo")
        h.errores_previos = set()
        h.escribir("AUDITORIA.md", "# Informe\n")
        self.assertIsNone(h.autoria.antes)
        self.assert_cuenta(h, h.verificar_cmd(), self.VIEJO)

    def test_si_no_se_puede_hacer_la_foto_cuentan_todos(self):
        self.repo_con_error()
        h = self.abrir()
        with mock.patch.object(herramientas, "informe_proyecto", side_effect=RuntimeError("boom")):
            self.assertTrue(h.escribir("AUDITORIA.md", "# Informe\n").startswith("OK"))
        self.assert_cuenta(h, h.verificar_cmd(), self.VIEJO)

    def test_verificar_directo_los_sigue_contando(self):
        self.repo_con_error()
        informe, errores = V.verificar(self.dir, sys.executable, ejecutar=False)
        self.assertGreater(errores, 0, informe)
        self.assertNotIn("PREEXISTENTES DEL CÓDIGO", informe)

    def test_terminar_no_se_rechaza(self):
        from agente.bucle import Agente

        self.repo_con_error()
        LLMGuion.respuestas = [llamada("escribir", ruta="AUDITORIA.md", contenido="# Informe\nroto.py no compila.\n"),
                               llamada("terminar", resumen="Auditoría en AUDITORIA.md."), FIN]
        falso = mock.patch("agente.bucle.cliente.detectar", lambda cfg: ("http://falso", "falso", 32768))
        with falso, mock.patch("agente.bucle.cliente.LLM", LLMGuion), mock.patch("agente.bucle.entorno", lambda: "prueba"), \
                mock.patch.object(Agente, "_revisar", return_value=("APROBADO", True)):
            a = Agente(config.cargar(), self.dir, interactivo=False)
            a.tarea("audita sin modificar nada")
        ids = {c["id"] for m in a.ctx.mensajes if m["role"] == "assistant" for c in m.get("tool_calls") or []
               if c["function"]["name"] == "terminar"}
        resultados = [m["content"] for m in a.ctx.mensajes if m["role"] == "tool" and m["tool_call_id"] in ids]
        self.assertEqual(resultados, ["Tarea terminada."])
        self.assertEqual(self.ejecutado(), [])


class TestQueEsCodigo(unittest.TestCase):
    def test_codigo_y_manifiestos(self):
        for nombre in ("a.py", "a.JS", "x/Cargo.toml", "package.json", "Dockerfile", "dockerfile.dev", "x.csproj", "run.bat"):
            self.assertTrue(es_ejecutable(Path(nombre)), nombre)
        for nombre in ("INFORME.md", "datos.json", "notas.txt", "config.yml", "foto.png", "estilo.css"):
            self.assertFalse(es_ejecutable(Path(nombre)), nombre)


if __name__ == "__main__":
    unittest.main()
