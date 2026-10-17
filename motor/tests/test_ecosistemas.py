"""Pruebas de Rust, Go, .NET, Node, Docker y YAML en verificar(), y de los permisos para acciones hacia fuera.

Las que necesitan cargo, go, dotnet, npm o Docker en marcha se saltan si no están en el equipo."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from typing import ClassVar

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from agente import conocimiento, ecosistemas
from agente.herramientas import INSTALACIONES, Herramientas, externo
from agente.verificacion import Informe, es_codigo, verificar


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_ecosistemas_"))

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def escribir(self, archivos: dict) -> None:
        for ruta, texto in archivos.items():
            p = self.dir / ruta
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(texto, encoding="utf-8")

    def verificar(self) -> tuple[str, int]:
        return verificar(self.dir, sys.executable, ".", 5)


@unittest.skipUnless(shutil.which("cargo"), "sin Rust")
class TestRust(Base):
    PROYECTO: ClassVar[dict] = {"Cargo.toml": '[package]\nname = "demo"\nversion = "0.1.0"\nedition = "2021"\n',
                "src/main.rs": "fn suma(a: i32, b: i32) -> i32 { a + b }\nfn main() { println!(\"{}\", suma(2, 3)); }\n"
                               "#[cfg(test)]\nmod t { #[test] fn s() { assert_eq!(super::suma(2, 3), 5); } }\n"}

    def test_compila_y_pasa_las_pruebas(self):
        self.escribir(self.PROYECTO)
        informe, errores = self.verificar()
        self.assertEqual(errores, 0, informe)
        self.assertIn("pasan 1 pruebas", informe)

    def test_error_de_compilacion_con_archivo_y_linea(self):
        self.escribir({**self.PROYECTO, "src/main.rs": self.PROYECTO["src/main.rs"].replace("a + b }", "a + c }")})
        informe, errores = self.verificar()
        self.assertEqual(errores, 1, informe)
        self.assertRegex(informe, r"main\.rs:1 .*`c`")

    def test_prueba_que_falla(self):
        self.escribir({**self.PROYECTO, "src/main.rs": self.PROYECTO["src/main.rs"].replace("a + b }", "a - b }")})
        informe, errores = self.verificar()
        self.assertGreaterEqual(errores, 1, informe)
        self.assertIn("cargo test falla", informe)


@unittest.skipUnless(shutil.which("go"), "sin Go")
class TestGo(Base):
    PROYECTO: ClassVar[dict] = {"go.mod": "module demo\n\ngo 1.22\n",
                "main.go": 'package main\n\nimport "fmt"\n\nfunc Suma(a, b int) int { return a + b }\n\n'
                           'func main() { fmt.Println(Suma(2, 3)) }\n',
                "main_test.go": 'package main\n\nimport "testing"\n\n'
                                'func TestSuma(t *testing.T) {\n\tif Suma(2, 3) != 5 {\n\t\tt.Fatal("mal")\n\t}\n}\n'}

    def test_compila_y_pasa_las_pruebas(self):
        self.escribir(self.PROYECTO)
        informe, errores = self.verificar()
        self.assertEqual(errores, 0, informe)
        self.assertIn("go vet limpio", informe)

    def test_error_de_compilacion_con_archivo_y_linea(self):
        self.escribir({**self.PROYECTO, "main.go": self.PROYECTO["main.go"].replace("a + b", "a + c")})
        informe, errores = self.verificar()
        self.assertGreaterEqual(errores, 1, informe)
        self.assertRegex(informe, r"main\.go:5 .*undefined: c")

    def test_sintaxis_al_escribir(self):
        h = Herramientas(self.dir)
        self.assertIn("ERROR", h.escribir("roto.go", "package main\n\nfunc main() {\n"))


@unittest.skipUnless(shutil.which("npm"), "sin Node")
class TestNode(Base):
    PROYECTO: ClassVar[dict] = {"package.json": '{"name":"demo","version":"1.0.0","type":"module","scripts":{"test":"node --test"}}\n',
                "suma.js": "export const suma = (a, b) => a + b;\n",
                "suma.test.js": 'import { test } from "node:test";\nimport assert from "node:assert/strict";\n'
                                'import { suma } from "./suma.js";\ntest("s", () => assert.equal(suma(2, 3), 5));\n'}

    def test_pasa_npm_test(self):
        self.escribir(self.PROYECTO)
        informe, errores = self.verificar()
        self.assertEqual(errores, 0, informe)
        self.assertIn("npm test OK", informe)

    def test_prueba_que_falla(self):
        self.escribir({**self.PROYECTO, "suma.js": "export const suma = (a, b) => a - b;\n"})
        informe, errores = self.verificar()
        self.assertEqual(errores, 1, informe)
        self.assertIn("npm test falla", informe)

    def test_faltan_dependencias(self):
        self.escribir({"package.json": '{"name":"demo","dependencies":{"express":"^5.0.0"}}\n'})
        informe, _ = self.verificar()
        self.assertIn("npm install", informe)


@unittest.skipUnless(shutil.which("dotnet"), "sin .NET")
class TestDotnet(Base):
    def test_error_de_compilacion_con_archivo_y_linea(self):
        self.escribir({"demo.csproj": '<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><OutputType>Exe</OutputType>'
                                      '<TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>\n',
                       "Program.cs": "System.Console.WriteLine(2 + x);\n"})
        informe, errores = self.verificar()
        self.assertEqual(errores, 1, informe)
        self.assertRegex(informe, r"Program\.cs:1 CS0103")


class TestDockerfile(Base):
    def revisar(self, dockerfile: str, extra: dict | None = None) -> Informe:
        self.escribir({"Dockerfile": dockerfile, **(extra or {})})
        inf = Informe()
        ecosistemas.revisar_dockerfile(self.dir / "Dockerfile", self.dir, inf)
        return inf

    def test_dockerfile_correcto(self):
        inf = self.revisar("FROM python:3.12-slim AS base\nCOPY app.py /app/\nRUN useradd app\nUSER app\n"
                           'CMD ["python", "/app/app.py"]\n', {"app.py": "print(1)\n"})
        self.assertEqual((inf.errores, inf.avisos), ([], []))

    def test_copy_de_algo_que_no_existe(self):
        inf = self.revisar("FROM python:3.12-slim\nCOPY requirements.txt .\nUSER nobody\n")
        self.assertEqual(len(inf.errores), 1)
        self.assertIn("requirements.txt", inf.errores[0])

    def test_credencial_en_la_imagen(self):
        inf = self.revisar("FROM node:24-alpine\nENV DB_PASSWORD=supersecreta\nENV PASSWORD_FILE=/run/secrets/db\nUSER node\n")
        self.assertEqual(len(inf.errores), 1, inf.errores)
        self.assertIn("credencial", inf.errores[0])

    def test_avisos_de_buenas_practicas(self):
        inf = self.revisar("FROM --platform=linux/amd64 nginx\nFROM golang:1.26-alpine AS build\nFROM build\n")
        avisos = " ".join(inf.avisos)
        self.assertIn("FROM nginx sin versión", avisos)
        self.assertNotIn("FROM build", avisos)  # una etapa anterior no es una imagen
        self.assertIn("root", avisos)

    def test_compose_y_yaml(self):
        self.escribir({"compose.yaml": "services:\n  web:\n    image: nginx:1.27\n",
                       "k8s/app.yaml": "apiVersion: v1\nkind: Pod\nmetadata:\n\tname: x\n"})
        informe, _ = self.verificar()
        self.assertIn("k8s\\app.yaml:4 YAML no admite tabuladores", informe)
        if shutil.which("docker"):
            self.assertNotIn("compose config falla", informe)

    @unittest.skipUnless(ecosistemas.docker_activo(), "Docker apagado")
    def test_construye_la_imagen(self):
        self.escribir({"Dockerfile": "FROM alpine:3.20\nRUN exit 3\nUSER nobody\n"})
        informe, errores = self.verificar()
        self.assertEqual(errores, 1, informe)
        self.assertIn("docker build falla", informe)


class TestHerramientasMultilenguaje(Base):
    def test_que_cuenta_como_codigo(self):
        for nombre in ("main.rs", "main.go", "Program.cs", "App.tsx", "Dockerfile", "api.Dockerfile", "compose.yaml",
                       "go.mod", ".dockerignore", "deploy.sh"):
            self.assertTrue(es_codigo(Path(nombre)), nombre)
        self.assertFalse(es_codigo(Path("foto.png")))

    def test_puntos_de_restauracion_incluyen_rust_y_dockerfile(self):
        self.escribir({"src/main.rs": "fn main() {}\n", "Dockerfile": "FROM alpine:3.20\n", "target/debug/x.rs": "basura"})
        h = Herramientas(self.dir)
        h._checkpoint()
        guardados = set(h.checkpoints[-1]["archivos"])
        self.assertIn("src\\main.rs", guardados)
        self.assertIn("Dockerfile", guardados)
        self.assertNotIn("target\\debug\\x.rs", guardados)

    def test_mapa_de_rust_y_go(self):
        self.escribir({"src/lib.rs": "pub fn contar() {}\nstruct Datos;\n", "srv/main.go": "package main\nfunc Arrancar() {}\n"})
        mapa = Herramientas(self.dir).mapa()
        self.assertIn("contar, Datos", mapa)
        self.assertIn("Arrancar", mapa)

    def test_compilaciones_largas_tienen_tiempo(self):
        for c in ("cargo build --release", "go test ./...", "docker build -t app .", "docker compose up -d --build",
                  "dotnet restore", "gradlew.bat build", "mvn -q package"):
            self.assertTrue(INSTALACIONES.search(c), c)
        for c in ("cargo --version", "go version", "docker ps", "dotnet --info"):
            self.assertFalse(INSTALACIONES.search(c), c)

    def test_acciones_hacia_fuera_piden_permiso(self):
        for c in ("ssh -o BatchMode=yes u@10.0.0.5 \"ls\"", "scp -P 2222 app.tgz u@host:/opt", "docker push yo/app:1",
                  "kubectl apply -f k8s", "terraform apply", "npm publish", "git push", "aws s3 cp a s3://b",
                  "curl -X POST https://api.example.com/x -d @a.json"):
            self.assertTrue(externo(c), c)
        for c in ("ssh-keygen -t ed25519 -f k", "docker build -t app .", "curl -s http://localhost:8080/health",
                  "curl -X POST http://127.0.0.1:8080/x -d {}", "kubectl get pods", "docker run -d app"):
            self.assertFalse(externo(c), c)

    def test_permiso_con_motivo_y_bloqueo_sin_permiso(self):
        pedidos = []
        h = Herramientas(self.dir, confirmar=lambda c, motivo: pedidos.append(motivo) or False)
        r = h.ejecutar("scp a.txt u@servidor:/tmp")
        self.assertIn("BLOQUEADO", r)
        self.assertIn("fuera de este equipo", pedidos[0])
        antiguo = Herramientas(self.dir, confirmar=lambda c: True)  # integraciones con confirmar(comando)
        self.assertIsNone(antiguo._permitido("ssh u@servidor \"echo ok\""))
        self.assertIn("BLOQUEADO", Herramientas(self.dir).ejecutar("docker push yo/app:1"))

    def test_un_script_con_ssh_dentro_tambien_pide_permiso(self):
        self.escribir({"deploy.bat": "@echo off\necho Conectando por ssh...\nscp -P 2222 app u@localhost:/opt\n",
                       "local.bat": "@echo off\necho ssh no se usa aqui\nver\n"})
        pedidos = []
        h = Herramientas(self.dir, confirmar=lambda c, motivo: pedidos.append(c) or False)
        self.assertIn("BLOQUEADO", h.ejecutar("deploy.bat"))
        self.assertIn("el script deploy.bat ejecuta: scp -P 2222", pedidos[0])
        self.assertNotIn("BLOQUEADO", h.ejecutar("call local.bat", segundos=10))  # un echo con 'ssh' no cuenta
        self.assertEqual(len(pedidos), 1)

    def test_la_revision_ve_los_comandos_ejecutados(self):
        from agente.revision import revisar

        class LLMFalso:
            contexto = 8192

            def __init__(self):
                self.recibido = []

            def chat(self, mensajes, *_):
                self.recibido.append(mensajes[1]["content"])
                return {"content": "REQUISITOS:\n- desplegar -> CUMPLE\nPROBLEMAS:\n- ninguno\nVEREDICTO: APROBADO"}

        h = Herramientas(self.dir)
        h.ejecutar("echo desplegado-ok")
        self.escribir({"main.go": "package main\n"})
        llm = LLMFalso()
        _, ok = revisar(llm, self.dir, "despliega", "", "", h.evidencia_texto())
        self.assertTrue(ok)
        self.assertIn("COMANDOS EJECUTADOS", llm.recibido[0])
        self.assertIn("echo desplegado-ok -> exit=0: desplegado-ok", llm.recibido[0])

    def test_aviso_de_credencial_escrita(self):
        h = Herramientas(self.dir)
        self.assertIn("credencial", h.escribir("config.py", 'API_KEY = "sk_live_' + "a" * 20 + '"\n'))
        self.assertNotIn("credencial", h.escribir("config2.py", 'import os\nAPI_KEY = os.environ["API_KEY"]\n'))

    def test_guias_nuevas_y_alias(self):
        for tema, titulo in (("rust", "# Rust"), ("golang", "# Go"), ("typescript", "# Node"), ("c#", "# .NET"),
                             ("contenedores", "# Docker"), ("servidor", "# Despliegue"), ("k8s", "# Despliegue")):
            self.assertTrue(conocimiento.guia(tema).startswith(titulo), tema)


if __name__ == "__main__":
    unittest.main()
