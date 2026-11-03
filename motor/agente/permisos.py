"""Qué puede hacer el agente sin preguntar: comandos peligrosos, salidas del equipo y transacciones en blockchain.
Solo mira el texto del comando; quien decide qué hacer con la respuesta es herramientas.py."""
import os
import re
from pathlib import Path

# Git que pierde trabajo o reescribe historia (o publica: push).
GIT_DESTRUCTIVO = (r"push|reset\s+--hard|clean\s+-\w*f|checkout\s+(-f\b|--\s|\.(\s|$))|restore\b(?!.*--staged)|"
                   r"branch\s+-D|stash\s+(drop|clear)|rebase|filter-branch|update-ref\s+-d|reflog\s+expire|rm\b|"
                   r"commit\b.*--amend|switch\b.*(\s-f\b|--discard-changes)")
PELIGROSOS = re.compile(
    r"\b(format\s+[a-z]:|diskpart|shutdown|bcdedit|cipher\s+/w|takeown|icacls|reg\s+(delete|add)|"
    rf"rd\s+/s|rmdir\s+/s|del\s+.*/[sq]|remove-item\b.*-recurse|rm\s+-r|git\s+({GIT_DESTRUCTIVO}))|"
    # bases de datos: borrar tablas, bases o todos los datos, y los volúmenes donde Docker los guarda
    r"\bdrop\s+(table|database|schema|collection|view)\b|\btruncate\s+(table\s+)?[\w\"`]|\bdropdb\b|"
    r"\.dropDatabase\(|\bflush(all|db)\b|\bdelete\s+from\s+[\w\"`.]+\s*(;|\"|'|$)|"
    r"\bdocker\s+volume\s+(rm|prune)\b|\bdocker[\s-]+compose\s+down\b[^&|]*\s(-v|--volumes)\b|"
    r"\bdocker\s+system\s+prune\b|"
    # matar procesos por nombre cierra los de todo el equipo (consolas, el IDE...): los propios se paran con procesos
    r"\btaskkill\b[^&|]*\s/IM\s|\bstop-process\b[^&|]*-name\b|\b(pkill|killall)\b|"
    r"\bwmic\s+process\b.*\b(delete|terminate)\b",
    re.IGNORECASE)
# Cerrar un proceso por su número: solo los que lanzó el agente. Los demás pueden ser del usuario o del propio Aura
# (una misión cerró a la fuerza el que tenía el puerto 1420 sin saber de quién era).
MATAR_PID = re.compile(r"\btaskkill\b[^&|]*?/PID\s+(\d+)|\bstop-process\b[^&|]*?-Id\s+(\d+)", re.IGNORECASE)
MATAR_NOMBRE = re.compile(r'\btaskkill\b[^&|]*?/IM\s+"?([^\s"&|]+)|\bstop-process\b[^&|]*?-Name\s+"?([^\s"&|,]+)',
                          re.IGNORECASE)
COLA_SIN_EFECTO = re.compile(r"\s*(?:\|\|\s*echo\b[^&|]*|[12]?>\s*nul)\s*$", re.IGNORECASE)  # 2>nul, || echo ...
# Instalan o quitan paquetes: sin package.json en la carpeta, npm sube y usa el de una carpeta superior (p. ej. el de
# C:\Users\<usuario>) y cambia SUS paquetes. Una prueba lo hizo: quitó 37 paquetes de la carpeta del usuario.
NPM_INSTALA = re.compile(r"\b(?:npm|pnpm|yarn)(?:\.cmd)?\s+(?:install|i|ci|add|uninstall|un|remove|rm|update|up|"
                         r"upgrade|prune|dedupe)\b", re.IGNORECASE)
