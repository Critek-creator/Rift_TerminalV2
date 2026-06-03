import { describe, it, expect, beforeEach } from 'vitest';
import { EnrichmentStore, ENRICHMENT_STORAGE_KEY } from '../enrichmentStore.svelte';

// Unit tests for EnrichmentStore.
// Tests the store's public API in isolation. App.svelte subscription wiring
// is integration territory and is NOT tested here.
//
// The store is now provider-agnostic (§9 class-3 generalization): entries dedup
// on (provider_id, entry_id) at an fs_path. The Index provider dogfoods it with
// provider_id="index", entry_id=<vault_id>, so vault_id/vault_kind remain
// surfaced for the tooltip.
//
// A fresh EnrichmentStore instance is created per test via beforeEach to
// guarantee no cross-test state leakage (avoids touching the singleton).

let store: EnrichmentStore;

beforeEach(() => {
  // Clear jsdom localStorage so snapshot data from one test does not rehydrate
  // into the next (EnrichmentStore now persists on every mutation).
  localStorage.clear();
  store = new EnrichmentStore();
});

describe('EnrichmentStore.ingest', () => {
  it('puts entry under fs_path key', () => {
    store.ingest({
      fs_path: '/home/user/project',
      provider_id: 'index',
      entry_id: 'p006',
      vault_id: 'p006',
      vault_kind: 'project',
      tags: ['rift', 'terminal'],
    });

    const result = store.get('/home/user/project');
    expect(result).toHaveLength(1);
    expect(result![0]).toEqual({
      provider_id: 'index',
      entry_id: 'p006',
      vault_id: 'p006',
      vault_kind: 'project',
      tags: ['rift', 'terminal'],
    });
  });

  it('with duplicate (provider_id, entry_id)+fs_path replaces entry, no duplication', () => {
    store.ingest({
      fs_path: '/home/user/project',
      provider_id: 'index',
      entry_id: 'p006',
      vault_id: 'p006',
      vault_kind: 'project',
      tags: ['first'],
    });
    store.ingest({
      fs_path: '/home/user/project',
      provider_id: 'index',
      entry_id: 'p006',
      vault_id: 'p006',
      vault_kind: 'project',
      tags: ['second'],
    });

    const result = store.get('/home/user/project');
    expect(result).toHaveLength(1);
    expect(result![0].tags).toEqual(['second']);
  });

  it('stacks entries with different entry_ids at same fs_path', () => {
    store.ingest({
      fs_path: '/home/user/project',
      provider_id: 'index',
      entry_id: 'p006',
      vault_id: 'p006',
      vault_kind: 'project',
      tags: ['rift'],
    });
    store.ingest({
      fs_path: '/home/user/project',
      provider_id: 'index',
      entry_id: 'pr003',
      vault_id: 'pr003',
      vault_kind: 'practices',
      tags: ['gotchas'],
    });

    const result = store.get('/home/user/project');
    expect(result).toHaveLength(2);
    const ids = result!.map((e) => e.vault_id);
    expect(ids).toContain('p006');
    expect(ids).toContain('pr003');
  });

  it('lets two different providers coexist at the same fs_path', () => {
    store.ingest({
      fs_path: '/a',
      provider_id: 'index',
      entry_id: 'p006',
      vault_id: 'p006',
      vault_kind: 'project',
      tags: [],
    });
    store.ingest({
      fs_path: '/a',
      provider_id: 'git',
      entry_id: 'blame',
      label: 'main@a1b2c3',
      tags: ['HEAD'],
    });

    const result = store.get('/a');
    expect(result).toHaveLength(2);
    const providers = result!.map((e) => e.provider_id);
    expect(providers).toContain('index');
    expect(providers).toContain('git');
  });
});

describe('EnrichmentStore.removeByVaultId (Index back-compat)', () => {
  it('drops entries across all fs_paths for the given vault_id', () => {
    store.ingest({ fs_path: '/a', provider_id: 'index', entry_id: 'p006', vault_id: 'p006', vault_kind: 'project', tags: [] });
    store.ingest({ fs_path: '/b', provider_id: 'index', entry_id: 'p006', vault_id: 'p006', vault_kind: 'project', tags: [] });

    store.removeByVaultId('p006');

    // Both paths should return undefined — empty arrays are pruned (key removed).
    expect(store.get('/a')).toBeUndefined();
    expect(store.get('/b')).toBeUndefined();
  });

  it('leaves other vault_ids intact when removing one vault_id', () => {
    store.ingest({ fs_path: '/a', provider_id: 'index', entry_id: 'p006',  vault_id: 'p006',  vault_kind: 'project',   tags: ['rift'] });
    store.ingest({ fs_path: '/a', provider_id: 'index', entry_id: 'pr003', vault_id: 'pr003', vault_kind: 'practices', tags: ['gotchas'] });

    store.removeByVaultId('p006');

    const result = store.get('/a');
    expect(result).toHaveLength(1);
    expect(result![0].vault_id).toBe('pr003');
    expect(result![0].tags).toEqual(['gotchas']);
  });
});

