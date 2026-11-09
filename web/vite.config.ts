import { defineConfig } from 'vite';

export default defineConfig({
  build: { target: 'es2022', outDir: 'dist', sourcemap: false },
  server: { port: 5173 },
});
