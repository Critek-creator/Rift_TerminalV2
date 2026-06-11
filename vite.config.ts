import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { resolve } from 'path';

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: false,
    watch: {
      ignored: ['**/src-tauri/**', '**/crates/**', '**/target/**'],
    },
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    sourcemap: false,
    rollupOptions: {
      input: {
        main: resolve(__dirname, 'index.html'),
        cockpitDetached: resolve(__dirname, 'cockpit-detached.html'),
        notifDetached: resolve(__dirname, 'notif-detached.html'),
      },
      output: {
        manualChunks(id) {
          if (id.includes('node_modules/@xterm/')) return 'xterm';
          if (id.includes('node_modules/@codemirror/') || id.includes('node_modules/codemirror/')) return 'codemirror';
          if (id.includes('node_modules/shiki/')) return 'shiki';
          if (id.includes('node_modules/d3-')) return 'd3';
        },
      },
    },
  },
});