# Salen del equipo (servidores, registros de imágenes, clústeres, nubes, paquetes publicados): siempre con permiso.
EXTERNOS = re.compile(
    r"\b(ssh|scp|sftp|rsync|plink|pscp)(\.exe)?\s|\bgit\s+push\b|\b(docker|podman)\s+(push|login)\b|"
    r"\bkubectl\s+(apply|create|delete|replace|patch|scale|rollout|edit|set|label|annotate|exec|cp|drain|cordon)\b|"
    r"\bhelm\s+(install|upgrade|uninstall|rollback|push)\b|\bterraform\s+(apply|destroy|import)\b|"
    r"\bpulumi\s+(up|destroy)\b|\b(npm|pnpm|yarn)\s+publish\b|\bcargo\s+publish\b|\bdotnet\s+nuget\s+push\b|"
    r"\btwine\s+upload\b|\bgh\s+(release|repo|pr|gist|secret)\s+(create|delete|edit|merge|upload|set)\b|"
    r"\b(az|aws|gcloud|flyctl|vercel|netlify|heroku|railway|doctl)\s|\bhardhat\s+verify\b|\bforge\s+verify-contract\b",
    re.IGNORECASE)
HTTP = re.compile(r"\b(curl|wget|Invoke-WebRequest|Invoke-RestMethod|iwr|irm)\b", re.IGNORECASE)
HTTP_ESCRIBE = re.compile(r"(-X\s*|--request\s+|-Method\s+)(POST|PUT|PATCH|DELETE)\b|(^|\s)(-d|--data(-raw|-binary)?|--json|"
                          r"-F|--form|-T|--upload-file)\s", re.IGNORECASE)
LOCALES = {"localhost", "127.0.0.1", "0.0.0.0", "host.docker.internal"}
# Aceptar licencias (SDK de Android) es decisión del usuario.
LICENCIAS = re.compile(r"\bsdkmanager(\.bat)?\b.*--licenses|\bflutter\s+doctor\s+--android-licenses", re.IGNORECASE)
LICENCIA = "aceptar licencias del SDK de Android (lo decide el usuario)"
# Bases de datos en otra máquina (servidor, nube): conectarse a ellas cuenta como salir del equipo.
CLIENTE_BD = re.compile(r"\b(psql|pg_dump|pg_dumpall|pg_restore|mysql|mysqldump|mariadb|mongosh|mongo|mongodump|"
                        r"mongorestore|mongoimport|redis-cli|sqlcmd|cqlsh|clickhouse-client)(\.exe)?\b", re.IGNORECASE)
HOST_BD = re.compile(r"(?:^|\s)(?:-h\s*|--host[=\s]+|-S\s+)([^\s\"',;]+)")
URL_BD = re.compile(r"\b(?:postgres(?:ql)?|mysql|mariadb|mongodb(\+srv)?|rediss?|sqlserver)://(?:[^@/\s]+@)?([^/:\s,?\"']+)",
                    re.IGNORECASE)
# Scripts del proyecto que lanza un comando: lo que llevan dentro también necesita permiso.
SCRIPT = re.compile(r'(?:^|[\s&|;(])"?((?:[\w.:-]+[\\/])*[\w.-]+\.(?:bat|cmd|ps1|sh))"?(?=$|[\s&|;)])', re.IGNORECASE)
COMENTARIO = re.compile(r"\s*(rem\b|::|#|echo\b|@echo\b|write-host\b)", re.IGNORECASE)
NPM_SCRIPT = re.compile(r"\b(?:npm|pnpm|yarn|bun)(?:\.cmd)?\s+(?:run(?:-script)?\s+)?([\w:.-]+)", re.IGNORECASE)
DESTRUCTIVO = "ejecutar un comando potencialmente destructivo"
AJENO = "cerrar un proceso que no lanzó el agente (puede ser del usuario o del propio Aura)"
POR_NOMBRE = "cerrar por su nombre TODOS esos programas del equipo, también los que tiene abiertos el usuario"
EXTERNO = "conectarse o publicar fuera de este equipo (servidor, registro de imágenes, clúster o nube)"
# Credenciales reales escritas en un archivo: deben ir en variables de entorno o en un .env fuera de git.
CREDENCIALES = re.compile(
    r"-----BEGIN (RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----|\bAKIA[0-9A-Z]{16}\b|\bgh[pousr]_[A-Za-z0-9]{36}\b|"
    r"\bxox[baprs]-[A-Za-z0-9-]{10,}|\bsk_live_[A-Za-z0-9]{16,}|\bAIza[0-9A-Za-z_-]{35}\b|"
    r"\b(password|passwd|secret|api_?key|token)\w*[\"']?\s*[:=]\s*[\"'][^\"'\s]{8,}[\"']", re.IGNORECASE)
