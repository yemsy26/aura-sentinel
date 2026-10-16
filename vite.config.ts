import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';

// Tauri sirve la interfaz desde dist/ (build) o desde este servidor (tauri dev).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ['**/src-tauri/**', '**/workspace/**'] } },
  build: { outDir: 'dist', target: 'es2022', chunkSizeWarningLimit: 8000 },
  test: { environment: 'jsdom', include: ['src/**/*.test.{ts,tsx}'] },
});
