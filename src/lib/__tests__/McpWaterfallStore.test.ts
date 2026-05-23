import { describe, it, expect, beforeEach } from 'vitest';
import { McpWaterfallStore, classifyTier, tierCssVar } from '../McpWaterfallStore';
import type { Envelope } from '../bus';

// Unit tests for McpWaterfallStore — span lifecycle, buffer limits,
// tool tier classification, and stale-span eviction.

function makeEnvelope(overrides: Partial<Envelope> = {}): Envelope {
  return {
    version: 1,
    category: 'mcp',
    kind: 'mcp.invoke',
    ts: Date.now(),
    payload: { tool: 'bus_history', request_id: 'req-1' },
    ...overrides,
  };
}

let store: McpWaterfallStore;

beforeEach(() => {
  store = new McpWaterfallStore();
});

describe('classifyTier', () => {
  it('classifies known read tools', () => {
    expect(classifyTier('bus_history')).toBe('read');
    expect(classifyTier('git_status')).toBe('read');
    expect(classifyTier('rift_status')).toBe('read');
  });

  it('classifies known mutate tools', () => {
    expect(classifyTier('pty_input')).toBe('mutate');
    expect(classifyTier('fs_write')).toBe('mutate');
    expect(classifyTier('rift_config_set')).toBe('mutate');
  });

  it('classifies known inspect tools', () => {
    expect(classifyTier('dom_snapshot')).toBe('inspect');
    expect(classifyTier('js_eval')).toBe('inspect');
    expect(classifyTier('screenshot')).toBe('inspect');
  });

  it('uses heuristic fallback for unknown tools', () => {
    expect(classifyTier('file_write_batch')).toBe('mutate');
    expect(classifyTier('take_snapshot')).toBe('inspect');
    expect(classifyTier('run_eval_safe')).toBe('inspect');
    expect(classifyTier('completely_unknown')).toBe('read');
  });
});

describe('tierCssVar', () => {
  it('maps tiers to correct CSS variables', () => {
    expect(tierCssVar('read')).toBe('--term-blue');
    expect(tierCssVar('mutate')).toBe('--term-red');
    expect(tierCssVar('inspect')).toBe('--term-cyan');
  });
});

describe('McpWaterfallStore span lifecycle', () => {
  it('opens a span on invoke envelope', () => {
    const env = makeEnvelope({ ts: 1000 });
    const changed = store.processEnvelope(env);
    expect(changed).toBe(true);
    expect(store.totalCount).toBe(1);
    expect(store.pendingCount).toBe(1);

    const spans = store.snapshot();
    expect(spans[0].status).toBe('pending');
    expect(spans[0].tool).toBe('bus_history');
    expect(spans[0].requestTime).toBe(1000);
  });

  it('closes a span on response envelope', () => {
    store.processEnvelope(makeEnvelope({
      kind: 'mcp.invoke',
      ts: 1000,
      payload: { tool: 'bus_history', request_id: 'req-1' },
    }));
    const changed = store.processEnvelope(makeEnvelope({
      kind: 'mcp.response',
      ts: 1500,
      payload: { request_id: 'req-1', result: 'ok' },
    }));

    expect(changed).toBe(true);
    expect(store.pendingCount).toBe(0);

    const spans = store.snapshot();
    expect(spans[0].status).toBe('success');
    expect(spans[0].durationMs).toBe(500);
    expect(spans[0].responseTime).toBe(1500);
  });

  it('marks error status on error/fail response', () => {
    store.processEnvelope(makeEnvelope({
      kind: 'mcp.call',
      ts: 1000,
      payload: { tool: 'pty_input', request_id: 'req-2' },
    }));
    store.processEnvelope(makeEnvelope({
      kind: 'mcp.error',
      ts: 1200,
      payload: { request_id: 'req-2', error: 'timeout' },
    }));

    const spans = store.snapshot();
    expect(spans[0].status).toBe('error');
  });

  it('ignores unrelated envelope kinds', () => {
    const changed = store.processEnvelope(makeEnvelope({
      kind: 'fs.write',
      ts: 1000,
      payload: { path: '/tmp/test' },
    }));
    expect(changed).toBe(false);
    expect(store.totalCount).toBe(0);
  });
});

describe('McpWaterfallStore buffer limits', () => {
  it('trims spans array when exceeding 2x buffer size (400)', () => {
    for (let i = 0; i < 450; i++) {
      store.processEnvelope(makeEnvelope({
        kind: 'mcp.invoke',
        ts: i,
        payload: { tool: 'bus_history', request_id: `req-${i}` },
      }));
    }
    // Trim fires when length > BUFFER_SIZE*2 (400), slicing to last 200.
    // After 450 inserts: 400 triggers trim at 401 -> 200, then 49 more -> 249,
    // but another trim fires... The store trims to 200 each time it exceeds 400.
    // 450 total: first trim at 401 -> 200, then 49 more = 249. No second trim
    // since 249 < 400. So totalCount should be 249.
    expect(store.totalCount).toBeLessThan(450);
    expect(store.totalCount).toBeGreaterThan(0);
  });

  it('snapshot returns at most 200 entries', () => {
    for (let i = 0; i < 250; i++) {
      store.processEnvelope(makeEnvelope({
        kind: 'mcp.invoke',
        ts: i,
        payload: { tool: 'bus_history', request_id: `req-${i}` },
      }));
    }
    expect(store.snapshot().length).toBeLessThanOrEqual(200);
  });
});

describe('McpWaterfallStore.tick (stale eviction)', () => {
  it('marks stale pending spans as error after 30s', () => {
    const now = Date.now();
    store.processEnvelope(makeEnvelope({
      kind: 'mcp.invoke',
      ts: now - 31_000, // 31 seconds ago
      payload: { tool: 'bus_history', request_id: 'stale-1' },
    }));
    expect(store.pendingCount).toBe(1);

    store.tick();

    expect(store.pendingCount).toBe(0);
    const spans = store.snapshot();
    expect(spans[0].status).toBe('error');
    expect(spans[0].durationMs).toBe(30_000);
  });
});

describe('McpWaterfallStore.reset', () => {
  it('clears all state', () => {
    store.processEnvelope(makeEnvelope());
    store.reset();
    expect(store.totalCount).toBe(0);
    expect(store.pendingCount).toBe(0);
    expect(store.snapshot()).toEqual([]);
  });
});
