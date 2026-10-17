# Node.js y TypeScript (Node 24 LTS, 2026)
- Proyecto: `npm init -y` y en package.json `"type": "module"` (import/export). Antes de añadir una librería: `version("express", "npm")`; instala con `npm install express` (y `npm install -D ...` para herramientas). Conserva `package-lock.json`.
- Pruebas sin dependencias: `import { test } from "node:test"; import assert from "node:assert/strict";` y `"scripts": {"test": "node --test"}`. Con frameworks, sin modo watch: `vitest run`, `jest --ci`.
- TypeScript: `npm install -D typescript @types/node`, `tsconfig.json` con `"strict": true`, `"module": "nodenext"`, `"noEmit": true` si solo compruebas tipos. Node 24 ejecuta `.ts` directamente quitando los tipos (sin `enum` ni `namespace`); la comprobación de tipos la hace `tsc --noEmit`.
- API: Express 5 o Fastify; valida la entrada, responde JSON con códigos HTTP correctos y añade `GET /health`. Puerto: `process.env.PORT ?? 3000`. Configuración: `node --env-file=.env app.js` (el .env en .gitignore).
- Errores: `try/catch` en código `async`, un manejador de errores central en la API y `process.exitCode = 1` en scripts de consola.
- Frontend: `npm create vite@latest app -- --template react-ts`, después `npm install` y `npm run build`. Las páginas se prueban con `ver_pantalla` sobre `http://localhost:PUERTO`.
- Servidores: `ejecutar("npm start", fondo=true)` y `curl -s http://localhost:3000/health`; detenlos con procesos al acabar.
- run.bat: `crear_bat("run", ["npm install", "npm start"])`; test.bat: `crear_bat("test", ["npm test"])`.
- `verificar` comprueba que estén las dependencias, `tsc --noEmit` (errores con archivo:línea), `npm run build` si existe y `npm test`.
