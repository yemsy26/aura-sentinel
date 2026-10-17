"""Herramientas del agente. Devuelven texto compacto (barato en tokens y fácil de leer para modelos pequeños)."""
import base64
import html.parser
import json
import os
import re
import subprocess
import tempfile
import time
import urllib.parse
import urllib.request
from pathlib import Path

from . import conocimiento, pantalla
from .config import RAIZ_INSTALACION
from .verificacion import chequeo_rapido, verificar

VACIAS = {"como", "para", "que", "con", "los", "las", "una", "uno", "del", "por", "the", "and", "for", "with", "how",
          "what", "este", "esta", "esto", "hay", "hacer", "puedo", "cual", "cuál", "cómo", "qué", "use", "using"}
LECCIONES = RAIZ_INSTALACION / "lecciones.md"
CODIGO = {".py", ".js", ".mjs", ".ts", ".html", ".htm", ".css", ".java", ".bat", ".cmd", ".json"}

IGNORAR = {".git", ".venv", "venv", "env", "node_modules", "__pycache__", "dist", "build", ".next", ".agente",
           "target", ".idea", ".vscode", ".pytest_cache", ".mypy_cache", ".ruff_cache", "site-packages"}
# Git que pierde trabajo o reescribe historia (o publica: push).
GIT_DESTRUCTIVO = (r"push|reset\s+--hard|clean\s+-\w*f|checkout\s+(-f\b|--\s|\.(\s|$))|restore\b(?!.*--staged)|"
                   r"branch\s+-D|stash\s+(drop|clear)|rebase|filter-branch|update-ref\s+-d|reflog\s+expire|rm\b|"
                   r"commit\b.*--amend|switch\b.*(\s-f\b|--discard-changes)")
PELIGROSOS = re.compile(
    r"\b(format\s+[a-z]:|diskpart|shutdown|bcdedit|cipher\s+/w|takeown|icacls|reg\s+(delete|add)|"
    rf"rd\s+/s|rmdir\s+/s|del\s+.*/[sq]|remove-item\b.*-recurse|rm\s+-r|git\s+({GIT_DESTRUCTIVO}))",
    re.IGNORECASE)
# Nunca se versionan (aunque falte .gitignore): estado del agente, entornos y cachés.
NO_VERSIONAR = ["--", ".", ":(exclude).agente"] + [f":(exclude,glob)**/{d}/**" for d in (
    ".venv", "venv", "node_modules", "__pycache__", ".pytest_cache", ".mypy_cache", ".ruff_cache")]
INSTALACIONES = re.compile(  # descargan e instalan: con poco tiempo se cortan a medias y dejan el entorno roto
    r"\b(venv|virtualenv|pip3?(\.exe)?\s+install|npm\s+(install|ci|i)\b|uv\s+(pip|venv|sync|add)|playwright\s+install)",
    re.IGNORECASE)
MIN_INSTALACION = 300  # segundos
# Archivos que se nombran en un texto (petición del usuario o paso del plan); no las librerías como three.js.
ARCHIVO_PEDIDO = r"\b[\w\-]+\.(?:py|bat|cmd|ps1|txt|json|md|js|ts|jsx|tsx|html|css|toml|ya?ml|ini|cfg|sql|csv|java|cs|go|rs|cpp|c|h)\b"
NO_ARCHIVO = re.compile(r"(node|next|nuxt|vue|react|three|d3|express|chart|p5|phaser|pixi|babylon|anime|alpine|ember|"
                        r"socket|solid|svelte)\.js$", re.IGNORECASE)
CREAR = re.compile(r"\b(crea|escrib|implement|añad|agreg|genera|construi)", re.IGNORECASE)


def archivos_pedidos(texto: str) -> set[str]:
    return {f for f in re.findall(ARCHIVO_PEDIDO, texto, re.IGNORECASE) if not NO_ARCHIVO.match(f)}


PLACEHOLDERS = re.compile(r"(\.\.\.\s*(resto|rest of|existing|código anterior|mismo)|# *(TODO|implementar aquí)|"
                          r"<\s*(tu|your)[ _]\w+\s*>)", re.IGNORECASE)
UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130 Safari/537.36"


def recortar(texto: str, maximo: int) -> str:
    """Conserva inicio y (sobre todo) final: los errores suelen estar al final."""
    if len(texto) <= maximo:
        return texto
    cab = maximo * 2 // 5
    return f"{texto[:cab]}\n[... {len(texto) - maximo} caracteres omitidos ...]\n{texto[-(maximo - cab):]}"


def decodificar(datos: bytes) -> str:
    try:
        return datos.decode("utf-8")
    except UnicodeDecodeError:
        return datos.decode("cp850", "replace")  # consola de Windows en español


def matar_arbol(pid: int) -> None:
    subprocess.run(["taskkill", "/T", "/F", "/PID", str(pid)], capture_output=True, check=False)


class _HtmlATexto(html.parser.HTMLParser):
    OMITIR = frozenset({"script", "style", "noscript", "svg", "nav", "footer", "header", "form", "iframe"})
    BLOQUE = frozenset({"p", "div", "br", "li", "tr", "h1", "h2", "h3", "h4", "h5", "pre", "section", "article",
                        "table"})

    def __init__(self):
        super().__init__()
        self.partes, self.omitir = [], 0

    def handle_starttag(self, tag, attrs):
        if tag in self.OMITIR:
            self.omitir += 1
        elif tag in self.BLOQUE:
            self.partes.append("\n")

    def handle_endtag(self, tag):
        if tag in self.OMITIR and self.omitir:
            self.omitir -= 1

    def handle_data(self, data):
        if not self.omitir:
            self.partes.append(data)

    def texto(self) -> str:
        t = re.sub(r"[ \t\r\f\v]+", " ", "".join(self.partes))
        return re.sub(r"\n\s*\n+", "\n\n", t).strip()


