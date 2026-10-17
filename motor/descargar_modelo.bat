@echo off
setlocal EnableExtensions
chcp 65001 >nul
cd /d "%~dp0"
title Descargar el modelo del agente
if not exist ".venv\Scripts\python.exe" (echo Ejecuta primero instalar.bat & pause & exit /b 1)
rem Uso: descargar_modelo.bat [--sin-vision] [--modelo CLAVE] [--carpeta DIR]
rem Baja de Hugging Face el modelo de agente.json (16,8 GB) y su vision (0,9 GB) a la carpeta de LM Studio.
".venv\Scripts\python.exe" -m agente.descargar %*
set "RC=%errorlevel%"
if not defined AGENTE_NOPAUSE pause
exit /b %RC%
