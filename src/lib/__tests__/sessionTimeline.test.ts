import { describe, it, expect } from 'vitest';
import { sourceMeta, isSeekable, activeSourceKeys, SOURCE_META } from '../sessionTimeline';

describe('sourceMeta', () => {
  it('returns metadata for every known source', () => {
    for (const key of ['command', 'error', 'agent', 'hook', 'fs', 'llm', 'mcp']) {
      const m = sourceMeta(key);
      expect(m.label.length).toBeGreaterThan(0);
      expect(m.color).toMatch(/var\(--/);
    }
    expect(Object.keys(SOURCE_META)).toHaveLength(7);
  });

  it('falls back safely for an unknown source (never blank)', () => {
    const m = sourceMeta('quantum');
    expect(m.label).toBe('QUANT'); // uppercased, capped at 5
    expect(m.color).toMatch(/var\(--/);
  });

  it('does not throw on empty source', () => {
    expect(() => sourceMeta('')).not.toThrow();
    expect(sourceMeta('').label).toBe('?');
  });
});

describe('isSeekable', () => {
  it('accepts real replay indices', () => {
    expect(isSeekable(0)).toBe(true);
    expect(isSeekable(42)).toBe(true);
    expect(isSeekable(Number.MAX_SAFE_INTEGER)).toBe(true);
  });

  it('rejects the u64::MAX command-history-only sentinel', () => {
    // u64::MAX serializes to a number beyond JS safe-integer range.
    expect(isSeekable(18446744073709551615)).toBe(false);
    expect(isSeekable(Number.MAX_SAFE_INTEGER + 1)).toBe(false);
  });

  it('rejects negative / non-finite indices', () => {
    expect(isSeekable(-1)).toBe(false);
    expect(isSeekable(NaN)).toBe(false);
    expect(isSeekable(Infinity)).toBe(false);
  });
});

describe('activeSourceKeys', () => {
  it('returns only enabled keys with the show_ prefix stripped', () => {
    const cfg = {
      show_commands: true,
      show_errors: true,
      show_agents: false,
      show_hooks: false,
      show_fs: true,
      show_llm_cost: false,
      show_mcp: false,
    };
    expect(activeSourceKeys(cfg).sort()).toEqual(['commands', 'errors', 'fs']);
  });

  it('returns an empty list when nothing is enabled', () => {
    expect(activeSourceKeys({ show_commands: false })).toEqual([]);
  });
});
