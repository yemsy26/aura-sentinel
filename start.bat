@echo off
setlocal enabledelayedexpansion
color 0B
echo =======================================================
echo          AURA-SENTINEL - INICIO COMPLETO
echo =======================================================
echo.
echo Cambiando al directorio del proyecto...
cd /d "%~dp0"

echo Forzando variables de entorno seguras...
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

echo.
echo Comprobando servidor Ollama local...
curl -s http://localhost:11434/api/tags >nul
if %errorlevel% neq 0 (
    echo [INFO] Servidor Ollama no detectado. Levantando motor de IA en segundo plano...
    start "" /B ollama serve >nul 2>&1
    
    echo Esperando a que el motor Ollama inicialice...
    timeout /t 4 /nobreak >nul
)

echo.
echo Compilando e iniciando la interfaz tactica de Tauri...
echo Por favor espera, esto puede tomar unos segundos...
echo.

call npm run tauri dev
pause
