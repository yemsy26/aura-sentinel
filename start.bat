@echo off
setlocal EnableExtensions
chcp 65001 >nul
color 0B
title Aura Sentinel
echo =======================================================
echo          AURA SENTINEL - INICIO
echo =======================================================
cd /d "%~dp0"
set "CARGO_TARGET_DIR=%CD%\target-audit"
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

echo.
echo Comprobando Bionic (LM Studio) en http://127.0.0.1:1235 ...
curl -s -o nul http://127.0.0.1:1235/api/v1/models
if errorlevel 1 (
    echo [AVISO] Bionic no responde. Abrelo antes de enviar misiones: el modelo se carga alli.
) else (
    echo [OK] Bionic disponible.
)

echo.
echo Comprobando las dependencias de la interfaz...
rem Siempre: si package.json cambio (p. ej. al actualizar desde GitHub) faltarian paquetes. Sin cambios tarda 1-2 s.
call npm install --no-audit --no-fund --loglevel=error
if errorlevel 1 goto :error

powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\preparar-inicio.ps1"
if errorlevel 1 goto :fin

echo.
echo Compilando e iniciando la interfaz de Tauri (la primera vez tarda unos minutos)...
call npm run tauri dev
goto :fin

:error
echo.
echo [ERROR] No se pudieron instalar las dependencias (npm install).

:fin
pause
