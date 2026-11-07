"""Código Python que borra fuera de lo suyo: la carpeta temporal del sistema entera, la del usuario, una del sistema o
el propio proyecto. Un test generado hacía shutil.rmtree(os.path.dirname(self.tmp)) en tearDown, con
self.tmp = tempfile.mkdtemp(), y cada vez que corrían los tests borraba sin papelera todo el %TEMP% del usuario.

Se mira el código sin ejecutarlo. Para no dar avisos falsos, un nombre solo cuenta como peligroso si TODAS sus
asignaciones en su ámbito lo son (self.x: en toda la clase), y lo que va debajo de una carpeta (join, /) no cuenta."""
import ast
import re

QUE = {"temporal": "la carpeta temporal del sistema entera (%TEMP%, con archivos de otros programas)",
       "usuario": "la carpeta del usuario", "sistema": "una carpeta del sistema",
       "proyecto": "la carpeta del proyecto entera (o una que lo contiene)"}
VARIABLES = {"TEMP": "temporal", "TMP": "temporal", "TMPDIR": "temporal", "USERPROFILE": "usuario", "HOME": "usuario",
             "HOMEPATH": "usuario", "APPDATA": "usuario", "LOCALAPPDATA": "usuario", "PROGRAMDATA": "sistema",
             "SYSTEMROOT": "sistema", "WINDIR": "sistema", "PROGRAMFILES": "sistema"}
RAIZ = re.compile(r"(?i)^(?:[a-z]:[\\/]?|[\\/]|[a-z]:[\\/](?:users|windows|program files[^\\/]*)(?:[\\/][^\\/]+)?[\\/]?)$")
USUARIO = re.compile(r"^~[\\/]?$")
AQUI = {".", "./", ".\\", "..", "../", "..\\"}  # la carpeta desde la que corre (la del proyecto) o la de encima
CREAN = {"mkdtemp", "TemporaryDirectory", "NamedTemporaryFile", "SpooledTemporaryFile", "TemporaryFile"}
# Envuelven una ruta sin cambiar a qué carpeta apunta (con un solo argumento: Path(a, b) ya es una subcarpeta).
IGUALES = {"abspath", "realpath", "normpath", "Path", "PurePath", "WindowsPath", "str", "fspath", "resolve", "absolute",
           "expanduser"}
SUBIR = {"..", "../", "..\\"}
PADRE = {"creado": "temporal", "archivo": "proyecto"}  # dirname de un temporal es %TEMP%; el de __file__, el proyecto


def _punto(n) -> str | None:
    if isinstance(n, ast.Name):
        return n.id
    if isinstance(n, ast.Attribute) and (base := _punto(n.value)):
        return f"{base}.{n.attr}"
    return None


def _fin(func) -> str:
    return func.id if isinstance(func, ast.Name) else func.attr if isinstance(func, ast.Attribute) else ""


def _texto(n) -> str | None:
    return n.value if isinstance(n, ast.Constant) and isinstance(n.value, str) else None


def _padre(tipo: str | None) -> str | None:
    return PADRE.get(tipo, tipo) if tipo else None


def _nodos(cuerpo):
    """Los nodos de estas sentencias sin entrar en funciones, clases ni lambdas (que son otro ámbito)."""
    pila = list(cuerpo)
    while pila:
        n = pila.pop()
        yield n
        if not isinstance(n, ast.FunctionDef | ast.AsyncFunctionDef | ast.ClassDef | ast.Lambda):
            pila.extend(ast.iter_child_nodes(n))


class _Ambito:
    def __init__(self, padre, clase: dict | None):
        self.padre, self.clase, self.asig = padre, clase, {}


class _Analisis:
    def __init__(self, arbol: ast.Module):
        self.llamadas, self.memo = [], {}
        self._recoger(arbol.body, _Ambito(None, None))

    def _recoger(self, cuerpo, amb: _Ambito) -> None:
        for n in _nodos(cuerpo):
            if isinstance(n, ast.ClassDef):
                self._recoger(n.body, _Ambito(amb, {}))
            elif isinstance(n, ast.FunctionDef | ast.AsyncFunctionDef):
                hijo = _Ambito(amb, amb.clase)
                a = n.args
                for arg in [*a.posonlyargs, *a.args, *a.kwonlyargs, a.vararg, a.kwarg]:
                    if arg:
                        hijo.asig.setdefault(arg.arg, []).append((None, hijo))
                self._recoger(n.body, hijo)
            elif isinstance(n, ast.Assign):
                for t in n.targets:
                    self._apuntar(t, n.value, amb)
            elif isinstance(n, ast.AnnAssign | ast.NamedExpr) and n.value is not None:
                self._apuntar(n.target, n.value, amb)
            elif isinstance(n, ast.AugAssign | ast.For | ast.AsyncFor | ast.comprehension):
                self._apuntar(n.target, None, amb)
            elif isinstance(n, ast.withitem) and n.optional_vars is not None:
                self._apuntar(n.optional_vars, n.context_expr, amb)
            elif isinstance(n, ast.Call):
                self.llamadas.append((n, amb))

    def _apuntar(self, destino, valor, amb: _Ambito) -> None:
        if isinstance(destino, ast.Tuple | ast.List):
            crea = isinstance(valor, ast.Call) and _fin(valor.func) == "mkstemp"  # fd, ruta = mkstemp()
            for i, d in enumerate(destino.elts):
                self._apuntar(d, "creado" if crea and i == 1 else None, amb)
            return
        if (nombre := _punto(destino)) is None:
            return
        tabla = amb.clase if nombre.startswith("self.") and amb.clase is not None else amb.asig
        tabla.setdefault(nombre, []).append((valor, amb))

    def _nombre(self, nombre: str, amb: _Ambito) -> str | None:
        if nombre == "__file__":
            return "archivo"
        if nombre.startswith("self.") and amb.clase is not None:
            valores = amb.clase.get(nombre, [])
        else:
            while amb is not None and nombre not in amb.asig:
                amb = amb.padre
            valores = amb.asig[nombre] if amb is not None else []
        if not valores:
            return None
        clave = (nombre, id(valores))
        if clave not in self.memo:
            self.memo[clave] = None  # x = dirname(x): sin bucles
            tipos = {v if isinstance(v, str) else self.tipo(v, a) for v, a in valores}
            self.memo[clave] = tipos.pop() if len(tipos) == 1 else None
        return self.memo[clave]

    def tipo(self, n, amb: _Ambito) -> str | None:
        """Qué es la carpeta a la que apunta la expresión: creado (un temporal hecho por el código), archivo
        (__file__), temporal, usuario, sistema, proyecto o None (otra cosa: lo normal)."""
        if n is None:
            return None
        if (t := _texto(n)) is not None:
            return "usuario" if USUARIO.match(t) else "sistema" if RAIZ.match(t) else "proyecto" if t in AQUI else None
        if isinstance(n, ast.Subscript):
            if isinstance(n.value, ast.Call) and _fin(n.value.func) == "mkstemp":
                return "creado"
            if _punto(n.value) == "os.environ" and (t := _texto(n.slice)):
                return VARIABLES.get(t.upper())
            return None
        if isinstance(n, ast.Attribute):
            if n.attr == "parent":
                return _padre(self.tipo(n.value, amb))
            if n.attr == "name":  # TemporaryDirectory().name, NamedTemporaryFile().name
                return "creado" if self.tipo(n.value, amb) == "creado" else None
            if _punto(n) == "tempfile.tempdir":
                return "temporal"
        if isinstance(n, ast.Name | ast.Attribute):
            return self._nombre(nombre, amb) if (nombre := _punto(n)) else None
        if not isinstance(n, ast.Call):
            return None
        fin, args = _fin(n.func), n.args
        receptor = n.func.value if isinstance(n.func, ast.Attribute) else None
        if fin in ("gettempdir", "gettempdirb"):
            return "temporal"
        if fin in CREAN:
            return "creado"
        if fin == "home" and (_punto(n.func) or "").endswith("Path.home"):
            return "usuario"
        if fin == "getenv" or (fin == "get" and _punto(receptor) == "os.environ"):
            return VARIABLES.get((_texto(args[0]) or "").upper()) if args else None
        if fin == "dirname" and len(args) == 1:
            return _padre(self.tipo(args[0], amb))
        if fin == "join" and len(args) > 1 and all(_texto(a) in SUBIR for a in args[1:]):  # join(tmp, "..")
            tipo = self.tipo(args[0], amb)
            for _ in args[1:]:
                tipo = _padre(tipo)
            return tipo
        if fin in IGUALES:
            if len(args) == 1:
                return self.tipo(args[0], amb)
            return self.tipo(receptor, amb) if not args and receptor is not None and fin != "Path" else None
        return None

    def borrados(self):
        for n, amb in self.llamadas:
            fin, f = _fin(n.func), n.func
            if fin == "rmtree" or (fin in ("remove", "unlink", "rmdir", "removedirs") and isinstance(f, ast.Attribute)
                                   and _punto(f.value) == "os"):
                objetivo = n.args[0] if n.args else next((k.value for k in n.keywords if k.arg == "path"), None)
            elif fin in ("unlink", "rmdir") and isinstance(f, ast.Attribute) and not n.args:  # Path(...).unlink()
                objetivo = f.value
            else:
                continue
            if (tipo := self.tipo(objetivo, amb)) in QUE:
                yield n.lineno, ast.unparse(n)[:120], QUE[tipo]


def aviso(donde: str, linea: int, llamada: str, que: str) -> str:
    return (f"{donde}:{linea} BORRA FUERA DE LO SUYO: {llamada} borra {que}, sin papelera. Borra solo lo que creó el "
            "programa o el test: la carpeta que devolvió mkdtemp (nunca su dirname ni su .parent), mejor con "
            "tempfile.TemporaryDirectory() o self.addCleanup(shutil.rmtree, carpeta).")


def peligrosos(codigo: str) -> list[tuple[int, str, str]]:
    """(línea, llamada, qué borra) de cada borrado fuera de lo suyo en este código Python."""
    try:
        arbol = ast.parse(codigo)
    except (SyntaxError, ValueError):
        return []
    return sorted(_Analisis(arbol).borrados())