# Blockchain: firman o envían transacciones o despliegan. Fuera de un nodo local, siempre con permiso: sin red
# explícita no se supone local, porque la CLI puede apuntar por defecto a mainnet con una cartera con fondos.
CRIPTO = "enviar una transacción o desplegar en una blockchain fuera de un nodo local (puede mover fondos reales)"
CRIPTO_CONFIG = "cambiar la red o la cartera por defecto de la CLI de Solana (afecta a todo el equipo)"
SOLANA = re.compile(
    r"\bsolana(?:\.exe)?\s+(?:transfer|airdrop|deploy|program\s+(?:deploy|write-buffer|upgrade|close|extend|set-\S+)|"
    r"(?:create|delegate|deactivate|withdraw|split|merge|redelegate|authorize)-\S+|stake-\S+|vote-\S+|"
    r"close-vote-account|new-nonce)\b|\bspl-token(?:\.exe)?\s+"
    r"(?!(?:balance|accounts|supply|account-info|display|address|help|-h|--help|-V|--version)\b)\S", re.IGNORECASE)
SOLANA_CONFIG = re.compile(r"\bsolana(?:\.exe)?\s+config\s+set\b", re.IGNORECASE)
ANCHOR = re.compile(r"\banchor\s+(?:deploy|upgrade|migrate|idl\s+(?:init|upgrade|erase-authority|set-authority|close|"
                    r"write-buffer|set-buffer))\b|\banchor\s+(test|run)\b", re.IGNORECASE)
EVM = re.compile(r"\bforge\s+(?:script\b.*--broadcast|create)\b|\bcast\s+(?:send|publish)\b|\bcast\s+rpc\s+eth_send",
                 re.IGNORECASE)
HARDHAT = re.compile(r"\bhardhat\s+(?:run|test|console|deploy|ignition\s+deploy)\b", re.IGNORECASE)
OTRAS_CADENAS = re.compile(
    r"\bsui\s+client\s+(?:call|publish|upgrade|transfer\S*|pay\S*|merge-coin|split-coin|ptb)\b|\baptos\s+(?:move\s+"
    r"(?:publish|run|run-script|deploy-object|upgrade-object)|account\s+(?:transfer|create|fund-with-faucet))\b",
    re.IGNORECASE)
URL_SOLANA = re.compile(r"(?:^|\s)(?:--url|-u)(?:\s+|=)?([^\s\"']+)")
CLUSTER_ANCHOR = re.compile(r"--provider\.cluster(?:\s+|=)([^\s\"']+)")
RPC_EVM = re.compile(r"(?:^|\s)(?:--rpc-url|-r|--fork-url)(?:\s+|=)([^\s\"']+)")
RED_HARDHAT = re.compile(r"--network(?:\s+|=)([^\s\"']+)")
SUFIJO_RED = {"principal": " — RED PRINCIPAL (mainnet)", "prueba": " — red de prueba",
              "desconocida": ""}


def _red(valor: str | None) -> str:
    """local, prueba, principal o desconocida para una URL o un nombre de red (Solana, Anchor, Foundry, Hardhat)."""
    if not valor:
        return "desconocida"
    v = valor.lower().strip("\"'")
    host = (re.match(r"\w+://([^/:]+)", v) or [None, ""])[1]
    if v in ("l", "localhost", "localnet", "hardhat") or host in LOCALES:
        return "local"
    if re.search(r"mainnet|^m$|^main$|^mainnet-beta$", v):
        return "principal"
    if re.search(r"devnet|testnet|sepolia|holesky|hoodi|amoy|goerli|fuji|^d$|^t$", v):
        return "prueba"
    return "desconocida"


