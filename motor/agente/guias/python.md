# Python moderno (3.12+, 2026)
- Entorno: `python -m venv .venv` y `python -m pip install -r requirements.txt` (ejecutar ya usa el .venv). Antes de añadir una dependencia: `version("paquete")` para ver la última versión y si tiene wheel para tu Python. Si no tiene wheel para Python 3.14, busca una alternativa o crea el venv con `py -3.12 -m venv .venv`.
- Sustitutos actuales: pygame -> pygame-ce (pygame no tiene wheel para 3.14); PyPDF2 -> pypdf; fuzzywuzzy -> rapidfuzz; requests sigue bien (httpx si necesitas async); os.path -> pathlib; setup.py -> pyproject.toml.
- Estilo: f-strings, `pathlib.Path`, `dataclasses`, anotaciones de tipo, `logging` en vez de prints de depuración, `argparse` para CLI, `with open(..., encoding="utf-8")`.
- Nunca `except:` vacío: captura la excepción concreta y muestra un mensaje útil.
- Punto de entrada: `def main() -> None: ...` y `if __name__ == "__main__": main()`. Separa lógica (funciones puras, testeables) de entrada/salida.
- Interfaz gráfica simple: tkinter (incluido). Web: Flask o FastAPI (ver guia("api")). Juegos: guia("pygame").
- Tests: pytest (`python -m pytest -q`), archivos `test_*.py` que IMPORTAN tu código y comprueban comportamiento real (ver guia("tests")).
- Programas que piden datos (`input()`): estructura la lógica en funciones y pruébala con tests; para probar el programa entero usa `echo 5| python main.py`.
- `verificar` ejecuta ruff (errores reales: nombres sin definir, variables sin usar), vulture (funciones que nunca se llaman = funciones sin conectar), los tests y el programa.
