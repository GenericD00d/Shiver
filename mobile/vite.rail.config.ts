import { fileURLToPath } from 'node:url';

import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// The quick rail's page, which tauri-plugin-shiver-rail shows in a WebView of its own over the server
// page (never inside it) and serves from the plugin's assets with WebViewAssetLoader. Generated, and
// built before Gradle runs (`build` and `beforeDevCommand`). The page has no IPC: Shiver's `api`
// reaches the core over the rail's channel instead, which stands in for Tauri's `invoke` and `listen`.
export default defineConfig({
  plugins: [react()],
  root: 'rail',
  // served under /assets/rail/, so every path is relative
  base: './',
  resolve: {
    dedupe: ['react', 'react-dom'],
    alias: [{ find: /^@tauri-apps\/api\/(core|event)$/, replacement: fileURLToPath(new URL('./rail/channel.ts', import.meta.url)) }]
  },
  build: {
    outDir: '../plugins/tauri-plugin-shiver-rail/android/src/main/assets/rail',
    emptyOutDir: true,
    target: 'es2022',
    // one script and one stylesheet, nothing split off to fetch later
    modulePreload: false,
    cssCodeSplit: false
  }
});
