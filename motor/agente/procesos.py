"""Control exacto de lo que lanza el agente: cada comando va en su propio «trabajo» de Windows (Job Object).

Antes se cerraba con taskkill /T, que decide qué es «hijo» por el número del proceso padre. Windows reutiliza esos
números, así que podía cerrar programas del usuario que no tenían nada que ver, y el agente no sabía qué era suyo (una
misión cerró la consola donde el usuario tenía abierto el servidor de Aura). Un trabajo sabe exactamente qué procesos
contiene (también los que abren sus hijos, como un `start`) y se cierra entero, sin tocar nada más."""
import ctypes
import os
import subprocess
from ctypes import wintypes
from pathlib import Path

EN_WINDOWS = os.name == "nt"
SUSPENDIDO = 0x00000004  # CREATE_SUSPENDED: se mete en el trabajo antes de que pueda lanzar nada
MATAR_AL_CERRAR = 0x00002000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: si el motor muere, mueren con él
LISTA_PROCESOS, LIMITES = 3, 9  # JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation
CONSULTAR, TERMINAR = 0x1000, 0x0001  # PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE


class _Basicos(ctypes.Structure):
    _fields_ = [("PerProcessUserTimeLimit", ctypes.c_int64), ("PerJobUserTimeLimit", ctypes.c_int64),
                ("LimitFlags", wintypes.DWORD), ("MinimumWorkingSetSize", ctypes.c_size_t),
                ("MaximumWorkingSetSize", ctypes.c_size_t), ("ActiveProcessLimit", wintypes.DWORD),
                ("Affinity", ctypes.c_size_t), ("PriorityClass", wintypes.DWORD), ("SchedulingClass", wintypes.DWORD)]


class _Limites(ctypes.Structure):
    _fields_ = [("BasicLimitInformation", _Basicos), ("IoInfo", ctypes.c_uint64 * 6),
                ("ProcessMemoryLimit", ctypes.c_size_t), ("JobMemoryLimit", ctypes.c_size_t),
                ("PeakProcessMemoryUsed", ctypes.c_size_t), ("PeakJobMemoryUsed", ctypes.c_size_t)]


if EN_WINDOWS:
    _k32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _k32.CreateJobObjectW.restype = wintypes.HANDLE
    _k32.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
    _k32.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
    _k32.TerminateJobObject.argtypes = [wintypes.HANDLE, wintypes.UINT]
    _k32.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
    _k32.QueryInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD,
                                               ctypes.c_void_p]
    _k32.OpenProcess.restype = wintypes.HANDLE
    _k32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    _k32.QueryFullProcessImageNameW.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR,
                                                ctypes.POINTER(wintypes.DWORD)]
    _k32.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
    _k32.CloseHandle.argtypes = [wintypes.HANDLE]
    _ntdll = ctypes.WinDLL("ntdll")
    _ntdll.NtResumeProcess.argtypes = [wintypes.HANDLE]


class Trabajo:
    """Un Job Object: los procesos de un comando y todo lo que abran.

    Ojo: con el handle a NULL, Windows entiende «el trabajo del propio llamador» (QueryInformationJobObject devuelve
    los procesos del motor y de quien lo lanzó). Por eso, cerrado el trabajo, ninguna operación llega a Windows."""

    def __init__(self):
        self.h = _k32.CreateJobObjectW(None, None)
        if not self.h:
            raise OSError(ctypes.get_last_error(), "CreateJobObject")
        self.matar_al_cerrar(True)

    def matar_al_cerrar(self, si: bool) -> None:
        if not self.h:
            return
        info = _Limites()
        info.BasicLimitInformation.LimitFlags = MATAR_AL_CERRAR if si else 0
        _k32.SetInformationJobObject(self.h, LIMITES, ctypes.byref(info), ctypes.sizeof(info))

    def meter(self, proc: subprocess.Popen) -> None:
        if not self.h or not _k32.AssignProcessToJobObject(self.h, int(proc._handle)):
            raise OSError(ctypes.get_last_error(), "AssignProcessToJobObject")

    def pids(self) -> list[int]:
        """Los procesos vivos del trabajo (la lista la lleva Windows: no hay que adivinar quién es hijo de quién)."""
        if not self.h:
            return []
        n = 4096
        buf = (ctypes.c_size_t * (n + 1))()  # 2 DWORD (≤ un ULONG_PTR) + la lista
        if not _k32.QueryInformationJobObject(self.h, LISTA_PROCESOS, buf, ctypes.sizeof(buf), None):
            return []
        cuantos = ctypes.cast(buf, ctypes.POINTER(wintypes.DWORD))[1]
        ini = 8 // ctypes.sizeof(ctypes.c_size_t)  # la lista empieza tras los dos DWORD
        return [int(buf[ini + i]) for i in range(min(cuantos, n))]

    def terminar(self) -> None:
        if self.h:
            _k32.TerminateJobObject(self.h, 1)

    def cerrar(self) -> None:
        if self.h:
            _k32.CloseHandle(self.h)
            self.h = None


def lanzar(args, **kw) -> tuple[subprocess.Popen, "Trabajo | None"]:
    """Popen dentro de un trabajo nuevo. Sin trabajo (fuera de Windows o si falla), el Popen de siempre."""
    if not EN_WINDOWS:
        return subprocess.Popen(args, **kw), None
    kw["creationflags"] = kw.get("creationflags", 0) | SUSPENDIDO
    proc = subprocess.Popen(args, **kw)
    trabajo = None
    try:
        trabajo = Trabajo()
        trabajo.meter(proc)
    except OSError:
        if trabajo:
            trabajo.cerrar()
        trabajo = None
    finally:
        _ntdll.NtResumeProcess(int(proc._handle))
    return proc, trabajo


def nombre(pid: int) -> str:
    """Nombre del ejecutable (aura-monitor.exe), o '' si ya no existe."""
    if not EN_WINDOWS:
        return ""
    h = _k32.OpenProcess(CONSULTAR, False, pid)
    if not h:
        return ""
    try:
        tam = wintypes.DWORD(1024)
        buf = ctypes.create_unicode_buffer(tam.value)
        return Path(buf.value).name if _k32.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(tam)) else ""
    finally:
        _k32.CloseHandle(h)


def cerrar_pid(pid: int) -> bool:
    """Cierra un proceso concreto (solo se llama con números sacados de un trabajo del agente). Nunca el motor."""
    h = _k32.OpenProcess(TERMINAR, False, pid) if EN_WINDOWS and pid != os.getpid() else None
    if not h:
        return False
    try:
        return bool(_k32.TerminateProcess(h, 1))
    finally:
        _k32.CloseHandle(h)


def matar_arbol(pid: int) -> None:
    """Solo para procesos sin trabajo (fuera de Windows o si no se pudo crear): el método antiguo."""
    subprocess.run(["taskkill", "/T", "/F", "/PID", str(pid)], capture_output=True, check=False)


def detener(proc: subprocess.Popen, trabajo: "Trabajo | None") -> None:
    if trabajo:
        trabajo.terminar()
    elif proc.poll() is None:
        matar_arbol(proc.pid)
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass
