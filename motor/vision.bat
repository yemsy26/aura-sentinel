@echo off
setlocal EnableExtensions
chcp 65001 >nul
cd /d "%~dp0"
rem Uso: vision.bat off   (libera VRAM: el modelo deja de ver imagenes)
rem      vision.bat on    (restaura la vision)
if "%~1"=="" (echo Uso: vision.bat on^|off [--modelo CLAVE] & pause & exit /b 1)
".venv\Scripts\python.exe" -m agente.modelo vision %*
set "RC=%errorlevel%"
if not defined AGENTE_NOPAUSE pause
exit /b %RC%
