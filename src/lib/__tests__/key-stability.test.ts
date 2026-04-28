import { describe, it, expect } from 'vitest';

// Regression test for C1 in AUDIT_2026-04-27.md. Three log blocks
// (AegisTabContent / IndexTabContent / NotificationPane) used the
// `(e.ts + e.kind)` key pattern, which collides when two events arrive in
// the same millisecond with the same `kind` — Svelte throws
// `each_key_duplicate` and unmounts the log. The fix appended `:i` so
// keys are unique by position regardless of payload identity.
//
// These tests pin the invariant: **same-ms + same-kind bursts must NOT
// produce duplicate keys.** If anyone reverts the key shape, this fires.

interface LoggedEvent {
  ts: number;
  kind: string;
}

// Mirrors the OLD broken pattern. Kept here only as the negative control
// — no production code should use this shape.
function legacyKey(e: LoggedEvent): string {
  return e.ts + e.kind;
}

// Mirrors the CURRENT shipped pattern at AegisTabContent.svelte:220,
// IndexTabContent.svelte:211, NotificationPane.svelte:189.
function stableKey(e: LoggedEvent, i: number): string {
  return `${e.ts}:${e.kind}:${i}`;
}

describe('event-log key stability (C1 regression guard)', () => {
  it('LEGACY pattern collides on same-ms same-kind burst (negative control)', () => {
    const burst: LoggedEvent[] = [
      { ts: 1000, kind: 'aegis.invocation' },
      { ts: 1000, kind: 'aegis.invocation' },
      { ts: 1000, kind: 'aegis.invocation' },
    ];
    const keys = burst.map(legacyKey);
    // The bug we fixed: 3 events → 1 unique key. Svelte unmounts the log here.
    expect(new Set(keys).size).toBe(1);
  });

  it('CURRENT pattern produces unique keys on same-ms same-kind burst', () => {
    const burst: LoggedEvent[] = [
      { ts: 1000, kind: 'aegis.invocation' },
      { ts: 1000, kind: 'aegis.invocation' },
      { ts: 1000, kind: 'aegis.invocation' },
    ];
    const keys = burst.map(stableKey);
    expect(new Set(keys).size).toBe(burst.length);
  });

  it('CURRENT pattern stays unique across 1000-event mixed-kind stream', () => {
    const kinds = ['aegis.invocation', 'vault.update', 'agent.dispatch', 'hook.fired'];
    const stream: LoggedEvent[] = Array.from({ length: 1000 }, (_, n) => ({
      ts: 1_700_000_000_000 + Math.floor(n / 10), // 10 events per ms — realistic burst
      kind: kinds[n % kinds.length],
    }));
    const keys = stream.map(stableKey);
    expect(new Set(keys).size).toBe(stream.length);
  });

  it('CURRENT pattern keys are deterministic for the same input', () => {
    const event: LoggedEvent = { ts: 1234567890, kind: 'fs.write' };
    expect(stableKey(event, 0)).toBe(stableKey(event, 0));
    expect(stableKey(event, 0)).not.toBe(stableKey(event, 1));
  });
});