describe('EnrichmentStore.removeByProvider', () => {
  it('drops only the named provider, leaving others intact', () => {
    store.ingest({ fs_path: '/a', provider_id: 'index', entry_id: 'p006', vault_id: 'p006', vault_kind: 'project', tags: [] });
    store.ingest({ fs_path: '/a', provider_id: 'git', entry_id: 'blame', label: 'x', tags: [] });

    store.removeByProvider('git');

    const result = store.get('/a');
    expect(result).toHaveLength(1);
    expect(result![0].provider_id).toBe('index');
  });
});

describe('EnrichmentStore.removeByProviderAtPath', () => {
  it('removes a provider only at the named path, leaving the same provider elsewhere', () => {
    store.ingest({ fs_path: '/a', provider_id: 'git', entry_id: 'blame', label: 'x', tags: [] });
    store.ingest({ fs_path: '/b', provider_id: 'git', entry_id: 'blame', label: 'y', tags: [] });

    store.removeByProviderAtPath('git', '/a');

    expect(store.get('/a')).toBeUndefined();
    expect(store.get('/b')).toHaveLength(1);
  });

  it('leaves other providers at the same path intact', () => {
    store.ingest({ fs_path: '/a', provider_id: 'index', entry_id: 'p006', vault_id: 'p006', vault_kind: 'project', tags: [] });
    store.ingest({ fs_path: '/a', provider_id: 'git', entry_id: 'blame', label: 'x', tags: [] });

    store.removeByProviderAtPath('git', '/a');

    const result = store.get('/a');
    expect(result).toHaveLength(1);
    expect(result![0].provider_id).toBe('index');
  });
});

describe('EnrichmentStore reactivity contract', () => {
  it('map identity changes on ingest (Svelte 5 assign-replace)', () => {
    const before = store.map;
    store.ingest({ fs_path: '/a', provider_id: 'index', entry_id: 'p006', vault_id: 'p006', vault_kind: 'project', tags: [] });
    expect(store.map).not.toBe(before);
  });
});

