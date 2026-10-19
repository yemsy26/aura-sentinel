"""Ids del DOM que busca el JS/TS y no existen (sin modelo): python -m unittest tests.test_ids_dom -v"""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from agente.verificacion import Informe, ids_dom, verificar

# Aura-Monitor (Tauri 2, r490): main.ts busca "cpu-percent" y el index.html tiene "cpu-usage".
INDEX = """<!doctype html>
<html><body>
  <div class="card"><span id="cpu-usage">--</span></div>
  <span id="ram-total"></span><span id="ram-used"></span>
  <p id="status"><b id="status-text"></b></p>
  <script type="module" src="/src/main.ts"></script>
</body></html>
"""
MAIN_TS = """import { invoke } from "@tauri-apps/api/core";

async function updateStats() {
  const s: any = await invoke("get_system_stats");
  document.getElementById("cpu-percent")!.innerHTML = `${s.cpu} %`;
  document.getElementById("ram-total")!.innerHTML = `${s.total}`;
  document.querySelector<HTMLElement>("#ram-used")!.textContent = `${s.used}`;
  document.getElementById("status-text")!.textContent = "OK";
}
"""


class IdsDom(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_ids_dom_"))

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def escribir(self, ruta: str, texto: str) -> None:
        p = self.dir / ruta
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(texto, encoding="utf-8")

    def errores(self) -> list[str]:
        inf = Informe()
        ids_dom(self.dir, self.dir, inf)
        return inf.errores

    def test_el_caso_aura_monitor_da_error_con_archivo_linea_y_parecido(self):
        self.escribir("index.html", INDEX)
        self.escribir("src/main.ts", MAIN_TS)
        informe, errores = verificar(self.dir, sys.executable)
        self.assertGreaterEqual(errores, 1, informe)
        self.assertIn(f"{Path('src/main.ts')}:5 busca el elemento id='cpu-percent'", informe)
        self.assertIn("¿Querías 'cpu-usage', que sí existe?", informe)
        self.assertEqual(len(self.errores()), 1)  # ram-total, ram-used (querySelector<T>) y status-text existen

    def test_ids_que_existen_no_dan_error(self):
        self.escribir("index.html", '<canvas id=c></canvas><ul id="lista"></ul><b id="marcador"></b>'
                                    '<script src="app.js"></script>'
                                    '<script>document.getElementById("marcador").textContent = "0";</script>')
        self.escribir("app.js", 'const ctx = document.getElementById("c").getContext("2d");\n'
                                "document.querySelectorAll('#lista li').forEach(li => li.remove());\n"
                                'document.querySelector("#lista > li.activo");\n')
        self.assertEqual(self.errores(), [])

    def test_querySelector_y_script_en_linea_que_faltan(self):
        self.escribir("index.html", '<div id="marcador"></div>\n<script>\n  const m = document.querySelector("#marcadr");'
                                    "\n</script>\n")
        self.escribir("a.js", 'document.querySelectorAll("#fantasma .x");\n')
        self.escribir("b.jsx", 'export const f = () => document.getElementById("fantasma");\n')
        errores = self.errores()
        self.assertEqual(len(errores), 2, errores)
        self.assertIn("a.js:1 busca el elemento id='fantasma'", errores[0])
        self.assertIn("También en b.jsx:1.", errores[0])
        self.assertIn("index.html:3 busca el elemento id='marcadr'", errores[1])
        self.assertIn("¿Querías 'marcador'", errores[1])

    def test_ids_creados_por_script_no_dan_error(self):
        self.escribir("index.html", '<div id="app"></div><script type="module" src="/src/main.tsx"></script>')
        self.escribir("src/main.tsx", (
            'const a = document.createElement("div"); a.id = "nuevo"; document.body.append(a);\n'
            'const b = document.createElement("p"); b.setAttribute("id", "otro");\n'
            "document.getElementById(\"app\")!.innerHTML = '<span id=\"tercero\"></span>' + \"<i id=\\\"cuarto\\\"></i>\";\n"
            'export const Vista = () => <section id="quinto"><b id={"sexto"} /></section>;\n'
            'React.createElement("div", { id: "septimo" });\n'
            + "".join(f'document.getElementById("{i}");\n'
                      for i in ("nuevo", "otro", "tercero", "cuarto", "quinto", "sexto", "septimo"))))
        self.assertEqual(self.errores(), [])

    def test_plantillas_variables_y_comentarios_se_ignoran(self):
        self.escribir("index.html", '<div id="cpu-0"></div><script src="app.ts"></script>')
        self.escribir("app.ts", (
            "for (let n = 0; n < 4; n++) document.getElementById(`cpu-${n}`);\n"
            "const nombre = 'x'; document.getElementById(nombre);\n"
            'document.getElementById("cpu-" + 1);\n'
            "document.querySelector(`#fila-${nombre}`);\n"
            'document.querySelector("#" + nombre);\n'
            "document.querySelector(`#lista ${nombre}`);\n"
            '// document.getElementById("viejo");\n'
            'pintar(); // antes: document.getElementById("antiguo")\n'
            '/* document.querySelector("#otro") */\n'))
        self.assertEqual(self.errores(), [])

    def test_no_mira_node_modules_dist_ni_gen_de_tauri(self):
        self.escribir("index.html", '<div id="app"></div>')
        self.escribir("node_modules/lib/index.js", 'document.getElementById("de-la-libreria");\n')
        self.escribir("dist/assets/index.js", 'document.getElementById("compilado");\n')
        self.escribir("src-tauri/tauri.conf.json", "{}")
        self.escribir("src-tauri/gen/schemas/x.js", 'document.getElementById("generado");\n')
        self.escribir("vendor/chart.min.js", 'document.getElementById("minificado");\n')
        self.assertEqual(self.errores(), [])
        # un id que solo está en dist/index.html (compilado, se regenera) no cuenta como existente
        self.escribir("dist/index.html", '<div id="solo-en-dist"></div>')
        self.escribir("src/main.ts", 'document.getElementById("solo-en-dist");\n')
        self.assertEqual(len(self.errores()), 1)

    def test_sin_html_no_hay_dom_con_el_que_comparar(self):
        self.escribir("lib.ts", 'export const raiz = () => document.getElementById("raiz");\n')
        self.assertEqual(self.errores(), [])


if __name__ == "__main__":
    unittest.main()
