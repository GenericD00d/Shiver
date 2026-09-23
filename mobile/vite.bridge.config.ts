import { defineConfig } from 'vite';

// Two self-contained IIFEs that Rust includes with `include_str!`:
//   default mode          -> generated/bridge.js          (evaluated in a server page after load)
//   --mode document-start -> generated/document-start.js  (runs before any page script)
export default defineConfig(({ mode }) => {
  const entry = mode === 'document-start' ? 'document-start' : 'bridge';

  return {
    build: {
      outDir: 'src-tauri/generated',
      emptyOutDir: false,
      target: 'es2022',
      lib: {
        entry: entry === 'bridge' ? 'bridge/index.ts' : 'bridge/document-start.ts',
        name: entry === 'bridge' ? '__ShiverBridge' : '__ShiverDocumentStart',
        formats: ['iife'],
        fileName: () => `${entry}.js`
      }
    }
  };
});
