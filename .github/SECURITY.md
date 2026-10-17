# Política de seguridad

## Versiones con soporte

| Versión | Soporte |
| ------- | ------- |
| 4.2.x   | Sí      |
| < 4.2   | No      |

## Cómo reportar una vulnerabilidad

No abras un issue público. Usa el reporte privado de GitHub: pestaña **Security** del repositorio →
**Report a vulnerability**. Incluye:

- Versión de Aura Sentinel y de Windows.
- Descripción del problema y su impacto.
- Pasos para reproducirlo (y, si es posible, una prueba de concepto mínima).

Se confirmará la recepción, se investigará el caso y se te mantendrá informado hasta publicar la corrección. Si lo
deseas, se reconocerá tu aporte en el `CHANGELOG.md`.

## Alcance

Aura Sentinel funciona en local: el agente lee y escribe archivos y ejecuta comandos en la carpeta de trabajo que
elijas, con el modelo servido por LM Studio en tu equipo. Se consideran vulnerabilidades, entre otras:

- Que el agente pueda escribir o ejecutar fuera de la carpeta de trabajo sin confirmación.
- Que la interfaz abra recursos externos que no sean `localhost`.
- Que se envíen datos del usuario o de sus proyectos fuera del equipo.

No se consideran vulnerabilidades las acciones que el propio usuario confirma en el diálogo de permisos.
