@echo off
setlocal EnableExtensions
chcp 65001 >nul
cd /d "%~dp0"
title Cargar modelo (ajuste automatico a VRAM)
if not exist ".venv\Scripts\python.exe" (echo Ejecuta primero instalar.bat & pause & exit /b 1)
rem Uso: cargar_modelo.bat [--modelo CLAVE] [--contexto N] [--remedir]
".venv\Scripts\python.exe" -m agente.modelo cargar %*
set "RC=%errorlevel%"
if not defined AGENTE_NOPAUSE pause
exit /b %RC%
