"""La guía de roles: existe, se encuentra por sus alias, el método la nombra, api.md/web.md remiten a ella y trae la lista por pantalla."""
import unittest

from agente import conocimiento, prompts, revision


class TestGuiaRoles(unittest.TestCase):
    def test_la_guia_dice_lo_que_importa(self):
        g = conocimiento.guia("roles")
        for frase in ("requiere_rol", "login_required", "vendedor_id", "costo", "login_user", "/register", "403"):
            self.assertIn(frase, g)

    def test_lista_por_pantalla_y_prueba_de_marcadores(self):
        g = conocimiento.guia("roles")
        for frase in ("LISTA POR PANTALLA", "Panel de inicio", "Listas (ventas, facturas", "Detalles", "no edita ni borra",
                      "data-*", "generate_password_hash(password)", "PRUEBA DE MARCADORES", "7.77", "NOTA-ADMIN-77",
                      "iter_rules()", "crear_app(ruta_db)", "POST directo"):
            self.assertIn(frase, g)

    def test_el_marcador_no_es_un_cliente_y_se_prohibe_filtrar_por_el_marcador(self):
        """m4140: con «Cliente Secreto» como marcador, una misión escondió en la app los clientes con «secreto» en el nombre."""
        g = conocimiento.guia("roles")
        self.assertNotIn("Cliente Secreto", g)
        self.assertIn("el nombre de un cliente no sirve de marcador", g)
        self.assertIn("no filtres por el texto del marcador", g)

    def test_alias(self):
        base = conocimiento.guia("roles")
        for alias in ("permisos", "empleados", "login", "flask-login", "autorización"):
            self.assertEqual(conocimiento.guia(alias), base, alias)

    def test_el_metodo_y_las_guias_de_web_remiten(self):
        self.assertIn("roles)", prompts.METODO)
        self.assertIn('guia("roles")', prompts.METODO)
        self.assertIn("marcadores", prompts.METODO)
        for tema in ("api", "web"):
            self.assertIn('guia("roles")', conocimiento.guia(tema), tema)
            self.assertIn("marcadores", conocimiento.guia(tema), tema)

    def test_el_revisor_no_toma_esas_frases_como_correcciones_de_version(self):
        for tema in ("api", "web"):
            lineas = [x for x in conocimiento.guia(tema).splitlines() if 'guia("roles")' in x]
            self.assertTrue(lineas)
            for linea in lineas:
                self.assertIsNone(revision.CORRIGE.search(linea), linea)
        self.assertNotIn("roles", revision.TEMAS.values())


if __name__ == "__main__":
    unittest.main()
