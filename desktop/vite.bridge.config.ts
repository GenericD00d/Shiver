import { defineConfig } from 'vite';

// the bridge is injected into every Sharkord page as an initialization script, so it has to be
// one self-contained IIFE with no imports and no module semantics. rust picks the file up with
// include_str! at compile time.
export default defineConfig({
  build: {
    outDir: 'src-tauri/generated',
    emptyOutDir: false,
    target: 'es2022',
    lib: {
      entry: 'bridge/index.ts',
      name: '__ShiverBridge',
      formats: ['iife'],
      fileName: () => 'bridge.js'
    }
  }
});
