# bin/hunt.exe — resolvedor SAT del motor

Lo usa la herramienta `resolver_sat` del agente. Es el lanzador `hunt` del motor híbrido
[Sat-hibrido-rust](https://github.com/yemsy26/Sat-hibrido-rust) (CDCL con capa de paridad GF(2), ruptura de simetrías
y enjambre de hilos), del mismo autor que Aura Sentinel.

- Origen: `Sat-hibrido-rust` en `5c1393acd2e1044bb2ba77d230a14d3536f68b09` y su dependencia
  [Chronos-VFS](https://github.com/yemsy26/Chronos-VFS) (MIT o Apache-2.0) en `d7d398bf3c9afad3bbfd021d32112c67d3bec8c9`.
- Compilado con Rust 1.94.1 y `-C strip=symbols`. SHA-256: `3415EE2FF018D7D6060262215CA23B92F81EE5E2DF9E5C5AC15A1C60DD75E52E`
- Uso: `hunt.exe formula.cnf [--timeout S] [--model archivo] [--workers K]`. Códigos de salida de la competición SAT:
  10 satisfacible, 20 insatisfacible, 124 sin respuesta en el tiempo dado.

## Compilarlo de nuevo

Con Rust 1.82 o superior:

```bat
git clone https://github.com/yemsy26/Sat-hibrido-rust
git clone https://github.com/yemsy26/Chronos-VFS Sat-hibrido-rust\dpll_check\Chronos-VFS
cd Sat-hibrido-rust\dpll_check
cargo build --release --bin hunt
```

El ejecutable queda en `target\release\hunt.exe`. Para usar otro sin sustituir este, pon su ruta en la variable de
entorno `AGENTE_HUNT`.
