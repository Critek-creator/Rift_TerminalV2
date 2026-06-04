// commandFailureStore.svelte.ts — Phase 5 / R1.5: persistent shell-failure log.
//
// The per-line badge affordance is in-context but EPHEMERAL — xterm disposes
// the decoration when the line ages out of scrollback, so a failure scrolls
// away and is unrecoverable. This store subscribes to the `command.failed`
// bus stream (published by Terminal.svelte's R0 capture) and keeps a bounded,
// reactive, cross-pane log so the bottom-chrome issues list can surface
// failures that have long since scrolled off — and stays distinct from the
// Errors tab (Rift's own Tauri errors), per spec §6.
//
// Consecutive identical failures (same command + exit) cluster into one entry
// with a repeat count, so a retry loop reads as "×4", not four rows.

import { subscribe, type Envelope } from './bus';
import { failureClusterKey, type FailureContext } from './errorHandoff';

export interface FailureEntry extends FailureContext {
  /** Stable id for keying + the explain action. */
  id: string;
  sessionId: number | null;
  /** Epoch ms of the most recent occurrence. */
  ts: number;
  /** How many times this identical failure fired consecutively. */
  repeatCount: number;
  /** Cleared once the user has opened/acknowledged it. */
  acknowledged: boolean;
}

const MAX_ENTRIES = 50;
let nextId = 0;

let entries = $state<FailureEntry[]>([]);

let unsub: (() => Promise<void>) | undefined;
let started = false;

function ingest(env: Envelope): void {
  if (env.kind !== 'command.failed') return;
  const p = (env.payload ?? {}) as Partial<FailureContext> & { session_id?: number };
  if (typeof p.command !== 'string' || typeof p.exitCode !== 'number') return;

  const ts = Date.now();
  const key = failureClusterKey(p.command, p.exitCode);

  // Cluster only with the MOST RECENT entry — "consecutive" identical failures.
  const head = entries[0];
  if (head && failureClusterKey(head.command, head.exitCode) === key) {
    entries = [
      { ...head, ts, repeatCount: head.repeatCount + 1, acknowledged: false },
      ...entries.slice(1),
    ];
    return;
  }

  const entry: FailureEntry = {
    id: `fail-${nextId++}`,
    sessionId: p.session_id ?? null,
    command: p.command,
    cwd: p.cwd ?? null,
    exitCode: p.exitCode,
    durationMs: p.durationMs ?? null,
    startRow: p.startRow ?? 0,
    endRow: p.endRow ?? 0,
    scrollbackTail: Array.isArray(p.scrollbackTail) ? p.scrollbackTail : [],
    ts,
    repeatCount: 1,
    acknowledged: false,
  };
  entries = [entry, ...entries].slice(0, MAX_ENTRIES);
}

export const commandFailureStore = {
  /** Newest-first list of recent shell-command failures. */
  get entries(): FailureEntry[] {
    return entries;
  },
  /** Count of failures the user hasn't acknowledged yet (drives the chip). */
  get unacknowledgedCount(): number {
    return entries.reduce((n, e) => (e.acknowledged ? n : n + 1), 0);
  },
  /** Total entries (for an "N issues" label). */
  get count(): number {
    return entries.length;
  },

  /** Mark every entry acknowledged (called when the panel opens). */
  acknowledgeAll(): void {
    if (entries.every((e) => e.acknowledged)) return;
    entries = entries.map((e) => (e.acknowledged ? e : { ...e, acknowledged: true }));
  },

  /** Drop a single entry (per-row dismiss). */
  remove(id: string): void {
    entries = entries.filter((e) => e.id !== id);
  },

  /** Clear the whole log. */
  clear(): void {
    entries = [];
  },

  /** Reconstruct a FailureContext for the explain invocation. */
  contextFor(id: string): FailureContext | undefined {
    const e = entries.find((x) => x.id === id);
    if (!e) return undefined;
    return {
      command: e.command,
      cwd: e.cwd,
      exitCode: e.exitCode,
      durationMs: e.durationMs,
      startRow: e.startRow,
      endRow: e.endRow,
      scrollbackTail: e.scrollbackTail,
    };
  },

  /** Subscribe to the command.failed stream. Idempotent. */
  async start(): Promise<void> {
    if (started) return;
    started = true;
    unsub = await subscribe({ category: 'pty' }, ingest);
  },
  async stop(): Promise<void> {
    started = false;
    await unsub?.();
    unsub = undefined;
  },
};