def red_solana_por_defecto() -> str:
    config = Path(os.environ.get("USERPROFILE", Path.home())) / ".config" / "solana" / "cli" / "config.yml"
    try:
        m = re.search(r"^json_rpc_url:\s*['\"]?([^'\"\s]+)", config.read_text(encoding="utf-8"), re.MULTILINE)
    except OSError:
        return "desconocida"
    return _red(m.group(1)) if m else "desconocida"


def _en_archivo(raiz: Path | None, nombres: tuple[str, ...], patron: str) -> str | None:
    for nombre in nombres:
        try:
            m = re.search(patron, (raiz / nombre).read_text(encoding="utf-8", errors="replace"), re.MULTILINE)
        except (OSError, TypeError):
            continue
        if m:
            return m.group(1)
    return None


def transaccion(comando: str, raiz: Path | None = None) -> str | None:
    """Motivo si el comando firma o envía una transacción, despliega o cambia la red de una blockchain fuera de un nodo
    local; None si no. Las lecturas (saldo, estado) no cuentan."""
    if SOLANA_CONFIG.search(comando):
        return CRIPTO_CONFIG
    if SOLANA.search(comando):
        m = URL_SOLANA.search(comando)
        red = _red(m.group(1)) if m else red_solana_por_defecto()
    elif (m := ANCHOR.search(comando)):
        c = CLUSTER_ANCHOR.search(comando)
        valor = c.group(1) if c else _en_archivo(raiz, ("Anchor.toml",), r"^\s*cluster\s*=\s*['\"]([^'\"]+)")
        red = "local" if valor is None and m.group(1) else _red(valor)  # test/run sin Anchor.toml: no despliega
    elif EVM.search(comando):
        m = RPC_EVM.search(comando)
        red = _red(m.group(1) if m else os.environ.get("ETH_RPC_URL"))
    elif HARDHAT.search(comando):
        m = RED_HARDHAT.search(comando)
        valor = m.group(1) if m else os.environ.get("HARDHAT_NETWORK") or _en_archivo(
            raiz, ("hardhat.config.ts", "hardhat.config.js", "hardhat.config.cjs", "hardhat.config.mjs"),
            r"defaultNetwork\s*:\s*['\"]([^'\"]+)")
        red = _red(valor or "hardhat")
    elif OTRAS_CADENAS.search(comando):
        red = "desconocida"
    else:
        return None
    return None if red == "local" else CRIPTO + SUFIJO_RED[red]


def _host_remoto(host: str) -> bool:
    """Un nombre sin punto (servicio de Docker Compose, contenedor) o localhost es local; lo demás, otra máquina."""
    host = host.lower().strip("[]")
    if host.startswith(("/", "\\")):  # socket o tubería local (mysql -S /tmp/mysql.sock)
        return False
    return host not in LOCALES and not host.startswith("127.") and ("." in host or ":" in host)


def bd_remota(comando: str) -> bool:
    if any(m.group(1) or _host_remoto(m.group(2)) for m in URL_BD.finditer(comando)):  # mongodb+srv: siempre nube
        return True
    return bool(CLIENTE_BD.search(comando)) and any(_host_remoto(h) for h in HOST_BD.findall(comando))


def externo(comando: str) -> bool:
    """¿El comando actúa fuera del equipo? Las peticiones HTTP que escriben solo cuentan si no van a localhost."""
    if EXTERNOS.search(comando) or bd_remota(comando):
        return True
    if HTTP.search(comando) and HTTP_ESCRIBE.search(comando):
        return any(h.lower() not in LOCALES for h in re.findall(r"https?://([^/\s:'\"]+)", comando))
    return False
