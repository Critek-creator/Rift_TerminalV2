/**
 * EnrichmentStore — Phase 8.6.1 frontend store.
 *
 * Holds §9 capability class 3 (data enrichment) payloads emitted by the
 * vault-walker translator (Category::Index, kind="enrichment"). App.svelte
 * populates via subscription; Tree.svelte reads in 8.6.2.
 *
 * Shape is snake_case — bus.ts performs no key transformation; payloads
 * pass through as the raw JSON the Rust side built.
 */

export interface EnrichmentEntry {
  vault_id: string;    // e.g. "p006"
  vault_kind: string;  // "project" | "practices" | "research" | "skill" | "lore" | "agent" | "hook"
  tags: string[];      // vault-sourced tags; may be empty in v1
}

export class EnrichmentStore {
  /**
   * key = fs_path (canonical absolute project-root path, forward-slash-normalized).
   * Assign-replace on every mutation so Svelte 5 $derived consumers re-run.
   */
  map = $state(new Map<string, EnrichmentEntry[]>());

  /** Flips true on walk.complete envelope; consumers can show "loading" UX. */
  loaded = $state(false);

  /**
   * Add or replace an enrichment entry. If an entry with the same vault_id
   * already exists at this fs_path, it is replaced (no duplicates).
   * Assign-replace pattern mirrors Tree.svelte:104-108 (collapsedDirs precedent).
   */
  ingest(payload: {
    fs_path: string;
    vault_id: string;
    vault_kind: string;
    tags: string[];
  }): void {
    const { fs_path, vault_id, vault_kind, tags } = payload;
    const next = new Map(this.map);
    const existing = next.get(fs_path) ?? [];
    const filtered = existing.filter((e) => e.vault_id !== vault_id);
    next.set(fs_path, [...filtered, { vault_id, vault_kind, tags }]);
    this.map = next;
  }

  /**
   * Drop all entries (across every fs_path) where entry.vault_id === vault_id.
   * Called when walker emits vault.update kind=deleted.
   * Empty arrays are pruned — after removal the key is absent, so get() returns
   * undefined (consistent contract for callers).
   */
  removeByVaultId(vault_id: string): void {
    const next = new Map<string, EnrichmentEntry[]>();
    for (const [fs_path, entries] of this.map) {
      const kept = entries.filter((e) => e.vault_id !== vault_id);
      if (kept.length > 0) {
        next.set(fs_path, kept);
      }
      // empty arrays → key pruned; get() returns undefined
    }
    this.map = next;
  }

  /**
   * Read entries for an fs_path, or undefined if none exist.
   * Used by Tree.svelte $derived lookups in 8.6.2.
   */
  get(fs_path: string): EnrichmentEntry[] | undefined {
    return this.map.get(fs_path);
  }
}

/** Singleton instance — App.svelte populates via subscription, Tree.svelte reads. */
export const enrichmentStore = new EnrichmentStore();
