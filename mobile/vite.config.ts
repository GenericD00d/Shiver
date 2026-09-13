/// <reference types="node" />
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// 1421 so the mobile client's dev server does not collide with the desktop client's 1420
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1421,
    strictPort: true,
    // the Android device loads the dev server over the network rather than from localhost
    host: process.env.TAURI_DEV_HOST || false
  }
});
