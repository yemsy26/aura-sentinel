# SpectraSAT 🧮⚡

Motor de satisfacibilidad booleana (SAT/CNF) escrito en Rust, diseñado para resolver restricciones booleanas dentro de agentes como Aura-Sentinel. No es un solver de aritmética general ni de optimización de grafos.

## 🚀 Arquitectura en Cascada de 4 Capas (v1.1.0)

SpectraSAT usa heurísticas geométricas y algebraicas para buscar candidatos, y una búsqueda DPLL exacta como verificador de respaldo. Solo certifica SAT si la asignación satisface cada cláusula original y solo certifica UNSAT si la búsqueda exacta agota todas las opciones:

1. **Pistas GF(2) y SDP:** pueden ayudar a proponer asignaciones, pero no son aceptadas por sí solas como prueba de UNSAT.
2. **Búsqueda guiada:** cualquier asignación candidata se vuelve a comprobar contra todas las cláusulas originales; una fórmula general k-CNF nunca se recorta a 3-CNF.
3. **DPLL exacto acotado:** decide SAT/UNSAT con propagación unitaria hasta un máximo de 50 000 nodos.
4. **Límite explícito:** si se alcanza el tope de variables, tamaño o nodos, devuelve `INVALID_INPUT` o `UNKNOWN_SEARCH_LIMIT`; nunca presenta una búsqueda incompleta como veredicto.

## 📦 Integración como Librería (FFI)

SpectraSAT expone un puente C-FFI seguro que retorna resultados estructurados en JSON, optimizado para ser consumido directamente por el NLU del agente, evitando que el LLM tenga que hacer cálculos deductivos.

```json
{
  "status": "SAT_CERTIFIED",
  "assignment": [true, true, true, false, false, false, false, true]
}
```

## 🛠️ Herramientas Binarias

El repositorio incluye herramientas interactivas:
- cargo run --bin spectrasat_stdio: CLI pipeline para conectar el motor con shells externos vía stdin/stdout.
- cargo run --bin benchmark_suite: Batería de pruebas de estrés para evaluar el EigenSolver disperso.
- cargo run --bin validate: Set de tests unitarios rápidos contra instancias conocidas (ej: Control de Accesos).

## 📄 Licencia
Este componente forma parte de la arquitectura Core de Aura-Sentinel.
Autor: Ramon Antonio Burgos Jerez.
