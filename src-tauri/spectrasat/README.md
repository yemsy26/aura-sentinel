# SpectraSAT 🧮⚡

Motor de inferencia y satisfacibilidad lógica (SAT) ultrarrápido escrito en Rust, diseñado para actuar como cerebro matemático de agentes de Inteligencia Artificial (como Aura-Sentinel). 

## 🚀 Arquitectura en Cascada de 4 Capas (v1.1.0)

A diferencia de los solvers DPLL tradicionales, SpectraSAT utiliza una arquitectura geométrica y algebraica en memoria que garantiza la ausencia de falsos positivos:

1. **Pre-filtro GF(2):** Extracción de subsistemas de ecuaciones lineales sobre el campo de Galois. Detecta contradicciones algebraicas masivas (Tseitin) en O(N^3) de forma determinista y paralela.
2. **Relajación SDP (Semidefinite Programming):** Transforma las cláusulas en hiperplanos geométricos usando el método ADMM y la jerarquía de Lasserre. Actúa como un faro espectral que guía la ramificación.
3. **B&B guiado por SDP + Verificador Estricto:** Branch-and-Bound que explora el subespacio continuo proyectado por el SDP. Incorpora un verificador booleano estricto de clausuras en las hojas para evitar que el SDP certifique falsos positivos matemáticos.
4. **DPLL Fallback Exhaustivo:** Si la relajación SDP falla al construir un certificado exacto, el sistema desciende a un solucionador DPLL puro (Davis-Putnam-Logemann-Loveland) con propagación unitaria. Esto garantiza que el motor sea **Completo y Correcto por Construcción**.

## 📦 Integración como Librería (FFI)

SpectraSAT expone un puente C-FFI seguro que retorna resultados estructurados en JSON, optimizado para ser consumido directamente por el NLU del agente, evitando que el LLM tenga que hacer cálculos deductivos.

`json
{
  "status": "SAT_CERTIFIED",
  "assignment": [true, true, true, false, false, false, false, true]
}
`

## 🛠️ Herramientas Binarias

El repositorio incluye herramientas interactivas:
- cargo run --bin spectrasat_stdio: CLI pipeline para conectar el motor con shells externos vía stdin/stdout.
- cargo run --bin benchmark_suite: Batería de pruebas de estrés para evaluar el EigenSolver disperso.
- cargo run --bin validate: Set de tests unitarios rápidos contra instancias conocidas (ej: Control de Accesos).

## 📄 Licencia
Este componente forma parte de la arquitectura Core de Aura-Sentinel.
Autor: Ramon Antonio Burgos Jerez.
