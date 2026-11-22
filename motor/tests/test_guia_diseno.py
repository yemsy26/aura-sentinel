"""La guía de diseño visual: existe, es breve, se encuentra por sus alias, el método la nombra y web.md remite a ella."""
import re
import unittest

from agente import conocimiento, prompts, revision

ALIAS = ("diseño", "ux", "ui", "estilo", "estilos", "visual", "apariencia", "responsive", "movil", "móvil")


class TestGuiaDiseno(unittest.TestCase):
    def test_la_guia_dice_lo_que_importa(self):
        g = conocimiento.guia("diseno")
        for frase in ("manda sobre esta guía", "IDENTIDAD", "no del azul por defecto", "ferretería", "veterinaria",
                      "`:root`", "--marca", "--error", "var(--...)", "UN SOLO CSS", "estilo.css", "base.html",
                      "SIN EMOJIS COMO ICONOS", "ni de CDN", "sin internet la web pierde el estilo", "se copia a `static/`", "SVG en línea", "`:focus-visible`", "outline: none", "vacío",
                      "375 px", "overflow-x: auto", "tabla-scroll", "4,5:1", "ver_pantalla"):
            self.assertIn(frase, g)

    def test_es_breve(self):
        """La ventana útil del modelo es chica: web.md pesa 2,7 KB y roles 4,3 KB."""
        self.assertLessEqual(len(conocimiento.guia("diseno").encode("utf-8")), 3300)

    def test_no_pide_lo_que_web_y_roles_prohiben(self):
        g = conocimiento.guia("diseno")
        self.assertNotIn("onclick", g)
        self.assertNotIn("style=", g)
        self.assertNotIn("jQuery", g)
        for linea in g.splitlines():  # Bootstrap solo sale como lo que NO se carga de un CDN
            if "bootstrap" in linea.lower():
                self.assertIn("ni de CDN", linea)

    def test_no_ofrece_vista_movil_que_ver_pantalla_no_tiene(self):
        g = conocimiento.guia("diseno")
        self.assertIn("vista de escritorio", g)
        self.assertNotRegex(g, r"ver_pantalla[^\n]*(?:móvil|375)")

    def test_alias(self):
        base = conocimiento.guia("diseno")
        for alias in ALIAS:
            self.assertEqual(conocimiento.guia(alias), base, alias)

    def test_los_alias_nuevos_no_pisan_otros(self):
        """«tema» e «interfaz» se dejaron fuera a propósito: valen para casi cualquier cosa (una app de escritorio, un tema de aprender)."""
        for alias in ("tema", "interfaz"):
            self.assertNotEqual(conocimiento.guia(alias), conocimiento.guia("diseno"), alias)
        for alias, destino in (("login", "roles"), ("css", "web"), ("html", "web"), ("canvas", "html-juego"),
                               ("negocio", "roles"), ("flask", "api")):
            self.assertEqual(conocimiento._clave(alias), destino, alias)

    def test_el_metodo_y_web_remiten(self):
        self.assertIn("web, diseno, api,", prompts.METODO)
        self.assertIn('guia("diseno")', prompts.METODO)
        self.assertIn("sin emojis como iconos", prompts.METODO)
        self.assertIn('guia("diseno")', conocimiento.guia("web"))

    def test_el_metodo_crece_poco(self):
        """D2: la regla de una línea no pasa de 450 caracteres (el método ya ocupa buena parte de la ventana)."""
        linea = next(x for x in prompts.METODO.splitlines() if x.startswith("- Webs y paneles con pantallas"))
        self.assertLessEqual(len(linea), 450)

    def test_el_revisor_no_toma_esas_frases_como_correcciones_de_version(self):
        lineas = [x for x in conocimiento.guia("web").splitlines() if 'guia("diseno")' in x]
        self.assertTrue(lineas)
        for linea in lineas:
            self.assertIsNone(revision.CORRIGE.search(linea), linea)
        self.assertNotIn("diseno", revision.TEMAS.values())

    def test_la_guia_esta_entre_las_disponibles(self):
        self.assertIn("diseno", conocimiento.guia("noexiste-xyz"))
        self.assertTrue(re.search(r"Disponibles: .*\bdiseno\b", conocimiento.guia("noexiste-xyz")))


if __name__ == "__main__":
    unittest.main()
