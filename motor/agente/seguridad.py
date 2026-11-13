"""Dos cosas de una web entregada que se ven leyendo el código (sin ejecutarlo) y que ponen en riesgo los datos de quien
la usa. Salieron de la entrega de un negocio que funcionaba (la prueba del usuario con la 4.13.4):
- los tests borran la base de datos real de la app (un os.remove en setUp): quien pulsa test.bat pierde todo lo que guardó;
- una ruta de registro sin entrar, en un sistema con roles: cualquiera se crea una cuenta y ve lo de dentro.
(app.run(host="0.0.0.0", debug=True) se midió y no se avisa: sale en el 37 % de las entregas guardadas, es un hábito del modelo.)
Para no dar avisos falsos, solo se marca lo que se ve claro: la ruta de lo borrado tiene que resolverse a un archivo de base
de datos con el mismo nombre que usa el código de la app (lo que sale de tempfile no cuenta)."""
import ast
import re
from pathlib import Path

from .proyecto import Informe, relativa

ES_BASE = re.compile(r"(?i)\.(?:db|sqlite3?)$")
NOMBRE_BASE = re.compile(r"(?i)[\w.\-]+\.(?:db|sqlite3?)\b")
TEMPORALES = {"mkdtemp", "TemporaryDirectory", "NamedTemporaryFile", "gettempdir", "mktemp"}
SIN_CAMBIO = {"abspath", "realpath", "normpath", "Path", "PurePath", "str", "fspath", "resolve", "absolute", "expanduser"}
RUTA_REGISTRO = re.compile(r"(?i)(?:^|/)(?:register|registro|registrar|registrarse|signup|sign-up|crear[-_]?cuenta|nueva[-_]?cuenta)/?$")
PIDE_ENTRAR = re.compile(r"(?i)login|requer|auth|admin|permiso|permission|\brol|role|protect|jwt|restrict")
CREA_USUARIO = re.compile(r"(?i)INSERT\s+INTO\s+(?:usuarios?|users?)\b")
# Crear la cuenta dentro de un «if» que mira quién es (o cuántos usuarios hay: solo el primero) no deja entrar a cualquiera.
GUARDA = re.compile(r"(?i)is_authenticated|current_user|session|\brol\b|role|admin|\bg\.user|authorization|total|count|cnt|"
                    r"primer|first|\blen\(")
HAY_ROLES = re.compile(r"(?i)\b(?:rol|roles|role)\b")


def _punto(n) -> str | None:
    if isinstance(n, ast.Name):
        return n.id
    if isinstance(n, ast.Attribute) and (base := _punto(n.value)):
        return f"{base}.{n.attr}"
    return None


def _fin(func) -> str:
    return func.id if isinstance(func, ast.Name) else func.attr if isinstance(func, ast.Attribute) else ""


def _ruta(n, env: dict[str, str]) -> str | None:
    """El texto de una ruta, con «?» donde no se sabe y «<temp>» si sale de tempfile. None si no se puede leer."""
    if isinstance(n, ast.Constant) and isinstance(n.value, str):
        return n.value
    if isinstance(n, ast.Name):
        return env.get(n.id)
    if isinstance(n, ast.Attribute):
        return env.get(_punto(n) or "")
    if isinstance(n, ast.JoinedStr):
        return "".join(p.value if isinstance(p, ast.Constant) else "?" for p in n.values)
    if isinstance(n, ast.BinOp) and isinstance(n.op, ast.Div | ast.Add):
        return "/".join((_ruta(x, env) or "?") for x in (n.left, n.right))
    if isinstance(n, ast.Call):
        nombre = _fin(n.func)
        if nombre in TEMPORALES:
            return "<temp>"
        if nombre == "join" or (nombre in SIN_CAMBIO and len(n.args) > 1):
            return "/".join((_ruta(a, env) or "?") for a in n.args)
        if nombre in SIN_CAMBIO and n.args:
            return _ruta(n.args[0], env)
        if nombre == "dirname":
            return "?"
    return None


def _asignaciones(arbol: ast.AST) -> dict[str, str]:
    env: dict[str, str] = {}
    pasos = sorted((n for n in ast.walk(arbol) if isinstance(n, ast.Assign)), key=lambda n: n.lineno)
    for n in pasos:
        valor = _ruta(n.value, env)
        if valor is None:
            continue
        for t in n.targets:
            if (nombre := _punto(t)):
                env[nombre] = valor
    return env


def _bases_de_la_app(codigos: list[str]) -> set[str]:
    return {m.group(0).lower() for c in codigos for m in NOMBRE_BASE.finditer(c)}


def _es_test(p: Path) -> bool:
    return bool(re.match(r"(?i)test", p.name)) or p.name.lower().endswith("_test.py") or any(
        parte.lower() in ("tests", "test") for parte in p.parts[:-1])


