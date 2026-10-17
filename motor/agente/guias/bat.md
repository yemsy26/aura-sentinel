# Archivos .bat (Windows)
- Usa `crear_bat`: añade `chcp 65001`, `cd /d "%~dp0"` (rutas relativas a la carpeta del .bat), comprobación de errores en cada paso y pausa final.
- El .bat va en la MISMA carpeta que el programa, o usa rutas relativas correctas (`python juego\main.py`). `verificar` comprueba que los archivos que llama existen.
- Python: `usar_venv=true` crea `.venv` e instala `requirements.txt` la primera vez.
- Web/servidor: `start "" http://localhost:8000` antes de `python -m http.server 8000`.
- Sin emojis en `echo`: la consola de Windows puede mostrarlos rotos.
- Rutas con espacios entre comillas: `"C:\Mis juegos\main.py"`.
- Prueba cada .bat con `ejecutar("run.bat")`; si arranca un servidor o un juego que no termina, `ejecutar("run.bat", fondo=true)` y luego detenlo.
