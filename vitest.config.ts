import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Vitest config — jsdom env so DOM globals are available for tests that
// touch addEventListener / dispatchEvent / etc. without spinning up a real
// browser. Pure-TS module tests run cheaply here too. See
// `src/lib/__tests__/` for examples.
//
// Why vitest (not playwright): the regression-class bugs we keep hitting
// (each-key duplicates, mount-race in $effect cleanup, drag-promote state
// transitions, terminal fit defer) are unit-testable without a real
// browser. Playwright pays for real-browser fidelity Rift's tests don't
// need yet, and against the dev-server URL it can't see Tauri's
// `window.__TAURI__` anyway. Defer playwright (or `tauri-driver`) until a
// bug class genuinely requires it.
//
// svelte plugin added: required so `.svelte.ts` files (which use Svelte 5
// runes like $state) are compiled before Vitest runs them. Without it,
// $state / $derived / $effect remain as bare undefined identifiers in jsdom.
export default defineConfig({
  plugins: [svelte()],
  test: {
    environment: 'jsdom',
    include: ['src/**/__tests__/**/*.test.ts'],
    // Tauri APIs are mocked per-test via `vi.mock('@tauri-apps/api/core')`.
    // No global setup file yet; add one if mock boilerplate gets repetitive.
  },
});
