# MEV: bots de arbitraje, liquidaciones y backrun
- Alcance: arbitraje entre DEX, liquidaciones de préstamos y backrun (operar justo después de una transacción grande). No se cubren ataques a otros usuarios (sandwich, front-running de sus órdenes).
- Arquitectura (un módulo por pieza, cada una con tests):
  1. Datos: suscripción WebSocket a bloques y eventos (`eth_subscribe` "newHeads"/"logs"; en Solana `accountSubscribe`/`logsSubscribe`). La URL del nodo va en `.env`.
  2. Estado en memoria: reservas de pools Uniswap V2 (evento `Sync`), precio y liquidez de V3 (`sqrtPriceX96`, ticks), posiciones de préstamo (factor de salud).
  3. Búsqueda: rutas de 2-3 saltos entre pools del mismo par; liquidaciones cuando el factor de salud < 1.
  4. Simulación EXACTA antes de enviar: `eth_call` sobre un fork local (`anvil --fork-url`) o `revm` en Rust; en Solana `simulateTransaction`.
  5. Beneficio neto = salida − entrada − gas × precio del gas − propina al constructor de bloques. Solo se envía si supera un umbral configurable.
  6. Ejecución atómica: contrato ejecutor que revierte si no hay beneficio (`require(saldoFinal > saldoInicial + minimo)`), con flash loan si hace falta (Balancer sin comisión, Aave 0,05 %).
  7. Envío privado (los arbitrajes por el mempool público se los quedan otros): bundles de Flashbots/MEV-Share (`eth_sendBundle` a un relay) en Ethereum; bundles y propinas de Jito en Solana.
- Matemáticas de Uniswap V2 en enteros (wei): `salida = entrada*997*reservaSalida // (reservaEntrada*1000 + entrada*997)`.
- Entrada óptima EXACTA entre dos pools V2 (comprar en A, vender en B; A entra `ra_in` y sale `ra_out`, B entra `rb_in` y sale `rb_out`): `a = 997**2 * ra_out * rb_out`, `b = 1000**2 * ra_in * rb_in`, `c = 997 * (1000 * rb_in + 997 * ra_out)`; si `isqrt(a*b) > b` hay arbitraje y `entrada = (math.isqrt(a*b) - b) // c`. No uses búsquedas aproximadas. Test: compárala con la mejor entrada de una búsqueda ternaria sobre el beneficio (deben coincidir) y con un caso sin diferencia de precio (beneficio 0). Otras fórmulas: dedúcelas con `calcular`. V3: usa la librería de matemáticas del protocolo, no aproximaciones.
- Lenguaje: Rust (`alloy`, `revm`, `tokio`) para latencia; TypeScript (`viem`) para prototipos; Python (`web3`) para análisis de datos.
- Configuración por red en `config/redes.json` (chainId, variable del RPC, direcciones de routers, pools y del ejecutor) y parámetros (umbral mínimo, gas máximo) en `.env`.
- Pruebas: matemáticas de pools contra valores conocidos; simulación de una oportunidad real en un fork local con anvil (sin enviar nada fuera); tests del contrato ejecutor con Foundry (revierte sin beneficio, solo el dueño puede llamarlo). Modo `--simular` por defecto: el bot calcula y registra lo que haría sin enviar.
- Operar en una red real (enviar bundles o transacciones) siempre con el permiso del usuario, una cartera dedicada con fondos limitados y la clave fuera del código.
