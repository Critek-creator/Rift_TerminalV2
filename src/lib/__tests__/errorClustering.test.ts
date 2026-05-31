import { describe, it, expect } from 'vitest';
import {
  normalizeSignature,
  extractLocation,
  clusterEvents,
  payloadToString,
} from '../errorClustering';
import type { Envelope } from '../bus';

function env(over: Partial<Envelope> = {}): Envelope {
  return {
    version: 1,
    category: 'system',
    kind: 'error',
    ts: 0,
    payload: null,
    ...over,
  };
}

describe('normalizeSignature', () => {
  it('collapses events differing only by clock time', () => {
    const a = normalizeSignature('error', 'failed at 2026-05-31T01:40:53Z retrying');
    const b = normalizeSignature('error', 'failed at 2026-05-31T09:12:00Z retrying');
    expect(a).toBe(b);
  });

  it('collapses events differing only by counter/number', () => {
    const a = normalizeSignature('error', 'connection refused (attempt 3)');
    const b = normalizeSignature('error', 'connection refused (attempt 17)');
    expect(a).toBe(b);
  });

  it('collapses events differing only by hex address or uuid', () => {
    expect(normalizeSignature('panic', 'null at 0xdeadbeef'))
      .toBe(normalizeSignature('panic', 'null at 0x00c0ffee'));
    expect(normalizeSignature('mcp.err', 'req 11111111-2222-3333-4444-555555555555 failed'))
      .toBe(normalizeSignature('mcp.err', 'req aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee failed'));
  });

  it('keeps genuinely different messages distinct', () => {
    const a = normalizeSignature('error', 'disk full');
    const b = normalizeSignature('error', 'permission denied');
    expect(a).not.toBe(b);
  });

  it('keeps the same message under different kinds distinct', () => {
    expect(normalizeSignature('warn', 'timeout')).not.toBe(normalizeSignature('error', 'timeout'));
  });
});

describe('extractLocation', () => {
  it('parses a POSIX file:line', () => {
    expect(extractLocation('thread panicked at src/lib.rs:1260')).toEqual({ file: 'src/lib.rs', line: 1260 });
  });

  it('parses a Windows file:line', () => {
    expect(extractLocation('error in crates\\rift-bus\\src\\lane.rs:42 here'))
      .toEqual({ file: 'crates\\rift-bus\\src\\lane.rs', line: 42 });
  });

  it('returns the last (innermost) location when several appear', () => {
    expect(extractLocation('a.ts:1 called b.ts:99')).toEqual({ file: 'b.ts', line: 99 });
  });

  it('returns null when no location is present', () => {
    expect(extractLocation('just a plain message')).toBeNull();
  });
});

describe('clusterEvents', () => {
  it('folds repeated structural duplicates into one cluster with a count', () => {
    const events = [
      env({ ts: 100, payload: 'connection refused (attempt 1)' }),
      env({ ts: 200, payload: 'connection refused (attempt 2)' }),
      env({ ts: 300, payload: 'connection refused (attempt 3)' }),
    ];
    const clusters = clusterEvents(events);
    expect(clusters).toHaveLength(1);
    expect(clusters[0].count).toBe(3);
    expect(clusters[0].firstTs).toBe(100);
    expect(clusters[0].lastTs).toBe(300);
    // Sample = newest member.
    expect(clusters[0].sample).toBe('connection refused (attempt 3)');
  });

  it('separates distinct problems and sorts by most-recent activity', () => {
    const events = [
      env({ ts: 100, kind: 'error', payload: 'disk full' }),
      env({ ts: 250, kind: 'error', payload: 'permission denied' }),
      env({ ts: 400, kind: 'error', payload: 'disk full' }),
    ];
    const clusters = clusterEvents(events);
    expect(clusters).toHaveLength(2);
    // disk-full cluster last seen at 400 → sorts first.
    expect(clusters[0].sample).toBe('disk full');
    expect(clusters[0].count).toBe(2);
    expect(clusters[1].sample).toBe('permission denied');
    expect(clusters[1].count).toBe(1);
  });

  it('attaches a parsed location to the cluster when the payload carries one', () => {
    const clusters = clusterEvents([env({ ts: 1, payload: 'panic at src/lib.rs:1260' })]);
    expect(clusters[0].location).toEqual({ file: 'src/lib.rs', line: 1260 });
  });

  it('returns an empty array for no events', () => {
    expect(clusterEvents([])).toEqual([]);
  });
});

describe('payloadToString', () => {
  it('passes strings through and serializes objects', () => {
    expect(payloadToString('hi')).toBe('hi');
    expect(payloadToString({ a: 1 })).toBe('{"a":1}');
    expect(payloadToString(null)).toBe('');
  });
});
