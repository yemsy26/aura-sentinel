@echo off
setlocal EnableExtensions
chcp 65001 >nul
cd /d "%~dp0"
title Instalar Agente Local
echo === Agente Local: instalacion ===
where uv >nul 2>nul
if %errorlevel%==0 (
  if not exist ".venv\Scripts\python.exe" uv venv .venv --python 3.12 -q || goto :error
  uv pip install --python .venv\Scripts\python.exe -q "mcp>=1.2" ruff vulture playwright "sympy>=1.13" || goto :error
) else (
  if not exist ".venv\Scripts\python.exe" python -m venv .venv || goto :error
  ".venv\Scripts\python.exe" -m pip install -q "mcp>=1.2" ruff vulture playwright "sympy>=1.13" || goto :error
)
echo [info] Las pruebas web usan Edge o Chrome ya instalados (sin descargas extra).
echo [info] Motores complementarios predeterminados (Sat-hibrido-rust: se descarga de su repositorio de GitHub)...
".venv\Scripts\python.exe" -m agente.complementos asegurar
echo [1/2] Pruebas internas...
".venv\Scripts\python.exe" -m unittest discover -s tests || goto :error
echo [2/2] Registrando MCP agente-local en Bionic y LM Studio...
".venv\Scripts\python.exe" -m agente.instalar --desactivar-bionic-core %* || goto :error
echo.
echo [OK] Instalado. Siguiente: cargar_modelo.bat y luego agente.bat "C:\ruta\de\tu\proyecto"
goto :fin
:error
echo.
echo [ERROR] La instalacion fallo en el paso anterior.
:fin
if not defined AGENTE_NOPAUSE pause
