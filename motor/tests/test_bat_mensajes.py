"""Los mensajes de un .bat de crear_bat salen enteros: «echo C -> F» no imprimía nada y creaba un archivo «F», y un & | < en un
echo o en la descripción cortaba o abortaba el .bat (m4204). Las redirecciones y órdenes intencionadas siguen igual."""
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_agente import borrar

from agente.herramientas import Herramientas, mensaje_bat
from agente.proyecto import Informe
from agente.verificacion import bats

# línea del modelo -> lo que debe verse en pantalla
MENSAJES = {
    "echo === Conversor Celsius -> Fahrenheit ===": "=== Conversor Celsius -> Fahrenheit ===",
    "echo Requiere Python >= 3.8": "Requiere Python >= 3.8",
    "echo Compilar & ejecutar": "Compilar & ejecutar",
    "echo === Menu | Opciones ===": "=== Menu | Opciones ===",
    "echo Uso: run.bat <archivo> [salida]": "Uso: run.bat <archivo> [salida]",
    "echo >>> Iniciando el servidor": ">>> Iniciando el servidor",
    "echo Ve a Archivo > Abrir para continuar": "Ve a Archivo > Abrir para continuar",
    "echo Abre => http://localhost:8000": "Abre => http://localhost:8000",
    "echo Espera 1 || 2 minutos": "Espera 1 || 2 minutos",
    "echo Escribe tu opcion >": "Escribe tu opcion >",
    'echo Pulsa "S & N" o ^& para seguir': 'Pulsa "S & N" o & para seguir',
}
INTACTAS = [
    "echo x > archivo", "echo. > .env", "echo a>>log.txt", "echo a >nul 2>&1", "echo a 2>nul", "echo a 1>&2", "echo a >&2",
    'echo a > "%TEMP%\\x.log"', "echo a > logs\\salida", "echo %DATE% >> registro.txt", "echo y| findstr y",
    "echo y | del /p x.tmp", "echo. & echo Listo", "echo Instalando && python -m pip --version",
    "echo Listo & python main.py > log.txt", "echo a ^> b ^& c", 'echo "a & b > c"', "echo %PATH%",
    "echo CLAVE=> .env", "echo DB_PASSWORD=>> .env", "echo ^<html^>> index.html", "echo Listo & timeout /t 1", "echo | set /p=Texto",
]


class Mensajes(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_msj_"))
        self.h = Herramientas(self.dir)

    def tearDown(self):
        self.h.detener_todos()
        borrar(self.dir)

    def crear(self, comandos, descripcion="") -> str:
        self.assertTrue(self.h.crear_bat("run", comandos, descripcion).startswith("OK"))
        return (self.dir / "run.bat").read_text(encoding="utf-8")

    def abrir(self) -> subprocess.CompletedProcess:
        return subprocess.run(["cmd", "/c", str(self.dir / "run.bat")], cwd=self.dir, env={**os.environ, "AGENTE_NOPAUSE": "1"},
                              stdin=subprocess.DEVNULL, capture_output=True, text=True, encoding="utf-8", errors="replace",
                              timeout=60, check=False)

    def archivos(self) -> set[str]:
        return {p.name for p in self.dir.iterdir() if p.name != ".agente"}

    def test_los_mensajes_salen_enteros_y_no_crean_archivos(self):
        texto = self.crear([*MENSAJES, "title Ventas & Compras"], "Conversor C -> F & más\n\nUso: <archivo> | salir")
        r = self.abrir()
        self.assertEqual((r.returncode, r.stderr), (0, ""), r.stdout)
        vistas = [linea.rstrip() for linea in r.stdout.splitlines()]
        for mensaje in ["Conversor C -> F & más", "Uso: <archivo> | salir", *MENSAJES.values()]:
            self.assertIn(mensaje, vistas)
        self.assertEqual(self.archivos(), {"run.bat"})
        self.assertNotIn("FALTA", texto)  # «ejecutar», «Compras»... no son programas que buscar
        inf = Informe()
        bats(self.dir, [self.dir / "run.bat"], inf)
        self.assertEqual(inf.errores, [])

    def test_las_redirecciones_y_ordenes_intencionadas_funcionan(self):
        self.crear(["echo dato > salida.txt", "echo. > .env", "echo uno>>log.txt", "echo dos >> log.txt", "echo callado >nul",
                    "echo aviso 1>&2", "echo si| findstr si", "echo. & echo Listo -> fin", "rem a > b & c"])
        r = self.abrir()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(self.archivos(), {"run.bat", "salida.txt", ".env", "log.txt"})
        self.assertEqual((self.dir / "salida.txt").read_text().strip(), "dato")
        self.assertEqual((self.dir / "log.txt").read_text().split(), ["uno", "dos"])
        self.assertIn("aviso", r.stderr)
        self.assertNotIn("callado", r.stdout)
        vistas = [linea.rstrip() for linea in r.stdout.splitlines()]
        self.assertIn("si", vistas)
        self.assertIn("Listo -> fin", vistas)

    def test_lo_intencionado_no_se_toca(self):
        for linea in INTACTAS:
            with self.subTest(linea):
                self.assertEqual(mensaje_bat(linea, self.dir), linea)

    def test_un_bat_del_proyecto_tras_ampersand_sigue_siendo_una_orden(self):
        (self.dir / "preparar.bat").write_bytes(b"@echo off\r\n")
        self.assertEqual(mensaje_bat("echo Antes & preparar", self.dir), "echo Antes & preparar")
        self.assertEqual(mensaje_bat("echo Antes & despues", self.dir), "echo Antes ^& despues")

    def test_el_titulo_del_nombre_del_archivo_se_escapa(self):
        self.h.crear_bat("ventas & compras", ["echo hola"])
        self.assertIn("title ventas ^& compras\n", (self.dir / "ventas & compras.bat").read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
