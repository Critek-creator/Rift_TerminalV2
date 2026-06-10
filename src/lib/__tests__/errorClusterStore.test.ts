/**
 * Unit tests for ErrorClusterStore (errorClusterStore.svelte.ts).
 *
 * Tests cover:
 *   - push() accumulates events and the cluster list updates.
 *   - pushAll() ingests a batch in one call.
 *   - clear() resets both events and clusters to empty.
 *   - Structurally-similar events collapse into one cluster (reuses
 *     clusterEvents() contract; this is a smoke-test, not a re-test
 *     of the pure function already covered by errorClustering.test.ts).
 *   - Distinct events produce separate clusters.
 *   - Buffer cap: pushing > MAX_EVENTS events trims the oldest half.
 *   - totalCount and clusterCount getters.
 *   - Module-level singleton identity.
 */

import { describe, it, expect, beforeEach } from 'vitest';
import { ErrorClusterStore, errorClusterStore } from '../errorClusterStore.svelte';
import type { Envelope } from '../bus';

function makeEnv(over: Partial<Envelope> = {}): Envelope {
  return {
    version: 1,
    category: 'system',
    kind: 'system.error',
    ts: Date.now(),
    payload: 'test error',
    ...over,
  };
}

// Each test uses a fresh store instance to guarantee no cross-test bleed.
let store: ErrorClusterStore;

beforeEach(() => {
  store = new ErrorClusterStore();
});

// ── push / accumulation ─────────────────────────────────────────────────────

describe('ErrorClusterStore.push', () => {
  it('starts empty', () => {
    expect(store.events).toHaveLength(0);
    expect(store.clusters).toHaveLength(0);
    expect(store.totalCount).toBe(0);
    expect(store.clusterCount).toBe(0);
  });

  it('accumulates pushed events and reflects them in totalCount', () => {
    store.push(makeEnv({ ts: 100 }));
    store.push(makeEnv({ ts: 200 }));
    expect(store.totalCount).toBe(2);
    expect(store.events).toHaveLength(2);
  });

  it('preserves push order (oldest → newest)', () => {
    store.push(makeEnv({ ts: 100, payload: 'first' }));
    store.push(makeEnv({ ts: 200, payload: 'second' }));
    expect(store.events[0].ts).toBe(100);
    expect(store.events[1].ts).toBe(200);
  });
});

// ── pushAll ─────────────────────────────────────────────────────────────────

describe('ErrorClusterStore.pushAll', () => {
  it('is a no-op for an empty array', () => {
    store.pushAll([]);
    expect(store.totalCount).toBe(0);
  });

  it('appends all envelopes at once', () => {
    const batch = [
      makeEnv({ ts: 1, payload: 'a' }),
      makeEnv({ ts: 2, payload: 'b' }),
      makeEnv({ ts: 3, payload: 'c' }),
    ];
    store.pushAll(batch);
    expect(store.totalCount).toBe(3);
  });
});

// ── clear ───────────────────────────────────────────────────────────────────

describe('ErrorClusterStore.clear', () => {
  it('resets events and clusters to empty', () => {
    store.push(makeEnv({ ts: 1 }));
    store.push(makeEnv({ ts: 2 }));
    store.clear();
    expect(store.events).toHaveLength(0);
    expect(store.clusters).toHaveLength(0);
    expect(store.totalCount).toBe(0);
    expect(store.clusterCount).toBe(0);
  });
});

// ── clustering behaviour ─────────────────────────────────────────────────────

