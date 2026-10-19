"""El revisor recibe las prácticas actuales de las guías de los temas del proyecto (solo las frases que vienen al caso):
un requisito que nombra una API que ya no existe no se marca como CRITICO, y uno incumplido de verdad sí."""
import json
import os
import shutil
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import revision

TAREA = ("App de escritorio con Tauri. Usa la API @tauri-apps/api/invoke para llamar al comando get_system_stats cada "
         "1000 milisegundos y muestra los valores en el HTML.")
MAIN_TS = ('import { invoke } from "@tauri-apps/api/core";\n\n'
           "async function actualizar() {\n"
           '  const s = await invoke<{ cpu: number }>("get_system_stats");\n'
           '  document.getElementById("cpu")!.textContent = `${s.cpu}`;\n'
           "}\n"
           "actualizar();\n"
           "setInterval(actualizar, 1000);\n")
MAIN_RS = ("#[derive(serde::Serialize)]\nstruct Stats { cpu: f32 }\n\n#[tauri::command]\nfn get_system_stats() -> Stats {\n"
           "    Stats { cpu: 1.0 }\n}\n\nfn main() {\n    tauri::Builder::default()\n"
           "        .invoke_handler(tauri::generate_handler![get_system_stats])\n"
           '        .run(tauri::generate_context!())\n        .expect("error al iniciar la app");\n}\n')


class RevisorFalso:
    """Modelo de guion que aplica las reglas del prompt al pie de la letra: lo que las PRÁCTICAS ACTUALES dicen que no
    existe es una desviación justificada; un requisito que no está en el código es CRITICO."""
    contexto = 32768

    def __init__(self):
        self.prompts = []

    def chat(self, mensajes, *_):
        sistema, usuario = mensajes[0]["content"], mensajes[1]["content"]
        self.prompts.append(usuario)
        codigo = usuario.split("CÓDIGO:\n", 1)[1]
        practicas = usuario.split("PRÁCTICAS ACTUALES", 1)[1].split("\n\n", 1)[0] if "PRÁCTICAS ACTUALES" in usuario else ""
        requisitos, problemas = [], []
        if "@tauri-apps/api/invoke" not in codigo:
            if "desviación justificada" in sistema and "`@tauri-apps/api/invoke` no existe" in practicas:
                requisitos.append("- usar @tauri-apps/api/invoke -> CUMPLE (desviación justificada: en Tauri 2 es api/core)")
            else:
                requisitos.append("- usar @tauri-apps/api/invoke -> NO CUMPLE: importa de @tauri-apps/api/core")
                problemas.append("- [CRITICO] src/main.ts:1 - no usa @tauri-apps/api/invoke - cambia el import")
        if "setInterval" in codigo:
            requisitos.append("- cada 1000 ms -> CUMPLE (src/main.ts:8)")
        else:
            requisitos.append("- cada 1000 ms -> NO CUMPLE: nada vuelve a llamar al comando")
            problemas.append("- [CRITICO] src/main.ts:7 - los valores no se refrescan - añade setInterval(actualizar, 1000)")
        veredicto = "CAMBIOS NECESARIOS" if problemas else "APROBADO"
        return {"content": "REQUISITOS:\n" + "\n".join(requisitos) + "\nPROBLEMAS:\n" + ("\n".join(problemas) or "- ninguno")
                           + f"\nVEREDICTO: {veredicto}"}


