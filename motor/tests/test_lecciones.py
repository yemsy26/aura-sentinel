"""Pruebas de las lecciones (lecciones.py): sobreviven las que se usan y cada tarea recibe solo las que tienen que ver
con ella, en su mensaje y no en el prompt de sistema. Sin modelo: el de las tareas es un guion."""
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from typing import ClassVar
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import config, lecciones, prompts

T = 1_800_000_000
PYGAME = "En pygame-ce no uses emojis en los textos: la fuente por defecto no los tiene y salen cuadrados."
TAURI = "En Tauri 2 invoke se importa de @tauri-apps/api/core; @tauri-apps/api/tauri ya no existe."
NPM = "npm install en una carpeta sin package.json cambia los paquetes de la carpeta superior: crea antes package.json."
CMD = 'En cmd, start necesita un título vacío antes de una ruta entre comillas: start "" "C:\\ruta\\app.exe".'
VENV = "pip install con el Python del sistema falla por permisos: instala en el .venv del proyecto."


class Temporal(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_lecciones_"))
        for nombre, valor in (("ARCHIVO", self.dir / "lecciones.md"), ("USO", self.dir / "lecciones_uso.json")):
            parche = mock.patch.object(lecciones, nombre, valor)
            parche.start()
            self.addCleanup(parche.stop)

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def uso(self) -> dict:
        return json.loads(lecciones.USO.read_text(encoding="utf-8"))


class Olvido(Temporal):
    def test_se_olvida_la_que_lleva_mas_sin_usarse_no_la_mas_antigua(self):
        with mock.patch.object(lecciones, "MAX", 3):
            for i, leccion in enumerate((PYGAME, TAURI, NPM), 1):
                self.assertIn("guardada", lecciones.guardar(leccion, T + i))
            self.assertIn(PYGAME, lecciones.para_tarea("Crea un juego de la serpiente", ahora=T + 4))
            lecciones.guardar(CMD, T + 5)
            self.assertEqual([PYGAME, NPM, CMD], lecciones.leer())  # TAURI: la que llevaba más sin usarse
            self.assertIn("ya estaba", lecciones.guardar(NPM, T + 6))  # se repitió: cuenta como uso
            lecciones.guardar(VENV, T + 7)
            self.assertEqual([NPM, CMD, VENV], lecciones.leer())
        self.assertEqual({"usada": T + 6, "usos": 1}, self.uso()[lecciones._clave(NPM)])
        self.assertNotIn(lecciones._clave(TAURI), self.uso())  # el uso de las olvidadas no se queda
        self.assertTrue(lecciones.ARCHIVO.read_text(encoding="utf-8").startswith("# Lecciones aprendidas\n- "))

    def test_las_de_antes_sin_registro_se_olvidan_en_su_orden_salvo_si_se_usan(self):
        lecciones.ARCHIVO.write_text(f"# Lecciones aprendidas\n- {PYGAME}\n- {TAURI}\n\n* {NPM}\n## otra cosa\n",
                                     encoding="utf-8")
        self.assertEqual([PYGAME, TAURI, NPM], lecciones.leer())  # viñetas de cualquier tipo, sin títulos
        with mock.patch.object(lecciones, "MAX", 3):
            lecciones.para_tarea("Haz un juego de naves", ahora=T)
            lecciones.guardar(CMD, T + 1)
        self.assertEqual([PYGAME, NPM, CMD], lecciones.leer())

    def test_leccion_demasiado_corta(self):
        self.assertIn("concreta", lecciones.guardar("usa venv"))
        self.assertFalse(lecciones.ARCHIVO.exists())


class Relacionadas(Temporal):
    def setUp(self):
        super().setUp()
        for i, leccion in enumerate((PYGAME, TAURI, NPM, CMD, VENV)):
            lecciones.guardar(leccion, T + i)

    def elegidas(self, tarea: str, raiz: Path | None = None) -> list[str]:
        texto = lecciones.para_tarea(tarea, raiz, T + 100)
        return [l for l in (PYGAME, TAURI, NPM, CMD, VENV) if l in texto]

    def test_por_el_tema_que_nombra_la_tarea(self):
        self.assertEqual([PYGAME], self.elegidas("Crea un juego de la serpiente con puntuación"))  # juego → pygame
        self.assertEqual([CMD], self.elegidas("Escribe un .bat que arranque el servidor"))  # cmd → bat
        self.assertEqual({"usada": T + 100, "usos": 1}, self.uso()[lecciones._clave(CMD)])

    def test_por_los_archivos_del_proyecto(self):
        proyecto = self.dir / "app"
        (proyecto / "src-tauri").mkdir(parents=True)
        (proyecto / "src-tauri" / "tauri.conf.json").write_text("{}", encoding="utf-8")
        (proyecto / "package.json").write_text("{}", encoding="utf-8")
        (proyecto / "node_modules" / "x").mkdir(parents=True)
        (proyecto / "node_modules" / "x" / "main.py").write_text("", encoding="utf-8")  # dependencias: no cuentan
        self.assertEqual({"tauri", "node"}, lecciones.temas_proyecto(proyecto))
        self.assertEqual([TAURI, NPM], self.elegidas("Arregla el botón de guardar, que no responde", proyecto))

    def test_por_dos_palabras_con_significado(self):
        self.assertEqual([VENV], self.elegidas("Instala las dependencias con pip en el sistema"))

    def test_ninguna_si_no_tienen_que_ver(self):
        self.assertEqual("", lecciones.para_tarea("Crea un archivo de notas", ahora=T + 100))
        self.assertTrue(all(v["usos"] == 0 for v in self.uso().values()))  # nada marcado como usado

    def test_como_mucho_max_tarea(self):
        for i in range(12):
            lecciones.guardar(f"En pygame, el sprite {i} necesita convert_alpha() tras cargar la imagen {i}.", T + 10 + i)
        texto = lecciones.para_tarea("Haz un juego de plataformas", ahora=T + 100)
        self.assertEqual(lecciones.MAX_TAREA, texto.count("\n- "))
        self.assertLessEqual(len(texto), lecciones.MAX_CHARS_TAREA + 200)


class LLMFin:
    """Modelo de guion: termina enseguida y guarda lo que recibió."""
    recibidos: ClassVar[list] = []

    def __init__(self, base, modelo, contexto, cfg):
        self.contexto = contexto

    def chat(self, mensajes, herramientas, max_tokens, razonar, al_progreso=None):
        LLMFin.recibidos.append([dict(m) for m in mensajes])
        return {"content": "Listo.", "tool_calls": [], "finish_reason": "stop", "usage": {}}


class EnLaTarea(Temporal):
    def setUp(self):
        super().setUp()
        lecciones.guardar(PYGAME, T)
        lecciones.guardar(TAURI, T)
        falso = lambda cfg: ("http://falso", "falso", 32768)
        for p in (mock.patch("agente.bucle.cliente.detectar", falso), mock.patch("agente.bucle.cliente.LLM", LLMFin),
                  mock.patch("agente.bucle.entorno", lambda: "prueba")):
            p.start()
            self.addCleanup(p.stop)

    def test_van_en_el_mensaje_de_la_tarea_y_no_en_el_sistema(self):
        from agente.bucle import Agente
        LLMFin.recibidos = []
        a = Agente(config.cargar(), self.dir / "p", interactivo=False)
        self.assertNotIn("LECCIONES", a.ctx.mensajes[0]["content"])
        self.assertNotIn("LECCIONES", prompts.sistema(self.dir, 32768, "prueba"))
        a.tarea("Haz un juego de naves", max_pasos=1)
        mensaje = LLMFin.recibidos[0][1]["content"]
        self.assertTrue(mensaje.startswith("Haz un juego de naves"))
        self.assertIn(f"LECCIONES DE TAREAS ANTERIORES (tienen que ver con esta; evita repetir estos errores):\n- {PYGAME}",
                      mensaje)
        self.assertNotIn(TAURI, mensaje)
        LLMFin.recibidos = []
        a.tarea("sigue", max_pasos=1)  # continuar no repite las lecciones
        self.assertNotIn("LECCIONES", LLMFin.recibidos[0][-1]["content"])


if __name__ == "__main__":
    unittest.main()
