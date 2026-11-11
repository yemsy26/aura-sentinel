"""Ingeniería inversa de APK (apps propias o con permiso): instalar apktool y jadx con SHA-256 fijado, tenerlas en el
PATH de los comandos, la guía, y que verificar compruebe que un APK decodificado se reconstruye y que el APK que se
entrega está alineado y firmado (sin eso no se instala)."""
import hashlib
import io
import shutil
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import conocimiento, ecosistemas, inversa
from agente.proyecto import Informe
from agente.verificacion import _de_android

APK = Path(__file__).resolve().parent / "datos" / "notas.apk"  # app de prueba propia (aapt2 + javac + d8, firmada)


def falsas(contenido_jar: bytes, contenido_zip: bytes) -> dict:
    return {"apktool": {"version": "9.9", "bytes": len(contenido_jar), "url": "https://x.test/apktool_9.9.jar",
                        "sha256": hashlib.sha256(contenido_jar).hexdigest()},
            "jadx": {"version": "8.8", "bytes": len(contenido_zip), "url": "https://x.test/jadx-8.8.zip",
                     "sha256": hashlib.sha256(contenido_zip).hexdigest()}}


def un_zip(nombres: dict) -> bytes:
    b = io.BytesIO()
    with zipfile.ZipFile(b, "w") as z:
        for n, t in nombres.items():
            z.writestr(n, t)
    return b.getvalue()


class Instalar(unittest.TestCase):
    def setUp(self):
        self.base = Path(tempfile.mkdtemp(prefix="test_inversa_"))
        self.jar, self.zip = b"jar falso", un_zip({"bin/jadx.bat": "@echo jadx", "lib/jadx.jar": "x"})
        self.parches = [mock.patch.object(inversa, "BASE", self.base)]
        for p in self.parches:
            p.start()

    def tearDown(self):
        for p in self.parches:
            p.stop()
        shutil.rmtree(self.base, ignore_errors=True)

    def bajar(self, contenidos: dict):
        return mock.patch.object(inversa, "_bajar", lambda url, destino: destino.write_bytes(contenidos[url]))

    def test_instala_comprueba_la_huella_y_deja_apktool_bat(self):
        datos = falsas(self.jar, self.zip)
        with mock.patch.dict(inversa.HERRAMIENTAS, datos), self.bajar({datos["apktool"]["url"]: self.jar,
                                                                        datos["jadx"]["url"]: self.zip}):
            hecho = inversa.instalar()
            self.assertTrue(all("instalado" in h for h in hecho), hecho)
            self.assertEqual((self.base / "apktool-9.9" / "apktool.jar").read_bytes(), self.jar)
            self.assertIn('java -jar "%~dp0apktool.jar" %*', (self.base / "apktool-9.9" / "apktool.bat").read_text())
            self.assertEqual(inversa.jadx(), self.base / "jadx-8.8" / "bin" / "jadx.bat")
            self.assertEqual(sorted(p.name for p in self.base.iterdir()), ["apktool-9.9", "jadx-8.8"])  # sin restos
            self.assertTrue(all("ya estaba" in h for h in inversa.instalar()))

    def test_huella_distinta_no_instala_nada(self):
        datos = falsas(self.jar, self.zip)
        with (mock.patch.dict(inversa.HERRAMIENTAS, datos), self.bajar({datos["apktool"]["url"]: b"otro contenido"}),
              self.assertRaisesRegex(RuntimeError, "SHA-256 no coincide")):
            inversa.instalar(["apktool"])
        self.assertIsNone(inversa.apktool())
        self.assertEqual(list(self.base.iterdir()), [])

    def test_zip_con_rutas_fuera_de_su_carpeta_no_se_extrae(self):
        malo = un_zip({"../../fuera.bat": "echo"})
        datos = falsas(self.jar, malo)
        with (mock.patch.dict(inversa.HERRAMIENTAS, datos), self.bajar({datos["jadx"]["url"]: malo}),
              self.assertRaisesRegex(RuntimeError, "fuera de su carpeta")):
            inversa.instalar(["jadx"])
        self.assertFalse((self.base.parent / "fuera.bat").exists())

    def test_entorno_y_resumen(self):
        datos = falsas(self.jar, self.zip)
        with mock.patch.dict(inversa.HERRAMIENTAS, datos), mock.patch.object(inversa, "build_tools", lambda: None):
            self.assertEqual(inversa.resumen(), "")  # sin nada instalado ni SDK, el entorno no dice nada
            with self.bajar({datos["apktool"]["url"]: self.jar, datos["jadx"]["url"]: self.zip}):
                inversa.instalar()
            self.assertEqual(inversa.resumen(), "ingeniería inversa de APK: apktool 9.9, jadx 8.8")
            ruta = inversa.con_herramientas({"PATH": "C:\\otro"})["PATH"]
            self.assertTrue(ruta.startswith(str(self.base / "apktool-9.9")) and ruta.endswith("C:\\otro"))
            self.assertIn(str(self.base / "jadx-8.8" / "bin"), ruta)

    def test_si_faltan_dice_como_instalarlas(self):
        with mock.patch.object(inversa, "build_tools", lambda: Path("C:/sdk/build-tools/36.0.0")):
            texto = inversa.resumen()
        self.assertIn("faltan apktool y jadx", texto)
        self.assertIn("instalar_herramientas.bat", texto)


