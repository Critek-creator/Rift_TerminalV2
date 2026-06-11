/**
 * EnrichmentStore — frontend store for §9 capability class 3 (data enrichment).
 *
 * Originally hard-wired to the Index vault-walker (Category::Index,
 * kind="enrichment"). Generalized to the provider-agnostic enrichment registry
 * (Category::System, kind="enrichment.attach") so ANY integration can enrich a
 * filesystem node — the class-3 analogue of the control-endpoint action
 * registry. Entries are keyed per (provider_id, entry_id) at an fs_path.
 *
 * Conflict resolution: per-path, per-(provider_id, entry_id), last-write-wins.
 * Two providers (or two entries) at the same path coexist as separate rows —
 * no merge. The Index provider dogfoods this with provider_id="index",
 * entry_id=<vault_id>, so each vault stays its own row exactly as before.
 *
 * Shape is snake_case — bus.ts performs no key transformation; payloads pass
 * through as the raw JSON the Rust side built.
 *
 * Resilience: entries are snapshotted to localStorage after every mutation so
 * that entries attached early in a vault-walk (whose bus envelopes may scroll
 * out of the bounded replay buffer before this store subscribes) survive a
 * reload. On init, the store rehydrates from the snapshot and then reconciles
 * with live bus events — live events always win (last-write-wins unchanged).
 *
 * Audit debt #3 fix: localStorage snapshot / rehydrate path. A fuller fix
 * (bus-level persistent replay or server-side snapshot endpoint) would require
 * backend changes; that design is deferred — see followups.
 */

/** localStorage key for the enrichment snapshot. */
export const ENRICHMENT_STORAGE_KEY = 'rift.enrichment.snapshot.v1';

/**
 * Hard cap on the number of unique fs_paths kept in the live map.
 * When exceeded, the least-recently-touched paths are evicted first.
 */
const MAX_PATHS = 2000;

/**
 * Maximum number of paths persisted to localStorage (most-recently-touched
 * first). Keeps the serialized snapshot well inside the 5 MB quota.
 */
const MAX_PERSIST_PATHS = 500;

export interface EnrichmentEntry {
  /** Integration namespace, e.g. "index", "git". */
  provider_id: string;
  /** Unique within (provider_id, fs_path) — the dedup slot key. */
  entry_id: string;
  /** Display label for the tooltip row (falls back to entry_id). */
  label?: string;
  /** Provider-sourced tags; may be empty. */
  tags: string[];
  /** Provider-specific opaque bag (Index: { vault_id, vault_kind }). */
  data?: unknown;
  // Index-compat conveniences, surfaced when provider_id === "index":
  vault_id?: string;
  vault_kind?: string;
}

/** Serialized shape written to localStorage. */
interface EnrichmentSnapshot {
  /** Incremented if the schema ever needs a breaking migration. */
  version: 1;
  /** Entries from the in-memory map, flattened with fs_path inline. */
  entries: Array<EnrichmentEntry & { fs_path: string }>;
}

/**
 * Deserialize a localStorage snapshot. Returns an empty Map on any parse or
 * schema error so the store degrades gracefully rather than throwing on init.
 */
function loadSnapshot(storage: Storage = localStorage): Map<string, EnrichmentEntry[]> {
  try {
    const raw = storage.getItem(ENRICHMENT_STORAGE_KEY);
    if (!raw) return new Map();
    const parsed: EnrichmentSnapshot = JSON.parse(raw);
    // Version guard — discard snapshots from unknown future versions.
    if (parsed.version !== 1 || !Array.isArray(parsed.entries)) return new Map();
    const map = new Map<string, EnrichmentEntry[]>();
    for (const { fs_path, ...entry } of parsed.entries) {
      if (typeof fs_path !== 'string' || typeof entry.provider_id !== 'string') continue;
      const bucket = map.get(fs_path) ?? [];
      bucket.push(entry);
      map.set(fs_path, bucket);
    }
    return map;
  } catch {
    // Corrupt JSON or SecurityError (private browsing with storage disabled) —
    // degrade to empty rather than crashing.
    return new Map();
  }
}

/**
 * Serialize the current map to localStorage. Silently swallows errors
 * (QuotaExceededError, SecurityError) so a full storage quota never crashes
 * the app — enrichment dots just won't persist until space is freed.
 */
function saveSnapshot(map: Map<string, EnrichmentEntry[]>, storage: Storage = localStorage): void {
  try {
    const entries: Array<EnrichmentEntry & { fs_path: string }> = [];
    for (const [fs_path, bucket] of map) {
      for (const entry of bucket) {
        entries.push({ fs_path, ...entry });
      }
    }
    const snapshot: EnrichmentSnapshot = { version: 1, entries };
    storage.setItem(ENRICHMENT_STORAGE_KEY, JSON.stringify(snapshot));
  } catch {
    // Quota exceeded or security error — best-effort, do not throw.
  }
}

export class EnrichmentStore {
  /**
   * key = fs_path (canonical absolute path, forward-slash-normalized).
   * Assign-replace on every mutation so Svelte 5 $derived consumers re-run.
   *
   * Initialized from the localStorage snapshot so entries survive a reload
   * even when vault-walk envelopes have scrolled out of the bus replay buffer.
   * Live bus events reconcile on top via ingest() — last-write-wins unchanged.
   *
   * An injectable `_storage` parameter is accepted so tests can pass a
   * controlled Storage instance instead of the real localStorage.
   */
  map = $state(new Map<string, EnrichmentEntry[]>());

  /** Flips true on walk.complete envelope; consumers can show "loading" UX. */
  loaded = $state(false);

