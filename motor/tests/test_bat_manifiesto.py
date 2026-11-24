"""C15 (m4185 rust_palabras): el .bat de doble clic tiene que encontrar su proyecto. Un run.bat con `cargo run` estaba en la raíz y
el Cargo.toml en palabras\\: con doble clic daba «could not find Cargo.toml» y verificar(ruta="palabras") no miraba ese .bat."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente.herramientas import Herramientas
from agente.verificacion import informe_proyecto, manifiestos_bat, mensaje_manifiesto

CD = 'cd /d "%~dp0"'


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_manifiesto_"))

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def crear(self, *archivos: str) -> None:
        for a in archivos:
            p = self.dir / a
            p.parent.mkdir(parents=True, exist_ok=True)
            if not a.endswith("/"):
                p.write_text("x\n", encoding="utf-8")

    def lineas(self, *ordenes: str, donde: str = ".") -> list:
        return manifiestos_bat((self.dir / donde).resolve(), [CD, *ordenes])


class TestSeMarca(Base):
    def test_cada_herramienta_sin_su_manifiesto(self):
        casos = {"cargo run": "Cargo.toml", "cargo test": "Cargo.toml", "go run .": "go.mod", "go test ./...": "go.mod",
                 "npm start": "package.json", "pnpm install": "package.json", "call npm run dev": "package.json",
                 "dotnet run": "*.csproj", "mvn clean package": "pom.xml", "gradlew.bat build": "build.gradle",
                 "flutter run": "pubspec.yaml", "docker compose up": "compose.yaml", "docker-compose up -d": "compose.yaml",
                 'start "" cmd /k cargo run': "Cargo.toml"}
        for orden, manifiesto in casos.items():
            with self.subTest(orden=orden):
                r = self.lineas(orden)
                self.assertEqual(len(r), 1, r)
                self.assertEqual(r[0][2], manifiesto)

    def test_el_caso_de_m4185_dice_donde_esta(self):
        self.crear("palabras/Cargo.toml", "palabras/src/main.rs")
        _, orden, manifiesto, cerca = self.lineas("cargo run -- %*")[0]
        self.assertEqual((orden, manifiesto, cerca), ("cargo run -- %*", "Cargo.toml", ["palabras"]))
        self.assertIn("cd /d \"%~dp0palabras\"", mensaje_manifiesto(orden, manifiesto, cerca))

    def test_a_dos_niveles(self):
        self.crear("a/b/Cargo.toml")
        self.assertEqual(self.lineas("cargo build")[0][3], ["a/b"])

    def test_cd_relativo_pushd_y_popd(self):
        self.crear("sub/Cargo.toml")
        self.assertEqual(len(self.lineas("cd sub", "cargo run")), 0)  # en sub sí está
        self.assertEqual(len(self.lineas("cd sub", "cd ..", "cargo run")), 1)  # volvió a la raíz, donde no está
        self.assertEqual(len(self.lineas("pushd sub", "cargo build", "popd", "cargo run")), 1)  # tras popd, la raíz
        self.assertEqual(len(self.lineas("cd sub && cargo build && cd .. && cargo run")), 1)

    def test_el_bat_dentro_de_una_subcarpeta(self):
        self.crear("sub/Cargo.toml", "otra/x.txt")
        self.assertEqual(len(self.lineas("cargo run", donde="sub")), 0)
        self.assertEqual(len(self.lineas("cargo run", donde="otra")), 1)
        self.assertEqual(len(manifiestos_bat((self.dir / "otra").resolve(), [CD, "cd .."])), 0)


class TestNoSeMarca(Base):
    def test_con_el_manifiesto_en_su_sitio(self):
        self.crear("Cargo.toml", "go.mod", "package.json", "App.csproj", "pom.xml", "build.gradle", "pubspec.yaml", "compose.yaml")
        for orden in ("cargo run", "go run .", "npm start", "dotnet run", "mvn clean package", "gradlew build", "flutter run",
                      "docker compose up"):
            with self.subTest(orden=orden):
                self.assertEqual(self.lineas(orden), [])

    def test_el_cd_que_lleva_al_proyecto(self):
        self.crear("palabras/Cargo.toml")
        self.assertEqual(self.lineas('cd /d "%~dp0palabras"', "cargo run"), [])
        self.assertEqual(self.lineas('cd /d "%~dp0palabras" && cargo run -- %*'), [])
        self.assertEqual(self.lineas("cd palabras && cargo test"), [])

    def test_argumentos_que_cambian_el_sitio(self):
        for orden in ("cargo run --manifest-path palabras/Cargo.toml", "go run main.go", "go test hola_test.go",
                      "npm --prefix web run dev", "npm run build --prefix web", "dotnet run --project src/App",
                      "dotnet build App.sln", "mvn -f sub/pom.xml package", "docker compose -f deploy/compose.yaml up",
                      "gradlew -p sub build"):
            with self.subTest(orden=orden):
                self.assertEqual(self.lineas(orden), [])

    def test_el_bat_que_crea_el_manifiesto(self):
        self.assertEqual(self.lineas("cargo new demo", "cd demo", "cargo run"), [])
        self.assertEqual(self.lineas("go mod init demo", "go run ."), [])
        self.assertEqual(self.lineas("npm init -y", "npm start"), [])

    def test_carpeta_desconocida(self):
        self.assertEqual(self.lineas("cd /d %PROYECTO%", "cargo run"), [])
        self.assertEqual(self.lineas("cd inexistente", "cargo run"), [])
        self.assertEqual(self.lineas("if exist palabras (cd palabras)", "cargo run"), [])
        self.assertEqual(self.lineas("for %%d in (a b) do cd %%d", "npm start"), [])

    def test_lo_que_no_es_una_orden_de_construccion(self):
        for orden in ("echo cargo run", "rem cargo run", ":: npm start", "cargo --version", "cargo install ripgrep",
                      "npx serve", "npm --version", "python app.py", "flutter doctor", "if exist Cargo.toml cargo run",
                      "docker ps", "go version"):
            with self.subTest(orden=orden):
                self.assertEqual(self.lineas(orden), [])


class TestVerificar(Base):
    def test_verificar_de_una_subcarpeta_mira_el_bat_de_la_raiz(self):
        self.crear("sub/notas.txt")
        (self.dir / "run.bat").write_text(CD + "\ncargo run -- %*\n", encoding="utf-8")
        inf = informe_proyecto(self.dir, sys.executable, "sub")
        self.assertTrue(any("run.bat:2 'cargo run -- %*' necesita Cargo.toml" in e for e in inf.errores), inf.errores)

    def test_sin_bat_en_la_raiz_o_con_el_bat_bien_no_hay_nada_nuevo(self):
        self.crear("sub/notas.txt")
        self.assertEqual([e for e in informe_proyecto(self.dir, sys.executable, "sub").errores if "necesita" in e], [])
        (self.dir / "run.bat").write_text(CD + "\necho hola\n", encoding="utf-8")
        self.assertEqual([e for e in informe_proyecto(self.dir, sys.executable, "sub").errores if "necesita" in e], [])

    def test_verificar_de_la_raiz_tambien(self):
        (self.dir / "test.bat").write_text(CD + "\nnpm test\n", encoding="utf-8")
        inf = informe_proyecto(self.dir, sys.executable, ".")
        self.assertEqual(sum("test.bat:2 'npm test' necesita package.json" in e for e in inf.errores), 1, inf.errores)


class TestCrearBat(Base):
    def setUp(self):
        super().setUp()
        self.h = Herramientas(self.dir)

    def test_rechaza_el_caso_visto_con_el_cd_que_lo_arregla(self):
        self.crear("palabras/Cargo.toml")
        r = self.h.crear_bat("run", ["cargo run -- %*"])
        self.assertTrue(r.startswith("ERROR:"), r)
        self.assertIn('cd /d "%~dp0palabras"', r)
        self.assertIn("No he creado run.bat", r)
        self.assertFalse((self.dir / "run.bat").exists())

    def test_crea_si_ya_lleva_el_cd(self):
        self.crear("palabras/Cargo.toml")
        r = self.h.crear_bat("run", ['cd /d "%~dp0palabras"', "cargo run -- %*"])
        self.assertTrue(r.startswith("OK creado"), r)

    def test_crea_si_el_proyecto_aun_no_existe_o_esta_en_la_raiz(self):
        self.assertTrue(self.h.crear_bat("run", ["cargo run"]).startswith("OK creado"))  # verificar lo dirá al final
        self.crear("Cargo.toml")
        self.assertTrue(self.h.crear_bat("test", ["cargo test"]).startswith("OK creado"))


if __name__ == "__main__":
    unittest.main()
