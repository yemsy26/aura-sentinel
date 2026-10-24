"""Abrir la app Tauri de verdad: cuándo se puede (compilada, dist/ al día, puerto de devUrl libre), qué informa verificar
y que ver_pantalla enseñe la ventana real en vez del frontend en un navegador (sin invoke ni datos)."""
import importlib.util
import json
import os
import shutil
import socket
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from agente import app_tauri, ecosistemas, pantalla
from agente.herramientas import Herramientas
from agente.verificacion import Informe

EN_WINDOWS = os.name == "nt"
HAY_PLAYWRIGHT = importlib.util.find_spec("playwright") is not None


def puerto_libre() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_app_"))
        self.tauri = self.dir / "src-tauri"
        self.tauri.mkdir()
        self.puerto = puerto_libre()
        self.conf({"devUrl": f"http://localhost:{self.puerto}", "frontendDist": "../dist"})
        (self.dir / "src").mkdir()
        (self.dir / "src" / "main.ts").write_text("console.log(1);\n", encoding="utf-8")
        (self.dir / "index.html").write_text('<script type="module" src="/src/main.ts"></script>', encoding="utf-8")
        self.exe = mock.patch.object(app_tauri, "ejecutable", lambda d, env: Path(sys.executable))
        self.exe.start()

    def tearDown(self):
        self.exe.stop()
        shutil.rmtree(self.dir, ignore_errors=True)

    def conf(self, build: dict) -> None:
        (self.tauri / "tauri.conf.json").write_text(json.dumps({"identifier": "com.prueba.app", "build": build}),
                                                     encoding="utf-8")

    def compilar_dist(self) -> None:
        (self.dir / "dist").mkdir(exist_ok=True)
        (self.dir / "dist" / "index.html").write_text("<p>compilado</p>", encoding="utf-8")
        futuro = time.time() + 5
        os.utime(self.dir / "dist" / "index.html", (futuro, futuro))


@unittest.skipUnless(EN_WINDOWS, "solo Windows")
class Preparar(Base):
    def test_sin_compilar(self):
        with mock.patch.object(app_tauri, "ejecutable", lambda d, env: None):
            fallo, *_ = app_tauri.preparar(self.tauri, {})
        self.assertIn("no está compilada", fallo["motivo"])

    def test_sin_dist_o_con_dist_viejo(self):
        fallo, *_ = app_tauri.preparar(self.tauri, {})
        self.assertIn("falta dist/index.html: ejecuta npm run build", fallo["motivo"])
        self.compilar_dist()
        os.utime(self.dir / "dist" / "index.html", (time.time() - 60, time.time() - 60))
        fallo, *_ = app_tauri.preparar(self.tauri, {})
        self.assertIn("anterior a tus cambios del frontend", fallo["motivo"])

    def test_puerto_ocupado_por_otro_programa(self):
        """El caso real: el puerto 1420 era el del propio Aura en desarrollo; no se toca, se avisa."""
        self.compilar_dist()
        with socket.socket() as s:
            s.bind(("127.0.0.1", self.puerto))
            s.listen()
            fallo, *_ = app_tauri.preparar(self.tauri, {})
        self.assertIn(f"el puerto {self.puerto} de devUrl ya lo usa otro programa", fallo["motivo"])

    def test_listo_para_abrir(self):
        self.compilar_dist()
        fallo, exe, dist, puerto = app_tauri.preparar(self.tauri, {})
        self.assertEqual((fallo, exe, dist, puerto), (None, Path(sys.executable), (self.dir / "dist").resolve(), self.puerto))
        self.conf({"frontendDist": "../dist"})  # sin devUrl, la de depuración lleva el frontend dentro
        self.assertEqual(app_tauri.preparar(self.tauri, {})[2:], (None, 0))

    @unittest.skipUnless(HAY_PLAYWRIGHT, "sin playwright")
    def test_la_app_que_se_cierra_al_arrancar_es_un_error(self):
        self.compilar_dist()
        r = app_tauri.probar(self.tauri, dict(os.environ), self.dir / "app.png", 1)  # python sin script: sale enseguida
        self.assertEqual(r["estado"], "error")
        self.assertIn("la app se cerró al arrancar", r["errores"][0])
        self.assertFalse(app_tauri._escucha(self.puerto))  # el servidor de dist/ se cerró


