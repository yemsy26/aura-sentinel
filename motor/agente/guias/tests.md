# Tests que de verdad prueban (cualquier lenguaje)
- Un test importa TU código y comprueba un comportamiento concreto con `assert`. Comprobar solo que una librería carga ("pygame funciona") NO es un test del proyecto.
- Cubre: caso normal, bordes (0, vacío, negativos, máximos), errores esperados (`pytest.raises(ValueError)`), y cada requisito del usuario (un test por requisito).
- Juegos: prueba la lógica sin ventana (SDL_VIDEODRIVER=dummy en pygame): colisión suma puntos, golpe quita vida, vidas 0 -> fin, reiniciar() -> estado inicial, pausa detiene el movimiento.
- Estado aislado: cada test empieza limpio (fixture que reinicia/crea de nuevo el objeto). Si un test pasa solo y falla junto a otros, hay estado compartido.
- Cuando un test falla, arregla el CÓDIGO. Solo cambia el test si el test estaba mal, y nunca lo debilites (quitar asserts, aceptar cualquier valor) para que pase.
- Nombres que explican: `test_dividir_entre_cero_lanza_error`.
- Python: pytest (`python -m pytest -q`). Java con Maven: JUnit 5 (`mvn -q test`). JS: pruebas de lógica con `node --test` (archivos `*.test.mjs`, `import test from "node:test"` y `import assert from "node:assert/strict"`).
