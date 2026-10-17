# Guía de contribución

Gracias por tu interés en Aura Sentinel. Esta guía explica cómo preparar el entorno, qué se espera de un cambio y
cómo proponerlo.

## Antes de empezar

- Busca en los [issues](https://github.com/yemsy26/aura-sentinel/issues) si el fallo o la idea ya están abiertos.
- Para cambios grandes (nuevas herramientas del agente, cambios de arquitectura), abre primero un issue y acuerda
  el enfoque.
- Los fallos de seguridad no se publican en issues: sigue la [política de seguridad](SECURITY.md).

## Entorno de desarrollo

Requisitos y pasos completos en el [README](../README.md#instalación-en-un-equipo-nuevo). En resumen:

```powershell
git clone https://github.com/yemsy26/aura-sentinel.git
cd aura-sentinel
powershell -ExecutionPolicy Bypass -File setup.ps1 -SinModelo   # sin descargar el modelo (~17 GB)
start.bat                                                        # arranca la app en modo desarrollo
```

## Pruebas

Todo cambio debe dejar las pruebas en verde:

```powershell
npm test                                   # TypeScript, interfaz (Vitest) y Rust (cargo test)
cd motor
.venv\Scripts\python -m unittest discover -s tests   # motor (no necesita el modelo)
```

Si corriges un fallo, añade una prueba que falle sin el arreglo. `motor\probar.bat` ejecuta además una misión real
con el modelo cargado en Bionic.

## Estilo

- **Idioma**: interfaz, mensajes y documentación en español.
- **TypeScript/React**: modo estricto; los efectos de React usan cuerpo de bloque (no devuelven valores).
- **Rust**: `cargo fmt` y sin avisos de `cargo clippy`.
- **Python (motor)**: solo biblioteca estándar salvo las dependencias de `instalar.bat`; sin avisos de `ruff`.
- Comentarios breves que expliquen el porqué, no el qué.

## Commits

Mensajes en español con el formato de [Conventional Commits](https://www.conventionalcommits.org/es/v1.0.0/):

```
fix(ui): la Terminal ya no deja la ventana en blanco
feat(motor): descarga del modelo con verificación SHA-256
docs: instalación en un equipo nuevo
```

Tipos habituales: `feat`, `fix`, `docs`, `refactor`, `test`, `build`, `chore`. Un `!` (`feat!:`) marca un cambio
incompatible.

## Pull requests

1. Crea una rama desde `main` (`fix/descripcion-corta`, `feat/descripcion-corta`).
2. Mantén el cambio acotado a un solo objetivo.
3. Actualiza `CHANGELOG.md` y la documentación afectada (`README.md`, `motor/LEEME.md`).
4. Completa la plantilla del pull request e indica cómo lo probaste.

Al contribuir aceptas que tu aportación se publique bajo la [licencia MIT](../LICENSE) del proyecto y que respetas
el [código de conducta](CODE_OF_CONDUCT.md).
