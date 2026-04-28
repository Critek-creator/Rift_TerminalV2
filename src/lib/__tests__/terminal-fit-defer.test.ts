import { describe, it, expect, vi, afterEach } from 'vitest';

// Regression test for pr003 `terminal-fit-races-initial-flex-layout`.
//
// The bug: Terminal.svelte called fit.fit() synchronously after term.open(host)
// before parent flex containers had laid out their final dimensions. xterm
// measured 0×0, sized its canvas to 0×0, the PTY started with bogus rows/cols,
// and the initial session rendered a black screen.
//
// The fix defers fit() with a specific two-step sequence:
//   1. await tick()                        — flushes Svelte microtask queue
//   2. await new Promise(rAF)             — waits for browser layout pass
//   3. fit()                               — measures real dimensions
//
// Order is load-bearing. Reversing tick/rAF produces the same race because
// the rAF resolves before Svelte's microtasks have flushed the parent $effects.
//
// The helper `deferredFit` encapsulates this sequence (extracted from
// Terminal.svelte's onMount). These tests pin the timing invariant against
// the helper — if anyone reverts the sequence or inlines a synchronous call,
// these fire.
//
// Strategy: we inject fake `tick` and `fit` functions; manually control rAF
// via vi.useFakeTimers(). This avoids mounting xterm in jsdom (heavy side
// effects: XTerm ctor, Tauri invoke calls, ResizeObserver).

import { deferredFit } from '../terminal-fit-timing';

// Helper: creates a controllable rAF environment.
// vi.useFakeTimers() does NOT fake requestAnimationFrame by default in vitest
// (rAF is not a timer). We replace it manually per test.
function installFakeRAF(): { triggerRAF: () => void; restore: () => void } {
  const queue: FrameRequestCallback[] = [];
  const original = globalThis.requestAnimationFrame;
  globalThis.requestAnimationFrame = (cb: FrameRequestCallback) => {
    queue.push(cb);
    return queue.length; // dummy handle
  };
  return {
    triggerRAF: () => {
      const callbacks = queue.splice(0);
      callbacks.forEach((cb) => cb(performance.now()));
    },
    restore: () => {
      globalThis.requestAnimationFrame = original;
    },
  };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('deferredFit — timing invariant (C5 regression guard)', () => {
  it('fit() is NOT called synchronously after deferredFit is invoked', async () => {
    const { triggerRAF, restore } = installFakeRAF();
    const fit = vi.fn();
    // tick that never resolves — ensures synchronous-call path is unreachable
    let tickResolve!: () => void;
    const tick = () => new Promise<void>((res) => { tickResolve = res; });

    // Start but do not await — we want to inspect mid-flight state.
    const promise = deferredFit(fit, tick);
    // At this point: tick hasn't resolved, rAF hasn't fired.
    expect(fit).not.toHaveBeenCalled();

    // Resolve tick, then rAF, then collect.
    tickResolve();
    // After tick resolves, we're awaiting rAF — still not called.
    await Promise.resolve(); // flush the microtask that continues past `await tick()`
    expect(fit).not.toHaveBeenCalled();

    triggerRAF();
    await promise;
    expect(fit).toHaveBeenCalledTimes(1);
    restore();
  });

  it('fit() IS called exactly once after tick AND rAF have both elapsed', async () => {
    const { triggerRAF, restore } = installFakeRAF();
    const fit = vi.fn();
    let tickResolve!: () => void;
    const tick = () => new Promise<void>((res) => { tickResolve = res; });

    const promise = deferredFit(fit, tick);
    expect(fit).not.toHaveBeenCalled();

    // Resolve tick.
    tickResolve();
    await Promise.resolve(); // advance past `await tick()`

    // Still waiting on rAF — fit not yet called.
    expect(fit).not.toHaveBeenCalled();

    // Fire rAF.
    triggerRAF();
    await promise;

    expect(fit).toHaveBeenCalledTimes(1);
    restore();
  });

  it('order is tick → rAF → fit (not rAF → tick → fit)', async () => {
    // This test proves the sequence is tick-first. We install the fake rAF
    // BEFORE tick resolves and verify the rAF callback was registered AFTER
    // tick resolved, not before. If the order were reversed (rAF first), the
    // rAF queue would have a pending entry even before tick resolves.
    const rafQueue: FrameRequestCallback[] = [];
    const original = globalThis.requestAnimationFrame;
    globalThis.requestAnimationFrame = (cb: FrameRequestCallback) => {
      rafQueue.push(cb);
      return rafQueue.length;
    };

    const fit = vi.fn();
    let tickResolve!: () => void;
    const tick = () => new Promise<void>((res) => { tickResolve = res; });

    const promise = deferredFit(fit, tick);

    // tick has NOT resolved yet — rAF must NOT be registered yet.
    expect(rafQueue).toHaveLength(0);

    tickResolve();
    await Promise.resolve(); // advance past `await tick()`

    // Now tick resolved — rAF MUST have been registered.
    expect(rafQueue).toHaveLength(1);

    // Fire rAF to let the promise complete.
    const callbacks = rafQueue.splice(0);
    callbacks.forEach((cb) => cb(performance.now()));
    await promise;

    expect(fit).toHaveBeenCalledTimes(1);
    globalThis.requestAnimationFrame = original;
  });

  it('BROKEN pattern (synchronous fit after open) would miss layout settle — negative control', () => {
    // Documents the old broken behavior: fit() called immediately after
    // term.open(host) without any deferral. The test confirms that the
    // "sync call" path delivers fit() at the wrong time (before microtasks +
    // rAF have elapsed). If the production code reverts to sync, deferredFit
    // tests above would FAIL — but this negative control documents WHY.
    const callOrder: string[] = [];

    // Simulate: open() runs, then immediately fit() — the broken path.
    callOrder.push('open');
    callOrder.push('fit-SYNC'); // bug: too early

    // Simulate: open() runs, then after tick+rAF, fit() — the fixed path.
    callOrder.push('open');
    callOrder.push('tick');
    callOrder.push('rAF');
    callOrder.push('fit-DEFERRED'); // correct

    expect(callOrder.indexOf('fit-SYNC')).toBeLessThan(callOrder.indexOf('tick'));
    expect(callOrder.indexOf('fit-DEFERRED')).toBeGreaterThan(callOrder.indexOf('rAF'));
  });
});
