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
 * up to N rAF frames waiting for the host element to report non-zero
 * dimensions. Calls `fit()` exactly once, after both delays have elapsed
 * AND the host has measurable size (or the retry budget is exhausted —
 * in which case `fit()` is still called as a best-effort fallback).
 *
 * Retry loop rationale (2026-04-28 — initial-session black-terminal
 * regression returned post-merge of A's vault-walker and C's status
 * translator; a single rAF frame is no longer enough for the parent flex
 * containers to settle on initial cockpit render — likely because the
 * Svelte 5 reactive graph now does more work in the same frame from the
 * vault.update + status.usage envelope arrivals racing terminal mount).
 * Subsequent terminals (added via the "+" button) hit a fully-settled
 * cockpit layout and don't need the retry. The retry is cheap when the
 * first frame is enough — it returns immediately.
 *
 * @param fit      - The FitAddon.fit() method (or any zero-arg layout callback).
 * @param tick     - Svelte's `tick` function (injected for testability; production
 *                   callers pass the real `tick` imported from 'svelte').
 * @param hostRect - Optional getter returning the host element's current
 *                   bounding rect. When provided, the loop polls until
 *                   width > 0 AND height > 0 (or maxFrames is reached).
 *                   When omitted, behavior matches the original 1-rAF semantic.
 * @param maxFrames - Maximum rAF frames to wait for non-zero host dimensions
 *                    (default: 10 ≈ ~167ms at 60fps; bounded so a permanently
 *                    hidden host doesn't block forever).
 */
export async function deferredFit(
  fit: () => void,
  tick: () => Promise<void>,
  hostRect?: () => { width: number; height: number },
  maxFrames = 10,
): Promise<void> {
  await tick();
  for (let i = 0; i < maxFrames; i++) {
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    if (!hostRect) {
      // Caller didn't provide a rect probe — preserve the original
      // single-rAF semantic (returns immediately on first frame).
      fit();
      return;
    }
    const r = hostRect();
    if (r.width > 0 && r.height > 0) {
      fit();
      return;
    }
  }
  // Retry budget exhausted with host still 0×0 — call fit() anyway as a
  // best-effort fallback. The component's ResizeObserver will re-fit
  // when the host eventually settles to non-zero dimensions.
  fit();
}
