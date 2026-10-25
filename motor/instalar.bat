@echo off
setlocal EnableExtensions
chcp 65001 >nul
cd /d "%~dp0"
title Instalar Agente Local
echo === Agente Local: instalacion ===
rem uv crea el entorno con Python 3.12 (lo descarga si el equipo no lo tiene) y las versiones exactas de requisitos.txt.
rem Se busca tambien fuera del PATH: lo que se abre desde el escritorio puede no recibir el PATH del usuario.
set "UV_VERSION=0.13.0"
set "UV_SHA256=088962f9e7b7bd9ea740c04c650b2a21c8928c345bd99ac24350dc924dba656c"
set "UV="
for %%U in (uv.exe) do set "UV=%%~$PATH:U"
for %%U in ("%USERPROFILE%\.local\bin\uv.exe" "%USERPROFILE%\.cargo\bin\uv.exe" "%LOCALAPPDATA%\Microsoft\WinGet\Links\uv.exe" "%USERPROFILE%\scoop\shims\uv.exe" "%~dp0.herramientas\uv.exe") do if not defined UV if exist "%%~U" set "UV=%%~U"
if not defined UV call :descargar_uv
if defined UV (
  echo [info] Entorno de Python 3.12 con uv: si el equipo no tiene Python 3.12, uv lo descarga.
  if not exist ".venv\Scripts\python.exe" "%UV%" venv .venv --python 3.12 -q || goto :error
  "%UV%" pip install --python .venv\Scripts\python.exe -q -r requisitos.txt || goto :error
) else (
  echo [info] Sin uv: creo el entorno con el Python del equipo.
  if not exist ".venv\Scripts\python.exe" py -3.12 -m venv .venv 2>nul
  if not exist ".venv\Scripts\python.exe" python -m venv .venv || goto :error
  ".venv\Scripts\python.exe" -m pip install -q -r requisitos.txt || goto :error
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

rem Sin uv en el equipo: baja la version fijada de su repositorio oficial y solo la usa si cuadra su SHA-256.
:descargar_uv
set "DESTINO=%~dp0.herramientas"
echo [info] No encuentro uv: descargo uv %UV_VERSION% (15 MB) de su repositorio de GitHub (astral-sh/uv)...
if not exist "%DESTINO%" mkdir "%DESTINO%"
"%SystemRoot%\System32\curl.exe" -fsSL --retry 3 -o "%DESTINO%\uv.zip" "https://github.com/astral-sh/uv/releases/download/%UV_VERSION%/uv-x86_64-pc-windows-msvc.zip" || (echo [AVISO] No pude descargar uv. & exit /b 1)
set "HUELLA="
for /f "skip=1 delims=" %%H in ('""%SystemRoot%\System32\certutil.exe" -hashfile "%DESTINO%\uv.zip" SHA256"') do if not defined HUELLA set "HUELLA=%%H"
set "HUELLA=%HUELLA: =%"
if /i not "%HUELLA%"=="%UV_SHA256%" (echo [AVISO] El uv descargado no cuadra con su SHA-256: no se usa. & del "%DESTINO%\uv.zip" & exit /b 1)
"%SystemRoot%\System32\tar.exe" -xf "%DESTINO%\uv.zip" -C "%DESTINO%" || exit /b 1
del "%DESTINO%\uv.zip"
for /r "%DESTINO%" %%F in (uv.exe) do if not defined UV if exist "%%F" set "UV=%%F"
exit /b 0

:error
echo.
echo [ERROR] La instalacion fallo en el paso anterior.
if not defined AGENTE_NOPAUSE pause
exit /b 1
:fin
if not defined AGENTE_NOPAUSE pause
