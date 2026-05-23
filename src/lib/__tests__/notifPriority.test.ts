import { describe, it, expect, beforeEach, vi } from 'vitest';

// Unit tests for notifPriority.svelte.ts — scoring algorithm, sort ordering,
// enable/disable toggle, and reset. Uses Svelte 5 runes ($state, $derived)
// which are compiled by the svelte vite plugin in vitest.config.ts.

// Mock localStorage (jsdom provides one, but ensure clean state).
beforeEach(() => {
  localStorage.clear();
});

// Dynamic import after localStorage is cleared so loadFromStorage starts fresh.
async function loadModule() {
  // Force a fresh module on each test to reset $state variables.
  // vitest caches modules, so we reset the module registry.
  vi.resetModules();
  const mod = await import('../notifPriority.svelte');
  return mod.notifPriority;
}

describe('notifPriority scoring', () => {
  it('returns 0 for unknown kinds', async () => {
    const np = await loadModule();
    expect(np.getScore('unknown.kind')).toBe(0);
  });

  it('increases score on click interactions', async () => {
    const np = await loadModule();
    np.recordInteraction('error.crash', 'click');
    // Click weight = 3, plus recency boost (very recent = ~10)
    const score = np.getScore('error.crash');
    expect(score).toBeGreaterThan(0);
  });

  it('click scores higher than expand', async () => {
    const np = await loadModule();
    np.recordInteraction('kind-a', 'click');
    np.recordInteraction('kind-b', 'expand');
    expect(np.getScore('kind-a')).toBeGreaterThan(np.getScore('kind-b'));
  });

  it('ignore interactions reduce score', async () => {
    const np = await loadModule();
    // Base score from clicks
    np.recordInteraction('test.kind', 'click');
    const scoreAfterClick = np.getScore('test.kind');

    // Add many ignores
    for (let i = 0; i < 20; i++) {
      np.recordInteraction('test.kind', 'ignore');
    }
    const scoreAfterIgnores = np.getScore('test.kind');
    // Score should be lower (ignore weight = -1)
    expect(scoreAfterIgnores).toBeLessThan(scoreAfterClick);
  });

  it('score is floored at 0 (never negative)', async () => {
    const np = await loadModule();
    for (let i = 0; i < 50; i++) {
      np.recordInteraction('test.kind', 'ignore');
    }
    expect(np.getScore('test.kind')).toBe(0);
  });
});

describe('notifPriority.sortByPriority', () => {
  it('sorts items by descending score', async () => {
    const np = await loadModule();
    // Give kind-a higher score than kind-b
    np.recordInteraction('kind-a', 'click');
    np.recordInteraction('kind-a', 'click');
    np.recordInteraction('kind-b', 'expand');

    const items = [
      { kind: 'kind-b', label: 'B' },
      { kind: 'kind-a', label: 'A' },
      { kind: 'kind-c', label: 'C' },
    ];
    const sorted = np.sortByPriority(items);
    expect(sorted[0].kind).toBe('kind-a');
    expect(sorted[1].kind).toBe('kind-b');
    // kind-c has no interactions, score 0 — should be last
    expect(sorted[2].kind).toBe('kind-c');
  });

  it('returns original order when disabled', async () => {
    const np = await loadModule();
    np.recordInteraction('kind-a', 'click');
    np.setEnabled(false);

    const items = [
      { kind: 'kind-b', label: 'B' },
      { kind: 'kind-a', label: 'A' },
    ];
    const sorted = np.sortByPriority(items);
    expect(sorted[0].kind).toBe('kind-b');
    expect(sorted[1].kind).toBe('kind-a');
  });
});

describe('notifPriority.setEnabled', () => {
  it('disables recording when set to false', async () => {
    const np = await loadModule();
    np.setEnabled(false);
    np.recordInteraction('test.kind', 'click');
    expect(np.getScore('test.kind')).toBe(0);
  });

  it('can be re-enabled', async () => {
    const np = await loadModule();
    np.setEnabled(false);
    np.setEnabled(true);
    expect(np.isEnabled()).toBe(true);
    np.recordInteraction('test.kind', 'click');
    expect(np.getScore('test.kind')).toBeGreaterThan(0);
  });
});

describe('notifPriority.reset', () => {
  it('clears all scores and localStorage', async () => {
    const np = await loadModule();
    np.recordInteraction('test.kind', 'click');
    expect(np.getScore('test.kind')).toBeGreaterThan(0);

    np.reset();
    expect(np.getScore('test.kind')).toBe(0);
    expect(localStorage.getItem('rift-notif-priority-interactions')).toBeNull();
  });
});

describe('notifPriority localStorage persistence', () => {
  it('persists interactions to localStorage', async () => {
    const np = await loadModule();
    np.recordInteraction('test.kind', 'click');
    const stored = localStorage.getItem('rift-notif-priority-interactions');
    expect(stored).not.toBeNull();
    const parsed = JSON.parse(stored!);
    expect(parsed['test.kind']).toBeDefined();
    expect(parsed['test.kind'].clicks).toHaveLength(1);
  });
});