class Herramientas:
    def __init__(self, raiz: str | Path, max_chars: int = 6000, timeout: int = 120, confirmar=None):
        """confirmar(comando) -> bool. None = rechazar comandos peligrosos."""
        self.raiz = Path(raiz).expanduser().resolve()
        self.raiz.mkdir(parents=True, exist_ok=True)
        self.dir_agente = self.raiz / ".agente"
        self.max_chars, self.timeout, self.confirmar = max_chars, timeout, confirmar
        self.procesos: dict[int, dict] = {}
        self.paso = 0               # lo actualiza el bucle
        self.paso_escritura = -1    # último paso que modificó archivos
        self.paso_prueba = -1       # último paso que ejecutó algo con éxito tras escribir
        self.verificacion = {"paso": -1, "errores": -1, "informe": ""}
        self.revisar_cb = None      # lo conecta el bucle o el servidor MCP (necesita el modelo)
        self.cfg_llm = None         # config del agente (servidor/modelo) para ver_pantalla; None = agente.json
        self.reescrituras: dict[str, int] = {}
        self.inicio_tarea = 0
        self.checkpoints: list[dict] = []   # estados que pasaron verificar sin errores
        self.fallos_desde_verde = 0

    def nueva_tarea(self, paso_inicio: int = 0) -> None:
        self.reescrituras.clear()
        self.inicio_tarea = paso_inicio

    def python_proyecto(self) -> str:
        venv = self.raiz / ".venv" / "Scripts" / "python.exe"
        return str(venv) if venv.exists() else "python"

    # ---------- utilidades ----------
    def ruta(self, r: str) -> Path:
        p = Path(r.strip().strip('"')) if r else Path(".")
        p = (p if p.is_absolute() else self.raiz / p).resolve()
        if p != self.raiz and self.raiz not in p.parents:
            raise ValueError(f"Ruta fuera del proyecto: {r}. Usa rutas relativas a {self.raiz}")
        return p

    def rel(self, p: Path) -> str:
        return str(p.relative_to(self.raiz)) if p != self.raiz else "."

    def _validar(self, p: Path, contenido: str) -> str:
        avisos = []
        suf = p.suffix.lower()
        try:
            if suf == ".py":
                compile(contenido, str(p), "exec")
            elif suf == ".json":
                json.loads(contenido)
        except SyntaxError as e:
            avisos.append(f"ERROR DE SINTAXIS línea {e.lineno}: {e.msg}. Corrígelo con editar.")
        except ValueError as e:
            avisos.append(f"JSON INVÁLIDO: {e}. Corrígelo.")
        if PLACEHOLDERS.search(contenido):
            avisos.append("AVISO: parece haber marcadores incompletos (TODO/'...resto'). Escribe el código completo.")
        if not avisos:
            try:
                avisos.append(chequeo_rapido(self.raiz, p).strip())  # lint inmediato, como los agentes avanzados
            except Exception:
                pass
        avisos = [a for a in avisos if a]
        return (" " + "\n".join(avisos)) if avisos else " Sintaxis OK." if suf in (".py", ".json") else ""

    @staticmethod
    def _compila(p: Path, texto: str) -> bool:
        if p.suffix.lower() != ".py":
            return p.suffix.lower() in CODIGO and len(texto.strip()) > 0
        try:
            compile(texto, str(p), "exec")
            return True
        except SyntaxError:
            return False

    def _guardar(self, p: Path, contenido: str) -> None:
        p.parent.mkdir(parents=True, exist_ok=True)
        if p.suffix.lower() in (".bat", ".cmd"):
            contenido = contenido.replace("\r\n", "\n").replace("\n", "\r\n")
        fd, tmp = tempfile.mkstemp(prefix=f".{p.name}.", suffix=".tmp", dir=p.parent)
        try:
            with os.fdopen(fd, "w", encoding="utf-8", newline="") as f:
                f.write(contenido)
            os.replace(tmp, p)
        finally:
            if os.path.exists(tmp):
                os.unlink(tmp)
        self.paso_escritura = self.paso

    # ---------- archivos ----------
    def listar(self, ruta: str = ".", profundidad: int = 2) -> str:
        base = self.ruta(ruta)
        if not base.exists():
            return f"No existe: {ruta}"
        lineas, limite = [], 200

        def rec(d: Path, nivel: int):
            try:
                hijos = sorted(d.iterdir(), key=lambda x: (x.is_file(), x.name.lower()))
            except OSError:
                return
            for h in hijos:
                if len(lineas) >= limite:
                    return
                if h.name in IGNORAR:
                    continue
                if h.is_dir():
                    lineas.append("  " * nivel + h.name + "/")
                    if nivel + 1 < profundidad:
                        rec(h, nivel + 1)
                else:
                    lineas.append("  " * nivel + f"{h.name} ({h.stat().st_size} B)")

        rec(base, 0)
        if not lineas:
            return f"{self.rel(base)}/ está vacío."
        extra = f"\n[... límite de {limite} entradas]" if len(lineas) >= limite else ""
        return f"{self.rel(base)}/\n" + "\n".join(lineas) + extra

    def leer(self, ruta: str, desde: int = 1, lineas: int = 200) -> str:
        p = self.ruta(ruta)
        if not p.is_file():
            return f"No existe el archivo: {ruta}. Usa listar para ver los nombres reales."
        todas = p.read_text(encoding="utf-8", errors="replace").splitlines()
        desde = max(1, int(desde))
        sel = todas[desde - 1: desde - 1 + max(1, min(int(lineas), 400))]
        cuerpo = "\n".join(f"{i}| {l}" for i, l in enumerate(sel, desde))
        fin = desde + len(sel) - 1
        resto = f"\n[líneas {desde}-{fin} de {len(todas)}; usa desde={fin + 1} para seguir]" if fin < len(todas) else ""
        return recortar(f"{self.rel(p)} ({len(todas)} líneas)\n{cuerpo}{resto}", self.max_chars)

    def buscar(self, patron: str, ruta: str = ".", extension: str = "") -> str:
        try:
            rx = re.compile(patron, re.IGNORECASE)
        except re.error:
            rx = re.compile(re.escape(patron), re.IGNORECASE)
        base, hallados = self.ruta(ruta), []
        archivos = [base] if base.is_file() else base.rglob(f"*{extension}" if extension else "*")
        for f in archivos:
            if len(hallados) >= 50:
                break
            if not f.is_file() or any(x in IGNORAR for x in f.relative_to(self.raiz).parts) or f.stat().st_size > 1_000_000:
                continue
            try:
                for n, linea in enumerate(f.read_text(encoding="utf-8", errors="ignore").splitlines(), 1):
                    if rx.search(linea):
                        hallados.append(f"{self.rel(f)}:{n}: {linea.strip()[:160]}")
                        if len(hallados) >= 50:
                            break
            except OSError:
                continue
        return recortar("\n".join(hallados), self.max_chars) if hallados else f"Sin coincidencias para '{patron}'."

    def escribir(self, ruta: str, contenido: str) -> str:
        p = self.ruta(ruta)
        existia = p.exists()
        antes = p.read_text(encoding="utf-8", errors="replace") if existia else ""
        nota = ""
        if "\n" not in contenido and contenido.count("\\n") >= 3 and p.suffix.lower() in CODIGO:
            # el modelo mandó el código con saltos de línea escapados: se reparan
            contenido = contenido.replace("\\r\\n", "\n").replace("\\n", "\n").replace("\\t", "\t").replace('\\"', '"')
            nota = " (saltos de línea escapados corregidos)"
        if re.fullmatch(r"\s*[\[(<][^\])>\n]{0,80}[\])>]\s*", contenido) or re.fullmatch(r"\s*(\.\.\.|…)\s*", contenido):
            return (f"BLOQUEADO: el contenido para {self.rel(p)} es solo un marcador ({contenido.strip()[:60]!r}), no el "
                    "archivo. Escribe el contenido COMPLETO real (o usa editar para cambiar una parte).")
        if (existia and p.suffix.lower() in CODIGO and len(antes) > 800 and len(contenido) < len(antes) * 0.15):
            return (f"BLOQUEADO: el nuevo {self.rel(p)} tendría {len(contenido)} caracteres frente a {len(antes)} "
                    "actuales: parece truncado y destruiría el archivo. Usa editar para cambiar solo la parte necesaria.")
        veces = self.reescrituras.get(self.rel(p), 0)
        if existia and veces >= 3 and self._compila(p, antes):
            return (f"BLOQUEADO: ya reescribiste {self.rel(p)} entero {veces} veces en esta tarea y eso reintroduce "
                    "errores. El archivo actual es válido: usa leer para ver la zona exacta y editar para cambiar solo "
                    "esas líneas.")
        self._guardar(p, contenido)
        n = contenido.count("\n") + 1
        txt = f"OK {'reemplazado' if existia else 'creado'}: {self.rel(p)} ({n} líneas){nota}.{self._validar(p, contenido)}"
        if existia and re.match(r"(test_|.*_test\.|.*\.test\.)", p.name):
            contar = lambda t: len(re.findall(r"^\s*(?:async\s+)?def test|@Test\b|^\s*(?:test|it)\(", t, re.MULTILINE))
            n_antes, n_ahora = contar(antes), contar(contenido)
            if n_antes >= 3 and n_ahora < n_antes * 0.7:
                txt += (f"\n[CONDUCTA: los tests bajaron de {n_antes} a {n_ahora}. No borres ni debilites tests para que "
                        "pasen: arregla el código. Si un test estaba mal, corrígelo, no lo elimines.]")
        if existia:
            veces = self.reescrituras[self.rel(p)] = self.reescrituras.get(self.rel(p), 0) + 1
            if veces >= 2:
                txt += (f"\n[CONDUCTA: es la {veces + 1}ª vez que reescribes {self.rel(p)} entero. Usa editar para cambios "
                        "concretos: reescribir todo reintroduce errores ya corregidos.]")
        return txt

    def editar(self, ruta: str, buscar: str, reemplazar: str) -> str:
        p = self.ruta(ruta)
        if not p.is_file():
            return f"No existe el archivo: {ruta}."
        texto = p.read_text(encoding="utf-8", errors="replace")
        veces = texto.count(buscar) if buscar else 0
        if veces > 1:
            return f"'buscar' aparece {veces} veces en {ruta}. Incluye más líneas de contexto para que sea único."
        if veces == 1:
            nuevo = texto.replace(buscar, reemplazar, 1)
        else:
            # Tolerancia: comparar línea a línea ignorando espacios al inicio/fin.
            objetivo = [l.strip() for l in buscar.strip("\n").splitlines()]
            lineas = texto.splitlines(keepends=True)
            ini = next((i for i in range(len(lineas) - len(objetivo) + 1)
                        if objetivo and [l.strip() for l in lineas[i:i + len(objetivo)]] == objetivo), None)
            if ini is None:
                return (f"No encontré el texto a reemplazar en {ruta}. Usa leer para copiar el texto exacto "
                        f"(o escribir para reemplazar el archivo completo).")
            fin_linea = "\n" if lineas[ini + len(objetivo) - 1].endswith("\n") else ""
            nuevo = "".join(lineas[:ini]) + reemplazar.rstrip("\n") + fin_linea + "".join(lineas[ini + len(objetivo):])
        self._guardar(p, nuevo)
        return f"OK editado: {self.rel(p)}.{self._validar(p, nuevo)}"

    # ---------- comandos ----------
    def _entorno(self) -> dict:
        env = dict(os.environ, AGENTE_NOPAUSE="1", PYTHONUTF8="1", PYTHONIOENCODING="utf-8",
                   PIP_DISABLE_PIP_VERSION_CHECK="1", PYTHONDONTWRITEBYTECODE="1", CI="1")
        venv = self.raiz / ".venv" / "Scripts"
        if venv.is_dir():  # el venv del proyecto tiene prioridad
            env["PATH"] = f"{venv}{os.pathsep}{env.get('PATH', '')}"
            env["VIRTUAL_ENV"] = str(venv.parent)
        return env

    def _permitido(self, comando: str) -> str | None:
        if not PELIGROSOS.search(comando):
            return None
        if self.confirmar and self.confirmar(comando):
            return None
        return "BLOQUEADO: comando potencialmente destructivo no autorizado. Busca otra forma o pide al usuario que lo haga."

    def ejecutar(self, comando: str, segundos: int = 0, fondo: bool = False) -> str:
        if (bloqueo := self._permitido(comando)):
            return bloqueo
        if fondo:
            return self._fondo(comando)
        limite = max(5, min(int(segundos or self.timeout), 900))
        if INSTALACIONES.search(comando):
            limite = max(limite, MIN_INSTALACION)
        t0 = time.time()
        proc = subprocess.Popen(comando, shell=True, cwd=self.raiz, env=self._entorno(), stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                creationflags=subprocess.CREATE_NO_WINDOW)
        try:
            salida, _ = proc.communicate(timeout=limite)
        except subprocess.TimeoutExpired:
            matar_arbol(proc.pid)
            salida, _ = proc.communicate()
            return (f"TIMEOUT tras {limite}s (proceso detenido). Si es un servidor o programa que no termina, usa "
                    f"fondo=true. Salida parcial:\n{recortar(decodificar(salida), self.max_chars)}")
        codigo = proc.returncode
        if codigo == 0:
            self.paso_prueba = self.paso
        elif self.checkpoints:
            self.fallos_desde_verde += 1
        texto = decodificar(salida).strip() or "(sin salida)"
        return f"exit={codigo} ({time.time() - t0:.1f}s)\n{recortar(texto, self.max_chars)}" + (self._sugerir_restaurar() if codigo else "")

    def _fondo(self, comando: str) -> str:
        n = max(self.procesos, default=0) + 1
        self.dir_agente.mkdir(exist_ok=True)
        log = self.dir_agente / f"proceso_{n}.log"
        f = open(log, "wb")  # noqa: SIM115 -- abierto mientras viva el proceso; se cierra al detenerlo
        proc = subprocess.Popen(comando, shell=True, cwd=self.raiz, env=self._entorno(), stdin=subprocess.DEVNULL,
                                stdout=f, stderr=subprocess.STDOUT,
                                creationflags=subprocess.CREATE_NO_WINDOW | subprocess.CREATE_NEW_PROCESS_GROUP)
        self.procesos[n] = {"proc": proc, "log": log, "archivo": f, "comando": comando}
        time.sleep(4)  # captura errores de arranque
        estado = f"TERMINÓ (exit={proc.returncode})" if proc.poll() is not None else "en ejecución"
        if proc.poll() is None:
            self.paso_prueba = self.paso
        return f"Proceso #{n} {estado}. Log inicial:\n{self._cola_log(n)}\nUsa procesos(accion='ver', id={n}) o 'detener'."

    def _cola_log(self, n: int, maximo: int = 2500) -> str:
        p = self.procesos[n]
        p["archivo"].flush()
        return recortar(decodificar(p["log"].read_bytes()).strip() or "(sin salida aún)", maximo)

    def procesos_cmd(self, accion: str = "listar", id: int = 0) -> str:
        if accion == "listar" or not id:
            if not self.procesos:
                return "No hay procesos en segundo plano."
            return "\n".join(f"#{n} {'vivo' if p['proc'].poll() is None else 'terminado'}: {p['comando']}"
                             for n, p in self.procesos.items())
        if id not in self.procesos:
            return f"No existe el proceso #{id}."
        if accion == "detener":
            matar_arbol(self.procesos[id]["proc"].pid)
            return f"Proceso #{id} detenido."
        vivo = self.procesos[id]["proc"].poll() is None
        return f"#{id} {'vivo' if vivo else 'terminado'}. Log:\n{self._cola_log(id, self.max_chars)}"

    def detener_todos(self) -> None:
        for p in self.procesos.values():
            if p["proc"].poll() is None:
                matar_arbol(p["proc"].pid)
            p["archivo"].close()

    def crear_bat(self, nombre: str, comandos: list, descripcion: str = "", usar_venv: bool = False) -> str:
        nombre = nombre if nombre.lower().endswith((".bat", ".cmd")) else nombre + ".bat"
        if isinstance(comandos, str):
            comandos = [c for c in comandos.splitlines() if c.strip()]
        lineas = ["@echo off", "setlocal EnableExtensions", "chcp 65001 >nul", 'cd /d "%~dp0"',
                  f"title {Path(nombre).stem}", 'set "RC=0"']
        if descripcion:
            lineas.append(f"echo {descripcion}")
        if usar_venv:
            lineas += ['if not exist ".venv\\Scripts\\python.exe" (',
                       "  echo Creando entorno virtual .venv ...",
                       "  python -m venv .venv || (set \"RC=1\" & goto :error)", ")",
                       'call ".venv\\Scripts\\activate.bat"',
                       "if exist requirements.txt (",
                       "  python -m pip install -q -r requirements.txt || (set \"RC=1\" & goto :error)", ")"]
        reales = [c.strip() for c in comandos if c.strip()]
        total = sum(1 for c in reales if not re.match(r"(echo|rem|cls|title)\b", c, re.IGNORECASE))
        i = 0
        for c in reales:
            if re.match(r"(echo|rem|cls|title)\b", c, re.IGNORECASE):  # mensajes: tal cual, sin numerar ni comprobar
                lineas.append(c)
                continue
            i += 1
            lineas += [f"echo [{i}/{total}] {c.replace('%', '%%').replace('&', '^&').replace('>', '^>').replace('<', '^<').replace('|', '^|')}",
                       f"{'call ' if c.lower().endswith(('.bat', '.cmd')) and not c.lower().startswith('call ') else ''}{c}",
                       'if errorlevel 1 (set "RC=%errorlevel%" & goto :error)']
        lineas += ["echo.", "echo [OK] Terminado correctamente.", "goto :fin", ":error", "echo.",
                   "echo [ERROR] Falló el paso anterior (código %RC%).", ":fin",
                   "if not defined AGENTE_NOPAUSE pause", "exit /b %RC%"]
        p = self.ruta(nombre)
        self._guardar(p, "\n".join(lineas) + "\n")
        return (f"OK creado: {self.rel(p)} ({total} pasos{', con .venv' if usar_venv else ''}). "
                f"Pruébalo con ejecutar(comando='{self.rel(p)}') o fondo=true si arranca un servidor.")

    # ---------- git ----------
    def _git(self, *args: str, linea: str = "") -> tuple[int, str]:
        """Solo el repositorio de la carpeta del proyecto: nunca sube a uno de una carpeta superior."""
        env = dict(self._entorno(), GIT_CEILING_DIRECTORIES=str(self.raiz.parent), GIT_TERMINAL_PROMPT="0")
        base = ["git", "--no-pager", "-c", "color.ui=false", "-c", "core.quotepath=false"]
        r = subprocess.run(subprocess.list2cmdline(base) + " " + linea if linea else base + list(args), cwd=self.raiz,
                           env=env, stdin=subprocess.DEVNULL, capture_output=True, timeout=120,
                           creationflags=subprocess.CREATE_NO_WINDOW, check=False)
        return r.returncode, decodificar(r.stdout + r.stderr).strip()

    def git(self, accion: str = "estado", mensaje: str = "", nombre: str = "", ruta: str = "", comando: str = "") -> str:
        rutas = ["--", self.rel(self.ruta(ruta))] if ruta else NO_VERSIONAR
        if accion == "estado":
            codigo, salida = self._git("status", "--short", "--branch", *rutas)
        elif accion == "diff":
            codigo, salida = self._git("diff", "HEAD", "--stat", "--patch", *rutas)
            if codigo != 0 and "not a git repository" not in salida:  # aún sin commits: lo preparado
                codigo, salida = self._git("diff", "--cached", "--stat", "--patch", *rutas)
            if codigo == 0:
                _, nuevos = self._git("ls-files", "--others", "--exclude-standard", *rutas)
                salida = "\n".join(x for x in (salida, nuevos and "Archivos nuevos sin seguimiento:\n" + nuevos) if x)
                salida = salida or "Sin cambios respecto al último commit."
        elif accion == "log":
            codigo, salida = self._git("log", "--oneline", "--decorate", "-n", "15", *(rutas if ruta else []))
        elif accion == "commit":
            if not mensaje.strip():
                return "Indica 'mensaje' para el commit (una frase: qué cambia y por qué)."
            codigo, salida = self._git("add", "-A", *rutas)
            if codigo == 0:
                codigo, salida = self._git("commit", "-m", mensaje.strip(), *(rutas if ruta else []))
        elif accion == "rama":
            if not re.fullmatch(r"[\w][\w./-]*", nombre or ""):
                return "Indica 'nombre' para la rama (letras, números, - _ / .)."
            existe = self._git("rev-parse", "--verify", "--quiet", f"refs/heads/{nombre}")[0] == 0
            codigo, salida = self._git("switch", nombre) if existe else self._git("switch", "-c", nombre)
        elif accion == "otro":
            comando = re.sub(r"^\s*git\s+", "", comando or "").strip()
            if not comando:
                return "Indica 'comando' (lo que va tras 'git', p. ej. 'init' o 'show HEAD')."
            if re.search(r"\bpush\b", comando, re.IGNORECASE):
                return "BLOQUEADO: el agente nunca hace push. Súbelo tú cuando hayas revisado los commits."
            if (bloqueo := self._permitido("git " + comando)):
                return bloqueo
            codigo, salida = self._git(linea=comando)
        else:
            return "accion debe ser: estado, diff, log, commit, rama u otro."
        if "not a git repository" in salida:
            return "exit=128 La carpeta del proyecto no es un repositorio git. Créalo con git(accion='otro', comando='init')."
        return f"exit={codigo} git {accion}\n{recortar(salida, self.max_chars)}" if codigo else recortar(salida or "OK", self.max_chars)

    # ---------- web ----------
    def _descargar(self, url: str, datos: bytes | None = None) -> tuple[str, str]:
        req = urllib.request.Request(url, datos, {"User-Agent": UA, "Accept-Language": "es,en;q=0.8"})
        with urllib.request.urlopen(req, timeout=20) as r:
            tipo = r.headers.get("Content-Type", "")
            cuerpo = r.read(3_000_000)
            enc = r.headers.get_content_charset() or "utf-8"
        return cuerpo.decode(enc, "replace"), tipo

    def web_buscar(self, consulta: str) -> str:
        limpiar = lambda s: re.sub(r"\s+", " ", html.unescape(re.sub(r"<[^>]+>", "", s))).strip()
        res, fallos = [], []
        try:  # 1) DuckDuckGo
            pagina, _ = self._descargar("https://html.duckduckgo.com/html/",
                                        urllib.parse.urlencode({"q": consulta}).encode())
            for m in re.finditer(r'class="result__a"[^>]*href="([^"]+)"[^>]*>(.*?)</a>.*?class="result__snippet"[^>]*>(.*?)</a>',
                                 pagina, re.DOTALL):
                url = m.group(1)
                if "uddg=" in url:
                    url = urllib.parse.unquote(url.split("uddg=")[1].split("&")[0])
                res.append((limpiar(m.group(2)), url, limpiar(m.group(3))))
        except Exception as e:
            fallos.append(f"DuckDuckGo: {e}")
        if not res:  # 2) Bing como respaldo
            try:
                pagina, _ = self._descargar("https://www.bing.com/search?" + urllib.parse.urlencode({"q": consulta, "setlang": "es"}))
                for bloque in re.findall(r'<li class="b_algo"(.*?)</li>', pagina, re.DOTALL):
                    a = re.search(r'<h2[^>]*>\s*<a[^>]*href="([^"]+)"[^>]*>(.*?)</a>', bloque, re.DOTALL)
                    if not a:
                        continue
                    url = html.unescape(a.group(1))
                    if "bing.com/ck/a" in url and (u := re.search(r"[?&]u=a1([^&]+)", url)):
                        cod = u.group(1)
                        url = base64.urlsafe_b64decode(cod + "=" * (-len(cod) % 4)).decode("utf-8", "replace")
                    snip = re.search(r"<p[^>]*>(.*?)</p>", bloque, re.DOTALL)
                    res.append((limpiar(a.group(2)), url, limpiar(snip.group(1)) if snip else ""))
            except Exception as e:
                fallos.append(f"Bing: {e}")
        if not res:
            return ("Sin resultados" + (f" ({'; '.join(fallos)})" if fallos else "") +
                    ". Prueba otra consulta más corta (en inglés suele haber más).")
        return "\n".join(f"{i}. {t}\n   {u}\n   {s[:220]}" for i, (t, u, s) in enumerate(res[:6], 1))

    def web_leer(self, url: str, buscar: str = "") -> str:
        if not url.startswith(("http://", "https://")):
            return "URL inválida: debe empezar por http:// o https://"
        try:
            cuerpo, tipo = self._descargar(url)
        except Exception as e:
            return f"No se pudo descargar {url}: {e}"
        if "html" in tipo:
            parser = _HtmlATexto()
            parser.feed(cuerpo)
            texto = parser.texto()
        else:
            texto = cuerpo
        if buscar:  # párrafos con más palabras clave distintas, en su orden original
            claves = {c for c in re.findall(r"\w{3,}", buscar.lower()) if c not in VACIAS}
            puntuados = [(sum(c in p.lower() for c in claves), i, p) for i, p in enumerate(texto.split("\n\n"))]
            mejores = sorted((x for x in puntuados if x[0] > 0), key=lambda x: (-x[0], x[1]))[:14]
            if mejores:
                texto = "\n\n".join(p for _, _, p in sorted(mejores, key=lambda x: x[1]))
        return recortar(f"{url}\n{texto}", self.max_chars)

    # ---------- calidad y conocimiento ----------
    def verificar_cmd(self, ruta: str = ".", segundos: int = 8) -> str:
        informe, errores = verificar(self.raiz, self.python_proyecto(), ruta, max(4, min(int(segundos), 60)), self.max_chars)
        self.verificacion = {"paso": self.paso, "errores": errores, "informe": informe}
        if errores == 0:
            self.paso_prueba = self.paso
            n = self._checkpoint()
            informe += (f"\nSIN ERRORES: guardado checkpoint #{n} (versión que funciona). Los avisos son secundarios: "
                        "atiende solo los de lógica sin conectar y no toques lo que ya funciona. Sigue con el plan.")
        else:
            self.fallos_desde_verde += 1
            informe += "\nCorrige los ERRORES (empieza por el primero), usa editar y vuelve a llamar a verificar."
            informe += self._sugerir_restaurar()
        return informe

    def _fuentes(self) -> list[Path]:
        return [p for p in self.raiz.rglob("*") if p.is_file() and p.suffix.lower() in CODIGO | {".txt", ".md", ".toml", ".cfg", ".ini"}
                and not (IGNORAR & set(p.relative_to(self.raiz).parts[:-1])) and p.stat().st_size < 500_000]

    @property
    def archivo_checkpoints(self) -> Path:
        return self.dir_agente / "checkpoints.json"

    def _cargar_checkpoints(self) -> None:
        if not self.checkpoints and self.archivo_checkpoints.exists():
            try:
                self.checkpoints = json.loads(self.archivo_checkpoints.read_text(encoding="utf-8"))
            except ValueError:
                self.checkpoints = []

    def _checkpoint(self) -> int:
        self._cargar_checkpoints()
        archivos = {self.rel(p): p.read_text(encoding="utf-8", errors="replace") for p in self._fuentes()}
        if self.checkpoints and self.checkpoints[-1]["archivos"] == archivos:
            return len(self.checkpoints)
        self.checkpoints.append({"paso": self.paso, "fecha": time.strftime("%Y-%m-%d %H:%M"), "archivos": archivos})
        self.checkpoints = self.checkpoints[-3:]
        self.fallos_desde_verde = 0
        self.dir_agente.mkdir(exist_ok=True)
        self.archivo_checkpoints.write_text(json.dumps(self.checkpoints, ensure_ascii=False), encoding="utf-8")
        return len(self.checkpoints)

    def _sugerir_restaurar(self) -> str:
        if self.checkpoints and self.fallos_desde_verde >= 3:
            return (f"\n[ATENCIÓN: en el paso {self.checkpoints[-1]['paso']} tenías una versión que FUNCIONABA (checkpoint "
                    f"#{len(self.checkpoints)}) y desde entonces llevas {self.fallos_desde_verde} fallos. Si no lo arreglas "
                    "en el próximo intento, usa restaurar() y aplica cambios más pequeños.]")
        return ""

    def restaurar(self, checkpoint: int = 0) -> str:
        self._cargar_checkpoints()
        if not self.checkpoints:
            return "No hay checkpoints: aún no ha habido ninguna verificación sin errores."
        n = checkpoint if 0 < checkpoint <= len(self.checkpoints) else len(self.checkpoints)
        cp = self.checkpoints[n - 1]
        for rel, texto in cp["archivos"].items():
            self._guardar(self.raiz / rel, texto)
        nuevos = sorted({self.rel(p) for p in self._fuentes()} - set(cp["archivos"]))
        self.reescrituras.clear()
        self.fallos_desde_verde = 0
        self.verificacion = {"paso": self.paso, "errores": 0, "informe": f"restaurado checkpoint #{n}"}
        self.paso_prueba = self.paso
        extra = f" Archivos creados después (no se tocaron): {', '.join(nuevos)}." if nuevos else ""
        return (f"OK restaurado el checkpoint #{n} (versión que funcionaba, {cp.get('fecha', 'paso ' + str(cp['paso']))}): "
                f"{len(cp['archivos'])} archivos.{extra} Haz cambios pequeños y verifica tras cada uno.")

    def ver_pantalla(self, ruta: str = "", pregunta: str = "", segundos: int = 2) -> str:
        return pantalla.ver(self, ruta, pregunta, segundos)

    def revisar_cmd(self, requisitos: str = "", enfoque: str = "") -> str:
        if not self.revisar_cb:
            return "Revisión no disponible (sin conexión con el modelo)."
        texto, _ = self.revisar_cb(requisitos, enfoque)
        return texto

    def guia(self, tema: str) -> str:
        return conocimiento.guia(tema)

    def version(self, paquete: str, ecosistema: str = "pypi") -> str:
        return conocimiento.version(self, paquete, ecosistema)

    def investigar(self, pregunta: str) -> str:
        return conocimiento.investigar(self, pregunta)

    def aprender(self, leccion: str) -> str:
        leccion = " ".join(leccion.split())[:220]
        if len(leccion) < 15:
            return "La lección debe ser una frase concreta (qué pasó y cómo evitarlo)."
        previas = LECCIONES.read_text(encoding="utf-8").splitlines() if LECCIONES.exists() else ["# Lecciones aprendidas"]
        if any(leccion.lower() in l.lower() for l in previas):
            return "Esa lección ya estaba guardada."
        cuerpo = [l for l in previas[1:] if l.strip()][-39:] + [f"- {leccion}"]  # máximo 40, las más recientes
        LECCIONES.write_text(previas[0] + "\n" + "\n".join(cuerpo) + "\n", encoding="utf-8")
        return "Lección guardada: se tendrá en cuenta en las próximas tareas."

    def mapa(self, maximo: int = 1500) -> str:
        """Archivos del proyecto con sus nombres públicos: tras compactar, el modelo no inventa módulos ni nombres."""
        import ast
        lineas = []
        for p in sorted(self._fuentes(), key=lambda x: self.rel(x)):
            texto = p.read_text(encoding="utf-8", errors="replace")
            nombres = []
            if p.suffix == ".py":
                try:
                    for n in ast.parse(texto).body:
                        if isinstance(n, ast.ClassDef):
                            metodos = [m.name for m in n.body if isinstance(m, ast.FunctionDef) and not m.name.startswith("__")]
                            nombres.append(f"class {n.name}({', '.join(metodos[:8])})")
                        elif isinstance(n, ast.FunctionDef):
                            nombres.append(f"{n.name}()")
                        elif isinstance(n, ast.Assign) and isinstance(n.targets[0], ast.Name) and n.targets[0].id.isupper():
                            nombres.append(n.targets[0].id)
                except SyntaxError:
                    nombres.append("(error de sintaxis)")
            elif p.suffix in (".js", ".mjs", ".java"):
                nombres = re.findall(r"(?:^|\s)(?:class|function|interface|record)\s+(\w+)", texto)[:10]
            lineas.append(f"- {self.rel(p)}: {', '.join(nombres)[:220]}" if nombres else f"- {self.rel(p)}")
        return "\n".join(lineas)[:maximo]

    # ---------- plan ----------
    @property
    def archivo_plan(self) -> Path:
        return self.dir_agente / "plan.md"

    def plan(self, pasos: list) -> str:
        if isinstance(pasos, str):
            pasos = [p.strip(" -*0123456789.") for p in pasos.splitlines() if p.strip()]
        pasos = [re.sub(r"^\d+[.)]\s*", "", str(p).strip()) for p in pasos if str(p).strip()]  # sin "1. 1."
        if not pasos:
            return "El plan necesita al menos un paso."
        self.dir_agente.mkdir(exist_ok=True)
        self.archivo_plan.write_text("# Plan\n" + "".join(f"- [ ] {i}. {p}\n" for i, p in enumerate(pasos, 1)),
                                     encoding="utf-8")
        return f"Plan guardado ({len(pasos)} pasos). Empieza por el paso 1.\n{self.plan_texto()}"

    def paso_hecho(self, numero: int, nota: str = "") -> str:
        if not self.archivo_plan.exists():
            return "No hay plan. Crea uno con plan."
        lineas = self.archivo_plan.read_text(encoding="utf-8").splitlines()
        patron = re.compile(rf"^- \[[ x!]\] {int(numero)}\. ")
        for i, l in enumerate(lineas):
            if patron.match(l):
                if (re.search(r"prob|test|verific|ejecut|comprob|revis", l, re.IGNORECASE)
                        and self.paso_prueba < max(self.paso_escritura, self.inicio_tarea)):
                    return (f"RECHAZADO: el paso {numero} es de prueba/verificación y no hay ninguna ejecución con éxito "
                            "después de tu último cambio. Ejecuta la prueba (o verificar) y luego márcalo.")
                faltan = sorted(f for f in archivos_pedidos(l) if not self.existe(f)) if CREAR.search(l) else []
                if faltan:  # conducta: marcar como hecho un paso antes de hacerlo
                    return (f"RECHAZADO: el paso {numero} pide {', '.join(faltan)} y todavía no existe. "
                            "Créalo con escribir y después márcalo como hecho.")
                base = re.sub(r" — .*$", "", l[6:])
                lineas[i] = f"- [x] {base}" + (f" — {nota.strip()[:150]}" if nota.strip() else "")
                self.archivo_plan.write_text("\n".join(lineas) + "\n", encoding="utf-8")
                pendientes = [l for l in lineas if l.startswith("- [ ]")]
                sig = f"Siguiente: {pendientes[0][6:]}" if pendientes else "Todos los pasos hechos: verifica y usa terminar."
                return f"Paso {numero} marcado como hecho. {sig}"
        return f"No existe el paso {numero}.\n{self.plan_texto()}"

    def existe(self, nombre: str) -> bool:
        """El archivo existe en el proyecto (con su ruta o solo por nombre, en cualquier carpeta)."""
        if (self.raiz / nombre).exists():
            return True
        return any(not (IGNORAR & set(p.parts)) for p in self.raiz.rglob(Path(nombre).name))

    def plan_texto(self) -> str:
        if not self.archivo_plan.exists():
            return ""
        return "\n".join(l for l in self.archivo_plan.read_text(encoding="utf-8").splitlines() if l.startswith("- ["))