describe('ErrorClusterStore cluster derivation', () => {
  it('folds structurally-identical events into one cluster', () => {
    // Same message, different attempt counter — collapses via normalizeSignature.
    store.push(makeEnv({ ts: 100, payload: 'connection refused (attempt 1)' }));
    store.push(makeEnv({ ts: 200, payload: 'connection refused (attempt 2)' }));
    store.push(makeEnv({ ts: 300, payload: 'connection refused (attempt 3)' }));

    expect(store.clusterCount).toBe(1);
    expect(store.clusters[0].count).toBe(3);
    expect(store.clusters[0].firstTs).toBe(100);
    expect(store.clusters[0].lastTs).toBe(300);
  });

  it('keeps distinct messages as separate clusters', () => {
    store.push(makeEnv({ ts: 100, payload: 'disk full' }));
    store.push(makeEnv({ ts: 200, payload: 'permission denied' }));

    expect(store.clusterCount).toBe(2);
  });

  it('sorts clusters by most-recent activity (lastTs descending)', () => {
    // Push "disk full" first (ts 100, 400), "perm denied" in the middle (ts 200).
    store.push(makeEnv({ ts: 100, payload: 'disk full' }));
    store.push(makeEnv({ ts: 200, payload: 'permission denied' }));
    store.push(makeEnv({ ts: 400, payload: 'disk full' }));

    expect(store.clusters[0].sample).toBe('disk full');   // lastTs 400
    expect(store.clusters[1].sample).toBe('permission denied'); // lastTs 200
  });

  it('clusterCount equals the number of distinct structural signatures', () => {
    store.push(makeEnv({ ts: 1, kind: 'error', payload: 'alpha' }));
    store.push(makeEnv({ ts: 2, kind: 'warn',  payload: 'beta' }));
    store.push(makeEnv({ ts: 3, kind: 'error', payload: 'alpha' }));

    // "error alpha" and "warn beta" are distinct — kind is part of the signature.
    expect(store.clusterCount).toBe(2);
  });
});

// ── buffer cap ───────────────────────────────────────────────────────────────

describe('ErrorClusterStore buffer cap (MAX_EVENTS = 2000)', () => {
  it('does not trim when below the cap', () => {
    for (let i = 0; i < 50; i++) {
      store.push(makeEnv({ ts: i }));
    }
    expect(store.totalCount).toBe(50);
  });

  // 2001 single pushes through the reactive assign-replace buffer take ~2s
  // locally and ~10s on CI Windows runners — past vitest's 5s default. The
  // synchronous 2001-push burst is a test-only worst case (real traffic is
  // event-rate; replay bursts go through pushAll), so widen the timeout
  // rather than weaken the cap coverage.
  it('trims to at most MAX_EVENTS/2 entries when the cap is exceeded', { timeout: 30_000 }, () => {
    // Push 2001 events — should trigger a trim leaving ≤ 1000.
    for (let i = 0; i < 2001; i++) {
      store.push(makeEnv({ ts: i, payload: `event ${i}` }));
    }
    expect(store.totalCount).toBeLessThanOrEqual(1000);
    expect(store.totalCount).toBeGreaterThan(0);
  });

  it('retains the newest events after a trim', { timeout: 30_000 }, () => {
    for (let i = 0; i < 2001; i++) {
      store.push(makeEnv({ ts: i, payload: `event ${i}` }));
    }
    // The last pushed event should survive the trim.
    const last = store.events[store.events.length - 1];
    expect(last.ts).toBe(2000);
  });
});

// ── singleton identity ────────────────────────────────────────────────────────

describe('errorClusterStore singleton', () => {
  it('exports the same object every import', async () => {
    const { errorClusterStore: imported } = await import('../errorClusterStore.svelte');
    expect(imported).toBe(errorClusterStore);
  });

  it('is an instance of ErrorClusterStore', () => {
    expect(errorClusterStore).toBeInstanceOf(ErrorClusterStore);
  });
});

// ── Svelte 5 reactivity contract ──────────────────────────────────────────────

describe('ErrorClusterStore reactivity contract', () => {
  it('events array identity changes on each push (assign-replace)', () => {
    const before = store.events;
    store.push(makeEnv({ ts: 1 }));
    expect(store.events).not.toBe(before);
  });

  it('events array identity changes after clear', () => {
    store.push(makeEnv({ ts: 1 }));
    const before = store.events;
    store.clear();
    expect(store.events).not.toBe(before);
  });
});