class Guia(unittest.TestCase):
    def test_alias_y_contenido(self):
        for tema in ("apktool", "jadx", "smali", "decompilar", "js-minificado", "ingenieria-inversa"):
            self.assertEqual(conocimiento._clave(tema), "ingenieria-inversa")
        texto = conocimiento.guia("smali")
        self.assertIn("zipalign -p -f 4 sin_firmar.apk alineado.apk", texto)
        self.assertIn("const-string", texto)
        self.assertIn("propias o con permiso", texto)

    def test_el_java_de_jadx_no_se_compila(self):
        d = Path(tempfile.mkdtemp(prefix="test_jadx_"))
        try:
            (d / "salida" / "sources" / "com").mkdir(parents=True)
            (d / "salida" / "resources").mkdir()
            java = d / "salida" / "sources" / "com" / "A.java"
            java.write_text("class A {}", encoding="utf-8")
            self.assertTrue(_de_android(java, d))
            (d / "salida" / "resources").rmdir()
            self.assertFalse(_de_android(java, d))  # un sources/ cualquiera sí se compila
        finally:
            shutil.rmtree(d, ignore_errors=True)


@unittest.skipUnless(APK.is_file() and inversa.apktool() and inversa.build_tools(),
                     "hace falta apktool y las build-tools del SDK (y tests/datos/notas.apk)")
class VerificarApk(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_apk_"))
        self.emulador = mock.patch.object(ecosistemas, "_emulador", lambda: None)  # nada se instala en un emulador
        self.emulador.start()

    def tearDown(self):
        self.emulador.stop()
        shutil.rmtree(self.dir, ignore_errors=True)

    def informe(self) -> Informe:
        inf = Informe()
        ecosistemas.apk(self.dir, self.dir, inf)
        return inf

    def decodificar(self) -> Path:
        dec = self.dir / "dec"
        codigo, salida = ecosistemas.correr_texto(f'java -jar "{inversa.apktool()}" d -f -o "{dec}" "{APK}"', self.dir,
                                                  120, ecosistemas.ENTORNO)
        self.assertEqual(codigo, 0, salida)
        return dec

    def test_apk_firmado_y_alineado(self):
        shutil.copy(APK, self.dir / "app.apk")
        inf = self.informe()
        self.assertEqual(inf.errores, [])
        self.assertIn("app.apk: APK com.yemsy.notas 1.4.2: aapt2 lo lee, está alineado y firmado", inf.ok)

    def test_el_mas_reciente_sin_firmar_es_un_error(self):
        dec = self.decodificar()
        shutil.copy(APK, self.dir / "original.apk")
        codigo, salida = ecosistemas.correr_texto(f'java -jar "{inversa.apktool()}" b "{dec}" -o "{self.dir / "nuevo.apk"}"',
                                                  self.dir, 120, ecosistemas.ENTORNO)
        self.assertEqual(codigo, 0, salida)
        inf = self.informe()
        self.assertIn("dec: el APK decodificado se reconstruye (apktool b)", inf.ok)
        self.assertEqual(len(inf.errores), 2, inf.errores)  # sin alinear y sin firmar
        self.assertTrue(any("nuevo.apk: no está alineado" in e for e in inf.errores))
        self.assertTrue(any("nuevo.apk: no está firmado" in e for e in inf.errores))

    def test_smali_roto_da_archivo_y_linea(self):
        dec = self.decodificar()
        smali = dec / "smali" / "com" / "yemsy" / "notas" / "MainActivity.smali"
        smali.write_text(smali.read_text(encoding="utf-8").replace("const-string v0, \"Guardado en",
                                                                   "const-strinx v0, \"Guardado en"), encoding="utf-8")
        inf = self.informe()
        self.assertTrue(any(e.startswith("apktool b: dec\\smali\\com\\yemsy\\notas\\MainActivity.smali:") and
                            "const-strinx" in e for e in inf.errores), inf.errores)
        self.assertFalse((dec / "build").exists())  # se reconstruye en una copia: el proyecto no se toca


if __name__ == "__main__":
    unittest.main()
