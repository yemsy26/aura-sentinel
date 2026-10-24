"""El agente controla exactamente lo que lanza (trabajos de Windows) y no cierra nada del usuario: ni con procesos
detener, ni al cerrarse el motor, ni con taskkill /IM o /PID. Una misión cerró la consola donde el usuario tenía
abierto el servidor de Aura."""
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
import uuid
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from agente import procesos
from agente.herramientas import Herramientas

PING = Path(os.environ.get("SystemRoot", r"C:\Windows")) / "System32" / "PING.EXE"


def vivo(pid: int) -> bool:
    return bool(procesos.nombre(pid))


def esperar(condicion, segundos: float = 5) -> bool:
    fin = time.time() + segundos
    while time.time() < fin:
        if condicion():
            return True
        time.sleep(0.1)
    return condicion()


@unittest.skipUnless(procesos.EN_WINDOWS and PING.exists(), "solo Windows")
class Trabajos(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_proc_"))
        # un programa con nombre único: aunque algo fallara, taskkill /IM nunca tocaría nada de fuera de la prueba
        self.nombre = f"aura_prueba_{uuid.uuid4().hex[:8]}.exe"
        self.exe = self.dir / self.nombre
        shutil.copy(PING, self.exe)
        self.usuario = subprocess.Popen([str(self.exe), "-n", "60", "127.0.0.1"], stdout=subprocess.DEVNULL,
                                        creationflags=subprocess.CREATE_NO_WINDOW)  # «el programa que dejó el usuario»
        self.h = Herramientas(self.dir)

    def tearDown(self):
        self.h.detener_todos()
        for p in list(self.h.propios()):
            procesos.cerrar_pid(p)
        self.usuario.kill()
        self.usuario.wait()
        time.sleep(0.3)
        shutil.rmtree(self.dir, ignore_errors=True)

    def lanzar_fondo(self) -> int:
        self.h.ejecutar(f'"{self.exe}" -n 60 127.0.0.1', fondo=True)
        n = max(self.h.procesos)
        self.assertTrue(esperar(lambda: any(nombre == self.nombre for nombre in self.h.propios().values())))
        return n

    def test_un_trabajo_cerrado_no_devuelve_los_procesos_del_motor(self):
        """Con el handle a NULL, Windows devolvía los procesos del trabajo del llamador: la limpieza cerró la prueba."""
        trabajo = procesos.Trabajo()
        trabajo.cerrar()
        self.assertEqual(trabajo.pids(), [])
        trabajo.terminar()  # no hace nada
        self.assertFalse(procesos.cerrar_pid(os.getpid()))

    def test_el_trabajo_incluye_lo_que_abre_con_start_y_nada_mas(self):
        _, trabajo = procesos.lanzar(f'start "" /b "{self.exe}" -n 60 127.0.0.1 & "{self.exe}" -n 60 127.0.0.1',
                                        shell=True, creationflags=subprocess.CREATE_NO_WINDOW, stdout=subprocess.DEVNULL)
        self.assertTrue(esperar(lambda: [procesos.nombre(p) for p in trabajo.pids()].count(self.nombre) == 2))
        self.assertNotIn(self.usuario.pid, trabajo.pids())
        trabajo.terminar()
        self.assertTrue(esperar(lambda: not trabajo.pids()))
        self.assertIsNone(self.usuario.poll())
        trabajo.cerrar()

    def test_detener_solo_cierra_lo_suyo(self):
        n = self.lanzar_fondo()
        self.assertIn(self.nombre, self.h.procesos_cmd("listar"))
        self.assertIn("solo lo que lanzó ese comando", self.h.procesos_cmd("detener", n))
        self.assertTrue(esperar(lambda: not self.h.propios()))
        self.assertIsNone(self.usuario.poll())

    def test_taskkill_por_nombre_cierra_solo_los_del_agente(self):
        self.lanzar_fondo()
        r = self.h.ejecutar(f"taskkill /F /IM {self.nombre} 2>nul")
        self.assertIn("Cerré 1 proceso(s) que lanzó el agente", r)
        self.assertTrue(esperar(lambda: self.nombre not in self.h.propios().values()))
        self.assertIsNone(self.usuario.poll())
        r = self.h.ejecutar(f"taskkill /F /IM {self.nombre}")  # ya no queda ninguno suyo: el del usuario no se toca
        self.assertIn("BLOQUEADO: no hay ningún programa con ese nombre lanzado por el agente", r)
        self.assertIsNone(self.usuario.poll())

    def test_por_numero_los_suyos_si_y_los_del_usuario_no(self):
        self.lanzar_fondo()
        suyo = next(p for p, nombre in self.h.propios().items() if nombre == self.nombre)
        self.assertIn("Cerré 1 proceso(s)", self.h.ejecutar(f"taskkill /F /PID {suyo}"))
        self.assertIn("BLOQUEADO: ese proceso no lo lanzó el agente", self.h.ejecutar(f"taskkill /F /PID {self.usuario.pid}"))
        self.assertIsNone(self.usuario.poll())

    def test_lo_que_deja_abierto_un_comando_queda_bajo_su_control(self):
        r = self.h.ejecutar(f'start "" /b "{self.exe}" -n 60 127.0.0.1 >nul', segundos=10)
        self.assertIn("Sigue abierto lo que lanzó", r)
        self.assertIn(self.nombre, r)
        n = max(self.h.procesos)
        self.assertIn(f"#{n} vivo", self.h.procesos_cmd("listar"))
        self.h.procesos_cmd("detener", n)
        self.assertTrue(esperar(lambda: not self.h.propios()))
        self.assertIsNone(self.usuario.poll())

    def test_si_el_motor_muere_mueren_sus_procesos_y_no_los_del_usuario(self):
        """Aura ya no usa taskkill /T al detener: cierra el motor y Windows cierra sus trabajos."""
        guion = self.dir / "motor.py"
        comando = f'"{self.exe}" -n 60 127.0.0.1'
        guion.write_text(
            "import sys, time\n"
            f"sys.path.insert(0, {str(Path(__file__).resolve().parent.parent)!r})\n"
            "from agente.herramientas import Herramientas\n"
            f"h = Herramientas({str(self.dir)!r})\n"
            f"h.ejecutar({comando!r}, fondo=True)\n"
            "time.sleep(1)\n"
            "print(*h.propios(), flush=True)\n"
            "time.sleep(60)\n", encoding="utf-8")
        motor = subprocess.Popen([sys.executable, str(guion)], stdout=subprocess.PIPE, text=True,
                                 creationflags=subprocess.CREATE_NO_WINDOW)
        pids = [int(x) for x in motor.stdout.readline().split()]
        self.assertTrue(pids)
        motor.kill()  # como Aura al pulsar Detener: TerminateProcess del motor, sin /T
        motor.wait()
        self.assertTrue(esperar(lambda: not any(vivo(p) for p in pids)))
        self.assertIsNone(self.usuario.poll())


if __name__ == "__main__":
    unittest.main()
