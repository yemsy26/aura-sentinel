"""Descarga del modelo: reanuda, verifica el SHA-256 y no repite lo que ya está bien (sin red: urlopen simulado)."""
import hashlib
import io
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from agente import descargar

DATOS = bytes(range(256)) * 64  # 16 KiB
SHA = hashlib.sha256(DATOS).hexdigest()


class Respuesta(io.BytesIO):
    def __init__(self, datos: bytes, status: int):
        super().__init__(datos)
        self.status = status

    def __enter__(self):
        return self

    def __exit__(self, *_):
        return False


def servidor(pedidos: list):
    """urlopen falso que respeta la cabecera Range."""
    def urlopen(pedido, timeout=None):
        rango = pedido.headers.get("Range") if hasattr(pedido, "headers") else None
        pedidos.append(rango)
        desde = int(rango.split("=")[1].rstrip("-")) if rango else 0
        return Respuesta(DATOS[desde:], 206 if rango else 200)
    return urlopen


class TestDescargar(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        silencio = mock.patch.object(descargar, "print", create=True)  # el progreso no ensucia la salida de las pruebas
        silencio.start()
        self.addCleanup(silencio.stop)

    def test_descarga_y_verifica(self):
        pedidos = []
        with mock.patch.object(descargar.urllib.request, "urlopen", servidor(pedidos)):
            ruta = descargar.asegurar("u/r", "m.gguf", self.dir, len(DATOS), SHA)
        self.assertEqual(ruta.read_bytes(), DATOS)
        self.assertFalse((self.dir / "m.gguf.part").exists())
        self.assertEqual(pedidos, [None])

    def test_reanuda_lo_que_quedo_a_medias(self):
        (self.dir / "m.gguf.part").write_bytes(DATOS[:5000])
        pedidos = []
        with mock.patch.object(descargar.urllib.request, "urlopen", servidor(pedidos)):
            descargar.asegurar("u/r", "m.gguf", self.dir, len(DATOS), SHA)
        self.assertEqual(pedidos, ["bytes=5000-"])
        self.assertEqual((self.dir / "m.gguf").read_bytes(), DATOS)

    def test_no_repite_lo_que_ya_esta_bien(self):
        (self.dir / "m.gguf").write_bytes(DATOS)
        with mock.patch.object(descargar.urllib.request, "urlopen", side_effect=AssertionError("no debía descargar")):
            descargar.asegurar("u/r", "m.gguf", self.dir, len(DATOS), SHA)

    def test_rechaza_un_archivo_danado(self):
        with mock.patch.object(descargar.urllib.request, "urlopen", servidor([])), \
                self.assertRaisesRegex(RuntimeError, "SHA-256"):
            descargar.asegurar("u/r", "m.gguf", self.dir, len(DATOS), "0" * 64)
        self.assertFalse((self.dir / "m.gguf").exists())
        self.assertFalse((self.dir / "m.gguf.part").exists())

    def test_reintenta_tras_un_corte(self):
        llamadas = {"n": 0}
        real = servidor([])

        def con_corte(pedido, timeout=None):
            llamadas["n"] += 1
            if llamadas["n"] == 1:
                raise descargar.urllib.error.URLError("sin red")
            return real(pedido, timeout)
        with mock.patch.object(descargar.urllib.request, "urlopen", con_corte), \
                mock.patch.object(descargar.time, "sleep"):
            descargar.asegurar("u/r", "m.gguf", self.dir, len(DATOS), SHA)
        self.assertEqual(llamadas["n"], 2)


if __name__ == "__main__":
    unittest.main()
