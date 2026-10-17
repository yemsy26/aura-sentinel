# APIs web en Python (2026)
- Flask 3 (simple) o FastAPI (validación automática con Pydantic v2 y documentación en /docs). Consulta `version("flask")` / `version("fastapi")`.
- Flask: `app = Flask(__name__)`; rutas con `@app.get("/tareas")` y `@app.post("/tareas")`; lee JSON con `request.get_json(silent=True) or {}`; valida campos y devuelve `jsonify(error="...")`, 400. Arranque: `flask --app app run --port 5055` o `app.run(port=5055)` dentro de `if __name__ == "__main__":`.
- FastAPI: `app = FastAPI()`, modelos `class Tarea(BaseModel): titulo: str`; arranque `uvicorn app:app --port 5055` (`pip install fastapi uvicorn`).
- Estado en memoria: guárdalo en un objeto con función `reiniciar()` o crea la app con una fábrica `crear_app()`; así cada test empieza limpio (si no, los tests se contaminan entre sí).
- Tests: Flask `cliente = app.test_client()`; FastAPI `from fastapi.testclient import TestClient` (necesita httpx). Fixture de pytest que reinicia el estado antes de cada test. Prueba casos OK y de error (400/404).
- Probar el servidor real: `ejecutar("python app.py", fondo=true)`, luego `ejecutar("curl -s http://127.0.0.1:5055/tareas")`; en cmd el JSON va con comillas dobles escapadas: `curl -s -X POST -H "Content-Type: application/json" -d "{\"titulo\":\"x\"}" http://127.0.0.1:5055/tareas`. Detén el proceso con procesos(accion="detener").
- CORS solo si una web de otro origen consume la API (`flask-cors` o `CORSMiddleware`).
