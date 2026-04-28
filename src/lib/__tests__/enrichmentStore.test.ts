import { describe, it, expect, beforeEach } from 'vitest';
import { EnrichmentStore } from '../enrichmentStore.svelte';

// Unit tests for EnrichmentStore — Phase 8.6.1.
// Tests the store's public API in isolation. App.svelte subscription wiring
// is integration territory and is NOT tested here.
//
// A fresh EnrichmentStore instance is created per test via beforeEach to
// guarantee no cross-test state leakage (avoids touching the singleton).

let store: EnrichmentStore;

beforeEach(() => {
  store = new EnrichmentStore();
});

describe('EnrichmentStore.ingest', () => {
  it('puts entry under fs_path key', () => {
    store.ingest({
      fs_path: '/home/user/project',
      vault_id: 'p006',
      vault_kind: 'project',
      tags: ['rift', 'terminal'],
    });

    const result = store.get('/home/user/project');
    expect(result).toHaveLength(1);
    expect(result![0]).toEqual({
      vault_id: 'p006',
      vault_kind: 'project',
      tags: ['rift', 'terminal'],
    });
  });

  it('with duplicate vault_id+fs_path replaces entry, no duplication', () => {
    store.ingest({
      fs_path: '/home/user/project',
      vault_id: 'p006',
      vault_kind: 'project',
      tags: ['first'],
    });
    store.ingest({
      fs_path: '/home/user/project',
      vault_id: 'p006',
      vault_kind: 'project',
      tags: ['second'],
    });

    const result = store.get('/home/user/project');
    expect(result).toHaveLength(1);
    expect(result![0].tags).toEqual(['second']);
  });

  it('stacks entries with different vault_ids at same fs_path', () => {
    store.ingest({
      fs_path: '/home/user/project',
      vault_id: 'p006',
      vault_kind: 'project',
      tags: ['rift'],
    });
    store.ingest({
      fs_path: '/home/user/project',
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
});

describe('EnrichmentStore.removeByVaultId', () => {
  it('drops entries across all fs_paths for the given vault_id', () => {
    store.ingest({ fs_path: '/a', vault_id: 'p006', vault_kind: 'project', tags: [] });
    store.ingest({ fs_path: '/b', vault_id: 'p006', vault_kind: 'project', tags: [] });

    store.removeByVaultId('p006');

    // Both paths should return undefined — empty arrays are pruned (key removed).
    expect(store.get('/a')).toBeUndefined();
    expect(store.get('/b')).toBeUndefined();
  });

  it('leaves other vault_ids intact when removing one vault_id', () => {
    store.ingest({ fs_path: '/a', vault_id: 'p006',  vault_kind: 'project',   tags: ['rift'] });
    store.ingest({ fs_path: '/a', vault_id: 'pr003', vault_kind: 'practices', tags: ['gotchas'] });

    store.removeByVaultId('p006');

    const result = store.get('/a');
    expect(result).toHaveLength(1);
    expect(result![0].vault_id).toBe('pr003');
    expect(result![0].tags).toEqual(['gotchas']);
  });
});

describe('EnrichmentStore reactivity contract', () => {
  it('map identity changes on ingest (Svelte 5 assign-replace)', () => {
    const before = store.map;
    store.ingest({ fs_path: '/a', vault_id: 'p006', vault_kind: 'project', tags: [] });
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
