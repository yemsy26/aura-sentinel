"""Agente Local: agente de programación para modelos locales (LM Studio / Bionic)."""
from .entorno import limpiar_proceso

__version__ = "1.0.0"
limpiar_proceso()  # antes que cualquier módulo copie os.environ para los comandos del proyecto
