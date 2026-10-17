"""Pruebas de los motores complementarios, la verificación de Solidity/Anchor/Android/Flutter, las bases de datos,
experimentar y lo aprendido por tema. Sin red: las descargas y los compiladores se simulan salvo que estén instalados."""
import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.error
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import android, complementos, conocimiento, ecosistemas, matematicas, prompts
from agente.herramientas import LICENCIA, PELIGROSOS, Herramientas, bd_remota, despachar, externo
from agente.verificacion import Informe

NO_EJECUTAR = mock.patch("agente.herramientas.subprocess.Popen", side_effect=AssertionError("no debe ejecutarse"))
HUNT = complementos.ruta("sat-hibrido")


class Temporal(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)


class Complementos(Temporal):
    def setUp(self):
        super().setUp()
        parche = mock.patch.object(complementos, "DIR", self.dir / "complementos")
        parche.start()
        self.addCleanup(parche.stop)

    def release(self, binario: bytes, sha: str):
        archivos = {"https://api.github.com/repos/yemsy26/Sat-hibrido-rust/releases/latest": json.dumps({
            "tag_name": "v1.0.0", "assets": [
                {"name": "hunt-x86_64-pc-windows-msvc.exe", "browser_download_url": "https://x/hunt.exe"},
                {"name": "hunt-x86_64-pc-windows-msvc.exe.sha256", "browser_download_url": "https://x/hunt.sha"}]}).encode(),
            "https://x/hunt.exe": binario, "https://x/hunt.sha": f"{sha}  hunt.exe\n".encode()}
        return mock.patch.object(complementos, "_descargar", lambda url: archivos[url])

    def test_sin_activar_no_hay_herramienta(self):
        datos = complementos.listar()
        self.assertEqual([d["id"] for d in datos], ["sat-hibrido"])
        self.assertFalse(datos[0]["activo"])
        self.assertIn("github.com/yemsy26/Sat-hibrido-rust", datos[0]["repo"])
        self.assertEqual(datos[0]["usa"][0]["nombre"], "Chronos-VFS")
        with mock.patch.dict("os.environ", {}, clear=False) as entorno:
            entorno.pop("AGENTE_HUNT", None)
            self.assertIsNone(matematicas.ruta_hunt())
            (self.dir / "f.cnf").write_text("p cnf 1 1\n1 0\n", encoding="utf-8")
            r = Herramientas(self.dir).resolver_sat("f.cnf")
            self.assertIn("no está activado", r)
            self.assertIn("python-sat", r)
            self.assertIn("NO activado", prompts._sat())

    @unittest.skipUnless(HUNT, "activa antes sat-hibrido (python -m agente.complementos activar sat-hibrido)")
    def test_activa_desde_la_version_publicada_y_comprueba_la_sha(self):
        datos = HUNT.read_bytes()
        with self.release(datos, hashlib.sha256(datos).hexdigest()):
            texto = complementos.activar("sat-hibrido", aviso=lambda _: None)
        self.assertIn("versión v1.0.0", texto)
        self.assertEqual(complementos.ruta("sat-hibrido"), self.dir / "complementos" / "sat-hibrido" / "hunt.exe")
        self.assertIn("desactivado", complementos.desactivar("sat-hibrido"))
        self.assertIsNone(complementos.ruta("sat-hibrido"))

    def test_sha_distinta_no_se_instala(self):
        with self.release(b"MZ falso", "0" * 64), self.assertRaisesRegex(RuntimeError, "SHA-256"):
            complementos.activar("sat-hibrido", aviso=lambda _: None)
        self.assertIsNone(complementos.ruta("sat-hibrido"))

    def test_sin_version_publicada_compila_y_si_no_puede_lo_explica(self):
        no_hay = urllib.error.HTTPError("u", 404, "Not Found", None, None)
        with mock.patch.object(complementos, "_descargar", side_effect=no_hay), \
                mock.patch.object(complementos.shutil, "which", return_value=None), \
                self.assertRaisesRegex(RuntimeError, "rustup"):
            complementos.activar("sat-hibrido", aviso=lambda _: None)

    @unittest.skipUnless(HUNT, "activa antes sat-hibrido")
    def test_sin_version_publicada_usa_el_codigo_del_repositorio(self):
        def compilar(_c, destino, _aviso):
            shutil.copyfile(HUNT, destino)
            return "compilado de Sat-hibrido-rust@abc1234"

        with mock.patch.object(complementos, "_desde_release", return_value=None), \
                mock.patch.object(complementos, "_compilar", compilar):
            self.assertIn("compilado de Sat-hibrido-rust@abc1234", complementos.activar("sat-hibrido", aviso=lambda _: None))

    def test_un_binario_que_no_resuelve_no_se_activa(self):
        falso = self.dir / "falso.exe"
        shutil.copyfile(sys.executable, falso)  # existe y arranca, pero no es un resolvedor SAT
        with mock.patch.object(complementos, "_desde_release", lambda c, d: shutil.copyfile(falso, d) and "v0"), \
                self.assertRaisesRegex(RuntimeError, "no funciona"):
            complementos.activar("sat-hibrido", aviso=lambda _: None)
        self.assertIsNone(complementos.ruta("sat-hibrido"))

    def test_al_instalar_se_activa_el_predeterminado_y_se_respeta_si_lo_desactivas(self):
        activados = []

        def activar(id_, aviso=print):
            activados.append(id_)
            return "activado"

        with mock.patch.object(complementos, "activar", activar):
            self.assertEqual(complementos.asegurar(), ["activado"])  # primera instalación
            complementos.desactivar("sat-hibrido")
            complementos._guardar({"sat-hibrido": {"activo": False}})
            self.assertIn("desactivado por el usuario", complementos.asegurar()[0])  # reinstalar no lo reactiva
        self.assertEqual(activados, ["sat-hibrido"])

    def test_sin_red_la_instalacion_sigue(self):
        with mock.patch.object(complementos, "_descargar", side_effect=urllib.error.URLError("sin red")), \
                mock.patch.object(complementos.shutil, "which", return_value=None):
            mensajes = complementos.asegurar(aviso=lambda _: None)
        self.assertIn("no se pudo activar ahora", mensajes[0])
        self.assertIn("Motores complementarios", mensajes[0])
        self.assertIsNone(complementos.ruta("sat-hibrido"))

    def test_cli_en_json_para_aura(self):
        r = subprocess.run([sys.executable, "-m", "agente.complementos", "listar", "--json"], capture_output=True,
                           encoding="utf-8", cwd=Path(__file__).resolve().parent.parent, check=True)
        self.assertEqual(json.loads(r.stdout)[0]["id"], "sat-hibrido")