def _f(nombre, desc, props, req):
    return {"type": "function", "function": {"name": nombre, "description": desc,
                                             "parameters": {"type": "object", "properties": props, "required": req}}}


S, I, B = {"type": "string"}, {"type": "integer"}, {"type": "boolean"}
LISTA = {"type": "array", "items": {"type": "string"}}

ESQUEMAS = [
    _f("plan", "Crea o reemplaza el plan de trabajo (3-10 pasos concretos: requisitos, tests reales, verificar, revisar, .bat).",
       {"pasos": LISTA}, ["pasos"]),
    _f("paso_hecho", "Marca un paso del plan como hecho.", {"numero": I, "nota": S}, ["numero"]),
    _f("listar", "Lista archivos y carpetas del proyecto.", {"ruta": S, "profundidad": I}, []),
    _f("leer", "Lee un archivo con números de línea (por bloques).", {"ruta": S, "desde": I, "lineas": I}, ["ruta"]),
    _f("buscar", "Busca texto o regex en los archivos del proyecto.", {"patron": S, "ruta": S, "extension": S}, ["patron"]),
    _f("escribir", "Crea o reemplaza un archivo con su contenido COMPLETO (sin '...' ni TODO). Valida sintaxis .py/.json.",
       {"ruta": S, "contenido": S}, ["ruta", "contenido"]),
    _f("editar", "Reemplaza un fragmento exacto de un archivo existente (copia el texto tal cual de leer).",
       {"ruta": S, "buscar": S, "reemplazar": S}, ["ruta", "buscar", "reemplazar"]),
    _f("ejecutar", "Ejecuta un comando de Windows (cmd) en la carpeta del proyecto. Usa fondo=true para servidores "
                   "o programas que no terminan. Si existe .venv se usa automáticamente.",
       {"comando": S, "segundos": I, "fondo": B}, ["comando"]),
    _f("procesos", "Ver o detener procesos lanzados con fondo=true.",
       {"accion": {"type": "string", "enum": ["listar", "ver", "detener"]}, "id": I}, ["accion"]),
    _f("crear_bat", "Crea un .bat robusto (UTF-8, se detiene en errores, crea .venv e instala requirements.txt si usar_venv).",
       {"nombre": S, "comandos": LISTA, "descripcion": S, "usar_venv": B}, ["nombre", "comandos"]),
    _f("web_buscar", "Busca en internet (documentación, errores, librerías). Devuelve títulos, URLs y resúmenes.",
       {"consulta": S}, ["consulta"]),
    _f("web_leer", "Lee el texto de una página web. 'buscar' filtra párrafos con esas palabras.",
       {"url": S, "buscar": S}, ["url"]),
    _f("investigar", "Busca en internet y lee las mejores páginas en UN paso: devuelve los párrafos relevantes con su "
                     "fuente. Úsalo para APIs, errores o librerías que no conozcas bien.", {"pregunta": S}, ["pregunta"]),
    _f("guia", "Buenas prácticas ACTUALES y cómo verificar para: python, pygame, html-juego, web, api, java, tests, bat. "
               "Llámala al empezar un proyecto.", {"tema": S}, ["tema"]),
    _f("version", "Última versión real de un paquete y si es compatible con tu Python (evita librerías obsoletas).",
       {"paquete": S, "ecosistema": {"type": "string", "enum": ["pypi", "npm", "maven"]}}, ["paquete"]),
    _f("verificar", "Control de calidad de TODO el proyecto: errores, código que nunca se usa (funciones sin conectar), "
                    "tests, y ejecución real (juego pygame sin ventana, web en navegador, Java). Úsalo tras construir y "
                    "tras cada corrección.", {"ruta": S, "segundos": I}, []),
    _f("ver_pantalla", "Captura la interfaz (un .html, una URL http://localhost:PUERTO o el .py de un juego pygame) y "
                       "comprueba si se ve bien: texto cortado o sobre fondo cargado, elementos fuera de pantalla, "
                       "pantalla en negro. Si el "
                       "modelo cargado ve imágenes, la revisa él. 'pregunta' = qué mirar además.",
       {"ruta": S, "pregunta": S, "segundos": I}, []),
    _f("revisar", "Revisión independiente: otro análisis compara cada requisito con el código y señala lo que falta o "
                  "no funciona. 'enfoque' = qué mirar con más cuidado.", {"requisitos": S, "enfoque": S}, []),
    _f("restaurar", "Vuelve a la última versión que pasó verificar sin errores (checkpoint). Úsalo si tus cambios "
                    "rompieron algo que funcionaba y no consigues arreglarlo.", {"checkpoint": I}, []),
    _f("aprender", "Guarda una lección de una frase tras resolver un error no obvio, para no repetirlo en el futuro.",
       {"leccion": S}, ["leccion"]),
    _f("git", "Git del proyecto (local, nunca push): estado, diff (cambios sin commit), log, commit (añade todo y "
              "confirma; 'mensaje' obligatorio), rama (crea o cambia a 'nombre'), otro ('comando' tras 'git'; lo "
              "destructivo pide confirmación). 'ruta' limita a un archivo o carpeta.",
       {"accion": {"type": "string", "enum": ["estado", "diff", "log", "commit", "rama", "otro"]}, "mensaje": S,
        "nombre": S, "ruta": S, "comando": S}, ["accion"]),
    _f("terminar", "Termina la tarea cuando todo está hecho, verificado y revisado. Resume qué se hizo y cómo usarlo.",
       {"resumen": S}, ["resumen"]),
]


def despachar(h: Herramientas, nombre: str, args: dict) -> str:
    tabla = {"plan": h.plan, "paso_hecho": h.paso_hecho, "listar": h.listar, "leer": h.leer, "buscar": h.buscar,
             "escribir": h.escribir, "editar": h.editar, "ejecutar": h.ejecutar, "procesos": h.procesos_cmd,
             "crear_bat": h.crear_bat, "web_buscar": h.web_buscar, "web_leer": h.web_leer, "investigar": h.investigar,
             "guia": h.guia, "version": h.version, "verificar": h.verificar_cmd, "revisar": h.revisar_cmd,
             "aprender": h.aprender, "restaurar": h.restaurar, "ver_pantalla": h.ver_pantalla, "git": h.git}
    if nombre not in tabla:
        return f"Herramienta desconocida: {nombre}. Disponibles: {', '.join(tabla)}, terminar."
    try:
        return tabla[nombre](**args)
    except TypeError as e:
        return f"Argumentos inválidos para {nombre}: {e}"
    except Exception as e:  # el modelo debe ver el error, no el agente caerse
        return f"ERROR en {nombre}: {type(e).__name__}: {e}"
