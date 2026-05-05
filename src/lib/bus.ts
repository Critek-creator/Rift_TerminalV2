/**
 * Frontend ↔ RiftBus bridge.
 *
 * Mirrors `crates/rift-bus/src/envelope.rs`. Keep these types in sync
 * — the Rust side is the source of truth.
 *
 * Spec: `decisions/§10.15_real-time_update_mechanism.md`.
 */

// Note: invoke().catch() handlers here are intentionally minimal — backend Tauri
// commands publish errors via rift_bus::publish_error before returning Err, so
// every error the frontend sees is already captured in Category::System on the bus.
import { Channel, invoke } from '@tauri-apps/api/core';

export type Category =
  | 'pty'
  | 'hook'
  | 'agent'
  | 'fs'
  | 'index'
  | 'aegis'
  | 'status'
  | 'system'
  | 'mcp';

export interface Envelope {
  version: number;
  category: Category;
  kind: string;
  ts: number;
  payload: unknown;
}

export interface SubscribeOptions {
  /** Filter to a single category. Omit to receive every category. */
  category?: Category;
}

// Ready-gate: subscribe() blocks until signalBusReady() is called.
// App.svelte calls rift_reset_for_reload first, then signals ready.
// This eliminates the race where $effects create subscriptions BEFORE
// the reset kills orphans from the previous page load.
let _readyResolve: () => void;
let _readyPromise = new Promise<void>((r) => { _readyResolve = r; });

export function signalBusReady(): void {
  _readyResolve();
}

export function resetBusReady(): void {
  _readyPromise = new Promise<void>((r) => { _readyResolve = r; });
}

/**
 * Subscribe to bus envelopes. Replay snapshot drains synchronously into
 * `onEnvelope` first, followed by live events. Returns an `unsubscribe`
 * function — call it on component teardown.
 */
export async function subscribe(
  options: SubscribeOptions,
  onEnvelope: (envelope: Envelope) => void,
): Promise<() => Promise<void>> {
  await _readyPromise;
  const channel = new Channel<Envelope>();
  channel.onmessage = onEnvelope;
  const id = await invoke<number>('bus_subscribe', {
    category: options.category ?? null,
    onEnvelope: channel,
  });
  return async () => {
    await invoke('bus_unsubscribe', { id });
  };
}

/**
 * Publish an envelope to the bus. Useful for demo/debug paths and the
 * eventual translator-module pattern when a translator runs in-process.
 */
export async function publish(
  category: Category,
  kind: string,
  payload?: unknown,
): Promise<void> {
  await invoke('bus_publish', {
    category,
    kind,
    payload: payload ?? null,
  });
}