class RevisorConPracticas(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_revisor_"))

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def escribir(self, archivos: dict) -> None:
        for ruta, texto in archivos.items():
            p = self.dir / ruta
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(texto, encoding="utf-8")

    def proyecto_tauri(self, main_ts: str = MAIN_TS, main_rs: str = MAIN_RS) -> None:
        self.escribir({
            "package.json": json.dumps({"name": "monitor", "dependencies": {"@tauri-apps/api": "^2"}}),
            "index.html": '<!doctype html>\n<p id="cpu"></p>\n<script type="module" src="/src/main.ts"></script>\n',
            "src/main.ts": main_ts,
            "src-tauri/Cargo.toml": '[package]\nname = "monitor"\n\n[dependencies]\ntauri = "2"\nserde = { version = "1" }\n',
            "src-tauri/src/main.rs": main_rs,
            "src-tauri/capabilities/default.json": '{"identifier": "default", "windows": ["main"], "permissions": ["core:default"]}',
            "src-tauri/tauri.conf.json": json.dumps({"identifier": "com.prueba.monitor", "app": {"windows": [{"title": "M"}]}}),
        })

    def test_api_inexistente_en_la_peticion_no_es_critico(self):
        self.proyecto_tauri()
        llm = RevisorFalso()
        texto, ok = revision.revisar(llm, self.dir, TAREA, "VERIFICACIÓN (web, rust, tauri, node): 0 ERRORES")
        self.assertTrue(ok, texto)
        self.assertNotIn("[CRITICO]", texto)
        self.assertIn("desviación justificada", texto)
        prompt = llm.prompts[0]
        self.assertIn("`@tauri-apps/api/invoke` no existe", prompt)
        self.assertIn('import { invoke, isTauri } from "@tauri-apps/api/core"', prompt)  # la forma correcta, al lado
        self.assertIn("`generate_handler!` se pueden invocar sin más", prompt)
        self.assertNotIn("tauri init", prompt)  # procedimiento, no algo que el revisor juzgue
        self.assertNotIn("Nada de tablas", prompt)

    def test_requisito_real_incumplido_sigue_siendo_critico(self):
        self.proyecto_tauri(main_ts=MAIN_TS.replace("setInterval(actualizar, 1000);\n", ""))
        texto, ok = revision.revisar(RevisorFalso(), self.dir, TAREA, "")
        self.assertFalse(ok)
        self.assertIn("[CRITICO] src/main.ts:7", texto)
        self.assertIn("setInterval", texto)
        self.assertNotIn("[CRITICO] src/main.ts:1", texto)  # la importación sigue sin ser un problema

    def test_sin_las_practicas_el_mismo_revisor_la_marcaria_critica(self):
        self.proyecto_tauri()
        with mock.patch.object(revision, "practicas", return_value=""):
            texto, ok = revision.revisar(RevisorFalso(), self.dir, TAREA, "")
        self.assertFalse(ok)
        self.assertIn("[CRITICO] src/main.ts:1", texto)

    def test_los_esquemas_generados_no_tapan_la_configuracion(self):
        self.proyecto_tauri()
        time.sleep(0.05)  # gen/ es lo más reciente: antes entraba primero y llenaba el cupo
        esquema = json.dumps({"definitions": {f"permiso_{i}": {"description": "x" * 200} for i in range(800)}})
        self.escribir({"src-tauri/gen/schemas/desktop-schema.json": esquema,
                       "src-tauri/icons/android/values/ic_launcher_background.xml": "<resources/>"})
        llm = RevisorFalso()
        revision.revisar(llm, self.dir, TAREA, "")
        prompt = llm.prompts[0]
        self.assertIn(f"=== src-tauri{os.sep}tauri.conf.json ===", prompt)
        self.assertIn(f"=== src-tauri{os.sep}capabilities{os.sep}default.json ===", prompt)
        self.assertNotIn("desktop-schema.json", prompt)
        self.assertNotIn("ic_launcher", prompt)
        self.assertLess(len(prompt), 8000)

    def test_temas_por_los_archivos(self):
        self.proyecto_tauri()
        fuentes = revision._fuentes(self.dir)
        self.assertEqual(revision.temas(fuentes, []), ["tauri", "rust", "node", "web"])
        juego = ["import pygame\n\npygame.init()\n"]
        self.assertEqual(revision.temas([Path("juego.py"), Path("requirements.txt")], juego), ["pygame", "python"])
        self.assertEqual(revision.temas([Path("notas.md")], ["texto"]), [])

    def test_practicas_solo_lo_que_viene_al_caso_y_con_tope(self):
        self.assertEqual(revision.practicas(["tauri"], "haz una app", "fn main() {}"), "")
        self.assertIn("pygame-ce", revision.practicas(["pygame", "python"], "un juego", "pygame-ce>=2.5\n"))
        todas = sorted(p.stem for p in revision.DIR_GUIAS.glob("*.md"))
        corpus = "\n".join((revision.DIR_GUIAS / f"{t}.md").read_text(encoding="utf-8") for t in todas)
        self.assertLessEqual(len(revision.practicas(todas, corpus, corpus)), revision.MAX_PRACTICAS)

    def test_las_practicas_salen_del_cupo_del_codigo(self):
        relleno = "".join(f"// línea de relleno {i}\n" for i in range(3000))
        self.proyecto_tauri(main_rs=MAIN_RS + relleno)

        class Pequeño(RevisorFalso):
            contexto = 8192

        llm = Pequeño()
        revision.revisar(llm, self.dir, TAREA, "")
        prompt = llm.prompts[0]
        guia = prompt.split("PRÁCTICAS ACTUALES", 1)[1].split("\n\n", 1)[0]
        self.assertTrue(guia.strip())
        cupo = int(8192 * 3.0 * 0.5)
        self.assertLessEqual(len(prompt.split("CÓDIGO:\n", 1)[1]) + len(guia), cupo + 100)


if __name__ == "__main__":
    unittest.main()
