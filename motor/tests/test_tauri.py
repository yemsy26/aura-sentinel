"""Proyectos Tauri 2: lo que compila pero no funciona (sin ventana, sin build.rs o icono, invoke sin registrar, API de
Tauri 1) y la guía."""
import json
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import importlib.util

from agente import ecosistemas, verificacion
from agente.conocimiento import guia
from agente.verificacion import Informe

HAY_NAVEGADOR = importlib.util.find_spec("playwright") is not None

MAIN_RS = """#[tauri::command]
fn get_system_stats() -> String { String::new() }

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![get_system_stats])
        .run(tauri::generate_context!())
        .expect("error");
}
"""
CONF = {"productName": "Aura-Monitor", "version": "0.1.0", "identifier": "com.prueba.monitor",
        "build": {"frontendDist": "../dist", "beforeBuildCommand": "npm run build"},
        "app": {"windows": [{"title": "Aura-Monitor", "width": 800, "height": 600}]}}


class Tauri(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_tauri_"))

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def escribir(self, ruta: str, texto: str | bytes) -> None:
        p = self.dir / ruta
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(texto) if isinstance(texto, bytes) else p.write_text(texto, encoding="utf-8")

    def proyecto(self, conf: dict | None = None, main_rs: str = MAIN_RS, js: str = "await invoke('get_system_stats');",
                 build_rs: bool = True, icono: bool = True) -> Informe:
        self.escribir("src-tauri/tauri.conf.json", json.dumps(CONF if conf is None else conf))
        self.escribir("src-tauri/Cargo.toml", '[package]\nname = "m"\n[dependencies]\ntauri = "2"\n')
        self.escribir("src-tauri/src/main.rs", main_rs)
        if build_rs:
            self.escribir("src-tauri/build.rs", "fn main() { tauri_build::build() }\n")
        if icono:
            self.escribir("src-tauri/icons/icon.ico", b"\0\0\1\0")
        self.escribir("src/main.ts", f"import {{ invoke }} from '@tauri-apps/api/core';\n{js}\n")
        inf = Informe()
        ecosistemas.tauri(self.dir, self.dir, inf)
        return inf

    def test_un_proyecto_correcto_no_da_errores(self):
        inf = self.proyecto()
        self.assertEqual(inf.errores, [])
        self.assertIn("tauri", inf.tipos)
        self.assertTrue(any("npm run build" in a for a in inf.avisos))  # dist aún no existe: se avisa, no es error
        self.assertTrue(any("1 comandos registrados" in o for o in inf.ok))

    def test_el_caso_aura_monitor_sin_ventana_build_rs_ni_icono(self):
        conf = {k: v for k, v in CONF.items() if k != "app"}
        rs = MAIN_RS.replace("String::new() }", "let mut s = sysinfo::System::new_all(); s.refresh_all(); "
                                                 "String::from(s.cpus()[0].cpu_usage().to_string()) }")
        inf = self.proyecto(conf, main_rs=rs, build_rs=False, icono=False)
        texto = "\n".join(inf.errores)
        self.assertIn("SIN ventana", texto)
        self.assertIn("falta build.rs", texto)
        if os.name == "nt":
            self.assertIn("icons/icon.ico", texto)
        self.assertTrue(any("0 % en la" in a for a in inf.avisos))

    def test_invoke_sin_registrar_y_api_de_tauri_1(self):
        rs = MAIN_RS + "\n#[tauri::command]\nfn olvidado() {}\n"
        inf = self.proyecto(main_rs=rs, js="await invoke('get_stats');")
        texto = "\n".join(inf.errores)
        self.assertIn("olvidado tiene #[tauri::command] pero no está", texto)
        self.assertIn('invoke("get_stats") no corresponde', texto)
        self.escribir("src/main.ts", "import { invoke } from '@tauri-apps/api/tauri';\ninvoke('get_system_stats');\n")
        inf = Informe()
        ecosistemas.tauri(self.dir, self.dir, inf)
        self.assertTrue(any("'@tauri-apps/api/tauri', que no existe en Tauri 2" in e for e in inf.errores))

    def test_window_tauri_sin_with_global_tauri(self):
        """El caso real: `"__TAURI__" in window` con el paquete de npm es falso DENTRO de la app."""
        inf = self.proyecto(js="if ('__TAURI__' in window) { await invoke('get_system_stats'); }")
        self.assertTrue(any("usa window.__TAURI__" in e and "isTauri()" in e for e in inf.errores))
        conf = {**CONF, "app": {**CONF["app"], "withGlobalTauri": True}}
        inf = self.proyecto(conf, js="await window.__TAURI__.core.invoke('get_system_stats');")
        self.assertFalse([e for e in inf.errores if "__TAURI__" in e])
        inf = self.proyecto(js="if ('__TAURI_INTERNALS__' in window) { await invoke('get_system_stats'); }")
        self.assertEqual(inf.errores, [])

    def test_comando_que_devuelve_el_json_como_texto(self):
        """El caso real: -> Result<String, String> con serde_json::to_string: en JS llega una cadena."""
        rs = MAIN_RS.replace("fn get_system_stats() -> String { String::new() }",
                             "fn get_system_stats() -> Result<String, String> {\n    let s = Stats { cpu: 1.0 };\n"
                             "    serde_json::to_string(&s).map_err(|e| e.to_string())\n}")
        inf = self.proyecto(main_rs=rs, js="const s = await invoke('get_system_stats'); s.cpu;")
        self.assertTrue(any("devuelve el JSON ya convertido en texto" in e for e in inf.errores))
        inf = self.proyecto(main_rs=rs, js="const s = JSON.parse(await invoke('get_system_stats'));")
        self.assertFalse([e for e in inf.errores if "convertido en texto" in e])  # el frontend lo parsea: vale
        inf = self.proyecto(js="await invoke('get_system_stats');")  # -> String sin serde_json: es un texto normal
        self.assertEqual(inf.errores, [])

    def test_icono_que_no_es_ico(self):
        self.proyecto()
        (self.dir / "src-tauri" / "icons" / "icon.ico").write_bytes(b"\x89PNG\r\n\x1a\n")  # PNG renombrado
        inf = Informe()
        ecosistemas.tauri(self.dir, self.dir, inf)
        self.assertTrue(any("no es un icono .ico de verdad" in e for e in inf.errores))

    def test_configuracion_de_tauri_1(self):
        inf = self.proyecto({"build": {"distDir": "../dist"}, "tauri": {"windows": [{}]}})
        self.assertTrue(any("configuración de Tauri 1" in e for e in inf.errores))

    def test_sin_tauri_no_hace_nada(self):
        inf = Informe()
        ecosistemas.tauri(self.dir, self.dir, inf)
        self.assertEqual((inf.errores, inf.tipos), ([], []))

    def test_la_guia(self):
        texto = guia("tauri")
        for clave in ("app.windows", "build.rs", "@tauri-apps/api/core", "npx tauri icon", "camelCase", "Mutex<System>"):
            self.assertIn(clave, texto)
        self.assertEqual(guia("tauri2")[:40], texto[:40])
        self.assertIn('guia("tauri")', guia("rust"))


class PaginaFuenteDeVite(unittest.TestCase):
    """index.html con <script type=module src=/src/main.ts> solo funciona con Vite: se prueba dist/, no la fuente."""

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_vite_"))
        (self.dir / "src").mkdir()
        (self.dir / "package.json").write_text('{"name": "x", "private": true}', encoding="utf-8")
        (self.dir / "index.html").write_text('<!doctype html><body><script type="module" src="/src/main.ts"></script>',
                                             encoding="utf-8")
        (self.dir / "src" / "main.ts").write_text("document.body.textContent = 'fuente';\n", encoding="utf-8")

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_detecta_la_pagina_fuente(self):
        self.assertEqual(verificacion.fuente_vite(self.dir / "index.html"), "/src/main.ts")
        (self.dir / "otra.html").write_text('<script src="app.js"></script>', encoding="utf-8")
        self.assertEqual(verificacion.fuente_vite(self.dir / "otra.html"), "")

    def test_sin_compilar_avisa_en_vez_de_dar_un_error_falso(self):
        inf = Informe()
        verificacion.paginas_vite(self.dir, [self.dir / "index.html"], inf)
        self.assertEqual(inf.errores, [])
        self.assertTrue(any("página fuente de Vite" in a for a in inf.avisos))

    @unittest.skipUnless(HAY_NAVEGADOR, "sin playwright")
    def test_prueba_la_compilada_de_dist(self):
        (self.dir / "dist" / "assets").mkdir(parents=True)
        (self.dir / "dist" / "index.html").write_text(
            '<!doctype html><body><script type="module" src="/assets/app.js"></script>', encoding="utf-8")
        (self.dir / "dist" / "assets" / "app.js").write_text("document.body.textContent = 'compilado ok';\n", encoding="utf-8")
        inf = Informe()
        verificacion.paginas_vite(self.dir, [self.dir / "index.html"], inf)
        self.assertEqual(inf.errores, [])
        self.assertTrue(any("compilado ok" in o for o in inf.ok), inf.ok)


if __name__ == "__main__":
    unittest.main()
