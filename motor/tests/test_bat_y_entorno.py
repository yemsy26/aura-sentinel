"""Lo que hizo fallar la misión Aura-Monitor fuera del código: run.bat sin call, el entorno heredado del Aura de
desarrollo, cerrar procesos ajenos y restaurar la versión de la que el usuario acababa de decir que fallaba."""
import io
import json
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_agente import LLMGuion, borrar, llamada

from agente import __main__ as cli
from agente import config
from agente.entorno import limpio
from agente.herramientas import Herramientas
from agente.verificacion import Informe, bats, con_call

LISTO = {"content": "Listo.", "tool_calls": [], "finish_reason": "stop", "usage": {}}


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_bat_"))
        self.h = Herramientas(self.dir)

    def tearDown(self):
        self.h.detener_todos()
        borrar(self.dir)


class Call(Base):
    def test_con_call_solo_donde_hace_falta(self):
        casos = {"npm install": "call npm install", "npx tauri dev": "call npx tauri dev",
                 "cd src-tauri && npm run build": "cd src-tauri && call npm run build",
                 "cargo build && npx tauri build || echo mal": "cargo build && call npx tauri build || echo mal",
                 "otro.bat": "call otro.bat", "call npm i": "call npm i", "python main.py": "python main.py",
                 "cargo test": "cargo test", "echo a ^& npm": "echo a ^& npm", 'cd "a && b" && yarn': 'cd "a && b" && call yarn'}
        for linea, esperado in casos.items():
            self.assertEqual(con_call(linea), esperado, linea)

    def test_crear_bat_pone_call_y_el_bat_sigue_despues_de_npm(self):
        """El caso real: sin call, run.bat hacía npm install y terminaba (nunca llegaba a npx tauri dev).
        Nunca con el npm de verdad: sin package.json en la carpeta, npm sube y toca el de una carpeta superior."""
        falsos = self.dir / "falsos"
        falsos.mkdir()
        (falsos / "npm.cmd").write_text("@echo npm falso %*\r\n", encoding="utf-8")
        (self.dir / "package.json").write_text('{"name": "prueba", "private": true}', encoding="utf-8")  # por si acaso
        self.h.crear_bat("run", ["npm install", 'python -c "print(\'segundo paso\')"'])
        texto = (self.dir / "run.bat").read_text(encoding="utf-8")
        self.assertIn("call npm install", texto)
        ruta = f"{falsos}{os.pathsep}{os.environ.get('PATH', '')}"
        with mock.patch.dict(os.environ, {"PATH": ruta}):  # el npm falso va primero en el PATH
            salida = self.h.ejecutar("run.bat", segundos=30)
        self.assertIn("npm falso install", salida)
        self.assertIn("segundo paso", salida)

    def test_verificar_avisa_de_npm_sin_call(self):
        (self.dir / "run.bat").write_text('@echo off\ncd /d "%~dp0"\nnpm install\nnpx tauri dev\npause\n', encoding="utf-8")
        inf = Informe()
        bats(self.dir, [self.dir / "run.bat"], inf)
        texto = "\n".join(inf.errores)
        self.assertIn("run.bat:3 'npm install' sin call", texto)
        self.assertIn("run.bat:4 'npx tauri dev' sin call", texto)  # sin call tampoco llega al pause
        (self.dir / "ok.bat").write_text("@echo off\ncall npm install\ncall npx tauri dev\npause\n", encoding="utf-8")
        (self.dir / "ultimo.bat").write_text("@echo off\nnpm start\n", encoding="utf-8")  # lo último: no corta nada
        inf = Informe()
        bats(self.dir, [self.dir / "ok.bat", self.dir / "ultimo.bat"], inf)
        self.assertFalse([e for e in inf.errores if "sin call" in e])


class Entorno(unittest.TestCase):
    def test_quita_lo_del_lanzador_y_respeta_lo_guardado(self):
        env = {"CARGO_TARGET_DIR": r"C:\aura\target-audit", "CARGO_PKG_NAME": "app", "CARGO_MANIFEST_DIR": r"C:\aura",
               "npm_config_cache": "x", "npm_lifecycle_event": "tauri", "INIT_CWD": r"C:\aura", "TAURI_CONFIG": "{}",
               "TAURI_SIGNING_PRIVATE_KEY": "clave", "RUSTUP_TOOLCHAIN": "stable", "CARGO_HOME": r"C:\u\.cargo",
               "PATH": r"C:\aura\node_modules\.bin;C:\Windows;C:\u\.cargo\bin"}
        sin_guardar = limpio(env, guardada=lambda nombre: None)
        for nombre in ("CARGO_TARGET_DIR", "CARGO_PKG_NAME", "CARGO_MANIFEST_DIR", "npm_config_cache",
                       "npm_lifecycle_event", "INIT_CWD", "TAURI_CONFIG", "RUSTUP_TOOLCHAIN"):
            self.assertNotIn(nombre, sin_guardar)
        self.assertEqual(sin_guardar["TAURI_SIGNING_PRIVATE_KEY"], "clave")
        self.assertEqual(sin_guardar["CARGO_HOME"], r"C:\u\.cargo")
        self.assertEqual(sin_guardar["PATH"], r"C:\Windows;C:\u\.cargo\bin")
        guardado = limpio(env, guardada=lambda nombre: env.get(nombre) if nombre == "CARGO_TARGET_DIR" else None)
        self.assertEqual(guardado["CARGO_TARGET_DIR"], r"C:\aura\target-audit")  # la puso el usuario: se respeta


