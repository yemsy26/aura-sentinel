"""Memoria de lo investigado: los párrafos útiles que investigar() leyó en la web, con su fuente y la fecha.

investigar() mira aquí antes de salir a internet y, si lo guardado cubre la pregunta, responde con ello. Es SQLite con
FTS5, que encuentra palabras exactas (nombres de APIs como invoke_handler o tauri.conf.json). Caduca pronto porque
las APIs cambian: a los DIAS_SIN_USO sin usarse o a los DIAS_MAX de leerse aunque se use; luego se vuelve a la web.
"""
import re
import sqlite3
import time
import unicodedata
from contextlib import closing
from typing import NamedTuple

from .config import RAIZ_INSTALACION

ARCHIVO = RAIZ_INSTALACION / "pasajes.db"
DIAS_SIN_USO = 30
DIAS_MAX = 90
# Fracción de las palabras de la pregunta que debe tener un pasaje (o la pregunta con la que se guardó) para
# responder de memoria. Alta a propósito: un dato viejo o de otra API engaña más que una búsqueda de más.
COBERTURA = 0.75
MAX_CHARS = 1500  # por pasaje: los párrafos más largos se parten por líneas
CANDIDATOS = 30
DIA = 86400
VACIAS = {"como", "para", "que", "con", "los", "las", "una", "uno", "unos", "unas", "del", "por", "sin", "sobre",
          "entre", "este", "esta", "esto", "estos", "estas", "hay", "hacer", "puedo", "cual", "cuales", "cuando",
          "donde", "mas", "son", "ser", "tiene", "ejemplo", "forma", "manera", "mejor", "mejores", "the", "and", "for",
          "with", "how", "what", "which", "why", "when", "where", "from", "into", "can", "does", "are", "not", "use",
          "using", "get", "example", "best", "way", "latest", "current", "you", "your"}
ESQUEMA = """
CREATE TABLE IF NOT EXISTS pasajes (id INTEGER PRIMARY KEY, texto TEXT NOT NULL UNIQUE, url TEXT NOT NULL,
    pregunta TEXT NOT NULL, leido REAL NOT NULL, usado REAL NOT NULL, usos INTEGER NOT NULL DEFAULT 0);
CREATE INDEX IF NOT EXISTS pasajes_url ON pasajes(url);
CREATE VIRTUAL TABLE IF NOT EXISTS pasajes_fts USING fts5(texto, pregunta, tokenize='unicode61 remove_diacritics 2');
"""


class Pasaje(NamedTuple):
    id: int
    texto: str
    url: str
    pregunta: str
    leido: float
    cobertura: float


def raices(texto: str) -> set[str]:
    """Palabras con significado, sin tildes ni mayúsculas y recortadas a 5 letras (command/commands,
    config/configure). Corta por '_' y '.' como FTS5: invoke_handler son 'invok' y 'handl'. Sin números sueltos."""
    plano = unicodedata.normalize("NFKD", texto.lower()).encode("ascii", "ignore").decode()
    return {p[:5] for p in re.findall(r"[a-z0-9]+", plano)
            if not p.isdigit() and p not in VACIAS and (len(p) >= 3 or any(c.isdigit() for c in p))}


def _partir(parrafo: str) -> list[str]:
    """Trozos de MAX_CHARS como mucho, cortados por líneas para no partir una línea de código."""
    trozos, actual = [], ""
    for linea in parrafo.splitlines():
        if actual and len(actual) + len(linea) + 1 > MAX_CHARS:
            trozos.append(actual)
            actual = ""
        actual = f"{actual}\n{linea}" if actual else linea
    if actual.strip():
        trozos.append(actual)
    return [t[:MAX_CHARS] for t in trozos]  # una sola línea enorme se recorta


def _abrir() -> sqlite3.Connection:
    con = sqlite3.connect(ARCHIVO, timeout=10)  # Aura y el MCP de Bionic pueden usarla a la vez
    con.execute("PRAGMA journal_mode=WAL")
    con.executescript(ESQUEMA)
    return con


def _borrar(con: sqlite3.Connection, ids: list[int]) -> None:
    con.executemany("DELETE FROM pasajes WHERE id = ?", [(i,) for i in ids])
    con.executemany("DELETE FROM pasajes_fts WHERE rowid = ?", [(i,) for i in ids])


def _caducar(con: sqlite3.Connection, ahora: float) -> None:
    _borrar(con, [i for (i,) in con.execute("SELECT id FROM pasajes WHERE usado < ? OR leido < ?",
                                            (ahora - DIAS_SIN_USO * DIA, ahora - DIAS_MAX * DIA))])


def guardar(pregunta: str, leidos: list[tuple[str, list[str]]], ahora: float | None = None) -> int:
    """Guarda los párrafos útiles de cada página (url, párrafos). Releer una página reemplaza lo guardado de ella.
    Devuelve cuántos pasajes nuevos hay; sin base de datos devuelve 0 e investigar sigue con la web."""
    ahora = time.time() if ahora is None else ahora
    nuevos = 0
    try:
        with closing(_abrir()) as con, con:
            for url, parrafos in leidos:
                _borrar(con, [i for (i,) in con.execute("SELECT id FROM pasajes WHERE url = ?", (url,))])
                for texto in (t for p in parrafos for t in _partir(p)):
                    cur = con.execute("INSERT OR IGNORE INTO pasajes (texto, url, pregunta, leido, usado) "
                                      "VALUES (?, ?, ?, ?, ?)", (texto, url, pregunta, ahora, ahora))
                    if cur.rowcount:
                        con.execute("INSERT INTO pasajes_fts (rowid, texto, pregunta) VALUES (?, ?, ?)",
                                    (cur.lastrowid, texto, pregunta))
                        nuevos += 1
    except (sqlite3.Error, OSError):
        return 0
    return nuevos


def buscar(pregunta: str, ahora: float | None = None) -> list[Pasaje]:
    """Pasajes vigentes que cubren al menos COBERTURA de las palabras de la pregunta, los más completos primero."""
    ahora = time.time() if ahora is None else ahora
    claves = raices(pregunta)
    if not claves:
        return []
    try:
        with closing(_abrir()) as con:
            with con:
                _caducar(con, ahora)
            filas = con.execute(
                "SELECT p.id, p.texto, p.url, p.pregunta, p.leido FROM pasajes_fts JOIN pasajes p ON "
                "p.id = pasajes_fts.rowid WHERE pasajes_fts MATCH ? ORDER BY bm25(pasajes_fts) LIMIT ?",
                (" OR ".join(f'"{c}"*' for c in sorted(claves)), CANDIDATOS)).fetchall()
    except (sqlite3.Error, OSError):
        return []
    hallados = []
    for i, texto, url, origen, leido in filas:
        comunes = len(claves & (raices(texto) | raices(origen)))
        if comunes >= min(2, len(claves)) and comunes / len(claves) >= COBERTURA:
            hallados.append(Pasaje(i, texto, url, origen, leido, comunes / len(claves)))
    return sorted(hallados, key=lambda p: -p.cobertura)  # estable: a igual cobertura, el orden de bm25


def usar(ids: list[int], ahora: float | None = None) -> None:
    """Los pasajes fundamentaron una respuesta: su plazo sin uso vuelve a empezar (no el de DIAS_MAX)."""
    ahora = time.time() if ahora is None else ahora
    try:
        with closing(_abrir()) as con, con:
            con.executemany("UPDATE pasajes SET usado = ?, usos = usos + 1 WHERE id = ?", [(ahora, i) for i in ids])
    except (sqlite3.Error, OSError):
        pass
