# Comprobaciones antes de `npm run tauri dev` (lo llama start.bat). Código de salida 1 = no se puede arrancar.
param([string]$Carpeta = (Split-Path -Parent $PSScriptRoot), [int]$Puerto = 1420)

function Contiene([string]$texto, [string]$parte) {
    return $texto -and $texto.IndexOf($parte, [StringComparison]::OrdinalIgnoreCase) -ge 0
}

# 1. Una ventana de Aura de una ejecución anterior bloquea el .exe que hay que recompilar.
$destino = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Carpeta 'src-tauri\target' }
$abierta = Get-CimInstance Win32_Process -Filter "Name='app.exe'" | Where-Object { Contiene $_.ExecutablePath $destino }
if ($abierta) {
    Write-Host "[ERROR] Aura ya está abierta (pid $($abierta[0].ProcessId)). Ciérrala y vuelve a ejecutar start.bat."
    exit 1
}

# 2. El puerto de la interfaz (Vite). Si lo ocupa un Vite de esta misma carpeta que quedó de antes, se cierra.
$escucha = Get-NetTCPConnection -LocalPort $Puerto -State Listen -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $escucha) { exit 0 }
$proceso = Get-CimInstance Win32_Process -Filter "ProcessId=$($escucha.OwningProcess)"
if ((Contiene $proceso.CommandLine $Carpeta) -and (Contiene $proceso.CommandLine 'vite')) {
    Stop-Process -Id $proceso.ProcessId -Force
    Write-Host "[OK] Cerré un servidor de la interfaz que había quedado abierto en el puerto $Puerto."
    exit 0
}
Write-Host "[ERROR] El puerto $Puerto lo usa otro programa: $($proceso.Name) (pid $($proceso.ProcessId)). Ciérralo y vuelve a intentarlo."
exit 1
