"""Leer un repositorio no ejecuta nada de lo que trae en .git (m4209): una carpeta copiada o descomprimida llega con su
configuración y sus hooks, y status, diff, log, show o blame los ejecutaban. src-tauri/src/ide.rs hace lo mismo."""
import re
import subprocess

# Órdenes que solo leen. commit, add, switch... siguen como siempre: sin su filtro se guardaría en claro lo que cifra.
LECTURAS = frozenset({
    "status", "diff", "log", "show", "blame", "annotate", "ls-files", "rev-parse", "rev-list", "merge-base", "cat-file",
    "for-each-ref", "branch", "tag", "grep", "describe", "shortlog", "reflog", "ls-tree", "diff-tree", "diff-index",
    "diff-files", "whatchanged", "range-diff", "count-objects", "name-rev", "show-ref", "symbolic-ref", "check-ignore",
    "check-attr", "var", "version", "verify-commit", "verify-tag"})
# También al escribir: una subcarpeta con forma de repositorio bare sí viaja con git clone, con su config y sus hooks.
SIEMPRE = (("safe.bareRepository", "explicit"),)
APAGADO = (("core.fsmonitor", "false"), ("log.showSignature", "false"), ("status.submoduleSummary", "false"),
           ("diff.submodule", "short"),
           ("core.hooksPath", "<sin-hooks>"))  # nombre imposible en Windows; /dev/null sería \dev\null de la unidad
SIN_RED = {"GIT_NO_LAZY_FETCH": "1", "GIT_ALLOW_PROTOCOL": "ninguno"}  # un clon parcial descarga con la orden que diga
# Programas que el repositorio puede nombrar y que no se apagan con una opción fija: hay que preguntar cuáles trae.
CONSULTA = ("config", "--show-scope", "--null", "--get-regexp", r"^(filter|lfs\.extension|gpg|diff)\.")
EJECUTABLE = re.compile(r"(filter\..*\.(clean|smudge|process|required)|lfs\.extension\..*\.(clean|smudge)|"
                        r"diff\.external|diff\..*\.(command|textconv)|gpg\.(.*\.)?program)", re.DOTALL)
# textconv: status -v, reflog -p y range-diff no obedecen --no-textconv; vacío no vale (git intenta lanzarlo y falla).
# El diff externo queda vacío: sin --ext-diff no se usa y, si alguien lo pide, la lectura falla sin ejecutar nada.
DEFECTO = {"required": "false", "textconv": "cat"}
GPG = {"gpg.program": "gpg", "gpg.openpgp.program": "gpg", "gpg.x509.program": "gpgsm", "gpg.ssh.program": "ssh-keygen"}
SIN_DRIVERS = ("--no-ext-diff", "--no-textconv")
SIN_SUBMODULOS = "--ignore-submodules=dirty"  # mirar dentro lanza git con la configuración del submódulo
BANDERAS = {"status": (SIN_SUBMODULOS,), "diff": (*SIN_DRIVERS, SIN_SUBMODULOS), "log": SIN_DRIVERS, "show": SIN_DRIVERS,
            "whatchanged": SIN_DRIVERS, "range-diff": SIN_DRIVERS, "diff-index": (SIN_SUBMODULOS,),
            "diff-files": (SIN_SUBMODULOS,), "blame": ("--no-textconv",), "annotate": ("--no-textconv",)}


def con_config(env: dict, pares) -> dict:
    """Añade configuración de git por el entorno (vale también para los git que lance git), detrás de la que ya haya."""
    try:
        n = int(env.get("GIT_CONFIG_COUNT") or 0)
    except ValueError:
        n = 0
    for clave, valor in pares:
        env[f"GIT_CONFIG_KEY_{n}"], env[f"GIT_CONFIG_VALUE_{n}"] = clave, valor
        n += 1
    env["GIT_CONFIG_COUNT"] = str(n)
    return env


def anulados(salida: str) -> list[tuple[str, str]]:
    """De la salida de CONSULTA: lo ejecutable que define el repositorio, devuelto a lo que digan el sistema y el usuario
    (su git-lfs sigue funcionando) o apagado si ellos no dicen nada."""
    trozos = salida.split("\0")
    usuario, repo = {}, []
    for ambito, dato in zip(trozos[::2], trozos[1::2]):
        clave, _, valor = dato.partition("\n")
        if ambito in ("system", "global", "command"):
            usuario[clave] = valor
        elif EJECUTABLE.fullmatch(clave):
            repo.append(clave)
    return [(c, usuario.get(c, GPG.get(c, DEFECTO.get(c.rpartition(".")[2], "")))) for c in dict.fromkeys(repo)]


def subcomando(linea: str) -> tuple[str, int, int]:
    """(orden, inicio, fin) en lo que va tras 'git', saltando -c, -C y demás opciones de delante."""
    salta = False
    for m in re.finditer(r'(?:"[^"]*"|[^\s"])+', linea):
        t = m.group().replace('"', "")
        if salta:
            salta = False
        elif t in ("-c", "-C", "--git-dir", "--work-tree", "--namespace", "--config-env", "--attr-source"):
            salta = True
        elif not t.startswith("-"):
            return t, m.start(), m.end()
    return "", len(linea), len(linea)


def preparar(resto: list[str] | str, env: dict, correr):
    """(resto, env, fallo) con lo que hay que ejecutar; correr(resto, env) lanza git. Solo cambian las lecturas. Si no se
    puede saber qué trae el repositorio, `fallo` es ese resultado y no se lee."""
    if isinstance(resto, str):
        orden, inicio, fin = subcomando(resto)
        consulta = resto[:inicio] + subprocess.list2cmdline(CONSULTA)  # con el mismo -C o --git-dir
    else:
        orden, consulta = resto[0], list(CONSULTA)
    if orden not in LECTURAS:
        return resto, env, None
    env = con_config(dict(env, **SIN_RED), APAGADO)
    r = correr(consulta, env)
    if r.returncode not in (0, 1):  # 1: no hay ninguno
        return resto, env, r
    try:
        pares = anulados(r.stdout.decode("utf-8"))
    except UnicodeDecodeError:  # un nombre que no se puede devolver tal cual no se podría apagar
        return resto, env, subprocess.CompletedProcess(consulta, 128, b"", "fatal: la configuración del repositorio no es "
                                                       "UTF-8; Aura no lo lee".encode())
    banderas = BANDERAS.get(orden, ())
    if isinstance(resto, str):
        resto = " ".join(x for x in (resto[:fin], *banderas, resto[fin:].strip()) if x)
    else:
        resto = [orden, *banderas, *resto[1:]]
    return resto, con_config(env, pares), None
