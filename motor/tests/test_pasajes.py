"""Pruebas de la memoria de lo investigado (pasajes.py) y de investigar() con memoria primero. Sin red: la búsqueda
y la lectura de páginas se simulan."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import pasajes
from agente.herramientas import ESQUEMAS, Herramientas, despachar

DIA = pasajes.DIA
T0 = 1_800_000_000.0
TAURI = "https://v2.tauri.app/develop/calling-rust/"
INVOKE = ("Define the command with #[tauri::command] and call it from the frontend with invoke('greet', "
          "{ name }) from @tauri-apps/api/core.")
REGISTRO = "Register every command in the builder: .invoke_handler(tauri::generate_handler![greet])."


class Temporal(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        parche = mock.patch.object(pasajes, "ARCHIVO", self.dir / "pasajes.db")
        parche.start()
        self.addCleanup(parche.stop)

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)


class Memoria(Temporal):
    def test_encuentra_lo_que_cubre_la_pregunta_y_no_lo_de_otra_api(self):
        self.assertEqual(2, pasajes.guardar("tauri 2 invoke command", [(TAURI, [INVOKE, REGISTRO])], T0))
        hallados = pasajes.buscar("How to call a Rust command from the frontend with invoke in Tauri?", T0)
        self.assertEqual([INVOKE], [p.texto for p in hallados])
        self.assertEqual(TAURI, hallados[0].url)
        self.assertEqual([], pasajes.buscar("tauri system tray icon", T0))
        self.assertEqual([], pasajes.buscar("¿cómo hacer esto?", T0))  # sin palabras con significado

    def test_repetir_la_pregunta_trae_todo_lo_leido_para_ella(self):
        pasajes.guardar("tauri 2 invoke command", [(TAURI, [INVOKE, REGISTRO])], T0)
        self.assertEqual({INVOKE, REGISTRO}, {p.texto for p in pasajes.buscar("Tauri 2: invoke command", T0)})

    def test_caduca_sin_uso_y_por_antiguedad_aunque_se_use(self):
        pasajes.guardar("tauri 2 invoke command", [(TAURI, [INVOKE])], T0)
        self.assertEqual([], pasajes.buscar("tauri invoke command", T0 + 31 * DIA))  # 30 días sin usarse
        pasajes.guardar("tauri 2 invoke command", [(TAURI, [INVOKE])], T0)
        for dia in (25, 50, 75):  # usado cada 25 días: sigue...
            hallados = pasajes.buscar("tauri invoke command", T0 + dia * DIA)
            self.assertEqual(1, len(hallados), dia)
            pasajes.usar([p.id for p in hallados], T0 + dia * DIA)
        self.assertEqual([], pasajes.buscar("tauri invoke command", T0 + 91 * DIA))  # ...hasta los 90 días

    def test_releer_una_pagina_reemplaza_lo_guardado_de_ella(self):
        pasajes.guardar("tauri invoke command", [(TAURI, [INVOKE])], T0)
        nuevo = "In Tauri 2 the invoke command is imported from @tauri-apps/api/core instead of @tauri-apps/api/tauri."
        pasajes.guardar("tauri invoke command", [(TAURI, [nuevo])], T0 + DIA)
        self.assertEqual([nuevo], [p.texto for p in pasajes.buscar("tauri invoke command", T0 + DIA)])

    def test_parrafos_largos_se_parten_por_lineas(self):
        linea = "let x = tauri::Builder::default(); // configura la ventana"
        trozos = pasajes._partir("\n".join([linea] * 60))
        self.assertGreater(len(trozos), 1)
        self.assertTrue(all(len(t) <= pasajes.MAX_CHARS for t in trozos))
        self.assertTrue(all(l == linea for t in trozos for l in t.splitlines()))

    def test_raices_como_fts5(self):
        self.assertEqual({"invok", "handl", "tauri", "conf", "json"}, pasajes.raices("invoke_handler tauri.conf.json"))
        self.assertEqual({"v2", "axum"}, pasajes.raices("Axum v2 2026 0.8 best example"))

    def test_sin_base_de_datos_no_falla(self):
        with mock.patch.object(pasajes, "ARCHIVO", self.dir):  # una carpeta: SQLite no puede abrirla
            self.assertEqual(0, pasajes.guardar("tauri invoke command", [(TAURI, [INVOKE])]))
            self.assertEqual([], pasajes.buscar("tauri invoke command"))
            pasajes.usar([1])


class Investigar(Temporal):
    def setUp(self):
        super().setUp()
        self.h = Herramientas(self.dir / "p")
        self.busquedas = []
        self.h.web_buscar = lambda consulta: (self.busquedas.append(consulta)
                                              or f"1. Calling Rust from the Frontend\n   {TAURI}\n   resumen")
        self.h.web_leer = lambda url, buscar="": f"{url}\n{INVOKE}\n\n{REGISTRO}"

    def test_memoria_primero_y_web_si_se_pide(self):
        pregunta = "tauri 2 invoke command from frontend"
        primera = despachar(self.h, "investigar", {"pregunta": pregunta})
        self.assertIn(f"### {TAURI}\n", primera)
        self.assertEqual(1, len(self.busquedas))
        segunda = despachar(self.h, "investigar", {"pregunta": "Tauri 2: call a command from the frontend with invoke"})
        self.assertEqual(1, len(self.busquedas))  # no salió a internet
        self.assertIn("De la memoria de lo ya investigado", segunda)
        self.assertIn(f"### {TAURI} (leída hoy)", segunda)
        self.assertIn(INVOKE, segunda)
        despachar(self.h, "investigar", {"pregunta": pregunta, "web": True})
        self.assertEqual(2, len(self.busquedas))
        despachar(self.h, "investigar", {"pregunta": "tauri system tray icon"})
        self.assertEqual(3, len(self.busquedas))  # otra API: a la web

    def test_lo_recortado_no_se_guarda(self):
        self.h.web_leer = lambda url, buscar="": (f"{url}\n{INVOKE}\n\n.invoke_handler(\n[... 900 caracteres "
                                                  "omitidos ...]\ntauri::generate_handler![greet])")
        self.h.investigar("tauri invoke command")
        self.assertEqual([INVOKE], [p.texto for p in pasajes.buscar("tauri invoke command")])

    def test_esquema_con_web(self):
        esquema = next(e for e in ESQUEMAS if e["function"]["name"] == "investigar")["function"]["parameters"]
        self.assertEqual({"type": "boolean"}, esquema["properties"]["web"])
        self.assertEqual(["pregunta"], esquema["required"])


if __name__ == "__main__":
    unittest.main()