SALIDA_FORGE = """Error: Compiler run failed:
Error (2314): Expected ';' but got '}'
 --> src/C.sol:5:44:
  |
Warning (2072): Unused local variable.
 --> src/C.sol:6:55:
"""
SALIDA_ANCHOR = """error[E0425]: cannot find value `cantida` in this scope
  --> programs/banco/src/lib.rs:14:31
   |
error: could not compile `banco` (lib) due to 1 previous error
"""


class Blockchain(Temporal):
    def test_errores_de_solidity_y_rust_con_su_linea(self):
        inf = Informe()
        self.assertEqual(ecosistemas._diagnosticos(SALIDA_FORGE, ecosistemas.SOLC, self.dir, self.dir, inf), 1)
        self.assertEqual(inf.errores, ["src\\C.sol:5 Expected ';' but got '}'"])
        self.assertEqual(inf.avisos, ["src\\C.sol:6 Unused local variable."])
        inf = Informe()
        ecosistemas._diagnosticos(SALIDA_ANCHOR, ecosistemas.RUSTC_LARGO, self.dir, self.dir, inf)
        self.assertEqual(inf.errores, ["programs\\banco\\src\\lib.rs:14 cannot find value `cantida` in this scope"])

    def test_anchor_explica_versiones_incompatibles_y_no_prueba_fuera_de_local(self):
        (self.dir / "Anchor.toml").write_text('[provider]\ncluster = "Devnet"\n', encoding="utf-8")
        (self.dir / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
        ordenes = []

        def correr(cmd, cwd, timeout=60, entorno=None):
            ordenes.append(cmd)
            return (1, SALIDA_ANCHOR) if fallar else (0, "ok")

        with mock.patch.object(ecosistemas.shutil, "which", return_value="anchor.exe"), \
                mock.patch.object(ecosistemas, "_correr", correr), \
                mock.patch.object(ecosistemas, "_version_de", lambda c: (0, 32, 1) if c[0] == "anchor" else (1, 18, 26)):
            fallar, inf = True, Informe()
            ecosistemas.anchor(self.dir, self.dir, inf)
            self.assertIn("Solana/Agave 2.x", " ".join(inf.avisos))
            fallar, inf = False, Informe()
            ecosistemas.anchor(self.dir, self.dir, inf)
            self.assertIn("anchor test desplegaría allí", " ".join(inf.avisos))
            self.assertNotIn(["anchor", "test"], ordenes)
            inf = Informe()
            ecosistemas.rust(self.dir, self.dir, inf)  # el workspace de Anchor no se compila dos veces
            self.assertNotIn("rust", inf.tipos)

    def test_hardhat_sin_script_de_test_no_da_aviso_falso(self):
        (self.dir / "package.json").write_text('{"name": "x"}', encoding="utf-8")
        (self.dir / "hardhat.config.js").write_text("module.exports = {};", encoding="utf-8")
        inf = Informe()
        with mock.patch.object(ecosistemas.shutil, "which", return_value="npm"):
            ecosistemas.node(self.dir, self.dir, inf)
            ecosistemas.solidity(self.dir, self.dir, inf)
        self.assertFalse([a for a in inf.avisos if "no hay pruebas" in a])

    def test_contratos_sueltos_piden_un_proyecto(self):
        (self.dir / "Token.sol").write_text("contract Token {}", encoding="utf-8")
        inf = Informe()
        ecosistemas.solidity(self.dir, self.dir, inf)
        self.assertIn("forge init", inf.avisos[0])

    def test_las_dependencias_de_foundry_no_son_del_proyecto(self):
        from agente.verificacion import archivos, ignorado
        (self.dir / "foundry.toml").write_text("[profile.default]", encoding="utf-8")
        for rel in ("lib/forge-std/scripts/vm.py", "lib/forge-std/package.json", "broadcast/D.s.sol/1/run.json",
                    "src/Boveda.sol", "scripts/analisis.py"):
            (self.dir / rel).parent.mkdir(parents=True, exist_ok=True)
            (self.dir / rel).write_text("x", encoding="utf-8")
        self.assertEqual([p.relative_to(self.dir).as_posix() for p in archivos(self.dir, ".py", ".json", ".sol")],
                         ["scripts/analisis.py", "src/Boveda.sol"])
        self.assertFalse(ecosistemas._buscar(self.dir, lambda p: p.name == "package.json"))
        otro = self.dir / "otro" / "lib" / "util.py"  # lib/ sin foundry.toml al lado sí es del proyecto
        otro.parent.mkdir(parents=True)
        otro.write_text("x", encoding="utf-8")
        self.assertFalse(ignorado(otro, self.dir))

    @unittest.skipUnless(shutil.which("forge") and (Path.home() / "AppData/Roaming/svm/0.8.28").is_dir(),
                         "Foundry con solc 0.8.28 en caché")
    def test_forge_real(self):
        (self.dir / "src").mkdir()
        (self.dir / "test").mkdir()
        (self.dir / "foundry.toml").write_text('[profile.default]\nlibs = []\noffline = true\nsolc_version = "0.8.28"\n',
                                               encoding="utf-8")
        (self.dir / "src" / "C.sol").write_text("// SPDX-License-Identifier: MIT\npragma solidity ^0.8.24;\n"
                                                "contract C { uint256 public x; function set(uint256 v) public { x = v; } }\n",
                                                encoding="utf-8")
        (self.dir / "test" / "C.t.sol").write_text(
            '// SPDX-License-Identifier: MIT\npragma solidity ^0.8.24;\nimport "../src/C.sol";\ncontract CTest {\n'
            "  function test_ok() public { C c = new C(); c.set(3); require(c.x() == 3); }\n"
            '  function test_mal() public { C c = new C(); c.set(2); require(c.x() == 3, "esperaba 3"); }\n}\n',
            encoding="utf-8")
        inf = Informe()
        ecosistemas.solidity(self.dir, self.dir, inf)
        self.assertIn("forge test falla (1 de 2)", inf.errores[0])
        self.assertIn("[FAIL: esperaba 3] test_mal()", inf.errores[0])


class Android(Temporal):
    def test_pistas_de_gradle(self):
        casos = {"SDK location not found. Define a valid SDK location": "abrir_android_studio",
                 "Failed to install the following Android SDK packages as some licences have not been accepted": "licencias",
                 "Failed to find Platform SDK with path: platforms;android-99": "compileSdk",
                 "Android Gradle plugin requires Java 17 to run": "JDK 17",
                 "Minimum supported Gradle version is 8.11.1. Current version is 8.4.": "distributionUrl"}
        for salida, esperado in casos.items():
            self.assertIn(esperado, " ".join(ecosistemas.pistas_gradle(salida)), salida)

    def test_errores_de_kotlin_java_y_dart(self):
        salida = ("e: file:///C:/p/app/src/main/java/a/MainActivity.kt:12:5 Unresolved reference 'foo'.\n"
                  "w: file:///C:/p/app/src/main/java/a/MainActivity.kt:3:1 Variable 'x' is never used\n"
                  "C:\\p\\app\\src\\main\\java\\a\\B.java:7: error: cannot find symbol\n")
        inf = Informe()
        for patron in (ecosistemas.KOTLIN, ecosistemas.JAVAC):
            ecosistemas._diagnosticos(salida, patron, Path("C:/p"), Path("C:/p"), inf)
        self.assertEqual(inf.errores, ["app\\src\\main\\java\\a\\MainActivity.kt:12 Unresolved reference 'foo'.",
                                       "app\\src\\main\\java\\a\\B.java:7 cannot find symbol"])
        self.assertEqual(len(inf.avisos), 1)
        inf = Informe()
        ecosistemas._diagnosticos("  error • Undefined name 'contador' • lib/main.dart:20:7 • undefined_identifier\n",
                                  ecosistemas.DART, self.dir, self.dir, inf)
        self.assertEqual(inf.errores, ["lib\\main.dart:20 Undefined name 'contador'"])

    def test_sin_wrapper_da_el_comando_con_el_gradle_del_equipo(self):
        (self.dir / "settings.gradle.kts").write_text('include(":app")', encoding="utf-8")
        (self.dir / "app").mkdir()
        (self.dir / "app" / "build.gradle.kts").write_text('plugins { id("com.android.application") }', encoding="utf-8")
        flutter = self.dir / "flutter_app" / "android" / "app"
        flutter.mkdir(parents=True)
        (self.dir / "flutter_app" / "pubspec.yaml").write_text("name: x\n", encoding="utf-8")
        (flutter / "build.gradle").write_text("apply plugin: 'com.android.application'", encoding="utf-8")
        inf = Informe()
        with mock.patch.object(android, "sdk", return_value=self.dir), \
                mock.patch.object(android, "gradle_local", return_value=("8.10.2", Path("C:/g/gradle.bat"))), \
                mock.patch.object(ecosistemas, "_version_de", return_value=(17, 0, 14)):
            ecosistemas.android_gradle(self.dir, self.dir, inf)
        self.assertEqual(inf.tipos, ["android"])
        self.assertEqual(len(inf.errores), 1)  # el android/ de Flutter no cuenta como proyecto aparte
        self.assertIn('"C:\\g\\gradle.bat" wrapper --gradle-version 8.10.2', inf.errores[0])

    def test_java_antiguo_y_sin_sdk(self):
        (self.dir / "settings.gradle").write_text("", encoding="utf-8")
        (self.dir / "build.gradle").write_text("apply plugin: 'com.android.application'", encoding="utf-8")
        for sdk, java, esperado in ((None, (17, 0, 1), "SDK de Android"), (self.dir, (1, 8, 0), "Java 8")):
            inf = Informe()
            with mock.patch.object(android, "sdk", return_value=sdk), \
                    mock.patch.object(ecosistemas, "_version_de", return_value=java):
                ecosistemas.android_gradle(self.dir, self.dir, inf)
            self.assertIn(esperado, inf.errores[0])

    def test_elige_la_instalacion_completa_de_android_studio(self):
        for nombre, completa in (("Android Studio", False), ("Android Studio1", True)):
            raiz = self.dir / "Android" / nombre
            (raiz / "bin").mkdir(parents=True)
            (raiz / "bin" / "studio64.exe").write_bytes(b"")
            if completa:
                (raiz / "jbr" / "lib").mkdir(parents=True)
                (raiz / "jbr" / "lib" / "jvm.cfg").write_text("-server KNOWN", encoding="utf-8")
                (raiz / "product-info.json").write_text("{}", encoding="utf-8")
        with mock.patch.dict("os.environ", {"ProgramFiles": str(self.dir)}):
            self.assertEqual(android.android_studio(), self.dir / "Android" / "Android Studio1" / "bin" / "studio64.exe")

    def test_javac_no_se_mete_en_proyectos_android_ni_flutter(self):
        from agente.verificacion import _de_android
        flutter = self.dir / "app_flutter" / "android" / "app" / "src" / "Gen.java"
        nativo = self.dir / "app_kotlin" / "app" / "src" / "main" / "java" / "A.java"
        suelto = self.dir / "java_simple" / "Main.java"
        for f in (flutter, nativo, suelto):
            f.parent.mkdir(parents=True, exist_ok=True)
            f.write_text("class X {}", encoding="utf-8")
        (self.dir / "app_flutter" / "pubspec.yaml").write_text("name: x", encoding="utf-8")
        (self.dir / "app_kotlin" / "app" / "build.gradle.kts").write_text('plugins { id("com.android.application") }',
                                                                        encoding="utf-8")
        self.assertTrue(_de_android(flutter, self.dir))
        self.assertTrue(_de_android(nativo, self.dir))
        self.assertFalse(_de_android(suelto, self.dir))

    def test_resultados_junit(self):
        carpeta = self.dir / "app" / "build" / "test-results" / "testDebugUnitTest"
        carpeta.mkdir(parents=True)
        (carpeta / "TEST-a.xml").write_text(
            '<testsuite name="a" tests="3" skipped="0" failures="1" errors="0">'
            '<testcase name="suma" classname="a"/><testcase name="resta" classname="a">\n<failure message="x"/>'
            "</testcase></testsuite>", encoding="utf-8")
        inf = Informe()
        ecosistemas._pruebas_junit(self.dir, ".", inf)
        self.assertEqual(inf.errores, [".: fallan 1 de 3 pruebas unitarias: resta"])

    def test_entorno_con_sdk_y_android_studio(self):
        (self.dir / "platform-tools").mkdir()
        (self.dir / "platforms" / "android-35").mkdir(parents=True)
        with mock.patch.dict("os.environ", {"ANDROID_HOME": str(self.dir)}):
            env = Herramientas(self.dir / "p")._entorno()
            self.assertEqual(env["ANDROID_SDK_ROOT"], str(self.dir))
            self.assertIn(str(self.dir / "platform-tools"), env["PATH"])
            self.assertEqual(android.plataformas(), [35])
        h = Herramientas(self.dir / "p")
        with mock.patch.object(android, "android_studio", return_value=None):
            self.assertIn("No encuentro Android Studio", h.abrir_android_studio())
        with mock.patch.object(android, "android_studio", return_value=Path("C:/AS/bin/studio64.exe")), \
                mock.patch("agente.herramientas.subprocess.Popen") as popen:
            self.assertIn("sincronizará Gradle", despachar(h, "abrir_android_studio", {}))
        self.assertEqual(popen.call_args[0][0], [str(Path("C:/AS/bin/studio64.exe")), str(self.dir / "p")])

    @NO_EJECUTAR
    def test_aceptar_licencias_lo_decide_el_usuario(self, _popen):
        vistos = []
        h = Herramientas(self.dir, confirmar=lambda c, m: vistos.append(m) or False)
        self.assertIn("aceptar licencias lo decide el usuario", h.ejecutar("sdkmanager --licenses"))
        self.assertEqual(vistos, [LICENCIA])


class BasesDeDatos(unittest.TestCase):
    def test_remotas_piden_permiso_y_las_locales_no(self):
        for comando in ("psql -h db.ejemplo.com -U app", "mysql -h10.0.0.5 -u root", "redis-cli -h cache.ejemplo.net",
                        "mongosh mongodb+srv://cluster0.abc.mongodb.net", "sqlcmd -S servidor.ejemplo.com -Q x",
                        "set DATABASE_URL=postgres://u:p@db.ejemplo.com/app && npx prisma migrate deploy"):
            self.assertTrue(bd_remota(comando) and externo(comando), comando)
        for comando in ("psql -h localhost -U postgres", "docker compose exec -T postgres psql -h db -U postgres",
                        "mysql -h 127.0.0.1 -u root", "set DATABASE_URL=postgresql://u:p@localhost:5432/app",
                        "redis-cli ping", "curl -h"):
            self.assertFalse(bd_remota(comando), comando)

    def test_borrar_datos_pide_permiso(self):
        for comando in ('psql -c "DROP TABLE usuarios"', 'psql -c "TRUNCATE pedidos"', 'psql -c "DELETE FROM usuarios;"',
                        "docker compose down -v", "docker volume rm pgdatos", 'redis-cli FLUSHALL',
                        "mongosh --eval 'db.dropDatabase()'"):
            self.assertTrue(PELIGROSOS.search(comando), comando)
        for comando in ('psql -c "DELETE FROM usuarios WHERE id = 3"', "docker compose down", "docker compose up -d --wait"):
            self.assertFalse(PELIGROSOS.search(comando), comando)


class Procesos(unittest.TestCase):
    def test_lo_que_queda_en_segundo_plano_no_cuelga_la_espera(self):
        d = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, d, True)
        lineas = ["@echo off", 'start /b cmd /c "ping -n 8 127.0.0.1 > nul"', "echo listo"]
        (d / "servidor.bat").write_text("\n".join(lineas) + "\n", encoding="utf-8")
        inicio = time.time()
        r = Herramientas(d).ejecutar("servidor.bat", segundos=20)
        self.assertTrue(r.startswith("exit=0"), r)
        self.assertIn("listo", r)
        self.assertLess(time.time() - inicio, 6)  # el ping sigue 8 s en segundo plano

    def test_matar_por_nombre_pide_permiso(self):
        for comando in ("taskkill /F /IM cmd.exe", "taskkill /f /im anvil.exe 2>nul", "Stop-Process -Name node",
                        "pkill -f anvil", "wmic process where name='node.exe' delete"):
            self.assertTrue(PELIGROSOS.search(comando), comando)
        for comando in ("taskkill /PID 4321 /T /F", 'tasklist /FI "IMAGENAME eq anvil.exe"'):
            self.assertFalse(PELIGROSOS.search(comando), comando)


