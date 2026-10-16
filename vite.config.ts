import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';

// Tauri sirve la interfaz desde dist/ (build) o desde este servidor (tauri dev).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  // Solo interesa vigilar la interfaz: las carpetas de compilación (target, target-audit...) tienen miles de
  // archivos que cargo bloquea mientras escribe (EBUSY) y gastan CPU sin motivo.
  server: {
    port: 1420, strictPort: true,
    watch: { ignored: ['**/src-tauri/**', '**/workspace/**', '**/target/**', '**/target-*/**', '**/dist/**', '**/.agente/**', '**/.aura/**'] },
  },
  build: { outDir: 'dist', target: 'es2022', chunkSizeWarningLimit: 8000 },
  test: { environment: 'jsdom', include: ['src/**/*.test.{ts,tsx}'] },
});
