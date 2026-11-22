"""H18: leer con el principio ya leído y sin `desde` sigue donde se quedó (en vez de explicar), y compactar el contexto olvida lo leído.
Ferretería (23): el modelo repitió leer(lineas=340) cuatro veces sobre un archivo de 501 líneas y el bucle cortó la misión a la quinta."""
import shutil
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from agente.bucle import Agente
from agente.herramientas import Herramientas


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="test_leer_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)
        self.h = Herramientas(self.dir, max_chars=60000, timeout=30)
        self.addCleanup(self.h.detener_todos)
        self.h.escribir("app.py", "\n".join(f"linea {i}" for i in range(1, 502)))


class SigueDondeSeQuedo(Base):
    def test_la_misma_llamada_repetida_va_avanzando_hasta_el_final(self):
        uno = self.h.leer("app.py", lineas=340)
        self.assertIn("340| linea 340", uno)
        self.assertNotIn("341|", uno)
        dos = self.h.leer("app.py", lineas=340)
        self.assertIn("341| linea 341", dos)
        self.assertIn("501| linea 501", dos)
        self.assertNotIn("\n1| linea 1\n", dos)
        self.assertIn("Ya habías leído hasta la línea 340", dos)
        for _ in range(2):  # una vez entero ya no hay nada que servir
            tres = self.h.leer("app.py", lineas=340)
            self.assertNotIn("| linea", tres)
            self.assertIn("ya lo tienes entero (501 líneas)", tres)
            self.assertIn("desde=1", tres)

    def test_lineas_como_numero_de_linea_da_lo_que_falta_y_no_un_callejon(self):
        """Antes (test_peluqueria): leer(lineas=86) tras leer 1-50 devolvía una explicación sin contenido y el modelo repetía la llamada."""
        self.assertIn("50| linea 50", self.h.leer("app.py", desde=1, lineas=50))
        r = self.h.leer("app.py", lineas=86)
        self.assertIn("86| linea 86", r)  # la línea que quería ver
        self.assertIn("51| linea 51", r)  # empieza donde lo dejó
        self.assertNotIn("\n1| linea 1\n", r)
        self.assertIn("Ya habías leído hasta la línea 50", r)
        self.assertIn("'lineas' es CUÁNTAS líneas leer", r)
        self.assertIn("[líneas 51-136 de 501; usa desde=137 para seguir]", r)

    def test_la_nota_es_una_sola_linea_y_dice_que_hacer(self):
        self.h.leer("app.py", desde=1, lineas=10)
        nota = self.h.leer("app.py", lineas=10).splitlines()[1]
        self.assertTrue(nota.startswith("[Ya habías leído hasta la línea 10:") and nota.endswith("]"))
        self.assertIn("desde=N", nota)
        self.assertIn("desde=1", nota)

    def test_seguir_con_desde_explicito_alarga_lo_leido(self):
        self.h.leer("app.py", desde=1, lineas=100)
        self.h.leer("app.py", desde=101, lineas=100)
        r = self.h.leer("app.py", lineas=50)
        self.assertIn("201| linea 201", r)
        self.assertIn("Ya habías leído hasta la línea 200", r)

    def test_un_salto_no_alarga_lo_leido(self):
        self.h.leer("app.py", desde=1, lineas=100)
        self.h.leer("app.py", desde=300, lineas=10)
        r = self.h.leer("app.py", lineas=50)
        self.assertIn("101| linea 101", r)
        self.assertIn("Ya habías leído hasta la línea 100", r)

    def test_pedir_el_principio_a_proposito_vuelve_a_empezar(self):
        self.h.leer("app.py", lineas=340)
        self.assertIn("1| linea 1\n", self.h.leer("app.py", desde=1, lineas=5))
        r = self.h.leer("app.py", lineas=10)
        self.assertIn("6| linea 6", r)
        self.assertIn("Ya habías leído hasta la línea 5", r)


class LoQueNoCambia(Base):
    def test_archivo_cambiado_tarea_nueva_o_memoria_borrada_se_lee_desde_el_principio(self):
        self.h.leer("app.py", desde=1, lineas=50)
        self.h.escribir("app.py", "\n".join(f"otra {i}" for i in range(1, 502)))
        r = self.h.leer("app.py", lineas=86)
        self.assertIn("1| otra 1\n", r)
        self.assertIn("86| otra 86", r)
        self.assertNotIn("Ya habías leído", r)
        self.h.leer("app.py", desde=1, lineas=50)
        self.h.nueva_tarea()
        self.assertIn("1| otra 1\n", self.h.leer("app.py", lineas=10))
        self.h.leer("app.py", desde=1, lineas=50)
        self.h.leidos.clear()  # lo que hace compactar el contexto
        self.assertIn("1| otra 1\n", self.h.leer("app.py", lineas=10))

    def test_lectura_plana_con_desde_y_topes(self):
        self.assertIn("200| linea 200", self.h.leer("app.py"))
        self.assertIn("[líneas 1-200 de 501; usa desde=201 para seguir]", self.h.leer("app.py"))  # sin 'lineas' no continúa solo
        self.assertIn("100| linea 100", self.h.leer("app.py", desde=100, lineas=10))
        self.h.leidos.clear()
        r = self.h.leer("app.py", lineas=1000)
        self.assertIn("400| linea 400", r)
        self.assertNotIn("401|", r)
        self.h.leidos.clear()
        self.assertEqual(self.h.leer("app.py", lineas=-5).count("| linea"), 1)
        self.h.escribir("vacio.py", "")
        self.assertIn("(0 líneas)", self.h.leer("vacio.py", lineas=10))

    def test_otro_archivo_no_se_ve_afectado(self):
        self.h.escribir("otro.py", "\n".join(f"otra {i}" for i in range(1, 101)))
        self.h.leer("app.py", lineas=340)
        r = self.h.leer("otro.py", lineas=30)
        self.assertIn("1| otra 1\n", r)
        self.assertNotIn("Ya habías leído", r)


class CompactarOlvidaLoLeido(Base):
    def test_tras_compactar_se_puede_volver_a_leer_desde_el_principio(self):
        self.h.leer("app.py", lineas=340)
        agente = mock.Mock()  # solo lo que usa _compactar
        agente.h, agente.llm, agente.pasos_totales = self.h, object(), 44
        agente.ctx.compactar.return_value = "contexto compactado: 39887 -> 18425 tokens"
        agente._estado.return_value = "- [ ] 1. probar"
        Agente._compactar(agente)
        self.assertEqual(self.h.leidos, {})
        r = self.h.leer("app.py", lineas=340)
        self.assertIn("1| linea 1\n", r)
        self.assertIn("340| linea 340", r)
        self.assertNotIn("Ya habías leído", r)


if __name__ == "__main__":
    unittest.main()