class Verificar(Base):
    def informe(self, resultado: dict, errores_previos: list | None = None) -> Informe:
        inf = Informe()
        inf.errores += errores_previos or []
        with mock.patch.object(app_tauri, "probar_una_vez", return_value=resultado) as probar:
            ecosistemas.abrir_app(self.tauri, "src-tauri/tauri.conf.json", self.dir, inf)
        self.llamada = probar.called
        return inf

    def test_la_app_abre_y_se_ve_el_texto(self):
        inf = self.informe(app_tauri._resultado("ok", texto="CPU 9.6 % RAM 27.7 GB"))
        self.assertTrue(any("la app abre su ventana" in o and "CPU 9.6 %" in o for o in inf.ok))

    def test_errores_de_la_app_abierta(self):
        inf = self.informe(app_tauri._resultado("error", errores=["excepción JS: invoke is not a function"]))
        self.assertIn("al abrir la app de verdad: excepción JS: invoke is not a function", inf.errores[0])

    def test_sin_poder_abrirla_avisa(self):
        inf = self.informe(app_tauri._resultado("omitido", "falta dist/index.html: ejecuta npm run build"))
        self.assertTrue(any("no abrí la app para probarla: falta dist" in a for a in inf.avisos))

    def test_con_errores_de_compilacion_no_se_abre(self):
        self.informe(app_tauri._resultado("ok", texto="x"), ["main.rs:3 error[E0425]"])
        self.assertFalse(self.llamada)


class VerPantalla(Base):
    def setUp(self):
        super().setUp()
        self.h = Herramientas(self.dir)
        self.evaluar = mock.patch.object(pantalla, "evaluar", return_value=(None, "sin modelo"))
        self.evaluar.start()

    def tearDown(self):
        self.evaluar.stop()
        super().tearDown()

    def probar(self, estado: str, motivo: str = ""):
        def falso(d, env, png, espera):
            png.write_bytes(b"png")
            return app_tauri._resultado(estado, motivo, texto="CPU USAGE 9.6 %")
        return mock.patch.object(app_tauri, "probar", side_effect=falso)

    def test_sin_ruta_o_con_el_dev_url_se_ve_la_ventana_real(self):
        for ruta in ("", f"http://localhost:{self.puerto}", "app"):
            with self.probar("ok"):
                r = self.h.ver_pantalla(ruta)
            self.assertIn("en la app Tauri (su ventana de verdad, no un navegador", r, ruta)
            self.assertIn("«CPU USAGE 9.6 %»", r)

    def test_si_no_se_puede_abrir_avisa_de_que_el_navegador_no_es_la_app(self):
        """El caso real: capturas sin datos del frontend en un navegador llevaron al agente a «arreglar» lo que iba."""
        captura = mock.patch.object(pantalla, "capturar_web", side_effect=lambda raiz, que, png, espera: (png.write_bytes(b"x"), [])[1])
        with self.probar("omitido", "el puerto 1420 de devUrl ya lo usa otro programa"), captura:
            r = self.h.ver_pantalla(f"http://localhost:{self.puerto}")
        self.assertIn("NOTA: no pude abrir la app de verdad (el puerto 1420", r)
        self.assertIn("FUERA de Tauri", r)

    def test_otra_url_local_no_es_la_app(self):
        self.assertFalse(pantalla._es_dev(f"http://localhost:{self.puerto + 1}", self.tauri))
        self.assertTrue(pantalla._es_dev(f"http://127.0.0.1:{self.puerto}/", self.tauri))


if __name__ == "__main__":
    unittest.main()
