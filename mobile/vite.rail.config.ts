import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// The quick rail's page, which tauri-plugin-shiver-rail shows in a WebView of its own over the server
// page (never inside it) and serves from the plugin's assets with WebViewAssetLoader. Generated, and
// built before Gradle runs (`build` and `beforeDevCommand`).
export default defineConfig({
  plugins: [react()],
  root: 'rail',
  // served under /assets/rail/, so every path is relative
  base: './',
  resolve: { dedupe: ['react', 'react-dom'] },
  build: {
    outDir: '../plugins/tauri-plugin-shiver-rail/android/src/main/assets/rail',
    emptyOutDir: true,
    target: 'es2022',
    // one script and one stylesheet, nothing split off to fetch later
    modulePreload: false,
    cssCodeSplit: false
  }
});
