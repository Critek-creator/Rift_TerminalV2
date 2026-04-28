/**
 * terminal-fit-timing.ts
 *
 * Pure timing helper extracted from Terminal.svelte's onMount sequence.
 * Extracted to make the fit-defer ordering invariant unit-testable without
 * mounting xterm in jsdom (which would require heavy side-effect mocking).
 *
 * Bug context (pr003 `terminal-fit-races-initial-flex-layout`):
 *   Calling fit() synchronously after term.open(host) measured 0×0 because
 *   parent flex containers hadn't completed layout. The fix defers via one
 *   Svelte microtask tick (flushes parent $effects + flex recalculation)
 *   followed by one requestAnimationFrame (guarantees browser computed final
 *   dimensions before measuring).
 *
 * Order is load-bearing: tick → rAF → fit(). Reversing produces the same
 * race on initial render (rAF resolves before Svelte microtasks complete).
 */

/**
 * Defer `fit()` until layout has settled: one Svelte microtask tick, then
 * one rAF frame. Calls `fit()` exactly once, after both delays have elapsed.
 *
 * @param fit  - The FitAddon.fit() method (or any zero-arg layout callback).
 * @param tick - Svelte's `tick` function (injected for testability; production
 *               callers pass the real `tick` imported from 'svelte').
 */
export async function deferredFit(fit: () => void, tick: () => Promise<void>): Promise<void> {
  await tick();
  await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  fit();
}
