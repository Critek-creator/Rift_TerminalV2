// Stage 2 — restart-safe sessions: frontend coordination.
//
// The Rust side persists a `.snapshot.json` (serialized xterm buffers + cwds +
// the tiling layout tree) next to each launch's `.jsonl` audit log +
// `.summary.json` digest. On boot the most recent *prior*-launch snapshot is
// re-hydrated before fresh shells spawn. This module is the singleton that:
//   1. fetches the restore payload exactly once (shared across panes + the
//      sessionManager boot path),
//   2. lets each pane claim its own restore once (Stage 2b multi-pane),
//   3. tracks which pane is focused so only that pane drives periodic capture,
//   4. holds the per-pane snapshot providers so the coordinator can gather the
//      whole active session (all leaves + layout) into one write,
//   5. caches the session-snapshot config (restore flag + interval).

import { invoke } from '@tauri-apps/api/core';

/** One pane's restorable state — mirrors `rift_bus::snapshot::PaneSnapshot`. */
export interface PaneSnapshot {
  pane_id: number;
  serialized: string;
  cwd: string;
  rows: number;
  cols: number;
  project_root?: string | null;
}

/** Mirrors `rift_bus::snapshot::RestorePayload`. */
export interface RestorePayload {
  session_id: string;
  saved_ms: number;
  panes: PaneSnapshot[];
  /** Opaque `SplitNode` tiling tree (Stage 2b). Rebuilt by sessionManager. */
  layout?: unknown;
  digest?: string | null;
}

/** Session-snapshot config slice read from `config_get`. */
export interface SnapshotConfig {
  restoreEnabled: boolean;
  intervalSeconds: number;
}

let restorePromise: Promise<RestorePayload | null> | null = null;
const consumedPanes = new Set<number>();
let activePaneId: number | null = null;
let cfgPromise: Promise<SnapshotConfig> | null = null;

/** Each pane registers a fn returning its current PaneSnapshot, keyed by id. */
const paneProviders = new Map<number, () => PaneSnapshot | null>();

/**
 * The prior-launch restore payload, fetched once and shared. The Rust command
 * already gates on `restore_on_startup`, so this resolves `null` when restore
 * is disabled or nothing qualifies.
 */
export function getRestorePayload(): Promise<RestorePayload | null> {
  if (!restorePromise) {
    restorePromise = invoke<RestorePayload | null>('session_snapshot_latest').catch(
      () => null,
    );
  }
  return restorePromise;
}

/**
 * Claim the one-shot restore for a specific pane. Each pane id restores at most
 * once (guards against a remount re-hydrating). Returns `false` if already
 * consumed.
 */
export function claimPane(paneId: number | undefined): boolean {
  if (paneId === undefined) return false;
  if (consumedPanes.has(paneId)) return false;
  consumedPanes.add(paneId);
  return true;
}

/** Delete a consumed snapshot so it does not replay on the next boot. The
 *  in-memory payload is already cached, so panes still hydrate after this. */
export function clearSnapshot(sessionId: string): void {
  invoke('session_snapshot_clear', { sessionId }).catch(() => {});
}

/** Register a pane's snapshot provider (called on mount once serializable). */
export function registerPaneProvider(
  paneId: number | undefined,
  provider: () => PaneSnapshot | null,
): void {
  if (paneId === undefined) return;
  paneProviders.set(paneId, provider);
}

/** Remove a pane's provider (on destroy). */
export function unregisterPaneProvider(paneId: number | undefined): void {
  if (paneId === undefined) return;
  paneProviders.delete(paneId);
}

/** Gather current snapshots for the given leaf ids via their providers, in
 *  order. Skips panes whose provider is absent or returns null. */
export function gatherPanes(leafIds: number[]): PaneSnapshot[] {
  const out: PaneSnapshot[] = [];
  for (const id of leafIds) {
    const snap = paneProviders.get(id)?.();
    if (snap) out.push(snap);
  }
  return out;
}

/** Persist a full active-session snapshot (all panes + opaque layout tree). */
export function writeSnapshot(panes: PaneSnapshot[], layout: unknown): void {
  if (panes.length === 0) return;
  invoke('session_snapshot_write', { panes, layout: layout ?? null }).catch(() => {});
}

/** Session-snapshot config (restore flag + capture interval), fetched once. */
export function getSnapshotConfig(): Promise<SnapshotConfig> {
  if (!cfgPromise) {
    cfgPromise = invoke<{ session?: { restore_on_startup?: boolean; snapshot_interval_seconds?: number } }>(
      'config_get',
    )
      .then((c) => ({
        restoreEnabled: !!c?.session?.restore_on_startup,
        intervalSeconds: Math.max(0, Number(c?.session?.snapshot_interval_seconds ?? 0)),
      }))
      .catch(() => ({ restoreEnabled: false, intervalSeconds: 0 }));
  }
  return cfgPromise;
}

/** Record which pane is focused — only the active pane drives periodic capture. */
export function setActivePane(id: number | null): void {
  activePaneId = id;
}

/** True when `id` is the focused pane (and a valid id). */
export function isActivePane(id: number | undefined): boolean {
  return id !== undefined && activePaneId === id;
}
