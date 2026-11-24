import { defineConfig } from 'vitest/config';
import vue from '@vitejs/plugin-vue';
import desktop from './src-tauri/tauri.conf.json';

export default defineConfig({
  plugins: [vue()],
  server: { port: 1420, strictPort: true, watch: { ignored: ['**/src-tauri/**'] } },
  preview: {
    host: '127.0.0.1',
    port: 1421,
    strictPort: true,
    headers: { 'Content-Security-Policy': desktop.app.security.csp },
  },
  build: { target: 'es2022', chunkSizeWarningLimit: 900 },
  test: { environment: 'happy-dom', include: ['src/**/*.test.ts'], setupFiles: ['src/tests/setup.ts'] },
  clearScreen: false,
});