class Aprender(Temporal):
    def setUp(self):
        super().setUp()
        parche = mock.patch.object(conocimiento, "APRENDIDO", self.dir / "aprendido")
        parche.start()
        self.addCleanup(parche.stop)

    def test_lo_aprendido_aparece_en_la_guia_del_tema(self):
        h = Herramientas(self.dir / "p")
        r = despachar(h, "aprender", {"leccion": "forge install necesita git init antes; usa forge init --force .",
                                      "tema": "solidity"})
        self.assertIn("'evm'", r)  # solidity es un alias de evm
        self.assertIn("forge init --force", conocimiento.guia("foundry"))
        self.assertIn("ya estaba", h.aprender("forge install necesita git init antes", "evm"))
        self.assertIn("no válido", h.aprender("una receta larga suficiente", "../fuera"))
        h.aprender("Para NATS en Go: nats.Connect(nats.DefaultURL) y Drain() al cerrar.", "nats")
        self.assertIn("solo lo aprendido", conocimiento.guia("nats"))

    def test_experimentar_fuera_del_proyecto(self):
        h = Herramientas(self.dir / "p")
        r = despachar(h, "experimentar", {"codigo": "print(sum(range(10)))"})
        self.assertTrue(r.startswith("exit=0"), r)
        self.assertIn("45", r)
        self.assertTrue((self.dir / "p" / ".agente" / "experimento" / "prueba.py").is_file())
        self.assertFalse(list((self.dir / "p").glob("*.py")))
        self.assertIn("TIMEOUT", h.experimentar("import time\ntime.sleep(30)", segundos=5))
        self.assertIn("Lenguajes de experimentar", h.experimentar("x", "cobol"))
        vistos = []
        h.confirmar = lambda c, m: vistos.append(c) or False
        self.assertIn("BLOQUEADO", h.experimentar("@echo off\nrmdir /s /q C:\\datos", "bat"))
        self.assertEqual(vistos, ["rmdir /s /q C:\\datos"])


