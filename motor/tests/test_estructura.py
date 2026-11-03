"""La arquitectura del motor (ver ARQUITECTURA.md) comprobada: las conexiones entre módulos se pueden seguir leyendo la
cabecera de cada archivo, sin ciclos ni nombres que llegan de rebote."""
import ast
import unittest
from pathlib import Path

AGENTE = Path(__file__).resolve().parent.parent / "agente"
MODULOS = {p.stem: ast.parse(p.read_text(encoding="utf-8")) for p in AGENTE.glob("*.py")}
# Dentro de una función a propósito: comandos sueltos que no deben cargar lo demás al arrancar.
PEREZOSAS = {("__main__", "prueba_modelo"), ("instalar", "descargar")}


def importaciones(arbol):
    """(módulo destino, nombre importado o None, ¿dentro de una función?) de cada import relativo."""
    def recorrer(nodo, dentro):
        for hijo in ast.iter_child_nodes(nodo):
            if isinstance(hijo, ast.ImportFrom) and hijo.level == 1:
                for a in hijo.names:
                    if hijo.module:
                        yield hijo.module, a.name, dentro
                    elif a.name in MODULOS:
                        yield a.name, None, dentro
            yield from recorrer(hijo, dentro or isinstance(hijo, ast.FunctionDef | ast.AsyncFunctionDef))
    return recorrer(arbol, False)


def definidos(arbol) -> set[str]:
    nombres = set()
    for n in arbol.body:
        if isinstance(n, ast.FunctionDef | ast.AsyncFunctionDef | ast.ClassDef):
            nombres.add(n.name)
        elif isinstance(n, ast.Assign | ast.AnnAssign):
            for t in n.targets if isinstance(n, ast.Assign) else [n.target]:
                nombres |= {x.id for x in ast.walk(t) if isinstance(x, ast.Name)}
    return nombres


class TestEstructura(unittest.TestCase):
    def test_sin_ciclos_de_importacion(self):
        grafo = {m: {d for d, _, _ in importaciones(a)} for m, a in MODULOS.items()}
        estado = {}

        def visitar(m, camino):
            if estado.get(m) == "en curso":
                self.fail("Ciclo de importación: " + " -> ".join([*camino, m]))
            if m not in estado:
                estado[m] = "en curso"
                for d in sorted(grafo.get(m, ())):
                    visitar(d, [*camino, m])
                estado[m] = "hecho"

        for m in grafo:
            visitar(m, [])

    def test_las_importaciones_estan_en_la_cabecera(self):
        ocultas = {(m, d) for m, a in MODULOS.items() for d, _, dentro in importaciones(a) if dentro}
        self.assertEqual(ocultas - PEREZOSAS, set())

    def test_cada_nombre_se_importa_de_donde_vive(self):
        for m, a in MODULOS.items():
            for d, nombre, _ in importaciones(a):
                if nombre and d in MODULOS:
                    self.assertIn(nombre, definidos(MODULOS[d]), f"{m} importa {nombre} de {d}, que no lo define")

    def test_nada_privado_cruza_de_modulo(self):
        for m, a in MODULOS.items():
            for d, nombre, _ in importaciones(a):
                self.assertFalse(nombre and nombre.startswith("_"), f"{m} usa {d}.{nombre}: si se comparte, no es privado")

    def test_la_base_no_depende_del_resto(self):
        for base, permitidos in (("proyecto", {"procesos"}), ("permisos", set())):
            self.assertLessEqual({d for d, _, _ in importaciones(MODULOS[base])}, permitidos, base)


if __name__ == "__main__":
    unittest.main()
