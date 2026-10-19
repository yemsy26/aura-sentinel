"""Gravedad del revisor: un [CRITICO] que el propio revisor retira en su última frase no rechaza (el modelo local lista
así lo que está bien), y uno real sí. Los esquemas que Tauri genera (gen/, icons/) no son código para nadie."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import revision, verificacion

# Líneas reales del modelo local (Qwen3.6-35B-A3B) en la revisión de Aura-Monitor (r490).
RETIRADAS = [
    ("- [CRITICO] src-tauri/src/main.rs:20 - `System::new_all()` consume mucha memoria y tiempo al iniciar; para un monitor "
     "es preferible `System::new()`. Sin embargo, no rompe la funcionalidad."),
    ("- [CRITICO] src/main.ts:58-65 - `checkTauri()` invoca el comando para detectar Tauri. Esto es correcto según la lógica, "
     "pero es una ineficiencia. No es un error crítico de funcionamiento, pero es una mala práctica."),
    ("- [CRITICO] src-tauri/src/main.rs:30 - Devuelve `Result<SystemStats, String>`; es preferible `anyhow::Result`. "
     "Sin embargo, funciona."),
    ("- [CRITICO] src/main.ts:25-36 - El estado de RAM solo se muestra como texto. No es un error, pero es funcionalidad "
     "incompleta en cuanto a feedback visual."),
    "- [CRITICO] src/main.ts:1 - Importa `invoke` desde `@tauri-apps/api/core`. Esto es correcto para Tauri 2.",
    ("- [CRITICO] src-tauri/Cargo.toml:14 - Usa `sysinfo = \"0.37\"`. Esta versión es compatible con Tauri 2. No hay "
     "problemas de compatibilidad."),
    ("- [CRITICO] src/main.ts:43-53 - El bloque `catch` actualiza la UI con \"Error\". Esto es bueno. Pero no reintenta la "
     "llamada. Es aceptable."),
    ("- [CRITICO] src-tauri/src/main.rs:14-26 - `AppState` guarda una instancia de `System`. Es la forma estándar de usar "
     "`sysinfo` en un daemon, por lo que es aceptable."),
    "- [CRITICO] src-tauri/src/main.rs:56-60 - El `Builder` registra el comando. Es la estructura correcta para Tauri 2.",
    ("- [CRITICO] src\\main.ts:67-80 - `checkTauri` hace un `invoke` real en vez de `isTauri()`. Sin embargo, dado que "
     "funciona, no es CRÍTICO para la funcionalidad básica."),
    "- [CRITICO] src/main.ts:5 - nombre poco claro; no rompe la funcionalidad - renómbralo",  # una sola frase
]
REALES = [
    ("- [CRITICO] src/main.ts:79 - updateStats se llama una vez. Es correcto llamarla al inicio, pero falta "
     "setInterval(updateStats, 1000)."),
    "- [CRITICO] main.py:40 - el break no rompe el bucle externo y la partida nunca acaba - usa una bandera",
    "- [CRITICO] src/main.ts:5 - invoke(\"get_stats\") no existe. No es un error de compilación, pero falla al ejecutarse.",
    ("- [CRITICO] src/main.ts:18 - getElementById(\"cpu-percent\") no existe en index.html: el valor nunca se muestra - "
     "usa \"cpu-usage\". Esto es correcto en el resto del archivo, salvo esa línea."),
    "- [CRÍTICO] src-tauri/src/main.rs:58 - get_system_stats no está en generate_handler! - regístralo",  # con tilde
    "- [CRITICO] src-tauri/src/main.rs:41 - `cpus[0]` solo mide el primer núcleo. El requisito pide el uso de la CPU.",
]


def revision_con(problemas: list[str], veredicto: str = "APROBADO") -> str:
    return "REQUISITOS:\n- cada 1000 ms -> CUMPLE (src/main.ts:79)\nPROBLEMAS:\n" + "\n".join(problemas) + \
        f"\nVEREDICTO: {veredicto}"


class Gravedad(unittest.TestCase):
    def test_critico_retirado_en_su_ultima_frase_no_cuenta(self):
        for linea in RETIRADAS:
            with self.subTest(linea=linea[:60]):
                self.assertEqual(revision.criticos(revision_con([linea])), [])

    def test_critico_real_cuenta_aunque_diga_algo_positivo(self):
        for linea in REALES:
            with self.subTest(linea=linea[:60]):
                self.assertEqual(revision.criticos(revision_con([linea])), [linea])

    def test_revisar_aprueba_si_solo_quedan_criticos_retirados(self):
        class Modelo:
            contexto = 32768

            def __init__(self, texto):
                self.texto = texto

            def chat(self, *_):
                return {"content": self.texto}

        d = Path(tempfile.mkdtemp(prefix="agente_gravedad_"))
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        (d / "main.py").write_text("print('hola')\n", encoding="utf-8")
        _, ok = revision.revisar(Modelo(revision_con(RETIRADAS[:3] + ["- [MEDIO] main.py:1 - sin docstring - añádelo"])),
                                 d, "saluda", "")
        self.assertTrue(ok)
        _, ok = revision.revisar(Modelo(revision_con(RETIRADAS[:3] + REALES[:1])), d, "saluda", "")
        self.assertFalse(ok)  # el modelo dijo APROBADO, pero queda un CRITICO de verdad
        _, ok = revision.revisar(Modelo(revision_con(RETIRADAS[:1], "CAMBIOS NECESARIOS")), d, "saluda", "")
        self.assertFalse(ok)  # el filtro no le enmienda el veredicto

    def test_el_prompt_define_la_gravedad_por_la_consecuencia(self):
        """Por tipos ("lo frágil es MEDIO") el modelo rebajaba a MEDIO un id del DOM que no existe; por la consecuencia no."""
        sistema = " ".join(revision.SISTEMA.split())
        self.assertIn("CRITICO si, tal como está el código, al ejecutarlo algo falla o un requisito no se cumple", sistema)
        self.assertIn("MEDIO si solo fallaría en otras condiciones o es mejorable", sistema)
        self.assertIn("Lo que compruebas y está bien no es un problema: nunca lo marques CRITICO", sistema)


class GeneradoPorTauri(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_gen_"))

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def escribir(self, *rutas: str) -> None:
        for ruta in rutas:
            p = self.dir / ruta
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text("{}", encoding="utf-8")

    def test_gen_e_icons_junto_a_tauri_conf_no_son_codigo_para_nadie(self):
        """ignorado() lo usan el mapa, los checkpoints, vulture y el revisor."""
        self.escribir("src-tauri/tauri.conf.json", "src-tauri/capabilities/default.json", "src-tauri/gen/schemas/acl.json",
                      "src-tauri/icons/android/values/ic_launcher_background.xml", "gen/api.json", "otro/icons/a.json")
        vistos = {p.relative_to(self.dir).as_posix() for p in verificacion.archivos(self.dir, ".json", ".xml")}
        self.assertEqual(vistos, {"src-tauri/tauri.conf.json", "src-tauri/capabilities/default.json", "gen/api.json",
                                  "otro/icons/a.json"})  # gen/ e icons/ sin tauri.conf.json al lado son del proyecto
        self.assertFalse(hasattr(revision, "_generado"))  # la regla vive solo en ignorado()


if __name__ == "__main__":
    unittest.main()
