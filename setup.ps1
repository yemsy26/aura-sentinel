Write-Host "Configuracion de Aura Sentinel" -ForegroundColor Cyan
Write-Host "===============================================" -ForegroundColor Cyan

# Node.js y Rust compilan la interfaz Tauri
foreach ($herramienta in @(@("node", "-v", "https://nodejs.org/"), @("cargo", "-V", "https://rustup.rs/"))) {
    try {
        $version = & $herramienta[0] $herramienta[1]
        Write-Host "OK  $($herramienta[0]): $version" -ForegroundColor Green
    } catch {
        Write-Host "FALTA $($herramienta[0]). Instalalo desde $($herramienta[2]) y reintenta." -ForegroundColor Red
        exit 1
    }
}

# Agente Local es el motor de las misiones
$candidatas = @($env:AGENTE_LOCAL_DIR, "$env:USERPROFILE\.gemini\antigravity\scratch\lm studio", "$env:USERPROFILE\AgenteLocal")
$agente = $candidatas | Where-Object { $_ -and (Test-Path "$_\.venv\Scripts\python.exe") -and (Test-Path "$_\agente\__main__.py") } | Select-Object -First 1
if ($agente) {
    Write-Host "OK  Agente Local: $agente" -ForegroundColor Green
} else {
    Write-Host "FALTA Agente Local. Ejecuta instalar.bat en su carpeta o define AGENTE_LOCAL_DIR." -ForegroundColor Red
    exit 1
}

# Bionic (o LM Studio) sirve el modelo
try {
    Invoke-RestMethod -Uri "http://127.0.0.1:1235/api/v1/models" -TimeoutSec 4 | Out-Null
    Write-Host "OK  Bionic responde en :1235" -ForegroundColor Green
} catch {
    Write-Host "AVISO Bionic no responde en :1235. Abrelo antes de enviar misiones." -ForegroundColor Yellow
}

Write-Host "===============================================" -ForegroundColor Cyan
Write-Host "Instalando dependencias de la interfaz..." -ForegroundColor Yellow
npm install
Write-Host "Listo. Arranca Aura Sentinel con start.bat (o npm run tauri dev)." -ForegroundColor Green