class LlamadaCortada(Temporal):
    """Un archivo grande cortado por el máximo de la respuesta dejaba la conversación con JSON roto y el servidor del
    modelo contestaba HTTP 500 a todo lo siguiente."""

    def test_el_historial_queda_valido_y_el_archivo_se_escribe_por_partes(self):
        import io

        sys.path.insert(0, str(Path(__file__).resolve().parent))
        from test_agente import LLMGuion, llamada

        from agente import __main__ as cli
        from agente import config
        enviados = []

        class Grabadora(LLMGuion):
            def chat(self, mensajes, herramientas, max_tokens, razonar, al_progreso=None):
                enviados.append(json.loads(json.dumps(mensajes)))
                return super().chat(mensajes, herramientas, max_tokens, razonar, al_progreso)

        LLMGuion.respuestas = [
            {"content": "Creo la página.", "finish_reason": "length", "usage": {},
             "tool_calls": [{"id": "c1", "name": "escribir", "arguments": '{"ruta": "index.html", "contenido": "<!DOCTYPE'}]},
            llamada("escribir", ruta="index.html", contenido="<p>1</p>"),
            llamada("escribir", ruta="index.html", contenido="<p>2</p>", anadir=True),
            {"content": "Listo.", "tool_calls": [], "finish_reason": "stop", "usage": {}},
        ]
        ordenes = [{"tipo": "tarea", "texto": "crea un catálogo"}, {"tipo": "salir"}]
        entrada = io.TextIOWrapper(io.BytesIO("".join(json.dumps(o) + "\n" for o in ordenes).encode()), encoding="utf-8")
        salida = io.StringIO()
        falso = lambda cfg: ("http://falso", "falso", 32768)
        with mock.patch.object(cli, "detectar", falso), mock.patch("agente.bucle.cliente.detectar", falso), \
                mock.patch("agente.bucle.cliente.LLM", Grabadora), mock.patch("agente.bucle.entorno", lambda: "prueba"), \
                mock.patch.object(sys, "stdin", entrada), mock.patch.object(sys, "stdout", salida):
            self.assertEqual(cli.modo_json(config.cargar(), str(self.dir)), 0)
        cortada = next(m for m in enviados[1] if m.get("tool_calls"))
        self.assertEqual(cortada["tool_calls"][0]["function"]["arguments"], "{}")
        for mensajes in enviados:  # nunca se manda al servidor una llamada con JSON roto
            for m in mensajes:
                for tc in m.get("tool_calls") or []:
                    json.loads(tc["function"]["arguments"])
        aviso = next(m["content"] for m in enviados[1] if m["role"] == "tool")
        self.assertIn("se CORTÓ", aviso)
        self.assertIn("anadir=true", aviso)
        self.assertEqual((self.dir / "index.html").read_text(encoding="utf-8"), "<p>1</p>\n<p>2</p>")


