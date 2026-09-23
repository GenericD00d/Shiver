import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// tauri drives this dev server and needs a stable, non-random address
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: false,
    // shared/web is imported from outside this project
    fs: { allow: ['.', '../shared/web'] },
    watch: {
      ignored: ['**/src-tauri/**']
    }
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2022'
  }
});
