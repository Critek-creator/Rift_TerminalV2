import { describe, it, expect, vi, beforeEach } from 'vitest';

// Smoke test for `src/lib/bus.ts` — verifies the subscribe/publish
// frontend ↔ backend contract holds. Mocks `@tauri-apps/api/core` so
// the test runs in jsdom without a Tauri runtime.
//
// Bug classes this catches:
//   - Subscribe stops passing a Channel (would silently break replay drain).
//   - bus_unsubscribe id leaks (returned unsubscribe doesn't actually unsubscribe).
//   - Publish payload shape changes without matching backend command signature.
//
// What it does NOT catch: real envelope flow over the bus (that's
// integration territory, covered by the Rust-side tests in
// `crates/rift-bus/src/bus.rs`).

const invokeMock = vi.fn();

// Channel is a class with `.id` and settable `.onmessage`. The shape we
// care about is that `bus.ts` constructs one, sets onmessage, and passes
// it to invoke. Track instances so tests can introspect.
const channelInstances: Array<{ id: number; onmessage: ((env: unknown) => void) | null }> = [];

vi.mock('@tauri-apps/api/core', () => {
  let nextId = 1;
  class Channel<T> {
    id: number;
    onmessage: ((env: T) => void) | null = null;
    constructor() {
      this.id = nextId++;
      channelInstances.push(this as { id: number; onmessage: ((env: unknown) => void) | null });
    }
  }
  return { invoke: invokeMock, Channel };
});

beforeEach(async () => {
  invokeMock.mockReset();
  channelInstances.length = 0;
  // bus.ts has a ready-gate: subscribe() blocks until signalBusReady().
  const { signalBusReady } = await import('../bus');
  signalBusReady();
});

describe('bus.ts subscribe/unsubscribe round-trip', () => {
  it('subscribe constructs a Channel, sets onmessage, and invokes bus_subscribe', async () => {
    const { subscribe } = await import('../bus');
    invokeMock.mockResolvedValueOnce(42); // backend-assigned subscription id

    const handler = vi.fn();
    const unsub = await subscribe({ category: 'aegis' }, handler);

    expect(channelInstances).toHaveLength(1);
    expect(channelInstances[0].onmessage).toBe(handler);

    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith(
      'bus_subscribe',
      expect.objectContaining({
        category: 'aegis',
        onEnvelope: expect.any(Object),
      }),
    );
    expect(typeof unsub).toBe('function');
  });

  it('subscribe with no category sends category=null (subscribe-all)', async () => {
    const { subscribe } = await import('../bus');
    invokeMock.mockResolvedValueOnce(7);
    await subscribe({}, () => {});
    expect(invokeMock).toHaveBeenCalledWith(
      'bus_subscribe',
      expect.objectContaining({ category: null }),
    );
  });

  it('returned unsubscribe invokes bus_unsubscribe with the assigned id', async () => {
    const { subscribe } = await import('../bus');
    invokeMock.mockResolvedValueOnce(99);
    const unsub = await subscribe({ category: 'fs' }, () => {});

    invokeMock.mockResolvedValueOnce(undefined);
    await unsub();

    expect(invokeMock).toHaveBeenCalledTimes(2);
    expect(invokeMock).toHaveBeenLastCalledWith('bus_unsubscribe', { id: 99 });
  });
});

describe('bus.ts publish', () => {
  it('publish forwards category, kind, and payload', async () => {
    const { publish } = await import('../bus');
    invokeMock.mockResolvedValueOnce(undefined);

    await publish('hook', 'demo.click', { source: 'unit-test' });

    expect(invokeMock).toHaveBeenCalledWith('bus_publish', {
      category: 'hook',
      kind: 'demo.click',
      payload: { source: 'unit-test' },
    });
  });

  it('publish with omitted payload sends payload=null', async () => {
    const { publish } = await import('../bus');
    invokeMock.mockResolvedValueOnce(undefined);

    await publish('system', 'tick');

    expect(invokeMock).toHaveBeenCalledWith('bus_publish', {
      category: 'system',
      kind: 'tick',
      payload: null,
    });
  });
});
