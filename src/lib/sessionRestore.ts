// Stage 2 — restart-safe sessions: frontend coordination.
//
// The Rust side persists a `.snapshot.json` (serialized xterm buffer + cwd)
// next to each launch's `.jsonl` audit log + `.summary.json` digest. On boot
// the most recent *prior*-launch snapshot is re-hydrated into the terminal
// before a fresh shell spawns. This module is the small singleton that:
//   1. fetches the restore payload exactly once (shared across panes),
//   2. lets a single pane *claim* the restore (MVP restores one active pane),
//   3. tracks which pane is focused, so only that pane writes snapshots
//      (avoids multi-pane writers clobbering each other's snapshot file),
//   4. caches the session-snapshot config (restore flag + interval).
//
// Multi-pane layout restore is deliberately out of scope for the MVP (Stage
// 2b) — the on-disk schema already carries many panes, so it's additive.

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
  digest?: string | null;
}

/** Session-snapshot config slice read from `config_get`. */
export interface SnapshotConfig {
  restoreEnabled: boolean;
  intervalSeconds: number;
}

let restorePromise: Promise<RestorePayload | null> | null = null;
let restoreClaimed = false;
let activePaneId: number | null = null;
let cfgPromise: Promise<SnapshotConfig> | null = null;

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
 * Claim the one-shot restore for the calling pane. The first pane to claim wins
 * (MVP single-pane restore); subsequent callers get `false` and start fresh.
 */
export function claimRestore(): boolean {
  if (restoreClaimed) return false;
  restoreClaimed = true;
  return true;
}

/** Delete a consumed snapshot so it does not replay on the next boot. */
export function clearSnapshot(sessionId: string): void {
  invoke('session_snapshot_clear', { sessionId }).catch(() => {});
}

/** Persist a single pane's snapshot for the current launch (best-effort). */
export function writeSnapshot(pane: PaneSnapshot): void {
  invoke('session_snapshot_write', { panes: [pane] }).catch(() => {});
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

/** Record which pane is focused — only the active pane writes snapshots. */
export function setActivePane(id: number | null): void {
  activePaneId = id;
}

/** True when `id` is the focused pane (and a valid id). */
export function isActivePane(id: number | undefined): boolean {
  return id !== undefined && activePaneId === id;
}
