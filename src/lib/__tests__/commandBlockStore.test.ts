/**
 * Unit tests for CommandBlockStore (commandBlockStore.svelte.ts) — N3.1.
 *
 * Covers:
 *   - record() assigns a unique id, defaults bookmarked=false, and prepends
 *     newest-first.
 *   - byId / forSession / count / blocks getters.
 *   - toggleBookmark toggles, persists, returns the new state, and is a no-op
 *     (returns false) for an unknown id.
 *   - bookmarks getter reflects pinned blocks.
 *   - remove() and clear().
 *   - Bounded ring: recording past the cap drops the OLDEST non-bookmarked
 *     blocks, but a bookmarked block is retained even past the cap.
 *
 * Each test uses a fresh instance (errorClusterStore pattern) — no cross-test
 * bleed through the module singleton.
 */

import { describe, it, expect, beforeEach } from 'vitest';
import { CommandBlockStore, type CommandBlockInput } from '../commandBlockStore.svelte';

const MAX_BLOCKS = 500; // mirror of the store's internal cap (see #trim)

function input(over: Partial<CommandBlockInput> = {}): CommandBlockInput {
  return {
    sessionId: 1,
    command: 'echo hi',
    cwd: '/w',
    exitCode: 0,
    durationMs: 12,
    startRow: 0,
    endRow: 1,
    ts: 1_000,
    ...over,
  };
}

let store: CommandBlockStore;

beforeEach(() => {
  store = new CommandBlockStore();
});

describe('CommandBlockStore.record', () => {
  it('starts empty', () => {
    expect(store.blocks).toHaveLength(0);
    expect(store.count).toBe(0);
    expect(store.bookmarks).toHaveLength(0);
  });

  it('assigns an id, defaults bookmarked=false, and returns the block', () => {
    const block = store.record(input({ command: 'ls' }));
    expect(block.id).toMatch(/^blk-\d+$/);
    expect(block.bookmarked).toBe(false);
    expect(block.command).toBe('ls');
    expect(store.count).toBe(1);
    expect(store.byId(block.id)).toEqual(block);
  });

  it('assigns unique, increasing ids', () => {
    const a = store.record(input());
    const b = store.record(input());
    expect(a.id).not.toBe(b.id);
  });

  it('prepends newest-first', () => {
    const first = store.record(input({ command: 'first' }));
    const second = store.record(input({ command: 'second' }));
    expect(store.blocks[0].id).toBe(second.id);
    expect(store.blocks[1].id).toBe(first.id);
  });

  it('preserves the full payload', () => {
    const block = store.record(
      input({ sessionId: 7, command: 'cargo build', exitCode: 101, durationMs: 1200, cwd: null }),
    );
    expect(block).toMatchObject({
      sessionId: 7,
      command: 'cargo build',
      exitCode: 101,
      durationMs: 1200,
      cwd: null,
    });
  });
});

describe('CommandBlockStore queries', () => {
  it('byId returns undefined for an unknown id', () => {
    expect(store.byId('blk-999')).toBeUndefined();
  });

  it('forSession filters by sessionId', () => {
    store.record(input({ sessionId: 1, command: 'a' }));
    store.record(input({ sessionId: 2, command: 'b' }));
    store.record(input({ sessionId: 1, command: 'c' }));
    const s1 = store.forSession(1);
    expect(s1.map((b) => b.command)).toEqual(['c', 'a']); // newest-first
    expect(store.forSession(2)).toHaveLength(1);
    expect(store.forSession(99)).toHaveLength(0);
  });
});

describe('CommandBlockStore.toggleBookmark', () => {
  it('toggles, persists, and returns the new state', () => {
    const block = store.record(input());
    expect(store.toggleBookmark(block.id)).toBe(true);
    expect(store.byId(block.id)?.bookmarked).toBe(true);
    expect(store.bookmarks.map((b) => b.id)).toEqual([block.id]);
    expect(store.toggleBookmark(block.id)).toBe(false);
    expect(store.byId(block.id)?.bookmarked).toBe(false);
    expect(store.bookmarks).toHaveLength(0);
  });

  it('returns false (no-op) for an unknown id', () => {
    expect(store.toggleBookmark('blk-nope')).toBe(false);
  });
});

describe('CommandBlockStore.remove / clear', () => {
  it('remove drops a single block', () => {
    const a = store.record(input({ command: 'a' }));
    const b = store.record(input({ command: 'b' }));
    store.remove(a.id);
    expect(store.byId(a.id)).toBeUndefined();
    expect(store.byId(b.id)).toBeDefined();
    expect(store.count).toBe(1);
  });

  it('clear empties the store', () => {
    store.record(input());
    store.record(input());
    store.clear();
    expect(store.count).toBe(0);
  });
});

describe('CommandBlockStore bounded ring', () => {
  it('caps at MAX_BLOCKS, dropping the oldest non-bookmarked first', () => {
    const oldest = store.record(input({ command: 'oldest' }));
    for (let i = 0; i < MAX_BLOCKS + 50; i++) store.record(input({ command: `c${i}` }));
    expect(store.count).toBe(MAX_BLOCKS);
    expect(store.byId(oldest.id)).toBeUndefined(); // evicted
  });

  it('never evicts a bookmarked block, even past the cap', () => {
    const pinned = store.record(input({ command: 'pin-me' }));
    store.toggleBookmark(pinned.id);
    // Flood well past the cap with non-bookmarked blocks.
    for (let i = 0; i < MAX_BLOCKS + 50; i++) store.record(input({ command: `c${i}` }));
    // The pinned block survives; the ring is cap + the retained bookmark.
    expect(store.byId(pinned.id)).toBeDefined();
    expect(store.byId(pinned.id)?.bookmarked).toBe(true);
    expect(store.count).toBe(MAX_BLOCKS + 1);
    expect(store.bookmarks.map((b) => b.id)).toEqual([pinned.id]);
  });
});