def pruebas_borran_la_base(raiz: Path, pys: list[Path], inf: Informe) -> None:
    """Error si un test borra el archivo de la base de datos que usa la app."""
    tests = [p for p in pys if _es_test(p)]
    if not tests:
        return
    app = [p.read_text(encoding="utf-8", errors="replace") for p in pys if not _es_test(p)]
    usadas = _bases_de_la_app(app)
    if not usadas:
        return
    for t in tests:
        texto = t.read_text(encoding="utf-8", errors="replace")
        try:
            arbol = ast.parse(texto)
        except (SyntaxError, ValueError):
            continue
        env = _asignaciones(arbol)
        for n in sorted((x for x in ast.walk(arbol) if isinstance(x, ast.Call)), key=lambda x: x.lineno):
            nombre, objetivo = _fin(n.func), None
            if nombre in ("remove", "unlink") and isinstance(n.func, ast.Attribute) and _punto(n.func.value) == "os" and n.args:
                objetivo = n.args[0]
            elif nombre == "unlink" and isinstance(n.func, ast.Attribute):
                objetivo = n.func.value
            elif nombre == "remove" and isinstance(n.func, ast.Name) and n.args:
                objetivo = n.args[0]
            ruta = _ruta(objetivo, env) if objetivo is not None else None
            if not ruta or "<temp>" in ruta or not ES_BASE.search(ruta):
                continue
            if not re.search(r"[\\/]", ruta) and "os.chdir" in texto:  # un nombre suelto, tras cambiar de carpeta: no se sabe cuál
                continue
            archivo = re.split(r"[\\/]", ruta)[-1]
            if archivo.lower() in usadas:
                inf.errores.append(
                    f"{relativa(t, raiz)}:{n.lineno} borra {archivo}, la base de datos real que usa la aplicación: quien ejecute "
                    "los tests (test.bat, un doble clic) pierde todo lo que haya guardado. Haz que los tests usen una base propia "
                    "(un archivo temporal con tempfile, o \":memory:\") y no toquen la de la aplicación.")
                return


def _rutas_de_registro(arbol: ast.AST):
    """(función, ruta) de cada ruta de registro sin ningún decorador que pida entrar."""
    for f in ast.walk(arbol):
        if not isinstance(f, ast.FunctionDef | ast.AsyncFunctionDef):
            continue
        camino, otros = None, []
        for d in f.decorator_list:
            if (isinstance(d, ast.Call) and _fin(d.func) in ("route", "get", "post") and d.args
                    and isinstance(d.args[0], ast.Constant) and isinstance(d.args[0].value, str)):
                camino = camino or d.args[0].value
            else:
                otros.append(ast.unparse(d))
        if camino and RUTA_REGISTRO.search(camino) and not any(PIDE_ENTRAR.search(o) for o in otros):
            yield f, camino


def _crea_sin_guarda(f: ast.AST) -> bool:
    """¿Crea el usuario (hash de la contraseña o INSERT) fuera de cualquier «if» que mire quién es?"""
    padre = {c: n for n in ast.walk(f) for c in ast.iter_child_nodes(n)}
    for n in ast.walk(f):
        if not (isinstance(n, ast.Call) and (_fin(n.func) == "generate_password_hash" or (
                _fin(n.func) in ("execute", "executemany") and n.args and isinstance(n.args[0], ast.Constant)
                and isinstance(n.args[0].value, str) and CREA_USUARIO.search(n.args[0].value)))):
            continue
        guardado, arriba = False, padre.get(n)
        while arriba is not None and arriba is not f:
            guardado |= isinstance(arriba, ast.If) and bool(GUARDA.search(ast.unparse(arriba.test)))
            arriba = padre.get(arriba)
        if not guardado:
            return True
    return False


def seguridad_web(raiz: Path, pys: list[Path], inf: Informe) -> None:
    """Aviso: registro abierto a cualquiera en un sistema con roles."""
    codigos = {p: p.read_text(encoding="utf-8", errors="replace") for p in pys if not _es_test(p)}
    if not any(HAY_ROLES.search(c) for c in codigos.values()):
        return
    for p, codigo in codigos.items():
        try:
            arbol = ast.parse(codigo)
        except (SyntaxError, ValueError):
            continue
        for f, camino in _rutas_de_registro(arbol):
            if _crea_sin_guarda(f):
                inf.avisos.append(f"{relativa(p, raiz)}:{f.lineno} la ruta {camino} deja que cualquiera se cree una cuenta sin "
                                  "entrar, y el sistema tiene roles: si es para tu equipo, que solo el administrador cree "
                                  "usuarios (o quita el registro).")
                return
