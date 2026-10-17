"""Pruebas de version() en crates.io, Go, NuGet y Docker Hub (sin red: respuestas grabadas) y de la herramienta
preguntar, también por el protocolo --json."""
import io
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from test_agente import LLMGuion, llamada

from agente.herramientas import MAX_PREGUNTAS, Herramientas

RESPUESTAS = {
    "https://crates.io/api/v1/crates/serde": {"crate": {"name": "serde", "max_stable_version": "1.0.229",
                                                        "max_version": "2.0.0-rc.1", "description": "Serialización",
                                                        "documentation": "https://docs.rs/serde"}},
    "https://proxy.golang.org/github.com/!burnt!sushi/toml/@latest": {"Version": "v1.6.0", "Time": "2025-12-18T12:15:22Z"},
    "https://proxy.golang.org/github.com/golang/protobuf/@latest": {"Version": "v1.5.4", "Time": "2024-03-06T00:00:00Z"},
    "https://api.nuget.org/v3-flatcontainer/newtonsoft.json/index.json": {"versions": ["9.0.1", "13.0.3", "13.0.4",
                                                                                       "14.0.1-beta2"]},
    "https://hub.docker.com/v2/repositories/library/golang/tags?page_size=100&ordering=last_updated": {"results": [
        {"name": n} for n in ("tip-alpine", "latest", "1.27.2-windowsservercore-ltsc2025", "1.27.2", "1.27.2-alpine",
                              "1.26.9-alpine", "1.27-alpine", "1.9.7", "nanoserver")]},
}
MODS = {"https://proxy.golang.org/github.com/!burnt!sushi/toml/@v/v1.6.0.mod": "module github.com/BurntSushi/toml\n",
        "https://proxy.golang.org/github.com/golang/protobuf/@v/v1.5.4.mod":
            '// Deprecated: Use the "google.golang.org/protobuf" module instead.\nmodule github.com/golang/protobuf\n'}


def descargar_falso(url, datos=None):
    if url in MODS:
        return MODS[url], "text/plain"
    if url not in RESPUESTAS:
        raise OSError(f"HTTP Error 404: {url}")
    return json.dumps(RESPUESTAS[url]), "application/json"


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_fase2_"))
        self.h = Herramientas(self.dir)
        self.h._descargar = descargar_falso

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)


class TestVersiones(Base):
    def test_crates_usa_la_estable(self):
        r = self.h.version("serde", "crates")
        self.assertIn("serde 1.0.229", r)
        self.assertIn("cargo add serde", r)

    def test_go_escapa_mayusculas_y_avisa_si_esta_obsoleto(self):
        self.assertIn("go get github.com/BurntSushi/toml@v1.6.0", self.h.version("github.com/BurntSushi/toml", "go"))
        self.assertIn("OBSOLETO: Use the \"google.golang.org/protobuf\"", self.h.version("github.com/golang/protobuf", "golang"))
        self.assertIn("biblioteca estándar", self.h.version("net/http", "go"))

    def test_nuget_la_mayor_estable(self):
        r = self.h.version("Newtonsoft.Json", "nuget")
        self.assertIn("Newtonsoft.Json 13.0.4", r)
        self.assertIn("dotnet add package Newtonsoft.Json --version 13.0.4", r)

    def test_docker_ignora_tip_windows_y_latest(self):
        r = self.h.version("golang", "docker")
        self.assertIn("versión más reciente 1.27.2", r)
        self.assertIn("golang:1.27.2-alpine", r)
        self.assertNotIn("windowsservercore", r)
        self.assertNotIn("tip", r)
        self.assertIn("golang:1.27.2-alpine", self.h.version("golang:alpine", "docker"))

    def test_error_de_red_no_tumba_al_agente(self):
        self.assertIn("No pude consultar", self.h.version("no-existe", "crates"))


class TestPreguntar(Base):
    def test_sin_usuario_decide_el_agente(self):
        self.assertIn("No hay nadie a quien preguntar", self.h.preguntar("¿Puerto?", ["8080", "3000"]))

    def test_respuesta_y_limite_por_tarea(self):
        recibidas = []
        self.h.preguntar_cb = lambda p, o: recibidas.append((p, o)) or o[-1]
        self.assertEqual(self.h.preguntar("¿Base de datos?", ["SQLite", " ", "PostgreSQL"]), "Respuesta del usuario: PostgreSQL")
        self.assertEqual(recibidas[0], ("¿Base de datos?", ["SQLite", "PostgreSQL"]))  # sin opciones vacías
        for _ in range(MAX_PREGUNTAS - 1):
            self.h.preguntar("¿Otra?", ["a", "b"])
        self.assertIn("Ya preguntaste", self.h.preguntar("¿Una más?", ["a", "b"]))
        self.h.nueva_tarea()
        self.assertIn("Respuesta del usuario", self.h.preguntar("¿Nueva tarea?", ["a", "b"]))

    def test_pregunta_vacia(self):
        self.h.preguntar_cb = lambda p, o: "x"
        self.assertIn("Indica 'pregunta'", self.h.preguntar("  "))


class TestPreguntaPorJson(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="agente_fase2_json_"))

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_evento_pregunta_y_respuesta_libre(self):
        from agente import __main__ as cli
        from agente import config
        LLMGuion.respuestas = [
            llamada("preguntar", pregunta="¿En qué lenguaje?", opciones=["Go", "Rust"]),
            {"content": "Usaré Zig, como pediste.", "tool_calls": [], "finish_reason": "stop", "usage": {}},
        ]
        ordenes = [{"tipo": "tarea", "texto": "crea un servidor"}, {"tipo": "respuesta", "texto": "Zig"}, {"tipo": "salir"}]
        entrada = io.TextIOWrapper(io.BytesIO("".join(json.dumps(o) + "\n" for o in ordenes).encode()), encoding="utf-8")
        salida = io.StringIO()
        falso = lambda cfg: ("http://falso", "falso", 32768)
        with mock.patch.object(cli, "detectar", falso), mock.patch("agente.bucle.cliente.detectar", falso), \
                mock.patch("agente.bucle.cliente.LLM", LLMGuion), mock.patch("agente.bucle.entorno", lambda: "prueba"), \
                mock.patch.object(sys, "stdin", entrada), mock.patch.object(sys, "stdout", salida):
            self.assertEqual(cli.modo_json(config.cargar(), str(self.dir)), 0)
        eventos = [json.loads(linea) for linea in salida.getvalue().splitlines()]
        self.assertIn({"tipo": "pregunta", "pregunta": "¿En qué lenguaje?", "opciones": ["Go", "Rust"]}, eventos)
        paso = next(e for e in eventos if e["tipo"] == "paso" and e["herramienta"] == "preguntar")
        self.assertEqual(paso["resultado"], "Respuesta del usuario: Zig")
        self.assertFalse(paso["error"])


if __name__ == "__main__":
    unittest.main()