class ProcesosAjenos(Base):
    def test_cerrar_por_pid_un_proceso_que_no_es_suyo_se_bloquea(self):
        r = self.h.ejecutar("taskkill /F /PID 999999")
        self.assertIn("BLOQUEADO: ese proceso no lo lanzó el agente", r)
        self.assertIn("BLOQUEADO", self.h.ejecutar("powershell -c Stop-Process -Id 999999 -Force"))

    def test_los_suyos_si(self):
        self.h.ejecutar("ping -n 30 127.0.0.1 >nul", fondo=True)
        for pid in self.h.propios():  # el cmd y el ping que lanzó: los dos son suyos
            self.assertIsNone(self.h._permitido(f"taskkill /F /PID {pid}"))

    def test_con_permiso_del_usuario_pasa(self):
        h = Herramientas(self.dir, confirmar=lambda comando, motivo: "no lanzó el agente" in motivo)
        self.assertIsNone(h._permitido("taskkill /F /PID 999999"))


class NpmFueraDelProyecto(unittest.TestCase):
    """Sin package.json, npm install sube y cambia los paquetes de una carpeta superior (la del usuario tiene uno)."""

    def setUp(self):
        self.base = Path(tempfile.mkdtemp(prefix="agente_npm_"))
        (self.base / "package.json").write_text('{"name": "de-arriba"}', encoding="utf-8")  # como C:\Users\<usuario>
        self.raiz = self.base / "proyecto"
        self.h = Herramientas(self.raiz)

    def tearDown(self):
        shutil.rmtree(self.base, ignore_errors=True)

    def test_se_bloquea_antes_de_ejecutar_npm(self):
        for comando in ("npm install lodash", f'cd /d "{self.raiz}" & npm install', "cd web && npm i vite",
                        "pnpm add react", "npm uninstall jsdom"):
            r = self.h.ejecutar(comando, segundos=10)
            self.assertIn("no hay package.json y npm usaría el de", r, comando)
            self.assertIn(str(self.base), r)
        self.assertEqual(json.loads((self.base / "package.json").read_text(encoding="utf-8")), {"name": "de-arriba"})

    def test_un_bat_que_hace_npm_install_tambien(self):
        self.h.crear_bat("run", ["npm install", "npx vite"])
        self.assertIn("BLOQUEADO", self.h._npm_fuera("run.bat"))

    def test_con_su_package_json_o_sin_instalar_no(self):
        for comando in ("npm test", "npx tauri dev", "npm run build", "npm init -y"):
            self.assertIsNone(self.h._npm_fuera(comando), comando)
        (self.raiz / "package.json").write_text("{}", encoding="utf-8")
        self.assertIsNone(self.h._npm_fuera("npm install lodash"))


class CheckpointsYUsuario(Base):
    def test_restaurar_recuerda_lo_que_dijo_el_usuario_despues(self):
        (self.dir / "a.txt").write_text("versión entregada\n", encoding="utf-8")
        self.h.verificar_cmd()
        self.h.nueva_tarea(10, "al iniciar con el archivo bat no inicia, te paso la captura")
        (self.dir / "a.txt").write_text("cambio a medias\n", encoding="utf-8")
        r = self.h.restaurar()
        self.assertIn("pasó verificar sin errores", r)
        self.assertNotIn("versión que funcionaba", r)
        self.assertIn("«al iniciar con el archivo bat no inicia, te paso la captura»", r)
        self.assertIn("LO TIENE", r)
        guardado = json.loads((self.dir / ".agente" / "checkpoints.json").read_text(encoding="utf-8"))
        self.assertEqual(guardado[0]["despues"], ["al iniciar con el archivo bat no inicia, te paso la captura"])

    def test_un_checkpoint_posterior_no_lleva_la_nota(self):
        (self.dir / "a.txt").write_text("uno\n", encoding="utf-8")
        self.h.verificar_cmd()
        self.h.nueva_tarea(10, "no arranca")
        (self.dir / "a.txt").write_text("dos\n", encoding="utf-8")
        self.h.verificar_cmd()
        self.assertNotIn("OJO", self.h.restaurar())


class CheckpointsEnLaMision(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_cpm_"))

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def mision(self, textos: list[str], respuestas: list) -> None:
        LLMGuion.respuestas = respuestas
        ordenes = [*({"tipo": "tarea", "texto": t} for t in textos), {"tipo": "salir"}]
        entrada = io.TextIOWrapper(io.BytesIO("".join(json.dumps(o) + "\n" for o in ordenes).encode()), encoding="utf-8")
        falso = lambda cfg: ("http://falso", "falso", 32768)
        with mock.patch.object(cli, "detectar", falso), mock.patch("agente.bucle.cliente.detectar", falso), \
                mock.patch("agente.bucle.cliente.LLM", LLMGuion), mock.patch("agente.bucle.entorno", lambda: "prueba"), \
                mock.patch.object(sys, "stdin", entrada), mock.patch.object(sys, "stdout", io.StringIO()):
            cli.modo_json(config.cargar(), str(self.dir))

    def despues(self) -> list:
        return json.loads((self.dir / ".agente" / "checkpoints.json").read_text(encoding="utf-8"))[0].get("despues", [])

    def test_el_mensaje_siguiente_del_usuario_queda_anotado(self):
        self.mision(["crea a.txt", "el bat no arranca"],
                    [llamada("escribir", ruta="a.txt", contenido="x\n"), llamada("verificar"), LISTO, LISTO])
        self.assertEqual(self.despues(), ["el bat no arranca"])

    def test_continua_no_anota(self):
        self.mision(["crea a.txt", "continúa"],
                    [llamada("escribir", ruta="a.txt", contenido="x\n"), llamada("verificar"), LISTO, LISTO])
        self.assertEqual(self.despues(), [])


if __name__ == "__main__":
    unittest.main()
