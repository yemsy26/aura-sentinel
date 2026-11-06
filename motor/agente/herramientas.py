"""Herramientas del agente. Devuelven texto compacto (barato en tokens y fácil de leer para modelos pequeños)."""
import base64
import html.parser
import json
import os
import re
import shutil
import subprocess
import tempfile
import time
import urllib.parse
import urllib.request
from pathlib import Path

from . import android, conocimiento, lecciones, matematicas, pantalla, procesos
from . import mapa as mapa_repo
from .idiomas import resumen_idiomas
from .mapa import PATRONES_MAPA
from .permisos import (
    AJENO,
    COLA_SIN_EFECTO,
    COMENTARIO,
    CREDENCIALES,
    CRIPTO,
    CRIPTO_CONFIG,
    DESTRUCTIVO,
    EXTERNO,
    LICENCIA,
    LICENCIAS,
    MATAR_NOMBRE,
    MATAR_PID,
    NPM_INSTALA,
    NPM_SCRIPT,
    PELIGROSOS,
    POR_NOMBRE,
    SCRIPT,
    externo,
    transaccion,
)
from .proyecto import IGNORAR, correr, es_codigo, ignorado
from .verificacion import LANZADOR, chequeo_rapido, con_call, muertos_ahora, ordenes_bat, verificar

VACIAS = {"como", "para", "que", "con", "los", "las", "una", "uno", "del", "por", "the", "and", "for", "with", "how",
          "what", "este", "esta", "esto", "hay", "hacer", "puedo", "cual", "cuál", "cómo", "qué", "use", "using"}

LISTADO_IGNORA = IGNORAR - {"out"}  # listar y buscar sí enseñan out/
CD = re.compile(r'\bcd\s+(?:/d\s+)?(?:"([^"]+)"|([^\s&|"]+))', re.IGNORECASE)
# Nunca se versionan (aunque falte .gitignore): estado del agente, entornos y cachés.
NO_VERSIONAR = ["--", ".", ":(exclude).agente"] + [f":(exclude,glob)**/{d}/**" for d in (
    ".venv", "venv", "node_modules", "__pycache__", ".pytest_cache", ".mypy_cache", ".ruff_cache")]
INSTALACIONES = re.compile(  # descargan, instalan o compilan: con poco tiempo se cortan a medias
    r"\b(venv|virtualenv|pip3?(\.exe)?\s+install|npm\s+(install|ci|i)\b|npx\s|uv\s+(pip|venv|sync|add)|playwright\s+install|"
    r"cargo\s+(build|test|run|install|check|clippy|fetch|update|add)|rustup\s|go\s+(build|test|run|get|install|mod|vet)\b|"
    r"(docker|podman)\s+(build|pull|compose\s+(up|build|pull|run))|dotnet\s+(restore|build|test|publish|run|new|tool|add)|"
    r"(mvnw?|gradlew?)(\.cmd|\.bat)?\s|(winget|choco|scoop)\s+install|(flutter|dart)\s+(pub|create|build|test)\b|"
    r"sdkmanager|anchor\s+(build|test|init)\b|forge\s+(init|install|build|test)\b)",
    re.IGNORECASE)
MIN_INSTALACION = 300  # segundos
# Con doble clic, el .bat recibe el PATH del Explorador, que puede no traer el del usuario (pasa si es muy largo): el .bat
# funcionaba en las pruebas del motor y al usuario le decía «"python" no se reconoce».
INTERNAS_CMD = {"assoc", "call", "cd", "chdir", "cls", "copy", "del", "dir", "echo", "erase", "exit", "for", "ftype", "goto", "if",
                "md", "mkdir", "move", "path", "pause", "popd", "pushd", "rd", "ren", "rename", "rmdir", "rem", "set", "setlocal",
                "shift", "start", "title", "type", "ver", "vol"}
PATH_DEL_USUARIO = ('if defined FALTA for /f "skip=2 tokens=2,*" %%A in (\'reg query HKCU\\Environment /v Path 2^>nul\') '
                    'do call set "PATH=%%PATH%%;%%B"')
MAX_PREGUNTAS = 3  # por tarea: más es pedir al usuario lo que el agente puede decidir
# Archivos que se nombran en un texto (petición del usuario o paso del plan); no las librerías como three.js.
# Con varios puntos en el nombre (tauri.conf.json, vite.config.ts): antes solo se tomaba lo último («conf.json»).
ARCHIVO_PEDIDO = (r"\b[\w\-]+(?:\.[\w\-]+)*\.(?:py|bat|cmd|ps1|txt|json|md|js|ts|jsx|tsx|html|css|toml|ya?ml|ini|cfg|sql|"
                  r"csv|java|cs|go|rs|cpp|c|h)\b")
