import { describe, it, expect, beforeEach } from 'vitest';
import { CorrelationIndex } from '../correlationIndex';
import type { Envelope } from '../bus';

// Unit tests for CorrelationIndex — chain creation, lookup, eviction,
// duplicate handling, and reset.

function makeEnvelope(overrides: Partial<Envelope> = {}): Envelope {
  return {
    version: 1,
    category: 'system',
    kind: 'test',
    ts: Date.now(),
    payload: null,
    ...overrides,
  };
}

let idx: CorrelationIndex;

beforeEach(() => {
  idx = new CorrelationIndex();
});

describe('CorrelationIndex.index', () => {
  it('ignores envelopes without correlation_id', () => {
    const env = makeEnvelope({ correlation_id: undefined });
    idx.index(env);
    expect(idx.chainSize(undefined)).toBe(0);
  });

  it('creates a chain for a new correlation_id', () => {
    const env = makeEnvelope({ correlation_id: 'c1', ts: 100 });
    idx.index(env);
    expect(idx.chainSize('c1')).toBe(1);
  });

  it('appends to existing chain on same correlation_id', () => {
    idx.index(makeEnvelope({ correlation_id: 'c1', ts: 100 }));
    idx.index(makeEnvelope({ correlation_id: 'c1', ts: 200 }));
    expect(idx.chainSize('c1')).toBe(2);
  });
});

describe('CorrelationIndex.getChain', () => {
  it('returns empty array for unknown correlation_id', () => {
    expect(idx.getChain('nonexistent')).toEqual([]);
  });

  it('returns entries sorted by timestamp', () => {
    const e1 = makeEnvelope({ correlation_id: 'c1', ts: 300 });
    const e2 = makeEnvelope({ correlation_id: 'c1', ts: 100 });
    const e3 = makeEnvelope({ correlation_id: 'c1', ts: 200 });
    idx.index(e1);
    idx.index(e2);
    idx.index(e3);

    const chain = idx.getChain('c1');
    expect(chain.map((e) => e.ts)).toEqual([100, 200, 300]);
  });

  it('returns a copy — mutations do not affect the store', () => {
    idx.index(makeEnvelope({ correlation_id: 'c1', ts: 1 }));
    const chain = idx.getChain('c1');
    chain.push(makeEnvelope({ correlation_id: 'c1', ts: 999 }));
    expect(idx.chainSize('c1')).toBe(1);
  });
});

describe('CorrelationIndex.getRelated', () => {
  it('returns empty array when envelope has no correlation_id', () => {
    expect(idx.getRelated(makeEnvelope())).toEqual([]);
  });

  it('returns other envelopes in the chain, excluding the query envelope', () => {
    const e1 = makeEnvelope({ correlation_id: 'c1', ts: 1 });
    const e2 = makeEnvelope({ correlation_id: 'c1', ts: 2 });
    const e3 = makeEnvelope({ correlation_id: 'c1', ts: 3 });
    idx.index(e1);
    idx.index(e2);
    idx.index(e3);

    const related = idx.getRelated(e2);
    expect(related).toHaveLength(2);
    expect(related).not.toContain(e2);
  });
});

describe('CorrelationIndex eviction', () => {
  it('evicts oldest chain when exceeding MAX_CHAINS (1000)', () => {
    // Index 1001 unique chains — the first should be evicted.
    for (let i = 0; i < 1001; i++) {
      idx.index(makeEnvelope({ correlation_id: `chain-${i}`, ts: i }));
    }
    expect(idx.chainSize('chain-0')).toBe(0);
    expect(idx.chainSize('chain-1')).toBe(1);
    expect(idx.chainSize('chain-1000')).toBe(1);
  });
});

describe('CorrelationIndex.reset', () => {
  it('clears all chains', () => {
    idx.index(makeEnvelope({ correlation_id: 'c1', ts: 1 }));
    idx.index(makeEnvelope({ correlation_id: 'c2', ts: 2 }));
    idx.reset();
    expect(idx.chainSize('c1')).toBe(0);
    expect(idx.chainSize('c2')).toBe(0);
  });
});
