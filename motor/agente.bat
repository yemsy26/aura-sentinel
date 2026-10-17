@echo off
setlocal EnableExtensions
chcp 65001 >nul
title Agente Local
set "AG=%~dp0"
if not exist "%AG%.venv\Scripts\python.exe" (echo Ejecuta primero instalar.bat & pause & exit /b 1)
set "PROYECTO=%~1"
if "%PROYECTO%"=="" set /p "PROYECTO=Carpeta del proyecto (Enter = carpeta actual): "
if "%PROYECTO%"=="" set "PROYECTO=%CD%"
for %%I in ("%PROYECTO%") do set "PROYECTO=%%~fI"
cd /d "%AG%"
".venv\Scripts\python.exe" -m agente.modelo asegurar || (pause & exit /b 1)
".venv\Scripts\python.exe" -m agente "%PROYECTO%" %2 %3 %4 %5 %6 %7 %8 %9
if not defined AGENTE_NOPAUSE pause
