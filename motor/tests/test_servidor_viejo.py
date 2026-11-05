"""Dos fallos de la segunda misión de la peluquería (Aura 4.12.3 instalada):
- el servidor de la parte anterior seguía en el puerto 5000 con el código viejo; Windows dejó escuchar también al nuevo
  y las peticiones iban al viejo: la misión reinició el nuevo 8 veces sin ver su cambio;
- los «python -c» de varias líneas: cmd ejecuta solo la primera línea y da exit=0 sin salida."""
import shutil
import socket
import sys
import tempfile
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import procesos
from agente.herramientas import Herramientas


def puerto_libre() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class ServidorViejo(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_servidor_"))
        self.h = Herramientas(self.dir, max_chars=3000, timeout=30)
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)
        self.addCleanup(self.h.detener_todos)

    def test_varias_lineas_se_rechazan(self):
        r = self.h.ejecutar('python -c "\nprint(1)\nprint(2)\n"')
        self.assertTrue(r.startswith("RECHAZADO"))
        self.assertIn("experimentar", r)
        self.assertTrue(self.h.ejecutar('python -c "\nprint(1)"', fondo=True).startswith("RECHAZADO"))
        self.assertIn("hola", self.h.ejecutar("echo hola\n"))  # un salto al final no es otra línea

    @unittest.skipUnless(procesos.EN_WINDOWS, "dos servidores en el mismo puerto: comportamiento de Windows")
    def test_servidor_nuevo_en_el_puerto_del_viejo(self):
        puerto = puerto_libre()
        servidor = f'"{sys.executable}" -m http.server {puerto} --bind 127.0.0.1'
        self.assertIn("en ejecución", self.h.ejecutar(servidor, fondo=True))
        self.assertIn(puerto, procesos.escuchando())
        r = self.h.ejecutar(servidor, fondo=True)
        if "en ejecución" not in r:
            self.skipTest("este Python no deja escuchar dos veces en el mismo puerto")
        self.assertIn("Tu proceso #1", r)
        self.assertIn(f"puerto {puerto}", r)
        time.sleep(1)
        self.assertFalse(self.h._vivo(1))
        self.assertTrue(self.h._vivo(2))
        self.assertLessEqual(procesos.escuchando()[puerto], set(self.h.procesos[2]["trabajo"].pids()))

    @unittest.skipUnless(procesos.EN_WINDOWS, "netstat de Windows")
    def test_servidor_en_otro_puerto_no_se_toca(self):
        uno, dos = puerto_libre(), puerto_libre()
        self.h.ejecutar(f'"{sys.executable}" -m http.server {uno} --bind 127.0.0.1', fondo=True)
        r = self.h.ejecutar(f'"{sys.executable}" -m http.server {dos} --bind 127.0.0.1', fondo=True)
        self.assertNotIn("Tu proceso #1", r)
        self.assertTrue(self.h._vivo(1))
        self.assertNotIn("Tu proceso", self.h.procesos_cmd("ver", 1))  # ver el viejo no cierra el nuevo


if __name__ == "__main__":
    unittest.main()
