# Instalación completa de Aura Sentinel en un equipo nuevo (Windows 10/11 x64), tras clonar el repositorio:
#   powershell -ExecutionPolicy Bypass -File setup.ps1 [-Si] [-SinModelo] [-SinVision]
# 1) comprueba lo necesario, 2) dependencias de la interfaz, 3) el motor (Agente Local: entorno de Python,
# dependencias, pruebas y registro en Bionic/LM Studio), 4) descarga el modelo (16,8 GB + 0,9 GB de visión) y
# 5) lo ajusta a tu VRAM en Bionic (capas de expertos a la RAM/CPU, contexto que cabe) con sus ajustes de chat.
# Se puede repetir: lo que ya está hecho se salta. No pide permisos de administrador.
param([switch]$Si, [switch]$SinModelo, [switch]$SinVision)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8   # tildes bien también si la salida va a un archivo
$raiz = $PSScriptRoot
$motor = Join-Path $raiz 'motor'
$py = Join-Path $motor '.venv\Scripts\python.exe'
$env:AGENTE_NOPAUSE = '1'   # los .bat del motor no esperan una tecla
$pendiente = @()

function Paso($t) { Write-Host "`n== $t" -ForegroundColor Cyan }
function Ok($t) { Write-Host "  [OK] $t" -ForegroundColor Green }
function Aviso($t) { Write-Host "  [AVISO] $t" -ForegroundColor Yellow }
function Falta($t) { Write-Host "  [FALTA] $t" -ForegroundColor Red }
function Pregunta($t) {
    if ($Si) { return $true }
    return (Read-Host "  $t [S/n]") -notmatch '^[nN]'
}
# Los argumentos van entre comillas al llamarla: sin ellas PowerShell toma '-v' por un parámetro de la función.
function Version([string]$Exe, [string[]]$Argumentos) { try { & $Exe @Argumentos 2>$null | Select-Object -First 1 } catch { $null } }
function Bionic {
    foreach ($puerto in 1235, 1234) {
        try { Invoke-RestMethod "http://127.0.0.1:$puerto/api/v1/models" -TimeoutSec 4 | Out-Null; return $puerto } catch {}
    }
    return $null
}

Write-Host "Aura Sentinel - instalación" -ForegroundColor Cyan
Paso "1/5 Requisitos"
$bloquea = $false
$node = Version node '-v'
if ($node -and [version]($node.TrimStart('v')) -ge [version]'24.0.0') { Ok "Node.js $node" }
else { Falta "Node.js 24 LTS o superior: https://nodejs.org/ (o: winget install OpenJS.NodeJS.LTS)"; $bloquea = $true }
$git = Version git '--version'
if ($git) { Ok $git } else { Aviso "Sin Git: el panel Git de Aura y la herramienta git del agente no funcionarán. "
    + "https://git-scm.com/download/win (o: winget install Git.Git)" }
