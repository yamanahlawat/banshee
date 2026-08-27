import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  test: {
    environment: 'jsdom',
    // testing-library's auto-cleanup between tests needs real beforeEach/afterEach globals.
    globals: true,
  },
  resolve: {
    conditions: ['browser'],
  },
});
