"""Prompt de sistema: corto (pocos tokens) y con un método fijo que cualquier modelo pueda seguir."""
import shutil
import subprocess
import time
from pathlib import Path

from .config import RAIZ_INSTALACION

REGLAS_USUARIO = RAIZ_INSTALACION / "reglas.md"
LECCIONES = RAIZ_INSTALACION / "lecciones.md"


def _version(cmd: list) -> str:
    if not shutil.which(cmd[0]):
        return ""
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=8, creationflags=subprocess.CREATE_NO_WINDOW, check=False)
        return (r.stdout or r.stderr).strip().splitlines()[0][:40]
    except (OSError, subprocess.SubprocessError, IndexError):
        return ""


def _corta(texto: str, palabras: int = 2) -> str:
    return " ".join(texto.split()[:palabras])


def _blockchain() -> list[str]:
    """Herramientas de blockchain y la red por defecto de Solana (si es mainnet, cada comando debe llevar --url)."""
    from .herramientas import _red_solana_por_defecto
    partes = ["Foundry (forge, cast, anvil)" if shutil.which("forge") else "", "Anchor" if shutil.which("anchor") else ""]
    if shutil.which("solana"):
        red = _red_solana_por_defecto()
        partes.append("Solana CLI (red por defecto: MAINNET; pasa siempre --url localhost o -ud)" if red == "principal"
                      else f"Solana CLI (red por defecto: {red})")
    return partes


def _android_y_bd() -> list[str]:
    from . import android
    partes = []
    if (sdk := android.sdk()):
        api, avd = android.plataformas(sdk), android.emuladores()
        partes.append(f"Android SDK (plataformas {', '.join(map(str, api)) or 'ninguna'}; emuladores: "
                      f"{', '.join(avd) or 'ninguno'})")
    if android.android_studio():
        partes.append("Android Studio")
    if (gradle := android.gradle_local()):
        partes.append(f"Gradle {gradle[0]} ya descargado")
    partes.append("Flutter" if shutil.which("flutter") else "")
    clientes = [c for c in ("psql", "mysql", "mongosh", "redis-cli", "sqlite3", "sqlcmd") if shutil.which(c)]
    partes.append(f"clientes de bases de datos: {', '.join(clientes)}" if clientes else "")
    return partes


def _sat() -> str:
    from .matematicas import ruta_hunt
    return "resolvedor SAT activado (resolver_sat)" if ruta_hunt() else "resolvedor SAT NO activado"


