"""Pruebas del permiso para transacciones en blockchain y de las herramientas resolver_sat (hunt) y calcular (SymPy).

Ningún comando de blockchain llega a ejecutarse: subprocess.Popen falla si se intenta.
"""
import importlib.util
import itertools
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from agente import conocimiento, matematicas, prompts
from agente.herramientas import CRIPTO, CRIPTO_CONFIG, ESQUEMAS, Herramientas, despachar, externo, transaccion

NO_EJECUTAR = mock.patch("agente.herramientas.subprocess.Popen", side_effect=AssertionError("no debe ejecutarse"))


class Cripto(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.casa = self.dir / "casa"
        self.proyecto = self.dir / "proyecto"
        self.proyecto.mkdir()
        entorno = {k: v for k, v in os.environ.items() if k not in ("ETH_RPC_URL", "HARDHAT_NETWORK")}
        entorno["USERPROFILE"] = str(self.casa)
        parche = mock.patch.dict(os.environ, entorno, clear=True)
        parche.start()
        self.addCleanup(parche.stop)

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def red_solana(self, url):
        config = self.casa / ".config" / "solana" / "cli" / "config.yml"
        config.parent.mkdir(parents=True, exist_ok=True)
        config.write_text(f'---\njson_rpc_url: "{url}"\nkeypair_path: C:\\cartera.json\n', encoding="utf-8")

    def test_solana_sin_url_usa_la_red_por_defecto_de_la_cli(self):
        self.red_solana("https://api.mainnet-beta.solana.com")
        self.assertIn("RED PRINCIPAL", transaccion("solana transfer DESTINO 1"))
        self.assertIn("RED PRINCIPAL", transaccion("spl-token transfer MINT 5 DESTINO"))
        self.red_solana("http://127.0.0.1:8899")
        self.assertIsNone(transaccion("solana transfer DESTINO 1"))

    def test_sin_configuracion_ni_red_se_pide_permiso(self):
        self.assertTrue(transaccion("solana program deploy target\\deploy\\prog.so").startswith(CRIPTO))
        self.assertTrue(transaccion("anchor deploy").startswith(CRIPTO))
        self.assertTrue(transaccion("cast send 0xabc 'set(uint256)' 1").startswith(CRIPTO))
        self.assertTrue(transaccion("forge script script/Deploy.s.sol --broadcast").startswith(CRIPTO))
        self.assertTrue(transaccion("sui client publish").startswith(CRIPTO))
        self.assertTrue(transaccion("aptos move publish").startswith(CRIPTO))

    def test_lecturas_y_compilacion_no_piden_permiso(self):
        self.red_solana("https://api.mainnet-beta.solana.com")
        for comando in ("solana balance", "solana config get", "spl-token balance MINT", "spl-token accounts",
                        "cast balance 0xabc", "cast call 0xabc 'x()'", "forge build", "forge test -vv",
                        "forge script script/Deploy.s.sol", "anchor build", "npx hardhat node", "npx hardhat compile",
                        "anvil", "solana-test-validator", "npx hardhat test", "npx hardhat run scripts/deploy.js"):
            self.assertIsNone(transaccion(comando), comando)

    def test_nodo_local_explicito_no_pide_permiso(self):
        self.red_solana("https://api.mainnet-beta.solana.com")
        for comando in ("solana airdrop 2 --url localhost", "solana airdrop 2 -ul", "solana transfer D 1 -u l",
                        "forge script S --broadcast --rpc-url http://127.0.0.1:8545",
                        "cast send 0xabc --rpc-url http://localhost:8545", "anchor deploy --provider.cluster localnet",
                        "npx hardhat run s.js --network localhost", "npx hardhat ignition deploy m.ts --network hardhat"):
            self.assertIsNone(transaccion(comando), comando)

    def test_redes_remotas_dicen_cual(self):
        self.assertIn("red de prueba", transaccion("solana airdrop 2 -ud"))
        self.assertIn("RED PRINCIPAL", transaccion("solana program deploy p.so -um"))
        self.assertIn("red de prueba", transaccion("forge script S --broadcast --rpc-url https://eth-sepolia.example/v2/K"))
        self.assertIn("RED PRINCIPAL", transaccion("npx hardhat run s.js --network mainnet"))
        self.assertIn("red de prueba", transaccion("anchor test --provider.cluster devnet"))
        self.assertEqual(transaccion("solana config set --url localhost"), CRIPTO_CONFIG)

    def test_anchor_y_hardhat_leen_la_red_del_proyecto(self):
        (self.proyecto / "Anchor.toml").write_text('[provider]\ncluster = "Devnet"\nwallet = "w.json"\n', encoding="utf-8")
        self.assertIn("red de prueba", transaccion("anchor test", self.proyecto))
        (self.proyecto / "Anchor.toml").write_text('[provider]\ncluster = "Localnet"\n', encoding="utf-8")
        self.assertIsNone(transaccion("anchor deploy", self.proyecto))
        self.assertIsNone(transaccion("anchor test", self.dir))  # sin Anchor.toml, test no despliega
        (self.proyecto / "hardhat.config.ts").write_text('export default { defaultNetwork: "mainnet" };', encoding="utf-8")
        self.assertIn("RED PRINCIPAL", transaccion("npx hardhat run scripts/deploy.ts", self.proyecto))

    def test_verificar_contratos_publica_fuera(self):
        self.assertTrue(externo("npx hardhat verify --network sepolia 0xabc"))
        self.assertTrue(externo("forge verify-contract 0xabc src/C.sol:C"))

    @NO_EJECUTAR
    def test_sin_permiso_se_bloquea_y_el_usuario_ve_el_motivo(self, _popen):
        vistos = []
        h = Herramientas(self.proyecto, confirmar=lambda c, m: vistos.append((c, m)) or False)
        r = h.ejecutar("solana transfer DESTINO 1 -um")
        self.assertIn("BLOQUEADO", r)
        self.assertIn("nodo local", r)
        self.assertIn("RED PRINCIPAL", vistos[0][1])

    @NO_EJECUTAR
    def test_scripts_de_package_json_tambien_se_revisan(self, _popen):
        (self.proyecto / "package.json").write_text(json.dumps({"scripts": {
            "deploy": "hardhat run scripts/deploy.js --network sepolia", "test": "hardhat test"}}), encoding="utf-8")
        vistos = []
        h = Herramientas(self.proyecto, confirmar=lambda c, m: vistos.append((c, m)) or False)
        self.assertIn("BLOQUEADO", h.ejecutar("npm run deploy"))
        self.assertIn("package.json (deploy)", vistos[0][0])
        self.assertIn("red de prueba", vistos[0][1])

    def test_entorno_avisa_si_solana_apunta_a_mainnet(self):
        self.red_solana("https://api.mainnet-beta.solana.com")
        with mock.patch.object(prompts.shutil, "which", lambda n: "solana.exe" if n == "solana" else None):
            self.assertIn("MAINNET", " ".join(prompts._blockchain()))


def palomar(palomas, nidos):
    v = lambda i, j: i * nidos + j + 1
    cl = [[v(i, j) for j in range(nidos)] for i in range(palomas)]
    cl += [[-v(a, j), -v(b, j)] for j in range(nidos) for a, b in itertools.combinations(range(palomas), 2)]
    return f"p cnf {palomas * nidos} {len(cl)}\n" + "".join(" ".join(map(str, c)) + " 0\n" for c in cl)


@unittest.skipUnless(matematicas.ruta_hunt(), "falta bin/hunt.exe")
class ResolverSat(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.h = Herramientas(self.dir)

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_satisfacible_con_modelo_comprobado(self):
        (self.dir / "f.cnf").write_text("c ejemplo\np cnf 3 4\n1 2 0\n-1 2 0\n1 -2\n0\n-1 -2 3 0\n", encoding="utf-8")
        r = despachar(self.h, "resolver_sat", {"archivo": "f.cnf"})
        self.assertIn("SATISFACIBLE", r)
        self.assertIn("Variables verdaderas: 1 2 3", r)
        self.assertTrue((self.dir / "f.modelo").read_text(encoding="utf-8").startswith("v "))
        self.assertIn("resolver_sat f.cnf -> SATISFACIBLE", self.h.evidencia_texto())

    def test_insatisfacible(self):
        (self.dir / "p.cnf").write_text(palomar(5, 4), encoding="utf-8")
        self.assertTrue(self.h.resolver_sat("p.cnf").startswith("INSATISFACIBLE (20 variables"))
        (self.dir / "q.cnf").write_text(palomar(4, 4), encoding="utf-8")
        self.assertTrue(self.h.resolver_sat("q.cnf", modelo="sol/q.txt").startswith("SATISFACIBLE"))
        self.assertTrue((self.dir / "sol" / "q.txt").is_file())

    def test_errores_de_formato_con_su_linea(self):
        casos = {"1 2 0\n": "falta la cabecera", "p cnf 2\n1 0\n": "la cabecera debe ser",
                 "p cnf 2 1\n1 x 0\n": "línea 2: 'x' no es un literal"}
        for texto, esperado in casos.items():
            (self.dir / "m.cnf").write_text(texto, encoding="utf-8")
            self.assertIn(esperado, self.h.resolver_sat("m.cnf"))
        (self.dir / "a.cnf").write_text("p cnf 2 3\n1 0\n5 0\n", encoding="utf-8")
        r = self.h.resolver_sat("a.cnf")
        self.assertIn("se usa la 5", r)
        self.assertIn("hay 2", r)
        self.assertIn("No existe", self.h.resolver_sat("no.cnf"))

    def test_nunca_da_por_bueno_un_modelo_que_no_cumple(self):
        (self.dir / "f.cnf").write_text("p cnf 2 2\n1 0\n2 0\n", encoding="utf-8")

        def falso(orden, **_):
            Path(orden[orden.index("--model") + 1]).write_text("v 1 -2 0\n", encoding="utf-8")
            return subprocess.CompletedProcess(orden, 10, "s SATISFIABLE\n", "")

        with mock.patch.object(matematicas.subprocess, "run", falso):
            self.assertIn("no cumple la cláusula 2", self.h.resolver_sat("f.cnf"))

    def test_sin_respuesta_y_sin_resolvedor(self):
        (self.dir / "f.cnf").write_text("p cnf 1 1\n1 0\n", encoding="utf-8")
        with mock.patch.object(matematicas.subprocess, "run", side_effect=subprocess.TimeoutExpired("hunt", 1)):
            self.assertIn("SIN RESPUESTA en 5 s", self.h.resolver_sat("f.cnf", segundos=5))
        with mock.patch.object(matematicas, "ruta_hunt", return_value=None):
            self.assertIn("No está el resolvedor", self.h.resolver_sat("f.cnf"))


@unittest.skipUnless(importlib.util.find_spec("sympy"), "falta SymPy")
class Calcular(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.h = Herramientas(self.dir)

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_resultados_exactos(self):
        self.assertEqual(self.h.calcular("solve(x**2 - 5*x + 6, x)"), "[2, 3]")
        self.assertEqual(self.h.calcular("integrate(sin(x)**2, (x, 0, pi))"), "pi/2")
        self.assertEqual(self.h.calcular("Rational(1, 3) + Rational(1, 6)"), "1/2")
        self.assertEqual(self.h.calcular("N(sqrt(2), 30)"), "1.41421356237309504880168872421")
        self.assertEqual(self.h.calcular("a = 6\nprint(a)\nfactorial(a)"), "6\n720")
        self.assertIn("calcular -> OK", self.h.evidencia_texto())

    def test_solo_calculo(self):
        for codigo, esperado in (("import os", "importar 'os'"), ("open('a.txt', 'w')", "usar 'open'"),
                                 ("x.__class__", "usar el atributo interno"), ("sympify('__import__(1)')", "el texto"),
                                 ("from subprocess import run", "importar 'subprocess'")):
            self.assertIn(f"RECHAZADO: {esperado}", self.h.calcular(codigo), codigo)

    def test_errores_y_limites(self):
        self.assertEqual(self.h.calcular("a = 1\nb = a / 0"), "ERROR en la línea 2: ZeroDivisionError: division by zero")
        self.assertIn("ERROR DE SINTAXIS línea 1", self.h.calcular("def f(:"))
        self.assertIn("sin salida", self.h.calcular("a = 1"))
        self.assertIn("SIN RESPUESTA en 2 s", self.h.calcular("while True:\n    pass", segundos=2))


class Integracion(unittest.TestCase):
    def test_herramientas_y_guia(self):
        nombres = {e["function"]["name"] for e in ESQUEMAS}
        self.assertTrue({"calcular", "resolver_sat"} <= nombres)
        for tema in ("matematicas", "matemáticas", "sudoku", "sat"):
            self.assertIn("resolver_sat", conocimiento.guia(tema), tema)
        self.assertIn("calcular", prompts.METODO)


if __name__ == "__main__":
    unittest.main()
