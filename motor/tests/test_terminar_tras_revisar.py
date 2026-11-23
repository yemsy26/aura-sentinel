"""m4172/C4: revisar() aprueba el código y la tarea sigue sin llamar a terminar (las dos veterinarias de m4172 llegaron
al tope de pasos así). Al cabo de unos pasos, el motor se lo recuerda; no bloquea nada, _puede_terminar sigue siendo
la puerta real."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_agente import llamada, sin_lecciones_reales
from test_terminar_revision import APROBADO, CAMBIOS, LLMLento

from agente import config

setUpModule = sin_lecciones_reales


def leer(desde: int) -> dict:
    return llamada("leer", ruta="main.py", desde=desde, lineas=1)


class TrasRevisarAprobadoSinTerminar(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_avisorev_"))
        falso = lambda cfg: ("http://falso", "falso", 32768)
        self.parches = [mock.patch("agente.bucle.cliente.detectar", falso), mock.patch("agente.bucle.cliente.LLM", LLMLento),
                        mock.patch("agente.bucle.entorno", lambda: "prueba")]
        for p in self.parches:
            p.start()

    def tearDown(self):
        for p in self.parches:
            p.stop()
        shutil.rmtree(self.dir, ignore_errors=True)

    def resultados(self, respuestas: list) -> list[str]:
        """Ejecuta una tarea con el guion; devuelve el resultado de cada llamada del modelo, en orden."""
        from agente.bucle import Agente
        LLMLento.respuestas = respuestas
        a = Agente(config.cargar(), self.dir, interactivo=False)
        a.tarea("haz un programa que salude")
        self.assertEqual(LLMLento.respuestas, [])  # se consumió el guion entero
        return [m["content"] for m in a.ctx.mensajes if m["role"] == "tool"]

    def test_al_octavo_paso_sin_terminar_avisa(self):
        r = self.resultados([llamada("escribir", ruta="main.py", contenido="print('hola')\n"),
                             llamada("revisar"), APROBADO,
                             *[leer(i) for i in range(1, 9)],
                             llamada("terminar", resumen="Saluda.")])
        fillers = r[2:10]
        self.assertEqual(len(fillers), 8)
        self.assertFalse(any("ya aprobó el código" in t for t in fillers[:7]), fillers[:7])
        self.assertIn("ya aprobó el código", fillers[7])
        self.assertEqual(r[-1], "Tarea terminada.")

    def test_antes_del_octavo_no_avisa(self):
        r = self.resultados([llamada("escribir", ruta="main.py", contenido="print('hola')\n"),
                             llamada("revisar"), APROBADO,
                             *[leer(i) for i in range(1, 4)],
                             llamada("terminar", resumen="Saluda.")])
        self.assertFalse(any("ya aprobó el código" in t for t in r))
        self.assertEqual(r[-1], "Tarea terminada.")

    def test_una_revision_que_pide_cambios_apaga_el_aviso(self):
        r = self.resultados([llamada("escribir", ruta="main.py", contenido="print('adiós')\n"),
                             llamada("revisar"), APROBADO,
                             *[leer(i) for i in range(1, 4)],
                             llamada("revisar"), CAMBIOS,
                             *[leer(i) for i in range(10, 20)],
                             llamada("editar", ruta="main.py", buscar="adiós", reemplazar="hola"),
                             llamada("terminar", resumen="Saluda.")])  # con 2 revisiones manuales ya hechas, terminar no pide una 3ª
        self.assertFalse(any("ya aprobó el código" in t for t in r))
        self.assertEqual(r[-1], "Tarea terminada.")

    def test_sin_pasar_por_revisar_a_mano_no_avisa(self):
        """terminar() ya revisa por su cuenta: ese camino no necesita el recordatorio (no pierde pasos de por medio)."""
        r = self.resultados([llamada("escribir", ruta="main.py", contenido="print('hola')\n"),
                             *[leer(i) for i in range(1, 12)],
                             llamada("terminar", resumen="Saluda."), APROBADO])
        self.assertFalse(any("ya aprobó el código" in t for t in r))
        self.assertEqual(r[-1], "Tarea terminada.")


if __name__ == "__main__":
    unittest.main()