describe('EnrichmentStore.loaded flag', () => {
  it('defaults to false', () => {
    expect(store.loaded).toBe(false);
  });

  it('can be set to true (App.svelte sets this on walk.complete)', () => {
    store.loaded = true;
    expect(store.loaded).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// localStorage snapshot / rehydrate (audit debt #3 fix)
// ---------------------------------------------------------------------------
//
// Tests use an injected in-memory Storage mock so they never touch the shared
// jsdom localStorage and cannot interfere with unrelated test suites.

/** Minimal in-memory Storage implementation for injection into EnrichmentStore. */
function makeFakeStorage(): Storage {
  const data = new Map<string, string>();
  return {
    get length() { return data.size; },
    key(index: number): string | null {
      return [...data.keys()][index] ?? null;
    },
    getItem(key: string): string | null {
      return data.get(key) ?? null;
    },
    setItem(key: string, value: string): void {
      data.set(key, value);
    },
    removeItem(key: string): void {
      data.delete(key);
    },
    clear(): void {
      data.clear();
    },
  } as Storage;
}

describe('EnrichmentStore localStorage snapshot round-trip', () => {
  it('persists entries to storage after ingest and rehydrates into a fresh store', () => {
    const storage = makeFakeStorage();
    const storeA = new EnrichmentStore(storage);

    storeA.ingest({
      fs_path: '/project/src',
      provider_id: 'index',
      entry_id: 'p006',
      vault_id: 'p006',
      vault_kind: 'project',
      tags: ['rift', 'terminal'],
    });
    storeA.ingest({
      fs_path: '/project/src',
      provider_id: 'git',
      entry_id: 'blame',
      label: 'main@a1b2c3',
      tags: ['HEAD'],
    });

    // Snapshot must be present in storage after ingest.
    expect(storage.getItem(ENRICHMENT_STORAGE_KEY)).not.toBeNull();

    // A second store initialized from the same storage should have the same entries.
    const storeB = new EnrichmentStore(storage);
    const entries = storeB.get('/project/src');
    expect(entries).toHaveLength(2);
    const providers = entries!.map((e) => e.provider_id);
    expect(providers).toContain('index');
    expect(providers).toContain('git');
    // Check that nested fields survived serialization.
    const indexEntry = entries!.find((e) => e.provider_id === 'index');
    expect(indexEntry?.vault_id).toBe('p006');
    expect(indexEntry?.vault_kind).toBe('project');
    expect(indexEntry?.tags).toEqual(['rift', 'terminal']);
  });

  it('rehydrated store starts with loaded=false (walk.complete still needed)', () => {
    const storage = makeFakeStorage();
    const storeA = new EnrichmentStore(storage);
    storeA.ingest({ fs_path: '/a', provider_id: 'index', entry_id: 'p006', vault_id: 'p006', vault_kind: 'project', tags: [] });
    storeA.loaded = true;

    // A fresh store shares storage but NOT the loaded flag — bus must set it.
    const storeB = new EnrichmentStore(storage);
    expect(storeB.loaded).toBe(false);
  });

  it('degrades to empty map when storage contains corrupt JSON', () => {
    const storage = makeFakeStorage();
    storage.setItem(ENRICHMENT_STORAGE_KEY, '{ INVALID JSON ]]');
    const s = new EnrichmentStore(storage);
    // map should be empty — no crash.
    expect(s.map.size).toBe(0);
  });

  it('degrades to empty map when storage contains a version-mismatch snapshot', () => {
    const storage = makeFakeStorage();
    // Simulate a future schema version this build does not know.
    storage.setItem(ENRICHMENT_STORAGE_KEY, JSON.stringify({ version: 99, entries: [] }));
    const s = new EnrichmentStore(storage);
    expect(s.map.size).toBe(0);
  });

  it('degrades to empty map when storage is empty', () => {
    const storage = makeFakeStorage();
    const s = new EnrichmentStore(storage);
    expect(s.map.size).toBe(0);
  });

  it('live ingest after rehydrate deduplicates correctly (last-write-wins)', () => {
    const storage = makeFakeStorage();
    const storeA = new EnrichmentStore(storage);
    storeA.ingest({
      fs_path: '/a',
      provider_id: 'index',
      entry_id: 'p006',
      vault_id: 'p006',
      vault_kind: 'project',
      tags: ['old'],
    });

    // A new store rehydrates the stale entry, then a live bus event re-ingests.
    const storeB = new EnrichmentStore(storage);
    storeB.ingest({
      fs_path: '/a',
      provider_id: 'index',
      entry_id: 'p006',
      vault_id: 'p006',
      vault_kind: 'project',
      tags: ['live'],
    });

    const entries = storeB.get('/a');
    expect(entries).toHaveLength(1);
    expect(entries![0].tags).toEqual(['live']);
  });

  it('removeByProvider after rehydrate updates storage snapshot', () => {
    const storage = makeFakeStorage();
    const storeA = new EnrichmentStore(storage);
    storeA.ingest({ fs_path: '/a', provider_id: 'index', entry_id: 'p006', vault_id: 'p006', vault_kind: 'project', tags: [] });
    storeA.ingest({ fs_path: '/a', provider_id: 'git', entry_id: 'blame', label: 'x', tags: [] });

    storeA.removeByProvider('git');

    // A fresh store should see only the index entry.
    const storeB = new EnrichmentStore(storage);
    const entries = storeB.get('/a');
    expect(entries).toHaveLength(1);
    expect(entries![0].provider_id).toBe('index');
  });

  it('skips entries with missing fs_path or provider_id during rehydrate', () => {
    const storage = makeFakeStorage();
    // Manually write a snapshot with one malformed entry and one good entry.
    storage.setItem(
      ENRICHMENT_STORAGE_KEY,
      JSON.stringify({
        version: 1,
        entries: [
          { provider_id: 'index', entry_id: 'p006', vault_id: 'p006', vault_kind: 'project', tags: [] },   // missing fs_path
          { fs_path: '/good', entry_id: 'p007', vault_id: 'p007', vault_kind: 'project', tags: [] },        // missing provider_id
          { fs_path: '/good', provider_id: 'index', entry_id: 'p008', vault_id: 'p008', vault_kind: 'project', tags: ['ok'] }, // valid
        ],
      }),
    );
    const s = new EnrichmentStore(storage);
    // Only the valid entry should survive.
    expect(s.map.size).toBe(1);
    const entries = s.get('/good');
    expect(entries).toHaveLength(1);
    expect(entries![0].entry_id).toBe('p008');
  });
});
