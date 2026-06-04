// Reactive single-source config cache (P3 #17).
//
// Replaces the scattered per-component `invoke('config_get')` round-trips with
// one shared, reactive snapshot. `load()` fetches once and caches, deduping
// concurrent callers (e.g. App.svelte's appearance/alerts/llm loaders all
// firing on mount → a single IPC call). `reload()` forces a fresh fetch after a
// `config_save`. The owning surface (App.svelte) calls reload() on the
// `rift:config-changed` broadcast so every reader stays coherent.
//
// SettingsPanel deliberately keeps its own mutable working copy + optimistic
// save/rollback — it is the editor, not a reader, so it does NOT consume this
// store. Other read-only consumers can migrate to it incrementally.

import { invoke } from '@tauri-apps/api/core';
import type { RiftConfig } from './riftConfig';

let config = $state<RiftConfig | null>(null);
let error = $state<string | null>(null);
let inflight: Promise<RiftConfig | null> | null = null;

async function fetchConfig(): Promise<RiftConfig | null> {
  try {
    config = await invoke<RiftConfig>('config_get');
    error = null;
    return config;
  } catch (e) {
    error = String(e);
    console.error('[configStore] config_get failed', e);
    return null;
  }
}

/** Cached config, fetching once if absent. Concurrent callers share one
 *  in-flight request, so a burst of mount-time readers costs a single IPC. */
async function load(): Promise<RiftConfig | null> {
  if (config) return config;
  if (inflight) return inflight;
  inflight = fetchConfig().finally(() => { inflight = null; });
  return inflight;
}

/** Force a fresh fetch — call after a `config_save` (or on the
 *  `rift:config-changed` broadcast) so cached readers see the new state. */
async function reload(): Promise<RiftConfig | null> {
  inflight = fetchConfig().finally(() => { inflight = null; });
  return inflight;
}

export const configStore = {
  /** Latest snapshot, or null before the first load resolves. Reactive. */
  get current(): RiftConfig | null { return config; },
  /** Last load error, or null. Reactive. */
  get error(): string | null { return error; },
  load,
  reload,
};