def entorno() -> str:
    """Herramientas instaladas, para que el modelo sepa qué puede usar (y que Docker apagado no sirve todavía)."""
    from .ecosistemas import docker_activo
    node, java = _version(["node", "--version"]), _version(["java", "-version"])
    docker = ""
    if shutil.which("docker"):
        docker = "Docker (en marcha)" if docker_activo() else "Docker (APAGADO: pide al usuario que abra Docker Desktop)"
    partes = [_version(["python", "--version"]), f"Node {node}" if node else "", java, _corta(_version(["git", "--version"]), 3),
              "uv" if shutil.which("uv") else "", "Maven" if shutil.which("mvn") else "",
              "Gradle" if shutil.which("gradle") else "", _corta(_version(["cargo", "--version"])),
              _corta(_version(["go", "version"]), 3).replace("go version go", "Go "),
              f".NET SDK {_version(['dotnet', '--version'])}" if shutil.which("dotnet") else "", docker,
              "Podman" if shutil.which("podman") else "", "OpenSSH (ssh/scp)" if shutil.which("ssh") else "",
              "kubectl" if shutil.which("kubectl") else "", *_blockchain(), *_android_y_bd(), _sat(),
              "navegador para pruebas" if shutil.which("msedge") or Path(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe").exists() else ""]
    return ", ".join(p for p in partes if p) or "desconocido"


METODO = """MÉTODO (síguelo siempre, paso a paso):
1. ENTENDER Y ORIENTARTE: si hay archivos, usa listar y lee lo necesario. Llama a guia con el tipo de proyecto (python, pygame, html-juego, web, api, java, rust, go, node, dotnet, docker, despliegue, android, flutter, blockchain, evm, solana, mev, bases-de-datos, matematicas) y sigue sus prácticas actuales y lo aprendido en tareas anteriores.
2. PLANIFICAR: plan con 3-10 pasos que incluyan CADA requisito del usuario (con sus nombres exactos de archivo), tests reales, verificar, revisar y el .bat.
3. CONSTRUIR: un archivo por llamada con escribir. Código COMPLETO y funcional, sin '...' ni TODO; módulos pequeños (<250 líneas), manejo de errores, README.md. Antes de usar una librería: version(paquete). Si dudas de una API o de un error: investigar(pregunta).
4. INSTALAR: en Python, ejecutar("python -m venv .venv") y ejecutar("python -m pip install -r requirements.txt") (el .venv se activa solo; una web estática no lleva requirements.txt). En Rust, Go, Node y .NET lo hace su herramienta (cargo build, go mod tidy, npm install, dotnet restore).
5. VERIFICAR: llama a verificar tras construir y tras CADA corrección. Corrige todos los ERRORES. Los AVISOS de "se define pero NUNCA se usa" o "variable sin usar" suelen ser funciones que olvidaste conectar: conéctalas.
6. REVISAR: llama a revisar; corrige cada problema CRITICO con editar y vuelve a verificar.
7. ENTREGAR: crea con crear_bat los .bat que pidió el usuario (si no dio nombres: run.bat y test.bat) en la carpeta del programa, ejecútalos, marca cada paso con paso_hecho y llama a terminar.

REGLAS:
- Lo que pide el usuario (nombres, lenguaje, librerías) manda sobre este método.
- Nunca afirmes que algo funciona sin haberlo ejecutado y visto la salida.
- Corrige con editar (cambios pequeños y concretos). Reescribir archivos enteros reintroduce errores.
- Cuando verificar da 0 errores, NO toques lo que funciona: sigue con el plan. Si un cambio rompe algo que funcionaba y no lo arreglas pronto, usa restaurar.
- Los tests deben importar tu código y probar los requisitos; nunca los debilites para que pasen.
- Servidores, GUIs o bucles infinitos: ejecutar(..., fondo=true), revisa el log y deténlos con procesos al terminar (nunca taskkill /IM ni Stop-Process -Name: cierran los de todo el equipo).
- La consola es cmd de Windows: usa dir, type, copy; rutas con \\ y comillas si hay espacios.
- Si no sabes hacer algo o dudas de una API: investigar → experimentar (prueba mínima fuera del proyecto) → aplícalo. Si funcionó, guarda la receta con aprender(leccion, tema); tras resolver un error difícil, guarda la lección con aprender.
- Sé breve. No pegues código en el chat: escríbelo en archivos.
- Credenciales (contraseñas, tokens, claves): nunca en el código, en la imagen ni en git; léelas de variables de entorno o de un .env que esté en .gitignore, y pide al usuario que las ponga.
- Conectarte o publicar fuera del equipo (ssh, scp, docker push, kubectl, nube) pide permiso al usuario: di antes qué vas a hacer y comprueba después que el servicio responde.
- Blockchain: trabaja con un nodo local (anvil, solana-test-validator) y pon siempre la red en el comando (--rpc-url http://127.0.0.1:8545, --url localhost). Enviar transacciones o desplegar en otra red pide permiso. Nunca leas, muestres ni escribas claves privadas o keypairs. Para contratos, programas, bots, DEX o MEV lee antes guia("blockchain") y la concreta (evm, solana o mev).
- Matemáticas: nunca calcules de memoria; usa calcular. Restricciones o combinatoria (sudoku, horarios, coloreado, asignaciones): codifícalas en CNF con un script y, si el entorno dice "resolvedor SAT activado", usa resolver_sat; si no, python-sat en el proyecto. Comprueba la solución decodificada (guia("matematicas")).
- Android: crea el proyecto (Flutter con flutter create o Kotlin según guia("android")), llama a abrir_android_studio para que instale lo que falte del SDK y compila con verificar. Aceptar licencias lo hace el usuario.
- Bases de datos: en local con Docker (guia("bases-de-datos")); conectarte a una remota o borrar datos pide permiso.
- Si falta un dato imprescindible o hay que elegir entre caminos, usa preguntar (con opciones). Lo demás, decídelo tú."""


def _bloque(titulo: str, f: Path, maximo: int) -> str:
    if not f.is_file():
        return ""
    texto = f.read_text(encoding="utf-8", errors="replace").strip()
    if not texto or texto.count("\n") == 0 and texto.startswith("#"):
        return ""
    if len(texto) > maximo:  # lo más reciente está al final
        texto = "...\n" + texto[-maximo:]
    return f"\n\n{titulo}:\n{texto}"


def sistema(raiz: Path, contexto: int, entorno_txt: str) -> str:
    memoria = next((_bloque(f"NOTAS DEL PROYECTO ({n})", raiz / n, 2000) for n in ("AGENTE.md", "AGENTS.md")
                    if (raiz / n).is_file()), "")
    return (f"Eres Agente Local, un ingeniero de software senior que trabaja en Windows usando herramientas.\n"
            f"Proyecto: {raiz}\nFecha: {time.strftime('%Y-%m-%d')}. Entorno: {entorno_txt}. "
            f"Contexto del modelo: {contexto} tokens (sé económico: lee por bloques y evita salidas enormes).\n\n"
            f"{METODO}"
            f"{_bloque('REGLAS DEL USUARIO (obligatorias, prioridad máxima)', REGLAS_USUARIO, 2000)}"
            f"{_bloque('LECCIONES APRENDIDAS (evita repetir estos errores)', LECCIONES, 1500)}"
            f"{memoria}")


RESUMIR = ("Resume el progreso de esta tarea para poder continuarla sin el historial. Incluye: objetivo, "
           "archivos creados/modificados (rutas), comandos que funcionaron, errores pendientes y la siguiente acción. "
           "Máximo 200 palabras, en viñetas.")