class GuiasYVersiones(unittest.TestCase):
    def test_guias_nuevas_y_sus_alias(self):
        for tema, texto in (("cripto", "varias redes"), ("ethereum", "Foundry"), ("anchor", "Anchor"),
                            ("arbitraje", "Flashbots"), ("kotlin", "Jetpack Compose"), ("dart", "flutter create"),
                            ("postgres", "migraciones"), ("mev", "sandwich")):
            self.assertIn(texto, conocimiento.guia(tema), tema)
        for regla in ("experimentar", "abrir_android_studio", "bases-de-datos", "resolvedor SAT activado"):
            self.assertIn(regla, prompts.METODO)

    def test_versiones_de_google_y_pub(self):
        respuestas = {
            "https://dl.google.com/android/maven2/androidx/core/core-ktx/maven-metadata.xml":
                "<versions><version>1.18.0</version><version>1.19.0-alpha01</version><version>1.19.1</version>"
                "<version>1.20.0-rc01</version></versions>",
            "https://pub.dev/api/packages/provider": json.dumps({"name": "provider", "latest": {
                "version": "6.1.5", "pubspec": {"environment": {"sdk": ">=2.12.0 <4.0.0"}}}}),
        }
        h = mock.Mock()
        h._descargar = lambda url, datos=None: (respuestas[url], "text/plain")
        self.assertIn("androidx.core:core-ktx:1.19.1", conocimiento.version(h, "androidx.core:core-ktx", "google"))
        self.assertIn("grupo:artefacto", conocimiento.version(h, "core-ktx", "google"))
        self.assertIn("flutter pub add provider", conocimiento.version(h, "provider", "pub"))


if __name__ == "__main__":
    unittest.main()
