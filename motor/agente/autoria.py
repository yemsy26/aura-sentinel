"""Qué código del proyecto escribió Aura: `verificar` solo ejecuta el que es suyo o cuando la tarea lo ha cambiado.

m4198: en una auditoría de solo lectura de un repositorio ajeno, `verificar` lanzaba pytest desde la raíz y un script del repositorio, y
pedía npm install. «La carpeta empezó vacía» no sirve para decidirlo: allí empezó vacía y el repositorio se clonó después.
"""
import json
from pathlib import Path

from .proyecto import ignorado

FUENTE = {".py", ".js", ".mjs", ".cjs", ".ts", ".tsx", ".jsx", ".vue", ".svelte", ".html", ".htm", ".java", ".kt", ".kts", ".rs",
          ".go", ".cs", ".fs", ".vb", ".c", ".h", ".cpp", ".hpp", ".sol", ".dart", ".bat", ".cmd", ".ps1", ".sh", ".gradle"}
# No son código, pero deciden qué se instala, se compila o se ejecuta.
MANIFIESTOS = {"package.json", "tsconfig.json", "requirements.txt", "pyproject.toml", "setup.cfg", "pytest.ini", "tox.ini",
               "cargo.toml", "go.mod", "pom.xml", "makefile", "foundry.toml", "anchor.toml", "pubspec.yaml",
               "docker-compose.yml", "docker-compose.yaml", "compose.yml", "compose.yaml"}
DE_PROYECTO = {".csproj", ".fsproj", ".vbproj", ".sln", ".slnx", ".dockerfile"}
# Medido en 172 carpetas de misiones: 152 llegan al 80 %; las que traían código previo se quedan en el 75 % o menos.
PROPIO = 0.8
ARCHIVO = "escritos.json"  # estado del motor (estado_motor.ESTADO): el modelo no puede escribirlo


def es_ejecutable(p: Path) -> bool:
    nombre = p.name.lower()
    return (p.suffix.lower() in FUENTE | DE_PROYECTO or nombre in MANIFIESTOS or nombre == "dockerfile"
            or nombre.startswith("dockerfile."))


class Autoria:
    def __init__(self, raiz: Path, dir_agente: Path):
        self.raiz, self.archivo = raiz, dir_agente / ARCHIVO
        try:
            datos = json.loads(self.archivo.read_text(encoding="utf-8"))
            self.escritos, self.tarea_cambia = set(datos["escritos"]), bool(datos["tarea_cambia"])
            # Errores normalizados que tenía el código ajeno antes de la primera escritura de la tarea (None: sin foto).
            self.antes: set[str] | None = None if datos.get("antes") is None else set(datos["antes"])
        except (OSError, ValueError, KeyError, TypeError):
            self.escritos, self.tarea_cambia, self.antes = set(), False, None

    def _clave(self, p: Path) -> str:
        return p.relative_to(self.raiz).as_posix().lower()  # Windows no distingue mayúsculas

    def ajenos(self, rutas: list[Path], conocidos: set[str] | frozenset = frozenset()) -> dict[str, Path]:
        """{clave: ruta} de las que Aura no ha escrito y no están en conocidos."""
        return {c: p for p in rutas if (c := self._clave(p)) not in self.escritos and c not in conocidos}

    def _guardar(self) -> None:
        self.archivo.parent.mkdir(exist_ok=True)
        datos = {"escritos": sorted(self.escritos), "tarea_cambia": self.tarea_cambia,
                 "antes": None if self.antes is None else sorted(self.antes)}
        self.archivo.write_text(json.dumps(datos, ensure_ascii=False), encoding="utf-8")

    def anotar(self, p: Path) -> None:
        """Aura acaba de crear o cambiar p (escribir, editar, crear_bat)."""
        if es_ejecutable(p):
            self.escritos.add(self._clave(p))
            self.tarea_cambia = True
            self._guardar()

    def cambia(self, si: bool) -> None:
        """El modo grande sabe por su plan si va a tocar código."""
        if si != self.tarea_cambia:
            self.tarea_cambia = si
            self._guardar()

    def nueva(self) -> None:
        """Una petición nueva empieza sin cambios y sin foto de errores («continúa» no llama aquí)."""
        if self.tarea_cambia or self.antes is not None:
            self.tarea_cambia, self.antes = False, None
            self._guardar()

    def recordar(self, errores: set[str]) -> None:
        self.antes = errores
        self._guardar()

    def es_de_aura(self) -> bool:
        archivos = [p for p in self.raiz.rglob("*") if es_ejecutable(p) and p.is_file() and not ignorado(p, self.raiz)]
        archivos = [p for p in archivos if p.suffix.lower() in FUENTE] or archivos  # sin código fuente, los manifiestos
        return bool(archivos) and sum(self._clave(p) in self.escritos for p in archivos) >= PROPIO * len(archivos)

    def puede_ejecutar(self) -> bool:
        return self.tarea_cambia or self.es_de_aura()
