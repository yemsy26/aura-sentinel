# Aura-Sentinel 🚀🛡️
**Agente Autónomo de Ingeniería de Software DevSecOps de Nivel Industrial**  
*Desarrollado por Ramón Antonio Burgos Jerez*

> 🏆 **Nivel Alcanzado: Agente Autónomo Cognitivo Completo — Tier 0 (Architecture v4.0.0 - Septiembre 2026)**  
> Arquitectura de autonomía completa, determinismo formal, contratos de misión y ejecución gobernada por puertas de finalización verificables. Estándar de la industria equiparable a Devin 2.0 y SWE-agent, **pero ejecutado 100% en local con latencia cero.**

Aura-Sentinel es un sistema de ingeniería de software autónomo de alto rendimiento. Ejerce control absoluto sobre el ciclo de vida de desarrollo de software: deducción de contratos formales, particionado de fases, generación modular de código, validación sintáctica determinista, presupuestación de pasos y auto-reparación en bucle cerrado. 

**⚡ El Poder de lo Local:** Construido sobre un motor nativo ultrarrápido en **Rust (Tauri)** con **Monaco Editor**, Aura-Sentinel utiliza **Ollama** para dotar al agente de un cerebro local. Esto permite que el sistema opere a velocidades muy superiores a los agentes alojados en la nube (sin lidiar con cuotas de API, rate-limits o latencia de red), ofreciendo una **potencia industrial equivalente** mientras garantiza la soberanía y privacidad absoluta de tus datos y código fuente.

---

## ✨ Novedades Arquitectura v4 (Verification Hardening)

En esta última versión, el sistema ha sido sometido a un riguroso *Hardening Pass* estructural que lo hace criptográficamente inmune a las alucinaciones típicas de los LLMs:

- 🔒 **Zero LLM Authority (CompletionGate):** El LLM ya no puede decidir cuándo ha terminado una tarea. El cierre de misión está gobernado por una puerta matemática que exige **Evidencia Técnica Real**.
- 🧬 **State-Hash Evidence:** Cada prueba técnica (como pasar un test o compilar) está atada criptográficamente al *hash* exacto del workspace en ese milisegundo. Si el LLM rompe el código después de pasar un test, la evidencia se invalida automáticamente.
- 🧠 **Aprendizaje Adaptativo (AL-v2.5):** Memoria episódica avanzada que indexa qué estrategias (CompileFirst, TestDriven) funcionaron para qué tipo de tareas (huellas digitales), optimizando la ejecución de futuras misiones.
- 🛑 **Stall Recovery Activo:** Si el modelo entra en un bucle ciego de comandos en la terminal, el detector matemático de "Delta-Cero" interviene el FSM y fuerza un replanteamiento de estrategia.
- ⚙️ **Validación Estructural Independiente:** Los chequeos sintácticos (StaticAnalysis) están estrictamente separados de la validación de compilación real. El LLM está obligado a crear los entornos (Cargo.toml, package.json) y correr los comandos reales en la terminal, sin simulaciones.

---

## 🛠️ Módulos y Capacidades del Core en Rust

| Módulo / Capa Subyacente | Estado | Garantía Arquitectónica |
|---|---|---|
| **Contrato Formal (mission_contract)** | 🟢 Operativo | Criterios verificables exigidos antes del cierre. |
| **Puerta de Finalización (completion_gate)** | 🟢 Operativo | Evalúa el EvidenceGraph contra el state_hash actual. Bloquea cierres falsos. |
| **Grafo de Evidencias (evidence)** | 🟢 Operativo | Registro inmutable de operaciones (tests, builds) ligadas al hash del código. |
| **Enrutador Cognitivo (
outer, learning)**| 🟢 Operativo | (AL-v2.5) Decide la mejor estrategia (Compile vs Test) usando el historial local. |
| **Presupuesto Inteligente (step_budget)** | 🟢 Operativo | Distribuye pasos (40% dev / 30% verif / 20% repair) y previene loops infinitos. |
| **Detector Estancamiento (stall_detector)**| 🟢 Operativo | Detección de "Delta Cero". Redirige al agente hacia TOOL_THINK si se atasca. |
| **Gestor de Terminal (TOOL_TERMINAL)** | 🟢 Operativo | Bloquea encadenamiento malicioso (&&) y exige validación paso a paso real. |

---

## 🏗️ Instalación e Integración

Para desplegar este potente cerebro local, necesitas preparar tu entorno:

### Requisitos Previos

1. **Ollama:** El núcleo cognitivo del agente. Instala [Ollama](https://ollama.ai/) y asegúrate de que el servicio se esté ejecutando en tu máquina (http://localhost:11434).
2. **Modelos Locales:** Descarga el cerebro deseado. Recomendamos encarecidamente modelos orientados a código:
   `bash
   ollama run qwen2.5-coder:7b
   # O alternativamente: llama3.1
   `
3. **Rust & Cargo:** Instalado vía [rustup](https://rustup.rs/). Necesario para compilar el backend de Tauri.
4. **Node.js & npm/yarn:** Necesario para compilar el frontend.

### Pasos de Instalación

1. **Clonar el repositorio:**
   `bash
   git clone https://github.com/yemsy26/aura-sentinel.git
   cd aura-sentinel
   `

2. **Instalar dependencias del Frontend:**
   `bash
   npm install
   # o yarn install
   `

3. **Ejecutar en modo de desarrollo:**
   Aura-Sentinel se levanta mediante Tauri. En la raíz del proyecto, ejecuta:
   `bash
   npm run tauri dev
   `
   *El sistema compilará el backend en Rust (puede tardar unos minutos la primera vez) e iniciará la interfaz gráfica.*

4. **Configuración del Agente:**
   En la interfaz de Aura-Sentinel, asegúrate de configurar el endpoint apuntando a tu instancia de Ollama (http://localhost:11434) y selecciona tu modelo (ej. qwen2.5-coder:7b).

---

## ⚡ ¿Por qué Aura-Sentinel frente a Agentes Cloud?

* **Velocidad sin Compromisos:** Al correr Ollama en tu propio hardware, Aura procesa *tokens* y toma decisiones tácticas en milisegundos, permitiéndole iterar bucles de *Dev -> Test -> Fix* mucho más rápido que los agentes limitados por peticiones REST sobre internet.
* **Privacidad Zero-Trust:** Tu código propietario jamás abandona tu máquina. Ni una sola línea se envía a servidores de terceros para ser procesada.
* **Disciplina Estricta:** Los agentes de la nube suelen ser verbosos y propensos a la pereza ("Ya hice el archivo, asume que compila"). El runtime estricto de Aura-Sentinel bloquea matemáticamente este comportamiento. O compila en tu máquina local, o la misión no termina.

---

> **Aura Sentinel:** El estándar local para ingeniería de software delegada. Construido para resolver, no para alucinar.