$cargo = Version cargo '-V'
if ($cargo) { Ok $cargo } else { Falta "Rust: https://rustup.rs/ (elige la opción por defecto, x86_64-pc-windows-msvc)"; $bloquea = $true }
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$msvc = (Test-Path $vswhere) -and (& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
if ($msvc) { Ok "Herramientas de compilación de C++ (MSVC)" }
else { Falta "Build Tools de Visual Studio con 'Desarrollo para el escritorio con C++': https://visualstudio.microsoft.com/visual-cpp-build-tools/"; $bloquea = $true }
$webview = @('HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
             'HKCU:\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}') | Where-Object { Test-Path $_ }
if ($webview) { Ok "WebView2" } else { Falta "WebView2 Runtime: https://developer.microsoft.com/microsoft-edge/webview2/"; $bloquea = $true }
$uv = Get-Command uv -ErrorAction SilentlyContinue
$python = Version python '--version'
if ($uv) { Ok "uv (crea el entorno de Python del motor con Python 3.12)" }
elseif ($python -match 'Python 3\.(1[1-9]|[2-9]\d)') { Ok $python }
else { Falta "Python 3.11+ (https://www.python.org/) o uv (https://docs.astral.sh/uv/)"; $bloquea = $true }
$edge = @("${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe", "$env:ProgramFiles\Google\Chrome\Application\chrome.exe") | Where-Object { Test-Path $_ }
if ($edge) { Ok "Navegador para probar las páginas web: $(Split-Path -Leaf $edge[0])" } else { Aviso "Sin Edge ni Chrome: el agente no podrá probar páginas web." }
$gpu = Version nvidia-smi '--query-gpu=name,memory.total', '--format=csv,noheader'
if ($gpu) { Ok "GPU: $gpu" } else { Aviso "Sin GPU NVIDIA detectada: el modelo irá en CPU (mucho más lento)." }
if (Test-Path "$env:USERPROFILE\.lmstudio") { Ok "LM Studio / Bionic instalado" }
else { Falta "Bionic o LM Studio: https://lmstudio.ai/ (ábrelo una vez para que cree su carpeta)"; $bloquea = $true }
if ($bloquea) { Write-Host "`nInstala lo que falta y vuelve a ejecutar setup.ps1." -ForegroundColor Red; exit 1 }

Paso "2/5 Dependencias de la interfaz (npm)"
Push-Location $raiz
npm install --no-audit --no-fund --loglevel=error
if ($LASTEXITCODE) { Pop-Location; Falta "npm install falló."; exit 1 }
Pop-Location
Ok "node_modules al día"

Paso "3/5 Motor: Agente Local (entorno de Python, dependencias, pruebas, registro MCP en Bionic/LM Studio)"
& cmd /c "`"$motor\instalar.bat`""
if ($LASTEXITCODE) { Falta "La instalación del motor falló (mira el mensaje de arriba)."; exit 1 }
Ok "Motor instalado en $motor"

Paso "4/5 Modelo del agente (Qwen3.6-35B-A3B, de agente.json)"
$argumentos = @(); if ($SinVision) { $argumentos += '--sin-vision' }
if ($SinModelo) { Aviso "Saltado (-SinModelo). Después: motor\descargar_modelo.bat"; $pendiente += 'motor\descargar_modelo.bat' }
elseif (Pregunta "¿Descargar o comprobar ahora el modelo? (si falta son 17,7 GB; si ya está, solo se verifica)") {
    & $py -m agente.descargar @argumentos
    if ($LASTEXITCODE) { Aviso "La descarga no terminó: vuelve a ejecutar motor\descargar_modelo.bat (continúa donde se quedó)."; $pendiente += 'motor\descargar_modelo.bat' }
} else { $pendiente += 'motor\descargar_modelo.bat' }

Paso "5/5 Ajuste a tu equipo en Bionic"
$puerto = Bionic
if (-not $puerto) {
    Aviso "Bionic no responde (puerto 1235; LM Studio usa 1234). Ábrelo y activa su servidor local; después ejecuta motor\cargar_modelo.bat."
    $pendiente += 'abrir Bionic y ejecutar motor\cargar_modelo.bat'
} elseif ($pendiente -contains 'motor\descargar_modelo.bat') {
    Aviso "Sin el modelo descargado no se puede ajustar: después de descargarlo, ejecuta motor\cargar_modelo.bat."
    $pendiente += 'motor\cargar_modelo.bat'
} else {
    Ok "Bionic responde en :$puerto"
    Write-Host "  Midiendo la VRAM y cargando el modelo (la primera vez tarda unos minutos)..."
    & $py -m agente.modelo cargar
    if ($LASTEXITCODE) {
        Aviso "No se pudo cargar. Si Bionic no ve el modelo recién descargado, reinícialo y ejecuta motor\cargar_modelo.bat."
        $pendiente += 'motor\cargar_modelo.bat'
    } else {
        & $py -m agente.instalar --configurar-modelo auto
    }
}

Write-Host "`n===============================================" -ForegroundColor Cyan
if ($pendiente) {
    Write-Host "Instalado, con pasos pendientes:" -ForegroundColor Yellow
    $pendiente | Select-Object -Unique | ForEach-Object { Write-Host "  - $_" -ForegroundColor Yellow }
} else { Write-Host "Todo listo." -ForegroundColor Green }
Write-Host @"
Recomendado en Bionic: desactivar la voz (Voxtral) si no la usas: ocupa 2-5 GB de VRAM que el modelo aprovecha.
Arranca Aura con start.bat. Guía completa: README.md (sección Instalación en un equipo nuevo).
"@
