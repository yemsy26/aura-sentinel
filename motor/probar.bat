@echo off
setlocal EnableExtensions
chcp 65001 >nul
cd /d "%~dp0"
title Probar Agente Local
if not exist ".venv\Scripts\python.exe" (echo Ejecuta primero instalar.bat & pause & exit /b 1)
echo [1/2] Pruebas internas (sin modelo)...
".venv\Scripts\python.exe" -m unittest discover -s tests || goto :error
echo [2/2] Prueba real: el modelo crea, prueba y entrega un proyecto...
".venv\Scripts\python.exe" -m agente.modelo asegurar || goto :error
".venv\Scripts\python.exe" tests\e2e.py || goto :error
echo.
echo [OK] Todo funciona.
goto :fin
:error
echo.
echo [ERROR] Alguna prueba fallo (ver arriba).
:fin
if not defined AGENTE_NOPAUSE pause
