import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { resolve } from 'path';

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1422,
    strictPort: true,
    host: false,
    watch: {
      ignored: ['**/src-tauri/**', '**/crates/**', '**/target/**'],
    },
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    rollupOptions: {
      input: {
        main: resolve(__dirname, 'index.html'),
        cockpitDetached: resolve(__dirname, 'cockpit-detached.html'),
        notifDetached: resolve(__dirname, 'notif-detached.html'),
      },
    },
  },
});
