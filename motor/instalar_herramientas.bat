@echo off
setlocal EnableExtensions
chcp 65001 >nul
cd /d "%~dp0"
title Instalar herramientas de ingenieria inversa
if not exist ".venv\Scripts\python.exe" (echo Ejecuta primero instalar.bat & pause & exit /b 1)
rem Baja apktool 3.0.3 (15 MB) y jadx 1.5.6 (73 MB) de sus releases de GitHub, comprueba su SHA-256 y los deja en
rem %LOCALAPPDATA%\AuraSentinel\herramientas. Para abrir y parchear APKs propios o con permiso.
".venv\Scripts\python.exe" -m agente.inversa instalar
set "RC=%errorlevel%"
if not defined AGENTE_NOPAUSE pause
exit /b %RC%