  /** Storage backend — real localStorage in production, injected in tests. */
  private _storage: Storage;

  /**
   * Touch-order tracker for LRU eviction.
   * A path is deleted and re-inserted at the end on every touch so the Map
   * iteration order (insertion order) reflects recency: oldest first.
   */
  private _touchOrder = new Map<string, true>();

  constructor(storage: Storage = localStorage) {
    this._storage = storage;
    // Rehydrate from snapshot; live events overlay on top.
    this.map = loadSnapshot(storage);
    // Seed touch-order from snapshot paths (order is arbitrary on init, but
    // all paths need to be tracked so eviction works immediately).
    for (const path of this.map.keys()) {
      this._touchOrder.set(path, true);
    }
  }

  /** Mark a path as most-recently used. */
  private _touch(path: string): void {
    this._touchOrder.delete(path);
    this._touchOrder.set(path, true);
  }

  /**
   * Evict the least-recently-touched paths until the live map is within
   * MAX_PATHS. Creates a new Map identity so Svelte 5 $derived consumers
   * re-run (in-place Map.delete() is not tracked).
   */
  private _evictIfNeeded(): void {
    if (this._touchOrder.size <= MAX_PATHS) return;
    const toRemove = this._touchOrder.size - MAX_PATHS;
    const evicted: string[] = [];
    let removed = 0;
    for (const path of this._touchOrder.keys()) {
      if (removed >= toRemove) break;
      evicted.push(path);
      removed++;
    }
    for (const path of evicted) {
      this._touchOrder.delete(path);
    }
    const next = new Map(this.map);
    for (const path of evicted) {
      next.delete(path);
    }
    this.map = next;
  }

  /**
   * Serialize to localStorage, capping to MAX_PERSIST_PATHS most-recently-
   * touched paths so the snapshot never exhausts the 5 MB quota.
   */
  private _saveSnapshot(): void {
    // Collect all paths in recency order (newest last in _touchOrder).
    const allPaths = Array.from(this._touchOrder.keys());
    // Persist newest MAX_PERSIST_PATHS paths.
    const persistPaths = new Set(allPaths.slice(-MAX_PERSIST_PATHS));
    const limited = new Map<string, EnrichmentEntry[]>();
    for (const [path, entries] of this.map) {
      if (persistPaths.has(path)) limited.set(path, entries);
    }
    saveSnapshot(limited, this._storage);
  }

  /**
   * Add or replace an enrichment entry. An existing entry with the same
   * (provider_id, entry_id) at this fs_path is replaced (no duplicates).
   *
   * Assign-replace with a NEW Map identity: a plain `$state(new Map())` is not
   * deeply reactive (Map mutations via .set() aren't tracked, and
   * `this.map = this.map` reassigns the same reference so Svelte sees no
   * change) — only a fresh identity re-runs $derived consumers (Tree.svelte
   * enrichment tags).
   */
  ingest(entry: EnrichmentEntry & { fs_path: string }): void {
    const { fs_path, ...e } = entry;
    const existing = this.map.get(fs_path) ?? [];
    const filtered = existing.filter(
      (x) => !(x.provider_id === e.provider_id && x.entry_id === e.entry_id),
    );
    const next = new Map(this.map);
    next.set(fs_path, [...filtered, e]);
    this._touch(fs_path);
    // Assign first so _evictIfNeeded operates on the current map; it will
    // reassign this.map again if paths need to be evicted.
    this.map = next;
    this._evictIfNeeded();
    this._saveSnapshot();
  }

  /**
   * Remove a provider's entries. When `entry_id` is given, only that slot is
   * removed across all paths; otherwise every entry for the provider is dropped
   * (e.g. on integration shutdown). Empty arrays are pruned, so get() returns
   * undefined afterward.
   */
  removeByProvider(provider_id: string, entry_id?: string): void {
    const next = new Map<string, EnrichmentEntry[]>();
    for (const [fs_path, entries] of this.map) {
      const kept = entries.filter((e) => {
        const isTarget =
          e.provider_id === provider_id &&
          (entry_id === undefined || e.entry_id === entry_id);
        return !isTarget;
      });
      if (kept.length > 0) next.set(fs_path, kept);
      else this._touchOrder.delete(fs_path);
    }
    this.map = next;
    this._saveSnapshot();
  }

  /**
   * Remove a provider's entries at a single fs_path (the targeted
   * `enrichment.revoke` with a non-null fs_path). Empty arrays are pruned.
   */
  removeByProviderAtPath(provider_id: string, fs_path: string): void {
    const entries = this.map.get(fs_path);
    if (!entries) return;
    const kept = entries.filter((e) => e.provider_id !== provider_id);
    const next = new Map(this.map);
    if (kept.length > 0) {
      next.set(fs_path, kept);
    } else {
      next.delete(fs_path);
      this._touchOrder.delete(fs_path);
    }
    this.map = next;
    this._saveSnapshot();
  }

  /**
   * Backward-compat: drop the Index provider's entry for `vault_id`.
   * Maps onto the generic removeByProvider path.
   */
  removeByVaultId(vault_id: string): void {
    this.removeByProvider('index', vault_id);
  }

  /**
   * Read entries for an fs_path, or undefined if none exist.
   * Used by Tree.svelte $derived lookups.
   */
  get(fs_path: string): EnrichmentEntry[] | undefined {
    return this.map.get(fs_path);
  }
}

/** Singleton — populated via subscription, read by Tree.svelte. */
export const enrichmentStore = new EnrichmentStore();