NO_ARCHIVO = re.compile(r"(node|next|nuxt|vue|react|three|d3|express|chart|p5|phaser|pixi|babylon|anime|alpine|ember|"
                        r"socket|solid|svelte)(\.[\w\-]+)*\.js$", re.IGNORECASE)  # librerías: three.js, three.module.js
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
        self.leidos: dict[Path, tuple] = {}  # leído desde la línea 1 en la tarea: (mtime, tamaño, última línea)
        self.inicio_tarea = 0
        self.checkpoints: list[dict] = []   # estados que pasaron verificar sin errores
        self.fallos_desde_verde = 0
        self.evidencia: list[str] = []      # comandos ejecutados en la tarea y su resultado (para revisar)
        self.preguntar_cb = None            # preguntar_cb(pregunta, opciones) -> respuesta; lo conecta el bucle
        self.preguntas = 0
        self.errores_previos: set[str] | None = None  # modo grande: errores que ya estaban (no cuentan)
        self.muertos_previos: set[str] = set()  # código sin usar que ya estaba al empezar la tarea (no cuenta)
        self.idiomas_parcial: list[str] | None = None  # modo grande: archivos de la parte en curso (idiomas a medias)

    def nueva_tarea(self, paso_inicio: int = 0, texto: str = "") -> None:
        self.reescrituras.clear()
        self.leidos.clear()
        self.evidencia.clear()
        self.preguntas = 0
        self.inicio_tarea = paso_inicio
        if texto.strip():
            (self.dir_agente / pantalla.SESION).unlink(missing_ok=True)  # «continúa» conserva la sesión del navegador
            self._anotar_checkpoints(texto)
            self.muertos_previos = muertos_ahora(self.raiz) - set(self._foto().get("sin_conectar", []))
            self._guardar_foto(foto=sorted(self.muertos_previos))
        elif not self.muertos_previos:  # 'continúa' tras reabrir Aura sigue con la foto de la tarea que continúa
            self.muertos_previos = set(self._foto().get("foto", []))

    def tarea_interrumpida(self) -> None:
        """Lo que queda sin conectar es trabajo a medias de la tarea: la siguiente no debe tomarlo por código previo."""
        self._guardar_foto(foto=sorted(self.muertos_previos), sin_conectar=sorted(muertos_ahora(self.raiz) - self.muertos_previos))

    def _foto(self) -> dict:
        try:
            return json.loads((self.dir_agente / "sin_usar.json").read_text(encoding="utf-8"))
        except (OSError, ValueError):
            return {}

    def _guardar_foto(self, **datos) -> None:
        self.dir_agente.mkdir(exist_ok=True)
        (self.dir_agente / "sin_usar.json").write_text(json.dumps(datos, ensure_ascii=False), encoding="utf-8")

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
        nombre = p.name.lower()
        if CREDENCIALES.search(contenido) and not any(x in nombre for x in (".example", ".sample", "test", ".env")):
            avisos.append("AVISO: parece una credencial escrita en el archivo (clave, token o contraseña). Léela de una "
                          "variable de entorno o de un .env incluido en .gitignore, y pide al usuario que la ponga.")
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
            return es_codigo(p) and len(texto.strip()) > 0
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
                if h.name in LISTADO_IGNORA:
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

    def leer(self, ruta: str, desde: int | None = None, lineas: int | None = None) -> str:
        p = self.ruta(ruta)
        if not p.is_file():
            return f"No existe el archivo: {ruta}. Usa listar para ver los nombres reales."
        todas = p.read_text(encoding="utf-8", errors="replace").splitlines()
        firma = (p.stat().st_mtime_ns, p.stat().st_size)
        # Una misión pedía lineas=86 para ver la línea 86: recibía otra vez las líneas 1-86, repetía y se detenía sin
        # contestar. Sin desde y con el principio ya leído, pedir 'lineas' no da nada nuevo: se le explica.
        if desde is None and lineas is not None and (previo := self.leidos.get(p)) and previo[:2] == firma:
            n, visto = int(lineas), previo[2]
            seguir = (f"Para seguir donde lo dejaste usa desde={visto + 1}; " if visto < len(todas) else
                      "Ya lo tienes entero; ")
            return (f"{self.rel(p)}: ya leíste este archivo desde la línea 1 (hasta la {visto} de {len(todas)}) y no ha "
                    f"cambiado. 'lineas' es CUÁNTAS líneas leer, no el número de línea. {seguir}para empezar en la "
                    f"línea {n}, desde={n}. Si de verdad quieres otra vez el principio, pide desde=1.")
        desde = max(1, int(desde or 1))
        sel = todas[desde - 1: desde - 1 + max(1, min(int(lineas or 200), 400))]
        cuerpo = "\n".join(f"{i}| {l}" for i, l in enumerate(sel, desde))
        fin = desde + len(sel) - 1
        if desde == 1:
            self.leidos[p] = (*firma, fin)
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
            if not f.is_file() or ignorado(f, self.raiz) or f.stat().st_size > 1_000_000:
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

    def escribir(self, ruta: str, contenido: str, anadir: bool = False) -> str:
        p = self.ruta(ruta)
        existia = p.exists()
        antes = p.read_text(encoding="utf-8", errors="replace") if existia else ""
        if anadir:  # archivo largo por partes: cada parte cabe en una respuesta del modelo
            completo = antes + ("" if not antes or antes.endswith("\n") else "\n") + contenido
            self._guardar(p, completo)
            return (f"OK añadido a {self.rel(p)}: ahora tiene {completo.count(chr(10)) + 1} líneas. Sigue con la "
                    "siguiente parte; cuando esté completo, verifica.")
        nota = ""
        if "\n" not in contenido and contenido.count("\\n") >= 3 and es_codigo(p):
            # el modelo mandó el código con saltos de línea escapados: se reparan
            contenido = contenido.replace("\\r\\n", "\n").replace("\\n", "\n").replace("\\t", "\t").replace('\\"', '"')
            nota = " (saltos de línea escapados corregidos)"
        if re.fullmatch(r"\s*[\[(<][^\])>\n]{0,80}[\])>]\s*", contenido) or re.fullmatch(r"\s*(\.\.\.|…)\s*", contenido):
            return (f"BLOQUEADO: el contenido para {self.rel(p)} es solo un marcador ({contenido.strip()[:60]!r}), no el "
                    "archivo. Escribe el contenido COMPLETO real (o usa editar para cambiar una parte).")
        if (existia and es_codigo(p) and len(antes) > 800 and len(contenido) < len(antes) * 0.15):
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
        return android.con_android(env)

    def abrir_android_studio(self, ruta: str = ".") -> str:
        """Abre el proyecto en Android Studio, que sincroniza Gradle e instala lo que falte del SDK."""
        exe = android.android_studio()
        if not exe:
            return ("No encuentro Android Studio. Pide al usuario que lo instale (https://developer.android.com/studio) "
                    "y lo abra una vez.")
        p = self.ruta(ruta)
        subprocess.Popen([str(exe), str(p)], cwd=p if p.is_dir() else p.parent, stdin=subprocess.DEVNULL,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, close_fds=True,
                         creationflags=subprocess.DETACHED_PROCESS | subprocess.CREATE_NEW_PROCESS_GROUP)
        self._anotar(f"abrir_android_studio {self.rel(p)}", "abierto", "")
        return (f"Android Studio se está abriendo con {self.rel(p)}: sincronizará Gradle y ofrecerá instalar los "
                "componentes del SDK que falten (las licencias y descargas las acepta el usuario en su ventana). "
                "Cuando termine, compila con verificar.")

    def experimentar(self, codigo: str, lenguaje: str = "python", segundos: int = 60) -> str:
        """Prueba mínima en .agente/experimento (fuera del proyecto entregable) para comprobar una API o una idea."""
        extension = {"python": ".py", "node": ".js", "javascript": ".js", "powershell": ".ps1", "bat": ".bat",
                     "cmd": ".bat"}.get(lenguaje.strip().lower())
        if not extension:
            return "Lenguajes de experimentar: python, node, powershell, bat. Para otros, usa un proyecto temporal."
        carpeta = self.dir_agente / "experimento"
        shutil.rmtree(carpeta, ignore_errors=True)
        carpeta.mkdir(parents=True, exist_ok=True)  # en Windows el borrado puede tardar: mkdir fallaba a veces
        archivo = carpeta / f"prueba{extension}"
        archivo.write_text(codigo, encoding="utf-8")
        if extension in (".ps1", ".bat"):
            for linea in (l.strip() for l in codigo.splitlines()):
                if linea and not COMENTARIO.match(linea) and (bloqueo := self._permitido(linea)):
                    return bloqueo
        orden = {".py": f'"{self.python_proyecto()}" prueba.py', ".js": "node prueba.js",
                 ".ps1": "powershell -NoProfile -ExecutionPolicy Bypass -File prueba.ps1", ".bat": "cmd /c prueba.bat"}
        limite = max(5, min(int(segundos or 60), 300))
        codigo, salida = correr(orden[extension], carpeta, limite, self._entorno())
        if codigo is None:
            self._anotar(f"experimentar {lenguaje}", f"TIMEOUT {limite}s", decodificar(salida))
            return f"TIMEOUT tras {limite} s (¿espera datos o no termina?):\n{recortar(decodificar(salida), 2000)}"
        texto = decodificar(salida).strip() or "(sin salida)"
        self._anotar(f"experimentar {lenguaje}", f"exit={codigo}", texto)
        return (f"exit={codigo} (experimento en .agente\\experimento, fuera del proyecto)\n"
                f"{recortar(texto, self.max_chars)}")

    def _scripts(self, comando: str) -> list[tuple[str, str]]:
        """(ruta relativa, línea) de los scripts del proyecto que lanza el comando, sin comentarios ni echo."""
        lineas = []
        for nombre in SCRIPT.findall(comando):
            try:
                p = self.ruta(nombre)
            except ValueError:
                continue
            if p.is_file() and p.stat().st_size < 200_000:
                texto = p.read_text(encoding="utf-8", errors="replace")
                lineas += [(self.rel(p), l.strip()) for l in texto.splitlines() if l.strip() and not COMENTARIO.match(l)]
        if (nombres := NPM_SCRIPT.findall(comando)) and (self.raiz / "package.json").is_file():
            try:
                scripts = json.loads((self.raiz / "package.json").read_text(encoding="utf-8")).get("scripts") or {}
            except (OSError, ValueError, AttributeError):
                scripts = {}
            lineas += [(f"package.json ({n})", str(scripts[n])) for n in nombres if n in scripts]
        return lineas

    def _permitido(self, comando: str) -> str | None:
        motivo, detalle = None, comando
        for origen, linea in [("", comando), *self._scripts(comando)]:
            if (cripto := transaccion(linea, self.raiz)) or externo(linea) or LICENCIAS.search(linea):
                motivo = cripto or (LICENCIA if LICENCIAS.search(linea) else EXTERNO)
                detalle = linea if not origen else f"{comando}\n(el script {origen} ejecuta: {linea[:200]})"
                break
            donde = linea if not origen else f"{comando}\n(el script {origen} ejecuta: {linea[:200]})"
            if motivo is None and MATAR_NOMBRE.search(linea):
                motivo, detalle = POR_NOMBRE, donde
            if motivo is None and PELIGROSOS.search(linea):
                motivo, detalle = DESTRUCTIVO, donde
            if motivo is None and {int(a or b) for a, b in MATAR_PID.findall(linea)} - set(self.propios()):
                motivo, detalle = AJENO, donde
        if not motivo:
            return None
        if self.confirmar and self._pedir_permiso(detalle if detalle != comando else comando, motivo):
            return None
        if motivo.startswith((CRIPTO, CRIPTO_CONFIG)):
            return ("BLOQUEADO: enviar transacciones, desplegar o cambiar la red de una blockchain necesita el permiso "
                    "del usuario y no lo dio. Trabaja con un nodo local (anvil, solana-test-validator) y la red "
                    "explícita, o explícale qué quieres hacer, en qué red y con qué cartera.")
        if motivo == LICENCIA:
            return ("BLOQUEADO: aceptar licencias lo decide el usuario. Pídele que las acepte en Android Studio (SDK "
                    "Manager) y vuelve a verificar.")
        if motivo == POR_NOMBRE:
            return ("BLOQUEADO: no hay ningún programa con ese nombre lanzado por el agente, y los que hay son de otros "
                    "programas o del usuario: no se cierran. Los tuyos se ven y se paran con procesos (listar, detener).")
        if motivo == AJENO:
            return ("BLOQUEADO: ese proceso no lo lanzó el agente: puede ser del usuario o de otro programa (también el "
                    "propio Aura). Los tuyos se paran con procesos(accion='detener', id=N). Si ocupa un puerto que "
                    "necesitas, pide al usuario que lo cierre o usa otro puerto.")
        if motivo == EXTERNO:
            return ("BLOQUEADO: conectarse o publicar fuera del equipo necesita el permiso del usuario y no lo dio. "
                    "Explícale qué quieres hacer y por qué, o pídele que lo haga él.")
        return "BLOQUEADO: comando potencialmente destructivo no autorizado. Busca otra forma o pide al usuario que lo haga."

    def _pedir_permiso(self, comando: str, motivo: str) -> bool:
        try:
            return bool(self.confirmar(comando, motivo))
        except TypeError:  # confirmar(comando) de integraciones anteriores
            return bool(self.confirmar(comando))

    def _npm_fuera(self, comando: str) -> str | None:
        """npm install en una carpeta del proyecto sin package.json: npm usaría el de una carpeta superior."""
        lineas = [(comando, True), *((l, False) for _, l in self._scripts(comando))]
        for linea, con_cd in lineas:
            if not (m := NPM_INSTALA.search(linea)):
                continue
            carpeta = self.raiz
            for c in CD.finditer(linea[:m.start()]) if con_cd else ():
                destino = (c.group(1) or c.group(2)).strip()
                if "%" not in destino:
                    carpeta = carpeta / destino  # una ruta absoluta reemplaza a la anterior
            carpeta = carpeta.resolve()
            if self.raiz not in (carpeta, *carpeta.parents):
                continue
            for d in (carpeta, *carpeta.parents):
                if (d / "package.json").is_file() or (d / "node_modules").is_dir():
                    if self.raiz not in (d, *d.parents):
                        return (f"BLOQUEADO: en {self.rel(carpeta)} no hay package.json y npm usaría el de {d}, fuera del "
                                "proyecto: instalaría o quitaría paquetes allí. Crea antes el package.json del proyecto "
                                "(con escribir, o npm init -y en esa carpeta) y repite el comando.")
                    break
        return None

    def propios(self) -> dict[int, str]:
        """{número: nombre} de los procesos vivos que lanzó el agente (los dice Windows: están en sus trabajos)."""
        pids = set()
        for p in self.procesos.values():
            pids |= set(p["trabajo"].pids()) if p.get("trabajo") else ({p["proc"].pid} if p["proc"].poll() is None else set())
        return {pid: procesos.nombre(pid) for pid in pids}

    def _cerrar_propios(self, comando: str) -> str | None:
        """taskkill /IM x o /PID n (o Stop-Process) solo: se cierran los procesos del agente que encajan; los del
        usuario con el mismo nombre no se tocan. None = no es eso, o no hay nada propio (lo decide _permitido)."""
        orden = comando.strip()
        while (m := COLA_SIN_EFECTO.search(orden)) and m.group(0):
            orden = orden[:m.start()]
        nombres = {(a or b).lower().removesuffix(".exe") for a, b in MATAR_NOMBRE.findall(orden)}
        pids = {int(a or b) for a, b in MATAR_PID.findall(orden)}
        if re.search(r"[&|]", orden) or not (nombres or pids):
            return None
        mios = self.propios()
        if pids - set(mios):
            return None
        objetivo = sorted(pids | {p for p, n in mios.items() if n.lower().removesuffix(".exe") in nombres})
        if not objetivo:
            return None
        cerrados = [f"{mios[p] or 'proceso'} ({p})" for p in objetivo if procesos.cerrar_pid(p)]
        self._anotar(comando, f"cerrados {len(cerrados)} propios", "")
        return (f"exit=0\nCerré {len(cerrados)} proceso(s) que lanzó el agente: {', '.join(cerrados) or 'ninguno'}. Los "
                "demás programas del equipo con ese nombre no se tocan (pueden ser del usuario). Para cerrar todo lo de un "
                "comando en segundo plano usa procesos(accion='detener', id=N).")

    def ejecutar(self, comando: str, segundos: int = 0, fondo: bool = False) -> str:
        # cmd ejecuta solo la primera línea y calla el resto: un «python -c "» de varias líneas daba exit=0 sin salida,
        # y una misión dio vueltas creyendo que el código no imprimía (754 comandos así en 45 misiones guardadas).
        if "\n" in comando.strip():
            return ("RECHAZADO: el comando tiene varias líneas y cmd solo ejecuta la primera; el resto se ignora sin "
                    "error (saldría exit=0 y nada más). Para código de Python de varias líneas usa experimentar(codigo=...)"
                    " o escríbelo en un .py y ejecútalo; para varias órdenes, únelas con && en una sola línea.")
        if (cerrado := self._cerrar_propios(comando)) is not None:
            return cerrado
        if (bloqueo := self._permitido(comando) or self._npm_fuera(comando)):
            return bloqueo
        if fondo:
            return self._fondo(comando)
        limite = max(5, min(int(segundos or self.timeout), 900))
        if INSTALACIONES.search(comando):
            limite = max(limite, MIN_INSTALACION)
        t0 = time.time()
        quedan = []
        codigo, salida = correr(comando, self.raiz, limite, self._entorno(), quedan)
        abierto = self._registrar_abiertos(comando, quedan)
        if codigo is None:
            self._anotar(comando, f"TIMEOUT {limite}s", decodificar(salida))
            return (f"TIMEOUT tras {limite}s (proceso detenido). Si es un servidor o programa que no termina, usa "
                    f"fondo=true. Salida parcial:\n{recortar(decodificar(salida), self.max_chars)}")
        self._anotar(comando, f"exit={codigo}", decodificar(salida))
        if codigo == 0:
            self.paso_prueba = self.paso
        elif self.checkpoints:
            self.fallos_desde_verde += 1
        texto = decodificar(salida).strip() or "(sin salida)"
        return (f"exit={codigo} ({time.time() - t0:.1f}s)\n{recortar(texto, self.max_chars)}{abierto}"
                + (self._sugerir_restaurar() if codigo else ""))

    def _registrar_abiertos(self, comando: str, quedan: list) -> str:
        """Lo que el comando dejó abierto (una ventana con start, un demonio de compilación) queda como proceso propio."""
        if not quedan:
            return ""
        proc, trabajo = quedan[0]
        nombres = ", ".join(sorted({procesos.nombre(p) for p in trabajo.pids()} - {"", "conhost.exe"}))
        if not nombres:  # solo queda la consola oculta, que se cierra sola: avisar haría gastar un turno en cerrarla
            trabajo.cerrar()
            return ""
        n = max(self.procesos, default=0) + 1
        self.procesos[n] = {"proc": proc, "trabajo": trabajo, "comando": comando, "abierto": True}
        return (f"\n(Sigue abierto lo que lanzó: {nombres}. Es tuyo: proceso #{n}; ciérralo con "
                f"procesos(accion='detener', id={n}) cuando no haga falta.)")

    def _anotar(self, comando: str, estado: str, salida: str) -> None:
        cola = " | ".join(l.strip() for l in salida.strip().splitlines()[-3:])[:240]
        self.evidencia.append(f"{comando[:220]} -> {estado}{(': ' + cola) if cola else ''}")
        del self.evidencia[:-25]

    def evidencia_texto(self) -> str:
        return "\n".join(f"- {e}" for e in self.evidencia)[-3000:]

    def _fondo(self, comando: str) -> str:
        n = max(self.procesos, default=0) + 1
        self.dir_agente.mkdir(exist_ok=True)
        log = self.dir_agente / f"proceso_{n}.log"
        f = open(log, "wb")  # noqa: SIM115 -- abierto mientras viva el proceso; se cierra al detenerlo
        proc, trabajo = procesos.lanzar(comando, shell=True, cwd=self.raiz, env=self._entorno(), stdin=subprocess.DEVNULL,
                                        stdout=f, stderr=subprocess.STDOUT,
                                        creationflags=subprocess.CREATE_NO_WINDOW | subprocess.CREATE_NEW_PROCESS_GROUP)
        self.procesos[n] = {"proc": proc, "trabajo": trabajo, "log": log, "archivo": f, "comando": comando}
        time.sleep(4)  # captura errores de arranque
        self._anotar(comando, f"proceso #{n} en segundo plano" if proc.poll() is None else f"terminó exit={proc.returncode}", "")
        estado = f"TERMINÓ (exit={proc.returncode})" if proc.poll() is not None else "en ejecución"
        if proc.poll() is None:
            self.paso_prueba = self.paso
        return (f"Proceso #{n} {estado}. Log inicial:\n{self._cola_log(n)}\nUsa procesos(accion='ver', id={n}) o "
                f"'detener'.{self._mismo_puerto(n)}")

    def _mismo_puerto(self, n: int) -> str:
        """Un servidor de la parte anterior seguía en el puerto 5000 con el código viejo; Windows dejó escuchar también
        al nuevo y las peticiones iban al viejo: la misión reinició el nuevo 8 veces sin ver su cambio. Se cierra el
        viejo (es del agente: salió de uno de sus trabajos)."""
        nuevo = self.procesos[n].get("trabajo")
        otros = {k: p for k, p in self.procesos.items() if k < n and p.get("trabajo") and self._vivo(k)}
        if not nuevo or not otros:
            return ""
        puertos, mios, vivo = procesos.escuchando(), set(nuevo.pids()), self._vivo(n)
        avisos = []
        for k, p in otros.items():
            suyos = set(p["trabajo"].pids())
            viejos = sorted(pt for pt, pids in puertos.items() if pids & suyos)
            compartidos = [pt for pt in viejos if puertos[pt] & mios]
            if compartidos:
                procesos.detener(p["proc"], p["trabajo"])
                avisos.append(f"Tu proceso #{k} ({p['comando'][:80]}) también escuchaba en el puerto "
                              f"{', '.join(map(str, compartidos))}, con el código de antes, y las peticiones podían ir "
                              "a él. Lo he detenido: ahora responde este.")
            elif viejos and not vivo:
                avisos.append(f"Sigue vivo tu proceso #{k} ({p['comando'][:80]}) en el puerto "
                              f"{', '.join(map(str, viejos))}. Si es el mismo programa, detenlo con "
                              f"procesos(accion='detener', id={k}) y vuelve a lanzarlo.")
        return "".join(f"\n[{a}]" for a in avisos)

    def _vivo(self, n: int) -> bool:
        p = self.procesos[n]
        return bool(p["trabajo"].pids()) if p.get("trabajo") else p["proc"].poll() is None

    def _cola_log(self, n: int, maximo: int = 2500) -> str:
        p = self.procesos[n]
        if not p.get("archivo"):
            return "(sin registro: lo dejó abierto un comando normal)"
        p["archivo"].flush()
        return recortar(decodificar(p["log"].read_bytes()).strip() or "(sin salida aún)", maximo)

    def procesos_cmd(self, accion: str = "listar", id: int = 0) -> str:
        if accion == "listar" or not id:
            if not self.procesos:
                return "No hay procesos en segundo plano."
            return "\n".join(f"#{n} {'vivo' if self._vivo(n) else 'terminado'}: {p['comando']}{self._miembros(n)}"
                             for n, p in self.procesos.items())
        if id not in self.procesos:
            return f"No existe el proceso #{id}."
        if accion == "detener":
            procesos.detener(self.procesos[id]["proc"], self.procesos[id].get("trabajo"))
            return f"Proceso #{id} detenido (solo lo que lanzó ese comando; nada más del equipo)."
        vivo = self._vivo(id)
        return f"#{id} {'vivo' if vivo else 'terminado'}. Log:\n{self._cola_log(id, self.max_chars)}{self._mismo_puerto(id)}"

    def _miembros(self, n: int) -> str:
        t = self.procesos[n].get("trabajo")
        nombres = sorted({procesos.nombre(p) for p in t.pids()} - {"", "conhost.exe"}) if t else []
        return f" ({', '.join(nombres)})" if nombres else ""

    def detener_todos(self) -> None:
        """Al cerrar el agente: lo de segundo plano se cierra; lo que dejó abierto un comando normal, no."""
        for p in self.procesos.values():
            if not p.get("abierto"):
                procesos.detener(p["proc"], p.get("trabajo"))
            if p.get("archivo"):
                p["archivo"].close()
            if p.get("trabajo"):
                p["trabajo"].cerrar()

    def _programas_bat(self, comandos: list[str]) -> list[str]:
        """Los programas externos que lanzan las órdenes, sin los de cmd ni los que están en la carpeta del proyecto."""
        programas = []
        for c in comandos:
            for orden in ordenes_bat(c)[::2]:
                orden = re.sub(r"(?i)^call\s+", "", orden.strip().lstrip("@("))
                palabras = re.sub(f"(?i)^{LANZADOR}", "", orden).split()
                p = palabras[0].lower() if palabras else ""
                p = "python" if p == "python3" else p
                if re.fullmatch(r"[\w+-]+", p) and p not in INTERNAS_CMD and p not in programas \
                        and not any((self.raiz / f"{p}{ext}").is_file() for ext in (".bat", ".cmd", ".exe")):
                    programas.append(p)
        return programas

    @staticmethod
    def _con_py(linea: str) -> str:
        partes = ordenes_bat(linea)
        partes[::2] = [re.sub(rf'(?i)^(\s*{LANZADOR})python3?(?=[\s"]|$)', r"\1%PY%", o) for o in partes[::2]]
        return "".join(partes)

    def _buscar_programas(self, comandos: list[str], usar_venv: bool) -> list[str]:
        """Si falta un programa, recupera el PATH del usuario; Python además puede estar solo con el lanzador py."""
        programas = self._programas_bat(comandos)
        python = usar_venv or "python" in programas
        if python and "python" not in programas:
            programas.insert(0, "python")
        if not programas:
            return []
        lineas = ['set "FALTA="']
        lineas += [("call python --version >nul 2>&1" if p == "python" else f"where /q {p}") + ' || set "FALTA=1"' for p in programas]
        lineas.append(PATH_DEL_USUARIO)
        if python:
            lineas += ['set "PY=python"', 'call python --version >nul 2>&1 || (call py --version >nul 2>&1 && set "PY=py")',
                       ('call %PY% --version >nul 2>&1 || (echo [ERROR] No encuentro Python: instálalo desde '
                        'https://www.python.org/downloads/ & set "RC=9009" & goto :error)')]
        return lineas

    def crear_bat(self, nombre: str, comandos: list, descripcion: str = "", usar_venv: bool = False) -> str:
        nombre = nombre if nombre.lower().endswith((".bat", ".cmd")) else nombre + ".bat"
        if isinstance(comandos, str):
            comandos = [c for c in comandos.splitlines() if c.strip()]
        lineas = ["@echo off", "setlocal EnableExtensions", "chcp 65001 >nul", 'cd /d "%~dp0"',
                  'set "PYTHONUTF8=1"',  # tildes y símbolos sin error aunque la salida vaya a un archivo o una tubería
                  f"title {Path(nombre).stem}", 'set "RC=0"']
        if descripcion:
            lineas.append(f"echo {descripcion}")
        lineas += self._buscar_programas(comandos, usar_venv)
        if usar_venv:
            lineas += ['if not exist ".venv\\Scripts\\python.exe" (',
                       "  echo Creando entorno virtual .venv ...",
                       "  %PY% -m venv .venv || (set \"RC=1\" & goto :error)", ")",
                       'call ".venv\\Scripts\\activate.bat"', 'set "PY=python"',
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
                       con_call(self._con_py(c), self.raiz),  # npm, npx, otro .bat...: sin call el .bat terminaría ahí
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

    def _crear_repo(self) -> bool:
        """Sin repositorio, el commit lo crea (el modelo se ahorra el fallo, el init y repetir). Nunca dentro de otro."""
        if self._git("rev-parse", "--git-dir")[0] == 0:
            return False
        try:
            dentro = subprocess.run(["git", "rev-parse", "--show-toplevel"], cwd=self.raiz, capture_output=True,
                                    timeout=30, creationflags=subprocess.CREATE_NO_WINDOW, check=False).returncode == 0
        except (OSError, subprocess.SubprocessError):
            return False
        return not dentro and self._git("init")[0] == 0

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
            creado = self._crear_repo()
            codigo, salida = self._git("add", "-A", *rutas)
            if codigo == 0:
                codigo, salida = self._git("commit", "-m", mensaje.strip(), *(rutas if ruta else []))
                if codigo == 0 and creado:
                    salida = "Repositorio git creado en la carpeta del proyecto.\n" + salida
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
    def _descargar(self, url: str, datos: bytes | None = None, maximo: int = 3_000_000) -> tuple[str, str]:
        req = urllib.request.Request(url, datos, {"User-Agent": UA, "Accept-Language": "es,en;q=0.8"})
        with urllib.request.urlopen(req, timeout=20) as r:
            tipo = r.headers.get("Content-Type", "")
            cuerpo = r.read(maximo)
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
        nota = ""
        if re.match(r"https?://", ruta or ""):  # la página la abre verificar; una URL daba «No existe» y otra vuelta
            ruta, nota = ".", "\n(verificar revisa el proyecto, también la web en el navegador; para mirar una URL usa ver_pantalla)"
        informe, errores = verificar(self.raiz, self.python_proyecto(), ruta, max(4, min(int(segundos), 60)), self.max_chars,
                                     previos=self.muertos_previos if self.errores_previos is None else self.errores_previos,
                                     idiomas_parcial=self.idiomas_parcial)
        subcarpeta = ruta not in ("", ".") and (self.raiz / ruta).is_dir()  # un archivo verifica todo el proyecto
        self.verificacion = {"paso": self.paso, "errores": errores, "informe": informe, "ruta": ruta if subcarpeta else "."}
        if errores == 0:
            self.paso_prueba = self.paso
            n = self._checkpoint()
            informe += (f"\nSIN ERRORES: guardado checkpoint #{n} (sin errores de verificar). Los avisos son secundarios: "
                        "atiende solo los de lógica sin conectar y no toques lo que ya funciona. Sigue con el plan.")
        else:
            self.fallos_desde_verde += 1
            informe += "\nCorrige los ERRORES (empieza por el primero), usa editar y vuelve a llamar a verificar."
            informe += self._sugerir_restaurar()
        return informe + nota

    def _fuentes(self) -> list[Path]:
        return [p for p in self.raiz.rglob("*") if p.is_file() and (es_codigo(p) or p.suffix.lower() in (".txt", ".md", ".cfg", ".ini"))
                and not ignorado(p, self.raiz) and p.stat().st_size < 500_000]

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

    def _anotar_checkpoints(self, texto: str) -> None:
        """Lo que escribió el usuario DESPUÉS de cada versión guardada: si avisaba de un fallo, esa versión lo tiene
        (una misión restauró dos veces la versión de la que el usuario acababa de decir que no arrancaba)."""
        self._cargar_checkpoints()
        if not self.checkpoints:
            return
        nota = " ".join(texto.split())[:240]
        for cp in self.checkpoints:
            cp["despues"] = [*cp.get("despues", []), nota][-2:]
        self.archivo_checkpoints.write_text(json.dumps(self.checkpoints, ensure_ascii=False), encoding="utf-8")

    @staticmethod
    def _dijo_despues(cp: dict) -> str:
        dijo = cp.get("despues") or []
        if not dijo:
            return ""
        return (" OJO: después de esa versión el usuario escribió: " + " / ".join(f"«{d}»" for d in dijo)
                + ". Si avisaba de un fallo, esa versión LO TIENE: corrígelo sobre ella, no vuelvas a restaurarla.")

    def _sugerir_restaurar(self) -> str:
        if self.checkpoints and self.fallos_desde_verde >= 3:
            cp = self.checkpoints[-1]
            return (f"\n[ATENCIÓN: en el paso {cp['paso']} tenías una versión sin errores de verificar (checkpoint "
                    f"#{len(self.checkpoints)}) y desde entonces llevas {self.fallos_desde_verde} fallos. Si no lo arreglas "
                    f"en el próximo intento, usa restaurar() y aplica cambios más pequeños.{self._dijo_despues(cp)}]")
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
        return (f"OK restaurado el checkpoint #{n} (pasó verificar sin errores, {cp.get('fecha', 'paso ' + str(cp['paso']))}): "
                f"{len(cp['archivos'])} archivos.{extra}{self._dijo_despues(cp)} Haz cambios pequeños y verifica tras cada uno.")

    def ver_pantalla(self, ruta: str = "", pregunta: str = "", segundos: int = 2) -> str:
        return pantalla.ver(self, ruta, pregunta, segundos)

    def preguntar(self, pregunta: str, opciones: list | None = None) -> str:
        pregunta = (pregunta or "").strip()
        opciones = [str(o).strip() for o in (opciones or []) if str(o).strip()][:5]
        if not pregunta:
            return "Indica 'pregunta' (una frase clara) y, si hay varias posibilidades, 'opciones' (2-5)."
        if self.preguntar_cb is None:
            return ("No hay nadie a quien preguntar ahora: elige la opción más razonable, dilo en el resumen y sigue. "
                    "Si de verdad no puedes seguir, termina explicando qué dato falta.")
        if self.preguntas >= MAX_PREGUNTAS:
            return (f"Ya preguntaste {MAX_PREGUNTAS} veces en esta tarea: decide tú con la opción más razonable y "
                    "explícalo en el resumen.")
        self.preguntas += 1
        respuesta = str(self.preguntar_cb(pregunta, opciones) or "").strip()
        return f"Respuesta del usuario: {respuesta}" if respuesta else "El usuario no respondió: elige la opción más razonable."

    def revisar_cmd(self, requisitos: str = "", enfoque: str = "") -> str:
        if not self.revisar_cb:
            return "Revisión no disponible (sin conexión con el modelo)."
        texto, _ = self.revisar_cb(requisitos, enfoque)
        return texto

    def guia(self, tema: str) -> str:
        return conocimiento.guia(tema)

    def version(self, paquete: str, ecosistema: str = "pypi") -> str:
        return conocimiento.version(self, paquete, ecosistema)

    def investigar(self, pregunta: str, web: bool = False) -> str:
        return conocimiento.investigar(self, pregunta, web=web)

    def calcular(self, codigo: str, segundos: int = 30) -> str:
        return matematicas.calcular(self, codigo, segundos)

    def resolver_sat(self, archivo: str, segundos: int = 60, modelo: str = "") -> str:
        return matematicas.resolver_sat(self, archivo, segundos, modelo)

    def aprender(self, leccion: str, tema: str = "") -> str:
        if tema.strip():
            return conocimiento.aprender_tema(tema, leccion)
        return lecciones.guardar(leccion)

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
            elif p.suffix.lower() in PATRONES_MAPA:
                nombres = re.findall(PATRONES_MAPA[p.suffix.lower()], texto, re.MULTILINE)[:10]
            lineas.append(f"- {self.rel(p)}: {', '.join(nombres)[:220]}" if nombres else f"- {self.rel(p)}")
        return "\n".join(lineas)[:maximo]

    def mapa_cmd(self, ruta: str = "") -> str:
        return mapa_repo.texto(self.raiz, ruta, self.max_chars, "" if ruta else resumen_idiomas(self.raiz))

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
        verificado = ""
        for i, l in enumerate(lineas):
            if patron.match(l):
                # primero lo barato: un paso que crea un archivo que aún no existe («Crear test_x.py con tests»)
                faltan = sorted(f for f in archivos_pedidos(l) if not self.existe(f)) if CREAR.search(l) else []
                if faltan:  # conducta: marcar como hecho un paso antes de hacerlo
                    return (f"RECHAZADO: el paso {numero} pide {', '.join(faltan)} y todavía no existe. "
                            "Créalo con escribir y después márcalo como hecho.")
                if (re.search(r"prob|test|verific|ejecut|comprob|revis", l, re.IGNORECASE)
                        and self.paso_prueba < max(self.paso_escritura, self.inicio_tarea)):
                    if self.paso_escritura < self.inicio_tarea:  # nada escrito en esta tarea: no hay qué verificar
                        return (f"RECHAZADO: el paso {numero} es de prueba/verificación y no hay ninguna ejecución con "
                                "éxito después de tu último cambio. Ejecuta la prueba (o verificar) y luego márcalo.")
                    # verifica ahora en vez de rechazar: el modelo se ahorra dos vueltas (verificar y volver a marcar)
                    informe = self.verificar_cmd(".")
                    if self.verificacion["errores"]:
                        return (f"RECHAZADO: el paso {numero} es de prueba/verificación y he verificado ahora el proyecto: "
                                f"tiene errores.\n{informe}")
                    verificado = " (verificado ahora: 0 errores)"
                base = re.sub(r" — .*$", "", l[6:])
                lineas[i] = f"- [x] {base}" + (f" — {nota.strip()[:150]}" if nota.strip() else "")
                self.archivo_plan.write_text("\n".join(lineas) + "\n", encoding="utf-8")
                pendientes = [l for l in lineas if l.startswith("- [ ]")]
                sig = f"Siguiente: {pendientes[0][6:]}" if pendientes else "Todos los pasos hechos: verifica y usa terminar."
                return f"Paso {numero} marcado como hecho{verificado}. {sig}"
        return f"No existe el paso {numero}.\n{self.plan_texto()}"

    def existe(self, nombre: str) -> bool:
        """El archivo existe en el proyecto (con su ruta o solo por nombre, en cualquier carpeta)."""
        if (self.raiz / nombre).exists():
            return True
        return any(not (LISTADO_IGNORA & set(p.parts)) for p in self.raiz.rglob(Path(nombre).name))

    def plan_texto(self) -> str:
        if not self.archivo_plan.exists():
            return ""
        return "\n".join(l for l in self.archivo_plan.read_text(encoding="utf-8").splitlines() if l.startswith("- ["))


def despachar(h: Herramientas, nombre: str, args: dict) -> str:
    tabla = {"plan": h.plan, "paso_hecho": h.paso_hecho, "listar": h.listar, "leer": h.leer, "buscar": h.buscar,
             "escribir": h.escribir, "editar": h.editar, "ejecutar": h.ejecutar, "procesos": h.procesos_cmd,
             "crear_bat": h.crear_bat, "web_buscar": h.web_buscar, "web_leer": h.web_leer, "investigar": h.investigar,
             "calcular": h.calcular, "resolver_sat": h.resolver_sat,
             "guia": h.guia, "version": h.version, "verificar": h.verificar_cmd, "revisar": h.revisar_cmd,
             "aprender": h.aprender, "restaurar": h.restaurar, "ver_pantalla": h.ver_pantalla, "git": h.git,
             "preguntar": h.preguntar, "experimentar": h.experimentar,
             "abrir_android_studio": h.abrir_android_studio, "mapa": h.mapa_cmd}
    if nombre not in tabla:
        return f"Herramienta desconocida: {nombre}. Disponibles: {', '.join(tabla)}, terminar."
    try:
        return tabla[nombre](**args)
    except TypeError as e:
        return f"Argumentos inválidos para {nombre}: {e}"
    except Exception as e:  # el modelo debe ver el error, no el agente caerse
        return f"ERROR en {nombre}: {type(e).__name__}: {e}"
